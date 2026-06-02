use std::fmt::Display;

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ActionShortcuts {
    Quit,
    OpenSettings,
    SearchNotes,
    OpenNote,
    NewJournal,
    TogglePreview,
    Text(TextAction),
    // TUI navigation / file list
    ToggleSidebar,
    FocusEditor,
    FocusSidebar,
    OpenSortDialog,
    // File operations
    FileOperations,
    // Editor link navigation
    FollowLink,
    // Quick capture
    QuickNote,
    // Query panel
    ToggleQueryPanel,
    OpenSavedSearches,
    SaveCurrentQuery,
    // Workspace
    SwitchWorkspace,
    // In-buffer find (Ctrl+F by default; reopens / advances to next match if
    // already open).
    FindInBuffer,
}

impl ActionShortcuts {
    pub fn category(&self) -> ShortcutCategory {
        match self {
            ActionShortcuts::ToggleSidebar
            | ActionShortcuts::FocusSidebar
            | ActionShortcuts::FocusEditor
            | ActionShortcuts::OpenSortDialog
            | ActionShortcuts::ToggleQueryPanel
            | ActionShortcuts::OpenSavedSearches
            | ActionShortcuts::SaveCurrentQuery
            | ActionShortcuts::SwitchWorkspace => ShortcutCategory::Navigation,

            ActionShortcuts::SearchNotes
            | ActionShortcuts::OpenNote
            | ActionShortcuts::NewJournal
            | ActionShortcuts::FileOperations
            | ActionShortcuts::FollowLink
            | ActionShortcuts::QuickNote
            | ActionShortcuts::FindInBuffer => ShortcutCategory::Notes,

            ActionShortcuts::Text(_) => ShortcutCategory::TextEditing,

            ActionShortcuts::Quit
            | ActionShortcuts::OpenSettings
            | ActionShortcuts::TogglePreview => ShortcutCategory::Other,
        }
    }

    pub fn label(&self) -> String {
        match self {
            ActionShortcuts::Quit => "Quit".into(),
            ActionShortcuts::OpenSettings => "Settings".into(),
            ActionShortcuts::SearchNotes => "Search notes".into(),
            ActionShortcuts::OpenNote => "Open note".into(),
            ActionShortcuts::NewJournal => "New journal entry".into(),
            ActionShortcuts::TogglePreview => "Toggle preview".into(),
            ActionShortcuts::ToggleSidebar => "Toggle sidebar".into(),
            ActionShortcuts::FocusEditor => "Focus right".into(),
            ActionShortcuts::FocusSidebar => "Focus left".into(),
            ActionShortcuts::OpenSortDialog => "Sort options".into(),
            ActionShortcuts::FileOperations => "File operations".into(),
            ActionShortcuts::FollowLink => "Follow link".into(),
            ActionShortcuts::QuickNote => "Quick note".into(),
            ActionShortcuts::ToggleQueryPanel => "Toggle query panel".into(),
            ActionShortcuts::OpenSavedSearches => "Saved searches".into(),
            ActionShortcuts::SaveCurrentQuery => "Save current query".into(),
            ActionShortcuts::SwitchWorkspace => "Switch workspace".into(),
            ActionShortcuts::FindInBuffer => "Find in note".into(),
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

impl Display for ActionShortcuts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let action = match self {
            ActionShortcuts::Quit => "Quit".to_string(),
            ActionShortcuts::OpenSettings => "OpenSettings".to_string(),
            ActionShortcuts::SearchNotes => "SearchNotes".to_string(),
            ActionShortcuts::OpenNote => "OpenNote".to_string(),
            ActionShortcuts::NewJournal => "NewJournal".to_string(),
            ActionShortcuts::TogglePreview => "TogglePreview".to_string(),
            ActionShortcuts::Text(text_action) => format!("TextEditor-{}", text_action),
            ActionShortcuts::ToggleSidebar => "ToggleSidebar".to_string(),
            ActionShortcuts::FocusEditor => "FocusEditor".to_string(),
            ActionShortcuts::FocusSidebar => "FocusSidebar".to_string(),
            ActionShortcuts::OpenSortDialog => "OpenSortDialog".to_string(),
            ActionShortcuts::FileOperations => "FileOperations".to_string(),
            ActionShortcuts::FollowLink => "FollowLink".to_string(),
            ActionShortcuts::QuickNote => "QuickNote".to_string(),
            ActionShortcuts::ToggleQueryPanel => "ToggleQueryPanel".to_string(),
            ActionShortcuts::OpenSavedSearches => "OpenSavedSearches".to_string(),
            ActionShortcuts::SaveCurrentQuery => "SaveCurrentQuery".to_string(),
            ActionShortcuts::SwitchWorkspace => "SwitchWorkspace".to_string(),
            ActionShortcuts::FindInBuffer => "FindInBuffer".to_string(),
        };
        write!(f, "{}", action)
    }
}

impl TryFrom<String> for ActionShortcuts {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let action = match value.as_str() {
            "Quit" => ActionShortcuts::Quit,
            "OpenSettings" => ActionShortcuts::OpenSettings,
            "SearchNotes" => ActionShortcuts::SearchNotes,
            "OpenNote" => ActionShortcuts::OpenNote,
            "NewJournal" => ActionShortcuts::NewJournal,
            "TogglePreview" => ActionShortcuts::TogglePreview,
            "ToggleSidebar" => ActionShortcuts::ToggleSidebar,
            "FocusEditor" => ActionShortcuts::FocusEditor,
            "FocusSidebar" => ActionShortcuts::FocusSidebar,
            "OpenSortDialog" => ActionShortcuts::OpenSortDialog,
            "CycleSortField" => ActionShortcuts::OpenSortDialog,
            "SortReverseOrder" => ActionShortcuts::OpenSortDialog,
            "FileOperations" => ActionShortcuts::FileOperations,
            "FollowLink" => ActionShortcuts::FollowLink,
            "QuickNote" => ActionShortcuts::QuickNote,
            "ToggleQueryPanel" => ActionShortcuts::ToggleQueryPanel,
            "ToggleBacklinks" => ActionShortcuts::ToggleQueryPanel,
            "OpenSavedSearches" => ActionShortcuts::OpenSavedSearches,
            "SaveCurrentQuery" => ActionShortcuts::SaveCurrentQuery,
            "SwitchWorkspace" => ActionShortcuts::SwitchWorkspace,
            "FindInBuffer" => ActionShortcuts::FindInBuffer,
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
            ActionShortcuts::OpenSettings.category(),
            ShortcutCategory::Other
        );
        assert_eq!(
            ActionShortcuts::TogglePreview.category(),
            ShortcutCategory::Other
        );
    }

    #[test]
    fn action_shortcuts_labels() {
        assert_eq!(ActionShortcuts::Quit.label(), "Quit");
        assert_eq!(ActionShortcuts::OpenSettings.label(), "Settings");
        assert_eq!(ActionShortcuts::SearchNotes.label(), "Search notes");
        assert_eq!(ActionShortcuts::OpenNote.label(), "Open note");
        assert_eq!(ActionShortcuts::NewJournal.label(), "New journal entry");
        assert_eq!(ActionShortcuts::TogglePreview.label(), "Toggle preview");
        assert_eq!(ActionShortcuts::ToggleSidebar.label(), "Toggle sidebar");
        assert_eq!(ActionShortcuts::FocusEditor.label(), "Focus right");
        assert_eq!(ActionShortcuts::FocusSidebar.label(), "Focus left");
        assert_eq!(ActionShortcuts::OpenSortDialog.label(), "Sort options");
        assert_eq!(ActionShortcuts::FileOperations.label(), "File operations");
        assert_eq!(ActionShortcuts::FollowLink.label(), "Follow link");
        assert_eq!(ActionShortcuts::QuickNote.label(), "Quick note");
        assert_eq!(
            ActionShortcuts::ToggleQueryPanel.label(),
            "Toggle query panel"
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
    fn open_sort_dialog_roundtrip_and_legacy_alias() {
        assert_eq!(ActionShortcuts::OpenSortDialog.to_string(), "OpenSortDialog");
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TextAction {
    Bold,
    Italic,
    Link,
    Image,
    ToggleHeader,
    Header(u8),
    Underline,
    Strikethrough,
}

impl Display for TextAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            TextAction::Bold => "Bold".to_string(),
            TextAction::Italic => "Italic".to_string(),
            TextAction::Link => "Link".to_string(),
            TextAction::Image => "Image".to_string(),
            TextAction::ToggleHeader => "ToggleHeader".to_string(),
            TextAction::Header(level) => format!("Header{}", level),
            TextAction::Underline => "Underline".to_string(),
            TextAction::Strikethrough => "Strikethrough".to_string(),
        };
        write!(f, "{}", name)
    }
}

impl TryFrom<String> for TextAction {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let action = match value.as_str() {
            "Bold" => TextAction::Bold,
            "Italic" => TextAction::Italic,
            "Link" => TextAction::Link,
            "Image" => TextAction::Image,
            "ToggleHeader" => TextAction::ToggleHeader,
            "Underline" => TextAction::Underline,
            "Strikethrough" => TextAction::Strikethrough,
            _ => {
                if let Some(level) = value.strip_prefix("Header") {
                    match level.parse::<u8>() {
                        Ok(lvl) => TextAction::Header(lvl),
                        Err(e) => return Err(format!("Error parsing header level: {}", e)),
                    }
                } else {
                    return Err(format!("Error, not valid Text Action: {}", value));
                }
            }
        };
        Ok(action)
    }
}
