use iced::{
    Length::Fill,
    Task,
    widget::{container, markdown, scrollable},
};
use kimun_core::{NoteVault, nfs::VaultPath, note::NoteDetails};
use log::debug;

use crate::KimunMessage;

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
}

impl PreviewPage {
    pub fn new(text: String, vault: NoteVault, path: VaultPath) -> Self {
        let md = NoteDetails::new(&path, &text).get_markdown_and_links();
        let content = markdown::parse(&md.text).collect();
        let md_style = markdown::Style::from_palette(iced::Theme::TokyoNightStorm.palette());
        let md_settings = markdown::Settings::with_style(md_style);
        Self {
            vault,
            path,
            content,
            md_settings,
        }
    }

    pub fn load_note(&mut self, details: NoteDetails) {
        self.content = markdown::parse(&details.get_markdown_and_links().text).collect();
        self.path = details.path;
    }

    pub fn view(&self) -> iced::Element<crate::KimunMessage> {
        container(
            scrollable(markdown::view(&self.content, self.md_settings).map(|url| {
                KimunMessage::EditorMessage(EditorMessage::PreviewMessage(
                    PreviewMessage::LinkClicked(url),
                ))
            }))
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

    pub(crate) fn update(&self, pmessage: PreviewMessage) -> iced::Task<KimunMessage> {
        if let PreviewMessage::LinkClicked(link) = pmessage {
            debug!("Link: {}", link);
        };
        Task::none()
    }
}
