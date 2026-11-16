//! The new note / new directory dialog: one dialog, whose kind picks the
//! wording and what the confirm creates (see [`NewEntryKind`]).
//!
//! The typed name is a path relative to the chosen directory, which starts as
//! the one being browsed (a leading `/` starts from the vault root); the
//! directories in it are made with the entry. Tab moves to the directory
//! choice ([`DirectoryPicker`]): a filter over every vault directory, Enter
//! picks one and returns to the name. The resolved target shows under the
//! title as the user types, a name core refuses shows why, and an async check marks a name
//! that is already taken. Enter creates whenever the name resolves and is
//! not known to be taken: the create itself is exclusive, so a stale check
//! can never overwrite anything.

use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use tokio::task::JoinHandle;

use crate::components::Component;
use crate::components::dialogs::ValidationState;
use crate::components::dialogs::directory_picker::{DirectoryPicker, PickerEvent, directory_label};
use crate::components::event_state::EventState;
use crate::components::hint_row::HintRow;
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::logic::file_ops::{self, NewEntry, NewEntryKind};
use crate::logic::message::{AppMessage, AppTx, OverlayData};
use crate::settings::themes::Theme;

/// Which part of the dialog has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Name,
    Directory,
}

pub struct NewEntryDialog {
    kind: NewEntryKind,
    /// The directory the typed name is relative to: the one the dialog
    /// opened on until another is chosen.
    base: VaultPath,
    picker: DirectoryPicker,
    focus: Focus,
    /// The directory line from the last render, a click target.
    directory_rect: ratatui::layout::Rect,
    vault: Arc<NoteVault>,
    input: SingleLineInput,
    /// What the input resolves to: `None` while it is empty.
    target: Option<Result<VaultPath, String>>,
    validation_state: ValidationState,
    validation_task: Option<JoinHandle<()>>,
    /// Why the last create failed.
    error: Option<String>,
    /// A create is running: Enter waits for it (it ends the dialog or
    /// fails into [`Self::fail`]).
    creating: bool,
    hints: HintRow,
    /// The chips while the directory choice has the keyboard.
    directory_hints: HintRow,
}

impl NewEntryDialog {
    /// A dialog for `kind` under `base`, with `initial` typed in already.
    pub fn new(
        kind: NewEntryKind,
        base: VaultPath,
        initial: Option<String>,
        vault: Arc<NoteVault>,
        tx: &AppTx,
    ) -> Self {
        spawn_directory_load(&vault, tx);
        let mut dialog = Self {
            kind,
            base,
            picker: DirectoryPicker::new(),
            focus: Focus::Name,
            directory_rect: ratatui::layout::Rect::default(),
            vault,
            input: SingleLineInput::with_value(initial.unwrap_or_default()),
            target: None,
            validation_state: ValidationState::Idle,
            validation_task: None,
            error: None,
            creating: false,
            hints: HintRow::new(&[
                (KeyCode::Enter, "Enter", "Create"),
                (KeyCode::Tab, "Tab", "Directory"),
                (KeyCode::Esc, "Esc", "Cancel"),
            ])
            .with_gap(3),
            directory_hints: HintRow::new(&[
                (KeyCode::Enter, "Enter", "Choose"),
                (KeyCode::Tab, "Tab", "Name"),
                (KeyCode::Esc, "Esc", "Back"),
            ])
            .with_gap(3),
        };
        dialog.on_input_changed(tx);
        dialog
    }

    /// The vault's directories arrived for the directory choice.
    pub fn on_directories(&mut self, directories: Vec<VaultPath>) {
        self.picker.set_directories(directories, &self.base);
    }

    /// Give the keyboard to the directory choice (starting it afresh on the
    /// current directory) or back to the name.
    fn set_focus(&mut self, focus: Focus) {
        if focus == Focus::Directory {
            self.picker.reset(&self.base);
        }
        self.focus = focus;
    }

    /// Take `directory` as the one the name is relative to, and go back to
    /// the name.
    fn choose_directory(&mut self, directory: VaultPath, tx: &AppTx) {
        self.base = directory;
        self.focus = Focus::Name;
        self.on_input_changed(tx);
    }

    /// Resolve the input again and, when it names something, check whether
    /// that is taken. Replaces any check still running.
    fn on_input_changed(&mut self, tx: &AppTx) {
        if let Some(task) = self.validation_task.take() {
            task.abort();
        }
        self.error = None;
        self.creating = false;
        let typed = self.input.value();
        if typed.trim().is_empty() {
            self.target = None;
            self.validation_state = ValidationState::Idle;
            return;
        }
        match self.kind.target(&self.base, typed) {
            Ok(path) => {
                let vault = Arc::clone(&self.vault);
                let tx = tx.clone();
                let target = path.clone();
                self.validation_task = Some(tokio::spawn(async move {
                    let available = file_ops::availability(&vault, &target).await.is_free();
                    tx.send(AppMessage::OverlayData(OverlayData::NewEntryValidation {
                        target,
                        available,
                    }))
                    .ok();
                }));
                self.validation_state = ValidationState::Pending;
                self.target = Some(Ok(path));
            }
            Err(reason) => {
                self.validation_state = ValidationState::Idle;
                self.target = Some(Err(reason));
            }
        }
    }

    /// The availability check for `target` finished. A result for a path the
    /// input has since left is stale.
    pub fn on_validation(&mut self, target: &VaultPath, available: bool) {
        if !matches!(&self.target, Some(Ok(current)) if current == target) {
            return;
        }
        self.validation_task = None;
        self.validation_state = if available {
            ValidationState::Available
        } else {
            ValidationState::Taken
        };
    }

    /// The path Enter would create, when it may be created now.
    fn creatable(&self) -> Option<&VaultPath> {
        match &self.target {
            Some(Ok(path)) if !self.creating && self.validation_state != ValidationState::Taken => {
                Some(path)
            }
            _ => None,
        }
    }

    fn confirm(&mut self, tx: &AppTx) {
        let Some(path) = self.creatable().cloned() else {
            return;
        };
        self.creating = true;
        let (kind, vault, tx) = (self.kind, Arc::clone(&self.vault), tx.clone());
        tokio::spawn(async move {
            match kind.create(&vault, &path, None).await {
                Ok(entry) => {
                    let is_directory = matches!(entry, NewEntry::Directory(_));
                    entry.announce(&tx);
                    // Opening a note replaces the dialog; a directory opens
                    // nothing, so the dialog closes itself.
                    if is_directory {
                        tx.send(AppMessage::CloseOverlay).ok();
                    }
                }
                Err(e) => {
                    tx.send(AppMessage::OverlayData(OverlayData::Error(e))).ok();
                }
            }
        });
    }

    /// A click on a hint chip runs its key. Modal: every mouse event is
    /// consumed.
    pub fn handle_mouse(&mut self, m: &MouseEvent, tx: &AppTx) -> EventState {
        let chips = match self.focus {
            Focus::Name => &self.hints,
            Focus::Directory => &self.directory_hints,
        };
        if let Some(key) = chips.hit(m) {
            self.handle_key(key, tx);
            return EventState::Consumed;
        }
        match self.focus {
            Focus::Directory => {
                if let PickerEvent::Chosen(dir) = self.picker.handle_mouse(m) {
                    self.choose_directory(dir, tx);
                }
            }
            Focus::Name => {
                if crate::components::clickable::rect_hit(self.directory_rect, m)
                    && crate::components::clickable::is_left_press(m)
                {
                    self.set_focus(Focus::Directory);
                }
            }
        }
        EventState::Consumed
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        // Tab swaps between the name and the directory choice.
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            self.set_focus(match self.focus {
                Focus::Name => Focus::Directory,
                Focus::Directory => Focus::Name,
            });
            return EventState::Consumed;
        }
        if self.focus == Focus::Directory {
            match self.picker.handle_key(key) {
                PickerEvent::Chosen(dir) => self.choose_directory(dir, tx),
                PickerEvent::Back => self.focus = Focus::Name,
                PickerEvent::None => {}
            }
            return EventState::Consumed;
        }
        match self.input.handle_key(&key) {
            InputOutcome::Submit => {
                self.confirm(tx);
                EventState::Consumed
            }
            InputOutcome::Cancel => {
                tx.send(AppMessage::CloseOverlay).ok();
                EventState::Consumed
            }
            InputOutcome::Changed => {
                self.on_input_changed(tx);
                EventState::Consumed
            }
            InputOutcome::Consumed => EventState::Consumed,
            InputOutcome::NotConsumed => EventState::NotConsumed,
        }
    }
}

/// Load the vault's directories off the UI thread (they come from walking
/// the disk) for the directory choice.
fn spawn_directory_load(vault: &Arc<NoteVault>, tx: &AppTx) {
    let (vault, tx) = (Arc::clone(vault), tx.clone());
    tokio::spawn(async move {
        let listed = tokio::task::spawn_blocking(move || file_ops::list_directories(&vault)).await;
        if let Ok(Ok(directories)) = listed {
            tx.send(AppMessage::OverlayData(OverlayData::NewEntryDirectories(
                directories,
            )))
            .ok();
        }
    });
}

impl Component for NewEntryDialog {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let choosing = self.focus == Focus::Directory;
        let picker_height: u16 = if choosing { 8 } else { 0 };
        let height = 14 + picker_height + u16::from(self.error.is_some());
        let popup_area = super::fixed_centered_rect(58, height, rect);
        let title = format!(" {} ", self.kind.title());
        let inner = modal_chrome(
            f,
            popup_area,
            theme,
            ModalSpec {
                title: Some(&title),
                border: Some(Style::default().fg(theme.fg.to_ratatui())),
                ..Default::default()
            },
        );

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),             // 0: spacer
                Constraint::Length(1),             // 1: where it goes
                Constraint::Length(1),             // 2: separator
                Constraint::Length(1),             // 3: directory
                Constraint::Length(picker_height), // 4: directory choice
                Constraint::Length(1),             // 5: label
                Constraint::Length(3),             // 6: input
                Constraint::Length(1),             // 7: status
                Constraint::Length(1),             // 8: spacer
                Constraint::Length(1),             // 9: hint
                Constraint::Min(0),                // 10: error
            ])
            .split(inner);

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let gray = theme.gray.to_ratatui();

        let place = match &self.target {
            Some(Ok(path)) => format!("  {path}"),
            _ => format!("  in {}", self.base),
        };
        super::render_path_row(f, rows[1], &place, fg, bg);
        super::render_separator(f, rows[2], gray, bg);

        // The directory: the chosen one with how to change it, or the label
        // over the choice itself.
        self.directory_rect = rows[3];
        if choosing {
            f.render_widget(
                Paragraph::new("  DIRECTORY").style(Style::default().fg(fg).bg(bg)),
                rows[3],
            );
            self.picker.render(f, rows[4], theme);
        } else {
            let line = ratatui::text::Line::from(vec![
                ratatui::text::Span::styled("  DIRECTORY", Style::default().fg(gray)),
                ratatui::text::Span::styled(
                    format!("  {}", directory_label(&self.base).trim_start()),
                    Style::default().fg(fg),
                ),
                ratatui::text::Span::styled("  (Tab to change)", Style::default().fg(gray)),
            ]);
            f.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), rows[3]);
        }

        f.render_widget(
            Paragraph::new(format!("  {}", self.kind.field_label()))
                .style(Style::default().fg(gray).bg(bg)),
            rows[5],
        );
        super::render_name_field(f, rows[6], &mut self.input, self.validation_state, theme);
        match &self.target {
            // Why core refuses the name, where the status would be.
            Some(Err(reason)) => f.render_widget(
                Paragraph::new(format!("  {reason}"))
                    .style(Style::default().fg(theme.red.to_ratatui()).bg(bg)),
                rows[7],
            ),
            _ => super::render_validation_status(f, rows[7], self.validation_state, theme),
        }

        if choosing {
            let has_row = self.picker.highlighted().is_some();
            super::render_confirm_hints(
                f,
                rows[9],
                &mut self.directory_hints,
                has_row,
                has_row,
                fg,
                theme,
            );
        } else {
            let can_create = self.creatable().is_some();
            super::render_confirm_hints(
                f,
                rows[9],
                &mut self.hints,
                can_create,
                can_create,
                fg,
                theme,
            );
        }

        if let Some(msg) = &self.error {
            super::render_error_row(f, rows[10], msg, theme);
        }
    }
}

impl NewEntryDialog {
    #[cfg(test)]
    fn set_error_for_test(&mut self, msg: String) {
        use super::Dialog as _;
        self.set_error(msg);
    }

    /// The create failed: show why, and let the user change the name or try
    /// again.
    fn fail(&mut self, msg: String) {
        self.creating = false;
        self.error = Some(msg);
    }
}

impl_dialog!(
    @impl NewEntryDialog,
    fn set_error(&mut self, msg: String) {
        self.fail(msg);
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use tokio::sync::mpsc;

    async fn dialog(
        kind: NewEntryKind,
        base: &str,
        initial: Option<&str>,
    ) -> (
        NewEntryDialog,
        Arc<NoteVault>,
        AppTx,
        mpsc::UnboundedReceiver<AppMessage>,
    ) {
        let vault = crate::test_support::temp_vault("new-entry-dialog").await;
        let (tx, rx) = mpsc::unbounded_channel();
        let d = NewEntryDialog::new(
            kind,
            VaultPath::new(base),
            initial.map(str::to_string),
            Arc::clone(&vault),
            &tx,
        );
        (d, vault, tx, rx)
    }

    fn press(d: &mut NewEntryDialog, code: KeyCode, tx: &AppTx) {
        d.handle_key(KeyEvent::new(code, KeyModifiers::NONE), tx);
    }

    fn type_text(d: &mut NewEntryDialog, text: &str, tx: &AppTx) {
        for c in text.chars() {
            press(d, KeyCode::Char(c), tx);
        }
    }

    async fn next(rx: &mut mpsc::UnboundedReceiver<AppMessage>) -> AppMessage {
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out waiting for a message")
            .expect("channel closed")
    }

    #[tokio::test]
    async fn empty_input_resolves_nothing_and_enter_does_nothing() {
        let (mut d, _vault, tx, mut rx) = dialog(NewEntryKind::Note, "/", None).await;
        assert!(d.target.is_none());
        press(&mut d, KeyCode::Enter, &tx);
        tokio::task::yield_now().await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn typing_resolves_the_target_against_the_base() {
        let (mut d, _vault, tx, _rx) = dialog(NewEntryKind::Note, "/notes", None).await;
        type_text(&mut d, "Idea", &tx);
        assert_eq!(
            d.target,
            Some(Ok(VaultPath::new("/notes/idea.md"))),
            "the extension is added and the name lowercased"
        );
        assert_eq!(d.validation_state, ValidationState::Pending);
    }

    #[tokio::test]
    async fn a_refused_name_shows_why_and_blocks_enter() {
        let (mut d, _vault, tx, mut rx) = dialog(NewEntryKind::Note, "/", None).await;
        type_text(&mut d, "bad?", &tx);
        assert!(matches!(&d.target, Some(Err(reason)) if reason.contains('?')));
        assert_eq!(d.validation_state, ValidationState::Idle);
        assert!(d.creatable().is_none());
        press(&mut d, KeyCode::Enter, &tx);
        tokio::task::yield_now().await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn the_initial_text_resolves_at_once() {
        let (d, _vault, _tx, _rx) = dialog(NewEntryKind::Note, "/", Some("From search")).await;
        assert_eq!(d.target, Some(Ok(VaultPath::new("/from search.md"))));
    }

    #[tokio::test]
    async fn a_taken_name_is_marked_and_blocks_enter() {
        let (mut d, vault, tx, mut rx) = dialog(NewEntryKind::Note, "/", None).await;
        vault
            .create_note(&VaultPath::new("taken.md"), "x")
            .await
            .unwrap();
        type_text(&mut d, "taken", &tx);
        loop {
            match next(&mut rx).await {
                AppMessage::OverlayData(OverlayData::NewEntryValidation { target, available }) => {
                    assert!(!available);
                    d.on_validation(&target, available);
                    break;
                }
                // The directory list loading alongside.
                AppMessage::OverlayData(OverlayData::NewEntryDirectories(_)) => {}
                other => panic!("expected a validation result, got {other:?}"),
            }
        }
        assert_eq!(d.validation_state, ValidationState::Taken);
        assert!(d.creatable().is_none());
    }

    #[tokio::test]
    async fn a_stale_validation_result_is_ignored() {
        let (mut d, _vault, tx, _rx) = dialog(NewEntryKind::Note, "/", None).await;
        type_text(&mut d, "a", &tx);
        d.on_validation(&VaultPath::new("/other.md"), false);
        assert_eq!(d.validation_state, ValidationState::Pending);
    }

    #[tokio::test]
    async fn enter_creates_and_opens_a_nested_note() {
        let (mut d, vault, tx, mut rx) = dialog(NewEntryKind::Note, "/", None).await;
        type_text(&mut d, "projects/plan", &tx);
        press(&mut d, KeyCode::Enter, &tx);
        // The check result and the create result both arrive; the create
        // announces the note, then opens it.
        let mut created = None;
        let mut opened = None;
        while opened.is_none() {
            match next(&mut rx).await {
                AppMessage::FileOp(crate::logic::message::FileOp::Created(p)) => created = Some(p),
                AppMessage::OpenPath { path, .. } => opened = Some(path),
                AppMessage::OverlayData(OverlayData::Error(e)) => panic!("create failed: {e}"),
                _ => {}
            }
        }
        let path = VaultPath::new("/projects/plan.md");
        assert_eq!(created, Some(path.clone()));
        assert_eq!(opened, Some(path.clone()));
        assert!(vault.exists(&path).await);
    }

    #[tokio::test]
    async fn enter_creates_a_directory_then_closes_the_dialog() {
        let (mut d, vault, tx, mut rx) = dialog(NewEntryKind::Directory, "/", None).await;
        type_text(&mut d, "Archive/Old", &tx);
        press(&mut d, KeyCode::Enter, &tx);
        let mut created = None;
        loop {
            match next(&mut rx).await {
                AppMessage::FileOp(crate::logic::message::FileOp::Created(p)) => created = Some(p),
                AppMessage::CloseOverlay => break,
                AppMessage::OverlayData(OverlayData::Error(e)) => panic!("create failed: {e}"),
                _ => {}
            }
        }
        let path = VaultPath::new("/archive/old");
        assert_eq!(created, Some(path.clone()));
        assert!(vault.exists(&path).await);
    }

    #[tokio::test]
    async fn creating_over_an_existing_note_reports_it_and_keeps_the_dialog() {
        let (mut d, vault, tx, mut rx) = dialog(NewEntryKind::Note, "/", None).await;
        type_text(&mut d, "dup", &tx);
        // Created behind the dialog's back, so its check cannot have known.
        vault
            .create_note(&VaultPath::new("dup.md"), "original")
            .await
            .unwrap();
        press(&mut d, KeyCode::Enter, &tx);
        loop {
            if let AppMessage::OverlayData(OverlayData::Error(e)) = next(&mut rx).await {
                assert_eq!(e, "A note with that name already exists");
                break;
            }
        }
        let text = vault
            .load_note(&VaultPath::new("dup.md"))
            .await
            .unwrap()
            .raw_text;
        assert_eq!(text, "original");
    }

    /// A second Enter while the create runs must not start another one.
    #[tokio::test]
    async fn a_second_enter_while_creating_does_nothing() {
        let (mut d, vault, tx, mut rx) = dialog(NewEntryKind::Note, "/", None).await;
        type_text(&mut d, "once", &tx);
        press(&mut d, KeyCode::Enter, &tx);
        assert!(d.creating);
        press(&mut d, KeyCode::Enter, &tx);
        let mut opens = 0;
        while opens == 0 {
            if let AppMessage::OpenPath { .. } = next(&mut rx).await {
                opens += 1;
            }
        }
        // Let any stray second create finish and report.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        while let Ok(msg) = rx.try_recv() {
            assert!(
                !matches!(
                    msg,
                    AppMessage::OpenPath { .. } | AppMessage::OverlayData(OverlayData::Error(_))
                ),
                "a second create ran: {msg:?}"
            );
        }
        assert!(vault.exists(&VaultPath::new("/once.md")).await);
    }

    /// A failed create leaves the dialog usable: the error shows and Enter
    /// works again once the name changes.
    #[tokio::test]
    async fn a_failed_create_reports_and_can_be_retried() {
        let (mut d, vault, tx, mut rx) = dialog(NewEntryKind::Note, "/", None).await;
        type_text(&mut d, "dup", &tx);
        vault
            .create_note(&VaultPath::new("dup.md"), "x")
            .await
            .unwrap();
        press(&mut d, KeyCode::Enter, &tx);
        assert!(d.creating);
        loop {
            if let AppMessage::OverlayData(OverlayData::Error(e)) = next(&mut rx).await {
                d.set_error_for_test(e);
                break;
            }
        }
        assert!(!d.creating, "a failure frees the dialog");
        assert!(d.error.is_some());
        press(&mut d, KeyCode::Char('2'), &tx);
        assert!(d.error.is_none(), "typing clears the error");
        assert!(d.creatable().is_some(), "and Enter works on the new name");
    }

    fn dirs(paths: &[&str]) -> Vec<VaultPath> {
        paths.iter().map(VaultPath::new).collect()
    }

    /// The directory the dialog opened on is the default, and it is shown.
    #[tokio::test]
    async fn the_browsed_directory_is_the_default_choice() {
        let (mut d, _vault, tx, _rx) = dialog(NewEntryKind::Note, "/projects", None).await;
        d.on_directories(dirs(&["/", "/archive", "/projects"]));
        type_text(&mut d, "idea", &tx);
        assert_eq!(d.target, Some(Ok(VaultPath::new("/projects/idea.md"))));
        // Choosing starts on it.
        press(&mut d, KeyCode::Tab, &tx);
        assert_eq!(d.picker.highlighted(), Some(&VaultPath::new("/projects")));
    }

    /// Tab opens the choice, a pick moves the target, and the name typed so
    /// far is kept and re-resolved against the new directory.
    #[tokio::test]
    async fn choosing_another_directory_moves_the_target() {
        let (mut d, _vault, tx, _rx) = dialog(NewEntryKind::Note, "/projects", None).await;
        d.on_directories(dirs(&["/", "/archive", "/projects"]));
        type_text(&mut d, "sub/idea", &tx);
        assert_eq!(d.target, Some(Ok(VaultPath::new("/projects/sub/idea.md"))));

        press(&mut d, KeyCode::Tab, &tx);
        assert_eq!(d.focus, Focus::Directory);
        // Typing now filters directories, not the name.
        type_text(&mut d, "arch", &tx);
        assert_eq!(d.input.value(), "sub/idea");
        press(&mut d, KeyCode::Enter, &tx);

        assert_eq!(d.focus, Focus::Name, "back on the name");
        assert_eq!(d.base, VaultPath::new("/archive"));
        assert_eq!(d.target, Some(Ok(VaultPath::new("/archive/sub/idea.md"))));
        assert_eq!(d.input.value(), "sub/idea", "the name is kept");
    }

    /// Escape in the choice goes back to the name; it does not close the
    /// dialog or change the directory.
    #[tokio::test]
    async fn escape_in_the_directory_choice_goes_back_without_closing() {
        let (mut d, _vault, tx, mut rx) = dialog(NewEntryKind::Note, "/projects", None).await;
        d.on_directories(dirs(&["/", "/archive", "/projects"]));
        press(&mut d, KeyCode::Tab, &tx);
        press(&mut d, KeyCode::Down, &tx);
        press(&mut d, KeyCode::Esc, &tx);
        assert_eq!(d.focus, Focus::Name);
        assert_eq!(d.base, VaultPath::new("/projects"));
        while let Ok(msg) = rx.try_recv() {
            assert!(!matches!(msg, AppMessage::CloseOverlay), "dialog closed");
        }
    }

    /// Tab from the directory choice returns to the name too.
    #[tokio::test]
    async fn tab_toggles_between_the_name_and_the_directory() {
        let (mut d, _vault, tx, _rx) = dialog(NewEntryKind::Directory, "/", None).await;
        press(&mut d, KeyCode::Tab, &tx);
        assert_eq!(d.focus, Focus::Directory);
        press(&mut d, KeyCode::Tab, &tx);
        assert_eq!(d.focus, Focus::Name);
    }

    /// A note made in a chosen directory lands there, and a directory too.
    #[tokio::test]
    async fn creating_uses_the_chosen_directory_for_both_kinds() {
        for kind in [NewEntryKind::Note, NewEntryKind::Directory] {
            let (mut d, vault, tx, mut rx) = dialog(kind, "/", None).await;
            vault
                .create_directory(&VaultPath::new("/dest"))
                .await
                .unwrap();
            d.on_directories(dirs(&["/", "/dest"]));
            press(&mut d, KeyCode::Tab, &tx);
            type_text(&mut d, "dest", &tx);
            press(&mut d, KeyCode::Enter, &tx);
            type_text(&mut d, "made", &tx);
            press(&mut d, KeyCode::Enter, &tx);
            loop {
                match next(&mut rx).await {
                    AppMessage::OpenPath { .. } | AppMessage::CloseOverlay => break,
                    AppMessage::OverlayData(OverlayData::Error(e)) => panic!("failed: {e}"),
                    _ => {}
                }
            }
            let made = match kind {
                NewEntryKind::Note => VaultPath::new("/dest/made.md"),
                NewEntryKind::Directory => VaultPath::new("/dest/made"),
            };
            assert!(vault.exists(&made).await, "{kind:?}");
        }
    }

    /// The directories load in the background and reach the picker.
    #[tokio::test]
    async fn the_vault_directories_load_into_the_picker() {
        let (mut d, vault, _tx, mut rx) = dialog(NewEntryKind::Note, "/", None).await;
        vault
            .create_directory(&VaultPath::new("/found"))
            .await
            .unwrap();
        // The load may have started before the directory existed; ask again
        // through the same path the dialog uses.
        super::spawn_directory_load(&vault, &_tx);
        loop {
            if let AppMessage::OverlayData(OverlayData::NewEntryDirectories(dirs)) =
                next(&mut rx).await
                && dirs.contains(&VaultPath::new("/found"))
            {
                d.on_directories(dirs);
                break;
            }
        }
        assert!(d.picker.highlighted().is_some());
    }

    #[tokio::test]
    async fn escape_closes_the_dialog() {
        let (mut d, _vault, tx, mut rx) = dialog(NewEntryKind::Directory, "/", None).await;
        press(&mut d, KeyCode::Esc, &tx);
        assert!(matches!(rx.try_recv(), Ok(AppMessage::CloseOverlay)));
    }
}
