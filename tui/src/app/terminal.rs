//! The **Terminal session** (CONTEXT.md § App shell): the terminal state the
//! TUI holds while a **Screen** is up, entered once and left symmetrically.
//!
//! A guard, so the leave order cannot drift from the enter order and cannot be
//! forgotten on one exit path. Before this type existed the teardown was
//! written twice (normal exit and the panic hook) and skipped once (the loop
//! returning `Err` went straight back to `main` in raw mode).

use std::io::{self, Stdout, Write};

use crossterm::cursor::{SetCursorStyle, Show};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::prelude::CrosstermBackend;

pub struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    /// Raw mode, the alternate screen, bracketed paste, the kitty
    /// keyboard-enhancement flags where the terminal supports them, and — when
    /// the user has not opted out (ADR-0015) — mouse capture.
    pub fn enter(mouse_capture: bool) -> io::Result<Self> {
        enable_raw_mode()?;
        // Anything that fails past this point must undo raw mode, or the
        // error report prints into a terminal that no longer echoes.
        Self::enter_after_raw_mode(mouse_capture).inspect_err(|_| leave(&mut io::stdout()))
    }

    fn enter_after_raw_mode(mouse_capture: bool) -> io::Result<Self> {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
        // Required to receive F-keys and other special keys correctly in
        // terminals that speak the protocol (kitty, WezTerm).
        if supports_keyboard_enhancement().unwrap_or(false) {
            let _ = execute!(
                stdout,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
        }
        // Mouse reporting is all-or-nothing: enabling it suppresses the
        // terminal's native selection and middle-click paste.
        if mouse_capture {
            let _ = execute!(stdout, EnableMouseCapture);
        }
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }

    pub fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        leave(self.terminal.backend_mut());
    }
}

/// Undo everything [`TerminalSession::enter`] did. Best-effort and idempotent:
/// every step's error is ignored, because this runs on the way out — from
/// `Drop`, and from the panic hook, which aims it at stderr since it has no
/// backend to hand.
pub fn leave<W: Write>(w: &mut W) {
    let _ = disable_raw_mode();
    let _ = execute!(
        w,
        PopKeyboardEnhancementFlags,
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste,
        SetCursorStyle::DefaultUserShape,
        Show,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The leave sequence is written to whatever writer it is given, so the
    /// panic hook can aim it at stderr while `Drop` aims it at the backend.
    /// On Windows crossterm routes commands through the console API when the
    /// writer is not an ANSI console, so the byte check only holds on unix.
    #[cfg(not(windows))]
    #[test]
    fn leave_writes_every_teardown_sequence() {
        let mut out: Vec<u8> = Vec::new();
        leave(&mut out);
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("\x1b[?1049l"),
            "LeaveAlternateScreen missing: {s:?}"
        );
        assert!(
            s.contains("\x1b[?2004l"),
            "DisableBracketedPaste missing: {s:?}"
        );
        assert!(
            s.contains("\x1b[?1000l"),
            "DisableMouseCapture missing: {s:?}"
        );
        assert!(s.contains("\x1b[?25h"), "Show missing: {s:?}");
        assert!(
            s.contains("\x1b[0 q"),
            "SetCursorStyle::DefaultUserShape missing: {s:?}"
        );
        assert!(
            s.contains("\x1b[<1u"),
            "PopKeyboardEnhancementFlags missing: {s:?}"
        );
    }

    #[test]
    fn leave_is_best_effort_off_a_terminal() {
        // Not a tty: disable_raw_mode fails and must be ignored, not propagated.
        let mut out: Vec<u8> = Vec::new();
        leave(&mut out);
        leave(&mut out); // idempotent
    }
}
