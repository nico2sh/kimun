use std::sync::LazyLock;

use iced::{
    Background, Element, Length, Padding, Task, Theme, border,
    keyboard::{Key, key::Named},
    mouse::Interaction,
    theme::palette,
    widget::{container, mouse_area, scrollable},
};
use log::debug;
use state_data::StateData;

use crate::KimunMessage;

use super::{KimunComponent, KimunListElement};

pub static SCROLLABLE_ID: LazyLock<scrollable::Id> = LazyLock::new(scrollable::Id::unique);

pub struct KimunList<E>
where
    E: KimunListElement,
{
    state_data: StateData<E>,
}

impl<E> KimunList<E>
where
    E: KimunListElement,
{
    pub fn new() -> Self {
        let state_data = StateData::new();
        Self { state_data }
    }

    pub fn select_none(&mut self) {
        self.state_data.set_selected(None);
    }

    fn get_row_view<'a>(&'a self, index: usize, row: &'a E) -> Element<'a, KimunMessage> {
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
            .on_press(KimunMessage::Select(RowSelection::Enter))
            .interaction(Interaction::Pointer)
            .on_enter(KimunMessage::Select(RowSelection::Index(index)));

        ma.into()
    }

    pub fn set_elements(&mut self, data: Vec<E>) {
        self.state_data.set_elements(data);
    }

    pub fn get_selection(&self) -> Option<E> {
        self.state_data.get_selection()
    }

    fn internal_update(&mut self, message: RowSelection) -> Task<KimunMessage> {
        match message {
            RowSelection::Next => {
                self.state_data.select_next();
                let task = Task::done(KimunMessage::Select(RowSelection::Highlighted(
                    self.state_data.get_selected(),
                )));
                if let Some(index) = self.state_data.get_position(4) {
                    debug!("SCROLL");
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
                let task = Task::done(KimunMessage::Select(RowSelection::Highlighted(
                    self.state_data.get_selected(),
                )));
                if let Some(index) = self.state_data.get_position(4) {
                    debug!("SCROLL");
                    task.chain(scrollable::scroll_to(
                        SCROLLABLE_ID.clone(),
                        scrollable::AbsoluteOffset { x: 0.0, y: index },
                    ))
                } else {
                    task
                }
            }
            RowSelection::None => {
                self.state_data.set_selected(None);
                Task::done(KimunMessage::Select(RowSelection::Highlighted(None)))
            }
            RowSelection::Index(index) => {
                self.state_data.set_selected(Some(index));
                Task::done(KimunMessage::Select(RowSelection::Highlighted(
                    self.state_data.get_selected(),
                )))
            }
            RowSelection::Highlighted(path) => {
                // We don't do anything, this is just to notify we selected something
                debug!("Highlighting an element at {:?}", path);
                Task::none()
            }
            RowSelection::Enter => {
                if let Some(row) = &self.get_selection() {
                    debug!("And there's a selection {:?}", row);
                    row.on_select()
                } else {
                    Task::none()
                }
            }
        }
    }
}

impl<E> KimunComponent for KimunList<E>
where
    E: KimunListElement,
{
    fn update(&mut self, message: KimunMessage) -> iced::Task<KimunMessage> {
        if let KimunMessage::Select(message) = message {
            self.internal_update(message)
        } else {
            Task::none()
        }
    }

    fn view(&self) -> Element<KimunMessage> {
        let elements = self.state_data.get_elements();
        let rows = elements.iter().enumerate().map(|(i, e)| {
            let row_element = self.get_row_view(i, e);
            row_element
        });
        let list = iced::widget::Column::with_children(rows).padding(5);

        iced::widget::container(scrollable(list).id(SCROLLABLE_ID.clone()))
            .width(Length::Fill)
            .padding(Padding::from([2, 5]))
            .style(|t| row_style(t, false))
            .into()
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> iced::Task<KimunMessage> {
        match (key, modifiers) {
            (Key::Named(Named::ArrowDown), _) => {
                Task::done(KimunMessage::Select(RowSelection::Next))
            }
            (Key::Named(Named::ArrowUp), _) => {
                Task::done(KimunMessage::Select(RowSelection::Previous))
            }
            (Key::Named(Named::Enter), _) => Task::done(KimunMessage::Select(RowSelection::Enter)),
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
    None,
    Index(usize),
    Highlighted(Option<usize>),
    Enter,
}

pub mod state_data {
    use crate::components::KimunListElement;

    #[derive(Default)]
    pub struct StateData<E>
    where
        E: KimunListElement,
    {
        elements: Vec<E>,
        pub positions: Vec<f32>,
        pub selected: Option<usize>,
    }

    impl<E> StateData<E>
    where
        E: KimunListElement,
    {
        pub fn new() -> Self {
            Self {
                elements: Vec::new(),
                positions: Vec::new(),
                selected: None,
            }
        }

        pub fn get_elements(&self) -> &Vec<E> {
            &self.elements
        }

        pub fn set_elements(&mut self, data: Vec<E>) {
            let mut pos = 0.0;
            self.elements.clear();
            self.positions.clear();
            for row in data {
                self.positions.push(pos);
                pos += row.get_height();
                self.elements.push(row);
            }
        }

        pub fn get_selection(&self) -> Option<E> {
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

        // Gives the Y position using the height of each element
        // Adding an offset
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
