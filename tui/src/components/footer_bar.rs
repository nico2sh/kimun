//! The two-line **status bar** pinned to the bottom of the editor screen.
//!
//! Line 1 — context + actions: a focus-context indicator (`⌨ EDITOR` when a
//! text field holds the cursor, `≣ LIST` when a list/panel is focused)
//! followed by the focused surface's key hints, with the global hints
//! right-aligned. There is no editing "mode"; focus is the only state
//! (spec §7).
//!
//! Line 2 — document state: path · ln/col · modified/saved · backlink count
//! · git status · (in query contexts) match count.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::components::events::{AppEvent, AppTx};
use crate::components::hints::Hint;
use crate::settings::themes::Theme;

const FLASH_DURATION: Duration = Duration::from_secs(2);

/// Rows the status bar occupies.
pub const STATUS_BAR_HEIGHT: u16 = 2;

/// Document state shown on line 2. `None` fields render nothing — each
/// segment appears only when it has a value.
#[derive(Default)]
pub struct DocState<'a> {
    pub path: &'a str,
    pub dirty: bool,
    /// 1-based cursor line/column, when a text buffer holds the cursor.
    pub ln_col: Option<(usize, usize)>,
    /// Backlink count of the open note (async-loaded).
    pub backlinks: Option<usize>,
    /// Workspace git status summary, e.g. `git ✓` / `git ●3`.
    pub git: Option<String>,
    /// Result count when a query context is focused.
    pub matches: Option<usize>,
}

/// Everything the status bar shows for the current frame.
pub struct StatusContext<'a> {
    /// Label of the focused surface (panel or overlay), e.g. `EDITOR`.
    pub focus_label: &'a str,
    /// True when a text field holds the cursor (`⌨`); false for lists (`≣`).
    pub editing: bool,
    /// Key hints for the focused surface.
    pub hints: &'a [Hint],
    /// Always-on hints, right-aligned (from `hints::global_hints`).
    pub global_hints: &'a [Hint],
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
            global_hints,
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
        let keycap = Style::default().fg(theme.yellow.to_ratatui());

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
            // Right-aligned global hints first, so the left side knows how
            // much width remains.
            let mut right_spans: Vec<Span> = Vec::new();
            for (i, (key, label)) in global_hints.iter().enumerate() {
                if i > 0 {
                    right_spans.push(Span::styled("  ", secondary));
                }
                right_spans.push(Span::styled(format!("{key} "), keycap));
                right_spans.push(Span::styled(label.clone(), secondary));
            }
            let mut right_width: u16 = right_spans.iter().map(|s| s.content.width() as u16).sum();
            // Context hints outrank global hints: on a narrow terminal the
            // globals drop entirely rather than squeezing out the focus
            // indicator and the surface's own hints.
            const MIN_CONTEXT_WIDTH: u16 = 30;
            if right_width + 1 + MIN_CONTEXT_WIDTH > rows[0].width {
                right_spans.clear();
                right_width = 0;
            }
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(0), Constraint::Length(right_width + 1)])
                .split(rows[0]);

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
                    spans.push(Span::styled(format!("{key} "), keycap));
                    spans.push(Span::styled(label.clone(), secondary));
                }
            }
            f.render_widget(Paragraph::new(Line::from(spans)), cols[0]);
            f.render_widget(
                Paragraph::new(Line::from(right_spans)).alignment(Alignment::Right),
                cols[1],
            );
        }

        // ── Line 2: document state, `·`-separated segments ──────────────────
        // The path yields to the live segments: when the line would overflow,
        // the path is head-truncated with an ellipsis so ln/col, dirty state,
        // git, and match count stay visible.
        let tail_width: usize = {
            let mut w = 0usize;
            if let Some((ln, col)) = doc.ln_col {
                w += format!(" · ln {ln} col {col}").width();
            }
            w += if doc.dirty {
                " · ● modified".width()
            } else {
                " · ✓ saved".width()
            };
            if let Some(count) = doc.backlinks {
                w += format!(" · {count} backlinks").width();
            }
            if let Some(git) = &doc.git {
                w += " · ".width() + git.width();
            }
            if let Some(matches) = doc.matches {
                w += format!(" · {matches} matches").width();
            }
            w
        };
        let path_budget = (rect.width as usize).saturating_sub(tail_width + 1);
        let path_display = if doc.path.width() > path_budget {
            let keep: String = doc
                .path
                .chars()
                .rev()
                .scan(0usize, |acc, c| {
                    *acc += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                    (*acc < path_budget).then_some(c)
                })
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("…{keep}")
        } else {
            doc.path.to_string()
        };
        let mut segments: Vec<Span> = vec![Span::styled(format!(" {path_display}"), muted)];
        let push = |segments: &mut Vec<Span>, span: Span<'static>| {
            segments.push(Span::styled(" · ", muted));
            segments.push(span);
        };
        if let Some((ln, col)) = doc.ln_col {
            push(
                &mut segments,
                Span::styled(format!("ln {ln} col {col}"), muted),
            );
        }
        let state_span = if doc.dirty {
            Span::styled("● modified", Style::default().fg(theme.yellow.to_ratatui()))
        } else {
            Span::styled("✓ saved", Style::default().fg(theme.green.to_ratatui()))
        };
        push(&mut segments, state_span);
        if let Some(count) = doc.backlinks {
            push(
                &mut segments,
                Span::styled(format!("{count} backlinks"), muted),
            );
        }
        if let Some(git) = &doc.git {
            push(&mut segments, Span::styled(git.clone(), muted));
        }
        if let Some(matches) = doc.matches {
            push(
                &mut segments,
                Span::styled(
                    format!("{matches} matches"),
                    Style::default().fg(theme.fg_secondary.to_ratatui()),
                ),
            );
        }
        f.render_widget(Paragraph::new(Line::from(segments)), rows[1]);
    }
}

impl Default for FooterBar {
    fn default() -> Self {
        Self::new()
    }
}
