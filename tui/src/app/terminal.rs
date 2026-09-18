//! The **Terminal session** (CONTEXT.md § App shell): the terminal state the
//! TUI holds while a **Screen** is up, entered once and left symmetrically.
//!
//! A guard, so the leave order cannot drift from the enter order and cannot be
//! forgotten on one exit path. Before this type existed the teardown was
//! written twice (normal exit and the panic hook) and skipped once (the loop
//! returning `Err` went straight back to `main` in raw mode).

use std::io::{self, Stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Whether a **Terminal session** is currently up. Process-global because the
/// panic hook has no handle on the session — it only knows a session might
/// exist. Set once the terminal state starts changing, cleared by whoever
/// leaves first.
static SESSION_LIVE: AtomicBool = AtomicBool::new(false);

pub struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    keyboard_enhanced: bool,
}

impl TerminalSession {
    /// Raw mode, the alternate screen, bracketed paste, the kitty
    /// keyboard-enhancement flags where the terminal supports them, and — when
    /// the user has not opted out (ADR-0015) — mouse capture.
    pub fn enter(mouse_capture: bool) -> io::Result<Self> {
        enable_raw_mode()?;
        // From here on the terminal is modified, so a panic must restore it.
        SESSION_LIVE.store(true, Ordering::SeqCst);
        // Anything that fails past this point must undo raw mode, or the
        // error report prints into a terminal that no longer echoes.
        Self::enter_after_raw_mode(mouse_capture).inspect_err(|_| leave_if_live(&mut io::stdout()))
    }

    fn enter_after_raw_mode(mouse_capture: bool) -> io::Result<Self> {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
        // Required to receive F-keys and other special keys correctly in
        // terminals that speak the protocol (kitty, WezTerm).
        // Whether they went out is not only a rendering detail: with the
        // protocol on, the terminal spells Backspace and Ctrl-H differently,
        // which is what lets `ctrl_h::CtrlHPolicy` leave both keys working.
        //
        // So the flag records the *write*, not merely the terminal's answer to
        // the capability query. A push that fails leaves the session speaking
        // legacy bytes, and reporting it as enhanced would have `auto` skip
        // the erase-character probe and conclude the keys are distinguishable
        // — leaving a `^H`-Backspace user without the fix and no way to tell.
        let keyboard_enhanced = supports_keyboard_enhancement().unwrap_or(false)
            && execute!(
                stdout,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )
            .inspect_err(|e| tracing::warn!("keyboard enhancement push failed: {e}"))
            .is_ok();
        // Mouse reporting is all-or-nothing: enabling it suppresses the
        // terminal's native selection and middle-click paste.
        if mouse_capture {
            let _ = execute!(stdout, EnableMouseCapture);
        }
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self {
            terminal,
            keyboard_enhanced,
        })
    }

    pub fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }

    /// Whether the kitty keyboard-enhancement flags were successfully pushed
    /// — i.e. whether this session receives keys unambiguously. Read once at
    /// startup to resolve the Ctrl-H / Backspace tie (`app::ctrl_h`).
    pub fn keyboard_enhanced(&self) -> bool {
        self.keyboard_enhanced
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        leave_if_live(self.terminal.backend_mut());
    }
}

/// Leave once, whoever gets there first: the guard's `Drop` on a normal
/// exit, the panic hook on an unwinding main thread. A second caller finds
/// the flag already cleared and does nothing, so the teardown is never
/// written twice.
pub(crate) fn leave_if_live<W: Write>(w: &mut W) {
    if SESSION_LIVE.swap(false, Ordering::SeqCst) {
        leave(w);
    }
}

/// Undo everything [`TerminalSession::enter`] did. Best-effort and idempotent:
/// every step's error is ignored, because this runs on the way out — from
/// `Drop`, and from the panic hook, which aims it at stderr since it has no
/// backend to hand.
pub fn leave<W: Write>(w: &mut W) {
    let _ = disable_raw_mode();
    // On its own: on Windows this command is unsupported and would
    // short-circuit everything after it in one `execute!` chain.
    let _ = execute!(w, PopKeyboardEnhancementFlags);
    let _ = execute!(
        w,
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

    /// No session was ever entered — the CLI and MCP paths, and every test —
    /// so the panic hook must write nothing. The flag is process-global, so
    /// this test only ever reads it; nothing here sets it.
    #[test]
    fn leave_if_live_is_a_noop_when_no_session_was_entered() {
        let mut out: Vec<u8> = Vec::new();
        leave_if_live(&mut out);
        assert!(out.is_empty(), "wrote teardown with no session: {out:?}");
    }
}
