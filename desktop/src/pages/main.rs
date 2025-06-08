use dioxus::logger::tracing::debug;
use dioxus::{logger::tracing::info, prelude::*};
use kimun_core::NoteVault;

use crate::route::Route;
use crate::settings::Settings;

#[component]
pub fn Main() -> Element {
    let settings: Signal<Settings> = use_context();
    let workspace_dir = &settings.read().workspace_dir;
    if let Some(path) = workspace_dir {
        debug!("Opening workspace at {:?}", path);
        let last_path = settings.read().last_paths.last().map(|p| p.to_owned());

        let navigation_target = NavigationTarget::Internal(Route::Editor {
            note_path: last_path,
        });

        navigator().push(navigation_target);
    } else {
        navigator().push(Route::Settings {});
    };

    rsx! {}
}
