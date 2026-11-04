//! TAGS: every label in the vault with its note count, most-used first.

use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;

use crate::logic::list::{Emit, ListRow, RowSource, YankTarget};

/// One TAGS row: a label and how many notes carry it.
#[derive(Clone)]
pub struct TagEntry {
    pub label: String,
    pub count: usize,
}

impl ListRow for TagEntry {
    fn match_text(&self) -> Option<&str> {
        Some(&self.label)
    }

    fn yank_target(&self) -> Option<YankTarget> {
        // With the `#` sigil, so the copied text is usable as-is in a note —
        // unless the label can't be a hashtag (a frontmatter tag with spaces
        // or dashes): then the label itself.
        let text = if kimun_core::note::is_hashtag_label(&self.label) {
            format!("#{}", self.label)
        } else {
            self.label.clone()
        };
        Some(YankTarget::new(text, "tag"))
    }
}

/// The TAGS rows: core's `label_counts`, most-used first, ties alphabetical
/// (counts come in alphabetical). Empty on an index error. Async (index read).
pub async fn tag_entries(vault: &NoteVault) -> Vec<TagEntry> {
    let mut rows: Vec<TagEntry> = vault
        .label_counts()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(label, count)| TagEntry { label, count })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.count));
    rows
}

/// [`tag_entries`] as a [`RowSource`]: loads once (the query is ignored; a
/// local fuzzy filter over the label narrows the set).
pub struct TagSource {
    vault: Arc<NoteVault>,
}

impl TagSource {
    pub fn new(vault: Arc<NoteVault>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl RowSource<TagEntry> for TagSource {
    async fn load(&self, _query: &str, emit: Emit<TagEntry>) {
        emit.replace(tag_entries(&self.vault).await);
    }

    fn reload_on_query(&self) -> bool {
        false // load once; the local fuzzy filter narrows the set
    }
}
