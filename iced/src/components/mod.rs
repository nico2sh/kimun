use iced::{Element, Task};
use kimun_core::{ResultType, SearchResult, nfs::VaultPath};

use crate::{
    KimunMessage,
    fonts::{FONT_UI, FONT_UI_ITALIC},
    icons::{ICON, KimunIcon},
};

pub mod filtered_list;

pub trait KimunComponent {
    type Message: TryFrom<KimunMessage>;

    fn update(&mut self, message: Self::Message) -> Task<KimunMessage>;
    fn view(&self) -> Element<KimunMessage>;
    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage>;
}

#[derive(Clone, Debug)]
pub struct VaultRow {
    pub path: VaultPath,
    pub path_str: String,
    pub search_str: String,
    pub entry_type: VaultRowType,
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

// impl ListElement for VaultRow {
//     fn get_height_mult(&self) -> f32 {
//         match &self.entry_type {
//             VaultRowType::Note { title: _ } => 2.0,
//             VaultRowType::Directory => 1.0,
//             VaultRowType::Attachment => 1.0,
//             VaultRowType::NewNote => 2.0,
//         }
//     }
//
//     fn get_icon(&self) -> impl Into<egui::WidgetText> {
//         match &self.entry_type {
//             VaultRowType::Note { title: _ } => fonts::NOTE.to_string(),
//             VaultRowType::Directory => fonts::DIRECTORY.to_string(),
//             VaultRowType::Attachment => fonts::ATTACHMENT.to_string(),
//             VaultRowType::NewNote => {
//                 format!("{}+enter", helpers::cmd_ctrl())
//             }
//         }
//     }
//
//     fn get_label(&self) -> impl Into<egui::WidgetText> {
//         match &self.entry_type {
//             VaultRowType::Note { title } => {
//                 let path = self.path_str.to_owned();
//                 format!("{}\n{}", title, path)
//             }
//             VaultRowType::Directory => {
//                 let path = self.path_str.to_owned();
//                 path.to_string()
//             }
//             VaultRowType::Attachment => {
//                 let path = self.path_str.to_owned();
//                 path.to_string()
//             }
//             VaultRowType::NewNote => {
//                 let path = self.path_str.to_owned();
//                 format!("Create new note at:\n`{}`", path)
//             }
//         }
//     }
// }

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
}

// fn custom_button(theme: &Theme, status: Status) -> Style {
//     let palette = theme.extended_palette();
//     let base = styled(palette.background.weak);
//
//     match status {
//         Status::Active | Status::Disabled => base,
//         Status::Hovered | Status::Pressed => Style {
//             background: Some(Background::Color(palette.background.strong.color)),
//             ..base
//         },
//         // Status::Disabled => disabled(base),
//     }
// }
//
// fn styled(pair: palette::Pair) -> Style {
//     Style {
//         background: Some(Background::Color(pair.color)),
//         text_color: pair.text,
//         border: border::rounded(2),
//         ..Style::default()
//     }
// }
//
// fn _disabled(style: Style) -> Style {
//     Style {
//         background: style
//             .background
//             .map(|background| background.scale_alpha(0.5)),
//         text_color: style.text_color.scale_alpha(0.5),
//         ..style
//     }
// }

impl AsRef<str> for VaultRow {
    fn as_ref(&self) -> &str {
        &self.search_str
    }
}
