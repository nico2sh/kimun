use dioxus::logger::tracing::debug;
use dioxus::{logger::tracing::info, prelude::*};
use kimun_core::nfs::VaultPath;
use kimun_core::NoteVault;

use crate::route::Route;
use crate::settings::AppSettings;

#[component]
pub fn Main() -> Element {
    let settings: Signal<AppSettings> = use_context();
    let workspace_dir = &settings.read().workspace_dir;
    if let Some(path) = workspace_dir {
        debug!("[Main] Opening workspace at {:?}", path);
        let note_path = settings
            .read()
            .last_paths
            .last()
            .map_or_else(|| VaultPath::root(), |p| p.to_owned());
        navigator().replace(Route::Editor {
            note_path,
            create: false,
        });
    } else {
        navigator().replace(Route::Settings {});
    };

    rsx! {}
}
