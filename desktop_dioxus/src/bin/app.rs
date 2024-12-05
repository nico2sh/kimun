#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use log::info;

const _FONTS: &str = manganis::mg!(file("./assets/fonts.css"));
const _COLORS: &str = manganis::mg!(file("./assets/theme.css"));
const _STYLE: &str = manganis::mg!(file("./assets/main.css"));

fn main() {
    // Init logger
    env_logger::Builder::new()
        .filter(Some("noters"), log::LevelFilter::max())
        .init();
    info!("starting app");
    // Urls are relative to your Cargo.toml file

    dioxus::launch(desktop_dioxus::App);
}
