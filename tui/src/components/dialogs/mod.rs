pub use delete_dialog::DeleteConfirmDialog;
pub use rename_dialog::RenameDialog;
pub use move_dialog::MoveDialog;

pub mod delete_dialog;
pub mod rename_dialog;
pub mod move_dialog;

pub enum ActiveDialog {
    Delete(DeleteConfirmDialog),
    Rename(RenameDialog),
    Move(MoveDialog),
}

impl ActiveDialog {
    pub fn set_error(&mut self, msg: String) {
        match self {
            ActiveDialog::Delete(d) => d.error = Some(msg),
            ActiveDialog::Rename(d) => d.error = Some(msg),
            ActiveDialog::Move(d)   => d.error = Some(msg),
        }
    }
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
