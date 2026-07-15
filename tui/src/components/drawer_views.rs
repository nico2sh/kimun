//! The phase-03 drawer views: **TAGS**, **LINKS**, and **OUTLINE** — each a
//! thin adapter (`ListPanelSpec` + a `RowSource`) of the shared
//! [`QueryListPanel`] body, over core's vault API. Rebuilt on demand
//! (`refresh`) — the same engine-per-context pattern the sidebar uses per
//! directory.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_core::note::LinkType;
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, FileOp, InputEvent};
use crate::components::panel::panel_block;
use crate::components::query_list_panel::{ListPanelSpec, QueryListPanel};
use crate::components::rich_row::RichRow;
use crate::components::search_list::{Emit, RowSource, SearchRow};
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// TAGS
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct TagEntry {
    pub label: String,
    pub count: usize,
}

impl SearchRow for TagEntry {
    fn to_list_item(&self, theme: &Theme, _icons: &Icons, _selected: bool) -> ListItem<'static> {
        let aqua = Style::default().fg(theme.aqua.to_ratatui());
        RichRow::new("#", self.label.clone())
            .glyph_style(aqua)
            .title_style(aqua)
            .meta(self.count.to_string())
            .into_list_item(theme)
    }

    fn match_text(&self) -> Option<&str> {
        Some(&self.label)
    }

    fn visual_height(&self) -> u16 {
        1
    }
}

struct TagSource {
    vault: Arc<NoteVault>,
}

#[async_trait]
impl RowSource<TagEntry> for TagSource {
    async fn load(&self, _query: &str, emit: Emit<TagEntry>) {
        let mut rows: Vec<TagEntry> = self
            .vault
            .label_counts()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(label, count)| TagEntry { label, count })
            .collect();
        // Most-used first; ties alphabetical (counts come in alphabetical).
        rows.sort_by_key(|r| std::cmp::Reverse(r.count));
        emit.replace(rows);
    }

    fn reload_on_query(&self) -> bool {
        false // load once; the local fuzzy filter narrows the set
    }
}

/// Spec: Enter / click runs the tag's query in the FIND drawer.
pub struct TagsSpec;

impl ListPanelSpec for TagsSpec {
    type Row = TagEntry;
    const TITLE: &'static str = "Tags";

    fn submit(row: &TagEntry, tx: &AppTx) {
        tx.send(AppEvent::RunTagQuery(row.label.clone())).ok();
    }

    fn hints() -> Vec<(String, String)> {
        vec![("Enter".into(), "Run tag query".into())]
    }
}

/// The TAGS drawer: every `#tag` in the vault with its note count.
pub struct TagsPanel {
    vault: Arc<NoteVault>,
    body: QueryListPanel<TagsSpec>,
}

impl TagsPanel {
    pub fn new(vault: Arc<NoteVault>, icons: Icons) -> Self {
        Self {
            vault,
            body: QueryListPanel::new(icons),
        }
    }

    /// (Re)load the tag list. Called when the view is opened.
    pub fn refresh(&mut self, tx: &AppTx) {
        self.body.set_source(
            TagSource {
                vault: self.vault.clone(),
            },
            tx,
        );
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        self.body.hint_shortcuts()
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        self.body.handle_input(event, tx)
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.body.render(f, rect, theme, focused);
    }
}

// ---------------------------------------------------------------------------
// LINKS
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinksTab {
    Backlinks,
    Outgoing,
    Unlinked,
}

impl LinksTab {
    /// The sub-view order, single source for cycling and the tab bar.
    pub const ORDER: [LinksTab; 3] = [LinksTab::Backlinks, LinksTab::Outgoing, LinksTab::Unlinked];

    /// The tab `steps` away in [`Self::ORDER`], wrapping.
    fn cycled(self, steps: isize) -> LinksTab {
        let n = Self::ORDER.len() as isize;
        let i = Self::ORDER.iter().position(|t| *t == self).unwrap_or(0) as isize;
        Self::ORDER[((i + steps).rem_euclid(n)) as usize]
    }

    fn label(self) -> &'static str {
        match self {
            LinksTab::Backlinks => "backlinks",
            LinksTab::Outgoing => "outgoing",
            LinksTab::Unlinked => "unlinked",
        }
    }
}

#[derive(Clone)]
pub struct LinkEntry {
    pub path: VaultPath,
    pub title: String,
    pub filename: String,
}

impl LinkEntry {
    fn from_path(path: VaultPath) -> Self {
        let title = path.get_clean_name();
        let (_, filename) = path.get_parent_path();
        Self {
            path,
            title,
            filename,
        }
    }
}

impl SearchRow for LinkEntry {
    fn to_list_item(&self, theme: &Theme, icons: &Icons, _selected: bool) -> ListItem<'static> {
        let title = if self.title.is_empty() {
            self.filename.clone()
        } else {
            self.title.clone()
        };
        RichRow::new(icons.note, title)
            .filename(self.filename.clone())
            .into_list_item(theme)
    }

    fn match_text(&self) -> Option<&str> {
        Some(&self.filename)
    }

    fn visual_height(&self) -> u16 {
        2
    }
}

struct LinksSource {
    vault: Arc<NoteVault>,
    note: VaultPath,
    tab: LinksTab,
}

#[async_trait]
impl RowSource<LinkEntry> for LinksSource {
    async fn load(&self, _query: &str, emit: Emit<LinkEntry>) {
        if self.note.is_root_or_empty() {
            emit.replace(Vec::new());
            return;
        }
        let entries = match self.tab {
            LinksTab::Backlinks => self
                .vault
                .get_backlinks(&self.note)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|(entry, content)| {
                    let (_, filename) = entry.path.get_parent_path();
                    LinkEntry {
                        path: entry.path,
                        title: content.title,
                        filename,
                    }
                })
                .collect(),
            LinksTab::Outgoing => {
                let links = self
                    .vault
                    .get_markdown_and_links(&self.note)
                    .await
                    .map(|md| md.links)
                    .unwrap_or_default();
                let mut seen = HashSet::new();
                links
                    .into_iter()
                    .filter_map(|link| match link.ltype {
                        LinkType::Note(path) => seen
                            .insert(path.clone())
                            .then(|| LinkEntry::from_path(path)),
                        _ => None,
                    })
                    .collect()
            }
            LinksTab::Unlinked => {
                // Notes whose body mentions this note's name as plain text
                // but does not link to it: text-search the clean name, then
                // subtract the linking notes and the note itself.
                let name = self.note.get_clean_name();
                if name.is_empty() {
                    emit.replace(Vec::new());
                    return;
                }
                // Quote the name so multi-word names search as one literal
                // phrase, not an AND of words. Fetch both sets concurrently.
                let (backlinks, mentions) = tokio::join!(
                    self.vault.get_backlinks(&self.note),
                    self.vault.search_notes(kimun_core::quote_query_term(&name))
                );
                let linked: HashSet<VaultPath> = backlinks
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(entry, _)| entry.path)
                    .collect();
                mentions
                    .unwrap_or_default()
                    .into_iter()
                    // `is_like`: `self.note` may be relative while `entry.path` is
                    // index-absolute (adr/0021), so `==` would fail to exclude the
                    // open note from its own unlinked-mentions list.
                    .filter(|(entry, _)| {
                        !entry.path.is_like(&self.note) && !linked.contains(&entry.path)
                    })
                    .map(|(entry, content)| {
                        let (_, filename) = entry.path.get_parent_path();
                        LinkEntry {
                            path: entry.path,
                            title: content.title,
                            filename,
                        }
                    })
                    .collect()
            }
        };
        emit.replace(entries);
    }

    fn reload_on_query(&self) -> bool {
        false
    }
}

/// Spec: Enter / click opens the entry; rows are real notes, so right-click
/// opens the file-ops menu. No filter input — `b/o/u` are sub-view keys.
pub struct LinksSpec;

impl ListPanelSpec for LinksSpec {
    type Row = LinkEntry;
    const TITLE: &'static str = "Links";
    const HAS_FILTER: bool = false;

    fn submit(row: &LinkEntry, tx: &AppTx) {
        tx.send(AppEvent::open(row.path.clone())).ok();
    }

    fn context_event(row: &LinkEntry) -> Option<AppEvent> {
        Some(AppEvent::FileOp(FileOp::ShowMenu(row.path.clone())))
    }

    fn hints() -> Vec<(String, String)> {
        vec![
            ("b/o/u".into(), "Sub-view".into()),
            ("Enter".into(), "Open".into()),
        ]
    }
}

/// The LINKS drawer for the open note: backlinks / outgoing / unlinked
/// mentions as sub-tabs (`b` / `o` / `u`, or ←/→) over the shared body.
pub struct LinksPanel {
    vault: Arc<NoteVault>,
    note: VaultPath,
    tab: LinksTab,
    body: QueryListPanel<LinksSpec>,
    /// Screen cell each sub-view tab was drawn into on the last render —
    /// click-to-switch hit-test (keyboard ↔ mouse parity, spec §10).
    tab_cells: Vec<(LinksTab, Rect)>,
}

impl LinksPanel {
    pub fn new(vault: Arc<NoteVault>, icons: Icons) -> Self {
        Self {
            vault,
            note: VaultPath::empty(),
            tab: LinksTab::Backlinks,
            body: QueryListPanel::new(icons),
            tab_cells: Vec::new(),
        }
    }

    pub fn set_note(&mut self, note: VaultPath, tx: &AppTx) {
        if note != self.note || !self.body.is_loaded() {
            self.note = note;
            self.refresh(tx);
        }
    }

    pub fn tab(&self) -> LinksTab {
        self.tab
    }

    /// Switch to `tab`, used by leader paths (`l b/o/u`).
    pub fn show_tab(&mut self, tab: LinksTab, tx: &AppTx) {
        self.set_tab(tab, tx);
    }

    fn set_tab(&mut self, tab: LinksTab, tx: &AppTx) {
        if tab != self.tab {
            self.tab = tab;
            self.refresh(tx);
        }
    }

    fn refresh(&mut self, tx: &AppTx) {
        self.body.set_source(
            LinksSource {
                vault: self.vault.clone(),
                note: self.note.clone(),
                tab: self.tab,
            },
            tx,
        );
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        self.body.hint_shortcuts()
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        // Tab-bar concerns first (sub-view keys / tab clicks); the rest is
        // the shared body's.
        match event {
            InputEvent::Key(key) => match key.code {
                KeyCode::Char('b') => {
                    self.set_tab(LinksTab::Backlinks, tx);
                    return EventState::Consumed;
                }
                KeyCode::Char('o') => {
                    self.set_tab(LinksTab::Outgoing, tx);
                    return EventState::Consumed;
                }
                KeyCode::Char('u') => {
                    self.set_tab(LinksTab::Unlinked, tx);
                    return EventState::Consumed;
                }
                KeyCode::Left => {
                    self.set_tab(self.tab.cycled(-1), tx);
                    return EventState::Consumed;
                }
                KeyCode::Right => {
                    self.set_tab(self.tab.cycled(1), tx);
                    return EventState::Consumed;
                }
                _ => {}
            },
            InputEvent::Mouse(mouse) => {
                // A click on the tab bar switches the sub-view.
                if matches!(
                    mouse.kind,
                    ratatui::crossterm::event::MouseEventKind::Down(
                        ratatui::crossterm::event::MouseButton::Left
                    )
                ) && let Some(tab) = self
                    .tab_cells
                    .iter()
                    .find(|(_, r)| {
                        r.contains(ratatui::layout::Position::new(mouse.column, mouse.row))
                    })
                    .map(|(t, _)| *t)
                {
                    self.set_tab(tab, tx);
                    return EventState::Consumed;
                }
            }
            _ => {}
        }
        self.body.handle_input(event, tx)
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let block = panel_block("Links", theme, focused);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(inner);

        // Sub-view tab bar: the active tab pops; each tab's cell is recorded
        // so a click switches to it.
        self.tab_cells.clear();
        let mut spans = Vec::new();
        let mut x = rows[0].x;
        for (i, tab) in LinksTab::ORDER.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(
                    " · ",
                    Style::default().fg(theme.gray.to_ratatui()),
                ));
                x += 3;
            }
            let style = if tab == self.tab {
                Style::default()
                    .fg(theme.aqua.to_ratatui())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.gray.to_ratatui())
            };
            let w = tab.label().len() as u16; // labels are ASCII
            if x < rows[0].right() {
                self.tab_cells
                    .push((tab, Rect::new(x, rows[0].y, w.min(rows[0].right() - x), 1)));
            }
            spans.push(Span::styled(tab.label(), style));
            x += w;
        }
        f.render_widget(Paragraph::new(Line::from(spans)), rows[0]);

        self.body.render_in(f, rows[1], rect, theme, focused);
    }
}

// ---------------------------------------------------------------------------
// OUTLINE
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct OutlineEntry {
    pub heading: String,
    /// 1-based heading depth (H1 = 1).
    pub depth: usize,
}

impl SearchRow for OutlineEntry {
    fn to_list_item(&self, theme: &Theme, _icons: &Icons, _selected: bool) -> ListItem<'static> {
        let indent = "  ".repeat(self.depth.saturating_sub(1));
        RichRow::new(format!("{indent}≡"), self.heading.clone())
            .glyph_style(Style::default().fg(theme.gray.to_ratatui()))
            .into_list_item(theme)
    }

    fn match_text(&self) -> Option<&str> {
        Some(&self.heading)
    }

    fn visual_height(&self) -> u16 {
        1
    }
}

struct OutlineSource {
    vault: Arc<NoteVault>,
    note: VaultPath,
}

#[async_trait]
impl RowSource<OutlineEntry> for OutlineSource {
    async fn load(&self, _query: &str, emit: Emit<OutlineEntry>) {
        if self.note.is_root_or_empty() {
            emit.replace(Vec::new());
            return;
        }
        // Read the note and take the heading hierarchy from its content
        // chunks (document order). Each chunk's breadcrumb is the heading
        // path to it; the innermost part is the chunk's own heading.
        let Ok(details) = self.vault.load_note(&self.note).await else {
            emit.replace(Vec::new());
            return;
        };
        // One chunk per heading section (core contract), in document order;
        // a headingless preamble chunk has an empty breadcrumb and is skipped.
        let entries: Vec<OutlineEntry> = details
            .get_content_chunks()
            .into_iter()
            .filter_map(|chunk| {
                let depth = chunk.breadcrumb_parts().count();
                chunk.breadcrumb_last().map(|heading| OutlineEntry {
                    heading: heading.to_string(),
                    depth,
                })
            })
            .collect();
        emit.replace(entries);
    }

    fn reload_on_query(&self) -> bool {
        false
    }
}

/// Spec: Enter / click jumps the editor to the heading.
pub struct OutlineSpec;

impl ListPanelSpec for OutlineSpec {
    type Row = OutlineEntry;
    const TITLE: &'static str = "Outline";

    fn submit(row: &OutlineEntry, tx: &AppTx) {
        tx.send(AppEvent::JumpToHeading(row.heading.clone())).ok();
    }

    fn hints() -> Vec<(String, String)> {
        vec![("Enter".into(), "Jump to heading".into())]
    }
}

/// The OUTLINE drawer: the open note's headings as an indented tree.
pub struct OutlinePanel {
    vault: Arc<NoteVault>,
    note: VaultPath,
    body: QueryListPanel<OutlineSpec>,
}

impl OutlinePanel {
    pub fn new(vault: Arc<NoteVault>, icons: Icons) -> Self {
        Self {
            vault,
            note: VaultPath::empty(),
            body: QueryListPanel::new(icons),
        }
    }

    pub fn set_note(&mut self, note: VaultPath, tx: &AppTx) {
        if note != self.note || !self.body.is_loaded() {
            self.note = note;
            self.refresh(tx);
        }
    }

    /// Re-read the headings (e.g. after the buffer was saved).
    pub fn refresh(&mut self, tx: &AppTx) {
        self.body.set_source(
            OutlineSource {
                vault: self.vault.clone(),
                note: self.note.clone(),
            },
            tx,
        );
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        self.body.hint_shortcuts()
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        self.body.handle_input(event, tx)
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.body.render(f, rect, theme, focused);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_vault;

    use crate::components::search_list::SearchList;

    /// Poll a panel's list until the async load lands.
    async fn drain<R: SearchRow + Clone + Send + Sync + 'static>(list: &mut SearchList<R>) {
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            list.poll();
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tags_panel_lists_label_counts() {
        let vault = temp_vault("tags-panel").await;
        vault.validate_and_init().await.unwrap();
        vault
            .save_note(&VaultPath::note_path_from("a"), "x #alpha #beta")
            .await
            .unwrap();
        vault
            .save_note(&VaultPath::note_path_from("b"), "y #alpha")
            .await
            .unwrap();

        let mut panel = TagsPanel::new(vault, Icons::new(false));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.refresh(&tx);
        drain(panel.body.list_mut().unwrap()).await;

        let rows = panel.body.list().unwrap().visible_rows();
        let labels: Vec<(&str, usize)> = rows.iter().map(|r| (r.label.as_str(), r.count)).collect();
        // Most-used first.
        assert_eq!(labels, vec![("alpha", 2), ("beta", 1)]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn links_panel_tabs_track_note() {
        let vault = temp_vault("links-panel").await;
        vault.validate_and_init().await.unwrap();
        // projectx is linked from linker, mentioned (no link) in mentions.
        vault
            .save_note(&VaultPath::note_path_from("projectx"), "the note body")
            .await
            .unwrap();
        vault
            .save_note(
                &VaultPath::note_path_from("linker"),
                "links to [[projectx]] here",
            )
            .await
            .unwrap();
        vault
            .save_note(
                &VaultPath::note_path_from("mentions"),
                "talks about projectx without linking",
            )
            .await
            .unwrap();

        let mut panel = LinksPanel::new(vault, Icons::new(false));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Backlinks of projectx → linker.
        panel.set_note(VaultPath::note_path_from("projectx"), &tx);
        drain(panel.body.list_mut().unwrap()).await;
        let names: Vec<&str> = panel
            .body
            .list()
            .unwrap()
            .visible_rows()
            .iter()
            .map(|r| r.filename.as_str())
            .collect();
        assert_eq!(names, vec!["linker.md"], "backlinks tab");

        // Outgoing of linker → projectx.
        panel.set_note(VaultPath::note_path_from("linker"), &tx);
        panel.set_tab(LinksTab::Outgoing, &tx);
        drain(panel.body.list_mut().unwrap()).await;
        let names: Vec<&str> = panel
            .body
            .list()
            .unwrap()
            .visible_rows()
            .iter()
            .map(|r| r.filename.as_str())
            .collect();
        assert_eq!(names, vec!["projectx.md"], "outgoing tab");

        // Unlinked mentions of projectx → mentions (linker is excluded).
        panel.set_note(VaultPath::note_path_from("projectx"), &tx);
        panel.set_tab(LinksTab::Unlinked, &tx);
        drain(panel.body.list_mut().unwrap()).await;
        let names: Vec<&str> = panel
            .body
            .list()
            .unwrap()
            .visible_rows()
            .iter()
            .map(|r| r.filename.as_str())
            .collect();
        assert!(
            names.contains(&"mentions.md") && !names.contains(&"linker.md"),
            "unlinked tab: got {names:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn outline_panel_lists_headings_in_order() {
        let vault = temp_vault("outline-panel").await;
        vault.validate_and_init().await.unwrap();
        vault
            .save_note(
                &VaultPath::note_path_from("doc"),
                "# Top\nintro\n## Sub One\nbody\n## Sub Two\nmore\n# Second\nend\n",
            )
            .await
            .unwrap();

        let mut panel = OutlinePanel::new(vault, Icons::new(false));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_note(VaultPath::note_path_from("doc"), &tx);
        drain(panel.body.list_mut().unwrap()).await;

        let rows = panel.body.list().unwrap().visible_rows();
        let headings: Vec<(&str, usize)> =
            rows.iter().map(|r| (r.heading.as_str(), r.depth)).collect();
        assert_eq!(
            headings,
            vec![("Top", 1), ("Sub One", 2), ("Sub Two", 2), ("Second", 1)]
        );
    }
}
