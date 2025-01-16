use std::sync::Arc;

use eframe::egui;
const MAIN_FONT: &str = "main_font";
const MONO_FONT: &str = "mono_font";
const ICON_FONT: &str = "icon_font";

pub fn set_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        MAIN_FONT.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../res/fonts/InterVariable.ttf"
        ))),
    );
    fonts.font_data.insert(
        MONO_FONT.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../res/fonts/FiraCode-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        ICON_FONT.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../res/noters.ttf"
        ))),
    );

    let font_proportional_families = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    font_proportional_families.insert(0, MAIN_FONT.into());
    font_proportional_families.insert(1, ICON_FONT.into());
    let font_mono_families = fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default();
    font_mono_families.insert(0, MONO_FONT.into());

    ctx.set_fonts(fonts);
}

pub const NOTE: &str = "\u{E800}";
pub const DIRECTORY: &str = "\u{E802}";
pub const ATTACHMENT: &str = "\u{E803}";
