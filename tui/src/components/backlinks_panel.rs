use std::sync::Arc;

use kimun_core::nfs::VaultPath;
use kimun_core::NoteVault;
use ratatui::widgets::ListState;

use crate::components::events::{AppEvent, AppTx};
use crate::components::file_list::{SortField, SortOrder};
use crate::keys::KeyBindings;

// ---------------------------------------------------------------------------
// BacklinkEntry
// ---------------------------------------------------------------------------

/// A single backlink entry with preloaded context.
#[derive(Debug, Clone)]
pub struct BacklinkEntry {
    pub path: VaultPath,
    pub title: String,
    pub filename: String,
    /// The paragraph in this note that contains the link to the current note.
    pub context: String,
    /// Full note text, loaded lazily on first expand.
    pub full_text: Option<String>,
}

// ---------------------------------------------------------------------------
// ExpandState (private)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum ExpandState {
    Collapsed,
    Context,
    Full,
}

// ---------------------------------------------------------------------------
// BacklinksPanel
// ---------------------------------------------------------------------------

pub struct BacklinksPanel {
    entries: Vec<BacklinkEntry>,
    expand_states: Vec<ExpandState>,
    list_state: ListState,
    loading: bool,
    current_note: VaultPath,
    sort_field: SortField,
    sort_order: SortOrder,
    vault: Arc<NoteVault>,
    key_bindings: KeyBindings,
    scroll_offset: usize,
}

impl BacklinksPanel {
    pub fn new(vault: Arc<NoteVault>, key_bindings: KeyBindings) -> Self {
        Self {
            entries: Vec::new(),
            expand_states: Vec::new(),
            list_state: ListState::default(),
            loading: false,
            current_note: VaultPath::empty(),
            sort_field: SortField::Name,
            sort_order: SortOrder::Ascending,
            vault,
            key_bindings,
            scroll_offset: 0,
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn selected_path(&self) -> Option<&VaultPath> {
        self.list_state
            .selected()
            .and_then(|i| self.entries.get(i))
            .map(|e| &e.path)
    }

    // ── Loading ─────────────────────────────────────────────────────────

    /// Begin loading backlinks for `note_path`. Clears existing state, sets
    /// `loading = true`, and spawns a background task that sends
    /// `AppEvent::BacklinksLoaded` when finished.
    pub fn load(&mut self, note_path: VaultPath, tx: AppTx) {
        self.entries.clear();
        self.expand_states.clear();
        self.list_state.select(None);
        self.loading = true;
        self.current_note = note_path.clone();
        self.scroll_offset = 0;

        let vault = Arc::clone(&self.vault);
        tokio::spawn(async move {
            let entries = load_backlinks(&vault, &note_path).await;
            let _ = tx.send(AppEvent::BacklinksLoaded(entries));
        });
    }

    /// Called when the background task completes. Stores the entries, applies
    /// the current sort, and initialises expand states.
    pub fn on_loaded(&mut self, entries: Vec<BacklinkEntry>) {
        self.entries = entries;
        self.apply_sort();
        self.expand_states = vec![ExpandState::Collapsed; self.entries.len()];
        self.loading = false;
        if !self.entries.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    /// Sort `entries` (and their parallel `expand_states`) by the active
    /// sort field and order.
    pub fn apply_sort(&mut self) {
        let field = self.sort_field;
        let order = self.sort_order;

        // Build index permutation so we can reorder expand_states in sync.
        let mut indices: Vec<usize> = (0..self.entries.len()).collect();
        indices.sort_by(|&a, &b| {
            let cmp = match field {
                SortField::Name => self.entries[a]
                    .filename
                    .to_lowercase()
                    .cmp(&self.entries[b].filename.to_lowercase()),
                SortField::Title => self.entries[a]
                    .title
                    .to_lowercase()
                    .cmp(&self.entries[b].title.to_lowercase()),
            };
            match order {
                SortOrder::Ascending => cmp,
                SortOrder::Descending => cmp.reverse(),
            }
        });

        let sorted_entries: Vec<BacklinkEntry> =
            indices.iter().map(|&i| self.entries[i].clone()).collect();
        let sorted_states: Vec<ExpandState> = if self.expand_states.len() == self.entries.len() {
            indices.iter().map(|&i| self.expand_states[i]).collect()
        } else {
            vec![ExpandState::Collapsed; sorted_entries.len()]
        };

        self.entries = sorted_entries;
        self.expand_states = sorted_states;
    }

    /// Called when full text for a backlink entry has been loaded.
    pub fn on_full_text_loaded(&mut self, index: usize, text: String) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.full_text = Some(text);
        }
    }
}

// ---------------------------------------------------------------------------
// Standalone async helpers
// ---------------------------------------------------------------------------

/// Load all backlinks for `note_path` from the vault, fetching note text and
/// extracting context for each one.
async fn load_backlinks(vault: &NoteVault, note_path: &VaultPath) -> Vec<BacklinkEntry> {
    let backlinks = match vault.get_backlinks(note_path).await {
        Ok(bl) => bl,
        Err(_) => return Vec::new(),
    };

    let target_name = note_path.get_clean_name();

    let mut entries = Vec::with_capacity(backlinks.len());
    for (entry_data, content_data) in backlinks {
        let text = vault
            .get_note_text(&entry_data.path)
            .await
            .unwrap_or_default();
        let context = extract_context(&text, &target_name);
        let (_parent, filename) = entry_data.path.get_parent_path();

        entries.push(BacklinkEntry {
            path: entry_data.path,
            title: content_data.title,
            filename,
            context,
            full_text: None,
        });
    }

    entries
}

/// Find the paragraph in `text` that contains a link to `target_name`.
///
/// A "paragraph" is a run of consecutive non-blank lines. The function
/// searches for several link patterns (case-insensitive):
/// - `[[target_name]]`        — full wikilink
/// - `[[target_name`          — partial wikilink (e.g. with alias)
/// - `(target_name)`          — markdown link
/// - `(target_name.md)`       — markdown link with extension
///
/// If no match is found, returns the first non-blank line as a fallback.
fn extract_context(text: &str, target_name: &str) -> String {
    let target_lower = target_name.to_lowercase();

    // Build search needles (lowercase).
    let wikilink_full = format!("[[{}]]", target_lower);
    let wikilink_partial = format!("[[{}", target_lower);
    let md_link = format!("({})", target_lower);
    let md_link_ext = format!("({}.md)", target_lower);

    // Split text into paragraphs (groups of consecutive non-blank lines).
    let paragraphs = split_paragraphs(text);

    for para in &paragraphs {
        let lower = para.to_lowercase();
        if lower.contains(&wikilink_full)
            || lower.contains(&wikilink_partial)
            || lower.contains(&md_link)
            || lower.contains(&md_link_ext)
        {
            return para.clone();
        }
    }

    // Fallback: first non-blank line.
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_string()
}

/// Split text into paragraphs. A paragraph is one or more consecutive
/// non-blank lines. Blank lines act as separators.
fn split_paragraphs(text: &str) -> Vec<String> {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_context_finds_wikilink_paragraph() {
        let text = "\
# Heading

This is an intro paragraph.

Here I reference [[my-note]] in some context
that spans two lines.

Another paragraph without links.";

        let result = extract_context(text, "my-note");
        assert!(result.contains("[[my-note]]"));
        assert!(result.contains("that spans two lines"));
    }

    #[test]
    fn extract_context_fallback_to_first_line() {
        let text = "\
# No links here

Just a normal paragraph.";

        let result = extract_context(text, "other-note");
        assert_eq!(result, "# No links here");
    }

    #[test]
    fn extract_context_finds_markdown_link() {
        let text = "\
# Title

See [related](my-note.md) for details.

Unrelated content.";

        let result = extract_context(text, "my-note");
        assert!(result.contains("(my-note.md)"));
    }
}
