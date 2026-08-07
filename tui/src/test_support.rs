#![cfg(test)]
//! Shared test helpers (vault setup, input event constructors).

use std::sync::Arc;

use kimun_core::{NoteVault, SystemPath, VaultConfig};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

use crate::components::events::InputEvent;

/// A [`SystemPath`] for a path a test has already made absolute (a `TempDir`,
/// a host literal). Panics rather than returning a `Result`: a test handing
/// over a relative path is a broken test, not a failure case.
pub fn sys<P: AsRef<std::path::Path>>(path: P) -> SystemPath {
    SystemPath::try_absolute(&path).unwrap_or_else(|e| panic!("test path must be absolute: {e}"))
}

/// Spawn a fresh `NoteVault` rooted in a per-test temp directory.
/// `prefix` names the caller in the directory, for anyone reading a temp dir.
///
/// The unique part comes from `tempfile`, which creates the directory
/// atomically under a random name and retries on collision. Deriving it from
/// the pid and a counter instead looks unique and is not: nextest runs every
/// test in its own process, so the counter is almost always 0, nothing here
/// ever cleaned the directory up, and Windows recycles pids briskly over a
/// several-thousand-process run — a later test with the same prefix inherited
/// the earlier one's notes and failed with `NoteExists`. It reached CI as a
/// test that failed roughly one run in four.
///
/// The directory is deliberately leaked rather than guarded: callers hold the
/// vault, not a `TempDir`, and the OS clears its own temp directory.
pub async fn temp_vault(prefix: &str) -> Arc<NoteVault> {
    let dir = tempfile::Builder::new()
        .prefix(&format!("kimun_{prefix}_test_"))
        .tempdir()
        .unwrap()
        .keep();
    Arc::new(NoteVault::new(VaultConfig::new(sys(&dir))).await.unwrap())
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
