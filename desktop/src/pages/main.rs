use dioxus::logger::tracing::debug;
use dioxus::{logger::tracing::info, prelude::*};
use kimun_core::NoteVault;

use crate::route::Route;
use crate::settings::AppSettings;

#[component]
pub fn Main() -> Element {
    let settings: Signal<AppSettings> = use_context();
    let workspace_dir = &settings.read().workspace_dir;
    if let Some(path) = workspace_dir {
        debug!("Opening workspace at {:?}", path);
        let navigation_target = NavigationTarget::Internal(Route::Editor {});

        navigator().push(navigation_target);
    } else {
        navigator().push(Route::Settings {});
    };

    rsx! {}
}
