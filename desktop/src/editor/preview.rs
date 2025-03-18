use iced::{
    Length::Fill,
    Task,
    widget::{container, hover, markdown, scrollable},
};
use kimun_core::{NoteVault, nfs::VaultPath, note::NoteDetails};

use crate::{KimunMessage, KimunPage};

use super::EditorMessage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewMessage {
    Toggle,
    LinkClicked(markdown::Url),
}

impl From<PreviewMessage> for KimunMessage {
    fn from(value: PreviewMessage) -> Self {
        KimunMessage::EditorMessage(EditorMessage::PreviewMessage(value))
    }
}

pub struct PreviewPage {
    vault: NoteVault,
    path: VaultPath,
    content: Vec<markdown::Item>,
    md_settings: markdown::Settings,
    md_style: markdown::Style,
}

impl PreviewPage {
    pub fn new(content: String, vault: NoteVault, path: VaultPath) -> Self {
        let md = NoteDetails::new(&path, &content).get_markdown_and_links();
        let content = markdown::parse(&md.text).collect();
        let md_settings = markdown::Settings::default();
        let md_style = markdown::Style::from_palette(iced::Theme::TokyoNightStorm.palette());
        Self {
            vault,
            path,
            content,
            md_settings,
            md_style,
        }
    }

    pub fn load_note(&mut self, details: NoteDetails) {
        self.content = markdown::parse(&details.get_markdown_and_links().text).collect();
        self.path = details.path;
    }

    pub fn view(&self) -> iced::Element<crate::KimunMessage> {
        container(
            scrollable(
                markdown::view(&self.content, self.md_settings, self.md_style).map(|url| {
                    KimunMessage::EditorMessage(EditorMessage::PreviewMessage(
                        PreviewMessage::LinkClicked(url),
                    ))
                }),
            )
            .spacing(16)
            .width(Fill)
            .height(Fill),
        )
        .padding(10)
        .into()
    }

    pub fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> iced::Task<crate::KimunMessage> {
        if let Some(message) = super::manage_editor_hotkeys(key, modifiers, &self.vault, &self.path)
        {
            Task::done(message)
        } else {
            Task::none()
        }
    }
}
