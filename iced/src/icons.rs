use iced::Font;

pub const ICON_BYTES: &[u8] = include_bytes!("../res/icons.ttf");
pub const ICON: Font = Font::with_name("icons");

pub enum KimunIcon {
    Note,
    Directory,
    Attachment,
    SortUp,
    SortDown,
    SortNameUp,
    SortNameDown,
    List,
}

impl KimunIcon {
    pub fn get_char(&self) -> char {
        match self {
            KimunIcon::Note => '\u{E800}',
            KimunIcon::Directory => '\u{E802}',
            KimunIcon::Attachment => '\u{E803}',
            KimunIcon::SortUp => '\u{F160}',
            KimunIcon::SortDown => '\u{F161}',
            KimunIcon::SortNameUp => '\u{F15D}',
            KimunIcon::SortNameDown => '\u{F15F}',
            KimunIcon::List => '\u{E801}',
        }
    }
}

impl From<KimunIcon> for char {
    fn from(icon: KimunIcon) -> Self {
        icon.get_char()
    }
}
