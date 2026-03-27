use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use throbber_widgets_tui::ThrobberState;

use crate::app_screen::{AppScreen, ScreenKind};
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::indexing::{
    IndexingProgressState, render_indexing_overlay, spawn_running,
};
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

pub struct StartScreen {
    settings: AppSettings,
    theme: Theme,
    vault: Option<Arc<NoteVault>>,
    overlay: Option<IndexingProgressState>,
    throbber_state: ThrobberState,
}

impl StartScreen {
    pub fn new(settings: AppSettings, vault: Option<Arc<NoteVault>>) -> Self {
        let theme = settings.get_theme();
        Self {
            settings,
            theme,
            vault,
            overlay: None,
            throbber_state: ThrobberState::default(),
        }
    }
}

#[async_trait]
impl AppScreen for StartScreen {
    async fn on_enter(&mut self, tx: &AppTx) {
        if let Some(vault) = self.vault.clone() {
            let tx2 = tx.clone();
            let handle = tokio::spawn(async move {
                let result = vault
                    .validate_and_init()
                    .await
                    .map_err(|e| e.to_string())
                    .map(|r| r.duration);
                tx2.send(AppEvent::IndexingDone(result)).ok();
            });
            self.overlay = Some(spawn_running(handle, tx));
        } else {
            let path = self
                .settings
                .last_paths
                .last()
                .map_or_else(VaultPath::root, |p| p.to_owned());
            tx.send(AppEvent::OpenPath(path)).ok();
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

    async fn handle_app_message(&mut self, msg: AppEvent, tx: &AppTx) -> Option<AppEvent> {
        if let AppEvent::IndexingDone(_) = &msg {
            self.overlay = None;
            let path = self
                .settings
                .last_paths
                .last()
                .map_or_else(VaultPath::root, |p| p.to_owned());
            tx.send(AppEvent::OpenPath(path)).ok();
            return None;
        }
        Some(msg)
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
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    async fn make_vault() -> Arc<NoteVault> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir().join(format!("kimun_start_test_{nonce}"));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(NoteVault::new(&dir).await.unwrap())
    }

    fn key_event(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    #[tokio::test]
    async fn on_enter_vault_none_sends_open_path() {
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        screen.on_enter(&tx).await;
        let msg = rx.try_recv().expect("expected a message");
        assert!(
            matches!(msg, AppEvent::OpenPath(_)),
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
        let mut screen = StartScreen::new(AppSettings::default(), Some(vault));
        screen.on_enter(&tx).await;
        assert!(
            matches!(screen.overlay, Some(IndexingProgressState::Running { .. })),
            "overlay should be Running after on_enter with vault"
        );
        // Drain all messages and ensure none are OpenPath
        let messages: Vec<AppEvent> =
            std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        let has_open_path = messages
            .iter()
            .any(|m| matches!(m, AppEvent::OpenPath(_)));
        assert!(
            !has_open_path,
            "OpenPath should not be sent immediately when vault is Some"
        );
    }

    #[tokio::test]
    async fn handle_app_message_indexing_done_ok_clears_overlay_and_sends_open_path() {
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        screen.overlay = Some(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        let result = screen
            .handle_app_message(AppEvent::IndexingDone(Ok(Duration::from_secs(1))), &tx)
            .await;
        assert!(result.is_none(), "IndexingDone should be consumed (return None)");
        assert!(screen.overlay.is_none(), "overlay should be cleared");
        let msg = rx.try_recv().expect("expected OpenPath message");
        assert!(
            matches!(msg, AppEvent::OpenPath(_)),
            "expected OpenPath after indexing done"
        );
    }

    #[tokio::test]
    async fn handle_app_message_indexing_done_err_clears_overlay_and_sends_open_path() {
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        screen.overlay = Some(IndexingProgressState::Running {
            work: tokio::spawn(async {}),
            ticker: tokio::spawn(async {}),
        });
        let result = screen
            .handle_app_message(AppEvent::IndexingDone(Err("fail".to_string())), &tx)
            .await;
        assert!(result.is_none(), "IndexingDone should be consumed (return None)");
        assert!(screen.overlay.is_none(), "overlay should be cleared on error");
        let msg = rx.try_recv().expect("expected OpenPath message");
        assert!(
            matches!(msg, AppEvent::OpenPath(_)),
            "expected OpenPath even after failed indexing"
        );
    }

    #[tokio::test]
    async fn handle_input_blocked_while_overlay_running() {
        let (tx, mut rx) = unbounded_channel::<AppEvent>();
        let mut screen = StartScreen::new(AppSettings::default(), None);
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
        let messages: Vec<AppEvent> =
            std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
        let has_non_redraw = messages
            .iter()
            .any(|m| !matches!(m, AppEvent::Redraw));
        assert!(
            !has_non_redraw,
            "handle_input should not send non-Redraw messages"
        );
    }

    #[tokio::test]
    async fn handle_input_not_consumed_while_overlay_none() {
        let (tx, _rx) = unbounded_channel::<AppEvent>();
        let mut screen = StartScreen::new(AppSettings::default(), None);
        screen.overlay = None;
        let state = screen.handle_input(&key_event(KeyCode::Enter), &tx);
        assert!(
            matches!(state, EventState::NotConsumed),
            "input should not be consumed when overlay is None"
        );
    }
}
