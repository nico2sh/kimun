#![cfg(test)]
//! Shared test helpers (vault setup, input event constructors).

use std::sync::Arc;

use kimun_core::{NoteVault, VaultConfig};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

use crate::components::events::InputEvent;

/// Spawn a fresh `NoteVault` rooted in a per-test temp directory.
/// `prefix` disambiguates concurrent test names sharing the same temp dir.
pub async fn temp_vault(prefix: &str) -> Arc<NoteVault> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let thread_id = std::thread::current().id();
    let dir = std::env::temp_dir().join(format!("kimun_{prefix}_test_{nonce}_{thread_id:?}"));
    std::fs::create_dir_all(&dir).unwrap();
    Arc::new(NoteVault::new(VaultConfig::new(&dir)).await.unwrap())
}

#[allow(dead_code)]
pub fn key_event(code: KeyCode) -> InputEvent {
    InputEvent::Key(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

pub fn mouse_down_at(col: u16, row: u16) -> InputEvent {
    InputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    })
}
