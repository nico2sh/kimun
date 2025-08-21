use std::fmt::Display;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ActionShortcuts {
    OpenSettings,
    ToggleNoteBrowser,
    SearchNotes,
    OpenNote,
    NewJournal,
    TogglePreview,
    Text(TextAction),
}

impl Display for ActionShortcuts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let action = match self {
            ActionShortcuts::OpenSettings => "OpenSettings".to_string(),
            ActionShortcuts::ToggleNoteBrowser => "ToggleNoteBrowser".to_string(),
            ActionShortcuts::SearchNotes => "SearchNotes".to_string(),
            ActionShortcuts::OpenNote => "OpenNote".to_string(),
            ActionShortcuts::NewJournal => "NewJournal".to_string(),
            ActionShortcuts::TogglePreview => "TogglePreview".to_string(),
            ActionShortcuts::Text(text_action) => {
                format!("TextEditor-{}", text_action)
            }
        };
        write!(f, "{}", action)
    }
}

impl TryFrom<String> for ActionShortcuts {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let action = match value.as_str() {
            "OpenSettings" => ActionShortcuts::OpenSettings,
            "ToggleNoteBrowser" => ActionShortcuts::ToggleNoteBrowser,
            "SearchNotes" => ActionShortcuts::SearchNotes,
            "OpenNote" => ActionShortcuts::OpenNote,
            "NewJournal" => ActionShortcuts::NewJournal,
            "TogglePreview" => ActionShortcuts::TogglePreview,
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
