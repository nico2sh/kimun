use crate::route::Route;
use crate::settings::AppSettings;
use dioxus::logger::tracing::debug;
use dioxus::prelude::*;

#[component]
pub fn Start() -> Element {
    debug!("Starting");
    let settings: Signal<AppSettings> = use_context();
    let workspace_dir = &settings.read().workspace_dir;
    if let Some(_path) = workspace_dir {
        navigator().replace(Route::MainView {});
        // create false
    } else {
        navigator().replace(Route::Settings {});
    };

    rsx! {}
}
