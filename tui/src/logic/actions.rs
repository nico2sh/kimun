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
    // Layout (ADR-0050). Geometric focus between visible regions, view
    // placement and slot splitting. The TUI binds none of these by default.
    /// Move focus to the neighbouring region in that direction (no wrap;
    /// Up/Down only between stacked slots or groups). On disk it is one name
    /// per direction, `FocusRegionLeft` … `FocusRegionDown`, the way `Text`
    /// spells each `TextAction`; `Dir`'s order keeps the sort unchanged.
    FocusRegion(Dir),
    /// Show or hide the right drawer.
    ToggleRightDrawer,
    /// Save the open note now (the leader's `w` in the TUI, which binds no
    /// chord to it).
    SaveNote,
    /// The keyboard reference: every chord and leader sequence in effect
    /// (F1 in the desktop app). The TUI binds no chord to it: its F1 help is
    /// built in.
    KeyboardHelp,
    /// The search query syntax: where a query is typed (FIND, the desktop
    /// app's search browser; F1 there in the desktop app, which keeps F1 the
    /// keyboard reference everywhere else). The TUI binds no chord to it: its
    /// query help is built into its search views.
    QuerySyntax,
    /// Split the focused slot side by side: a new slot opens right of
    /// it (ADR-0052).
    SplitSlotRight,
    /// Split the focused slot into two stacked slots: a new slot
    /// opens below it (ADR-0052).
    SplitSlotDown,
    /// Close the focused slot; its views
    /// join the neighbouring slot taking over its area (ADR-0052).
    CloseSlot,
    /// Move the focused view to the nearest slot in that direction
    /// (ADR-0052 geometry). On disk it is one name per direction,
    /// `MoveViewToNeighbourLeft` … `MoveViewToNeighbourDown`, as
    /// `FocusRegion` spells each `Dir`.
    MoveViewToNeighbour(Dir),
    /// Zoom the focused view (or editor group) over everything right of the
    /// rail, or leave zoom (ADR-0053). Session only; Esc never leaves it.
    ToggleZoom,
    /// The desktop app's search browser: close it and show its query in the
    /// FIND view (ADR-0054). The TUI binds no chord to it.
    OpenQueryInSidebar,
}

/// Where an action is listed in a keyboard reference, next to the action
/// definitions so a new action cannot go ungrouped: [`ActionShortcuts::group`]
/// has no wildcard. This is the **one** classification of the actions: the
/// desktop app's F1 reference shows these headings, and the TUI's help
/// derives its own coarser categories from them
/// (`crate::keys::action_shortcuts::ShortcutCategory`). The order is the
/// reference's heading order.
///
/// [`ActionGroup::LeaderTree`] and [`ActionGroup::Unbound`] are headings of
/// the reference that no action is classified under: the leader tree's
/// sequences, and the actions a `[gui.keys]` override left without a chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActionGroup {
    /// Panels, focus and the arrangement of views (drawers in the TUI,
    /// slots in the desktop app).
    WindowAndPanels,
    /// What acts on the note being edited.
    Editor,
    /// Opening, creating and managing notes and their lists.
    FilesAndNotes,
    /// Searching, saved queries and Ask.
    SearchAndAsk,
    /// The leader tree's sequences (keys after the gateway). No action.
    LeaderTree,
    /// The app itself: preferences, workspaces, the palette, quitting.
    Other,
    /// Actions the frontend could bind but has no chord for now (a
    /// `[gui.keys]` override unbound them). No action is classified here.
    Unbound,
}

impl ActionGroup {
    /// Every heading, in display order.
    pub const ALL: [ActionGroup; 7] = [
        ActionGroup::WindowAndPanels,
        ActionGroup::Editor,
        ActionGroup::FilesAndNotes,
        ActionGroup::SearchAndAsk,
        ActionGroup::LeaderTree,
        ActionGroup::Other,
        ActionGroup::Unbound,
    ];

    /// The heading's text.
    pub fn title(self) -> &'static str {
        match self {
            ActionGroup::WindowAndPanels => "Window & panels",
            ActionGroup::Editor => "Editor",
            ActionGroup::FilesAndNotes => "Files & notes",
            ActionGroup::SearchAndAsk => "Search & Ask",
            ActionGroup::LeaderTree => "Leader tree",
            ActionGroup::Other => "Other",
            ActionGroup::Unbound => "Unbound",
        }
    }
}

impl ActionShortcuts {
    /// Every action, in `Ord` order. `Text` is listed once per fixed
    /// [`TextAction`] plus header levels 1 to 6.
    pub const ALL: [ActionShortcuts; 53] = [
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
        ActionShortcuts::ToggleRightDrawer,
        ActionShortcuts::SaveNote,
        ActionShortcuts::KeyboardHelp,
        ActionShortcuts::QuerySyntax,
        ActionShortcuts::SplitSlotRight,
        ActionShortcuts::SplitSlotDown,
        ActionShortcuts::CloseSlot,
        ActionShortcuts::MoveViewToNeighbour(Dir::Left),
        ActionShortcuts::MoveViewToNeighbour(Dir::Right),
        ActionShortcuts::MoveViewToNeighbour(Dir::Up),
        ActionShortcuts::MoveViewToNeighbour(Dir::Down),
        ActionShortcuts::ToggleZoom,
        ActionShortcuts::OpenQueryInSidebar,
    ];

    /// The heading `self` is listed under in a keyboard reference: the one
    /// place an action is classified. Never [`ActionGroup::LeaderTree`] or
    /// [`ActionGroup::Unbound`].
    pub fn group(&self) -> ActionGroup {
        use ActionShortcuts as A;
        match self {
            A::ToggleSidebar
            | A::ToggleRightDrawer
            | A::ToggleQueryPanel
            | A::FocusEditor
            | A::FocusSidebar
            | A::FocusRegion(_)
            | A::MoveViewToNeighbour(_)
            | A::SplitSlotRight
            | A::SplitSlotDown
            | A::CloseSlot
            | A::ToggleZoom => ActionGroup::WindowAndPanels,
            A::FindInBuffer | A::ReplaceInBuffer | A::FollowLink | A::SaveNote | A::Text(_) => {
                ActionGroup::Editor
            }
            A::OpenNote
            | A::NewJournal
            | A::QuickNote
            | A::FileOperations
            | A::OpenFileBrowser
            | A::OpenSortDialog
            | A::YankRow => ActionGroup::FilesAndNotes,
            A::SearchNotes
            | A::OpenSavedSearches
            | A::SaveCurrentQuery
            | A::OpenAsk
            | A::QuerySyntax
            | A::OpenQueryInSidebar => ActionGroup::SearchAndAsk,
            A::Quit
            | A::OpenPreferences
            | A::SwitchWorkspace
            | A::OpenCommandPalette
            | A::Leader
            | A::KeyboardHelp => ActionGroup::Other,
        }
    }

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
            ActionShortcuts::ToggleRightDrawer => "ToggleRightDrawer".to_string(),
            ActionShortcuts::SaveNote => "SaveNote".to_string(),
            ActionShortcuts::KeyboardHelp => "KeyboardHelp".to_string(),
            ActionShortcuts::QuerySyntax => "QuerySyntax".to_string(),
            ActionShortcuts::SplitSlotRight => "SplitSlotRight".to_string(),
            ActionShortcuts::SplitSlotDown => "SplitSlotDown".to_string(),
            ActionShortcuts::CloseSlot => "CloseSlot".to_string(),
            ActionShortcuts::MoveViewToNeighbour(Dir::Left) => {
                "MoveViewToNeighbourLeft".to_string()
            }
            ActionShortcuts::MoveViewToNeighbour(Dir::Right) => {
                "MoveViewToNeighbourRight".to_string()
            }
            ActionShortcuts::MoveViewToNeighbour(Dir::Up) => "MoveViewToNeighbourUp".to_string(),
            ActionShortcuts::MoveViewToNeighbour(Dir::Down) => {
                "MoveViewToNeighbourDown".to_string()
            }
            ActionShortcuts::ToggleZoom => "ToggleZoom".to_string(),
            ActionShortcuts::OpenQueryInSidebar => "OpenQueryInSidebar".to_string(),
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
            "ToggleRightDrawer" => ActionShortcuts::ToggleRightDrawer,
            "SaveNote" => ActionShortcuts::SaveNote,
            "KeyboardHelp" => ActionShortcuts::KeyboardHelp,
            "QuerySyntax" => ActionShortcuts::QuerySyntax,
            "SplitSlotRight" => ActionShortcuts::SplitSlotRight,
            "SplitSlotDown" => ActionShortcuts::SplitSlotDown,
            "CloseSlot" => ActionShortcuts::CloseSlot,
            "MoveViewToNeighbourLeft" => ActionShortcuts::MoveViewToNeighbour(Dir::Left),
            "MoveViewToNeighbourRight" => ActionShortcuts::MoveViewToNeighbour(Dir::Right),
            "MoveViewToNeighbourUp" => ActionShortcuts::MoveViewToNeighbour(Dir::Up),
            "MoveViewToNeighbourDown" => ActionShortcuts::MoveViewToNeighbour(Dir::Down),
            "ToggleZoom" => ActionShortcuts::ToggleZoom,
            "OpenQueryInSidebar" => ActionShortcuts::OpenQueryInSidebar,
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
            ActionShortcuts::ToggleRightDrawer => 25,
            ActionShortcuts::SaveNote => 26,
            ActionShortcuts::KeyboardHelp => 27,
            ActionShortcuts::QuerySyntax => 28,
            ActionShortcuts::SplitSlotRight => 29,
            ActionShortcuts::SplitSlotDown => 30,
            ActionShortcuts::CloseSlot => 31,
            ActionShortcuts::MoveViewToNeighbour(_) => 32,
            ActionShortcuts::ToggleZoom => 33,
            ActionShortcuts::OpenQueryInSidebar => 34,
        }
    }
    const VARIANTS: usize = 35;

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
            ActionShortcuts::ToggleRightDrawer,
            ActionShortcuts::SplitSlotRight,
            ActionShortcuts::SplitSlotDown,
            ActionShortcuts::CloseSlot,
            ActionShortcuts::MoveViewToNeighbour(Dir::Left),
            ActionShortcuts::MoveViewToNeighbour(Dir::Right),
            ActionShortcuts::MoveViewToNeighbour(Dir::Up),
            ActionShortcuts::MoveViewToNeighbour(Dir::Down),
            ActionShortcuts::ToggleZoom,
            ActionShortcuts::OpenQueryInSidebar,
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
                "ToggleRightDrawer",
                "SplitSlotRight",
                "SplitSlotDown",
                "CloseSlot",
                "MoveViewToNeighbourLeft",
                "MoveViewToNeighbourRight",
                "MoveViewToNeighbourUp",
                "MoveViewToNeighbourDown",
                "ToggleZoom",
                "OpenQueryInSidebar",
            ]
        );
    }

    /// `SaveNote`'s on-disk name is permanent: GUI keymaps write it.
    #[test]
    fn save_note_name() {
        assert_eq!(ActionShortcuts::SaveNote.to_string(), "SaveNote");
        assert_eq!(
            ActionShortcuts::try_from("SaveNote".to_string()),
            Ok(ActionShortcuts::SaveNote)
        );
    }

    /// `KeyboardHelp`'s on-disk name is permanent (`[gui.keys]` writes it),
    /// and it comes after every older action, so the serialized keymap's
    /// order is unchanged.
    #[test]
    fn keyboard_help_name() {
        assert_eq!(ActionShortcuts::KeyboardHelp.to_string(), "KeyboardHelp");
        assert_eq!(
            ActionShortcuts::try_from("KeyboardHelp".to_string()),
            Ok(ActionShortcuts::KeyboardHelp)
        );
        assert_eq!(
            ActionShortcuts::ALL[query_syntax_at() - 1],
            ActionShortcuts::KeyboardHelp
        );
    }

    /// Where `QuerySyntax` sits in [`ActionShortcuts::ALL`]: the last action
    /// before the layout-tree commands (ADR-0052), which came after it.
    fn query_syntax_at() -> usize {
        ActionShortcuts::ALL
            .iter()
            .position(|a| *a == ActionShortcuts::QuerySyntax)
            .unwrap()
    }

    /// `QuerySyntax`'s on-disk name is permanent (`[gui.keys]` writes it),
    /// and only the layout-tree commands (ADR-0052), `ToggleZoom` (ADR-0053)
    /// and `OpenQueryInSidebar` (ADR-0054), added after it, follow it, so the serialized keymap's order is unchanged.
    #[test]
    fn query_syntax_name() {
        assert_eq!(ActionShortcuts::QuerySyntax.to_string(), "QuerySyntax");
        assert_eq!(
            ActionShortcuts::try_from("QuerySyntax".to_string()),
            Ok(ActionShortcuts::QuerySyntax)
        );
        assert_eq!(
            ActionShortcuts::ALL[query_syntax_at() + 1..],
            [
                ActionShortcuts::SplitSlotRight,
                ActionShortcuts::SplitSlotDown,
                ActionShortcuts::CloseSlot,
                ActionShortcuts::MoveViewToNeighbour(Dir::Left),
                ActionShortcuts::MoveViewToNeighbour(Dir::Right),
                ActionShortcuts::MoveViewToNeighbour(Dir::Up),
                ActionShortcuts::MoveViewToNeighbour(Dir::Down),
                ActionShortcuts::ToggleZoom,
                ActionShortcuts::OpenQueryInSidebar,
            ]
        );
        assert_eq!(
            ActionShortcuts::QuerySyntax.group(),
            ActionGroup::SearchAndAsk
        );
    }

    /// Every action heading holds at least one action, the two non-action
    /// headings none, and the headings keep the reference's order.
    #[test]
    fn every_group_holds_an_action() {
        for group in ActionGroup::ALL {
            let holds = ActionShortcuts::ALL.iter().any(|a| a.group() == group);
            let action_heading = !matches!(group, ActionGroup::LeaderTree | ActionGroup::Unbound);
            assert_eq!(holds, action_heading, "{group:?}");
        }
        for pair in ActionGroup::ALL.windows(2) {
            assert!(pair[0] < pair[1], "{:?} !< {:?}", pair[0], pair[1]);
        }
        assert!(ActionGroup::WindowAndPanels < ActionGroup::Editor);
        assert!(ActionGroup::SearchAndAsk < ActionGroup::Other);
        assert_eq!(ActionShortcuts::KeyboardHelp.group(), ActionGroup::Other);
        assert_eq!(ActionShortcuts::FindInBuffer.group(), ActionGroup::Editor);
    }

    #[test]
    fn every_heading_has_a_title() {
        let titles: Vec<&str> = ActionGroup::ALL.iter().map(|g| g.title()).collect();
        assert_eq!(
            titles,
            [
                "Window & panels",
                "Editor",
                "Files & notes",
                "Search & Ask",
                "Leader tree",
                "Other",
                "Unbound"
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
