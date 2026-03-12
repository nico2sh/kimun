use crate::{app_screen::AppScreen, components::actions::Action};

pub struct StartScreen {}

impl AppScreen for StartScreen {
    fn update(&mut self, action: Action) -> Action {
        todo!()
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        todo!()
    }
}
