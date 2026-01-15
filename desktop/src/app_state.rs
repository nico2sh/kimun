use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::{
    components::{note_list::note_browse_entry::SortCriteria, preview_pane::PreviewListSource},
    settings::AppSettings,
};

#[derive(Clone, PartialEq)]
pub struct PreviewListState {
    pub source: PreviewListSource,
    pub sort_criteria: SortCriteria,
    pub sort_ascending: bool,
}

impl PreviewListState {
    pub fn new(
        source: PreviewListSource,
        sort_criteria: SortCriteria,
        sort_ascending: bool,
    ) -> Self {
        Self {
            source,
            sort_criteria,
            sort_ascending,
        }
    }

    pub fn from_source(source: PreviewListSource) -> Self {
        Self {
            source,
            sort_criteria: SortCriteria::None,
            sort_ascending: true,
        }
    }
}

impl Default for PreviewListState {
    fn default() -> Self {
        Self {
            source: Default::default(),
            sort_criteria: Default::default(),
            sort_ascending: Default::default(),
        }
    }
}

pub struct AppState {
    pub current_path: VaultPath,
    pub create_if_not_exists: bool,
    pub preview_mode: bool,
    pub show_browser: bool,
    pub show_preview_pane: Option<PreviewListState>,
    last_preview_list_state: PreviewListState,
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
            preview_mode: false,
            show_browser: false,
            show_preview_pane: None,
            last_preview_list_state: PreviewListState::default(),
        }
    }

    pub fn set_path(&mut self, path: &VaultPath, create_if_not_exists: bool) {
        self.current_path = path.to_owned();
        self.create_if_not_exists = create_if_not_exists;
    }

    pub fn toggle_browser(&mut self) {
        self.show_browser = !self.show_browser;
    }

    pub fn show_preview_pane(&mut self, state: Option<PreviewListState>) {
        if let Some(state) = state {
            self.set_preview_pane_state(state);
        } else {
            self.show_preview_pane = Some(self.last_preview_list_state.clone());
        }
    }

    pub fn hide_preview_pane(&mut self) {
        self.show_preview_pane = None;
    }

    pub fn set_preview_pane_state(&mut self, state: PreviewListState) {
        self.last_preview_list_state = state;
    }
}
