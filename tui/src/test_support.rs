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

/// A left press at (col,row) as a raw `MouseEvent`, for widgets that take
/// one directly ([`mouse_down_at`] wraps the same press in an `InputEvent`).
pub fn left_press(col: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Pointer motion to (col,row) — hover, with no button.
pub fn mouse_moved_at(col: u16, row: u16) -> InputEvent {
    InputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// Every event sent so far on `rx`.
pub fn drain(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<crate::components::events::AppEvent>,
) -> Vec<crate::components::events::AppEvent> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

pub fn mouse_down_at(col: u16, row: u16) -> InputEvent {
    InputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// The first cell where `text` is drawn in `buf` (matched cell by cell, so
/// wide glyphs line up), or `None`.
pub fn find_text(buf: &ratatui::buffer::Buffer, text: &str) -> Option<(u16, u16)> {
    let want: Vec<String> = text.chars().map(|c| c.to_string()).collect();
    let area = buf.area;
    (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
        .find(|&(x, y)| {
            want.iter().enumerate().all(|(i, c)| {
                let cx = x + i as u16;
                cx < area.right() && buf[(cx, y)].symbol() == c
            })
        })
}
