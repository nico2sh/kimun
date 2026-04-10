pub use delete_dialog::DeleteConfirmDialog;
pub use rename_dialog::RenameDialog;
pub use move_dialog::MoveDialog;
pub use file_ops_menu::FileOpsMenuDialog;
pub use create_note_dialog::CreateNoteDialog;
pub use help_dialog::HelpDialog;
pub use quick_note_modal::QuickNoteModal;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// ValidationState — shared by RenameDialog and MoveDialog
// ---------------------------------------------------------------------------

/// Tracks the current state of an async name / destination availability check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationState {
    /// No check has been triggered yet (initial state).
    Idle,
    /// A check is in progress.
    Pending,
    /// The name / destination is available (does not already exist).
    Available,
    /// The name / destination is already taken.
    Taken,
}

pub mod delete_dialog;
pub mod rename_dialog;
pub mod move_dialog;
pub mod file_ops_menu;
pub mod create_note_dialog;
pub mod help_dialog;
pub mod quick_note_modal;

pub enum ActiveDialog {
    Menu(FileOpsMenuDialog),
    Delete(DeleteConfirmDialog),
    Rename(RenameDialog),
    Move(MoveDialog),
    CreateNote(CreateNoteDialog),
    Help(HelpDialog),
    QuickNote(QuickNoteModal),
}

impl ActiveDialog {
    pub fn set_error(&mut self, msg: String) {
        match self {
            ActiveDialog::Menu(_)      => {} // menu has no error state
            ActiveDialog::Delete(d)    => d.error = Some(msg),
            ActiveDialog::Rename(d)    => d.error = Some(msg),
            ActiveDialog::Move(d)      => d.error = Some(msg),
            ActiveDialog::CreateNote(d)  => d.error = Some(msg),
            ActiveDialog::Help(_)      => {}
            ActiveDialog::QuickNote(d) => d.error = Some(msg),
        }
    }
}

impl Component for ActiveDialog {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match self {
            ActiveDialog::Menu(d)      => d.handle_key(*key, tx),
            ActiveDialog::Delete(d)    => d.handle_key(*key, tx),
            ActiveDialog::Rename(d)    => d.handle_key(*key, tx),
            ActiveDialog::Move(d)      => d.handle_key(*key, tx),
            ActiveDialog::CreateNote(d)  => d.handle_key(*key, tx),
            ActiveDialog::Help(d)      => d.handle_key(*key, tx),
            ActiveDialog::QuickNote(d) => d.handle_key(*key, tx),
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        match self {
            ActiveDialog::Menu(d)      => d.render(f, rect, theme, focused),
            ActiveDialog::Delete(d)    => d.render(f, rect, theme, focused),
            ActiveDialog::Rename(d)    => d.render(f, rect, theme, focused),
            ActiveDialog::Move(d)      => d.render(f, rect, theme, focused),
            ActiveDialog::CreateNote(d)  => d.render(f, rect, theme, focused),
            ActiveDialog::Help(d)      => d.render(f, rect, theme, focused),
            ActiveDialog::QuickNote(d) => d.render(f, rect, theme, focused),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared render helpers
// ---------------------------------------------------------------------------

/// Renders a pre-computed path string (should already include leading spaces).
pub(super) fn render_path_row(f: &mut Frame, rect: Rect, path: &str, fg: Color, bg: Color) {
    f.render_widget(
        Paragraph::new(path).style(Style::default().fg(fg).bg(bg)),
        rect,
    );
}

/// Renders a single-line horizontal rule (TOP border only).
pub(super) fn render_separator(f: &mut Frame, rect: Rect, fg_muted: Color, bg: Color) {
    Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(fg_muted))
        .style(Style::default().bg(bg))
        .render(rect, f.buffer_mut());
}

/// Renders `  Error: {msg}` in red.
pub(super) fn render_error_row(f: &mut Frame, rect: Rect, msg: &str, bg: Color) {
    f.render_widget(
        Paragraph::new(format!("  Error: {msg}"))
            .style(Style::default().fg(Color::Red).bg(bg)),
        rect,
    );
}

/// Renders `{enter_text}  [Esc] Cancel` split into two horizontal columns.
/// The Enter part is dimmed when `enter_active` is `false`.
pub(super) fn render_confirm_hint(
    f: &mut Frame,
    rect: Rect,
    enter_text: &str,
    enter_active: bool,
    fg: Color,
    fg_muted: Color,
    bg: Color,
) {
    let enter_style = if enter_active {
        Style::default().fg(fg).bg(bg)
    } else {
        Style::default().fg(fg_muted).bg(bg).add_modifier(Modifier::DIM)
    };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(enter_text.len() as u16 + 1),
            Constraint::Min(1),
        ])
        .split(rect);
    f.render_widget(Paragraph::new(enter_text).style(enter_style), chunks[0]);
    f.render_widget(
        Paragraph::new("  [Esc] Cancel").style(Style::default().fg(fg_muted).bg(bg)),
        chunks[1],
    );
}

// ---------------------------------------------------------------------------
// Layout helper
// ---------------------------------------------------------------------------

pub(super) fn centered_rect(percent_x: u16, percent_y: u16, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let popup_height = (area.height as u32 * percent_y as u32 / 100) as u16;
    let popup_width  = (area.width  as u32 * percent_x as u32 / 100) as u16;
    ratatui::layout::Rect {
        x: area.x + (area.width.saturating_sub(popup_width)) / 2,
        y: area.y + (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    }
}

/// Centre a dialog of exactly `width` × `height` characters.
pub(super) fn fixed_centered_rect(width: u16, height: u16, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    ratatui::layout::Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::KeyBindings;

    #[test]
    fn active_dialog_help_variant_compiles() {
        let dialog = HelpDialog::new(&KeyBindings::empty());
        let _active: ActiveDialog = ActiveDialog::Help(dialog);
    }
}
