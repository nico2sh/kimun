use dioxus::logger::tracing::debug;
use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::route::Route;
use crate::settings::AppSettings;
use crate::utils::encode_path;

#[component]
pub fn Start() -> Element {
    debug!("Starting");
    let settings: Signal<AppSettings> = use_context();
    let workspace_dir = &settings.read().workspace_dir;
    if let Some(path) = workspace_dir {
        debug!("[Main] Opening workspace at {:?}", path);
        let editor_path = settings
            .read()
            .last_paths
            .last()
            .map_or_else(VaultPath::root, |p| p.to_owned());
        debug!("Starting path found: {editor_path}");
        let encoded_path = encode_path(&editor_path);
        navigator().replace(Route::MainView {
            encoded_path,
            create: false,
        });
    } else {
        navigator().replace(Route::Settings {});
    };

    rsx! {}
}
