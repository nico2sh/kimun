use std::sync::Arc;
use std::sync::mpsc::Receiver;

use async_trait::async_trait;
use chrono::NaiveDate;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::file_list::{FileListComponent, FileListEntry};
use crate::keys::KeyBindings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

pub mod search_provider;

// ---------------------------------------------------------------------------
// NoteBrowserProvider trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait NoteBrowserProvider: Send + Sync {
    /// Called on every query change. Empty string = initial/empty state (recent notes).
    async fn load(&self, query: &str) -> Vec<FileListEntry>;

    /// Whether to prepend a "Create: <query>" entry when query is non-empty.
    /// Defaults to false. Used by future FileFinderProvider.
    fn allows_create(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// NoteBrowserModal
// ---------------------------------------------------------------------------

pub struct NoteBrowserModal {
    title: String,
    search_query: String,
    provider: Arc<dyn NoteBrowserProvider>,
    file_list: FileListComponent,
    preview_text: String,
    vault: Arc<NoteVault>,
    tx: AppTx,
    // List async loading
    load_task: Option<tokio::task::JoinHandle<()>>,
    load_rx: Option<Receiver<Vec<FileListEntry>>>,
    // Preview async loading
    preview_task: Option<tokio::task::JoinHandle<()>>,
    preview_rx: Option<Receiver<String>>,
}

impl NoteBrowserModal {
    pub fn new(
        title: impl Into<String>,
        provider: impl NoteBrowserProvider + 'static,
        vault: Arc<NoteVault>,
        key_bindings: KeyBindings,
        icons: Icons,
        tx: AppTx,
    ) -> Self {
        let file_list = FileListComponent::new(key_bindings, icons);
        let mut modal = Self {
            title: title.into(),
            search_query: String::new(),
            provider: Arc::new(provider),
            file_list,
            preview_text: String::new(),
            vault,
            tx: tx.clone(),
            load_task: None,
            load_rx: None,
            preview_task: None,
            preview_rx: None,
        };
        modal.schedule_load(tx);
        modal
    }

    // ── Async list loading ─────────────────────────────────────────────────

    fn schedule_load(&mut self, tx: AppTx) {
        if let Some(handle) = self.load_task.take() {
            handle.abort();
        }
        let query = self.search_query.clone();
        let provider = Arc::clone(&self.provider);
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        self.load_rx = Some(result_rx);

        let handle = tokio::spawn(async move {
            let entries = provider.load(&query).await;
            result_tx.send(entries).ok();
            tx.send(AppEvent::Redraw).ok();
        });
        self.load_task = Some(handle);
    }

    fn poll_load(&mut self) {
        let Some(rx) = &self.load_rx else { return };
        match rx.try_recv() {
            Ok(entries) => {
                self.file_list.clear();
                let mut create_entry: Option<FileListEntry> = None;
                for entry in entries {
                    if matches!(entry, FileListEntry::CreateNote { .. }) {
                        create_entry = Some(entry);
                    } else {
                        self.file_list.push_entry(entry);
                    }
                }
                if let Some(entry) = create_entry {
                    self.file_list.prepend_create_entry(entry);
                }
                self.load_rx = None;
                self.load_task = None;
                self.refresh_preview();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.load_rx = None;
            }
        }
    }

    // ── Async preview loading ──────────────────────────────────────────────

    fn schedule_preview(&mut self, path: VaultPath) {
        if let Some(handle) = self.preview_task.take() {
            handle.abort();
        }
        let vault = Arc::clone(&self.vault);
        let tx = self.tx.clone();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        self.preview_rx = Some(result_rx);

        let handle = tokio::spawn(async move {
            let text = vault.get_note_text(&path).await.unwrap_or_default();
            result_tx.send(text).ok();
            tx.send(AppEvent::Redraw).ok();
        });
        self.preview_task = Some(handle);
    }

    fn poll_preview(&mut self) {
        let Some(rx) = &self.preview_rx else { return };
        match rx.try_recv() {
            Ok(text) => {
                self.preview_text = text;
                self.preview_rx = None;
                self.preview_task = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.preview_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// Called after selection changes to kick off a preview load for the
    /// highlighted note, or clear the preview if a non-note entry is selected.
    fn refresh_preview(&mut self) {
        let maybe_path = self.file_list.selected_entry().and_then(|e| match e {
            FileListEntry::Note { path, .. } => Some(path.clone()),
            _ => None,
        });
        if let Some(path) = maybe_path {
            self.schedule_preview(path);
        } else {
            self.preview_text.clear();
            if let Some(h) = self.preview_task.take() {
                h.abort();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Component impl
// ---------------------------------------------------------------------------

impl Component for NoteBrowserModal {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseEventKind};

        if let InputEvent::Mouse(mouse) = event {
            let r = self.file_list.rendered_rect();
            let in_bounds = mouse.column >= r.x
                && mouse.column < r.x + r.width
                && mouse.row >= r.y
                && mouse.row < r.y + r.height;
            if !in_bounds {
                return EventState::NotConsumed;
            }
            match mouse.kind {
                MouseEventKind::Down(_) => {
                    if mouse.row > r.y {
                        let rel_row = mouse.row - r.y - 1;
                        let prev = self.file_list.selected_display_idx();
                        if let Some(idx) = self.file_list.select_at_visual_row(rel_row) {
                            if prev == Some(idx) {
                                // Second click on the same row — open the note.
                                if let Some(entry) = self.file_list.selected_entry() {
                                    if !matches!(entry, FileListEntry::CreateNote { .. }) {
                                        let path = entry.path().clone();
                                        tx.send(AppEvent::OpenPath(path)).ok();
                                        tx.send(AppEvent::CloseNoteBrowser).ok();
                                    }
                                }
                            } else {
                                self.refresh_preview();
                            }
                        }
                    }
                    EventState::Consumed
                }
                MouseEventKind::ScrollUp => {
                    self.file_list.scroll_up();
                    EventState::Consumed
                }
                MouseEventKind::ScrollDown => {
                    self.file_list.scroll_down();
                    EventState::Consumed
                }
                _ => EventState::Consumed, // consume all other mouse events while modal is open
            }
        } else {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match key.code {
            KeyCode::Esc => {
                tx.send(AppEvent::CloseNoteBrowser).ok();
                EventState::Consumed
            }
            KeyCode::Enter => {
                if let Some(entry) = self.file_list.selected_entry() {
                    match entry {
                        FileListEntry::CreateNote { .. } => {
                            // Future: create note from query
                        }
                        _ => {
                            let path = entry.path().clone();
                            tx.send(AppEvent::OpenPath(path)).ok();
                            tx.send(AppEvent::CloseNoteBrowser).ok();
                        }
                    }
                }
                EventState::Consumed
            }
            KeyCode::Up => {
                self.file_list.select_prev();
                self.refresh_preview();
                EventState::Consumed
            }
            KeyCode::Down => {
                self.file_list.select_next();
                self.refresh_preview();
                EventState::Consumed
            }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.search_query.push(c.to_ascii_uppercase());
                } else {
                    self.search_query.push(c);
                }
                self.schedule_load(tx.clone());
                EventState::Consumed
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.schedule_load(tx.clone());
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
        } // end key else branch
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, _focused: bool) {
        self.poll_load();
        self.poll_preview();

        let popup_rect = centered_rect(80, 75, area);

        // Clear the area behind the modal so the editor doesn't bleed through.
        f.render_widget(Clear, popup_rect);

        let outer_block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .border_style(theme.border_style(true))
            .style(theme.panel_style());
        let inner = outer_block.inner(popup_rect);
        f.render_widget(outer_block, popup_rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(inner);

        // ── Search box ────────────────────────────────────────────────────
        let search_block = Block::default()
            .title(" Search ")
            .borders(Borders::ALL)
            .border_style(theme.border_style(true))
            .style(theme.panel_style());
        let search_inner = search_block.inner(rows[0]);
        f.render_widget(search_block, rows[0]);
        f.render_widget(
            Paragraph::new(self.search_query.as_str()).style(
                Style::default()
                    .fg(theme.fg.to_ratatui())
                    .bg(theme.bg_panel.to_ratatui()),
            ),
            search_inner,
        );
        // Cursor at end of search text, clamped to box width.
        let cursor_x = (search_inner.x + self.search_query.chars().count() as u16)
            .min(search_inner.x + search_inner.width.saturating_sub(1));
        f.set_cursor_position((cursor_x, search_inner.y));

        // ── List + Preview ────────────────────────────────────────────────
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[1]);

        self.file_list.render(f, columns[0], theme, false);

        let preview_block = Block::default()
            .title(" Preview ")
            .borders(Borders::ALL)
            .border_style(theme.border_style(false))
            .style(theme.panel_style());
        let preview_inner = preview_block.inner(columns[1]);
        f.render_widget(preview_block, columns[1]);
        f.render_widget(
            Paragraph::new(self.preview_text.as_str()).style(
                Style::default()
                    .fg(theme.fg.to_ratatui())
                    .bg(theme.bg.to_ratatui()),
            ),
            preview_inner,
        );

        // ── Hint bar ──────────────────────────────────────────────────────
        f.render_widget(
            Paragraph::new("↑↓: navigate  |  Enter: open  |  Esc: close").style(
                Style::default().fg(theme.fg_secondary.to_ratatui()),
            ),
            rows[2],
        );
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        vec![
            ("↑↓".to_string(), "navigate".to_string()),
            ("Enter".to_string(), "open".to_string()),
            ("Esc".to_string(), "close".to_string()),
        ]
    }
}

// ---------------------------------------------------------------------------
// Layout helper
// ---------------------------------------------------------------------------

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_height = area.height * percent_y / 100;
    let popup_width = area.width * percent_x / 100;
    Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_rect_is_centered() {
        let area = Rect { x: 0, y: 0, width: 100, height: 40 };
        let r = centered_rect(80, 75, area);
        assert_eq!(r.width, 80);
        assert_eq!(r.height, 30);
        assert_eq!(r.x, 10); // (100 - 80) / 2
        assert_eq!(r.y, 5);  // (40 - 30) / 2
    }

    #[test]
    fn centered_rect_does_not_underflow() {
        // Very small area — must not panic.
        let area = Rect { x: 0, y: 0, width: 5, height: 5 };
        let _ = centered_rect(80, 75, area);
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(super) fn format_journal_date(date: NaiveDate) -> String {
    date.format("%A, %B %-d, %Y").to_string()
}
