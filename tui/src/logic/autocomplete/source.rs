//! Autocomplete candidates and where they come from.

use async_trait::async_trait;

/// One autocomplete candidate: the inserted/display text plus an optional
/// secondary line shown muted in the popup (a note path, a tag usage count).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionItem {
    pub display: String,
    pub secondary: Option<String>,
    /// Shown in the popup instead of `display` when set; `display` is still
    /// what gets inserted. See [`crate::logic::autocomplete::Suggestion::label`].
    pub label: Option<String>,
}

impl SuggestionItem {
    pub fn plain(display: impl Into<String>) -> Self {
        Self {
            display: display.into(),
            secondary: None,
            label: None,
        }
    }

    /// Show `label` in the popup while `display` stays the inserted text.
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl From<SuggestionItem> for super::state::Suggestion {
    fn from(item: SuggestionItem) -> Self {
        Self {
            display: item.display,
            secondary: item.secondary,
            label: item.label,
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

    /// Property keys (`%key`) starting with `prefix`, ignoring case and
    /// accents. Defaults to empty, like the saved searches.
    async fn properties_by_prefix(&self, _prefix: &str, _limit: usize) -> Vec<SuggestionItem> {
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
                        label: None,
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
                        label: None,
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
                        label: None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    async fn properties_by_prefix(&self, prefix: &str, limit: usize) -> Vec<SuggestionItem> {
        self.vault
            .suggest_property_keys_by_prefix(prefix, limit)
            .await
            .map(|keys| keys.into_iter().map(SuggestionItem::plain).collect())
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
                label: None,
            }],
            tags: vec![SuggestionItem::plain("todo")],
        };
        assert_eq!(m.notes_by_prefix("pro", 9).await.len(), 1);
        assert_eq!(m.notes_by_prefix("pro", 9).await[0].display, "projects");
        assert_eq!(m.tags_by_prefix("to", 9).await[0].display, "todo");
    }

    #[tokio::test]
    async fn vault_property_suggestions_match_keys_in_search_form() {
        use kimun_core::nfs::VaultPath;
        use kimun_core::{NoteVault, VaultConfig};
        let tmp = tempfile::TempDir::new().unwrap();
        let cfg = VaultConfig::new(crate::test_support::sys(tmp.path()));
        let vault = NoteVault::new(cfg).await.unwrap();
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(
                &VaultPath::note_path_from("/a.md"),
                "---\nStatus: x\nstart: 1\nR\u{e9}sum\u{e9}: z\n---\nbody",
            )
            .await
            .unwrap();
        let src = VaultSuggestions {
            vault: std::sync::Arc::new(vault),
        };
        let names = |items: Vec<SuggestionItem>| -> Vec<String> {
            items.into_iter().map(|i| i.display).collect()
        };
        assert_eq!(
            names(src.properties_by_prefix("st", 10).await),
            vec!["start", "status"]
        );
        assert_eq!(names(src.properties_by_prefix("ST", 10).await).len(), 2);
        assert_eq!(
            names(src.properties_by_prefix("res", 10).await),
            vec!["resume"]
        );
        assert_eq!(names(src.properties_by_prefix("", 2).await).len(), 2);
        assert!(src.properties_by_prefix("zzz", 10).await.is_empty());
    }
}
