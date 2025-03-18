use iced::{Font, font::Family};

pub const FONT_UI_BYTES: &[u8] = include_bytes!("../res/fonts/InterVariable.ttf");
pub const FONT_CODE_BYTES: &[u8] = include_bytes!("../res/fonts/FiraCode-Regular.ttf");

pub const FONT_UI: Font = Font::with_name("Inter");
pub const FONT_UI_ITALIC: Font = Font {
    family: Family::Name("Inter"),
    style: iced::font::Style::Italic,
    ..Font::DEFAULT
};

pub const FONT_CODE: Font = Font::with_name("Fira Code");
