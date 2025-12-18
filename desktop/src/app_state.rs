use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::settings::AppSettings;

pub struct AppState {
    pub current_path: VaultPath,
    pub create_if_not_exists: bool,
}

impl AppState {
    pub fn new(settings: &AppSettings) -> Self {
        let starting_path = settings
            .last_paths
            .last()
            .map_or_else(VaultPath::root, |p| p.to_owned());
        debug!("Starting path found: {starting_path}");

        Self {
            current_path: starting_path,
            create_if_not_exists: false,
        }
    }

    pub fn set_path(&mut self, path: &VaultPath, create_if_not_exists: bool) {
        self.current_path = path.to_owned();
        self.create_if_not_exists = create_if_not_exists;
    }
}
