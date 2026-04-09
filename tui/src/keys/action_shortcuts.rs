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
    CycleSortField,
    SortReverseOrder,
    // File operations
    FileOperations,
    // Editor link navigation
    FollowLink,
}

impl ActionShortcuts {
    pub fn category(&self) -> ShortcutCategory {
        match self {
            ActionShortcuts::ToggleSidebar
            | ActionShortcuts::FocusSidebar
            | ActionShortcuts::FocusEditor
            | ActionShortcuts::CycleSortField
            | ActionShortcuts::SortReverseOrder => ShortcutCategory::Navigation,

            ActionShortcuts::SearchNotes
            | ActionShortcuts::OpenNote
            | ActionShortcuts::NewJournal
            | ActionShortcuts::FileOperations
            | ActionShortcuts::FollowLink => ShortcutCategory::Notes,

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
            ActionShortcuts::FocusEditor => "Focus editor".into(),
            ActionShortcuts::FocusSidebar => "Focus sidebar".into(),
            ActionShortcuts::CycleSortField => "Cycle sort field".into(),
            ActionShortcuts::SortReverseOrder => "Reverse sort order".into(),
            ActionShortcuts::FileOperations => "File operations".into(),
            ActionShortcuts::FollowLink => "Follow link".into(),
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
            ActionShortcuts::CycleSortField => "CycleSortField".to_string(),
            ActionShortcuts::SortReverseOrder => "SortReverseOrder".to_string(),
            ActionShortcuts::FileOperations => "FileOperations".to_string(),
            ActionShortcuts::FollowLink => "FollowLink".to_string(),
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
            "CycleSortField" => ActionShortcuts::CycleSortField,
            "SortReverseOrder" => ActionShortcuts::SortReverseOrder,
            "FileOperations" => ActionShortcuts::FileOperations,
            "FollowLink" => ActionShortcuts::FollowLink,
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
            ActionShortcuts::CycleSortField.category(),
            ShortcutCategory::Navigation
        );
        assert_eq!(
            ActionShortcuts::SortReverseOrder.category(),
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
        assert_eq!(ActionShortcuts::FocusEditor.label(), "Focus editor");
        assert_eq!(ActionShortcuts::FocusSidebar.label(), "Focus sidebar");
        assert_eq!(ActionShortcuts::CycleSortField.label(), "Cycle sort field");
        assert_eq!(
            ActionShortcuts::SortReverseOrder.label(),
            "Reverse sort order"
        );
        assert_eq!(ActionShortcuts::FileOperations.label(), "File operations");
        assert_eq!(ActionShortcuts::FollowLink.label(), "Follow link");
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
