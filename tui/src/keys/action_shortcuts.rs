use std::fmt::Display;

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
/// screen's F-key fallback), not a keymap action.
pub const TUI_UNBOUND: [ActionShortcuts; 15] = [
    // Reachable via the rail and leader; no free chord.
    ActionShortcuts::ToggleQueryPanel,
    // `Tab` from an open find bar reaches the same state.
    ActionShortcuts::ReplaceInBuffer,
    ActionShortcuts::FocusRegion(Dir::Left),
    ActionShortcuts::FocusRegion(Dir::Right),
    ActionShortcuts::FocusRegion(Dir::Up),
    ActionShortcuts::FocusRegion(Dir::Down),
    ActionShortcuts::MoveViewToLeftDrawer,
    ActionShortcuts::MoveViewToRightDrawer,
    ActionShortcuts::SplitDrawerSlot,
    ActionShortcuts::UnsplitDrawerSlot,
    ActionShortcuts::ToggleRightDrawer,
    ActionShortcuts::SaveNote,
    ActionShortcuts::MoveViewUp,
    ActionShortcuts::MoveViewDown,
    ActionShortcuts::KeyboardHelp,
];

/// Groups an [`ActionShortcuts`] variant for display in the help modal.
/// The `Ord` order determines the section render order.
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
    pub fn category(&self) -> ShortcutCategory {
        match self {
            ActionShortcuts::Leader
            | ActionShortcuts::OpenCommandPalette
            | ActionShortcuts::ToggleSidebar
            | ActionShortcuts::OpenFileBrowser
            | ActionShortcuts::FocusSidebar
            | ActionShortcuts::FocusEditor
            | ActionShortcuts::OpenSortDialog
            | ActionShortcuts::ToggleQueryPanel
            | ActionShortcuts::OpenSavedSearches
            | ActionShortcuts::SaveCurrentQuery
            | ActionShortcuts::SwitchWorkspace
            | ActionShortcuts::FocusRegion(_)
            | ActionShortcuts::MoveViewToLeftDrawer
            | ActionShortcuts::MoveViewToRightDrawer
            | ActionShortcuts::SplitDrawerSlot
            | ActionShortcuts::UnsplitDrawerSlot
            | ActionShortcuts::ToggleRightDrawer
            | ActionShortcuts::MoveViewUp
            | ActionShortcuts::MoveViewDown => ShortcutCategory::Navigation,

            ActionShortcuts::SearchNotes
            | ActionShortcuts::OpenNote
            | ActionShortcuts::NewJournal
            | ActionShortcuts::FileOperations
            | ActionShortcuts::FollowLink
            | ActionShortcuts::QuickNote
            | ActionShortcuts::FindInBuffer
            | ActionShortcuts::ReplaceInBuffer
            | ActionShortcuts::YankRow
            | ActionShortcuts::OpenAsk
            | ActionShortcuts::SaveNote => ShortcutCategory::Notes,

            ActionShortcuts::Text(_) => ShortcutCategory::TextEditing,

            ActionShortcuts::Quit
            | ActionShortcuts::OpenPreferences
            | ActionShortcuts::KeyboardHelp => ShortcutCategory::Other,
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
            ActionShortcuts::MoveViewToLeftDrawer => "Move view to left drawer".into(),
            ActionShortcuts::MoveViewToRightDrawer => "Move view to right drawer".into(),
            ActionShortcuts::SplitDrawerSlot => "Split drawer".into(),
            ActionShortcuts::UnsplitDrawerSlot => "Unsplit drawer".into(),
            ActionShortcuts::ToggleRightDrawer => "Toggle right drawer".into(),
            ActionShortcuts::SaveNote => "Save note".into(),
            ActionShortcuts::MoveViewUp => "Move view up".into(),
            ActionShortcuts::MoveViewDown => "Move view down".into(),
            ActionShortcuts::KeyboardHelp => "Keyboard shortcuts".into(),
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
