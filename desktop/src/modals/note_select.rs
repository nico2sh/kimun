use iced::{
    Task,
    keyboard::{Key, key::Named},
};
use kimun_core::{
    nfs::{NoteEntryData, VaultPath},
    note::NoteContentData,
};

use crate::{
    KimunMessage,
    components::{
        KimunComponent, KimunListElement, filtered_list::ListViewMessage, list::KimunList,
    },
    editor::EditorMessage,
    fonts::{FONT_UI, FONT_UI_ITALIC},
    icons::{ICON, KimunIcon},
};

use super::KimunModal;

#[derive(Clone, Debug)]
pub struct NoteRow {
    pub path: VaultPath,
    pub title: String,
}

impl KimunListElement for NoteRow {
    fn get_view(&self) -> iced::Element<KimunMessage> {
        iced::widget::row![
            iced::widget::text(KimunIcon::Note.get_char()).font(ICON),
            iced::widget::column![
                iced::widget::text(self.title.to_owned()).font(FONT_UI),
                iced::widget::text(self.path.to_string()).font(FONT_UI_ITALIC)
            ]
        ]
        .spacing(8)
        .into()
    }

    fn get_height(&self) -> f32 {
        44.0
    }
}

impl From<(NoteEntryData, NoteContentData)> for NoteRow {
    fn from(value: (NoteEntryData, NoteContentData)) -> Self {
        Self {
            path: value.0.path,
            title: value.1.title,
        }
    }
}

pub struct NoteSelect {
    list: KimunList<NoteRow>,
}

impl NoteSelect {
    pub fn new() -> Self {
        let list = KimunList::new();
        Self { list }
    }

    pub fn set_elements(&mut self, elements: Vec<NoteRow>) {
        self.list.set_elements(elements);
    }
}

impl KimunModal for NoteSelect {
    fn view(&self) -> iced::Element<KimunMessage> {
        iced::widget::column![
            iced::widget::text("Multiple notes match the note name").size(22.0),
            self.list.view()
        ]
        .spacing(16)
        .into()
    }

    fn get_width(&self) -> iced::Length {
        400.into()
    }

    fn get_height(&self) -> iced::Length {
        300.into()
    }

    fn update(&mut self, message: KimunMessage) -> iced::Task<KimunMessage> {
        match message {
            KimunMessage::ListViewMessage(ListViewMessage::Enter) => {
                // We load the note
                if let Some(element) = self.list.get_selection() {
                    Task::batch([
                        Task::done(KimunMessage::CloseModal),
                        Task::done(KimunMessage::EditorMessage(EditorMessage::OpenNote(
                            element.path.clone(),
                        ))),
                    ])
                } else {
                    Task::none()
                }
            }
            _ => {
                if let Ok(list_msg) = message.try_into() {
                    self.list.update(list_msg)
                } else {
                    Task::none()
                }
            }
        }
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> iced::Task<KimunMessage> {
        match (key, modifiers) {
            (Key::Named(Named::Escape), _) => Task::done(KimunMessage::CloseModal),
            _ => self.list.key_press(key, modifiers),
        }
    }
}

struct NoteEntry {
    path: VaultPath,
    title: String,
}
