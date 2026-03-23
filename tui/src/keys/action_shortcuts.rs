use std::fmt::Display;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ActionShortcuts {
    Quit,
    OpenSettings,
    ToggleNoteBrowser,
    SearchNotes,
    OpenNote,
    NewJournal,
    TogglePreview,
    Text(TextAction),
    // TUI navigation / file list
    ToggleSidebar,
    FocusEditor,
    FocusSidebar,
    SortByName,
    SortByTitle,
    SortReverseOrder,
    // File operations
    DeleteEntry,
    RenameEntry,
    MoveEntry,
}

impl Display for ActionShortcuts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let action = match self {
            ActionShortcuts::Quit => "Quit".to_string(),
            ActionShortcuts::OpenSettings => "OpenSettings".to_string(),
            ActionShortcuts::ToggleNoteBrowser => "ToggleNoteBrowser".to_string(),
            ActionShortcuts::SearchNotes => "SearchNotes".to_string(),
            ActionShortcuts::OpenNote => "OpenNote".to_string(),
            ActionShortcuts::NewJournal => "NewJournal".to_string(),
            ActionShortcuts::TogglePreview => "TogglePreview".to_string(),
            ActionShortcuts::Text(text_action) => format!("TextEditor-{}", text_action),
            ActionShortcuts::ToggleSidebar => "ToggleSidebar".to_string(),
            ActionShortcuts::FocusEditor => "FocusEditor".to_string(),
            ActionShortcuts::FocusSidebar => "FocusSidebar".to_string(),
            ActionShortcuts::SortByName => "SortByName".to_string(),
            ActionShortcuts::SortByTitle => "SortByTitle".to_string(),
            ActionShortcuts::SortReverseOrder => "SortReverseOrder".to_string(),
            ActionShortcuts::DeleteEntry => "DeleteEntry".to_string(),
            ActionShortcuts::RenameEntry => "RenameEntry".to_string(),
            ActionShortcuts::MoveEntry => "MoveEntry".to_string(),
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
            "ToggleNoteBrowser" => ActionShortcuts::ToggleNoteBrowser,
            "SearchNotes" => ActionShortcuts::SearchNotes,
            "OpenNote" => ActionShortcuts::OpenNote,
            "NewJournal" => ActionShortcuts::NewJournal,
            "TogglePreview" => ActionShortcuts::TogglePreview,
            "ToggleSidebar" => ActionShortcuts::ToggleSidebar,
            "FocusEditor" => ActionShortcuts::FocusEditor,
            "FocusSidebar" => ActionShortcuts::FocusSidebar,
            "SortByName" => ActionShortcuts::SortByName,
            "SortByTitle" => ActionShortcuts::SortByTitle,
            "SortReverseOrder" => ActionShortcuts::SortReverseOrder,
            "DeleteEntry" => ActionShortcuts::DeleteEntry,
            "RenameEntry" => ActionShortcuts::RenameEntry,
            "MoveEntry" => ActionShortcuts::MoveEntry,
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
    fn delete_entry_roundtrip() {
        assert_eq!(ActionShortcuts::DeleteEntry.to_string(), "DeleteEntry");
        assert_eq!(
            ActionShortcuts::try_from("DeleteEntry".to_string()),
            Ok(ActionShortcuts::DeleteEntry)
        );
    }

    #[test]
    fn rename_entry_roundtrip() {
        assert_eq!(ActionShortcuts::RenameEntry.to_string(), "RenameEntry");
        assert_eq!(
            ActionShortcuts::try_from("RenameEntry".to_string()),
            Ok(ActionShortcuts::RenameEntry)
        );
    }

    #[test]
    fn move_entry_roundtrip() {
        assert_eq!(ActionShortcuts::MoveEntry.to_string(), "MoveEntry");
        assert_eq!(
            ActionShortcuts::try_from("MoveEntry".to_string()),
            Ok(ActionShortcuts::MoveEntry)
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
