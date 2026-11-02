//! Native egui frontend for kimün (ADR-0049). M0a skeleton: an empty window in
//! the user's theme.

mod theme;

use eframe::egui;
use kimun_notes::settings::AppSettings;

struct App;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("kimün");
        });
    }
}

fn main() -> eframe::Result {
    // Same file, same theme lookup as the TUI (ADR-0049 §6). The config is
    // never written, migrated or renamed here; the only possible side effect
    // is `theme_list()` creating an empty config directory. Not `get_theme()`:
    // that applies `adapt_to_terminal()`, which is a terminal-only concern.
    let settings = AppSettings::load_read_only().unwrap_or_else(|e| {
        eprintln!("kimun-gui: could not read the config ({e}); using the default theme");
        AppSettings::default()
    });
    let theme = if settings.theme.is_empty() {
        Default::default()
    } else {
        settings
            .theme_list()
            .into_iter()
            .find(|t| t.name == settings.theme)
            .unwrap_or_default()
    };
    let visuals = theme::visuals(&theme);
    eframe::run_native(
        "kimün",
        eframe::NativeOptions::default(),
        Box::new(move |cc| {
            // Both slots, so a system light/dark flip cannot swap the user's theme out.
            cc.egui_ctx
                .set_visuals_of(egui::Theme::Dark, visuals.clone());
            cc.egui_ctx.set_visuals_of(egui::Theme::Light, visuals);
            Ok(Box::new(App))
        }),
    )
}
