#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use log::info;

fn main() {
    // Init logger
    env_logger::Builder::new()
        .filter(Some("noters"), log::LevelFilter::max())
        .init();
    info!("starting app");

    dioxus::launch(desktop_notes::App);
}
