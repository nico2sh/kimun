//! The new note / new directory dialog: one dialog, whose kind picks the
//! wording and what the confirm creates (see [`NewEntryKind`]).
//!
//! The typed name is a path relative to the directory being browsed (a
//! leading `/` starts from the vault root); the directories in it are made
//! with the entry. The resolved target shows under the title as the user
//! types, a name core refuses shows why, and an async check marks a name
//! that is already taken. Enter creates whenever the name resolves and is
//! not known to be taken: the create itself is exclusive, so a stale check
//! can never overwrite anything.

use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyEvent, MouseEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;
use tokio::task::JoinHandle;

use crate::components::Component;
use crate::components::dialogs::ValidationState;
use crate::components::event_state::EventState;
use crate::components::hint_row::HintRow;
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::logic::file_ops::{self, NewEntry, NewEntryKind};
use crate::logic::message::{AppMessage, AppTx, OverlayData};
use crate::settings::themes::Theme;

pub struct NewEntryDialog {
    kind: NewEntryKind,
    /// The directory the typed name is relative to.
    base: VaultPath,
    vault: Arc<NoteVault>,
    input: SingleLineInput,
    /// What the input resolves to: `None` while it is empty.
    target: Option<Result<VaultPath, String>>,
    validation_state: ValidationState,
    validation_task: Option<JoinHandle<()>>,
    /// Why the last create failed.
    error: Option<String>,
    hints: HintRow,
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
        let mut dialog = Self {
            kind,
            base,
            vault,
            input: SingleLineInput::with_value(initial.unwrap_or_default()),
            target: None,
            validation_state: ValidationState::Idle,
            validation_task: None,
            error: None,
            hints: super::confirm_hints("Create"),
        };
        dialog.on_input_changed(tx);
        dialog
    }

    /// Resolve the input again and, when it names something, check whether
    /// that is taken. Replaces any check still running.
    fn on_input_changed(&mut self, tx: &AppTx) {
        if let Some(task) = self.validation_task.take() {
            task.abort();
        }
        self.error = None;
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
            Some(Ok(path)) if self.validation_state != ValidationState::Taken => Some(path),
            _ => None,
        }
    }

    fn confirm(&self, tx: &AppTx) {
        let Some(path) = self.creatable().cloned() else {
            return;
        };
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
        if let Some(key) = self.hints.hit(m) {
            self.handle_key(key, tx);
        }
        EventState::Consumed
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
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

impl Component for NewEntryDialog {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let height = if self.error.is_some() { 13 } else { 12 };
        let popup_area = super::fixed_centered_rect(56, height, rect);
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
                Constraint::Length(1), // 0: spacer
                Constraint::Length(1), // 1: where it goes
                Constraint::Length(1), // 2: separator
                Constraint::Length(1), // 3: label
                Constraint::Length(3), // 4: input
                Constraint::Length(1), // 5: status
                Constraint::Length(1), // 6: spacer
                Constraint::Length(1), // 7: hint
                Constraint::Min(0),    // 8: error
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
        f.render_widget(
            Paragraph::new(format!("  {}", self.kind.field_label()))
                .style(Style::default().fg(gray).bg(bg)),
            rows[3],
        );
        super::render_name_field(f, rows[4], &mut self.input, self.validation_state, theme);
        match &self.target {
            // Why core refuses the name, where the status would be.
            Some(Err(reason)) => f.render_widget(
                Paragraph::new(format!("  {reason}"))
                    .style(Style::default().fg(theme.red.to_ratatui()).bg(bg)),
                rows[5],
            ),
            _ => super::render_validation_status(f, rows[5], self.validation_state, theme),
        }

        let can_create = self.creatable().is_some();
        super::render_confirm_hints(
            f,
            rows[7],
            &mut self.hints,
            can_create,
            can_create,
            fg,
            theme,
        );

        if let Some(msg) = &self.error {
            super::render_error_row(f, rows[8], msg, theme);
        }
    }
}

impl_dialog!(NewEntryDialog, error);

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
        match next(&mut rx).await {
            AppMessage::OverlayData(OverlayData::NewEntryValidation { target, available }) => {
                assert!(!available);
                d.on_validation(&target, available);
            }
            other => panic!("expected a validation result, got {other:?}"),
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

    #[tokio::test]
    async fn escape_closes_the_dialog() {
        let (mut d, _vault, tx, mut rx) = dialog(NewEntryKind::Directory, "/", None).await;
        press(&mut d, KeyCode::Esc, &tx);
        assert!(matches!(rx.try_recv(), Ok(AppMessage::CloseOverlay)));
    }
}
