//! The two-line **status bar** pinned to the bottom of the editor screen.
//!
//! Line 1 — context + actions: a focus-context indicator (`⌨ EDITOR` when a
//! text field holds the cursor, `≣ LIST` when a list/panel is focused)
//! followed by the focused surface's key hints. There is no editing "mode";
//! focus is the only state (spec §7).
//!
//! Line 2 — document state: note path and modified/saved marker. Phase 04
//! enriches this with ln/col, backlink count, and git status.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::components::events::{AppEvent, AppTx};
use crate::settings::themes::Theme;

const FLASH_DURATION: Duration = Duration::from_secs(2);

/// Rows the status bar occupies.
pub const STATUS_BAR_HEIGHT: u16 = 2;

/// Document state shown on line 2.
pub struct DocState<'a> {
    pub path: &'a str,
    pub dirty: bool,
}

/// Everything the status bar shows for the current frame.
pub struct StatusContext<'a> {
    /// Label of the focused surface (panel or overlay), e.g. `EDITOR`.
    pub focus_label: &'a str,
    /// True when a text field holds the cursor (`⌨`); false for lists (`≣`).
    pub editing: bool,
    /// Key hints for the focused surface.
    pub hints: &'a [(String, String)],
    /// Document state for line 2.
    pub doc: DocState<'a>,
}

pub struct FooterBar {
    key_flash: Option<(String, Instant)>,
}

impl FooterBar {
    pub fn new() -> Self {
        Self { key_flash: None }
    }

    /// Show a key-flash message for 2 seconds. Schedules a delayed redraw so
    /// the message disappears even when no user input arrives in the meantime.
    pub fn flash(&mut self, text: String, tx: &AppTx) {
        self.key_flash = Some((text, Instant::now()));
        let tx2 = tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(FLASH_DURATION).await;
            let _ = tx2.send(AppEvent::Redraw);
        });
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, ctx: &StatusContext) {
        let StatusContext {
            focus_label,
            editing,
            hints,
            doc,
        } = ctx;
        // Expire stale key flash
        if let Some((_, instant)) = &self.key_flash
            && instant.elapsed() >= FLASH_DURATION
        {
            self.key_flash = None;
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(rect);

        let secondary = Style::default().fg(theme.fg_secondary.to_ratatui());
        let muted = Style::default().fg(theme.gray.to_ratatui());

        // ── Line 1: focus context + hints (or the key flash) ────────────────
        if let Some((flash, _)) = &self.key_flash {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    flash.as_str(),
                    Style::default()
                        .fg(theme.accent.to_ratatui())
                        .add_modifier(Modifier::BOLD),
                )))
                .alignment(Alignment::Center),
                rows[0],
            );
        } else {
            let glyph = if *editing { "⌨" } else { "≣" };
            let mut spans = vec![Span::styled(
                format!(" {glyph} {focus_label}  "),
                Style::default()
                    .fg(theme.fg_bright.to_ratatui())
                    .add_modifier(Modifier::BOLD),
            )];
            let sep = Span::styled("  ", secondary);
            for (i, (key, label)) in hints.iter().enumerate() {
                if i > 0 {
                    spans.push(sep.clone());
                }
                if key.is_empty() {
                    // Mode / command-line label from the nvim backend — make it pop.
                    spans.push(Span::styled(
                        format!(" {label} "),
                        Style::default()
                            .fg(theme.accent.to_ratatui())
                            .add_modifier(Modifier::BOLD),
                    ));
                } else {
                    spans.push(Span::styled(
                        format!("{key} "),
                        Style::default().fg(theme.yellow.to_ratatui()),
                    ));
                    spans.push(Span::styled(label.clone(), secondary));
                }
            }
            f.render_widget(Paragraph::new(Line::from(spans)), rows[0]);
        }

        // ── Line 2: document state ──────────────────────────────────────────
        let state_span = if doc.dirty {
            Span::styled("● modified", Style::default().fg(theme.yellow.to_ratatui()))
        } else {
            Span::styled("✓ saved", Style::default().fg(theme.green.to_ratatui()))
        };
        let line2 = Line::from(vec![
            Span::styled(format!(" {} ", doc.path), muted),
            Span::styled("· ", muted),
            state_span,
        ]);
        f.render_widget(Paragraph::new(line2), rows[1]);
    }
}

impl Default for FooterBar {
    fn default() -> Self {
        Self::new()
    }
}
