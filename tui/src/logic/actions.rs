//! The shared action vocabulary (ADR-0050 §6): the names every front end binds
//! chords to. Only the chords differ per front end — the TUI's table lives in
//! `crate::keys`, the GUI's in `gui/` — so the names, their on-disk spelling and
//! their order live here, free of any UI crate.

use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::logic::layout::Dir;
use crate::logic::text_action::TextAction;

/// A named app action a key chord can be bound to.
///
/// The `Display`/`TryFrom<String>` spellings are the stable names written to
/// `config.toml`; the variant order is the derived `Ord`, which sorts the
/// serialized keymap. New variants go at the end so neither changes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ActionShortcuts {
    Quit,
    OpenPreferences,
    SearchNotes,
    OpenNote,
    NewJournal,
    Text(TextAction),
    // TUI navigation / file list
    ToggleSidebar,
    OpenFileBrowser,
    FocusEditor,
    FocusSidebar,
    OpenSortDialog,
    // File operations
    FileOperations,
    // Editor link navigation
    FollowLink,
    // Quick capture
    QuickNote,
    /// Copy the selected list row's yank target (`YankTarget` in the TUI's
    /// search list) to the OS clipboard. Claimed only when the editor is not
    /// focused, so the editor keeps this chord for redo.
    YankRow,
    // Query panel
    ToggleQueryPanel,
    OpenSavedSearches,
    SaveCurrentQuery,
    /// Switch to the Ask workspace (ask a question, get an LLM answer with
    /// cited sources).
    OpenAsk,
    // Workspace
    SwitchWorkspace,
    // In-buffer find (Ctrl+F by default; reopens / advances to next match if
    // already open).
    FindInBuffer,
    // In-buffer replace: opens the find bar with the replace field already
    // revealed. Ships with no default chord — the Ctrl-letter namespace is
    // full, and `Tab` from an open find bar reaches the same state.
    ReplaceInBuffer,
    /// The leader gateway (Ctrl+B by default): starts a key sequence against
    /// the leader tree in every context, including mid-typing.
    Leader,
    /// The command palette (Ctrl+Shift+P by default): every leader command
    /// as a fuzzy list.
    OpenCommandPalette,
    // Layout (ADR-0050). Geometric focus between visible regions, drawer view
    // placement and slot splitting. The TUI binds none of these by default.
    /// Move focus to the neighbouring region in that direction (no wrap;
    /// Up/Down only between stacked slots or groups). On disk it is one name
    /// per direction, `FocusRegionLeft` … `FocusRegionDown`, the way `Text`
    /// spells each `TextAction`; `Dir`'s order keeps the sort unchanged.
    FocusRegion(Dir),
    /// Move the focused drawer view into the left drawer.
    MoveViewToLeftDrawer,
    /// Move the focused drawer view into the right drawer.
    MoveViewToRightDrawer,
    /// Split the focused drawer into two stacked slots.
    SplitDrawerSlot,
    /// Close the focused drawer slot, leaving the drawer unsplit.
    UnsplitDrawerSlot,
    /// Show or hide the right drawer.
    ToggleRightDrawer,
}

impl ActionShortcuts {
    /// Every action, in `Ord` order. `Text` is listed once per fixed
    /// [`TextAction`] plus header levels 1 to 6.
    pub const ALL: [ActionShortcuts; 45] = [
        ActionShortcuts::Quit,
        ActionShortcuts::OpenPreferences,
        ActionShortcuts::SearchNotes,
        ActionShortcuts::OpenNote,
        ActionShortcuts::NewJournal,
        ActionShortcuts::Text(TextAction::Bold),
        ActionShortcuts::Text(TextAction::Italic),
        ActionShortcuts::Text(TextAction::Link),
        ActionShortcuts::Text(TextAction::Image),
        ActionShortcuts::Text(TextAction::ToggleHeader),
        ActionShortcuts::Text(TextAction::Header(1)),
        ActionShortcuts::Text(TextAction::Header(2)),
        ActionShortcuts::Text(TextAction::Header(3)),
        ActionShortcuts::Text(TextAction::Header(4)),
        ActionShortcuts::Text(TextAction::Header(5)),
        ActionShortcuts::Text(TextAction::Header(6)),
        ActionShortcuts::Text(TextAction::Underline),
        ActionShortcuts::Text(TextAction::Strikethrough),
        ActionShortcuts::ToggleSidebar,
        ActionShortcuts::OpenFileBrowser,
        ActionShortcuts::FocusEditor,
        ActionShortcuts::FocusSidebar,
        ActionShortcuts::OpenSortDialog,
        ActionShortcuts::FileOperations,
        ActionShortcuts::FollowLink,
        ActionShortcuts::QuickNote,
        ActionShortcuts::YankRow,
        ActionShortcuts::ToggleQueryPanel,
        ActionShortcuts::OpenSavedSearches,
        ActionShortcuts::SaveCurrentQuery,
        ActionShortcuts::OpenAsk,
        ActionShortcuts::SwitchWorkspace,
        ActionShortcuts::FindInBuffer,
        ActionShortcuts::ReplaceInBuffer,
        ActionShortcuts::Leader,
        ActionShortcuts::OpenCommandPalette,
        ActionShortcuts::FocusRegion(Dir::Left),
        ActionShortcuts::FocusRegion(Dir::Right),
        ActionShortcuts::FocusRegion(Dir::Up),
        ActionShortcuts::FocusRegion(Dir::Down),
        ActionShortcuts::MoveViewToLeftDrawer,
        ActionShortcuts::MoveViewToRightDrawer,
        ActionShortcuts::SplitDrawerSlot,
        ActionShortcuts::UnsplitDrawerSlot,
        ActionShortcuts::ToggleRightDrawer,
    ];

    /// The layout direction a `FocusRegion` action moves focus in.
    pub fn focus_dir(&self) -> Option<Dir> {
        match self {
            ActionShortcuts::FocusRegion(dir) => Some(*dir),
            _ => None,
        }
    }
}

impl Display for ActionShortcuts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let action = match self {
            ActionShortcuts::Quit => "Quit".to_string(),
            ActionShortcuts::OpenPreferences => "OpenSettings".to_string(),
            ActionShortcuts::SearchNotes => "SearchNotes".to_string(),
            ActionShortcuts::OpenNote => "OpenNote".to_string(),
            ActionShortcuts::NewJournal => "NewJournal".to_string(),
            ActionShortcuts::Text(text_action) => format!("TextEditor-{}", text_action),
            ActionShortcuts::ToggleSidebar => "ToggleSidebar".to_string(),
            ActionShortcuts::OpenFileBrowser => "OpenFileBrowser".to_string(),
            ActionShortcuts::FocusEditor => "FocusEditor".to_string(),
            ActionShortcuts::FocusSidebar => "FocusSidebar".to_string(),
            ActionShortcuts::OpenSortDialog => "OpenSortDialog".to_string(),
            ActionShortcuts::FileOperations => "FileOperations".to_string(),
            ActionShortcuts::FollowLink => "FollowLink".to_string(),
            ActionShortcuts::QuickNote => "QuickNote".to_string(),
            ActionShortcuts::YankRow => "YankRow".to_string(),
            ActionShortcuts::ToggleQueryPanel => "ToggleQueryPanel".to_string(),
            ActionShortcuts::OpenSavedSearches => "OpenSavedSearches".to_string(),
            ActionShortcuts::OpenAsk => "OpenAsk".to_string(),
            ActionShortcuts::SaveCurrentQuery => "SaveCurrentQuery".to_string(),
            ActionShortcuts::SwitchWorkspace => "SwitchWorkspace".to_string(),
            ActionShortcuts::FindInBuffer => "FindInBuffer".to_string(),
            ActionShortcuts::ReplaceInBuffer => "ReplaceInBuffer".to_string(),
            ActionShortcuts::Leader => "Leader".to_string(),
            ActionShortcuts::OpenCommandPalette => "OpenCommandPalette".to_string(),
            ActionShortcuts::FocusRegion(Dir::Left) => "FocusRegionLeft".to_string(),
            ActionShortcuts::FocusRegion(Dir::Right) => "FocusRegionRight".to_string(),
            ActionShortcuts::FocusRegion(Dir::Up) => "FocusRegionUp".to_string(),
            ActionShortcuts::FocusRegion(Dir::Down) => "FocusRegionDown".to_string(),
            ActionShortcuts::MoveViewToLeftDrawer => "MoveViewToLeftDrawer".to_string(),
            ActionShortcuts::MoveViewToRightDrawer => "MoveViewToRightDrawer".to_string(),
            ActionShortcuts::SplitDrawerSlot => "SplitDrawerSlot".to_string(),
            ActionShortcuts::UnsplitDrawerSlot => "UnsplitDrawerSlot".to_string(),
            ActionShortcuts::ToggleRightDrawer => "ToggleRightDrawer".to_string(),
        };
        write!(f, "{}", action)
    }
}

impl TryFrom<String> for ActionShortcuts {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let action = match value.as_str() {
            "Quit" => ActionShortcuts::Quit,
            // "OpenSettings" is the stable on-disk name; "OpenPreferences"
            // accepted as an alias since the screen is named Preferences now.
            "OpenSettings" | "OpenPreferences" => ActionShortcuts::OpenPreferences,
            "SearchNotes" => ActionShortcuts::SearchNotes,
            "OpenNote" => ActionShortcuts::OpenNote,
            "NewJournal" => ActionShortcuts::NewJournal,
            "ToggleSidebar" => ActionShortcuts::ToggleSidebar,
            "OpenFileBrowser" => ActionShortcuts::OpenFileBrowser,
            "FocusEditor" => ActionShortcuts::FocusEditor,
            "FocusSidebar" => ActionShortcuts::FocusSidebar,
            "OpenSortDialog" => ActionShortcuts::OpenSortDialog,
            "CycleSortField" => ActionShortcuts::OpenSortDialog,
            "SortReverseOrder" => ActionShortcuts::OpenSortDialog,
            "FileOperations" => ActionShortcuts::FileOperations,
            "FollowLink" => ActionShortcuts::FollowLink,
            "QuickNote" => ActionShortcuts::QuickNote,
            "YankRow" => ActionShortcuts::YankRow,
            "ToggleQueryPanel" => ActionShortcuts::ToggleQueryPanel,
            "ToggleBacklinks" => ActionShortcuts::ToggleQueryPanel,
            "OpenSavedSearches" => ActionShortcuts::OpenSavedSearches,
            // "OpenAsk" is the stable on-disk name; "OpenRagAnswer" is the
            // legacy name from the pre-workspace Ask overlay, kept so
            // existing keybinding configs keep working.
            "OpenAsk" | "OpenRagAnswer" => ActionShortcuts::OpenAsk,
            "SaveCurrentQuery" => ActionShortcuts::SaveCurrentQuery,
            "SwitchWorkspace" => ActionShortcuts::SwitchWorkspace,
            "FindInBuffer" => ActionShortcuts::FindInBuffer,
            "ReplaceInBuffer" => ActionShortcuts::ReplaceInBuffer,
            "Leader" => ActionShortcuts::Leader,
            "OpenCommandPalette" => ActionShortcuts::OpenCommandPalette,
            "FocusRegionLeft" => ActionShortcuts::FocusRegion(Dir::Left),
            "FocusRegionRight" => ActionShortcuts::FocusRegion(Dir::Right),
            "FocusRegionUp" => ActionShortcuts::FocusRegion(Dir::Up),
            "FocusRegionDown" => ActionShortcuts::FocusRegion(Dir::Down),
            "MoveViewToLeftDrawer" => ActionShortcuts::MoveViewToLeftDrawer,
            "MoveViewToRightDrawer" => ActionShortcuts::MoveViewToRightDrawer,
            "SplitDrawerSlot" => ActionShortcuts::SplitDrawerSlot,
            "UnsplitDrawerSlot" => ActionShortcuts::UnsplitDrawerSlot,
            "ToggleRightDrawer" => ActionShortcuts::ToggleRightDrawer,
            _ => {
                if let Some(text_action) = value.strip_prefix("TextEditor-") {
                    match TextAction::try_from(text_action.to_string()) {
                        Ok(ta) => ActionShortcuts::Text(ta),
                        Err(e) => return Err(format!("Error extracting Text Action: {}", e)),
                    }
                } else {
                    return Err(format!("Error, non valid Action: {}", value));
                }
            }
        };
        Ok(action)
    }
}

impl From<ActionShortcuts> for String {
    fn from(value: ActionShortcuts) -> Self {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Maps every variant to a distinct ordinal. The match has no wildcard, so
    /// a new variant fails to compile here until it gets an ordinal, and then
    /// `all_lists_every_variant` fails until it is in [`ActionShortcuts::ALL`].
    fn ordinal(a: &ActionShortcuts) -> usize {
        match a {
            ActionShortcuts::Quit => 0,
            ActionShortcuts::OpenPreferences => 1,
            ActionShortcuts::SearchNotes => 2,
            ActionShortcuts::OpenNote => 3,
            ActionShortcuts::NewJournal => 4,
            ActionShortcuts::Text(_) => 5,
            ActionShortcuts::ToggleSidebar => 6,
            ActionShortcuts::OpenFileBrowser => 7,
            ActionShortcuts::FocusEditor => 8,
            ActionShortcuts::FocusSidebar => 9,
            ActionShortcuts::OpenSortDialog => 10,
            ActionShortcuts::FileOperations => 11,
            ActionShortcuts::FollowLink => 12,
            ActionShortcuts::QuickNote => 13,
            ActionShortcuts::YankRow => 14,
            ActionShortcuts::ToggleQueryPanel => 15,
            ActionShortcuts::OpenSavedSearches => 16,
            ActionShortcuts::SaveCurrentQuery => 17,
            ActionShortcuts::OpenAsk => 18,
            ActionShortcuts::SwitchWorkspace => 19,
            ActionShortcuts::FindInBuffer => 20,
            ActionShortcuts::ReplaceInBuffer => 21,
            ActionShortcuts::Leader => 22,
            ActionShortcuts::OpenCommandPalette => 23,
            ActionShortcuts::FocusRegion(_) => 24,
            ActionShortcuts::MoveViewToLeftDrawer => 25,
            ActionShortcuts::MoveViewToRightDrawer => 26,
            ActionShortcuts::SplitDrawerSlot => 27,
            ActionShortcuts::UnsplitDrawerSlot => 28,
            ActionShortcuts::ToggleRightDrawer => 29,
        }
    }
    const VARIANTS: usize = 30;

    #[test]
    fn all_lists_every_variant() {
        let mut seen = [false; VARIANTS];
        for a in &ActionShortcuts::ALL {
            seen[ordinal(a)] = true;
        }
        let missing: Vec<usize> = (0..VARIANTS).filter(|i| !seen[*i]).collect();
        assert!(
            missing.is_empty(),
            "ALL misses variant ordinals {missing:?}"
        );
    }

    #[test]
    fn all_is_in_ord_order_without_duplicates() {
        for pair in ActionShortcuts::ALL.windows(2) {
            assert!(pair[0] < pair[1], "{:?} !< {:?}", pair[0], pair[1]);
        }
    }

    #[test]
    fn every_action_name_round_trips() {
        for a in ActionShortcuts::ALL {
            assert_eq!(ActionShortcuts::try_from(a.to_string()), Ok(a.clone()));
        }
    }

    /// The new layout actions keep their on-disk names: GUI keymaps write them.
    #[test]
    fn layout_action_names() {
        let names: Vec<String> = [
            ActionShortcuts::FocusRegion(Dir::Left),
            ActionShortcuts::FocusRegion(Dir::Right),
            ActionShortcuts::FocusRegion(Dir::Up),
            ActionShortcuts::FocusRegion(Dir::Down),
            ActionShortcuts::MoveViewToLeftDrawer,
            ActionShortcuts::MoveViewToRightDrawer,
            ActionShortcuts::SplitDrawerSlot,
            ActionShortcuts::UnsplitDrawerSlot,
            ActionShortcuts::ToggleRightDrawer,
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        assert_eq!(
            names,
            [
                "FocusRegionLeft",
                "FocusRegionRight",
                "FocusRegionUp",
                "FocusRegionDown",
                "MoveViewToLeftDrawer",
                "MoveViewToRightDrawer",
                "SplitDrawerSlot",
                "UnsplitDrawerSlot",
                "ToggleRightDrawer",
            ]
        );
    }

    #[test]
    fn focus_region_actions_map_to_layout_dirs() {
        assert_eq!(
            ActionShortcuts::FocusRegion(Dir::Left).focus_dir(),
            Some(Dir::Left)
        );
        assert_eq!(
            ActionShortcuts::FocusRegion(Dir::Right).focus_dir(),
            Some(Dir::Right)
        );
        assert_eq!(
            ActionShortcuts::FocusRegion(Dir::Up).focus_dir(),
            Some(Dir::Up)
        );
        assert_eq!(
            ActionShortcuts::FocusRegion(Dir::Down).focus_dir(),
            Some(Dir::Down)
        );
        assert_eq!(ActionShortcuts::FocusSidebar.focus_dir(), None);
    }

    #[test]
    fn serde_uses_the_display_name() {
        #[derive(Serialize, Deserialize, Debug, PartialEq)]
        struct W {
            a: ActionShortcuts,
        }
        let w = W {
            a: ActionShortcuts::OpenPreferences,
        };
        let s = toml::to_string(&w).unwrap();
        assert_eq!(s.trim(), "a = \"OpenSettings\"");
        assert_eq!(toml::from_str::<W>(&s).unwrap(), w);
    }

    #[test]
    fn file_operations_roundtrip() {
        assert_eq!(
            ActionShortcuts::FileOperations.to_string(),
            "FileOperations"
        );
        assert_eq!(
            ActionShortcuts::try_from("FileOperations".to_string()),
            Ok(ActionShortcuts::FileOperations)
        );
    }

    #[test]
    fn saved_search_actions_roundtrip() {
        assert_eq!(
            ActionShortcuts::ToggleQueryPanel.to_string(),
            "ToggleQueryPanel"
        );
        assert_eq!(
            ActionShortcuts::try_from("ToggleQueryPanel".to_string()),
            Ok(ActionShortcuts::ToggleQueryPanel)
        );
        // legacy alias still parses to the renamed action
        assert_eq!(
            ActionShortcuts::try_from("ToggleBacklinks".to_string()),
            Ok(ActionShortcuts::ToggleQueryPanel)
        );
        assert_eq!(
            ActionShortcuts::try_from("OpenSavedSearches".to_string()),
            Ok(ActionShortcuts::OpenSavedSearches)
        );
        assert_eq!(
            ActionShortcuts::try_from("SaveCurrentQuery".to_string()),
            Ok(ActionShortcuts::SaveCurrentQuery)
        );
    }

    #[test]
    fn open_ask_roundtrip_and_legacy_alias() {
        assert_eq!(ActionShortcuts::OpenAsk.to_string(), "OpenAsk");
        assert_eq!(
            ActionShortcuts::try_from("OpenAsk".to_string()),
            Ok(ActionShortcuts::OpenAsk)
        );
        // legacy name from the pre-workspace Ask overlay still parses
        assert_eq!(
            ActionShortcuts::try_from("OpenRagAnswer".to_string()),
            Ok(ActionShortcuts::OpenAsk)
        );
    }

    #[test]
    fn open_sort_dialog_roundtrip_and_legacy_alias() {
        assert_eq!(
            ActionShortcuts::OpenSortDialog.to_string(),
            "OpenSortDialog"
        );
        assert_eq!(
            ActionShortcuts::try_from("OpenSortDialog".to_string()),
            Ok(ActionShortcuts::OpenSortDialog)
        );
        assert_eq!(
            ActionShortcuts::try_from("CycleSortField".to_string()),
            Ok(ActionShortcuts::OpenSortDialog)
        );
        assert_eq!(
            ActionShortcuts::try_from("SortReverseOrder".to_string()),
            Ok(ActionShortcuts::OpenSortDialog)
        );
    }
}
