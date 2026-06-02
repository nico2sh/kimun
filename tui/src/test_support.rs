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
    use std::sync::atomic::{AtomicU64, Ordering};
    // A process-wide monotonic counter guarantees a unique temp dir per call,
    // regardless of timing or thread scheduling. (A sub-second timestamp wraps
    // every second and collides under parallel test runs.)
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("kimun_{prefix}_test_{pid}_{nonce}"));
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
