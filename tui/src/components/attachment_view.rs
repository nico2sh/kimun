//! `AttachmentView` — the read-only surface the editor area shows when an
//! **Attachment** is opened (see CONTEXT.md), in place of the text editor.
//! Renders the attachment's metadata plus, for text files, a scrollable
//! preview of its content; binary files show metadata only. It never edits:
//! the attachment's verb is *open externally* (**FollowLink**, default Ctrl+N),
//! handled by the editor screen.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use kimun_core::nfs::VaultPath;
use kimun_core::{AttachmentContent, AttachmentDetails};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// How many lines a PageUp/PageDown leaves visible from the previous view.
const PAGE_OVERLAP: u16 = 2;

pub struct AttachmentView {
    details: AttachmentDetails,
    icons: Icons,
    key_bindings: KeyBindings,
    /// Topmost preview line shown (vertical scroll offset).
    scroll: u16,
    /// Preview body height in rows from the last render — clamps scrolling.
    viewport_height: u16,
    /// Total preview lines, computed once from the content.
    total_lines: u16,
}

impl AttachmentView {
    pub fn new(details: AttachmentDetails, icons: Icons, key_bindings: KeyBindings) -> Self {
        let total_lines = match &details.content {
            AttachmentContent::Text { text, .. } => {
                // `lines()` drops a trailing newline; count at least 1 so an
                // empty file still occupies a row.
                text.lines().count().max(1) as u16
            }
            AttachmentContent::Binary => 0,
        };
        Self {
            details,
            icons,
            key_bindings,
            scroll: 0,
            viewport_height: 0,
            total_lines,
        }
    }

    /// The opened attachment's vault path — used by the editor screen to open
    /// it with the OS default program.
    pub fn path(&self) -> &VaultPath {
        &self.details.path
    }

    /// Largest valid scroll offset given the last viewport height.
    fn max_scroll(&self) -> u16 {
        self.total_lines.saturating_sub(self.viewport_height)
    }

    fn scroll_by(&mut self, delta: i32) {
        let next = (self.scroll as i32 + delta).clamp(0, self.max_scroll() as i32);
        self.scroll = next as u16;
    }

    /// The metadata header lines shown above the preview.
    fn header_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let label = Style::default().fg(theme.gray.to_ratatui());
        let value = Style::default().fg(theme.fg.to_ratatui());
        let filename = self.details.path.get_parent_path().1;

        let kv = |k: &str, v: String| {
            Line::from(vec![
                Span::styled(format!("{k:<10}"), label),
                Span::styled(v, value),
            ])
        };

        let type_label = match &self.details.extension {
            Some(ext) => ext.to_uppercase(),
            None => "(no extension)".to_string(),
        };

        vec![
            Line::from(vec![
                Span::styled(
                    format!("{} ", self.icons.attachment),
                    Style::default().fg(theme.accent.to_ratatui()),
                ),
                Span::styled(
                    filename,
                    Style::default()
                        .fg(theme.fg_bright.to_ratatui())
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            kv("Path", self.details.path.to_string()),
            kv("Size", human_size(self.details.size)),
            kv("Modified", format_mtime(self.details.modified_secs)),
            kv("Type", type_label),
        ]
    }
}

impl Component for AttachmentView {
    fn handle_input(&mut self, event: &InputEvent, _tx: &AppTx) -> EventState {
        let page = self.viewport_height.saturating_sub(PAGE_OVERLAP).max(1) as i32;
        match event {
            InputEvent::Key(key) => match key.code {
                KeyCode::Up => self.scroll_by(-1),
                KeyCode::Down => self.scroll_by(1),
                KeyCode::PageUp => self.scroll_by(-page),
                KeyCode::PageDown => self.scroll_by(page),
                KeyCode::Home => self.scroll = 0,
                KeyCode::End => self.scroll = self.max_scroll(),
                _ => return EventState::NotConsumed,
            },
            InputEvent::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => self.scroll_by(-1),
                MouseEventKind::ScrollDown => self.scroll_by(1),
                _ => return EventState::NotConsumed,
            },
            _ => return EventState::NotConsumed,
        }
        EventState::Consumed
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let header = self.header_lines(theme);
        let header_height = header.len() as u16;

        // Header on top (fixed), a one-row gap, then the preview body.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(rect);

        f.render_widget(Paragraph::new(header), chunks[0]);

        let body = chunks[2];
        match &self.details.content {
            AttachmentContent::Text { text, truncated } => {
                let title = if *truncated {
                    " preview — truncated, open externally for the full file "
                } else {
                    " preview "
                };
                let block = Block::default()
                    .borders(Borders::TOP)
                    .title(title)
                    .border_style(Style::default().fg(theme.border_dim.to_ratatui()))
                    .title_style(Style::default().fg(theme.gray.to_ratatui()));
                let inner = block.inner(body);
                f.render_widget(block, body);
                self.viewport_height = inner.height;
                // Re-clamp after a resize so a shrunk viewport can't strand the
                // scroll past the new bottom.
                self.scroll = self.scroll.min(self.max_scroll());
                f.render_widget(
                    Paragraph::new(text.clone())
                        .style(Style::default().fg(theme.fg.to_ratatui()))
                        .scroll((self.scroll, 0)),
                    inner,
                );
            }
            AttachmentContent::Binary => {
                self.viewport_height = 0;
                let key = self
                    .key_bindings
                    .first_combo_for(&ActionShortcuts::FollowLink)
                    .unwrap_or_else(|| "the open key".to_string());
                let msg = Paragraph::new(vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        "Binary file — no preview.",
                        Style::default().fg(theme.fg_secondary.to_ratatui()),
                    )),
                    Line::from(Span::styled(
                        format!("Press {key} to open it with the default program."),
                        Style::default().fg(theme.gray.to_ratatui()),
                    )),
                ]);
                f.render_widget(msg, body);
            }
        }
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        crate::components::hints::hints_for(
            &self.key_bindings,
            &[(ActionShortcuts::FollowLink, "open externally")],
        )
    }
}

/// Formats a byte count as a human-readable size (`2.3 MB`, `512 B`).
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

/// Formats a Unix-second timestamp as a local-agnostic `YYYY-MM-DD HH:MM` (UTC).
fn format_mtime(secs: u64) -> String {
    match chrono::DateTime::from_timestamp(secs as i64, 0) {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_scales_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(2_411_724), "2.3 MB");
    }

    fn text_view(text: &str) -> AttachmentView {
        let details = AttachmentDetails {
            path: VaultPath::new("notes.txt"),
            size: text.len() as u64,
            modified_secs: 0,
            extension: Some("txt".to_string()),
            content: AttachmentContent::Text {
                text: text.to_string(),
                truncated: false,
            },
        };
        AttachmentView::new(details, Icons::new(false), KeyBindings::empty())
    }

    #[test]
    fn scroll_clamps_to_content() {
        let mut v = text_view("a\nb\nc\nd\ne");
        v.viewport_height = 2; // 5 lines, 2 visible -> max scroll 3
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Scrolling up from the top is a no-op.
        v.scroll_by(-1);
        assert_eq!(v.scroll, 0);
        // End jumps to the bottom-most valid offset.
        v.scroll = v.max_scroll();
        assert_eq!(v.scroll, 3);
        // Cannot scroll past the bottom.
        v.scroll_by(10);
        assert_eq!(v.scroll, 3);

        // A scroll-down mouse event is consumed and advances one line.
        v.scroll = 0;
        let ev = InputEvent::Mouse(ratatui::crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        });
        assert_eq!(v.handle_input(&ev, &tx), EventState::Consumed);
        assert_eq!(v.scroll, 1);
    }

    #[test]
    fn binary_view_has_no_preview_lines() {
        let details = AttachmentDetails {
            path: VaultPath::new("blob.bin"),
            size: 3,
            modified_secs: 0,
            extension: Some("bin".to_string()),
            content: AttachmentContent::Binary,
        };
        let v = AttachmentView::new(details, Icons::new(false), KeyBindings::empty());
        assert_eq!(v.total_lines, 0);
    }
}
