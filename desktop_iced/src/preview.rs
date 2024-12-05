use iced::{
    highlighter::{self, Highlighter},
    widget::{self, text_editor::Action},
    Element,
    Length::Fill,
    Renderer, Task, Theme,
};

use super::Message;

pub struct NotersPreviewer {
    content: widget::text_editor::Content,
}

impl NotersPreviewer {
    pub fn new() -> Self {
        Self {
            content: widget::text_editor::Content::new(),
        }
    }

    pub fn get_editor(&self) -> Element<'_, Message, Theme, Renderer> {
        let input = widget::text_editor(&self.content)
            .on_action(Message::Edit)
            .highlight_with::<Highlighter>(
                highlighter::Settings {
                    theme: highlighter::Theme::SolarizedDark,
                    token: "rs".to_string(),
                },
                |highlight, _theme| highlight.to_format(),
            )
            .height(Fill);
        return input.into();
    }

    pub fn update(&mut self, action: Action) -> Task<Message> {
        self.content.perform(action);
        Task::none()
    }
}
