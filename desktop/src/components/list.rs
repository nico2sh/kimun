use std::sync::LazyLock;

use iced::{
    Background, Element, Length, Padding, Task, Theme, border,
    keyboard::{Key, key::Named},
    mouse::Interaction,
    theme::palette,
    widget::{container, mouse_area, scrollable},
};
use state_data::StateData;

use crate::{
    KimunMessage,
    fonts::{FONT_UI, FONT_UI_ITALIC},
    icons::{ICON, KimunIcon},
};

use super::{KimunComponent, ListElement, VaultRow, filtered_list::VaultListMessage};

pub static SCROLLABLE_ID: LazyLock<scrollable::Id> = LazyLock::new(scrollable::Id::unique);

pub struct List {
    state_data: StateData,
}

impl List {
    pub fn new() -> Self {
        let state_data = StateData::new();
        Self { state_data }
    }

    pub fn select_none(&mut self) {
        self.state_data.set_selected(None);
    }

    fn get_row_view<'a>(&'a self, index: usize, row: &'a VaultRow) -> Element<'a, KimunMessage> {
        let selected = self
            .state_data
            .get_selected()
            .map_or_else(|| false, |s| s == index);
        let cont = iced::widget::container(row.get_view())
            .center_y(row.get_height())
            .padding(Padding {
                top: 0.0,
                right: 10.0,
                bottom: 0.0,
                left: 10.0,
            })
            .width(Length::Fill)
            .style(move |t| row_style(t, selected));
        let ma = mouse_area(cont)
            .on_press(VaultListMessage::Enter.into())
            .interaction(Interaction::Pointer)
            .on_enter(VaultListMessage::Select(RowSelection::Index(index)).into());

        ma.into()
    }

    pub fn set_elements(&mut self, data: Vec<VaultRow>) {
        self.state_data.set_elements(data);
    }

    pub(crate) fn get_selection(&self) -> Option<VaultRow> {
        self.state_data.get_selection()
    }
}

impl KimunComponent for List {
    type Message = RowSelection;

    fn update(&mut self, message: Self::Message) -> iced::Task<KimunMessage> {
        match message {
            RowSelection::Next => {
                self.state_data.select_next();
                let task = Task::done(KimunMessage::ListViewMessage(VaultListMessage::Selected(
                    self.state_data.get_selection(),
                )));
                if let Some(index) = self.state_data.get_position(4) {
                    task.chain(scrollable::scroll_to(
                        SCROLLABLE_ID.clone(),
                        scrollable::AbsoluteOffset { x: 0.0, y: index },
                    ))
                } else {
                    task
                }
            }
            RowSelection::Previous => {
                self.state_data.select_prev();
                let task = Task::done(KimunMessage::ListViewMessage(VaultListMessage::Selected(
                    self.state_data.get_selection(),
                )));
                if let Some(index) = self.state_data.get_position(4) {
                    task.chain(scrollable::scroll_to(
                        SCROLLABLE_ID.clone(),
                        scrollable::AbsoluteOffset { x: 0.0, y: index },
                    ))
                } else {
                    task
                }
            }
            RowSelection::Index(index) => {
                self.state_data.set_selected(Some(index));
                Task::done(VaultListMessage::Selected(self.state_data.get_selection()).into())
            }
            RowSelection::None => {
                self.state_data.set_selected(None);
                Task::done(VaultListMessage::Selected(None).into())
            }
        }
    }

    fn view(&self) -> Element<KimunMessage> {
        let elements = self.state_data.get_elements();
        let rows = elements.iter().enumerate().map(|(i, e)| {
            let row_element = self.get_row_view(i, e);
            row_element
        });
        let list = iced::widget::Column::with_children(rows).padding(5);

        scrollable(list).spacing(10).into()
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> iced::Task<KimunMessage> {
        match (key, modifiers) {
            (Key::Named(Named::ArrowDown), _) => {
                Task::done(VaultListMessage::Select(RowSelection::Next).into())
            }
            (Key::Named(Named::ArrowUp), _) => {
                Task::done(VaultListMessage::Select(RowSelection::Previous).into())
            }
            (Key::Named(Named::Enter), _) => Task::done(VaultListMessage::Enter.into()),
            _ => Task::none(),
        }
    }
}

fn row_style(theme: &Theme, selected: bool) -> iced::widget::container::Style {
    let palette = theme.extended_palette();
    if selected {
        styled(palette.background.strong)
    } else {
        styled(palette.background.base)
    }
}

fn styled(pair: palette::Pair) -> container::Style {
    container::Style {
        background: Some(Background::Color(pair.color)),
        text_color: Some(pair.text),
        border: border::rounded(2),
        ..container::Style::default()
    }
}

#[derive(Debug, Clone)]
pub enum RowSelection {
    Next,
    Previous,
    Index(usize),
    None,
}

impl TryFrom<KimunMessage> for RowSelection {
    type Error = ();

    fn try_from(value: KimunMessage) -> Result<Self, Self::Error> {
        if let KimunMessage::ListViewMessage(VaultListMessage::Select(selection)) = value {
            Ok(selection)
        } else {
            Err(())
        }
    }
}

pub mod state_data {
    use crate::components::{ListElement, VaultRow};

    #[derive(Default)]
    pub struct StateData {
        pub filter_text: String,
        elements: Vec<VaultRow>,
        pub positions: Vec<f32>,
        pub selected: Option<usize>,
    }

    impl StateData {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn get_elements(&self) -> &Vec<VaultRow> {
            &self.elements
        }

        pub fn set_elements(&mut self, data: Vec<VaultRow>) {
            let mut pos = 0.0;
            self.elements.clear();
            self.positions.clear();
            for row in data {
                self.positions.push(pos);
                pos += row.get_height();
                self.elements.push(row);
            }
        }

        pub fn get_selection(&self) -> Option<VaultRow> {
            if let Some(selected) = self.selected {
                let elements = self.get_elements();
                let sel = elements.get(selected);
                sel.cloned()
            } else {
                None
            }
        }

        pub fn get_selected(&self) -> Option<usize> {
            self.selected
        }

        pub fn get_position(&self, offset: usize) -> Option<f32> {
            self.selected.and_then(|index| {
                let i = index.saturating_sub(offset);
                self.positions.get(i).map(|u| u.to_owned())
            })
        }

        pub fn set_selected(&mut self, number: Option<usize>) {
            let elements = self.get_elements();
            if !elements.is_empty() {
                self.selected = number.map(|n| std::cmp::min(elements.len() - 1, n));
            } else {
                self.selected = None;
            }
        }

        pub fn select_next(&mut self) {
            let elements = self.get_elements();
            if !elements.is_empty() {
                self.selected = Some(if let Some(mut selected) = self.selected {
                    selected += 1;
                    if selected > elements.len() - 1 {
                        selected - elements.len()
                    } else {
                        selected
                    }
                } else {
                    0
                });
            } else {
                self.selected = None;
            }
        }

        pub fn select_prev(&mut self) {
            let elements = self.get_elements();
            if !elements.is_empty() {
                self.selected = Some(if let Some(mut selected) = self.selected {
                    if selected == 0 {
                        selected = elements.len() - 1;
                    } else {
                        selected -= 1;
                    }
                    selected
                } else {
                    0
                });
            } else {
                self.selected = None;
            }
        }
    }
}
