//! The TUI-only seams a `SearchList` varies across (see CONTEXT.md: Search
//! row, Suggestion source). The toolkit-free ones (`ListRow`, `RowSource`,
//! `Emit`, `Filter`, …) live in [`crate::logic::list`].

use async_trait::async_trait;
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

/// One autocomplete candidate: the inserted/display text plus an optional
/// secondary line shown muted in the popup (a note path, a tag usage count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionItem {
    pub display: String,
    pub secondary: Option<String>,
}

impl SuggestionItem {
    pub fn plain(display: impl Into<String>) -> Self {
        Self {
            display: display.into(),
            secondary: None,
        }
    }
}

/// Autocomplete candidates for the query input, kept separate from the vault
/// so the autocomplete host is testable in isolation.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait SuggestionSource: Send + Sync + 'static {
    async fn notes_by_prefix(&self, prefix: &str, limit: usize) -> Vec<SuggestionItem>;
    async fn tags_by_prefix(&self, prefix: &str, limit: usize) -> Vec<SuggestionItem>;

    /// Saved searches whose name matches `prefix` (case-insensitive). Each
    /// item's `display` is the name and `secondary` the stored query — the
    /// popup preview AND the text inserted on accept.
    /// Defaults to empty so non-search-box suggestion sources opt out.
    async fn saved_searches_by_prefix(&self, _prefix: &str, _limit: usize) -> Vec<SuggestionItem> {
        Vec::new()
    }
}

/// Production adapter over the vault. Formats the secondary line (note path,
/// tag usage count) so the popup looks exactly as before.
pub struct VaultSuggestions {
    pub vault: std::sync::Arc<kimun_core::NoteVault>,
}

#[async_trait]
impl SuggestionSource for VaultSuggestions {
    async fn notes_by_prefix(&self, prefix: &str, limit: usize) -> Vec<SuggestionItem> {
        self.vault
            .suggest_notes_by_prefix(prefix, limit)
            .await
            .map(|v| {
                v.into_iter()
                    .map(|n| SuggestionItem {
                        display: n.name,
                        secondary: Some(n.path.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    async fn tags_by_prefix(&self, prefix: &str, limit: usize) -> Vec<SuggestionItem> {
        self.vault
            .suggest_tags_by_prefix(prefix, limit)
            .await
            .map(|v| {
                v.into_iter()
                    .map(|t| SuggestionItem {
                        display: t.label,
                        secondary: Some(format!("{}×", t.usage_count)),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    async fn saved_searches_by_prefix(&self, prefix: &str, limit: usize) -> Vec<SuggestionItem> {
        // Prefix matching + casing live in core (`NoteVault`), like the
        // notes/tags suggestion sources. Here we only adapt to `SuggestionItem`:
        // the name is the popup row, the stored query is the muted preview AND
        // the text inserted on accept.
        self.vault
            .suggest_saved_searches_by_prefix(prefix, limit)
            .await
            .map(|v| {
                v.into_iter()
                    .map(|s| SuggestionItem {
                        display: s.name,
                        secondary: Some(s.query),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod suggestion_tests {
    use super::*;
    struct Mem {
        notes: Vec<SuggestionItem>,
        tags: Vec<SuggestionItem>,
    }
    #[async_trait]
    impl SuggestionSource for Mem {
        async fn notes_by_prefix(&self, p: &str, _n: usize) -> Vec<SuggestionItem> {
            self.notes
                .iter()
                .filter(|x| x.display.starts_with(p))
                .cloned()
                .collect()
        }
        async fn tags_by_prefix(&self, p: &str, _n: usize) -> Vec<SuggestionItem> {
            self.tags
                .iter()
                .filter(|x| x.display.starts_with(p))
                .cloned()
                .collect()
        }
    }
    #[tokio::test]
    async fn mem_suggestions_filter_by_prefix() {
        let m = Mem {
            notes: vec![SuggestionItem {
                display: "projects".into(),
                secondary: Some("work/projects".into()),
            }],
            tags: vec![SuggestionItem::plain("todo")],
        };
        assert_eq!(m.notes_by_prefix("pro", 9).await.len(), 1);
        assert_eq!(m.notes_by_prefix("pro", 9).await[0].display, "projects");
        assert_eq!(m.tags_by_prefix("to", 9).await[0].display, "todo");
    }
}
