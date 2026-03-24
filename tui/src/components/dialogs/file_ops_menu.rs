use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::crossterm::event::KeyCode;

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// FileOpsMenuDialog
// ---------------------------------------------------------------------------

/// Small menu dialog that lets the user pick a file operation.
///
/// ```text
/// ┌─ File Operations ────────────────────────────┐
/// │                                              │
/// │  notes/projects/kimun.md                     │
/// │                                              │
/// │  [D] Delete   [R] Rename   [M] Move          │
/// │                                              │
/// │  [Esc] Cancel                                │
/// └──────────────────────────────────────────────┘
/// ```
pub struct FileOpsMenuDialog {
    /// The vault entry this menu was opened for.
    pub path: VaultPath,
    /// Pre-computed `"  {path}"` for zero-allocation rendering.
    pub path_display: String,
}

impl FileOpsMenuDialog {
    pub fn new(path: VaultPath) -> Self {
        let path_display = format!("  {}", path);
        Self { path, path_display }
    }

    /// Handle a raw key event. Returns `Consumed` for all recognised keys so
    /// the event never leaks to the underlying panel.
    pub fn handle_key(
        &mut self,
        key: ratatui::crossterm::event::KeyEvent,
        tx: &AppTx,
    ) -> EventState {
        match key.code {
            KeyCode::Char('d') | KeyCode::Char('D') => {
                tx.send(AppEvent::ShowDeleteDialog(self.path.clone())).ok();
                EventState::Consumed
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                tx.send(AppEvent::ShowRenameDialog(self.path.clone())).ok();
                EventState::Consumed
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                tx.send(AppEvent::ShowMoveDialog(self.path.clone())).ok();
                EventState::Consumed
            }
            KeyCode::Esc => {
                tx.send(AppEvent::CloseDialog).ok();
                EventState::Consumed
            }
            _ => EventState::Consumed, // swallow unknown keys while menu is open
        }
    }
}

// ---------------------------------------------------------------------------
// Component trait
// ---------------------------------------------------------------------------

impl Component for FileOpsMenuDialog {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        // Fixed size: 46 wide × 9 tall
        // Border (2) + spacer + path + separator + actions + spacer + hint + spacer = 9 inner rows → 11 total
        // But keep it tight: border(2) + 7 inner rows = 9
        let popup_area = super::fixed_centered_rect(46, 9, rect);

        f.render_widget(Clear, popup_area);

        let outer_block = Block::default()
            .title(" File Operations ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.fg.to_ratatui()))
            .style(theme.panel_style());
        let inner = outer_block.inner(popup_area);
        f.render_widget(outer_block, popup_area);

        // ── Layout ────────────────────────────────────────────────────────────
        // Row 0: spacer
        // Row 1: path display
        // Row 2: separator (horizontal line)
        // Row 3: action row  [D] Delete  [R] Rename  [M] Move
        // Row 4: spacer
        // Row 5: hint row    [Esc] Cancel
        // Row 6: spacer

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 0: spacer
                Constraint::Length(1), // 1: path
                Constraint::Length(1), // 2: separator
                Constraint::Length(1), // 3: actions
                Constraint::Length(1), // 4: spacer
                Constraint::Length(1), // 5: hint
                Constraint::Min(0),    // 6: remainder
            ])
            .split(inner);

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();
        let fg_accent = theme.fg_selected.to_ratatui();

        // Row 1: path
        super::render_path_row(f, rows[1], &self.path_display, fg, bg);

        // Row 2: separator
        super::render_separator(f, rows[2], fg_muted, bg);

        // Row 3: action shortcuts — key letter highlighted, description muted
        //
        // Split into three equal columns.
        let action_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ])
            .split(rows[3]);

        let key_style = Style::default()
            .fg(fg_accent)
            .bg(bg)
            .add_modifier(Modifier::BOLD);
        let label_style = Style::default().fg(fg).bg(bg);

        for (col, (key, label)) in action_cols.iter().zip([
            ("[D]", " Delete"),
            ("[R]", " Rename"),
            ("[M]", " Move  "),
        ]) {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(1), // left padding
                    Constraint::Length(3), // "[D]"
                    Constraint::Min(1),    // " Delete"
                ])
                .split(*col);

            f.render_widget(
                Paragraph::new(key).style(key_style),
                chunks[1],
            );
            f.render_widget(
                Paragraph::new(label).style(label_style),
                chunks[2],
            );
        }

        // Row 5: hint
        f.render_widget(
            Paragraph::new("  [Esc] Cancel")
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[5],
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_sends_close_dialog() {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
        use tokio::sync::mpsc;

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
            let mut dialog = FileOpsMenuDialog::new(VaultPath::new("notes/test.md"));

            let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
            let state = dialog.handle_key(key, &tx);

            assert_eq!(state, EventState::Consumed);
            let event = rx.try_recv().expect("expected AppEvent::CloseDialog");
            assert!(matches!(event, AppEvent::CloseDialog));
        });
    }

    #[test]
    fn d_sends_show_delete_dialog() {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
        use tokio::sync::mpsc;

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let path = VaultPath::new("notes/test.md");
            let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
            let mut dialog = FileOpsMenuDialog::new(path.clone());

            let key = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE);
            let state = dialog.handle_key(key, &tx);

            assert_eq!(state, EventState::Consumed);
            let event = rx.try_recv().expect("expected AppEvent::ShowDeleteDialog");
            assert!(matches!(event, AppEvent::ShowDeleteDialog(_)));
        });
    }

    #[test]
    fn r_sends_show_rename_dialog() {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
        use tokio::sync::mpsc;

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let path = VaultPath::new("notes/test.md");
            let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
            let mut dialog = FileOpsMenuDialog::new(path.clone());

            let key = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE);
            let state = dialog.handle_key(key, &tx);

            assert_eq!(state, EventState::Consumed);
            let event = rx.try_recv().expect("expected AppEvent::ShowRenameDialog");
            assert!(matches!(event, AppEvent::ShowRenameDialog(_)));
        });
    }

    #[test]
    fn m_sends_show_move_dialog() {
        use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
        use tokio::sync::mpsc;

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let path = VaultPath::new("notes/test.md");
            let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
            let mut dialog = FileOpsMenuDialog::new(path.clone());

            let key = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE);
            let state = dialog.handle_key(key, &tx);

            assert_eq!(state, EventState::Consumed);
            let event = rx.try_recv().expect("expected AppEvent::ShowMoveDialog");
            assert!(matches!(event, AppEvent::ShowMoveDialog(_)));
        });
    }
}
