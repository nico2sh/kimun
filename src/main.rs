#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

pub mod core_notes;
pub mod desktop_app;

use desktop_app::App;
use dioxus_logger::tracing::{info, Level};

fn main() {
    // Init logger
    dioxus_logger::init(Level::DEBUG).expect("logger failed to init");
    // env_logger::Builder::new()
    //     .filter(Some("noters"), log::LevelFilter::max())
    //     .init();
    info!("starting app");

    dioxus::launch(App);
}
