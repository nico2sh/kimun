//! The **properties dialog** (M7a G2): the open note's frontmatter
//! properties, listed as key / type / value rows, added, edited, renamed and
//! deleted. One kit `Dialog` switches between the list and the add/edit
//! form (the TUI's one dialog); a delete asks first in a kit `AlertDialog`.
//! Every rule (validation, no-op edits, the store-anyway retry, the
//! rename-then-failed-set case) and every write is `logic::properties`';
//! the vault is changed by core only. Doors: leader `n p` and the palette,
//! the Footer's property count, the editor's right-click menu.
//!
//! **This dialog writes to the open note.** The rules that keep typed text
//! and the written frontmatter from overwriting each other:
//!
//! - **Flushed before it opens.** On the open note the autosave tick stops,
//!   the editor takes no input and the buffer is flushed; a flush that fails
//!   (or finds another one pending, or leaves the buffer dirty) refuses to
//!   open the dialog (`save failed — properties not opened`, the TUI's
//!   words), and the tick and the input come back.
//! - **One writer at a time.** A write holds the note
//!   ([`Workspace::note_held`]): the tick, the Save chord, opening another
//!   note and file operations wait until it and the reload after it are
//!   done. The dialog's buttons wait too, and Escape, Close and the backdrop
//!   do nothing while it runs. A write is refused (and nothing written)
//!   when the open note has unsaved edits after all.
//! - **The reload is the window's, not the dialog's.** The write job sends
//!   `NoteReloadFromDisk` itself whenever the file changed; the workspace
//!   reloads the clean buffer from disk (or, with unsaved edits, keeps them
//!   and says so), so closing the dialog during a write still reloads.
//! - **Stale results are dropped.** Loads and write results reach the open
//!   properties dialog only, and only for its own note.

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, IndexPath};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role, ScrollStrategy,
    SharedString, StatefulInteractiveElement, Styled, Subscription, UniformListScrollHandle,
    WeakEntity, Window, div, rems, uniform_list,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_core::note::{PropertyEntry, PropertyKind};
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::message::{AppTx, OverlayData};
use kimun_notes::logic::properties::{
    self as props, FormError, KIND_CHOICES, PropertyForm, PropertyWriteError, WritePlan,
};
use kimun_notes::logic::session::{NoteRead, ReloadOutcome, ReloadStep};
use tokio::task::JoinError;

use super::Workspace;
use super::file_dialogs::confirm_delete;
use super::key_suggestions::KeySuggestions;
use super::modal::{self, ModalKind, Slot};
use super::route::{NO_NOTE, NO_WORKSPACE};
use crate::input;
use crate::theme;

/// Element ids, for tests.
pub(crate) const TABLE_ID: &str = "properties-table";
pub(crate) const ADD_ID: &str = "properties-add";
pub(crate) const EDIT_ID: &str = "properties-edit";
pub(crate) const DELETE_ID: &str = "properties-delete";
pub(crate) const CLOSE_ID: &str = "properties-close";
pub(crate) const LIST_ERROR_ID: &str = "properties-error";
pub(crate) const KEY_INPUT_ID: &str = "property-key";
pub(crate) const VALUE_INPUT_ID: &str = "property-value";
pub(crate) const KIND_SELECT_ID: &str = "property-type";
pub(crate) const SAVE_ID: &str = "property-save";
pub(crate) const FORM_CANCEL_ID: &str = "property-cancel";
pub(crate) const STORE_ANYWAY_ID: &str = "property-store-anyway";
pub(crate) const FORM_ERROR_ID: &str = "property-error";

/// A row of the table, by its key (stable while rows come and go).
pub(crate) fn row_id(key: &str) -> SharedString {
    format!("property-row:{key}").into()
}

/// A key suggestion under the Key field.
pub(crate) fn suggestion_id(key: &str) -> SharedString {
    format!("property-suggestion:{key}").into()
}

/// The open was refused: the open note could not be flushed first (the
/// TUI's words).
pub(crate) const REFUSED: &str = "save failed — properties not opened";
/// A reload found unsaved typing and kept it (the TUI's words).
pub(crate) const KEPT_DIRTY: &str = "properties changed on disk — reopen the note to see them";
/// A reload's read failed: the buffer still holds the old text (the TUI's
/// words).
pub(crate) const RELOAD_FAILED: &str = "reload failed — reopen the note before editing";

/// The dialog's width, in rems.
const DIALOG_REMS: f32 = 36.;
/// A table row's height, in rems.
const ROW_REMS: f32 = 1.75;
/// The most rows the table shows before it scrolls.
const VISIBLE_ROWS: usize = 8;

/// What a confirmed form or delete asks the workspace to write.
pub(crate) enum PropertyJob {
    Write(WritePlan),
    Delete(String),
}

/// A kind as the Type select and the table say it (`auto`, `text`, …).
fn kind_label(kind: Option<PropertyKind>) -> &'static str {
    kind.map_or("auto", PropertyKind::as_str)
}

/// The Type select's choice for `label`.
fn kind_of(label: &str) -> Option<PropertyKind> {
    KIND_CHOICES
        .iter()
        .copied()
        .find(|kind| kind_label(*kind) == label)
        .flatten()
}

/// One table row: its key and its three cells, built once per load.
struct Row {
    key: String,
    cells: [SharedString; 3],
}

/// The add/edit form: the logic field model plus its widgets.
struct FormView {
    fields: PropertyForm,
    key: Entity<InputState>,
    value: Entity<InputState>,
    kind: Entity<SelectState<Vec<&'static str>>>,
    /// Known keys matching the Key field (not itself), and the one Up/Down
    /// is on; Enter takes it.
    suggestions: KeySuggestions,
    _subscriptions: Vec<Subscription>,
}

/// The dialog's view. See the module docs.
pub(crate) struct PropertiesDialog {
    path: VaultPath,
    vault: Arc<NoteVault>,
    tx: AppTx,
    workspace: WeakEntity<Workspace>,
    entries: Vec<PropertyEntry>,
    rows: Vec<Row>,
    /// The frontmatter would not parse: read-only, the reason shown.
    load_error: Option<String>,
    /// Nothing may be written in this vault (names differing by case).
    locked: Option<String>,
    loading: bool,
    /// A write is in flight: every write and every way out waits for it.
    busy: bool,
    selected: usize,
    known_keys: Vec<String>,
    /// (old, new) of the rename sent with the write in flight: if the set
    /// after it fails, the rename landed anyway.
    pending_rename: Option<(String, String)>,
    /// A failed delete (the list's error row).
    list_error: Option<SharedString>,
    /// The list's title (`Properties: <note>`), made once.
    list_title: SharedString,
    form: Option<FormView>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
}

impl PropertiesDialog {
    fn new(
        path: VaultPath,
        vault: Arc<NoteVault>,
        tx: AppTx,
        workspace: WeakEntity<Workspace>,
        locked: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let list_title = format!("Properties: {}", path.get_clean_name()).into();
        let mut dialog = Self {
            list_title,
            path,
            vault,
            tx,
            workspace,
            entries: Vec::new(),
            rows: Vec::new(),
            load_error: None,
            locked,
            loading: true,
            busy: false,
            selected: 0,
            known_keys: Vec::new(),
            pending_rename: None,
            list_error: None,
            form: None,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
        };
        dialog.reload();
        dialog
    }

    /// Read the note's properties and the vault's keys again (both report
    /// on the app channel).
    fn reload(&mut self) {
        self.loading = true;
        let (vault, path, tx) = (self.vault.clone(), self.path.clone(), self.tx.clone());
        tokio::spawn(async move {
            let _ = props::load(&vault, path, &tx).await;
        });
        let (vault, tx) = (self.vault.clone(), self.tx.clone());
        tokio::spawn(async move {
            let _ = props::load_keys(&vault, &tx).await;
        });
    }

    fn read_only(&self) -> bool {
        self.load_error.is_some() || self.locked.is_some()
    }

    /// Whether a write may start: loaded, writable, nothing in flight.
    fn can_write(&self) -> bool {
        !self.busy && !self.read_only() && !self.loading
    }

    /// The dialog's title: the note, or what the form does.
    fn title(&self) -> SharedString {
        match &self.form {
            Some(form) if form.fields.orig_key.is_some() => "Edit property".into(),
            Some(_) => "Add property".into(),
            None => self.list_title.clone(),
        }
    }

    // ---- Results ------------------------------------------------------------

    /// A result from the app channel. Loads and writes for another note are
    /// stale and dropped. True when it was this dialog's.
    pub(crate) fn on_data(
        &mut self,
        data: &OverlayData,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mine = match data {
            OverlayData::PropertiesLoaded { path, result } if *path == self.path => {
                self.loaded(result);
                true
            }
            OverlayData::PropertyKeysLoaded(keys) => {
                self.known_keys = keys.clone();
                self.refresh_suggestions(cx);
                true
            }
            OverlayData::PropertyWritten { path, result } if *path == self.path => {
                self.written(result, window, cx);
                true
            }
            _ => false,
        };
        cx.notify();
        mine
    }

    /// The vault's property keys, once they have loaded (tests).
    #[cfg(test)]
    pub(crate) fn known_keys(&self) -> &[String] {
        &self.known_keys
    }

    fn loaded(&mut self, result: &Result<Vec<PropertyEntry>, String>) {
        self.loading = false;
        match result {
            Ok(entries) => {
                self.entries = entries.clone();
                self.rows = entries
                    .iter()
                    .map(|entry| {
                        let (key, kind, value) = props::row_cells(entry);
                        Row {
                            key: entry.0.clone(),
                            cells: [key.into(), kind.into(), value.into()],
                        }
                    })
                    .collect();
                self.load_error = None;
                self.selected = self.selected.min(self.rows.len().saturating_sub(1));
            }
            Err(e) => {
                self.entries.clear();
                self.rows.clear();
                self.load_error = Some(e.clone());
            }
        }
    }

    /// A write finished (the reload is the workspace's: the job sent it).
    fn written(
        &mut self,
        result: &Result<String, PropertyWriteError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        let renamed = self.pending_rename.take();
        match result {
            Ok(_) => {
                self.close_form(window, cx);
                self.reload();
            }
            Err(PropertyWriteError::Malformed(msg)) => {
                self.close_form(window, cx);
                self.load_error = Some(msg.clone());
            }
            Err(e) => {
                if let Some((_, new)) = renamed {
                    // The rename landed before the set failed: the list
                    // reloads, and the form edits the key under its new name.
                    self.reload();
                    if let Some(form) = self.form.as_mut() {
                        form.fields.orig_key = Some(new);
                    }
                }
                match self.form.as_mut() {
                    Some(form) => {
                        Self::sync_fields(form, cx);
                        form.fields.show_write_error(e);
                    }
                    None => self.list_error = Some(e.message().to_string().into()),
                }
            }
        }
    }

    // ---- The list -------------------------------------------------------------

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = ix.min(self.rows.len() - 1);
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn open_add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_write() {
            return;
        }
        self.open_form(PropertyForm::add(), window, cx);
    }

    fn open_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_write() {
            return;
        }
        let Some((key, value)) = self.entries.get(self.selected) else {
            return;
        };
        let fields = PropertyForm::edit(key, value.as_ref());
        self.open_form(fields, window, cx);
    }

    /// `Delete…`: ask first (a kit `AlertDialog` over this one).
    fn start_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_write() {
            return;
        }
        let Some(key) = self.rows.get(self.selected).map(|r| r.key.clone()) else {
            return;
        };
        self.list_error = None;
        let view = cx.entity().downgrade();
        confirm_delete(
            window,
            cx,
            format!("Delete \"{key}\"?"),
            "It is removed from the note's frontmatter.",
            move |alert| {
                let view = view.clone();
                let key = key.clone();
                alert.on_ok(move |_, window, cx| {
                    let key = key.clone();
                    view.update(cx, |dialog, cx| dialog.confirm_delete(key, window, cx))
                        .ok();
                    true
                })
            },
        );
        cx.notify();
    }

    /// The delete was confirmed: remove `key` through the workspace.
    fn confirm_delete(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_write() {
            return;
        }
        match self.request(PropertyJob::Delete(key), cx) {
            Ok(()) => self.busy = true,
            Err(why) => self.list_error = Some(why.into()),
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Ask the workspace to run `job` on this note (it may refuse: say why).
    fn request(&self, job: PropertyJob, cx: &mut Context<Self>) -> Result<(), String> {
        let path = self.path.clone();
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.request_property_write(path, job, cx)
            })
            .unwrap_or_else(|_| Err("The window is closing.".to_string()))
    }

    fn on_table_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(KeyInput { key, mods }) = input::key_down_input(event) else {
            return;
        };
        if mods != Mods::NONE && mods != Mods::SHIFT {
            return;
        }
        match key {
            Key::Up => self.select(self.selected.saturating_sub(1), cx),
            Key::Down => self.select(self.selected + 1, cx),
            Key::Char(c) => match c.to_ascii_lowercase() {
                'a' => self.open_add(window, cx),
                'e' => self.open_edit(window, cx),
                'd' => self.start_delete(window, cx),
                _ => return,
            },
            Key::Delete => self.start_delete(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    // ---- The form -------------------------------------------------------------

    fn open_form(&mut self, fields: PropertyForm, window: &mut Window, cx: &mut Context<Self>) {
        self.list_error = None;
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Name")
                .default_value(fields.key.clone())
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Value")
                .default_value(fields.value.clone())
        });
        let labels: Vec<&'static str> = KIND_CHOICES.iter().map(|k| kind_label(*k)).collect();
        let chosen = KIND_CHOICES
            .iter()
            .position(|k| *k == fields.kind())
            .unwrap_or(0);
        let kind = cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(chosen)), window, cx));
        let subscriptions = vec![
            cx.subscribe(&key, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.key_edited(cx);
                }
            }),
            cx.subscribe(&value, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(form) = this.form.as_mut()
                {
                    Self::sync_fields(form, cx);
                    form.fields.clear_feedback();
                    cx.notify();
                }
            }),
            cx.subscribe(
                &kind,
                |this, _, event: &SelectEvent<Vec<&'static str>>, cx| {
                    let SelectEvent::Confirm(choice) = event;
                    if let Some(form) = this.form.as_mut() {
                        form.fields.set_kind(choice.and_then(kind_of));
                        cx.notify();
                    }
                },
            ),
        ];
        // Edit starts on the value (the key rarely changes); Add on the key.
        let focus = if fields.orig_key.is_some() {
            value.clone()
        } else {
            key.clone()
        };
        self.form = Some(FormView {
            fields,
            key,
            value,
            kind,
            suggestions: KeySuggestions::default(),
            _subscriptions: subscriptions,
        });
        focus.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Back to the list, the table focused.
    fn close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Copy the inputs' text into the field model.
    fn sync_fields(form: &mut FormView, cx: &App) {
        form.fields.key = form.key.read(cx).value().to_string();
        form.fields.value = form.value.read(cx).value().to_string();
    }

    fn key_edited(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.form.as_mut() {
            Self::sync_fields(form, cx);
            form.fields.clear_feedback();
        }
        self.refresh_suggestions(cx);
    }

    /// The known keys containing what the Key field holds (case-insensitive,
    /// prefix matches first), not the field's own text.
    fn refresh_suggestions(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        form.suggestions.refresh(&self.known_keys, &form.fields.key);
        cx.notify();
    }

    /// Put suggestion `ix` in the Key field.
    fn accept_suggestion(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let Some(key) = form.suggestions.get(ix).cloned() else {
            return;
        };
        // `set_value` emits no change: keep the model in step here.
        form.key
            .update(cx, |input, cx| input.set_value(key, window, cx));
        Self::sync_fields(form, cx);
        form.fields.clear_feedback();
        self.refresh_suggestions(cx);
    }

    fn key_focused(&self, window: &Window, cx: &App) -> bool {
        self.form
            .as_ref()
            .is_some_and(|form| form.key.read(cx).focus_handle(cx).is_focused(window))
    }

    /// Up/Down in the Key field: walk the suggestions. False when there is
    /// nothing to walk (the key goes on).
    fn step_suggestion(&mut self, down: bool, window: &Window, cx: &mut Context<Self>) -> bool {
        if !self.key_focused(window, cx) {
            return false;
        }
        let Some(form) = self.form.as_mut() else {
            return false;
        };
        if !form.suggestions.step(down) {
            return false;
        }
        cx.notify();
        true
    }

    /// Save the form (`forced`: the store-anyway retry). Validation and
    /// no-ops are `plan_write`'s; an unchanged edit goes back to the list.
    fn submit(
        &mut self,
        forced: Option<PropertyKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_write() {
            return;
        }
        let Some(form) = self.form.as_mut() else {
            return;
        };
        Self::sync_fields(form, cx);
        let plan = match props::plan_write(&self.entries, &form.fields, forced) {
            Ok(plan) => plan,
            Err(FormError::Unchanged) => {
                self.close_form(window, cx);
                return;
            }
            Err(e) => {
                form.fields.show_refusal(&e);
                cx.notify();
                return;
            }
        };
        form.fields.clear_feedback();
        let rename = plan.rename();
        match self.request(PropertyJob::Write(plan), cx) {
            Ok(()) => {
                self.busy = true;
                self.pending_rename = rename;
            }
            Err(why) => {
                if let Some(form) = self.form.as_mut() {
                    form.fields.error = Some(why);
                }
            }
        }
        cx.notify();
    }

    fn store_anyway(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.form.as_ref().and_then(|f| f.fields.store_anyway);
        if let Some(kind) = kind {
            self.submit(Some(kind), window, cx);
        }
    }

    // ---- Enter and Escape -------------------------------------------------------

    /// Enter: edit the selected row, take the highlighted suggestion, or
    /// save the form.
    pub(crate) fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.form.as_ref() {
            None => self.open_edit(window, cx),
            Some(form) => match form.suggestions.highlighted() {
                Some(ix) => self.accept_suggestion(ix, window, cx),
                None => self.submit(None, window, cx),
            },
        }
    }

    /// Escape (and the backdrop): a highlighted suggestion lets go, the form
    /// goes back to the list, the list closes. Nothing while a write runs.
    /// True when the dialog should close.
    pub(crate) fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        match self.form.as_mut() {
            Some(form) if form.suggestions.highlighted().is_some() => {
                form.suggestions.release();
                cx.notify();
                false
            }
            Some(_) => {
                self.close_form(window, cx);
                false
            }
            None => true,
        }
    }

    // ---- Tests ------------------------------------------------------------------

    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<[String; 3]> {
        self.rows
            .iter()
            .map(|r| r.cells.clone().map(|c| c.to_string()))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn is_loading(&self) -> bool {
        self.loading
    }

    #[cfg(test)]
    pub(crate) fn is_busy(&self) -> bool {
        self.busy
    }

    #[cfg(test)]
    pub(crate) fn in_form(&self) -> bool {
        self.form.is_some()
    }

    #[cfg(test)]
    pub(crate) fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn form_error(&self) -> Option<String> {
        self.form.as_ref()?.fields.error.clone()
    }

    #[cfg(test)]
    pub(crate) fn store_anyway_kind(&self) -> Option<PropertyKind> {
        self.form.as_ref()?.fields.store_anyway
    }

    #[cfg(test)]
    pub(crate) fn key_text(&self, cx: &App) -> String {
        self.form
            .as_ref()
            .map(|f| f.key.read(cx).value().to_string())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn suggestions(&self) -> Vec<String> {
        self.form
            .as_ref()
            .map(|f| f.suggestions.shown().to_vec())
            .unwrap_or_default()
    }

    /// Choose `kind` in the Type select, as a pick from its menu does.
    #[cfg(test)]
    pub(crate) fn choose_kind(
        &mut self,
        kind: Option<PropertyKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.form.as_mut() {
            let label = kind_label(kind);
            form.kind.update(cx, |select, cx| {
                select.set_selected_value(&label, window, cx)
            });
            form.fields.set_kind(kind);
        }
    }

    #[cfg(test)]
    pub(crate) fn chosen_kind(&self, cx: &App) -> Option<&'static str> {
        let form = self.form.as_ref()?;
        form.kind.read(cx).selected_value().copied()
    }
}

impl Focusable for PropertiesDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

// ---- Rendering ------------------------------------------------------------------

impl PropertiesDialog {
    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let row = &self.rows[ix];
        let selected = ix == self.selected;
        let hover = theme::hover_background(t);
        let [key, kind, value] = row.cells.clone();
        div()
            .id(row_id(&row.key))
            .test_support()
            .role(Role::Row)
            .aria_label(key.clone())
            .aria_selected(selected)
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .w_full()
            .h(rems(ROW_REMS))
            .px_2()
            .rounded_md()
            .text_sm()
            .when(selected, |row| row.bg(theme::selected_background(t)))
            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
            .child(
                div()
                    .w(rems(9.))
                    .flex_none()
                    .truncate()
                    .text_color(theme::foreground(t))
                    .child(key),
            )
            .child(
                div()
                    .w(rems(5.5))
                    .flex_none()
                    .truncate()
                    .text_color(theme::muted(t))
                    .child(kind),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme::foreground(t))
                    .child(value),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                window.focus(&this.focus, cx);
                // A second click on the selected row edits it (the TUI's).
                let again = this.selected == ix;
                this.select(ix, cx);
                if again || event.click_count() >= 2 {
                    this.open_edit(window, cx);
                }
            }))
            .into_any_element()
    }

    fn render_list(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let muted = theme::muted(t);
        let note = |text: SharedString| {
            div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(muted)
                .child(text)
                .into_any_element()
        };
        let body = if self.loading && self.rows.is_empty() && self.load_error.is_none() {
            note("Loading…".into())
        } else if let Some(error) = self.load_error.as_ref() {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .px_2()
                .py_2()
                .text_sm()
                .child(
                    div()
                        .text_color(theme::foreground(t))
                        .child("Read-only: fix the frontmatter in the editor."),
                )
                .child(
                    div()
                        .id(LIST_ERROR_ID)
                        .role(Role::Alert)
                        .aria_label(SharedString::from(error.clone()))
                        .text_color(theme::error(t))
                        .child(error.clone()),
                )
                .into_any_element()
        } else if self.rows.is_empty() {
            note("No properties. Press A or choose Add….".into())
        } else {
            let count = self.rows.len();
            let list = uniform_list(
                TABLE_ID,
                count,
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .filter(|ix| *ix < this.rows.len())
                        .map(|ix| this.render_row(ix, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full();
            div()
                .h(rems(ROW_REMS * count.min(VISIBLE_ROWS) as f32))
                .child(list)
                .into_any_element()
        };
        let focused = self.focus.is_focused(window);
        let header = |text: &'static str| div().text_xs().text_color(muted).child(text);
        div()
            .id("properties-list")
            .test_support()
            .role(Role::Table)
            .aria_label("Properties")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_table_key))
            .flex()
            .flex_col()
            .gap_1()
            .p_1()
            .rounded_md()
            .border_1()
            .border_color(if focused {
                theme::focus_border(t)
            } else {
                theme::divider(t)
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_3()
                    .px_2()
                    .child(header("Key").w(rems(9.)).flex_none())
                    .child(header("Type").w(rems(5.5)).flex_none())
                    .child(header("Value").flex_1()),
            )
            .child(body)
            .into_any_element()
    }

    fn render_list_mode(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let writable = self.can_write();
        let has_row = writable && !self.rows.is_empty();
        let deleting = self.busy && self.form.is_none();
        let hint = |key: char, text: &'static str| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .child(Kbd::new(input::keystroke(KeyInput::plain(Key::Char(key)))))
                .child(text)
        };
        let view = cx.entity().downgrade();
        let (add, edit, delete) = (view.clone(), view.clone(), view);
        let close = self.workspace.clone();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .when_some(self.locked.clone(), |col, why| {
                col.child(div().text_sm().text_color(theme::muted(t)).child(why))
            })
            .child(self.render_list(window, cx))
            .when(writable && !self.rows.is_empty(), |col| {
                col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_3()
                        .text_xs()
                        .text_color(theme::muted(t))
                        .child(hint('A', "add"))
                        .child(hint('E', "edit"))
                        .child(hint('D', "delete")),
                )
            })
            .when_some(self.list_error.clone(), |col, error| {
                col.child(
                    div()
                        .id(LIST_ERROR_ID)
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .text_sm()
                        .text_color(theme::error(t))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new(ADD_ID)
                            .label("Add…")
                            .disabled(!writable)
                            .on_click(move |_: &ClickEvent, window, cx| {
                                add.update(cx, |d, cx| d.open_add(window, cx)).ok();
                            }),
                    )
                    .child(
                        Button::new(EDIT_ID)
                            .label("Edit…")
                            .disabled(!has_row)
                            .on_click(move |_: &ClickEvent, window, cx| {
                                edit.update(cx, |d, cx| d.open_edit(window, cx)).ok();
                            }),
                    )
                    .child(
                        Button::new(DELETE_ID)
                            .label("Delete…")
                            .loading(deleting)
                            .disabled(!has_row)
                            .on_click(move |_: &ClickEvent, window, cx| {
                                delete.update(cx, |d, cx| d.start_delete(window, cx)).ok();
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new(CLOSE_ID)
                            .label("Close")
                            .disabled(self.busy)
                            .on_click(move |_: &ClickEvent, window, cx| {
                                // The workspace refuses while a write runs.
                                close
                                    .update(cx, |workspace, cx| {
                                        workspace.dismiss_properties(window, cx)
                                    })
                                    .ok();
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_form(
        &self,
        form: &FormView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let busy = self.busy;
        let label = |text: &'static str| div().text_xs().text_color(theme::muted(t)).child(text);
        let field = || div().flex().flex_col().gap_1();
        let suggestions = if self.key_focused(window, cx) {
            form.suggestions.render(
                "property-suggestions",
                "Known keys",
                suggestion_id,
                t,
                cx,
                |this: &mut Self, ix, window, cx| this.accept_suggestion(ix, window, cx),
            )
        } else {
            None
        };
        let current = form
            .fields
            .current_kind
            .map(|kind| format!("currently {kind}"));
        let error = form.fields.error.clone().map(SharedString::from);
        let store = form.fields.store_anyway;
        let view = cx.entity().downgrade();
        let (save, cancel, anyway) = (view.clone(), view.clone(), view);
        div()
            .flex()
            .flex_col()
            .gap_3()
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                this.confirm(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                // In the form, Escape never closes: back one layer.
                this.escape(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                if this.step_suggestion(false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                if this.step_suggestion(true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                field()
                    .child(label("Key"))
                    .child(
                        Input::new(&form.key)
                            .id(KEY_INPUT_ID)
                            .aria_label("Key")
                            .disabled(busy),
                    )
                    .children(suggestions),
            )
            .child(
                field()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(label("Type"))
                            .when_some(current, |row, current| {
                                row.child(
                                    div().text_xs().text_color(theme::muted(t)).child(current),
                                )
                            }),
                    )
                    .child(
                        Select::new(&form.kind)
                            .id(KIND_SELECT_ID)
                            .accessibility_label("Type")
                            .disabled(busy),
                    ),
            )
            .child(
                field()
                    .child(label("Value"))
                    .child(
                        Input::new(&form.value)
                            .id(VALUE_INPUT_ID)
                            .aria_label("Value")
                            .disabled(busy),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(t))
                            .child("Separate list items with commas."),
                    ),
            )
            .when_some(error, |col, error| {
                col.child(
                    div()
                        .id(FORM_ERROR_ID)
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .text_sm()
                        .text_color(theme::error(t))
                        .child(error),
                )
            })
            .when_some(store, |col, kind| {
                col.child(
                    div().child(
                        Button::new(STORE_ANYWAY_ID)
                            .label(format!("Store as {kind} anyway"))
                            .disabled(busy)
                            .on_click(move |_: &ClickEvent, window, cx| {
                                anyway.update(cx, |d, cx| d.store_anyway(window, cx)).ok();
                            }),
                    ),
                )
            })
            .child(
                modal::footer_row()
                    .child(
                        Button::new(FORM_CANCEL_ID)
                            .label("Cancel")
                            .disabled(busy)
                            .on_click(move |_: &ClickEvent, window, cx| {
                                cancel
                                    .update(cx, |d, cx| {
                                        if !d.busy {
                                            d.close_form(window, cx);
                                        }
                                    })
                                    .ok();
                            }),
                    )
                    .child(
                        Button::new(SAVE_ID)
                            .primary()
                            .label("Save")
                            .loading(busy)
                            .disabled(busy)
                            .on_click(move |_: &ClickEvent, window, cx| {
                                save.update(cx, |d, cx| d.submit(None, window, cx)).ok();
                            }),
                    ),
            )
            .into_any_element()
    }
}

impl Render for PropertiesDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.form.as_ref() {
            Some(form) => self.render_form(form, window, cx),
            None => self.render_list_mode(window, cx),
        };
        div()
            .id("properties-dialog")
            .test_support()
            .flex()
            .flex_col()
            .child(body)
    }
}

// ---- The workspace's side ---------------------------------------------------------

impl Workspace {
    /// Whether something holds the open note so nothing else may write it:
    /// a file operation, a property write, the reload after one, or the
    /// flush before the properties dialog opens, a vault change
    /// ([`reopen`](super::reopen)). The tick, the Save chord,
    /// opening another note and file operations wait while it does.
    pub(crate) fn note_held(&self) -> bool {
        self.file_task.is_some()
            || self.property_write.is_some()
            || self.note_reload.is_some()
            || self.properties_opening.is_some()
            || self.attachment_flushing.is_some()
            || self.search_flush.is_some()
            || self.updates.flushing.is_some()
            || self.reopen_running()
    }

    /// The Footer's property count and the like: the dialog for the open
    /// note, or say there is none.
    pub(crate) fn open_properties_for_open_note(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.notes.as_ref().and_then(|h| h.note.clone()) {
            Some(note) => self.open_properties(note, window, cx),
            None => self.flash(NO_NOTE, cx),
        }
    }

    /// Open the properties dialog for `path`. On the open note, flush it
    /// first and refuse when that fails (module docs). Refused while a modal
    /// is open.
    pub(crate) fn open_properties(
        &mut self,
        path: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) || self.properties_opening.is_some() {
            return;
        }
        if self.notes.is_none() {
            self.flash(NO_WORKSPACE, cx);
            return;
        }
        self.cancel_leader(cx);
        let read_only = self.status.read_only;
        let locked = read_only.then(|| {
            "Read-only: this vault has names that differ only by case, so nothing is written."
                .to_string()
        });
        let Some(host) = self.notes.as_ref() else {
            return;
        };
        let touches = host.note.as_ref().is_some_and(|note| note.is_like(&path));
        if !touches {
            self.show_properties(path, locked, window, cx);
            return;
        }
        if let Some(block) = self.flush_block(cx) {
            tracing::warn!("properties of {path} not opened: {}", block.reason());
            self.flash_error(REFUSED, cx);
            return;
        }
        // Read-only: nothing can be written, so nothing to flush; the dialog
        // only shows the properties.
        if read_only {
            self.show_properties(path, locked, window, cx);
            return;
        }
        // No tick from here: the flush is the only write until the dialog
        // is up (or refused).
        let Some(flush) = self.begin_note_flush(cx) else {
            tracing::warn!("properties of {path} not opened: a flush is outstanding");
            self.flash_error(REFUSED, cx);
            return;
        };
        self.properties_opening = Some(cx.spawn_in(window, async move |this, cx| {
            let flushed = tokio::spawn(flush.run()).await;
            this.update_in(cx, |workspace, window, cx| {
                workspace.properties_flushed(path, flushed, window, cx)
            })
            .ok();
        }));
        cx.notify();
    }

    /// The flush before the dialog ended: open it when everything is
    /// written, else refuse (the note keeps its edits, its tick and the
    /// keyboard).
    fn properties_flushed(
        &mut self,
        path: VaultPath,
        flushed: Result<kimun_notes::logic::session::FlushResult, JoinError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.properties_opening = None;
        let outcome = self.apply_flush(flushed, cx);
        let problem = self.flush_failure(&outcome, cx);
        self.restart_tick_unless_read_only();
        self.block_input(false, cx);
        if let Some(problem) = problem {
            tracing::warn!("properties of {path} not opened: {problem}");
            self.flash_error(REFUSED, cx);
        } else if self.notes.as_ref().is_some_and(|h| h.is_open_wanted()) {
            // Another note was asked for meanwhile: it opens instead.
        } else if !modal::dialog_open(window, cx) {
            self.show_properties(path, None, window, cx);
        }
        self.file_op_ended(cx);
    }

    /// Put the dialog for `path` in the modal slot; it loads the
    /// properties and the vault's keys itself.
    fn show_properties(
        &mut self,
        path: VaultPath,
        locked: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) {
            return;
        }
        let (Some(vault), Some(tx)) = (self.vault(), self.tx.clone()) else {
            return;
        };
        let workspace = cx.entity().downgrade();
        let view = cx.new(|cx| PropertiesDialog::new(path, vault, tx, workspace, locked, cx));
        self.properties = Some(view.downgrade());
        self.show_modal(
            Slot::Own(ModalKind::Properties),
            DIALOG_REMS,
            modal::focus_view(view.clone()),
            move |dialog, _, cx| {
                let (confirm, cancel) = (view.clone(), view.clone());
                dialog
                    .title(view.read(cx).title())
                    // Enter: edit the selected row, or save the form.
                    .on_ok(move |_, window, cx| {
                        confirm.update(cx, |dialog, cx| dialog.confirm(window, cx));
                        false
                    })
                    // Escape and the backdrop: back one layer; close from
                    // the list; nothing while a write runs.
                    .on_cancel(move |_, window, cx| {
                        cancel.update(cx, |dialog, cx| dialog.escape(window, cx))
                    })
                    .child(view.clone())
            },
            window,
            cx,
        );
    }

    /// The dialog's Close: close it, unless a write runs.
    pub(crate) fn dismiss_properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.property_write.is_some() {
            return;
        }
        if self.open_modal(window, cx) == Some(ModalKind::Properties) {
            self.close_modal(window, cx);
        }
    }

    /// Run `job` on `path` (the dialog validated it): the write, through
    /// core, holding the note until its result and the reload are in.
    /// Refused, with the reason, when another write runs or the open note
    /// could be overwritten by it.
    pub(crate) fn request_property_write(
        &mut self,
        path: VaultPath,
        job: PropertyJob,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.status.read_only {
            return Err("Read-only: nothing is written in this vault.".into());
        }
        if self.note_held() {
            return Err("Another change to the note is still being written.".into());
        }
        let dirty = self.editor_dirty(cx);
        let (Some(host), Some(tx)) = (self.notes.as_ref(), self.tx.clone()) else {
            return Err(NO_WORKSPACE.into());
        };
        if host.note.as_ref().is_some_and(|note| note.is_like(&path)) {
            if host.is_switching() || host.is_open_wanted() {
                return Err("The note is still opening. Try again in a moment.".into());
            }
            // The flush before the dialog opened left the buffer clean and
            // the dialog keeps the keyboard; should either have changed, the
            // write would meet the next save. Nothing is written then.
            if dirty || host.session.is_autosave_in_flight() {
                return Err(
                    "The note has unsaved edits: close this dialog and save it first.".into(),
                );
            }
        }
        let vault = host.session.vault().clone();
        let target = path.clone();
        let job = tokio::spawn(async move {
            match job {
                PropertyJob::Write(plan) => props::run_write(&vault, target, plan, &tx).await,
                PropertyJob::Delete(key) => props::run_delete(&vault, target, &key, &tx).await,
            }
        });
        let held = path.clone();
        let task = cx.spawn(async move |this, cx| {
            // The result itself comes on the app channel (it releases the
            // hold); only a job that never finished is handled here.
            if let Err(e) = job.await {
                this.update(cx, |workspace, cx| {
                    tracing::error!("property write for {held} failed to finish: {e}");
                    workspace.property_write = None;
                    workspace
                        .flash_error(format!("Couldn't change the properties of {held}: {e}"), cx);
                    workspace.file_op_ended(cx);
                })
                .ok();
            }
        });
        self.property_write = Some((path, task));
        Ok(())
    }

    /// `NoteReloadFromDisk`: a property write changed `path` on disk. The
    /// open note's clean buffer takes the disk text (clean, so no save
    /// writes the old frontmatter back); unsaved typing wins and is kept,
    /// with a flash. Independent of the dialog, which may be gone.
    pub(crate) fn reload_from_disk(&mut self, path: VaultPath, cx: &mut Context<Self>) {
        let Some(host) = self.notes.as_mut() else {
            return;
        };
        match host
            .session
            .begin_reload(&path, Some(&self.editor.read(cx).editor))
        {
            ReloadStep::NotOpen => {}
            ReloadStep::KeptDirty => {
                self.flash_error(KEPT_DIRTY, cx);
                self.refresh_note_properties();
            }
            ReloadStep::Read(read) => {
                // Keys typed now would be lost when the reload lands.
                self.block_input(true, cx);
                self.note_reload = Some(cx.spawn(async move |this, cx| {
                    let read = tokio::spawn(read.run()).await;
                    this.update(cx, |workspace, cx| workspace.reloaded(read, cx))
                        .ok();
                }));
            }
        }
    }

    /// The reload's read landed: load it clean, or say the buffer is stale.
    fn reloaded(&mut self, read: Result<NoteRead, JoinError>, cx: &mut Context<Self>) {
        self.note_reload = None;
        let outcome = match (read, self.notes.as_mut()) {
            (Ok(read), Some(host)) => self.editor.update(cx, |view, cx| {
                let outcome = host.session.finish_reload(read, Some(&mut view.editor));
                view.preedit = None;
                cx.notify();
                outcome
            }),
            (Err(e), _) => {
                tracing::error!("reloading the note failed to finish: {e}");
                ReloadOutcome::Failed
            }
            (_, None) => ReloadOutcome::NotOpen,
        };
        self.block_input(false, cx);
        if outcome == ReloadOutcome::Failed {
            self.flash_error(RELOAD_FAILED, cx);
        }
        if outcome != ReloadOutcome::NotOpen {
            self.refresh_note_properties();
            self.outline_saved(cx);
        }
        self.file_op_ended(cx);
        cx.notify();
    }

    /// A properties result from the app channel: a write releases the note
    /// and is flashed; every result goes to the open properties dialog,
    /// which drops one for another note. Stale ones (no such dialog) are
    /// dropped, but a failed write is still said.
    pub(crate) fn properties_data(&mut self, data: OverlayData, cx: &mut Context<Self>) {
        if let OverlayData::PropertyWritten { path, .. } = &data
            && self
                .property_write
                .as_ref()
                .is_some_and(|(held, _)| held == path)
        {
            self.property_write = None;
            self.file_op_ended(cx);
        }
        let dialog = self.properties.clone();
        self.with_window(cx, move |workspace, window, cx| {
            let open = workspace.open_modal(window, cx) == Some(ModalKind::Properties);
            let dialog = dialog.filter(|_| open).and_then(|d| d.upgrade());
            let shown = dialog.is_some_and(|dialog| {
                dialog.update(cx, |dialog, cx| dialog.on_data(&data, window, cx))
            });
            if let OverlayData::PropertyWritten { path, result } = data {
                match result {
                    Ok(text) => workspace.flash(capitalised(&text), cx),
                    Err(e) if !shown => workspace.flash_error(
                        format!("Couldn't change the properties of {path}: {}", e.message()),
                        cx,
                    ),
                    Err(_) => {}
                }
            }
        });
    }

    /// The open properties dialog (tests).
    #[cfg(test)]
    pub(crate) fn properties_dialog(&self) -> Option<Entity<PropertiesDialog>> {
        self.properties.as_ref()?.upgrade()
    }
}

/// `text` with its first letter in upper case (the logic's flashes are the
/// TUI's lower-case ones).
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
