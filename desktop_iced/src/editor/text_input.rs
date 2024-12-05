use iced::{
    highlighter::{self, Highlighter},
    keyboard::Key,
    widget::{row, text_editor, TextEditor},
    Element,
    Length::Fill,
    Renderer, Task, Theme,
};

use crate::AppScreen;

use super::Message;

pub struct NoteEditor {
    content: text_editor::Content,
}

impl NoteEditor {
    pub fn new() -> Self {
        Self {
            content: text_editor::Content::new(),
        }
    }
}

impl AppScreen for NoteEditor {
    fn get_view(&self) -> Element<Message> {
        let input = text_editor(&self.content)
            .on_action(Message::EditorAction)
            .highlight_with::<Highlighter>(
                highlighter::Settings {
                    theme: highlighter::Theme::SolarizedDark,
                    token: "md".to_string(),
                },
                |highlight, _theme| highlight.to_format(),
            )
            // .key_binding(|k| match k.key.as_ref() {
            //     Key::Named(iced::keyboard::key::Named::Tab) => {
            //         debug!("TAB");
            //         Some(text_editor::Binding::Sequence(vec![
            //             text_editor::Binding::Insert(' '),
            //             text_editor::Binding::Insert(' '),
            //         ]))
            //     }
            //     Key::Character(c) => Some(text_editor::Binding::Insert(c.chars().next().unwrap())),
            //     Key::Unidentified => todo!(),
            //     _ => None,
            // })
            .height(Fill);
        input.into()
    }

    fn update(&mut self, action: Message) -> Task<Message> {
        // let action = if let text_editor::Action::Edit(text_editor::Edit::Insert(char)) = action {
        //     if char.eq(&'\t') {
        //         text_editor::Action::Edit(text_editor::Edit::Insert('|'))
        //     } else {
        //         action
        //     }
        // } else {
        //     action
        // };
        if let Message::EditorAction(action) = action {
            self.content.perform(action);
        }
        Task::none()
    }
}
