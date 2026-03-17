use ratatui::Frame;

use crate::app::App;

pub fn ui(frame: &mut Frame, app: &mut App) {
    if let Some(screen) = &mut app.current_screen {
        screen.render(frame);
    }
}
