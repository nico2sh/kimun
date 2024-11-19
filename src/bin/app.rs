#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]
use noters::desktop;

use log::info;

const _TAILWIND_URL: &str = manganis::mg!(file("./public/tailwind.css"));

fn main() {
    // Init logger
    env_logger::Builder::new()
        .filter(Some("noters"), log::LevelFilter::max())
        .init();
    info!("starting app");
    // Urls are relative to your Cargo.toml file

    dioxus::launch(desktop::app);
}
