use std::sync::{Arc, LazyLock, Mutex};

use iced::{
    Background, Element, Length, Padding, Task, Theme, border,
    mouse::Interaction,
    theme::palette,
    widget::{container, mouse_area, text_input},
};
use kimun_core::{ResultType, SearchResult, nfs::VaultPath};
use log::debug;

use crate::{
    KimunMessage,
    editor::EditorMsg,
    fonts::{FONT_UI, FONT_UI_ITALIC},
    icons::{ICON, KimunIcon},
    style_units::{LARGE_PADDING, SMALL_SPACING},
};

use super::{
    KimunComponent, KimunListElement,
    list::{KimunList, RowSelection},
};

static TEXT_INPUT_ID: LazyLock<text_input::Id> = LazyLock::new(text_input::Id::unique);

#[derive(Debug, Clone)]
pub enum ListViewMsg {
    Initializing(Option<VaultRow>),
    Filter,
    UpdateFilterText { filter: String },
    Ready { filter: String, data: Vec<VaultRow> },
    PreviewUpdated(String),
    SelectHeader,
}

#[derive(Debug, Clone)]
pub enum SortMode {
    FileUp,
    FileDown,
    TitleUp,
    TitleDown,
}

impl From<ListViewMsg> for KimunMessage {
    fn from(value: ListViewMsg) -> Self {
        KimunMessage::ListViewMessage(value)
    }
}

impl std::fmt::Display for ListViewMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListViewMsg::Initializing(_) => write!(f, "Initializing"),
            ListViewMsg::Filter => write!(f, "Initialized"),
            ListViewMsg::UpdateFilterText { filter } => write!(f, "Filtering with {}", filter),
            ListViewMsg::Ready { filter, data: _ } => {
                write!(f, "Filtered with filter `{}`", filter)
            }
            ListViewMsg::PreviewUpdated(_) => write!(f, "Updated Preview"),
            ListViewMsg::SelectHeader => write!(f, "Selected Header"),
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
    list: KimunList<VaultRow>,
    header: Option<VaultRow>,
}

impl<F> FilteredList<F>
where
    F: FilteredListFunctions + 'static,
{
    pub fn new(fun: F) -> (Self, iced::Task<KimunMessage>) {
        let functions = Arc::new(Mutex::new(fun));
        let filter_text = String::new();
        let list = KimunList::new();
        (
            Self {
                functions,
                ready: false,
                filter_text,
                list,
                header: None,
            },
            iced::Task::batch([
                text_input::focus(TEXT_INPUT_ID.clone()),
                Task::done(ListViewMsg::Initializing(None).into()),
            ]),
        )
    }

    fn get_header_view(&self, header: &VaultRow) -> Element<'_, KimunMessage> {
        let selected = false;
        let v = iced::widget::row![
            iced::widget::text(KimunIcon::Note.get_char()).font(ICON),
            iced::widget::text("Create new note: ").font(FONT_UI),
            iced::widget::text(header.path.to_string()).font(FONT_UI_ITALIC),
        ]
        .spacing(SMALL_SPACING);
        let cont = iced::widget::container(v)
            .padding(LARGE_PADDING)
            .width(Length::Fill)
            .style(move |t| row_style(t, selected));
        let ma = mouse_area(cont)
            .on_press(ListViewMsg::SelectHeader.into())
            .interaction(Interaction::Pointer)
            .on_enter(KimunMessage::Select(RowSelection::None));

        ma.into()
    }

    pub fn get_selection(&self) -> Option<VaultRow> {
        self.list.get_selection()
    }

    fn internal_update(&mut self, message: ListViewMsg) -> Task<KimunMessage> {
        match message {
            ListViewMsg::Initializing(row) => {
                debug!("Initializing...");
                self.ready = false;
                self.list.select_none();
                let functions = self.functions.clone();
                Task::perform(async move { functions.lock().unwrap().init(row) }, |_| {
                    ListViewMsg::Filter.into()
                })
            }
            ListViewMsg::Filter => {
                debug!("Filter...");
                self.ready = false;
                self.header = self
                    .functions
                    .lock()
                    .unwrap()
                    .header_element(&self.filter_text);

                let functions = self.functions.clone();
                let filter_text = self.filter_text.clone();
                Task::perform(
                    async move { (functions.lock().unwrap().filter(&filter_text), filter_text) },
                    move |(result, filter)| {
                        ListViewMsg::Ready {
                            filter,
                            data: result,
                        }
                        .into()
                    },
                )
            }
            ListViewMsg::UpdateFilterText { filter } => {
                debug!("Updating the filter text");
                self.filter_text = filter;
                if self.ready {
                    // If it is ready, we retrigger the filter
                    Task::done(ListViewMsg::Filter.into())
                } else {
                    // If it is not ready, we don't do anything
                    // as we wait to be ready, this way we don't
                    // batch filter requests
                    Task::none()
                }
            }
            ListViewMsg::Ready { filter, data } => {
                debug!("Filtered!");
                self.list.set_elements(data);
                if filter != self.filter_text {
                    self.ready = false;
                    Task::done(ListViewMsg::Filter.into())
                } else {
                    self.ready = true;
                    Task::none()
                }
            }
            // ListViewMessage::Select(row_selection) => self.list.update(row_selection),
            ListViewMsg::PreviewUpdated(_string) => {
                // We don't do anything, this is just to notify we loaded the preview
                debug!("Updated Preview");
                Task::none()
            }
            ListViewMsg::SelectHeader => {
                if let Some(h) = &self.header {
                    h.on_select()
                } else {
                    Task::none()
                }
            }
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

impl<F> KimunComponent for FilteredList<F>
where
    F: FilteredListFunctions,
{
    fn update(&mut self, message: KimunMessage) -> iced::Task<KimunMessage> {
        if let KimunMessage::ListViewMessage(message) = message {
            self.internal_update(message)
        } else {
            self.list.update(message)
        }
    }

    fn view(&self) -> iced::Element<KimunMessage> {
        let text_filter = text_input("Search...", &self.filter_text)
            .on_input(|filter| {
                KimunMessage::ListViewMessage(ListViewMsg::UpdateFilterText { filter })
            })
            .id(TEXT_INPUT_ID.clone())
            .on_submit(KimunMessage::Select(RowSelection::Enter));

        // Insert header here
        if let Some(head_row) = &self.header {
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
        self.list.key_press(key, modifiers)
    }
}

/// The function that customize the behavior of the filtered list
/// Provides a function on how to filter, and how to behave on each entry
/// when clicked or selected. Also provides an optional first entry/header
/// under the list
pub trait FilteredListFunctions: Clone + Send + Sync {
    fn init(&mut self, row: Option<VaultRow>);
    fn filter<S: AsRef<str>>(&self, filter_text: S) -> Vec<VaultRow>;
    fn header_element(&self, filter_text: &str) -> Option<VaultRow>;
}

#[derive(Clone, Debug)]
pub struct VaultRow {
    pub path: VaultPath,
    pub path_str: String,
    pub search_str: String,
    pub entry_type: VaultRowType,
}

impl VaultRow {
    pub fn up_dir(from_path: &VaultPath) -> Self {
        let parent = from_path.get_parent_path().0;
        Self {
            path: parent,
            path_str: "..".to_string(),
            search_str: ".. up".to_string(),
            entry_type: VaultRowType::Directory,
        }
    }

    pub fn create_new_note(base_path: &VaultPath, note_text: &str) -> Self {
        let file_name = VaultPath::note_path_from(note_text);
        let path = base_path.append(&file_name);

        Self {
            path_str: path.to_string(),
            path,
            search_str: "New Note".to_string(),
            entry_type: VaultRowType::NewNote,
        }
    }

    pub fn get_sort_string(&self) -> String {
        match &self.entry_type {
            VaultRowType::Note { title: _ } => format!("2{}", self.path),
            VaultRowType::Directory => format!("1{}", self.path),
            VaultRowType::Attachment => format!("3{}", self.path),
            VaultRowType::NewNote => "0".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum VaultRowType {
    Note { title: String },
    Directory,
    Attachment,
    NewNote,
}

impl VaultRowType {
    pub fn get_order(&self) -> usize {
        match self {
            VaultRowType::Note { title: _ } => 2,
            VaultRowType::Directory => 1,
            VaultRowType::Attachment => 3,
            VaultRowType::NewNote => 0,
        }
    }
}

impl PartialOrd for VaultRowType {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (VaultRowType::Note { title: title1 }, VaultRowType::Note { title: title2 }) => {
                title1.partial_cmp(title2)
            }
            _ => self.get_order().partial_cmp(&other.get_order()),
        }
    }
}

impl From<SearchResult> for VaultRow {
    fn from(value: SearchResult) -> Self {
        match value.rtype {
            ResultType::Note(content_data) => {
                let title = content_data.title;
                let path = value.path;
                let file_name = path.get_parent_path().1;
                let file_name_no_ext = file_name.strip_suffix(".md").unwrap_or(file_name.as_str());
                let search_str = if title.contains(file_name_no_ext) {
                    title.clone()
                } else {
                    format!("{} {}", title, file_name_no_ext)
                };
                VaultRow {
                    path: path.clone(),
                    path_str: path.get_parent_path().1,
                    search_str,
                    entry_type: VaultRowType::Note { title },
                }
            }
            ResultType::Directory => {
                let name = value.path.get_parent_path().1;
                VaultRow {
                    path: value.path.clone(),
                    path_str: name.clone(),
                    search_str: name,
                    entry_type: VaultRowType::Directory,
                }
            }
            ResultType::Attachment => {
                let name = value.path.get_parent_path().1;
                VaultRow {
                    path: value.path.clone(),
                    path_str: name.clone(),
                    search_str: name,
                    entry_type: VaultRowType::Attachment,
                }
            }
        }
    }
}

impl KimunListElement for VaultRow {
    fn get_view(&self) -> Element<KimunMessage> {
        let path = self.path_str.to_string();
        match &self.entry_type {
            VaultRowType::Note { title } => {
                // two rows
                iced::widget::row![
                    iced::widget::text(KimunIcon::Note.get_char()).font(ICON),
                    iced::widget::column![
                        iced::widget::text(title.to_owned()).font(FONT_UI),
                        iced::widget::text(path).font(FONT_UI_ITALIC)
                    ]
                ]
                .spacing(8)
                .into()
            }
            VaultRowType::Directory => {
                // one row
                iced::widget::row![
                    iced::widget::text(KimunIcon::Directory.get_char()).font(ICON),
                    iced::widget::text(path).font(FONT_UI)
                ]
                .spacing(8)
                .into()
            }
            VaultRowType::Attachment => todo!(),
            VaultRowType::NewNote => todo!(),
        }
    }

    fn get_height(&self) -> f32 {
        match &self.entry_type {
            VaultRowType::Note { title: _ } => 44.0,
            VaultRowType::Directory => 24.0,
            VaultRowType::Attachment => 22.0,
            VaultRowType::NewNote => 24.0,
        }
    }

    fn on_select(&self) -> Task<KimunMessage> {
        match self.entry_type {
            VaultRowType::Note { title: _ } => {
                debug!("Open note");
                // We close first the modal, then we open the note
                Task::batch([
                    Task::done(KimunMessage::CloseModal),
                    Task::done(KimunMessage::EditorMessage(EditorMsg::OpenNote(
                        self.path.clone(),
                    ))),
                ])
            }
            VaultRowType::Directory => {
                let directory = self.clone();
                // let new_one = Self::new(self.path.clone(), self.vault.clone());

                Task::done(KimunMessage::ListViewMessage(ListViewMsg::Initializing(
                    Some(directory),
                )))
            }
            VaultRowType::Attachment => Task::none(),
            VaultRowType::NewNote => {
                debug!("New note");
                // We close first the modal, then we open the note
                Task::batch([
                    Task::done(KimunMessage::CloseModal),
                    Task::done(KimunMessage::EditorMessage(EditorMsg::NewNote(
                        self.path.clone(),
                    ))),
                ])
            }
        }
    }
}

impl AsRef<str> for VaultRow {
    fn as_ref(&self) -> &str {
        &self.search_str
    }
}
