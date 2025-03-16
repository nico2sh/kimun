use iced::Font;

pub const ICON_BYTES: &[u8] = include_bytes!("../res/icons.ttf");
pub const ICON: Font = Font::with_name("icons");

pub enum Icon {
    Note,
    Directory,
    Attachment,
    SortUp,
    SortDown,
    SortNameUp,
    SortNameDown,
    List,
}

impl Icon {
    pub fn get_char(&self) -> char {
        match self {
            Icon::Note => '\u{E800}',
            Icon::Directory => '\u{E802}',
            Icon::Attachment => '\u{E803}',
            Icon::SortUp => '\u{F160}',
            Icon::SortDown => '\u{F161}',
            Icon::SortNameUp => '\u{F15D}',
            Icon::SortNameDown => '\u{F15F}',
            Icon::List => '\u{E801}',
        }
    }
}

impl From<Icon> for char {
    fn from(icon: Icon) -> Self {
        icon.get_char()
    }
}
