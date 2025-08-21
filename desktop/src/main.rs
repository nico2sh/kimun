#![cfg_attr(feature = "bundle", windows_subsystem = "windows")]
use components::focus_manager::FocusManager;
use dioxus::prelude::*;

use dioxus_radio::hooks::use_init_radio_station;
use route::Route;
use settings::AppSettings;
use state::{AppState, KimunChannel};

use crate::global_events::{GlobalEvent, PubSub};

mod components;
pub mod global_events;
mod pages;
mod route;
mod settings;
pub mod state;
pub mod utils;

// The asset macro also minifies some assets like CSS and JS to make bundled smaller
// const MAIN_CSS: Asset = asset!("/assets/styling/main.css");
// Urls are relative to your Cargo.toml file
const FONTS: Asset = asset!("/assets/styling/fonts.css");
const ICONS: Asset = asset!("/assets/styling/icons.css");
const STYLE: Asset = asset!("/assets/styling/main.css");
pub const MARKDOWN_JS: Asset = asset!(
    "/assets/scripts/markdown.js",
    JsAssetOptions::new().with_minify(false).with_preload(true)
);

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let app_settings = use_signal(|| AppSettings::load_from_disk().unwrap());
    use_context_provider(move || app_settings);
    let pub_sub = PubSub::<GlobalEvent>::new();
    use_context_provider(move || pub_sub);
    let focus_manager = FocusManager::new();
    use_context_provider(move || focus_manager);
    let theme = app_settings.read().get_theme();

    use_init_radio_station::<AppState, KimunChannel>(AppState::default);

    rsx! {
        document::Link { rel: "stylesheet", href: theme.css }
        document::Link { rel: "stylesheet", href: FONTS }
        document::Link { rel: "stylesheet", href: ICONS }
        document::Link { rel: "stylesheet", href: STYLE }

        div { class: "app-container", Router::<Route> {} }
    }
}
