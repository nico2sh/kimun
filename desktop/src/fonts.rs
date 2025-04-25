use iced::Font;

pub const FONT_UI_BYTES: &[u8] = include_bytes!("../res/fonts/InterVariable.ttf");
pub const FONT_CODE_BYTES: &[u8] = include_bytes!("../res/fonts/FiraCode-Regular.ttf");

pub const FONT_UI: Font = Font::with_name("Inter");
pub const FONT_UI_ITALIC: Font = Font {
    style: iced::font::Style::Italic,
    ..FONT_UI
};
pub const FONT_UI_BOLD: Font = Font {
    weight: iced::font::Weight::Bold,
    ..FONT_UI
};

pub const FONT_CODE: Font = Font::with_name("Fira Code");
