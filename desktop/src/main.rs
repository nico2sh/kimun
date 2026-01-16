#![cfg_attr(feature = "bundle", windows_subsystem = "windows")]
use components::focus_manager::FocusManager;
use dioxus::prelude::*;

// use dioxus_radio::hooks::use_init_radio_station;
use editor_state::EditorState;
use route::Route;
use settings::AppSettings;

use crate::{
    app_state::AppState,
    global_events::{GlobalEvent, PubSub},
};

pub mod app_state;
mod components;
pub mod editor_state;
pub mod global_events;
mod pages;
mod route;
mod settings;
pub mod utils;

// Urls are relative to your Cargo.toml file
#[used]
static ICON_FONT: Asset = asset!("/assets/fonts/fontello.woff2");
#[used]
static APP_FONT: Asset = asset!("/assets/fonts/InterVariable.woff2");
#[used]
static APP_FONT_ITALIC: Asset = asset!("/assets/fonts/InterVariable-Italic.woff2");
#[used]
static APP_LOGO: Asset = asset!("/assets/images/kimun.png");

const FONTS_STYLE: Asset = asset!("/assets/styling/fonts.css");
const ICONS_STYLE: Asset = asset!("/assets/styling/fontello.css");
const MAIN_STYLE: Asset = asset!("/assets/styling/main.css");
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
    let app_state = use_signal(|| AppState::new(&app_settings.read()));
    use_context_provider(move || app_state);
    let pub_sub = PubSub::<GlobalEvent>::new();
    use_context_provider(move || pub_sub);
    let focus_manager = FocusManager::new();
    use_context_provider(move || focus_manager);
    let theme = app_settings.read().get_theme();

    use_context_provider(|| Signal::new(EditorState::default()));
    // use_init_radio_station::<AppState, KimunChannel>(AppState::default);

    rsx! {
        document::Link { rel: "stylesheet", href: theme.css }
        document::Link { rel: "stylesheet", href: FONTS_STYLE }
        document::Link { rel: "stylesheet", href: ICONS_STYLE }
        document::Link { rel: "stylesheet", href: MAIN_STYLE }

        div { class: "app-container", Router::<Route> {} }
    }
}
