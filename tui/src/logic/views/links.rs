//! LINKS: the open note's backlinks, outgoing note links and unlinked
//! mentions, one [`LinksTab`] at a time.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::{LinkType, NoteContentData, NoteLink, NoteTarget};
use kimun_core::{LinkResolution, NoteVault};

use crate::logic::list::{Emit, ListRow, RowSource, YankTarget};
use crate::logic::nav::LinksTab;

/// One LINKS row: a note, its title and its filename.
#[derive(Clone)]
pub struct LinkEntry {
    pub path: VaultPath,
    pub title: String,
    pub filename: String,
}

impl LinkEntry {
    /// A row for a note known only by path (outgoing links): the clean name
    /// stands in for the title.
    pub fn from_path(path: VaultPath) -> Self {
        let title = path.get_clean_name();
        let (_, filename) = path.get_parent_path();
        Self {
            path,
            title,
            filename,
        }
    }
}

impl ListRow for LinkEntry {
    fn match_text(&self) -> Option<&str> {
        Some(&self.filename)
    }

    fn yank_target(&self) -> Option<YankTarget> {
        Some(YankTarget::path(self.path.to_string()))
    }
}

/// The rows of `note`'s `tab`. Empty for no note (root/empty path) and on an
/// index error. Async (index reads; Unlinked runs two concurrently).
/// - **Backlinks:** core `get_backlinks`, in core's order.
/// - **Outgoing:** the note's `LinkType::Note` links (read from the file), in
///   document order, each target once.
/// - **Unlinked:** notes whose text mentions the note's clean name (searched
///   as one quoted phrase) minus the notes linking to it and the note itself.
pub async fn link_entries(vault: &NoteVault, note: &VaultPath, tab: LinksTab) -> Vec<LinkEntry> {
    if note.is_root_or_empty() {
        return Vec::new();
    }
    match tab {
        LinksTab::Backlinks => vault
            .get_backlinks(note)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(entry, content)| entry_row(entry, content))
            .collect(),
        LinksTab::Outgoing => {
            let links = vault
                .get_markdown_and_links(note)
                .await
                .map(|md| md.links)
                .unwrap_or_default();
            outgoing_entries(vault, links).await
        }
        LinksTab::Unlinked => {
            // Notes whose body mentions this note's name as plain text
            // but does not link to it: text-search the clean name, then
            // subtract the linking notes and the note itself.
            let name = note.get_clean_name();
            if name.is_empty() {
                return Vec::new();
            }
            // Quote the name so multi-word names search as one literal
            // phrase, not an AND of words. Fetch both sets concurrently.
            let (backlinks, mentions) = tokio::join!(
                vault.get_backlinks(note),
                vault.search_notes(kimun_core::quote_query_term(&name))
            );
            let linked: HashSet<VaultPath> = backlinks
                .unwrap_or_default()
                .into_iter()
                .map(|(entry, _)| entry.path)
                .collect();
            mentions
                .unwrap_or_default()
                .into_iter()
                // `is_like`: `note` may be relative while `entry.path` is
                // index-absolute, so `==` would fail to exclude the
                // open note from its own unlinked-mentions list.
                .filter(|(entry, _)| !entry.path.is_like(note) && !linked.contains(&entry.path))
                .map(|(entry, content)| entry_row(entry, content))
                .collect()
        }
    }
}

/// The notes `links` (a note's links, `get_markdown_and_links`) lead to, in
/// document order, each note once: every [`LinkType::Note`] target resolved
/// by core the way a followed link is (`NoteVault::resolve_note_target`) —
/// a wikilink name in any folder whose path ends with it, a path exactly.
/// An ambiguous name gives a row per candidate, a target with no note a row
/// for the path a new note would get. Shared by LINKS Outgoing and the MCP
/// outlinks.
pub async fn outgoing_entries(
    vault: &NoteVault,
    links: impl IntoIterator<Item = NoteLink>,
) -> Vec<LinkEntry> {
    // Each distinct target once.
    let mut seen = HashSet::new();
    let targets: Vec<NoteTarget> = links
        .into_iter()
        .filter_map(|link| match link.ltype {
            LinkType::Note(target) => seen.insert(target.clone()).then_some(target),
            _ => None,
        })
        .collect();
    let resolved = futures::future::join_all(
        targets
            .into_iter()
            .map(|target| outgoing_rows(vault, target)),
    )
    .await;
    // Two written forms can reach one note (`[[anton]]` and
    // `[[../work/anton]]`): list it once.
    let mut rows: Vec<LinkEntry> = Vec::new();
    for row in resolved.into_iter().flatten() {
        if !rows.iter().any(|r| r.path.is_like(&row.path)) {
            rows.push(row);
        }
    }
    rows
}

/// A row for an indexed note: titled by the note's own title.
fn entry_row(entry: NoteEntryData, content: NoteContentData) -> LinkEntry {
    let (_, filename) = entry.path.get_parent_path();
    LinkEntry {
        path: entry.path,
        title: content.title,
        filename,
    }
}

/// The rows one link target leads to (`NoteVault::resolve_note_target`):
/// the note, every candidate of an ambiguous name, or, when there is none
/// (or the lookup fails), a row for the path a new note would get (opening
/// it offers to create it).
async fn outgoing_rows(vault: &NoteVault, target: NoteTarget) -> Vec<LinkEntry> {
    match vault.resolve_note_target(&target).await {
        Ok(LinkResolution::Note(entry, content)) => vec![entry_row(entry, content)],
        Ok(LinkResolution::Ambiguous(hits)) => hits
            .into_iter()
            .map(|(entry, content)| entry_row(entry, content))
            .collect(),
        Ok(LinkResolution::Missing(path)) => vec![LinkEntry::from_path(path)],
        Err(_) => vec![LinkEntry::from_path(target.path().clone())],
    }
}

/// [`link_entries`] for one note and tab as a [`RowSource`]: one `Replace`,
/// the query ignored. A tab or note change is a new source.
pub struct LinksSource {
    vault: Arc<NoteVault>,
    note: VaultPath,
    tab: LinksTab,
}

impl LinksSource {
    pub fn new(vault: Arc<NoteVault>, note: VaultPath, tab: LinksTab) -> Self {
        Self { vault, note, tab }
    }
}

#[async_trait]
impl RowSource<LinkEntry> for LinksSource {
    async fn load(&self, _query: &str, emit: Emit<LinkEntry>) {
        emit.replace(link_entries(&self.vault, &self.note, self.tab).await);
    }

    fn reload_on_query(&self) -> bool {
        false
    }
}
