//! The Query panel's (FIND view's) rows and how they load: the default
//! backlinks query in all its spellings, the row ([`BacklinkEntry`]), the row
//! source ([`BacklinkSource`], wrap it in a
//! [`ResolvingRowSource`](crate::logic::list::ResolvingRowSource)), and the
//! loader that cuts each hit's context paragraph ([`load_query`]).

use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

use crate::logic::list::{Emit, ListRow, RowSource, YankTarget};
use crate::logic::query::{VAR_NOTE, emphasis_needles};

/// The canonical backlinks query (`<` / `lk:`; `>` is forward links). The
/// panel no longer starts on it — the LINKS drawer owns backlinks — but any
/// spelling of it still titles the panel "Backlinks".
pub const DEFAULT_QUERY: &str = "<{note}";
/// The long-form spelling of [`DEFAULT_QUERY`] (`lk:` is the documented
/// synonym of `<`), recognized so it also reads as the default.
pub const DEFAULT_QUERY_LONG: &str = "lk:{note}";

/// True when `query` is the default backlinks query in any spelling: the
/// canonical `<{note}`, the bare `<` sugar, the long form `lk:` — with or
/// without an order directive. Drives the "Backlinks" title and the
/// breadcrumb's blank-query condition, so every synonym reads as the default.
pub fn is_default_query(query: &str) -> bool {
    let expanded =
        kimun_core::expand_bare_note_prefixes(&kimun_core::strip_order_directive(query), VAR_NOTE);
    expanded == DEFAULT_QUERY || expanded == DEFAULT_QUERY_LONG
}

/// A single backlink entry with preloaded context.
#[derive(Debug, Clone)]
pub struct BacklinkEntry {
    pub path: VaultPath,
    pub title: String,
    pub filename: String,
    /// The paragraph in this note that contains the link to the current note.
    pub context: String,
    /// Full note text, loaded when backlinks are fetched.
    pub full_text: Option<String>,
}

impl ListRow for BacklinkEntry {
    fn match_text(&self) -> Option<&str> {
        Some(&self.filename)
    }

    fn yank_target(&self) -> Option<YankTarget> {
        Some(YankTarget::path(self.path.to_string()))
    }
}

/// Row source for the Query panel. It receives an already-resolved query
/// string — [`ResolvingRowSource`](crate::logic::list::ResolvingRowSource)
/// substitutes `{note}` and short-circuits the purely-note-dependent-but-no-note
/// case to an empty list ([`Unresolvable::Empty`](crate::logic::list::Unresolvable::Empty))
/// before this source is asked to load. Result ordering comes from the query
/// string's order directive, applied by the vault DB — the source no longer
/// sorts in memory beyond the no-directive default.
pub struct BacklinkSource {
    vault: Arc<NoteVault>,
}

impl BacklinkSource {
    pub fn new(vault: Arc<NoteVault>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl RowSource<BacklinkEntry> for BacklinkSource {
    async fn load(&self, query: &str, emit: Emit<BacklinkEntry>) {
        let mut entries = load_query(&self.vault, query).await;
        // The DB orders results only when the query carries an `or:` directive
        // (core applies the sort iff `order_by` is non-empty). Keep that
        // directive as the source of truth, but fall back to a stable
        // Name-ascending order when the query has none — otherwise default
        // backlinks come back in arbitrary DB scan order, and the sort dialog's
        // reported default (Name/Ascending) would not match the displayed list.
        if kimun_core::SearchTerms::from_query_string(query)
            .order_by
            .is_empty()
        {
            entries.sort_by_key(|e| e.filename.to_lowercase());
        }
        emit.replace(entries);
    }
}

/// Run `query` (already a resolved plain query string) and build entries.
/// Sources from full-text / query search via `vault.search_notes`.
pub async fn load_query(vault: &NoteVault, query: &str) -> Vec<BacklinkEntry> {
    let needles = emphasis_needles(query);
    let results = vault.search_notes(query).await.unwrap_or_default();
    let mut entries = Vec::with_capacity(results.len());
    for (entry_data, content_data) in results {
        let text = vault
            .get_note_text(&entry_data.path)
            .await
            .unwrap_or_default();
        let context = extract_context_multi(&text, &needles);
        let (_p, filename) = entry_data.path.get_parent_path();
        entries.push(BacklinkEntry {
            path: entry_data.path,
            title: content_data.title,
            filename,
            context,
            full_text: Some(text),
        });
    }
    entries
}

/// Split text into paragraphs. A paragraph is one or more consecutive
/// non-blank lines. Blank lines act as separators.
pub fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join("\n"));
    }

    paragraphs
}

/// Find the first paragraph containing any of `needles` (case-insensitive);
/// fall back to the first non-blank line.
pub fn extract_context_multi(text: &str, needles: &[String]) -> String {
    let lowered: Vec<String> = needles.iter().map(|n| n.to_lowercase()).collect();
    for para in &split_paragraphs(text) {
        let lower = para.to_lowercase();
        if lowered.iter().any(|n| !n.is_empty() && lower.contains(n)) {
            return para.clone();
        }
    }
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_context_matches_any_needle() {
        let text = "# Title\n\nIntro line.\n\nA paragraph mentioning widget here.\n";
        let result = extract_context_multi(text, &["widget".to_string()]);
        assert!(result.contains("widget"));
    }

    #[test]
    fn default_query_recognized_in_all_spellings() {
        // Bare `<` and the long form are first-class synonyms of the default
        // backlinks query: the panel title must read "Backlinks" and the
        // breadcrumb clear condition must treat them as blank.
        assert!(is_default_query(DEFAULT_QUERY));
        assert!(is_default_query("<"));
        assert!(is_default_query("lk:"));
        assert!(is_default_query("< or:title"));
        assert!(is_default_query("<{note} -or:file"));
        assert!(!is_default_query("<projects"));
        assert!(!is_default_query(">"));
        assert!(!is_default_query(""));
    }

    #[tokio::test]
    async fn query_panel_load_query_lists_matches() {
        let vault = crate::test_support::temp_vault("qp").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/a.md"), "alpha #todo")
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::note_path_from("/b.md"), "beta")
            .await
            .unwrap();
        let entries = load_query(&vault, "#todo").await;
        assert_eq!(entries.len(), 1);
        assert!(entries[0].filename.contains("a"));
    }
}
