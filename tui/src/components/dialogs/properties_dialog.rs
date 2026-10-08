//! The note's frontmatter properties: list, delete, and the add/edit form. Core calls are async; the dialog spawns each one and the
//! result returns through `OverlayData`.

use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_core::note::{
    FrontmatterFormat, PropertyEntry, PropertyInput, PropertyKind, PropertyValue,
    is_list_property_key, property_keys_match,
};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::components::button_row::ButtonRow;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, OverlayData};
use crate::components::key_picker::{KeyPicker, PickerOutcome};
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// Write errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum PropertyWriteError {
    /// Core refused the value (`VaultError::InvalidProperty`).
    Invalid(String),
    /// The note's frontmatter block does not parse; the dialog turns read-only.
    Malformed(String),
    /// Anything else (IO, lock timeout).
    Other(String),
}

impl PropertyWriteError {
    fn from_vault(e: kimun_core::error::VaultError) -> Self {
        use kimun_core::error::{FSError, VaultError};
        match &e {
            VaultError::InvalidProperty { message, .. } => Self::Invalid(message.clone()),
            VaultError::FSError(FSError::InvalidFrontmatter { .. }) => {
                Self::Malformed(e.to_string())
            }
            _ => Self::Other(e.to_string()),
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Invalid(m) | Self::Malformed(m) | Self::Other(m) => m,
        }
    }
}

// ---------------------------------------------------------------------------
// Dialog state
// ---------------------------------------------------------------------------

pub struct PropertiesDialog {
    path: VaultPath,
    vault: Arc<NoteVault>,
    entries: Vec<PropertyEntry>,
    load_error: Option<String>,
    loading: bool,
    /// A write is in flight: every write action is ignored, and so is every
    /// way to close the dialog or its form — it stays up until the result
    /// lands, so nothing typed in the editor meanwhile meets the reload.
    busy: bool,
    selected: usize,
    scroll: usize,
    mode: Mode,
    /// `[+ Add] [Edit] [Delete] [Close]`
    buttons: ButtonRow,
    /// `[Yes] [No]`
    confirm: ButtonRow,
    focus: ListFocus,
    known_keys: Vec<String>,
    /// Rects of visible list rows from the last render (index into entries).
    row_rects: Vec<(Rect, usize)>,
    /// (old, new) of a rename sent with the in-flight write: if the set
    /// after it fails, the rename already landed and the list must reload.
    pending_rename: Option<(String, String)>,
    pub(crate) error: Option<String>,
}

enum Mode {
    List,
    ConfirmDelete(String),
    Form(Box<PropertyForm>),
}

#[derive(Clone, Copy, PartialEq)]
enum ListFocus {
    Rows,
    Buttons,
}

const BTN_ADD: usize = 0;
const BTN_EDIT: usize = 1;
const BTN_DELETE: usize = 2;
const BTN_CLOSE: usize = 3;

const KEY_COL: usize = 14;
const TYPE_COL: usize = 9;

// ---------------------------------------------------------------------------
// List state
// ---------------------------------------------------------------------------

impl PropertiesDialog {
    pub fn new(path: VaultPath, vault: Arc<NoteVault>, tx: &AppTx) -> Self {
        let mut d = Self::new_unloaded(path, vault);
        d.reload(tx);
        super::spawn_property_keys(d.vault.clone(), tx);
        d
    }

    pub(crate) fn new_unloaded(path: VaultPath, vault: Arc<NoteVault>) -> Self {
        let mut d = Self {
            path,
            vault,
            entries: Vec::new(),
            load_error: None,
            loading: true,
            busy: false,
            selected: 0,
            scroll: 0,
            mode: Mode::List,
            buttons: ButtonRow::new(&["+ Add", "Edit", "Delete", "Close"]),
            confirm: ButtonRow::new(&["Yes", "No"]),
            focus: ListFocus::Rows,
            known_keys: Vec::new(),
            row_rects: Vec::new(),
            pending_rename: None,
            error: None,
        };
        d.sync_buttons();
        d
    }

    fn reload(&mut self, tx: &AppTx) {
        self.loading = true;
        let vault = self.vault.clone();
        let path = self.path.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = vault.get_properties(&path).await.map_err(|e| e.to_string());
            tx.send(AppEvent::OverlayData(OverlayData::PropertiesLoaded {
                path,
                result,
            }))
            .ok();
        });
    }

    pub(crate) fn handle_loaded(&mut self, result: &Result<Vec<PropertyEntry>, String>) {
        self.loading = false;
        match result {
            Ok(entries) => {
                self.entries = entries.clone();
                self.load_error = None;
                self.selected = self.selected.min(self.entries.len().saturating_sub(1));
            }
            Err(e) => {
                self.entries.clear();
                self.load_error = Some(e.clone());
            }
        }
        self.sync_buttons();
    }

    fn read_only(&self) -> bool {
        self.load_error.is_some()
    }

    fn sync_buttons(&mut self) {
        let writable = !self.read_only() && !self.busy && !self.loading;
        let has_row = writable && !self.entries.is_empty();
        self.buttons.set_enabled(BTN_ADD, writable);
        self.buttons.set_enabled(BTN_EDIT, has_row);
        self.buttons.set_enabled(BTN_DELETE, has_row);
        let idle = !self.busy;
        self.buttons.set_enabled(BTN_CLOSE, idle);
        if let Mode::Form(form) = &mut self.mode {
            form.buttons.set_enabled(0, idle);
            form.buttons.set_enabled(1, idle);
            form.store_btn.set_enabled(0, idle);
        }
    }

    pub(crate) fn set_keys(&mut self, keys: Vec<String>) {
        self.known_keys = keys;
        if let Mode::Form(form) = &mut self.mode {
            form.key.set_keys(self.known_keys.clone());
        }
    }

    /// A write finished. The editor reload is not sent here: the write task
    /// sends it itself whenever the file changed, so it still arrives when
    /// the dialog was dropped before this result (the user can't close it
    /// while busy, but a file operation can dismiss every overlay).
    pub(crate) fn handle_written(
        &mut self,
        result: &Result<String, PropertyWriteError>,
        tx: &AppTx,
    ) {
        self.busy = false;
        let renamed = self.pending_rename.take();
        match result {
            Ok(flash) => {
                self.mode = Mode::List;
                tx.send(AppEvent::FlashMessage(flash.clone())).ok();
                self.reload(tx);
                super::spawn_property_keys(self.vault.clone(), tx);
            }
            Err(PropertyWriteError::Malformed(msg)) => {
                self.mode = Mode::List;
                self.load_error = Some(msg.clone());
            }
            Err(e) => {
                if let Some((_, new)) = renamed {
                    // The rename landed before the set failed: the file
                    // changed (the task already told the editor), so reload
                    // the list and stop treating `old` as the key.
                    self.reload(tx);
                    super::spawn_property_keys(self.vault.clone(), tx);
                    if let Mode::Form(form) = &mut self.mode {
                        form.orig_key = Some(new);
                    }
                }
                self.show_write_error(e);
            }
        }
        self.sync_buttons();
    }

    /// A read result addressed to `path`: ignored unless it is this dialog's note.
    pub(crate) fn on_loaded(
        &mut self,
        path: &VaultPath,
        result: &Result<Vec<PropertyEntry>, String>,
    ) {
        if *path == self.path {
            self.handle_loaded(result);
        }
    }

    /// A write result addressed to `path`: ignored unless it is this dialog's note.
    pub(crate) fn on_written(
        &mut self,
        path: &VaultPath,
        result: &Result<String, PropertyWriteError>,
        tx: &AppTx,
    ) {
        if *path == self.path {
            self.handle_written(result, tx);
        }
    }

    /// (key, type, value) as the list shows them.
    pub(crate) fn row_cells(&self, i: usize) -> (String, String, String) {
        let (key, value) = &self.entries[i];
        match value {
            Some(v) => (key.clone(), v.kind().as_str().to_string(), v.to_string()),
            None => (key.clone(), "—".to_string(), String::new()),
        }
    }

    fn can_write(&self) -> bool {
        !self.busy && !self.read_only() && !self.loading
    }

    fn start_delete(&mut self) {
        if self.busy || self.read_only() {
            return;
        }
        if let Some((key, _)) = self.entries.get(self.selected) {
            self.mode = Mode::ConfirmDelete(key.clone());
            self.confirm.set_focused(Some(1)); // default No
        }
    }

    fn confirm_delete(&mut self, tx: &AppTx) {
        let Mode::ConfirmDelete(key) = std::mem::replace(&mut self.mode, Mode::List) else {
            return;
        };
        self.busy = true;
        self.sync_buttons();
        let vault = self.vault.clone();
        let path = self.path.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = vault
                .remove_property(&path, &key)
                .await
                .map(|_| "property removed".to_string())
                .map_err(PropertyWriteError::from_vault);
            if result.is_ok() {
                tx.send(AppEvent::NoteReloadFromDisk(path.clone())).ok();
            }
            tx.send(AppEvent::OverlayData(OverlayData::PropertyWritten {
                path,
                result,
            }))
            .ok();
        });
    }

    fn open_add(&mut self) {
        self.mode = Mode::Form(Box::new(PropertyForm::add(&self.known_keys)));
    }

    fn open_edit(&mut self) {
        if let Some((key, value)) = self.entries.get(self.selected) {
            self.mode = Mode::Form(Box::new(PropertyForm::edit(
                key,
                value.as_ref(),
                &self.known_keys,
            )));
        }
    }

    fn press(&mut self, button: usize, tx: &AppTx) {
        if self.busy || (button != BTN_CLOSE && !self.can_write()) {
            return;
        }
        match button {
            BTN_ADD => self.open_add(),
            BTN_EDIT => self.open_edit(),
            BTN_DELETE => self.start_delete(),
            BTN_CLOSE => {
                tx.send(AppEvent::CloseOverlay).ok();
            }
            _ => {}
        }
    }

    fn select_next(&mut self) {
        self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match &self.mode {
            Mode::Form(_) => return self.handle_form_key(key, tx),
            Mode::ConfirmDelete(_) => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm_delete(tx),
                    KeyCode::Enter if self.confirm.focused() == Some(0) => self.confirm_delete(tx),
                    KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                        let next = if self.confirm.focused() == Some(0) {
                            1
                        } else {
                            0
                        };
                        self.confirm.set_focused(Some(next));
                    }
                    _ => self.mode = Mode::List, // n, N, Esc, Enter-on-No: cancel
                }
                return EventState::Consumed;
            }
            Mode::List => {}
        }
        self.error = None;
        let can_write = self.can_write();
        match key.code {
            KeyCode::Esc if !self.busy => {
                tx.send(AppEvent::CloseOverlay).ok();
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    ListFocus::Rows => {
                        self.buttons.set_focused(None);
                        self.buttons.focus_next();
                        ListFocus::Buttons
                    }
                    ListFocus::Buttons => {
                        self.buttons.set_focused(None);
                        ListFocus::Rows
                    }
                };
            }
            KeyCode::Left if self.focus == ListFocus::Buttons => {
                self.buttons.focus_prev();
            }
            KeyCode::Right if self.focus == ListFocus::Buttons => {
                self.buttons.focus_next();
            }
            KeyCode::Enter if self.focus == ListFocus::Buttons => {
                if let Some(b) = self.buttons.focused() {
                    self.press(b, tx);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Enter | KeyCode::Char('e') if can_write => self.open_edit(),
            KeyCode::Char('a') if can_write => self.open_add(),
            KeyCode::Char('d') | KeyCode::Delete if can_write => self.start_delete(),
            _ => {}
        }
        EventState::Consumed
    }

    pub fn handle_mouse(&mut self, ev: &MouseEvent, tx: &AppTx) -> EventState {
        if let Mode::Form(_) = self.mode {
            return self.handle_form_mouse(ev, tx);
        }
        match ev.kind {
            MouseEventKind::ScrollDown => self.select_next(),
            MouseEventKind::ScrollUp => self.selected = self.selected.saturating_sub(1),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Mode::ConfirmDelete(_) = self.mode {
                    match self.confirm.hit(ev) {
                        Some(0) => self.confirm_delete(tx),
                        Some(_) => self.mode = Mode::List,
                        None => {}
                    }
                    return EventState::Consumed;
                }
                if let Some(b) = self.buttons.hit(ev) {
                    self.press(b, tx);
                    return EventState::Consumed;
                }
                let pos = Position {
                    x: ev.column,
                    y: ev.row,
                };
                if let Some(&(_, i)) = self.row_rects.iter().find(|(r, _)| r.contains(pos)) {
                    // A click on the already-selected row (so also the second
                    // half of a double-click) opens it for editing.
                    let again = self.selected == i;
                    self.selected = i;
                    self.focus = ListFocus::Rows;
                    if again && self.can_write() {
                        self.open_edit();
                    }
                }
            }
            _ => {}
        }
        EventState::Consumed
    }

    #[cfg(test)]
    pub(crate) fn row_origin(&self, i: usize) -> Option<(u16, u16)> {
        self.row_rects
            .iter()
            .find(|(_, idx)| *idx == i)
            .map(|(r, _)| (r.x, r.y))
    }
}

// ---------------------------------------------------------------------------
// List rendering
// ---------------------------------------------------------------------------

/// Pad or truncate (with `…`) `s` to exactly `width` display columns.
fn fit(s: &str, width: usize) -> String {
    if s.width() <= width {
        return format!("{s}{}", " ".repeat(width - s.width()));
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > width {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    w += 1;
    out.push_str(&" ".repeat(width.saturating_sub(w)));
    out
}

impl PropertiesDialog {
    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme) {
        let popup = super::fixed_centered_rect(60, 16, rect);
        let title = match &self.mode {
            Mode::Form(form) if form.orig_key.is_some() => " Edit property ".to_string(),
            Mode::Form(_) => " Add property ".to_string(),
            _ => format!(" Properties: {} ", self.path.get_clean_name()),
        };
        let inner = modal_chrome(
            f,
            popup,
            theme,
            ModalSpec {
                title: Some(&title),
                border: None,
                bg: crate::components::panel::ModalBg::Panel,
            },
        );
        self.row_rects.clear();
        if inner.height < 4 {
            return;
        }
        if let Mode::Form(_) = self.mode {
            self.render_form(f, inner, theme);
            return;
        }
        let bg = theme.bg_panel.to_ratatui();
        let normal = Style::default().fg(theme.fg.to_ratatui()).bg(bg);
        let gray = Style::default().fg(theme.gray.to_ratatui()).bg(bg);
        let row_at = |i: u16| Rect {
            x: inner.x,
            y: inner.y + i,
            width: inner.width,
            height: 1,
        };
        let buttons_row = inner.height - 2;
        let footer_row = inner.height - 1;

        f.render_widget(
            Paragraph::new("  Key            Type      Value").style(gray),
            row_at(0),
        );

        let has_error_row = self.error.is_some();
        let body_rows = (buttons_row as usize - 1).saturating_sub(has_error_row as usize);

        if self.loading {
            f.render_widget(Paragraph::new("  Loading…").style(gray), row_at(1));
        } else if let Some(msg) = &self.load_error {
            super::render_error_row(f, row_at(1), msg, theme);
            f.render_widget(
                Paragraph::new("  Read-only: fix the frontmatter in the editor.").style(gray),
                row_at(2),
            );
        } else if self.entries.is_empty() {
            f.render_widget(
                Paragraph::new("  No properties. Press a or click + Add.").style(gray),
                row_at(1),
            );
        } else {
            if self.selected < self.scroll {
                self.scroll = self.selected;
            } else if body_rows > 0 && self.selected >= self.scroll + body_rows {
                self.scroll = self.selected + 1 - body_rows;
            }
            let value_w = (inner.width as usize).saturating_sub(2 + KEY_COL + 1 + TYPE_COL + 1);
            let visible: Vec<usize> = (self.scroll..self.entries.len()).take(body_rows).collect();
            for (n, i) in visible.into_iter().enumerate() {
                let (key, kind, value) = self.row_cells(i);
                let marker = if i == self.selected { "▶ " } else { "  " };
                let text = format!(
                    "{marker}{} {} {}",
                    fit(&key, KEY_COL),
                    fit(&kind, TYPE_COL),
                    fit(&value, value_w)
                );
                let style = if i == self.selected {
                    Style::default()
                        .fg(theme.selection_fg.to_ratatui())
                        .bg(theme.selection_bg.to_ratatui())
                } else {
                    normal
                };
                let r = row_at(1 + n as u16);
                f.render_widget(Paragraph::new(text).style(style), r);
                self.row_rects.push((r, i));
            }
        }

        if let Some(msg) = &self.error {
            super::render_error_row(f, row_at(buttons_row - 1), msg, theme);
        }

        let brow = row_at(buttons_row);
        if let Mode::ConfirmDelete(key) = &self.mode {
            let label = format!(" Delete \"{key}\"? y/n ");
            let lw = (label.width() as u16).min(brow.width);
            f.render_widget(
                Paragraph::new(label).style(normal.add_modifier(Modifier::BOLD)),
                Rect { width: lw, ..brow },
            );
            self.confirm.render(
                f,
                Rect {
                    x: brow.x + lw,
                    width: brow.width - lw,
                    ..brow
                },
                theme,
            );
        } else {
            self.buttons.render(f, brow, theme);
        }
        f.render_widget(
            Paragraph::new("  ↑↓ select · Enter edit · a add · d delete · Esc close").style(gray),
            row_at(footer_row),
        );
    }
}

// ---------------------------------------------------------------------------
// Form state
// ---------------------------------------------------------------------------

/// `None` = auto (let core decide against the vault's type).
const TYPES: [Option<PropertyKind>; 7] = [
    None,
    Some(PropertyKind::Text),
    Some(PropertyKind::Number),
    Some(PropertyKind::Bool),
    Some(PropertyKind::Date),
    Some(PropertyKind::DateTime),
    Some(PropertyKind::List),
];

/// Width of the form's field labels (`" Key    "`).
const LABEL_W: u16 = 8;

#[derive(Clone, Copy, PartialEq)]
enum FormFocus {
    Key,
    Kind,
    Value,
    StoreAnyway,
    Buttons,
}

struct PropertyForm {
    /// `None` for Add; the original key for Edit.
    orig_key: Option<String>,
    /// The original value's kind (Edit), shown as "(currently <kind>)".
    current_kind: Option<PropertyKind>,
    /// The original entry had no value (YAML `due:`).
    was_bare: bool,
    /// The Value text the Edit form was pre-filled with (`None` for Add or a
    /// bare key): saving it unchanged under auto writes nothing.
    orig_text: Option<String>,
    key: KeyPicker,
    /// Index into TYPES; 0 = auto.
    kind_idx: usize,
    value: SingleLineInput,
    focus: FormFocus,
    /// `[Save] [Cancel]`
    buttons: ButtonRow,
    /// Shown under Value: core's refusal or a local check.
    error: Option<String>,
    /// Offered after an `auto` save was refused: retry forced to this kind.
    store_anyway: Option<PropertyKind>,
    /// `[Store as <kind> anyway]`
    store_btn: ButtonRow,
    // Rects from the last render.
    value_rect: Option<Rect>,
    kind_prev_rect: Option<Rect>,
    kind_next_rect: Option<Rect>,
}

/// What a form key or click asks the dialog to do once the form borrow ends.
enum FormAction {
    None,
    Submit,
    StoreAnyway,
    Cancel,
}

impl PropertyForm {
    fn blank(orig_key: Option<String>, key: &str, known_keys: &[String]) -> Self {
        let mut picker = KeyPicker::new(key);
        picker.set_keys(known_keys.to_vec());
        Self {
            orig_key,
            current_kind: None,
            was_bare: false,
            orig_text: None,
            key: picker,
            kind_idx: 0,
            value: SingleLineInput::new(),
            focus: FormFocus::Key,
            buttons: ButtonRow::new(&["Save", "Cancel"]),
            error: None,
            store_anyway: None,
            store_btn: ButtonRow::new(&["Store anyway"]),
            value_rect: None,
            kind_prev_rect: None,
            kind_next_rect: None,
        }
    }

    fn add(known_keys: &[String]) -> Self {
        Self::blank(None, "", known_keys)
    }

    fn edit(key: &str, value: Option<&PropertyValue>, known_keys: &[String]) -> Self {
        let mut form = Self::blank(Some(key.to_string()), key, known_keys);
        match value {
            Some(v) => {
                form.current_kind = Some(v.kind());
                // An empty list displays as `[]`; pre-fill it as empty.
                let text = match v {
                    PropertyValue::List(items) if items.is_empty() => String::new(),
                    other => other.to_string(),
                };
                form.orig_text = Some(text.clone());
                form.value = SingleLineInput::with_value(text);
            }
            None => form.was_bare = true,
        }
        form
    }

    fn kind(&self) -> Option<PropertyKind> {
        TYPES[self.kind_idx]
    }

    /// The field as `PropertyInput` values: comma-split for a list (chosen,
    /// or `auto` on a key that was a list or that core always keeps as a
    /// list), one untrimmed value otherwise.
    fn input_values(&self) -> Vec<String> {
        let as_list = match self.kind() {
            Some(k) => k == PropertyKind::List,
            None => {
                self.current_kind == Some(PropertyKind::List)
                    || is_list_property_key(self.key.value().trim())
            }
        };
        if as_list {
            self.value
                .value()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        } else {
            vec![self.value.value().to_string()]
        }
    }

    /// The kind the input infers to: the "Store as <kind> anyway" target.
    fn inferred_kind(&self) -> PropertyKind {
        let v = self.input_values();
        if v.len() > 1 {
            PropertyKind::List
        } else {
            PropertyValue::infer(v.first().map_or("", String::as_str)).kind()
        }
    }

    fn clear_feedback(&mut self) {
        self.error = None;
        self.store_anyway = None;
        if self.focus == FormFocus::StoreAnyway {
            self.focus = FormFocus::Value;
        }
    }

    fn cycle_kind(&mut self, forward: bool) {
        let n = TYPES.len();
        self.kind_idx = if forward {
            (self.kind_idx + 1) % n
        } else {
            (self.kind_idx + n - 1) % n
        };
        self.clear_feedback();
    }

    fn set_focus(&mut self, focus: FormFocus) {
        if focus != FormFocus::Buttons {
            self.buttons.set_focused(None);
        }
        self.focus = focus;
    }

    /// Key → Kind → Value → [StoreAnyway] → Save → Cancel → Key.
    fn focus_next(&mut self) {
        match self.focus {
            FormFocus::Key => self.set_focus(FormFocus::Kind),
            FormFocus::Kind => self.set_focus(FormFocus::Value),
            FormFocus::Value if self.store_anyway.is_some() => {
                self.set_focus(FormFocus::StoreAnyway)
            }
            FormFocus::Value | FormFocus::StoreAnyway => {
                self.focus = FormFocus::Buttons;
                self.buttons.set_focused(None);
                if !self.buttons.focus_next() {
                    self.set_focus(FormFocus::Key);
                }
            }
            FormFocus::Buttons => {
                if !self.buttons.focus_next() {
                    self.set_focus(FormFocus::Key);
                }
            }
        }
    }

    fn focus_prev(&mut self) {
        match self.focus {
            FormFocus::Key => {
                self.focus = FormFocus::Buttons;
                self.buttons.set_focused(None);
                if !self.buttons.focus_prev() {
                    self.focus_prev_field();
                }
            }
            FormFocus::Buttons => {
                if !self.buttons.focus_prev() {
                    self.focus_prev_field();
                }
            }
            FormFocus::StoreAnyway => self.set_focus(FormFocus::Value),
            FormFocus::Value => self.set_focus(FormFocus::Kind),
            FormFocus::Kind => self.set_focus(FormFocus::Key),
        }
    }

    /// The field before the button row.
    fn focus_prev_field(&mut self) {
        if self.store_anyway.is_some() {
            self.set_focus(FormFocus::StoreAnyway);
        } else {
            self.set_focus(FormFocus::Value);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> FormAction {
        // The key picker's open list owns Enter/Esc/arrows first.
        if self.focus == FormFocus::Key {
            let before = self.key.value().to_string();
            match self.key.handle_key(&key) {
                PickerOutcome::Accepted(_) | PickerOutcome::Changed | PickerOutcome::Consumed => {
                    if self.key.value() != before {
                        self.clear_feedback();
                    }
                    return FormAction::None;
                }
                PickerOutcome::Submit => return FormAction::Submit,
                PickerOutcome::Cancel => return FormAction::Cancel,
                PickerOutcome::NotConsumed => {} // Tab etc. below
            }
        }
        match key.code {
            KeyCode::Esc => FormAction::Cancel,
            // Up/Down reach here only when the key picker's list is closed.
            KeyCode::Tab | KeyCode::Down => {
                self.focus_next();
                FormAction::None
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.focus_prev();
                FormAction::None
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                if self.focus == FormFocus::Kind =>
            {
                self.cycle_kind(key.code != KeyCode::Left);
                FormAction::None
            }
            KeyCode::Left if self.focus == FormFocus::Buttons => {
                self.buttons.focus_prev();
                FormAction::None
            }
            KeyCode::Right if self.focus == FormFocus::Buttons => {
                self.buttons.focus_next();
                FormAction::None
            }
            KeyCode::Enter => match self.focus {
                FormFocus::StoreAnyway => FormAction::StoreAnyway,
                FormFocus::Buttons if self.buttons.focused() == Some(1) => FormAction::Cancel,
                _ => FormAction::Submit,
            },
            _ if self.focus == FormFocus::Value => {
                if self.value.handle_key(&key) == InputOutcome::Changed {
                    self.clear_feedback();
                }
                FormAction::None
            }
            _ => FormAction::None,
        }
    }

    fn handle_click(&mut self, col: u16, row: u16) -> FormAction {
        let pos = Position { x: col, y: row };
        let hit = |r: Option<Rect>| r.is_some_and(|r| r.contains(pos));
        match self.key.handle_click(col, row) {
            PickerOutcome::Accepted(_) => {
                self.set_focus(FormFocus::Key);
                self.clear_feedback();
                return FormAction::None;
            }
            PickerOutcome::Consumed => {
                self.set_focus(FormFocus::Key);
                self.key.cursor_to_end();
                return FormAction::None;
            }
            _ => {}
        }
        if hit(self.value_rect) {
            self.set_focus(FormFocus::Value);
            // Cursor at the end, as a click into the field promises.
            let text = self.value.value().to_string();
            self.value.set_value(text);
            return FormAction::None;
        }
        if hit(self.kind_prev_rect) || hit(self.kind_next_rect) {
            self.set_focus(FormFocus::Kind);
            self.cycle_kind(hit(self.kind_next_rect));
            return FormAction::None;
        }
        if self.store_anyway.is_some() && self.store_btn.hit_at(col, row).is_some() {
            return FormAction::StoreAnyway;
        }
        match self.buttons.hit_at(col, row) {
            Some(0) => FormAction::Submit,
            Some(_) => FormAction::Cancel,
            None => FormAction::None,
        }
    }
}

impl PropertiesDialog {
    fn handle_form_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        let Mode::Form(form) = &mut self.mode else {
            return EventState::Consumed;
        };
        let action = form.handle_key(key);
        self.run_form_action(action, tx);
        EventState::Consumed
    }

    fn handle_form_mouse(&mut self, ev: &MouseEvent, tx: &AppTx) -> EventState {
        if ev.kind != MouseEventKind::Down(MouseButton::Left) {
            return EventState::Consumed;
        }
        let Mode::Form(form) = &mut self.mode else {
            return EventState::Consumed;
        };
        let action = form.handle_click(ev.column, ev.row);
        self.run_form_action(action, tx);
        EventState::Consumed
    }

    fn run_form_action(&mut self, action: FormAction, tx: &AppTx) {
        match action {
            FormAction::None => {}
            FormAction::Submit => self.submit(None, tx),
            FormAction::StoreAnyway => self.press_store_anyway(tx),
            // A save in flight keeps the form up until its result.
            FormAction::Cancel if self.busy => {}
            FormAction::Cancel => self.mode = Mode::List,
        }
    }

    /// The `[Store as <kind> anyway]` button: retry forced to that kind.
    pub(crate) fn press_store_anyway(&mut self, tx: &AppTx) {
        if let Mode::Form(form) = &self.mode
            && let Some(kind) = form.store_anyway
        {
            self.submit(Some(kind), tx);
        }
    }

    /// Validate and spawn the write. `forced` overrides the type cycler
    /// (the "store anyway" retry).
    fn submit(&mut self, forced: Option<PropertyKind>, tx: &AppTx) {
        if self.busy {
            return;
        }
        let Mode::Form(form) = &mut self.mode else {
            return;
        };
        let new_key = form.key.value().trim().to_string();
        if new_key.is_empty() {
            form.error = Some("key required".into());
            form.store_anyway = None;
            return;
        }
        // Add never overwrites: a key the note already has is edited instead.
        if form.orig_key.is_none()
            && self
                .entries
                .iter()
                .any(|(k, _)| property_keys_match(k, &new_key))
        {
            form.error = Some(format!("'{new_key}' already exists — edit it"));
            form.store_anyway = None;
            return;
        }
        // An Edit saved as pre-filled (same key, type auto, same text) would
        // only re-run inference on Display text: write nothing.
        if forced.is_none()
            && form.kind().is_none()
            && form.orig_key.as_deref() == Some(new_key.as_str())
            && form.orig_text.as_deref() == Some(form.value.value())
        {
            self.mode = Mode::List;
            return;
        }
        let values = form.input_values();
        let value_empty = values.iter().all(|v| v.trim().is_empty());
        let renamed = form
            .orig_key
            .as_deref()
            .filter(|o| *o != new_key)
            .map(str::to_string);
        // A bare key whose value is still empty: rename only (or nothing).
        let rename_only = form.was_bare && value_empty;
        if value_empty && !rename_only {
            form.error = Some("value required".into());
            form.store_anyway = None;
            return;
        }
        if rename_only && renamed.is_none() {
            self.mode = Mode::List;
            return;
        }
        let mut input = PropertyInput::new(values).forced(forced.or(form.kind()));
        // Edit under auto: the note's own kind is the guess when the vault
        // has none (core still checks it against the vault's kind).
        if let Some(kind) = form.current_kind {
            input = input.implied(kind);
        }
        form.clear_feedback();
        self.pending_rename = renamed.clone().map(|old| (old, new_key.clone()));
        self.busy = true;
        self.sync_buttons();
        let vault = self.vault.clone();
        let path = self.path.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            // Whether the file changed: the rename alone counts, even if the
            // set after it fails.
            let mut changed = false;
            let result = async {
                if let Some(old) = &renamed {
                    // A refused rename is never a type mismatch: report it as
                    // `Other` so the form offers no "Store as … anyway".
                    vault
                        .rename_property(&path, old, &new_key)
                        .await
                        .map_err(|e| match PropertyWriteError::from_vault(e) {
                            PropertyWriteError::Invalid(m) => PropertyWriteError::Other(m),
                            other => other,
                        })?;
                    changed = true;
                }
                if !rename_only {
                    vault
                        .set_property_from_input(&path, &new_key, &input, FrontmatterFormat::Toml)
                        .await
                        .map_err(PropertyWriteError::from_vault)?;
                    changed = true;
                }
                Ok::<_, PropertyWriteError>("property saved".to_string())
            }
            .await;
            // Sent by the task, not the result handler: the editor must
            // reload even when the dialog closed before the result arrives.
            if changed {
                tx.send(AppEvent::NoteReloadFromDisk(path.clone())).ok();
            }
            tx.send(AppEvent::OverlayData(OverlayData::PropertyWritten {
                path,
                result,
            }))
            .ok();
        });
    }

    /// A failed write: under Value in the form (with `[Store as … anyway]`
    /// after an `auto` mismatch), or the list's error row after a delete.
    fn show_write_error(&mut self, e: &PropertyWriteError) {
        let Mode::Form(form) = &mut self.mode else {
            self.error = Some(e.message().to_string());
            return;
        };
        form.error = Some(e.message().to_string());
        form.store_anyway = match e {
            PropertyWriteError::Invalid(_) if form.kind().is_none() => Some(form.inferred_kind()),
            _ => None,
        };
        if let Some(kind) = form.store_anyway {
            form.store_btn = ButtonRow::new(&[&format!("Store as {kind} anyway")]);
        }
    }

    #[cfg(test)]
    pub(crate) fn form_error(&self) -> Option<String> {
        match &self.mode {
            Mode::Form(form) => form.error.clone(),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn store_anyway_kind(&self) -> Option<PropertyKind> {
        match &self.mode {
            Mode::Form(form) => form.store_anyway,
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn form_kind(&self) -> Option<PropertyKind> {
        match &self.mode {
            Mode::Form(form) => form.kind(),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn form_orig_key(&self) -> Option<String> {
        match &self.mode {
            Mode::Form(form) => form.orig_key.clone(),
            _ => None,
        }
    }

    #[cfg(test)]
    fn form_focus(&self) -> Option<FormFocus> {
        match &self.mode {
            Mode::Form(form) => Some(form.focus),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn kind_next_origin(&self) -> Option<(u16, u16)> {
        match &self.mode {
            Mode::Form(form) => form.kind_next_rect.map(|r| (r.x, r.y)),
            _ => None,
        }
    }

    fn render_form(&mut self, f: &mut Frame, inner: Rect, theme: &Theme) {
        let Mode::Form(form) = &mut self.mode else {
            return;
        };
        let bg = theme.bg_panel.to_ratatui();
        let normal = Style::default().fg(theme.fg.to_ratatui()).bg(bg);
        let gray = Style::default().fg(theme.gray.to_ratatui()).bg(bg);
        let accent = Style::default()
            .fg(theme.accent.to_ratatui())
            .bg(bg)
            .add_modifier(Modifier::BOLD);
        let row_at = |i: u16| Rect {
            x: inner.x,
            y: inner.y + i,
            width: inner.width,
            height: 1,
        };
        let field_at = |i: u16| Rect {
            x: inner.x + LABEL_W,
            width: inner.width.saturating_sub(LABEL_W + 1),
            ..row_at(i)
        };
        let label = |f: &mut Frame, text: &str, row: u16, focused: bool| {
            let style = if focused { accent } else { normal };
            f.render_widget(
                Paragraph::new(format!(" {text}")).style(style),
                Rect {
                    width: LABEL_W.min(inner.width),
                    ..row_at(row)
                },
            );
        };
        const KEY_ROW: u16 = 1;
        const KIND_ROW: u16 = 2;
        const VALUE_ROW: u16 = 3;

        label(f, "Key", KEY_ROW, form.focus == FormFocus::Key);
        label(f, "Type", KIND_ROW, form.focus == FormFocus::Kind);
        label(f, "Value", VALUE_ROW, form.focus == FormFocus::Value);

        // Type cycler: ‹ kind ›   (currently <kind>)
        let kind_row = field_at(KIND_ROW);
        let kind = form.kind().map_or("auto", PropertyKind::as_str);
        let kind_text = format!(" {kind} ");
        let kind_w = kind_text.width() as u16;
        let kind_style = if form.focus == FormFocus::Kind {
            Style::default()
                .fg(theme.selection_fg.to_ratatui())
                .bg(theme.selection_bg.to_ratatui())
        } else {
            normal
        };
        let cell = |x: u16, w: u16| Rect {
            x,
            width: w.min(kind_row.right().saturating_sub(x)),
            ..kind_row
        };
        let prev = cell(kind_row.x, 1);
        let text = cell(kind_row.x + 1, kind_w);
        let next = cell(kind_row.x + 1 + kind_w, 1);
        f.render_widget(Paragraph::new("‹").style(normal), prev);
        f.render_widget(Paragraph::new(kind_text).style(kind_style), text);
        f.render_widget(Paragraph::new("›").style(normal), next);
        form.kind_prev_rect = Some(prev);
        form.kind_next_rect = Some(next);
        if let Some(current) = form.current_kind {
            let hint_x = next.x + 1;
            f.render_widget(
                Paragraph::new(format!("   (currently {current})")).style(gray),
                cell(hint_x, kind_row.right().saturating_sub(hint_x)),
            );
        }

        let value_rect = field_at(VALUE_ROW);
        form.value_rect = Some(value_rect);
        let vstyle = Style::default()
            .fg(theme.fg_bright.to_ratatui())
            .bg(theme.bg.to_ratatui());
        form.value
            .render(f, value_rect, vstyle, 0, form.focus == FormFocus::Value);

        if let Some(msg) = &form.error {
            f.render_widget(
                Paragraph::new(msg.as_str())
                    .style(Style::default().fg(theme.red.to_ratatui()).bg(bg)),
                field_at(VALUE_ROW + 1),
            );
        }
        if form.store_anyway.is_some() {
            form.store_btn
                .set_focused((form.focus == FormFocus::StoreAnyway).then_some(0));
            let r = field_at(VALUE_ROW + 2);
            // ButtonRow pads one cell on the left; align the bracket with the field.
            form.store_btn.render(
                f,
                Rect {
                    x: r.x.saturating_sub(1),
                    width: r.width + 1,
                    ..r
                },
                theme,
            );
        }

        let btn_row = inner.height.saturating_sub(2);
        form.buttons.render(f, row_at(btn_row), theme);
        f.render_widget(
            Paragraph::new("  Tab/↑↓ next field · ←→ type · Enter save · Esc back").style(gray),
            row_at(inner.height.saturating_sub(1)),
        );
        // The picker renders last so its suggestion list sits on top.
        form.key.render(
            f,
            field_at(KEY_ROW),
            inner,
            theme,
            form.focus == FormFocus::Key,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::note::PropertyValue;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use ratatui::{Terminal, backend::TestBackend};
    use tokio::sync::mpsc::unbounded_channel;

    use crate::components::events::InputEvent;
    use crate::settings::themes::Theme;
    use crate::test_support::{mouse_down_at, temp_vault};

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    async fn dialog_with(
        entries: Vec<PropertyEntry>,
    ) -> (
        PropertiesDialog,
        AppTx,
        tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    ) {
        let vault = temp_vault("props_dialog").await;
        let (tx, rx) = unbounded_channel();
        let mut d = PropertiesDialog::new_unloaded(VaultPath::note_path_from("/n.md"), vault);
        d.handle_loaded(&Ok(entries));
        (d, tx, rx)
    }

    fn sample() -> Vec<PropertyEntry> {
        vec![
            ("status".into(), Some(PropertyValue::Text("draft".into()))),
            (
                "tags".into(),
                Some(PropertyValue::List(vec!["work".into(), "q4".into()])),
            ),
            ("due".into(), None),
        ]
    }

    fn draw(d: &mut PropertiesDialog) {
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| d.render(f, f.area(), &theme)).unwrap();
    }

    fn mouse(col: u16, row: u16) -> MouseEvent {
        match mouse_down_at(col, row) {
            InputEvent::Mouse(m) => m,
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn rows_show_key_type_value() {
        let (d, _tx, _rx) = dialog_with(sample()).await;
        assert_eq!(
            d.row_cells(0),
            ("status".into(), "text".into(), "draft".into())
        );
        assert_eq!(
            d.row_cells(1),
            ("tags".into(), "list".into(), "work, q4".into())
        );
        assert_eq!(d.row_cells(2), ("due".into(), "—".into(), String::new()));
    }

    #[tokio::test]
    async fn arrows_and_jk_move_selection() {
        let (mut d, tx, _rx) = dialog_with(sample()).await;
        d.handle_key(k(KeyCode::Down), &tx);
        d.handle_key(k(KeyCode::Char('j')), &tx);
        assert_eq!(d.selected, 2);
        d.handle_key(k(KeyCode::Down), &tx);
        assert_eq!(d.selected, 2, "stops at the end");
        d.handle_key(k(KeyCode::Char('k')), &tx);
        assert_eq!(d.selected, 1);
    }

    #[tokio::test]
    async fn click_selects_then_second_click_edits() {
        let (mut d, tx, _rx) = dialog_with(sample()).await;
        draw(&mut d);
        let (x, y) = d.row_origin(1).unwrap();
        d.handle_mouse(&mouse(x + 1, y), &tx);
        assert_eq!(d.selected, 1);
        assert!(matches!(d.mode, Mode::List));
        d.handle_mouse(&mouse(x + 1, y), &tx);
        assert!(
            matches!(d.mode, Mode::Form(_)),
            "second click opens the form"
        );
    }

    #[tokio::test]
    async fn delete_asks_and_y_removes_from_disk() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        d.vault
            .create_note(&d.path, "+++\nstatus = \"draft\"\n+++\nbody")
            .await
            .unwrap();
        d.handle_key(k(KeyCode::Char('d')), &tx);
        assert!(matches!(d.mode, Mode::ConfirmDelete(ref key) if key == "status"));
        d.handle_key(k(KeyCode::Char('y')), &tx);
        assert!(d.busy, "write in flight");
        // The spawned remove reports back as PropertyWritten (after the
        // editor reload it sends itself).
        assert!(next_written(&mut rx).await.is_ok());
        assert!(d.vault.get_properties(&d.path).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_cancelled_by_n_or_esc() {
        for code in [KeyCode::Char('n'), KeyCode::Esc] {
            let (mut d, tx, mut rx) = dialog_with(sample()).await;
            d.handle_key(k(KeyCode::Delete), &tx);
            d.handle_key(k(code), &tx);
            assert!(matches!(d.mode, Mode::List));
            assert!(rx.try_recv().is_err(), "nothing written");
        }
    }

    #[tokio::test]
    async fn malformed_frontmatter_is_read_only() {
        let vault = temp_vault("props_dialog").await;
        let (tx, mut rx) = unbounded_channel();
        let mut d = PropertiesDialog::new_unloaded(VaultPath::note_path_from("/n.md"), vault);
        d.handle_loaded(&Err("'a' appears more than once".into()));
        for code in [
            KeyCode::Char('a'),
            KeyCode::Char('e'),
            KeyCode::Char('d'),
            KeyCode::Enter,
        ] {
            d.handle_key(k(code), &tx);
        }
        assert!(matches!(d.mode, Mode::List));
        assert!(rx.try_recv().is_err());
        assert!(!d.buttons.is_enabled(BTN_ADD));
        assert!(d.buttons.is_enabled(BTN_CLOSE));
    }

    #[tokio::test]
    async fn esc_closes_dialog() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        d.handle_key(k(KeyCode::Esc), &tx);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::CloseOverlay)));
    }

    #[tokio::test]
    async fn successful_write_reloads_list_and_flashes() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        d.busy = true;
        d.handle_written(&Ok("property saved".into()), &tx);
        assert!(!d.busy);
        let mut saw_reload = false;
        let mut saw_flash = false;
        while let Ok(evt) = rx.try_recv() {
            match evt {
                AppEvent::NoteReloadFromDisk(_) => saw_reload = true,
                AppEvent::FlashMessage(m) => saw_flash = m == "property saved",
                _ => {}
            }
        }
        assert!(saw_flash);
        assert!(
            !saw_reload,
            "the write task sends the editor reload, not the result handler"
        );
        assert!(d.loading, "list re-read requested");
    }

    /// Open the form on a fresh Add and save `key = value` (type auto).
    fn add_via_keys(d: &mut PropertiesDialog, tx: &AppTx, key: &str, value: &str) {
        d.handle_key(k(KeyCode::Char('a')), tx);
        type_into(d, tx, key);
        d.handle_key(k(KeyCode::Tab), tx);
        d.handle_key(k(KeyCode::Tab), tx);
        type_into(d, tx, value);
        d.handle_key(k(KeyCode::Enter), tx);
    }

    async fn next_reload(rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>) -> VaultPath {
        loop {
            let evt = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("timely reload")
                .expect("open channel");
            if let AppEvent::NoteReloadFromDisk(p) = evt {
                return p;
            }
        }
    }

    #[tokio::test]
    async fn closing_mid_save_still_reloads_the_editor() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        vault_note(&d, "body").await;
        let path = d.path.clone();
        let vault = d.vault.clone();
        add_via_keys(&mut d, &tx, "k", "v");
        assert!(d.busy, "write in flight");
        // Esc / Close: the editor drops the dialog before the result lands.
        drop(d);
        assert_eq!(next_reload(&mut rx).await, path);
        assert_eq!(
            vault.get_property(&path, "k").await.unwrap(),
            Some(Some(PropertyValue::Text("v".into())))
        );
    }

    #[tokio::test]
    async fn closing_mid_delete_still_reloads_the_editor() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        vault_note(&d, "+++\nstatus = \"draft\"\n+++\nbody").await;
        let path = d.path.clone();
        d.handle_key(k(KeyCode::Char('d')), &tx);
        d.handle_key(k(KeyCode::Char('y')), &tx);
        drop(d);
        assert_eq!(next_reload(&mut rx).await, path);
    }

    #[tokio::test]
    async fn results_for_another_note_are_ignored() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        let other = VaultPath::note_path_from("/other.md");
        d.busy = true;
        d.on_written(&other, &Ok("property saved".into()), &tx);
        assert!(d.busy, "another note's write result is not this dialog's");
        assert!(rx.try_recv().is_err(), "nothing flashed or reloaded");
        d.on_loaded(&other, &Ok(vec![]));
        assert_eq!(d.entries.len(), 3, "another note's entries are not loaded");
        // The dialog's own path still lands.
        let own = d.path.clone();
        d.on_written(&own, &Ok("property saved".into()), &tx);
        assert!(!d.busy);
        d.on_loaded(&own, &Ok(vec![]));
        assert!(d.entries.is_empty());
    }

    #[tokio::test]
    async fn unchanged_edit_save_writes_nothing() {
        let (mut d, tx, mut rx) = dialog_with(vec![(
            "authors".into(),
            Some(PropertyValue::List(vec!["Doe, John".into()])),
        )])
        .await;
        let text = "+++\nauthors = [\"Doe, John\"]\n+++\nbody";
        vault_note(&d, text).await;
        d.handle_key(k(KeyCode::Enter), &tx); // edit row 0
        d.handle_key(k(KeyCode::Enter), &tx); // save unchanged
        assert!(matches!(d.mode, Mode::List), "back to the list");
        assert!(!d.busy, "no write spawned");
        let spawned =
            tokio::time::timeout(std::time::Duration::from_millis(300), next_written(&mut rx))
                .await;
        assert!(spawned.is_err(), "nothing written");
        assert_eq!(d.vault.get_note_text(&d.path).await.unwrap(), text);
    }

    #[tokio::test]
    async fn auto_edit_keeps_the_notes_own_kind_when_the_vault_has_none() {
        let (mut d, tx, mut rx) = dialog_with(vec![(
            "code".into(),
            Some(PropertyValue::Text("42".into())),
        )])
        .await;
        vault_note(&d, "+++\ncode = \"42\"\n+++\nbody").await;
        d.handle_key(k(KeyCode::Enter), &tx); // edit row 0
        d.handle_key(k(KeyCode::Tab), &tx); // Type (auto)
        d.handle_key(k(KeyCode::Tab), &tx); // Value
        d.handle_key(k(KeyCode::Backspace), &tx);
        type_into(&mut d, &tx, "3");
        d.handle_key(k(KeyCode::Enter), &tx);
        assert!(next_written(&mut rx).await.is_ok());
        assert_eq!(
            d.vault.get_property(&d.path, "code").await.unwrap(),
            Some(Some(PropertyValue::Text("43".into())))
        );
    }

    #[tokio::test]
    async fn add_with_an_existing_key_is_refused_locally() {
        let (mut d, tx, mut rx) = dialog_with(vec![(
            "status".into(),
            Some(PropertyValue::Text("draft".into())),
        )])
        .await;
        let text = "+++\nstatus = \"draft\"\n+++\nbody";
        vault_note(&d, text).await;
        add_via_keys(&mut d, &tx, "Status", "done");
        assert_eq!(
            d.form_error().as_deref(),
            Some("'Status' already exists — edit it")
        );
        assert!(!d.busy, "nothing spawned");
        let spawned =
            tokio::time::timeout(std::time::Duration::from_millis(300), next_written(&mut rx))
                .await;
        assert!(spawned.is_err(), "nothing written");
        assert_eq!(d.vault.get_note_text(&d.path).await.unwrap(), text);
    }

    #[tokio::test]
    async fn malformed_write_error_turns_read_only() {
        let (mut d, tx, _rx) = dialog_with(sample()).await;
        d.busy = true;
        d.handle_written(&Err(PropertyWriteError::Malformed("bad block".into())), &tx);
        assert!(d.read_only());
        assert!(!d.buttons.is_enabled(BTN_ADD));
    }

    async fn vault_note(d: &PropertiesDialog, text: &str) {
        d.vault.create_note(&d.path, text).await.unwrap();
    }

    async fn next_written(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    ) -> Result<String, PropertyWriteError> {
        loop {
            let evt = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("timely")
                .expect("open channel");
            if let AppEvent::OverlayData(OverlayData::PropertyWritten { result, .. }) = evt {
                return result;
            }
        }
    }

    fn type_into(d: &mut PropertiesDialog, tx: &AppTx, s: &str) {
        for c in s.chars() {
            d.handle_key(k(KeyCode::Char(c)), tx);
        }
    }

    #[tokio::test]
    async fn add_writes_a_new_property() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        vault_note(&d, "body").await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        type_into(&mut d, &tx, "priority");
        d.handle_key(k(KeyCode::Tab), &tx); // → Type
        d.handle_key(k(KeyCode::Tab), &tx); // → Value
        type_into(&mut d, &tx, "2");
        d.handle_key(k(KeyCode::Enter), &tx);
        assert!(next_written(&mut rx).await.is_ok());
        assert_eq!(
            d.vault.get_property(&d.path, "priority").await.unwrap(),
            Some(Some(PropertyValue::Number(2.0)))
        );
    }

    #[tokio::test]
    async fn empty_key_and_empty_value_are_refused_locally() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        d.handle_key(k(KeyCode::Enter), &tx);
        assert_eq!(d.form_error().as_deref(), Some("key required"));
        type_into(&mut d, &tx, "k");
        d.handle_key(k(KeyCode::Enter), &tx);
        assert_eq!(d.form_error().as_deref(), Some("value required"));
        assert!(rx.try_recv().is_err(), "nothing spawned");
    }

    // The brief's tests pressed Esc after typing a key "to close the
    // suggestion list if open". With no known keys the list never opens, and
    // Esc on a closed list return to the list, so those Esc
    // presses are dropped here.

    #[tokio::test]
    async fn key_change_renames_then_sets() {
        let (mut d, tx, mut rx) = dialog_with(vec![(
            "status".into(),
            Some(PropertyValue::Text("draft".into())),
        )])
        .await;
        vault_note(&d, "+++\na = 1\nstatus = \"draft\"\nz = 2\n+++\nbody").await;
        d.handle_key(k(KeyCode::Enter), &tx); // edit row 0
        for _ in 0.."status".len() {
            d.handle_key(k(KeyCode::Backspace), &tx);
        }
        type_into(&mut d, &tx, "state");
        d.handle_key(k(KeyCode::Tab), &tx);
        d.handle_key(k(KeyCode::Tab), &tx);
        for _ in 0.."draft".len() {
            d.handle_key(k(KeyCode::Backspace), &tx);
        }
        type_into(&mut d, &tx, "done");
        d.handle_key(k(KeyCode::Enter), &tx);
        assert!(next_written(&mut rx).await.is_ok());
        assert_eq!(
            d.vault.get_note_text(&d.path).await.unwrap(),
            "+++\na = 1\nstate = \"done\"\nz = 2\n+++\nbody"
        );
    }

    #[tokio::test]
    async fn bare_key_rename_keeps_it_valueless() {
        let (mut d, tx, mut rx) = dialog_with(vec![("due".into(), None)]).await;
        vault_note(&d, "---\ndue:\n---\nbody").await;
        d.handle_key(k(KeyCode::Enter), &tx);
        for _ in 0.."due".len() {
            d.handle_key(k(KeyCode::Backspace), &tx);
        }
        type_into(&mut d, &tx, "deadline");
        d.handle_key(k(KeyCode::Enter), &tx);
        assert!(next_written(&mut rx).await.is_ok());
        assert_eq!(
            d.vault.get_note_text(&d.path).await.unwrap(),
            "---\ndeadline:\n---\nbody"
        );
    }

    #[tokio::test]
    async fn mismatch_offers_store_anyway_which_forces_the_kind() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        // Two other notes make `priority` a number key.
        for n in ["/o1.md", "/o2.md"] {
            d.vault
                .create_note(&VaultPath::note_path_from(n), "+++\npriority = 1\n+++\nx")
                .await
                .unwrap();
        }
        vault_note(&d, "body").await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        type_into(&mut d, &tx, "priority");
        d.handle_key(k(KeyCode::Tab), &tx);
        d.handle_key(k(KeyCode::Tab), &tx);
        type_into(&mut d, &tx, "high");
        d.handle_key(k(KeyCode::Enter), &tx);
        let result = next_written(&mut rx).await;
        assert!(matches!(result, Err(PropertyWriteError::Invalid(_))));
        d.handle_written(&result, &tx);
        assert_eq!(d.store_anyway_kind(), Some(PropertyKind::Text));
        d.press_store_anyway(&tx);
        assert!(next_written(&mut rx).await.is_ok());
        assert_eq!(
            d.vault.get_property(&d.path, "priority").await.unwrap(),
            Some(Some(PropertyValue::Text("high".into())))
        );
    }

    #[tokio::test]
    async fn forced_type_parse_error_has_no_store_anyway() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        vault_note(&d, "body").await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        type_into(&mut d, &tx, "when");
        d.handle_key(k(KeyCode::Tab), &tx); // Type
        for _ in 0..4 {
            d.handle_key(k(KeyCode::Right), &tx); // auto→text→number→bool→date
        }
        d.handle_key(k(KeyCode::Tab), &tx);
        type_into(&mut d, &tx, "not a date");
        d.handle_key(k(KeyCode::Enter), &tx);
        let result = next_written(&mut rx).await;
        d.handle_written(&result, &tx);
        assert!(d.form_error().is_some());
        assert_eq!(d.store_anyway_kind(), None);
    }

    #[tokio::test]
    async fn rename_then_failed_set_reloads_and_keeps_new_key() {
        let (mut d, tx, mut rx) = dialog_with(vec![(
            "status".into(),
            Some(PropertyValue::Text("draft".into())),
        )])
        .await;
        vault_note(&d, "+++\nstatus = \"draft\"\n+++\nbody").await;
        d.handle_key(k(KeyCode::Enter), &tx); // edit row 0
        for _ in 0.."status".len() {
            d.handle_key(k(KeyCode::Backspace), &tx);
        }
        type_into(&mut d, &tx, "when");
        d.handle_key(k(KeyCode::Tab), &tx); // Type
        for _ in 0..4 {
            d.handle_key(k(KeyCode::Right), &tx); // → date
        }
        d.handle_key(k(KeyCode::Tab), &tx);
        for _ in 0.."draft".len() {
            d.handle_key(k(KeyCode::Backspace), &tx);
        }
        type_into(&mut d, &tx, "not a date");
        d.handle_key(k(KeyCode::Enter), &tx);
        // The task reports the changed file to the editor before the result.
        assert_eq!(next_reload(&mut rx).await, d.path);
        let result = next_written(&mut rx).await;
        assert!(result.is_err());
        d.handle_written(&result, &tx);
        // The rename landed even though the set failed.
        assert_eq!(
            d.vault.get_note_text(&d.path).await.unwrap(),
            "+++\nwhen = \"draft\"\n+++\nbody"
        );
        assert!(matches!(d.mode, Mode::Form(_)), "form stays open");
        assert!(d.form_error().is_some());
        assert_eq!(d.form_orig_key().as_deref(), Some("when"));
        assert!(d.loading, "list re-read requested");
    }

    #[tokio::test]
    async fn second_save_while_busy_is_ignored() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        vault_note(&d, "body").await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        type_into(&mut d, &tx, "k");
        d.handle_key(k(KeyCode::Tab), &tx);
        d.handle_key(k(KeyCode::Tab), &tx);
        type_into(&mut d, &tx, "v");
        d.handle_key(k(KeyCode::Enter), &tx);
        d.handle_key(k(KeyCode::Enter), &tx); // double press
        assert!(next_written(&mut rx).await.is_ok());
        let second =
            tokio::time::timeout(std::time::Duration::from_millis(300), next_written(&mut rx))
                .await;
        assert!(second.is_err(), "only one write was spawned");
    }

    #[tokio::test]
    async fn esc_in_form_returns_to_list_without_writing() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        d.handle_key(k(KeyCode::Esc), &tx); // key list closed already → back to list
        assert!(matches!(d.mode, Mode::List));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn arrows_move_between_form_fields() {
        let (mut d, tx, _rx) = dialog_with(vec![]).await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        assert!(d.form_focus() == Some(FormFocus::Key));
        d.handle_key(k(KeyCode::Down), &tx);
        assert!(d.form_focus() == Some(FormFocus::Kind));
        d.handle_key(k(KeyCode::Down), &tx);
        assert!(d.form_focus() == Some(FormFocus::Value));
        d.handle_key(k(KeyCode::Down), &tx);
        assert!(d.form_focus() == Some(FormFocus::Buttons), "Value → Save");
        d.handle_key(k(KeyCode::Up), &tx);
        assert!(d.form_focus() == Some(FormFocus::Value));
        d.handle_key(k(KeyCode::Up), &tx);
        d.handle_key(k(KeyCode::Up), &tx);
        assert!(d.form_focus() == Some(FormFocus::Key));
    }

    #[tokio::test]
    async fn open_key_suggestions_keep_the_arrows() {
        let (mut d, tx, _rx) = dialog_with(vec![]).await;
        d.set_keys(vec!["due".into(), "status".into()]);
        d.handle_key(k(KeyCode::Char('a')), &tx);
        d.handle_key(k(KeyCode::Char('u')), &tx); // "u" matches both → list opens
        d.handle_key(k(KeyCode::Down), &tx);
        assert!(
            d.form_focus() == Some(FormFocus::Key),
            "Down moves in the open list, not to the next field"
        );
        d.handle_key(k(KeyCode::Esc), &tx); // close the list
        d.handle_key(k(KeyCode::Down), &tx);
        assert!(d.form_focus() == Some(FormFocus::Kind));
    }

    #[tokio::test]
    async fn clicking_type_arrows_cycles_kind() {
        let (mut d, tx, _rx) = dialog_with(vec![]).await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        draw(&mut d);
        let (x, y) = d.kind_next_origin().unwrap();
        d.handle_mouse(&mouse(x, y), &tx);
        assert_eq!(d.form_kind(), Some(PropertyKind::Text));
    }

    #[tokio::test]
    async fn buttons_disabled_before_first_load() {
        let vault = temp_vault("props_dialog").await;
        let d = PropertiesDialog::new_unloaded(VaultPath::note_path_from("/n.md"), vault);
        assert!(!d.buttons.is_enabled(BTN_ADD));
        assert!(!d.buttons.is_enabled(BTN_EDIT));
        assert!(!d.buttons.is_enabled(BTN_DELETE));
        assert!(d.buttons.is_enabled(BTN_CLOSE));
    }

    #[tokio::test]
    async fn single_click_on_keyboard_selected_row_edits() {
        let (mut d, tx, _rx) = dialog_with(sample()).await;
        d.handle_key(k(KeyCode::Down), &tx);
        assert_eq!(d.selected, 1);
        draw(&mut d);
        let (x, y) = d.row_origin(1).unwrap();
        d.handle_mouse(&mouse(x + 1, y), &tx);
        assert!(
            matches!(d.mode, Mode::Form(ref f) if f.orig_key.as_deref() == Some("tags")),
            "one click on the selected row opens it"
        );
    }

    #[tokio::test]
    async fn busy_ignores_write_actions() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        draw(&mut d);
        d.busy = true;
        d.sync_buttons();
        for code in [KeyCode::Char('d'), KeyCode::Char('a'), KeyCode::Enter] {
            d.handle_key(k(code), &tx);
            assert!(matches!(d.mode, Mode::List), "{code:?} opened nothing");
        }
        // Click where Add, Edit and Delete rendered while enabled.
        d.busy = false;
        d.sync_buttons();
        draw(&mut d);
        let spots: Vec<(usize, u16, u16)> = [BTN_ADD, BTN_EDIT, BTN_DELETE]
            .into_iter()
            .map(|b| {
                let (x, y) = (0..24u16)
                    .flat_map(|y| (0..80u16).map(move |x| (x, y)))
                    .find(|&(x, y)| d.buttons.hit_at(x, y) == Some(b))
                    .expect("button rendered");
                (b, x, y)
            })
            .collect();
        d.busy = true;
        d.sync_buttons();
        for (b, x, y) in spots {
            d.handle_mouse(&mouse(x, y), &tx);
            assert!(matches!(d.mode, Mode::List), "button {b} opened nothing");
        }
        // A press that bypasses the disabled-button hit test is gated too.
        d.press(BTN_ADD, &tx);
        d.press(BTN_EDIT, &tx);
        d.press(BTN_DELETE, &tx);
        assert!(matches!(d.mode, Mode::List));
        assert!(rx.try_recv().is_err(), "nothing written");
    }

    #[tokio::test]
    async fn confirm_footer_shows_yn_hint() {
        let (mut d, tx, _rx) = dialog_with(sample()).await;
        d.handle_key(k(KeyCode::Char('d')), &tx);
        let theme = Theme::gruvbox_dark();
        let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
        t.draw(|f| d.render(f, f.area(), &theme)).unwrap();
        let buf = t.backend().buffer().clone();
        let text: String = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Delete \"status\"? y/n"), "{text}");
    }

    #[tokio::test]
    async fn auto_on_a_core_list_key_splits_on_commas() {
        let (mut d, tx, mut rx) = dialog_with(vec![]).await;
        vault_note(&d, "body").await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        type_into(&mut d, &tx, "aliases");
        d.handle_key(k(KeyCode::Tab), &tx);
        d.handle_key(k(KeyCode::Tab), &tx);
        type_into(&mut d, &tx, "a, b");
        d.handle_key(k(KeyCode::Enter), &tx);
        assert!(next_written(&mut rx).await.is_ok());
        assert_eq!(
            d.vault.get_property(&d.path, "aliases").await.unwrap(),
            Some(Some(PropertyValue::List(vec!["a".into(), "b".into()])))
        );
    }

    #[tokio::test]
    async fn refused_rename_offers_no_store_anyway() {
        let (mut d, tx, mut rx) = dialog_with(vec![
            ("status".into(), Some(PropertyValue::Text("draft".into()))),
            ("due".into(), Some(PropertyValue::Text("soon".into()))),
        ])
        .await;
        let text = "+++\nstatus = \"draft\"\ndue = \"soon\"\n+++\nbody";
        vault_note(&d, text).await;
        d.handle_key(k(KeyCode::Enter), &tx); // edit row 0 (status)
        for _ in 0.."status".len() {
            d.handle_key(k(KeyCode::Backspace), &tx);
        }
        type_into(&mut d, &tx, "due");
        d.handle_key(k(KeyCode::Enter), &tx);
        let result = next_written(&mut rx).await;
        assert!(result.is_err());
        d.handle_written(&result, &tx);
        assert!(d.form_error().is_some(), "refusal shown");
        assert_eq!(d.store_anyway_kind(), None);
        assert_eq!(d.vault.get_note_text(&d.path).await.unwrap(), text);
    }

    #[tokio::test]
    async fn clicking_the_key_field_puts_the_cursor_at_the_end() {
        let (mut d, tx, _rx) = dialog_with(vec![(
            "status".into(),
            Some(PropertyValue::Text("draft".into())),
        )])
        .await;
        d.handle_key(k(KeyCode::Enter), &tx);
        d.handle_key(k(KeyCode::Home), &tx);
        draw(&mut d);
        let Mode::Form(form) = &d.mode else {
            panic!("form")
        };
        let r = form.key.field_rect().unwrap();
        d.handle_mouse(&mouse(r.x, r.y), &tx);
        type_into(&mut d, &tx, "x");
        let Mode::Form(form) = &d.mode else {
            panic!("form")
        };
        assert_eq!(form.key.value(), "statusx");
    }

    /// Where `buttons` renders button `b` on the 80x24 test frame.
    fn button_spot(buttons: &ButtonRow, b: usize) -> (u16, u16) {
        (0..24u16)
            .flat_map(|y| (0..80u16).map(move |x| (x, y)))
            .find(|&(x, y)| buttons.hit_at(x, y) == Some(b))
            .expect("button rendered")
    }

    fn closed(rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>) -> bool {
        let mut closed = false;
        while let Ok(evt) = rx.try_recv() {
            closed |= matches!(evt, AppEvent::CloseOverlay);
        }
        closed
    }

    /// A write in flight keeps the dialog open: Esc, the Close button and a
    /// direct press are ignored until the result lands, then Esc closes.
    #[tokio::test]
    async fn busy_list_cannot_be_closed_until_the_result_arrives() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        draw(&mut d);
        let (x, y) = button_spot(&d.buttons, BTN_CLOSE);
        d.busy = true;
        d.sync_buttons();
        d.handle_key(k(KeyCode::Esc), &tx);
        d.handle_mouse(&mouse(x, y), &tx);
        d.press(BTN_CLOSE, &tx);
        assert!(!closed(&mut rx), "no CloseOverlay while saving");
        d.handle_written(&Ok("property removed".into()), &tx);
        closed(&mut rx);
        d.handle_key(k(KeyCode::Esc), &tx);
        assert!(closed(&mut rx), "Esc closes once the write landed");
    }

    /// The form's Cancel (Esc, the button by key or click) is ignored while
    /// its save is in flight: the form stays up until the result.
    #[tokio::test]
    async fn busy_form_cancel_is_ignored_until_the_result_arrives() {
        let (mut d, tx, mut rx) = dialog_with(sample()).await;
        d.handle_key(k(KeyCode::Char('a')), &tx);
        draw(&mut d);
        let Mode::Form(form) = &d.mode else {
            panic!("form")
        };
        let (x, y) = button_spot(&form.buttons, 1);
        d.busy = true;
        d.sync_buttons();
        d.handle_key(k(KeyCode::Esc), &tx);
        assert!(matches!(d.mode, Mode::Form(_)), "Esc ignored while saving");
        d.handle_mouse(&mouse(x, y), &tx);
        assert!(matches!(d.mode, Mode::Form(_)), "Cancel click ignored");
        d.run_form_action(FormAction::Cancel, &tx);
        assert!(matches!(d.mode, Mode::Form(_)), "Cancel ignored");
        assert!(!closed(&mut rx));
        d.handle_written(&Err(PropertyWriteError::Other("nope".into())), &tx);
        d.handle_key(k(KeyCode::Esc), &tx);
        assert!(matches!(d.mode, Mode::List), "Esc cancels once idle");
        d.handle_key(k(KeyCode::Esc), &tx);
        assert!(closed(&mut rx), "and the next Esc closes the dialog");
    }
}
