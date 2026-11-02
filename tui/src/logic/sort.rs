//! Sort field / order and the state the sort dialog edits.

use crate::settings::{SortFieldSetting, SortOrderSetting};

#[derive(Clone, PartialEq, Debug)]
pub enum SortField {
    Name,
    Title,
    /// Sort by this property key (search form). Query-backed lists put it
    /// in the query; listings order by values fetched from the index (see
    /// `entry_order`).
    Property(String),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl From<SortFieldSetting> for SortField {
    fn from(s: SortFieldSetting) -> Self {
        match s {
            SortFieldSetting::Name => Self::Name,
            SortFieldSetting::Title => Self::Title,
        }
    }
}

impl From<SortOrderSetting> for SortOrder {
    fn from(s: SortOrderSetting) -> Self {
        match s {
            SortOrderSetting::Ascending => Self::Ascending,
            SortOrderSetting::Descending => Self::Descending,
        }
    }
}

impl From<SortField> for SortFieldSetting {
    fn from(s: SortField) -> Self {
        match s {
            SortField::Name | SortField::Property(_) => Self::Name,
            SortField::Title => Self::Title,
        }
    }
}

impl From<SortOrder> for SortOrderSetting {
    fn from(s: SortOrder) -> Self {
        match s {
            SortOrder::Ascending => Self::Ascending,
            SortOrder::Descending => Self::Descending,
        }
    }
}

impl SortField {
    pub fn label(&self) -> String {
        match self {
            Self::Name => "N".to_string(),
            Self::Title => "T".to_string(),
            Self::Property(key) => key.clone(),
        }
    }

    /// Next field in the dialog's cycle. `allow_property` comes from the
    /// list's `SortableList::allows_property`.
    pub fn cycle(&self, allow_property: bool) -> Self {
        match self {
            Self::Name => Self::Title,
            Self::Title if allow_property => Self::Property(String::new()),
            Self::Title | Self::Property(_) => Self::Name,
        }
    }
}

impl SortOrder {
    pub fn label(self) -> char {
        match self {
            Self::Ascending => '↑',
            Self::Descending => '↓',
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// What the sort dialog shows and changes for one list.
#[derive(Debug, Clone, PartialEq)]
pub struct SortState {
    pub field: SortField,
    pub order: SortOrder,
    /// `Some` only for lists that can group directories (the sidebar).
    pub group_dirs: Option<bool>,
}
