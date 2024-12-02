use desktop_egui::DesktopApp;
use eframe::egui::{self};

fn main() -> eframe::Result {
    env_logger::Builder::new()
        .filter(Some("note"), log::LevelFilter::max())
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 768.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Note",
        options,
        Box::new(|cc| Ok(Box::new(DesktopApp::new(cc)?))),
    )
}
