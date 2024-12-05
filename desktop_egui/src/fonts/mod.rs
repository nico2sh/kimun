pub mod icons;
use eframe::egui::{self, FontDefinitions};

pub fn set_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    set_icon_fonts(&mut fonts);
    set_ui_fonts(&mut fonts);

    ctx.set_fonts(fonts);
}

fn set_icon_fonts(fonts: &mut FontDefinitions) {
    let icons = egui::FontData::from_static(include_bytes!("../../res/noters.ttf"));

    fonts.font_data.insert("icons".to_owned(), icons);

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(1, "icons".to_owned());
}

fn set_ui_fonts(fonts: &mut FontDefinitions) {
    let inter_variable = egui::FontData::from_static(include_bytes!("../../res/InterVariable.ttf"));
    let inter_variable_italic =
        egui::FontData::from_static(include_bytes!("../../res/InterVariable-Italic.ttf"));

    fonts
        .font_data
        .insert("inter_variable".to_owned(), inter_variable);
    fonts
        .font_data
        .insert("inter_variable_italic".to_owned(), inter_variable_italic);

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "inter_variable".to_owned());
}
