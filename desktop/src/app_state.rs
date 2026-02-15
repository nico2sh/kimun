use std::fmt::Display;

use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::{
    components::{
        focus_manager::FocusManager,
        modal::ModalType,
        note_list::{note_browse_entry::SortCriteria, note_list_loader::SearchStateData},
    },
    settings::AppSettings,
};

#[derive(Clone, Debug, PartialEq)]
pub struct PreviewListState {
    pub source: String,
    pub sort_criteria: SortCriteria,
    pub sort_ascending: bool,
}

impl Display for PreviewListState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Source: {}, Sort Criteria: {:?}, Sort Ascending: {}",
            self.source, self.sort_criteria, self.sort_ascending
        )
    }
}

impl PreviewListState {
    pub fn new(source: String, sort_criteria: SortCriteria, sort_ascending: bool) -> Self {
        Self {
            source,
            sort_criteria,
            sort_ascending,
        }
    }

    pub fn from_source(source: SearchStateData) -> Self {
        Self {
            source: source.filter_value,
            sort_criteria: source.sort_criteria,
            sort_ascending: source.sort_ascending,
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
    modal_manager: ModalType,
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
            modal_manager: ModalType::None,
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

    pub fn get_modal_mut(&mut self) -> &mut ModalType {
        &mut self.modal_manager
    }

    pub fn get_modal(&self) -> &ModalType {
        &self.modal_manager
    }

    pub fn close_modal(&mut self) {
        let focus_manager = use_context::<FocusManager>();
        self.modal_manager = ModalType::None;
        focus_manager.focus_prev();
    }

    pub fn set_modal(&mut self, modal: ModalType) {
        self.modal_manager = modal;
    }
}
