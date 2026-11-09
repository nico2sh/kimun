//! The TUI-only seams a `SearchList` varies across (see CONTEXT.md: Search
//! row, Suggestion source). The toolkit-free ones (`ListRow`, `RowSource`,
//! `Emit`, `Filter`, …) live in [`crate::logic::list`].

use ratatui::widgets::ListItem;

pub use crate::logic::list::ListRow;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// The TUI's renderer half of a list row: the toolkit-free [`ListRow`]
/// (filtering, yank) plus how the row draws in the terminal.
pub trait SearchRow: ListRow {
    /// Collapsed one-or-few-line rendering. `selected` lets a row self-style.
    fn to_list_item(&self, theme: &Theme, icons: &Icons, selected: bool) -> ListItem<'static>;

    /// Terminal rows this collapsed item occupies (mouse hit-testing / scroll).
    fn visual_height(&self) -> u16 {
        1
    }
}

pub use crate::logic::autocomplete::source::{SuggestionItem, SuggestionSource, VaultSuggestions};
