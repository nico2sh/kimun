pub mod start;

use ratatui::Frame;

use crate::components::actions::Action;

pub trait AppScreen {
    fn update(&mut self, action: Action) -> Action;
    fn render(&mut self, f: &mut Frame);
}
