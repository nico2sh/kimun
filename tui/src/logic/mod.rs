//! The **Logic layer**: feature state, rules and intents with no UI crate in
//! sight. `.github/scripts/check-logic-layer.sh` fails CI if anything under this
//! tree names `ratatui`, `crossterm`, `egui`, `eframe` or `gpui`. See ADR-0049.

pub mod actions;
pub mod ask_flow;
pub mod autocomplete;
pub mod clipboard;
pub mod input;
pub mod keyboard_help;
pub mod layout;
pub mod layout_tree;
pub mod list;

pub mod attachment;
pub mod doc_meta;
pub mod editor;
pub mod editor_region;
pub mod file_ops;
pub mod files;
pub mod follow;
pub mod leader_route;
pub mod pickers;
pub mod preview;
pub mod query;
pub mod saved_search;
pub mod view_registry;
pub mod views;

pub mod message;
pub mod nav;
pub mod onboarding;
pub mod overlay_open;
pub mod palette;
pub mod pinned;
pub mod preferences;
pub mod properties;
pub mod rag_caps;
pub mod session;
pub mod sort;
pub mod startup;
pub mod task_slot;
pub mod text_action;
pub mod text_match;
pub mod update;
pub mod which_key;
pub mod workspaces;
