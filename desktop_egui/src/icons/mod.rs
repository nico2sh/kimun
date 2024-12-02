use eframe::egui;

pub fn set_icon_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("icons".into(), font_data());
    if let Some(font_keys) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        font_keys.insert(1, "icons".into());
    }

    ctx.set_fonts(fonts);
}

fn font_bytes() -> &'static [u8] {
    &*include_bytes!("../../res/noters.ttf")
}

fn font_data() -> egui::FontData {
    egui::FontData::from_static(font_bytes())
}

pub const NOTE: &str = "\u{E800}";
pub const DIRECTORY: &str = "\u{E802}";
pub const ATTACHMENT: &str = "\u{E803}";
