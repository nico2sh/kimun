//! `ThreadPanel` — the editor area's Ask-workspace content (see CONTEXT.md:
//! **Ask workspace**, **Thread**; adr/0030). Owns the conversation `Thread`
//! and the docked question composer. State-only skeleton for now: render
//! draws a placeholder line and input is a no-op — Task 9 wires up the real
//! composer submit / turn navigation / regenerate behavior.
//!
//! `PanelSet` hands this back to its caller via `take_ask` (rather than
//! dropping it, the way `clear_attachment` drops an `AttachmentView`) so the
//! conversation survives the user switching to another editor-area view.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ask::Thread;
use crate::components::Component;
use crate::components::single_line_input::SingleLineInput;
use crate::settings::themes::Theme;

/// Which part of the Ask workspace has keyboard focus within the editor
/// area: the question composer, or the turn list above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadFocus {
    Composer,
    Turns,
}

/// The Ask workspace's editor-area content: the conversation `Thread` plus
/// the docked question composer. See the module doc for lifetime notes.
pub struct ThreadPanel {
    thread: Thread,
    /// Wired up in Task 9 (question submit).
    #[allow(dead_code)]
    composer: SingleLineInput,
    /// Whether the Ask capability (Kimün server reachable with an LLM
    /// configured) is currently available. Losing it disables the composer
    /// without evicting the thread — the thread's answers are already local
    /// (CONTEXT.md: **Ask workspace**).
    capability: bool,
    /// Wired up in Task 9 (composer vs. turn-list navigation).
    #[allow(dead_code)]
    focus: ThreadFocus,
}

impl ThreadPanel {
    pub fn new() -> Self {
        Self {
            thread: Thread::default(),
            composer: SingleLineInput::new(),
            capability: true,
            focus: ThreadFocus::Composer,
        }
    }

    /// Update whether the Ask capability is currently available. See
    /// adr/0030: losing capability disables the composer but never evicts
    /// the thread.
    pub fn set_capability(&mut self, on: bool) {
        self.capability = on;
    }

    pub fn thread(&self) -> &Thread {
        &self.thread
    }

    pub fn thread_mut(&mut self) -> &mut Thread {
        &mut self.thread
    }
}

impl Default for ThreadPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for ThreadPanel {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let msg = if self.capability {
            "Ask workspace — coming soon"
        } else {
            "Ask workspace — unavailable (no capability)"
        };
        let line = Line::from(Span::styled(
            msg,
            Style::default().fg(theme.fg_secondary.to_ratatui()),
        ));
        f.render_widget(Paragraph::new(line), rect);
    }

    // `handle_input` and `hint_shortcuts` use the `Component` defaults
    // (no-op / empty) until Task 9.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_thread_panel_starts_empty_with_capability_and_composer_focus() {
        let panel = ThreadPanel::new();
        assert!(panel.thread().is_empty());
        assert!(panel.capability);
        assert_eq!(panel.focus, ThreadFocus::Composer);
    }

    #[test]
    fn set_capability_toggles_the_flag() {
        let mut panel = ThreadPanel::new();
        panel.set_capability(false);
        assert!(!panel.capability);
        panel.set_capability(true);
        assert!(panel.capability);
    }

    #[test]
    fn thread_mut_allows_mutating_the_conversation() {
        let mut panel = ThreadPanel::new();
        panel.thread_mut().ask("q?".to_string());
        assert_eq!(panel.thread().turns().len(), 1);
    }
}
