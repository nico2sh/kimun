use std::fmt::Display;

use crate::logic::actions::ActionGroup;
pub use crate::logic::actions::ActionShortcuts;
use crate::logic::layout::Dir;
use crate::logic::text_action::TextAction;

/// Actions the TUI deliberately ships without a default chord. Every action in
/// [`ActionShortcuts::ALL`] is either bound by `default_keybindings` or listed
/// here (or is a `Text(..)` action: formatting lives on the leader), pinned by
/// a test in `settings`. The layout actions (ADR-0050) have no TUI meaning yet:
/// the TUI has one drawer, one slot and one group, and its shortcut tier
/// ignores them even if a user binds one. `SaveNote` is the leader's `w`
/// (save now); the TUI has no free chord for it. `KeyboardHelp` is the
/// desktop app's F1 reference; the TUI's F1 help is built in (the editor
/// screen's F-key fallback), not a keymap action. `QuerySyntax` is the
/// desktop app's F1 in FIND and its search browser; the TUI's search views
/// show their query help themselves.
pub const TUI_UNBOUND: [ActionShortcuts; 19] = [
    // Reachable via the rail and leader; no free chord.
    ActionShortcuts::ToggleQueryPanel,
    // `Tab` from an open find bar reaches the same state.
    ActionShortcuts::ReplaceInBuffer,
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

/// The TUI help modal's heading for an [`ActionShortcuts`] variant. The
/// `Ord` order determines the section render order. Not a classification of
/// its own: [`ActionShortcuts::category`] derives it from the one grouping,
/// [`ActionShortcuts::group`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShortcutCategory {
    Navigation,
    Notes,
    TextEditing,
    Other,
}

impl Display for ShortcutCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShortcutCategory::Navigation => write!(f, "Navigation"),
            ShortcutCategory::Notes => write!(f, "Notes"),
            ShortcutCategory::TextEditing => write!(f, "Text Editing"),
            ShortcutCategory::Other => write!(f, "Other"),
        }
    }
}

impl ActionShortcuts {
    /// The TUI help heading, derived from [`ActionShortcuts::group`] (the
    /// desktop app's headings): Window & panels is Navigation; Editor, Files
    /// & notes and Search & Ask are Notes, formatting aside (Text Editing);
    /// Other stays Other.
    ///
    /// The one exception is the TUI's Navigation heading, which also holds
    /// what moves you elsewhere in the app (the file browser, the sort
    /// dialog, the saved searches and saving a query, the workspace switcher,
    /// the palette and the leader) while the desktop app files those by
    /// subject. Those actions are named here so the TUI help reads as it
    /// always has; a new action needs no entry.
    pub fn category(&self) -> ShortcutCategory {
        use ActionShortcuts as A;
        match self {
            A::Text(_) => return ShortcutCategory::TextEditing,
            A::OpenFileBrowser
            | A::OpenSortDialog
            | A::OpenSavedSearches
            | A::SaveCurrentQuery
            | A::SwitchWorkspace
            | A::OpenCommandPalette
            | A::Leader => return ShortcutCategory::Navigation,
            _ => {}
        }
        match self.group() {
            ActionGroup::WindowAndPanels => ShortcutCategory::Navigation,
            ActionGroup::Editor | ActionGroup::FilesAndNotes | ActionGroup::SearchAndAsk => {
                ShortcutCategory::Notes
            }
            // `group` never answers the two non-action headings.
            ActionGroup::Other | ActionGroup::LeaderTree | ActionGroup::Unbound => {
                ShortcutCategory::Other
            }
        }
    }

    pub fn label(&self) -> String {
        match self {
            ActionShortcuts::Quit => "Quit".into(),
            ActionShortcuts::OpenPreferences => "Preferences".into(),
            ActionShortcuts::SearchNotes => "Search notes".into(),
            ActionShortcuts::OpenNote => "Open note".into(),
            ActionShortcuts::NewJournal => "New journal entry".into(),
            ActionShortcuts::ToggleSidebar => "Toggle drawer".into(),
            ActionShortcuts::OpenFileBrowser => "Open file browser".into(),
            ActionShortcuts::FocusEditor => "Focus right".into(),
            ActionShortcuts::FocusSidebar => "Focus left".into(),
            ActionShortcuts::OpenSortDialog => "Sort options".into(),
            ActionShortcuts::FileOperations => "File operations".into(),
            ActionShortcuts::FollowLink => "Follow link".into(),
            ActionShortcuts::QuickNote => "Quick note".into(),
            ActionShortcuts::YankRow => "Copy selected row".into(),
            ActionShortcuts::ToggleQueryPanel => "Toggle query drawer".into(),
            ActionShortcuts::OpenSavedSearches => "Saved searches".into(),
            ActionShortcuts::OpenAsk => "Ask".into(),
            ActionShortcuts::SaveCurrentQuery => "Save current query".into(),
            ActionShortcuts::SwitchWorkspace => "Switch workspace".into(),
            ActionShortcuts::FindInBuffer => "Find in note".into(),
            ActionShortcuts::ReplaceInBuffer => "Replace in note".into(),
            ActionShortcuts::Leader => "Leader menu".into(),
            ActionShortcuts::OpenCommandPalette => "Command palette".into(),
            ActionShortcuts::FocusRegion(Dir::Left) => "Focus region left".into(),
            ActionShortcuts::FocusRegion(Dir::Right) => "Focus region right".into(),
            ActionShortcuts::FocusRegion(Dir::Up) => "Focus region up".into(),
            ActionShortcuts::FocusRegion(Dir::Down) => "Focus region down".into(),
            ActionShortcuts::ToggleRightDrawer => "Toggle right drawer".into(),
            ActionShortcuts::SaveNote => "Save note".into(),
            ActionShortcuts::KeyboardHelp => "Keyboard shortcuts".into(),
            ActionShortcuts::QuerySyntax => "Query syntax".into(),
            ActionShortcuts::SplitSlotRight => "Split slot right".into(),
            ActionShortcuts::SplitSlotDown => "Split slot down".into(),
            ActionShortcuts::CloseSlot => "Close slot".into(),
            ActionShortcuts::MoveViewToNeighbour(Dir::Left) => "Move view left".into(),
            ActionShortcuts::MoveViewToNeighbour(Dir::Right) => "Move view right".into(),
            ActionShortcuts::MoveViewToNeighbour(Dir::Up) => "Move view up".into(),
            ActionShortcuts::MoveViewToNeighbour(Dir::Down) => "Move view down".into(),
            ActionShortcuts::ToggleZoom => "Toggle zoom".into(),
            ActionShortcuts::OpenQueryInSidebar => "Open query in sidebar".into(),
            ActionShortcuts::Text(ta) => match ta {
                TextAction::Bold => "Bold".into(),
                TextAction::Italic => "Italic".into(),
                TextAction::Link => "Insert link".into(),
                TextAction::Image => "Insert image".into(),
                TextAction::ToggleHeader => "Toggle header".into(),
                TextAction::Header(n) => format!("Header {n}"),
                TextAction::Underline => "Underline".into(),
                TextAction::Strikethrough => "Strikethrough".into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_category_order() {
        assert!(ShortcutCategory::Navigation < ShortcutCategory::Notes);
        assert!(ShortcutCategory::Notes < ShortcutCategory::TextEditing);
        assert!(ShortcutCategory::TextEditing < ShortcutCategory::Other);
    }

    #[test]
    fn shortcut_category_display() {
        assert_eq!(ShortcutCategory::Navigation.to_string(), "Navigation");
        assert_eq!(ShortcutCategory::Notes.to_string(), "Notes");
        assert_eq!(ShortcutCategory::TextEditing.to_string(), "Text Editing");
        assert_eq!(ShortcutCategory::Other.to_string(), "Other");
    }

    #[test]
    fn action_shortcuts_categories() {
        assert_eq!(
            ActionShortcuts::ToggleSidebar.category(),
            ShortcutCategory::Navigation
        );
        assert_eq!(
            ActionShortcuts::FocusSidebar.category(),
            ShortcutCategory::Navigation
        );
        assert_eq!(
            ActionShortcuts::FocusEditor.category(),
            ShortcutCategory::Navigation
        );
        assert_eq!(
            ActionShortcuts::OpenSortDialog.category(),
            ShortcutCategory::Navigation
        );
        assert_eq!(
            ActionShortcuts::ToggleQueryPanel.category(),
            ShortcutCategory::Navigation
        );
        assert_eq!(
            ActionShortcuts::OpenSavedSearches.category(),
            ShortcutCategory::Navigation
        );
        assert_eq!(
            ActionShortcuts::SaveCurrentQuery.category(),
            ShortcutCategory::Navigation
        );
        assert_eq!(
            ActionShortcuts::SwitchWorkspace.category(),
            ShortcutCategory::Navigation
        );

        assert_eq!(
            ActionShortcuts::SearchNotes.category(),
            ShortcutCategory::Notes
        );
        assert_eq!(
            ActionShortcuts::OpenNote.category(),
            ShortcutCategory::Notes
        );
        assert_eq!(
            ActionShortcuts::NewJournal.category(),
            ShortcutCategory::Notes
        );
        assert_eq!(
            ActionShortcuts::FileOperations.category(),
            ShortcutCategory::Notes
        );
        assert_eq!(
            ActionShortcuts::FollowLink.category(),
            ShortcutCategory::Notes
        );
        assert_eq!(
            ActionShortcuts::QuickNote.category(),
            ShortcutCategory::Notes
        );
        assert_eq!(
            ActionShortcuts::FindInBuffer.category(),
            ShortcutCategory::Notes
        );

        assert_eq!(
            ActionShortcuts::Text(TextAction::Bold).category(),
            ShortcutCategory::TextEditing
        );
        assert_eq!(
            ActionShortcuts::Text(TextAction::Header(2)).category(),
            ShortcutCategory::TextEditing
        );

        assert_eq!(ActionShortcuts::Quit.category(), ShortcutCategory::Other);
        assert_eq!(
            ActionShortcuts::OpenPreferences.category(),
            ShortcutCategory::Other
        );
    }

    /// The derived categories are the ones the TUI help always showed: the
    /// heading of every action that existed before the grouping was unified,
    /// frozen here (new actions need no entry).
    #[test]
    fn derived_categories_match_the_tui_help() {
        use ShortcutCategory::{Navigation as Nav, Notes, Other, TextEditing as Text};
        let expected = [
            (ActionShortcuts::Quit, Other),
            (ActionShortcuts::OpenPreferences, Other),
            (ActionShortcuts::SearchNotes, Notes),
            (ActionShortcuts::OpenNote, Notes),
            (ActionShortcuts::NewJournal, Notes),
            (ActionShortcuts::Text(TextAction::Bold), Text),
            (ActionShortcuts::Text(TextAction::Header(3)), Text),
            (ActionShortcuts::ToggleSidebar, Nav),
            (ActionShortcuts::OpenFileBrowser, Nav),
            (ActionShortcuts::FocusEditor, Nav),
            (ActionShortcuts::FocusSidebar, Nav),
            (ActionShortcuts::OpenSortDialog, Nav),
            (ActionShortcuts::FileOperations, Notes),
            (ActionShortcuts::FollowLink, Notes),
            (ActionShortcuts::QuickNote, Notes),
            (ActionShortcuts::YankRow, Notes),
            (ActionShortcuts::ToggleQueryPanel, Nav),
            (ActionShortcuts::OpenSavedSearches, Nav),
            (ActionShortcuts::SaveCurrentQuery, Nav),
            (ActionShortcuts::OpenAsk, Notes),
            (ActionShortcuts::SwitchWorkspace, Nav),
            (ActionShortcuts::FindInBuffer, Notes),
            (ActionShortcuts::ReplaceInBuffer, Notes),
            (ActionShortcuts::Leader, Nav),
            (ActionShortcuts::OpenCommandPalette, Nav),
            (ActionShortcuts::FocusRegion(Dir::Left), Nav),
            (ActionShortcuts::FocusRegion(Dir::Down), Nav),
            (ActionShortcuts::SplitSlotRight, Nav),
            (ActionShortcuts::SplitSlotDown, Nav),
            (ActionShortcuts::CloseSlot, Nav),
            (ActionShortcuts::ToggleZoom, Nav),
            (ActionShortcuts::MoveViewToNeighbour(Dir::Left), Nav),
            (ActionShortcuts::ToggleRightDrawer, Nav),
            (ActionShortcuts::SaveNote, Notes),
            (ActionShortcuts::KeyboardHelp, Other),
        ];
        for (action, category) in expected {
            assert_eq!(action.category(), category, "{action:?}");
        }
    }

    #[test]
    fn action_shortcuts_labels() {
        assert_eq!(ActionShortcuts::Quit.label(), "Quit");
        assert_eq!(ActionShortcuts::OpenPreferences.label(), "Preferences");
        assert_eq!(ActionShortcuts::SearchNotes.label(), "Search notes");
        assert_eq!(ActionShortcuts::OpenNote.label(), "Open note");
        assert_eq!(ActionShortcuts::NewJournal.label(), "New journal entry");
        assert_eq!(ActionShortcuts::ToggleSidebar.label(), "Toggle drawer");
        assert_eq!(
            ActionShortcuts::OpenFileBrowser.label(),
            "Open file browser"
        );
        assert_eq!(ActionShortcuts::FocusEditor.label(), "Focus right");
        assert_eq!(ActionShortcuts::FocusSidebar.label(), "Focus left");
        assert_eq!(ActionShortcuts::OpenSortDialog.label(), "Sort options");
        assert_eq!(ActionShortcuts::FileOperations.label(), "File operations");
        assert_eq!(ActionShortcuts::FollowLink.label(), "Follow link");
        assert_eq!(ActionShortcuts::QuickNote.label(), "Quick note");
        assert_eq!(
            ActionShortcuts::ToggleQueryPanel.label(),
            "Toggle query drawer"
        );
        assert_eq!(ActionShortcuts::OpenSavedSearches.label(), "Saved searches");
        assert_eq!(
            ActionShortcuts::SaveCurrentQuery.label(),
            "Save current query"
        );
        assert_eq!(ActionShortcuts::SwitchWorkspace.label(), "Switch workspace");
        assert_eq!(ActionShortcuts::FindInBuffer.label(), "Find in note");
        assert_eq!(ActionShortcuts::Text(TextAction::Bold).label(), "Bold");
        assert_eq!(ActionShortcuts::Text(TextAction::Italic).label(), "Italic");
        assert_eq!(
            ActionShortcuts::Text(TextAction::Link).label(),
            "Insert link"
        );
        assert_eq!(
            ActionShortcuts::Text(TextAction::Image).label(),
            "Insert image"
        );
        assert_eq!(
            ActionShortcuts::Text(TextAction::ToggleHeader).label(),
            "Toggle header"
        );
        assert_eq!(
            ActionShortcuts::Text(TextAction::Header(1)).label(),
            "Header 1"
        );
        assert_eq!(
            ActionShortcuts::Text(TextAction::Header(2)).label(),
            "Header 2"
        );
        assert_eq!(
            ActionShortcuts::Text(TextAction::Underline).label(),
            "Underline"
        );
        assert_eq!(
            ActionShortcuts::Text(TextAction::Strikethrough).label(),
            "Strikethrough"
        );
    }

    #[test]
    fn open_file_browser_roundtrip() {
        assert_eq!(
            ActionShortcuts::OpenFileBrowser.to_string(),
            "OpenFileBrowser"
        );
        assert_eq!(
            ActionShortcuts::try_from("OpenFileBrowser".to_string()),
            Ok(ActionShortcuts::OpenFileBrowser)
        );
        assert_eq!(
            ActionShortcuts::OpenFileBrowser.category(),
            ShortcutCategory::Navigation
        );
    }
}
