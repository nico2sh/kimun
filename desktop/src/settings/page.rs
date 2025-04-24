use iced::{Task, Theme};

use iced::widget::column;

use crate::fonts::FONT_UI_BOLD;
use crate::modals::Modals;
use crate::modals::vault_indexer::IndexType;
use crate::style_units::{SMALL_PADDING, SMALL_SPACING};
use crate::{ErrorMsg, InitializeOptions, KimunMessage, KimunPageView};

use super::Settings;

#[derive(Debug, Clone)]
pub enum SettingsMsg {
    ThemeSelected(Theme),
    SaveAndClose,
    Browse,
}

impl From<SettingsMsg> for KimunMessage {
    fn from(value: SettingsMsg) -> Self {
        KimunMessage::SettingsChange(value)
    }
}

pub struct SettingsPage {
    settings: Settings,
    themes: iced::widget::combo_box::State<Theme>,
    selected_theme: Option<Theme>,
    saved: bool,
}

impl SettingsPage {
    pub fn new(settings: Settings) -> Self {
        let themes = iced::widget::combo_box::State::new(Theme::ALL.to_vec());
        let selected_theme = settings.theme.clone();
        Self {
            settings,
            themes,
            selected_theme: Some(selected_theme),
            saved: true,
        }
    }

    fn section_appearance(&self) -> iced::Element<KimunMessage> {
        section(
            "Appearance",
            column![
                iced::widget::text("Select a theme"),
                iced::widget::combo_box(
                    &self.themes,
                    "Theme",
                    self.selected_theme.as_ref(),
                    |theme| SettingsMsg::ThemeSelected(theme).into()
                )
            ]
            .spacing(SMALL_SPACING)
            .into(),
        )
    }

    fn section_workspace(&self) -> iced::Element<KimunMessage> {
        let mut button_fast_index = iced::widget::button("Fast Index");
        let mut button_full_index = iced::widget::button("Full Index");
        if let Some(path) = &self.settings.workspace_dir {
            button_fast_index = button_fast_index.on_press(KimunMessage::ShowModal(
                Modals::VaultIndex(path.to_owned(), IndexType::Fast),
            ));
            button_full_index = button_full_index.on_press(KimunMessage::ShowModal(
                Modals::VaultIndex(path.to_owned(), IndexType::Full),
            ));
        }

        section(
            "Vault",
            iced::widget::column![
                iced::widget::text("Vault Path"),
                iced::widget::row![
                    iced::widget::container(
                        iced::widget::text(
                            self.settings
                                .workspace_dir
                                .as_ref()
                                .map(|path| path.to_string_lossy())
                                .unwrap_or_default()
                        )
                        .size(14)
                        .width(iced::Length::Fill)
                    )
                    .style(|theme: &Theme| {
                        let palette = theme.extended_palette();
                        iced::widget::container::Style {
                            background: Some(iced::Background::Color(
                                palette.background.weak.color,
                            )),
                            border: iced::border::rounded(2)
                                .color(palette.background.weak.text)
                                .width(1),
                            ..Default::default()
                        }
                    })
                    .padding(SMALL_PADDING),
                    iced::widget::button("Browse").on_press(SettingsMsg::Browse.into())
                ]
                .width(iced::Length::Fill)
                .spacing(SMALL_SPACING),
                iced::widget::vertical_space().height(SMALL_SPACING),
                iced::widget::row![
                    button_fast_index.width(iced::Length::FillPortion(1)),
                    button_full_index.width(iced::Length::FillPortion(1))
                ]
                .spacing(SMALL_SPACING)
            ]
            .spacing(SMALL_SPACING)
            .into(),
        )
    }
}

impl KimunPageView for SettingsPage {
    fn update(&mut self, message: KimunMessage) -> Task<KimunMessage> {
        if let KimunMessage::SettingsChange(set) = message {
            match set {
                SettingsMsg::ThemeSelected(theme) => {
                    // We update the theme
                    self.settings.theme = theme.clone();
                    self.selected_theme = Some(theme.clone());
                    self.saved = false;
                    Task::done(KimunMessage::SettingsUpdated(self.settings.clone()))
                }
                SettingsMsg::SaveAndClose => {
                    if !self.saved {
                        match self.settings.save_to_disk() {
                            Ok(_) => Task::done(KimunMessage::Initialize(
                                InitializeOptions::new().with_settings(self.settings.clone()),
                            )),
                            Err(e) => Task::done(KimunMessage::Error(ErrorMsg::Add(e.to_string()))),
                        }
                    } else {
                        Task::done(KimunMessage::Initialize(
                            InitializeOptions::new().with_settings(self.settings.clone()),
                        ))
                    }
                }
                SettingsMsg::Browse => Task::none(),
            }
        } else {
            Task::none()
        }
    }

    fn view(&self) -> iced::Element<KimunMessage> {
        let mut close_button = iced::widget::button("Save and Close");
        if self.settings.workspace_dir.is_some() {
            close_button = close_button.on_press(SettingsMsg::SaveAndClose.into());
        }
        iced::widget::container(
            iced::widget::column![
                self.section_appearance(),
                self.section_workspace(),
                iced::widget::vertical_space(),
                close_button,
            ]
            .spacing(8),
        )
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into()
    }

    fn key_press(
        &self,
        _key: &iced::keyboard::Key,
        _modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        Task::none()
    }
}

fn section<S: AsRef<str>>(
    title: S,
    content: iced::Element<KimunMessage>,
) -> iced::Element<KimunMessage> {
    column![
        iced::widget::rich_text![iced::widget::span(title.as_ref().to_string()).font(FONT_UI_BOLD)]
            .size(18)
            .on_link_click(iced::never),
        iced::widget::container(content)
            .style(section_style)
            .padding(SMALL_PADDING)
            .width(400),
    ]
    .spacing(SMALL_SPACING)
    .into()
}

fn section_style(theme: &Theme) -> iced::widget::container::Style {
    let palette = theme.extended_palette();
    styled(palette.background.strong)
}

fn styled(pair: iced::theme::palette::Pair) -> iced::widget::container::Style {
    iced::widget::container::Style {
        background: Some(iced::Background::Color(pair.color)),
        text_color: Some(pair.text),
        border: iced::border::rounded(4).color(pair.text).width(0),
        ..iced::widget::container::Style::default()
    }
}
