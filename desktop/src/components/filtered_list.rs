use std::sync::{Arc, LazyLock, Mutex};

use iced::{
    Background, Element, Length, Padding, Task, Theme, border,
    keyboard::{Key, key::Named},
    mouse::Interaction,
    theme::palette,
    widget::{container, mouse_area, scrollable, text_input},
};
use log::debug;
use state_data::StateData;

use crate::KimunMessage;

use super::{KimunComponent, ListElement, VaultRow};

static SCROLLABLE_ID: LazyLock<scrollable::Id> = LazyLock::new(scrollable::Id::unique);
static TEXT_INPUT_ID: LazyLock<text_input::Id> = LazyLock::new(text_input::Id::unique);

#[derive(Debug, Clone)]
pub enum VaultListMessage {
    Initializing,
    Filter,
    UpdateFilterText { filter: String },
    Ready { filter: String, data: Vec<VaultRow> },
    Select(RowSelection),
    Selected(Option<VaultRow>),
    PreviewUpdated(String),
    Enter,
}

#[derive(Debug, Clone)]
pub enum RowSelection {
    Next,
    Previous,
    Index(usize),
    None,
}

#[derive(Debug, Clone)]
pub enum SortMode {
    FileUp,
    FileDown,
    TitleUp,
    TitleDown,
}

impl From<VaultListMessage> for KimunMessage {
    fn from(value: VaultListMessage) -> Self {
        KimunMessage::ListViewMessage(value)
    }
}

impl TryFrom<KimunMessage> for VaultListMessage {
    type Error = ();

    fn try_from(value: KimunMessage) -> Result<Self, Self::Error> {
        if let KimunMessage::ListViewMessage(state) = value {
            Ok(state)
        } else {
            Err(())
        }
    }
}

impl std::fmt::Display for VaultListMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultListMessage::Initializing => write!(f, "Initializing"),
            VaultListMessage::Filter => write!(f, "Initialized"),
            VaultListMessage::UpdateFilterText { filter } => write!(f, "Filtering with {}", filter),
            VaultListMessage::Ready { filter, data: _ } => {
                write!(f, "Filtered with filter `{}`", filter)
            }
            VaultListMessage::Select(row_selection) => write!(f, "Selecting: {:?}", row_selection),
            VaultListMessage::Selected(path) => write!(f, "Selected: {:?}", path),
            VaultListMessage::PreviewUpdated(_) => write!(f, "Updated Preview"),
            VaultListMessage::Enter => write!(f, "Entered"),
        }
    }
}

pub struct FilteredList<F>
where
    F: FilteredListFunctions + 'static,
{
    functions: Arc<Mutex<F>>,
    ready: bool,
    state_data: StateData,
}

impl<F> FilteredList<F>
where
    F: FilteredListFunctions + 'static,
{
    pub fn new(functions: F) -> (Self, iced::Task<KimunMessage>) {
        let functions = Arc::new(Mutex::new(functions));
        let state_data = StateData::new();
        (
            Self {
                functions,
                ready: false,
                state_data,
            },
            iced::Task::batch([
                text_input::focus(TEXT_INPUT_ID.clone()),
                Task::done(VaultListMessage::Initializing.into()),
            ]),
        )
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

impl<F> KimunComponent for FilteredList<F>
where
    F: FilteredListFunctions,
{
    type Message = VaultListMessage;

    fn update(&mut self, message: Self::Message) -> iced::Task<KimunMessage> {
        match message {
            VaultListMessage::Initializing => {
                debug!("Initializing...");
                self.ready = false;
                self.state_data.set_selected(None);
                let functions = self.functions.clone();
                Task::perform(async move { functions.lock().unwrap().init() }, |_| {
                    VaultListMessage::Filter.into()
                })
            }
            VaultListMessage::Filter => {
                debug!("Filter...");
                self.ready = false;
                let functions = self.functions.clone();
                let filter_text = self.state_data.filter_text.clone();
                Task::perform(
                    async move { (functions.lock().unwrap().filter(&filter_text), filter_text) },
                    move |(result, filter)| {
                        VaultListMessage::Ready {
                            filter,
                            data: result,
                        }
                        .into()
                    },
                )
            }
            VaultListMessage::UpdateFilterText { filter } => {
                debug!("Updating the filter text");
                self.state_data.filter_text = filter;
                if self.ready {
                    // If it is ready, we retrigger the filter
                    Task::done(VaultListMessage::Filter.into())
                } else {
                    // If it is not ready, we don't do anything
                    // as we wait to be ready, this way we don't
                    // batch filter requests
                    Task::none()
                }
            }
            VaultListMessage::Ready { filter, data } => {
                debug!("Filtered!");
                self.state_data.set_elements(data);
                if filter != self.state_data.filter_text {
                    self.ready = false;
                    Task::done(VaultListMessage::Filter.into())
                } else {
                    self.ready = true;
                    Task::none()
                }
            }
            VaultListMessage::Select(row_selection) => match row_selection {
                RowSelection::Next => {
                    self.state_data.select_next();
                    let task = Task::done(KimunMessage::ListViewMessage(
                        VaultListMessage::Selected(self.state_data.get_selection()),
                    ));
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
                    let task = Task::done(KimunMessage::ListViewMessage(
                        VaultListMessage::Selected(self.state_data.get_selection()),
                    ));
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
            },
            VaultListMessage::Selected(path) => {
                // We don't do anything, this is just to notify we selected something
                debug!("Highlighting an element at {:?}", path);
                Task::none()
            }
            VaultListMessage::PreviewUpdated(_string) => {
                // We don't do anything, this is just to notify we loaded the preview
                debug!("Updated Preview");
                Task::none()
            }
            VaultListMessage::Enter => {
                debug!("Selected an element");
                if let Some(row) = &self.state_data.get_selection() {
                    debug!("And there's a selection {:?}", row);
                    self.functions.lock().unwrap().on_entry(row)
                } else {
                    Task::none()
                }
            }
        }
    }

    fn view(&self) -> iced::Element<KimunMessage> {
        let text_filter = text_input("Search...", &self.state_data.filter_text)
            .on_input(|filter| {
                KimunMessage::ListViewMessage(VaultListMessage::UpdateFilterText { filter })
            })
            .id(TEXT_INPUT_ID.clone())
            .on_submit(VaultListMessage::Enter.into());

        let elements = self.state_data.get_elements();
        let rows = elements.iter().enumerate().map(|(i, e)| {
            let row_element = self.get_row_view(i, e);
            row_element
        });
        let list = iced::widget::Column::with_children(rows).padding(5);

        container(
            iced::widget::column![text_filter, scrollable(list).id(SCROLLABLE_ID.clone())]
                .spacing(10),
        )
        .width(300)
        .padding(10)
        .into()
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        match (key, modifiers) {
            (Key::Named(Named::Escape), _) => Task::done(KimunMessage::CloseModal),
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

/// The functions that customize the behavior of the filtered list
/// Provides a function on how to filter, and how to behave on each entry
/// when clicked or selected. Also provides an optional first entry/header
/// under the list
pub trait FilteredListFunctions: Clone + Send + Sync {
    fn init(&mut self);
    fn filter<S: AsRef<str>>(&self, filter_text: S) -> Vec<VaultRow>;
    fn on_entry(&mut self, element: &VaultRow) -> Task<KimunMessage>;
    fn header_element(&self, state_data: &StateData) -> Option<VaultRow>;
    fn button_icon(&self) -> Option<String>;
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
