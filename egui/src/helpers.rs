use eframe::egui;

#[cfg(not(target_os = "macos"))]
pub fn cmd_ctrl() -> String {
    "ctrl".to_string()
}

#[cfg(target_os = "macos")]
pub fn cmd_ctrl() -> String {
    "cmd".to_string()
}

pub fn info_pill<S: AsRef<str>>(ui: &mut egui::Ui, label: S) {
    egui::Frame::default()
        .inner_margin(2.0)
        .outer_margin(2.0)
        .corner_radius(5.0)
        .fill(ui.visuals().code_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .show(ui, |ui| {
            ui.add(egui::Label::new(label.as_ref()));
        });
}
