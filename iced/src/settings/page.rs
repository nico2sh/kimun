use iced::Task;

use iced::widget::column;

use crate::{KimunMessage, KimunPage};

pub struct SettingsPage {}

impl SettingsPage {
    pub fn new() -> Self {
        Self {}
    }
}

impl KimunPage for SettingsPage {
    fn update(&mut self, message: crate::KimunMessage) -> anyhow::Result<Task<KimunMessage>> {
        Ok(Task::none())
    }

    fn view(&self) -> iced::Element<crate::KimunMessage> {
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
