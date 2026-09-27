use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::error::VaultError;
use kimun_core::nfs::VaultPath;
use throbber_widgets_tui::ThrobberState;

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::indexing::{IndexingProgressState, render_indexing_overlay, spawn_running};
use crate::settings::SharedSettings;
use crate::settings::themes::Theme;

pub struct StartScreen {
    settings: SharedSettings,
    theme: Theme,
    vault: Option<Arc<NoteVault>>,
    overlay: Option<IndexingProgressState>,
    throbber_state: ThrobberState,
}

impl StartScreen {
    pub fn new(settings: SharedSettings, vault: Option<Arc<NoteVault>>) -> Self {
        let theme = settings.read().unwrap().get_theme();
        Self {
            settings,
            theme,
            vault,
            overlay: None,
            throbber_state: ThrobberState::default(),
        }
    }

    /// Where the restored session lands: the newest history entry still on
    /// disk, or the vault root when none is. Notes moved or deleted outside
    /// kimün are skipped, not offered for re-creation — the user asked for
    /// their last note, not for a new one — and dropped from the history so
    /// they stop turning up in recents.
    ///
    /// When the note it would have landed on is the one gone, it parks a
    /// flash saying so: the editor opening somewhere else unannounced reads
    /// as kimün losing the note.
    async fn restore_target(&self, tx: &AppTx) -> VaultPath {
        let history = self.settings.read().unwrap().current_last_paths();
        // No vault means no workspace: the open is routed to onboarding, and
        // there is nothing to check the history against.
        let Some(vault) = &self.vault else {
            return history.first().cloned().unwrap_or_else(VaultPath::root);
        };
        let mut live = Vec::with_capacity(history.len());
        for path in &history {
            if vault.exists(path).await {
                live.push(path.clone());
            }
        }
        if live.len() != history.len() {
            self.settings.write().unwrap().replace_path_history(&live);
        }
        if let Some(last) = history.first()
            && live.first() != Some(last)
        {
            tx.send(AppEvent::ParkFlash(format!(
                "{last} is gone (moved or deleted outside kimün)"
            )))
            .ok();
        }
        live.first().cloned().unwrap_or_else(VaultPath::root)
    }
}

#[async_trait]
impl AppScreen for StartScreen {
    async fn on_enter(&mut self, tx: &AppTx) {
        if let Some(vault) = self.vault.clone() {
            let tx2 = tx.clone();
            let handle = tokio::spawn(async move {
                match vault.validate_and_init().await {
                    Ok(report) => {
                        tx2.send(AppEvent::IndexingDone(Ok(report.duration))).ok();
                    }
                    Err(e @ VaultError::CaseConflict { .. }) => {
                        // Route structural vault conflicts to VaultConflict so the main
                        // loop can clear the vault path and redirect to settings.
                        // To support a future VaultError conflict type: add one arm here.
                        tx2.send(AppEvent::VaultConflict(e.to_string())).ok();
                    }
                    Err(e) => {
                        tx2.send(AppEvent::IndexingDone(Err(e.to_string()))).ok();
                    }
                }
            });
            self.overlay = Some(spawn_running(handle, tx));
        } else {
            let path = self.restore_target(tx).await;
            tx.send(AppEvent::open(path)).ok();
        }
    }

    fn get_kind(&self) -> ScreenKind {
        ScreenKind::Start
    }

    fn handle_input(&mut self, _event: &InputEvent, _tx: &AppTx) -> EventState {
        if matches!(self.overlay, Some(IndexingProgressState::Running { .. })) {
            return EventState::Consumed;
        }
        EventState::NotConsumed
    }

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) {
        if let AppEvent::IndexingDone(_) = &msg {
            self.overlay = None;
            let path = self.restore_target(tx).await;
            tx.send(AppEvent::open(path)).ok();
        }
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        if let Some(ref state) = self.overlay {
            render_indexing_overlay(
                f,
                state,
                &mut self.throbber_state,
                &self.theme,
                "Initializing vault…",
            );
            return;
        }
        let block = ratatui::widgets::Block::default()
            .title("Start app")
            .borders(ratatui::widgets::Borders::ALL);
        f.render_widget(block, f.area());
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::settings::AppSettings;
    use crate::test_support::{key_event, temp_vault};
    use kimun_core::VaultConfig;
    use ratatui::crossterm::event::KeyCode;
    use std::sync::{Arc, RwLock};
    use tokio::sync::mpsc::unbounded_channel;

    fn shared_defaults() -> SharedSettings {
        Arc::new(RwLock::new(AppSettings::default()))
    }

    async fn make_vault() -> Arc<NoteVault> {
        temp_vault("start").await
    }

    #[tokio::test]
    async fn on_enter_vault_none_sends_open_path() {
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        let mut screen = StartScreen::new(shared_defaults(), None);
        screen.on_enter(&tx).await;
        let msg = rx.try_recv().expect("expected a message");
        assert!(
            matches!(msg, AppEvent::OpenPath { .. }),
            "expected OpenPath, got {:?}",
            msg
        );
        assert!(
            screen.overlay.is_none(),
            "overlay should be None when vault is None"
        );
    }

    #[tokio::test]
    async fn on_enter_vault_some_sets_overlay_and_defers_open_path() {
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        let vault = make_vault().await;
        let mut screen = StartScreen::new(shared_defaults(), Some(vault));
        screen.on_enter(&tx).await;
        assert!(
            matches!(screen.overlay, Some(IndexingProgressState::Running { .. })),
            "overlay should be Running after on_enter with vault"
        );
        // Drain all messages and ensure none are OpenPath
        let messages: Vec<AppEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        let has_open_path = messages
            .iter()
            .any(|m| matches!(m, AppEvent::OpenPath { .. }));
        assert!(
            !has_open_path,
            "OpenPath should not be sent immediately when vault is Some"
        );
    }

    #[tokio::test]
    async fn handle_app_message_indexing_done_ok_clears_overlay_and_sends_open_path() {
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        let mut screen = StartScreen::new(shared_defaults(), None);
        screen.overlay = Some(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        screen
            .handle_app_message(AppEvent::IndexingDone(Ok(Duration::from_secs(1))), &tx)
            .await;
        assert!(screen.overlay.is_none(), "overlay should be cleared");
        let msg = rx.try_recv().expect("expected OpenPath message");
        assert!(
            matches!(msg, AppEvent::OpenPath { .. }),
            "expected OpenPath after indexing done"
        );
    }

    #[tokio::test]
    async fn handle_app_message_indexing_done_err_clears_overlay_and_sends_open_path() {
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        let mut screen = StartScreen::new(shared_defaults(), None);
        screen.overlay = Some(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        screen
            .handle_app_message(AppEvent::IndexingDone(Err("fail".to_string())), &tx)
            .await;
        assert!(
            screen.overlay.is_none(),
            "overlay should be cleared on error"
        );
        let msg = rx.try_recv().expect("expected OpenPath message");
        assert!(
            matches!(msg, AppEvent::OpenPath { .. }),
            "expected OpenPath even after failed indexing"
        );
    }

    /// A vault holding `existing` notes, and settings whose history for it is
    /// `history` (newest first). Returns the history dir guard with the rest.
    async fn restore_fixture(
        history: &[&str],
        existing: &[&str],
    ) -> (StartScreen, SharedSettings, tempfile::TempDir) {
        let vault = make_vault().await;
        for note in existing {
            vault
                .create_note(&VaultPath::new(*note), "text")
                .await
                .unwrap();
        }
        let history_dir = tempfile::TempDir::new().unwrap();
        let settings = AppSettings::for_test_workspace(
            "ws",
            vault.workspace_path(),
            crate::test_support::sys(history_dir.path()),
        );
        let paths: Vec<VaultPath> = history.iter().map(|p| VaultPath::new(*p)).collect();
        settings.history_for("ws").write(&paths).unwrap();
        let settings: SharedSettings = Arc::new(RwLock::new(settings));
        let screen = StartScreen::new(settings.clone(), Some(vault));
        (screen, settings, history_dir)
    }

    /// Runs the post-indexing restore and returns every event it sent.
    async fn restore(screen: &mut StartScreen) -> Vec<AppEvent> {
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        screen
            .handle_app_message(AppEvent::IndexingDone(Ok(Duration::from_secs(0))), &tx)
            .await;
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    fn opened(events: &[AppEvent]) -> Vec<VaultPath> {
        events
            .iter()
            .filter_map(|e| match e {
                AppEvent::OpenPath { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn restore_skips_a_last_note_gone_from_disk() {
        let (mut screen, _settings, _dir) =
            restore_fixture(&["gone.md", "still.md"], &["still.md"]).await;
        let events = restore(&mut screen).await;
        assert_eq!(opened(&events), vec![VaultPath::new("still.md")]);
    }

    fn parked_flashes(events: &[AppEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                AppEvent::ParkFlash(msg) => Some(msg.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn restore_tells_the_user_their_last_note_is_gone() {
        let (mut screen, _settings, _dir) =
            restore_fixture(&["gone.md", "still.md"], &["still.md"]).await;
        let events = restore(&mut screen).await;
        let flashes = parked_flashes(&events);
        assert_eq!(
            flashes.len(),
            1,
            "expected one parked flash, got {flashes:?}"
        );
        assert!(
            flashes[0].contains("gone.md"),
            "flash must name the note: {flashes:?}"
        );
    }

    /// Only the note the user expected to land on is worth a word; older
    /// entries vanishing from recents is not news.
    #[tokio::test]
    async fn restore_is_silent_when_only_older_entries_are_gone() {
        let (mut screen, _settings, _dir) =
            restore_fixture(&["still.md", "gone.md"], &["still.md"]).await;
        let events = restore(&mut screen).await;
        assert!(parked_flashes(&events).is_empty());
    }

    #[tokio::test]
    async fn restore_lands_on_the_root_when_no_history_entry_survives() {
        let (mut screen, _settings, _dir) = restore_fixture(&["a.md", "b.md"], &[]).await;
        let events = restore(&mut screen).await;
        assert_eq!(opened(&events), vec![VaultPath::root()]);
    }

    #[tokio::test]
    async fn restore_drops_notes_gone_from_disk_from_the_history() {
        let (mut screen, settings, _dir) =
            restore_fixture(&["gone.md", "still.md", "also_gone.md"], &["still.md"]).await;
        restore(&mut screen).await;
        assert_eq!(
            settings.read().unwrap().current_last_paths(),
            vec![VaultPath::new("still.md")]
        );
    }

    #[tokio::test]
    async fn handle_input_blocked_while_overlay_running() {
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        let mut screen = StartScreen::new(shared_defaults(), None);
        screen.overlay = Some(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        let state = screen.handle_input(&key_event(KeyCode::Enter), &tx);
        assert!(
            matches!(state, EventState::Consumed),
            "input should be consumed while overlay is running"
        );
        // Drain the ticker Redraw messages but confirm no other app-level messages
        let messages: Vec<AppEvent> = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        let has_non_redraw = messages.iter().any(|m| !matches!(m, AppEvent::Redraw));
        assert!(
            !has_non_redraw,
            "handle_input should not send non-Redraw messages"
        );
    }

    #[tokio::test]
    async fn handle_input_not_consumed_while_overlay_none() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let mut screen = StartScreen::new(shared_defaults(), None);
        screen.overlay = None;
        let state = screen.handle_input(&key_event(KeyCode::Enter), &tx);
        assert!(
            matches!(state, EventState::NotConsumed),
            "input should not be consumed when overlay is None"
        );
    }

    // Linux only: macOS and Windows filesystems are case-insensitive by default,
    // so creating note.md + Note.md would silently overwrite on those platforms.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn on_enter_case_conflict_sends_vault_conflict_not_indexing_done() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("note.md"), "a").unwrap();
        std::fs::write(tmp.path().join("Note.md"), "b").unwrap();

        let vault = Arc::new(
            NoteVault::new(VaultConfig::new(crate::test_support::sys(tmp.path())))
                .await
                .unwrap(),
        );
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        let mut screen = StartScreen::new(shared_defaults(), Some(vault));
        screen.on_enter(&tx).await;

        // Drain events until VaultConflict arrives; skip Redraw ticks from the spinner.
        let conflict_msg = loop {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("timed out waiting for VaultConflict")
                .expect("channel closed");

            match msg {
                AppEvent::VaultConflict(details) => break details,
                AppEvent::Redraw => continue,
                AppEvent::IndexingDone(_) => panic!("expected VaultConflict, got IndexingDone"),
                _ => continue,
            }
        };

        assert!(
            conflict_msg.contains("note.md") && conflict_msg.contains("Note.md"),
            "conflict message should name both files, got: {}",
            conflict_msg
        );
    }
}
