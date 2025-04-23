use iced::{Task, Theme};

use iced::widget::column;

use crate::{KimunMessage, KimunPageView};

use super::Settings;

#[derive(Debug, Clone)]
pub enum SettingsMessage {
    ThemeSelected(Theme),
}

impl From<SettingsMessage> for KimunMessage {
    fn from(value: SettingsMessage) -> Self {
        KimunMessage::SettingsChange(value)
    }
}

pub struct SettingsPage {
    settings: Settings,
    themes: iced::widget::combo_box::State<Theme>,
    selected_theme: Option<Theme>,
}

impl SettingsPage {
    pub fn new() -> Self {
        let settings = Settings::load_from_disk().unwrap();
        let themes = iced::widget::combo_box::State::new(Theme::ALL.to_vec());
        let selected_theme = settings.theme.clone();
        Self {
            settings,
            themes,
            selected_theme: Some(selected_theme),
        }
    }
}

impl KimunPageView for SettingsPage {
    fn update(&mut self, message: KimunMessage) -> Task<KimunMessage> {
        if let KimunMessage::SettingsChange(set) = message {
            match set {
                SettingsMessage::ThemeSelected(theme) => {
                    // We update the theme
                    self.settings.theme = theme.clone();
                    self.selected_theme = Some(theme.clone());
                    Task::done(KimunMessage::SettingsUpdated(self.settings.clone()))
                }
            }
        } else {
            Task::none()
        }
    }

    fn view(&self) -> iced::Element<KimunMessage> {
        iced::widget::container(
            column![
                iced::widget::text("Select a theme"),
                iced::widget::combo_box(
                    &self.themes,
                    "Theme",
                    self.selected_theme.as_ref(),
                    |theme| SettingsMessage::ThemeSelected(theme).into()
                )
            ]
            .spacing(4),
        )
        .padding(16)
        .width(250)
        .into()
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        Task::none()
    }
}
