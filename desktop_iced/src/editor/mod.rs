mod note_browser;

use core_notes::{nfs::NotePath, NoteVault};
use iced::{
    highlighter::{self, Highlighter},
    widget::*,
    Element,
    Length::Fill,
    Task,
};
use log::debug;
use note_browser::NoteBrowser;

use crate::{AppScreen, Message};

pub enum EditorMessage {
    TextAction(text_editor::Action),
}

pub struct Editor {
    vault: NoteVault,
    current_path: NotePath,
    content: text_editor::Content,
    browser: NoteBrowser,
}

impl Editor {
    pub fn new(vault: NoteVault, current_path: NotePath) -> Self {
        debug!("Creating Editor");
        let browser = NoteBrowser::new(&current_path, &vault);
        Self {
            vault,
            current_path,
            content: text_editor::Content::new(),
            browser,
        }
    }
}

impl AppScreen for Editor {
    fn get_view(&self) -> Element<Message> {
        let browser_view = self.browser.get_view();

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
            .height(Fill)
            .padding(5);
        let row = row![browser_view, input];

        let w = container(row.spacing(10)).padding(10).into();
        w
    }

    fn update(&mut self, action: Message) -> Task<Message> {
        if let Message::EditorAction(action) = action {
            self.content.perform(action);
        }
        Task::none()
    }
}
