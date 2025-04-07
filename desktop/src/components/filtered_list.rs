use std::sync::{Arc, LazyLock, Mutex};

use iced::{
    Background, Element, Length, Padding, Task, Theme, border,
    keyboard::{Key, key::Named},
    mouse::Interaction,
    theme::palette,
    widget::{container, mouse_area, text_input},
};
use log::debug;

use crate::{
    KimunMessage,
    fonts::{FONT_UI, FONT_UI_ITALIC},
    icons::{ICON, KimunIcon},
};

use super::{
    KimunComponent, VaultRow,
    list::{List, RowSelection},
};

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
    filter_text: String,
    list: List,
}

impl<F> FilteredList<F>
where
    F: FilteredListFunctions + 'static,
{
    pub fn new(functions: F) -> (Self, iced::Task<KimunMessage>) {
        let functions = Arc::new(Mutex::new(functions));
        let filter_text = String::new();
        let list = List::new();
        (
            Self {
                functions,
                ready: false,
                filter_text,
                list,
            },
            iced::Task::batch([
                text_input::focus(TEXT_INPUT_ID.clone()),
                Task::done(VaultListMessage::Initializing.into()),
            ]),
        )
    }

    fn get_header_view(&self, header: VaultRow) -> Element<'_, KimunMessage> {
        let selected = false;
        let v = iced::widget::row![
            iced::widget::text(KimunIcon::Note.get_char()).font(ICON),
            iced::widget::text("Create new note: ").font(FONT_UI),
            iced::widget::text(header.path.to_string()).font(FONT_UI_ITALIC),
        ]
        .spacing(8);
        let cont = iced::widget::container(v)
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
            .on_enter(VaultListMessage::Select(RowSelection::Index(0)).into());

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
                self.list.select_none();
                let functions = self.functions.clone();
                Task::perform(async move { functions.lock().unwrap().init() }, |_| {
                    VaultListMessage::Filter.into()
                })
            }
            VaultListMessage::Filter => {
                debug!("Filter...");
                self.ready = false;
                let functions = self.functions.clone();
                let filter_text = self.filter_text.clone();
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
                self.filter_text = filter;
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
                self.list.set_elements(data);
                if filter != self.filter_text {
                    self.ready = false;
                    Task::done(VaultListMessage::Filter.into())
                } else {
                    self.ready = true;
                    Task::none()
                }
            }
            VaultListMessage::Select(row_selection) => self.list.update(row_selection),
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
                if let Some(row) = &self.list.get_selection() {
                    debug!("And there's a selection {:?}", row);
                    self.functions.lock().unwrap().on_entry(row)
                } else {
                    Task::none()
                }
            }
        }
    }

    fn view(&self) -> iced::Element<KimunMessage> {
        let text_filter = text_input("Search...", &self.filter_text)
            .on_input(|filter| {
                KimunMessage::ListViewMessage(VaultListMessage::UpdateFilterText { filter })
            })
            .id(TEXT_INPUT_ID.clone())
            .on_submit(VaultListMessage::Enter.into());

        // Insert header here
        let header = self
            .functions
            .lock()
            .unwrap()
            .header_element(&self.filter_text);
        if let Some(head_row) = header {
            let h = self.get_header_view(head_row);
            container(iced::widget::column![text_filter, h, self.list.view()])
                .width(300)
                .padding(10)
                .into()
        } else {
            container(iced::widget::column![text_filter, self.list.view()])
                .width(300)
                .padding(10)
                .into()
        }
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
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

/// The functions that customize the behavior of the filtered list
/// Provides a function on how to filter, and how to behave on each entry
/// when clicked or selected. Also provides an optional first entry/header
/// under the list
pub trait FilteredListFunctions: Clone + Send + Sync {
    fn init(&mut self);
    fn filter<S: AsRef<str>>(&self, filter_text: S) -> Vec<VaultRow>;
    fn on_entry(&mut self, element: &VaultRow) -> Task<KimunMessage>;
    fn header_element(&self, filter_text: &str) -> Option<VaultRow>;
    fn button_icon(&self) -> Option<String>;
}
