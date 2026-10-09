//! The two-line **status bar** pinned to the bottom of the editor screen.
//!
//! Line 1 — context + actions: a focus-context indicator (`⌨ EDITOR` when a
//! text field holds the cursor, `≣ LIST` when a list/panel is focused)
//! followed by the focused surface's key hints, with the global hints
//! right-aligned. There is no editing "mode"; focus is the only state.
//!
//! Line 2 — document state: path · ln/col · modified/saved · backlink count
//! · git status · (in query contexts) match count.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_segmentation::UnicodeSegmentation;
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
    /// Property count of the open note — renders the clickable `⊞ N props`.
    pub props: Option<usize>,
    /// Workspace git status summary, e.g. `git ✓` / `git ●3`.
    pub git: Option<String>,
    /// Result count when a query context is focused.
    pub matches: Option<usize>,
    /// Link-under-cursor affordance: `→ target · N backlinks`.
    pub link: Option<String>,
    /// Newer release available, e.g. `⬆ 0.18.0` — clickable, opens the
    /// update dialog.
    pub update: Option<String>,
    /// RAG server status, e.g. `rag: online` — absent when no server is set.
    pub rag: Option<String>,
    /// Set when the server reports an update: replaces the `rag` status with
    /// this clickable text (opens the server-update dialog).
    pub rag_update: Option<String>,
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

/// A line-2 segment that reacts to a click.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FooterTarget {
    /// `⊞ N props` — opens the properties dialog.
    Props,
    /// `⬆ x.y.z` — opens the update dialog.
    Update,
    /// `rag: server update` — opens the server-update dialog.
    ServerUpdate,
    /// `N backlinks` — opens the LINKS drawer on its backlinks tab.
    Backlinks,
    /// `→ target` — follows the link under the cursor (the mouse's Ctrl+N).
    Link,
}

pub struct FooterBar {
    key_flash: Option<(String, Instant)>,
    /// Clickable segments from the last render, clipped to the row.
    targets: Vec<(Rect, FooterTarget)>,
}

impl FooterBar {
    pub fn new() -> Self {
        Self {
            key_flash: None,
            targets: Vec::new(),
        }
    }

    /// The flash message being shown, if any (expiry not checked).
    #[cfg(test)]
    pub fn flash_text(&self) -> Option<&str> {
        self.key_flash.as_ref().map(|(t, _)| t.as_str())
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
            if let Some(n) = doc.props {
                w += " · ".width() + props_label(n).width();
            }
            if let Some(count) = doc.backlinks {
                w += format!(" · {count} backlinks").width();
            }
            if let Some(git) = &doc.git {
                w += " · ".width() + git.width();
            }
            if let Some(matches) = doc.matches {
                w += format!(" · {matches} matches").width();
            }
            if let Some(update) = &doc.update {
                w += " · ".width() + update.width();
            }
            if let Some(rag) = doc.rag_update.as_ref().or(doc.rag.as_ref()) {
                w += " · ".width() + rag.width();
            }
            w
        };
        let path_budget = (rect.width as usize).saturating_sub(tail_width + 1);
        let path_display = fit_path(doc.path, path_budget);
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
        self.targets.clear();
        // Where the next pushed segment will land: everything already pushed
        // plus the separator `push` puts in front of it.
        let row = rows[1];
        let target_rect = |segments: &[Span], label: &str| -> Option<Rect> {
            let x_before: u16 = segments
                .iter()
                .map(|s| s.content.width() as u16)
                .sum::<u16>()
                + " · ".width() as u16;
            let x = row.x.saturating_add(x_before);
            (x < row.right()).then(|| Rect {
                x,
                y: row.y,
                width: (label.width() as u16).min(row.right() - x),
                height: 1,
            })
        };
        if let Some(n) = doc.props {
            let label = props_label(n);
            if let Some(r) = target_rect(&segments, &label) {
                self.targets.push((r, FooterTarget::Props));
            }
            push(&mut segments, Span::styled(label, theme.action()));
        }
        if let Some(count) = doc.backlinks {
            let label = format!("{count} backlinks");
            if let Some(r) = target_rect(&segments, &label) {
                self.targets.push((r, FooterTarget::Backlinks));
            }
            push(&mut segments, Span::styled(label, theme.action()));
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
        if let Some(link) = &doc.link {
            if let Some(r) = target_rect(&segments, link) {
                self.targets.push((r, FooterTarget::Link));
            }
            push(&mut segments, Span::styled(link.clone(), theme.action()));
        }
        if let Some(update) = &doc.update {
            if let Some(r) = target_rect(&segments, update) {
                self.targets.push((r, FooterTarget::Update));
            }
            push(
                &mut segments,
                Span::styled(update.clone(), theme.action().add_modifier(Modifier::BOLD)),
            );
        }
        if let Some(label) = &doc.rag_update {
            if let Some(r) = target_rect(&segments, label) {
                self.targets.push((r, FooterTarget::ServerUpdate));
            }
            push(&mut segments, Span::styled(label.clone(), theme.action()));
        } else if let Some(rag) = &doc.rag {
            push(
                &mut segments,
                Span::styled(rag.clone(), Style::default().fg(theme.green.to_ratatui())),
            );
        }
        for (r, _) in &self.targets {
            crate::components::clickable::register(*r);
        }
        f.render_widget(Paragraph::new(Line::from(segments)), rows[1]);
    }
}

fn props_label(n: usize) -> String {
    if n == 0 {
        "⊞ props".to_string()
    } else {
        format!("⊞ {n} props")
    }
}

impl FooterBar {
    /// The clickable segment under (col,row) from the last render.
    pub fn target_at(&self, col: u16, row: u16) -> Option<FooterTarget> {
        crate::components::clickable::target_at(&self.targets, col, row)
    }
}

impl Default for FooterBar {
    fn default() -> Self {
        Self::new()
    }
}

/// Fit `path` into `budget` display columns for the footer. When it overflows,
/// keep the trailing portion (the note-name end is the most useful part) and
/// prefix it with `…`. Truncation lands on grapheme-cluster boundaries and
/// measures by rendered width, so a multi-codepoint cluster (emoji presentation
/// sequence, combining mark) is never split or reordered.
fn fit_path(path: &str, budget: usize) -> String {
    if path.width() <= budget {
        return path.to_string();
    }
    let mut acc = 0usize;
    let keep: String = path
        .graphemes(true)
        .rev()
        .take_while(|g| {
            acc += g.width();
            acc < budget
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{keep}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_segment_is_clickable_where_drawn() {
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::gruvbox_dark();
        let mut bar = FooterBar::new();
        let mut t = Terminal::new(TestBackend::new(100, 2)).unwrap();
        let ctx = StatusContext {
            focus_label: "EDITOR",
            editing: true,
            hints: &[],
            global_hints: &[],
            doc: DocState {
                path: "n.md",
                props: Some(3),
                ..Default::default()
            },
        };
        t.draw(|f| bar.render(f, f.area(), &theme, &ctx)).unwrap();
        let buf = t.backend().buffer().clone();
        let (col, _) = crate::test_support::find_text(&buf, "⊞ 3 props").expect("segment drawn");
        assert_eq!(bar.target_at(col, 1), Some(FooterTarget::Props));
        assert_eq!(bar.target_at(col + 8, 1), Some(FooterTarget::Props));
        assert_eq!(bar.target_at(col.saturating_sub(2), 1), None);
        assert_eq!(bar.target_at(col, 0), None, "line 1 is not the segment");
    }

    #[test]
    fn update_segment_is_clickable_where_drawn() {
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::gruvbox_dark();
        let mut bar = FooterBar::new();
        let mut t = Terminal::new(TestBackend::new(100, 2)).unwrap();
        let ctx = StatusContext {
            focus_label: "EDITOR",
            editing: true,
            hints: &[],
            global_hints: &[],
            doc: DocState {
                path: "n.md",
                props: Some(3),
                update: Some("⬆ 9.9.9".into()),
                ..Default::default()
            },
        };
        t.draw(|f| bar.render(f, f.area(), &theme, &ctx)).unwrap();
        let buf = t.backend().buffer().clone();
        let (col, _) = crate::test_support::find_text(&buf, "⬆ 9.9.9").expect("segment drawn");
        assert_eq!(bar.target_at(col, 1), Some(FooterTarget::Update));
        assert_eq!(bar.target_at(col + 6, 1), Some(FooterTarget::Update));
        assert_eq!(bar.target_at(col.saturating_sub(2), 1), None);
    }

    #[test]
    fn backlinks_and_link_segments_are_clickable() {
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::gruvbox_dark();
        let mut bar = FooterBar::new();
        let mut t = Terminal::new(TestBackend::new(120, 2)).unwrap();
        let ctx = StatusContext {
            focus_label: "EDITOR",
            editing: true,
            hints: &[],
            global_hints: &[],
            doc: DocState {
                path: "n.md",
                backlinks: Some(4),
                link: Some("→ other".into()),
                ..Default::default()
            },
        };
        t.draw(|f| bar.render(f, f.area(), &theme, &ctx)).unwrap();
        let buf = t.backend().buffer().clone();
        let at = |s: &str| crate::test_support::find_text(&buf, s).expect("drawn").0;
        assert_eq!(
            bar.target_at(at("4 backlinks"), 1),
            Some(FooterTarget::Backlinks)
        );
        assert_eq!(
            bar.target_at(at("→ other") + 2, 1),
            Some(FooterTarget::Link)
        );
    }

    #[test]
    fn no_props_segment_without_count() {
        use ratatui::{Terminal, backend::TestBackend};
        let theme = Theme::gruvbox_dark();
        let mut bar = FooterBar::new();
        let mut t = Terminal::new(TestBackend::new(100, 2)).unwrap();
        let ctx = StatusContext {
            focus_label: "EDITOR",
            editing: true,
            hints: &[],
            global_hints: &[],
            doc: DocState {
                path: "n.md",
                ..Default::default()
            },
        };
        t.draw(|f| bar.render(f, f.area(), &theme, &ctx)).unwrap();
        assert!(!(0..100).any(|c| bar.target_at(c, 1).is_some()));
    }

    #[test]
    fn short_path_returned_whole() {
        assert_eq!(fit_path("notes/foo", 20), "notes/foo");
    }

    #[test]
    fn overflowing_ascii_path_keeps_trailing_with_ellipsis() {
        // "abcdefghij" (10 cols) into budget 5: keep trailing clusters while
        // cumulative width stays strictly < 5 → "ghij" (4 cols); "f" would
        // reach 5 and stop. Matches the pre-extraction truncation behavior.
        assert_eq!(fit_path("abcdefghij", 5), "…ghij");
    }

    #[test]
    fn cjk_width_counted_as_two_columns() {
        // "猫猫猫" is 6 display cols; budget 6 fits whole (no ellipsis).
        assert_eq!(fit_path("猫猫猫", 6), "猫猫猫");
    }

    #[test]
    fn emoji_cluster_not_split_or_reordered() {
        // Flag 🇪🇸 = two regional indicators, one cluster (2 cols). Budget 2 on
        // "z🇪🇸" forces the cut to land *inside* the flag. A per-codepoint
        // truncation would keep a lone regional indicator (🇸 — half a flag);
        // grapheme-aware truncation must never emit a partial cluster, so the
        // flag is kept whole or dropped entirely (here: dropped → just "…").
        let es = "\u{1F1F8}"; // ES regional indicator (the 2nd half)
        let flag = "\u{1F1EA}\u{1F1F8}";
        let path = format!("z{flag}");
        let out = fit_path(&path, 2);
        assert!(out.starts_with('…'), "expected ellipsis prefix: {out:?}");
        assert!(
            !out.contains(es) || out.contains(flag),
            "regional indicator emitted without its full flag cluster: {out:?}"
        );
    }
}
