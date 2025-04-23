use iced::Task;

use iced::widget::column;

use crate::{KimunMessage, KimunPageView};

pub struct SettingsPage {}

impl SettingsPage {
    pub fn new() -> Self {
        Self {}
    }
}

impl KimunPageView for SettingsPage {
    fn update(&mut self, message: KimunMessage) -> Task<KimunMessage> {
        Task::none()
    }

    fn view(&self) -> iced::Element<KimunMessage> {
        column![].into()
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        Task::none()
    }
}
