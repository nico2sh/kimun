//! The phase-03 drawer views: **TAGS**, **LINKS**, and **OUTLINE** — each a
//! thin adapter (`ListPanelSpec` + a `RowSource`) of the shared
//! [`QueryListPanel`] body, over core's vault API. Rebuilt on demand
//! (`refresh`) — the same engine-per-context pattern the sidebar uses per
//! directory.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
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
use crate::components::search_list::{Emit, RowSource, SearchRow, YankTarget};
use crate::keys::key_combo::KeyCombo;
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
    pub fn new(vault: Arc<NoteVault>, icons: Icons, yank_combos: Vec<KeyCombo>) -> Self {
        Self {
            vault,
            body: QueryListPanel::new(icons, yank_combos),
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

    fn yank_target(&self) -> Option<YankTarget> {
        Some(YankTarget::path(self.path.to_string()))
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
                    // index-absolute, so `==` would fail to exclude the
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
    pub fn new(vault: Arc<NoteVault>, icons: Icons, yank_combos: Vec<KeyCombo>) -> Self {
        Self {
            vault,
            note: VaultPath::empty(),
            tab: LinksTab::Backlinks,
            body: QueryListPanel::new(icons, yank_combos),
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
                let r = Rect::new(x, rows[0].y, w.min(rows[0].right() - x), 1);
                crate::components::clickable::register(r);
                self.tab_cells.push((tab, r));
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

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OutlineEntry {
    pub heading: String,
    /// 1-based nesting depth: how many headings enclose this one, plus one.
    /// Not the markdown level — `## A` over `#### B` is depth 1 and 2.
    pub depth: usize,
    /// How many headings above share this text: what tells `# Notes` and
    /// `## Notes` apart when jumping.
    pub occurrence: usize,
}

impl OutlineEntry {
    /// The OUTLINE rows of a note body, in document order.
    fn all_of(text: &str) -> Vec<Self> {
        // Depth counts the open ancestors with a lower level, as a breadcrumb
        // does.
        let mut open_levels: Vec<u8> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        kimun_core::note::note_headings(text)
            .into_iter()
            .map(|heading| {
                open_levels.retain(|lvl| *lvl < heading.level);
                open_levels.push(heading.level);
                let count = seen.entry(heading.text.clone()).or_default();
                let occurrence = *count;
                *count += 1;
                OutlineEntry {
                    heading: heading.text,
                    depth: open_levels.len(),
                    occurrence,
                }
            })
            .collect()
    }
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

    fn yank_target(&self) -> Option<YankTarget> {
        // The heading text alone — the row carries no note path, and the depth
        // is presentation, not content.
        Some(YankTarget::new(self.heading.clone(), "heading"))
    }
}

/// Spec: Enter / click jumps the editor to the heading.
pub struct OutlineSpec;

impl ListPanelSpec for OutlineSpec {
    type Row = OutlineEntry;
    const TITLE: &'static str = "Outline";

    fn submit(row: &OutlineEntry, tx: &AppTx) {
        tx.send(AppEvent::JumpToHeading {
            heading: row.heading.clone(),
            occurrence: row.occurrence,
        })
        .ok();
    }

    fn hints() -> Vec<(String, String)> {
        vec![("Enter".into(), "Jump to heading".into())]
    }
}

/// The OUTLINE drawer: the open note's headings as an indented tree, read
/// from the editor buffer (not the file). Refreshed when revealed, on each
/// autosave tick and on a jump — so between ticks it may trail the buffer by
/// up to `autosave_interval_secs`; the jump itself always reads the live
/// buffer.
pub struct OutlinePanel {
    note: VaultPath,
    /// Buffer revision `entries` was computed at; `None` = unknown, re-parse.
    revision: Option<NonZeroU64>,
    entries: Vec<OutlineEntry>,
    body: QueryListPanel<OutlineSpec>,
}

impl OutlinePanel {
    pub fn new(icons: Icons, yank_combos: Vec<KeyCombo>) -> Self {
        Self {
            note: VaultPath::empty(),
            revision: None,
            entries: Vec::new(),
            body: QueryListPanel::new(icons, yank_combos),
        }
    }

    /// Show `note`'s headings from its buffer `text` (asked for only when
    /// needed) at `revision` (`None` when unknown, e.g. a buffer just
    /// replaced). Cheap to call often: an unchanged revision skips the parse,
    /// and unchanged headings keep the list — and so its filter and
    /// selection — as they are.
    pub fn sync(
        &mut self,
        note: &VaultPath,
        revision: Option<NonZeroU64>,
        text: impl FnOnce() -> String,
        tx: &AppTx,
    ) {
        let same_note = *note == self.note && self.body.is_loaded();
        if same_note && revision.is_some() && revision == self.revision {
            return;
        }
        let entries = OutlineEntry::all_of(&text());
        self.revision = revision;
        if same_note && entries == self.entries {
            return;
        }
        self.note = note.clone();
        self.entries = entries;
        self.body.set_rows(self.entries.clone(), tx);
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

    /// The listed headings.
    #[cfg(test)]
    pub fn headings_for_test(&self) -> Vec<String> {
        self.body
            .list()
            .map(|l| l.visible_rows().iter().map(|r| r.heading.clone()).collect())
            .unwrap_or_default()
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

        let mut panel = TagsPanel::new(
            vault,
            Icons::new(false),
            vec![crate::keys::default_yank_combo()],
        );
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

        let mut panel = LinksPanel::new(
            vault,
            Icons::new(false),
            vec![crate::keys::default_yank_combo()],
        );
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

    fn outline_panel() -> OutlinePanel {
        OutlinePanel::new(Icons::new(false), vec![crate::keys::default_yank_combo()])
    }

    fn rows(panel: &OutlinePanel) -> Vec<(String, usize)> {
        panel
            .body
            .list()
            .unwrap()
            .visible_rows()
            .iter()
            .map(|r| (r.heading.clone(), r.depth))
            .collect()
    }

    fn sync(panel: &mut OutlinePanel, revision: Option<u64>, text: &str, tx: &AppTx) {
        let text = text.to_string();
        panel.sync(
            &VaultPath::note_path_from("doc"),
            revision.and_then(NonZeroU64::new),
            move || text,
            tx,
        );
    }

    fn owned(rows: &[(&str, usize)]) -> Vec<(String, usize)> {
        rows.iter().map(|(h, d)| (h.to_string(), *d)).collect()
    }

    #[test]
    fn outline_panel_lists_headings_in_order() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut panel = outline_panel();
        let text = "# Top\nintro\n## Sub One\nbody\n## Sub Two\nmore\n# Second\nend\n";
        sync(&mut panel, Some(1), text, &tx);
        assert_eq!(
            rows(&panel),
            owned(&[("Top", 1), ("Sub One", 2), ("Sub Two", 2), ("Second", 1)])
        );
    }

    #[test]
    fn outline_panel_lists_a_heading_with_no_body() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut panel = outline_panel();
        let text = "# Title\n## Title2\nbody\n### Skipped [[link|Shown]] #tag\n";
        sync(&mut panel, Some(1), text, &tx);
        // Rendered as the chunker renders breadcrumbs: links and tag markers gone.
        assert_eq!(
            rows(&panel),
            owned(&[("Title", 1), ("Title2", 2), ("Skipped Shown tag", 3)])
        );
    }

    #[test]
    fn outline_rows_number_same_named_headings_and_jump_by_that() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut panel = outline_panel();
        sync(&mut panel, Some(1), "# Notes\n## Other\n## Notes\n", &tx);
        let list = panel.body.list().unwrap();
        let occurrences: Vec<usize> = list.visible_rows().iter().map(|r| r.occurrence).collect();
        assert_eq!(occurrences, [0, 0, 1]);

        OutlineSpec::submit(list.visible_rows()[2], &tx);
        assert!(matches!(
            rx.try_recv(),
            Ok(AppEvent::JumpToHeading { heading, occurrence: 1 }) if heading == "Notes"
        ));
    }

    #[test]
    fn outline_depth_is_nesting_not_heading_level() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut panel = outline_panel();
        sync(&mut panel, Some(1), "## A\n#### B\n# C\n", &tx);
        assert_eq!(rows(&panel), owned(&[("A", 1), ("B", 2), ("C", 1)]));
    }

    #[test]
    fn outline_sync_keeps_filter_and_selection_while_headings_are_unchanged() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut panel = outline_panel();
        sync(&mut panel, Some(1), "# A\n## Beta\n## Bet\n", &tx);
        let list = panel.body.list_mut().unwrap();
        list.set_query("bet");
        list.select_next();
        let picked = list.selected_row().unwrap().heading.clone();

        // A body edit: new revision, same headings — the list stays as it is.
        sync(&mut panel, Some(2), "# A\nnew body\n## Beta\n## Bet\n", &tx);
        let list = panel.body.list().unwrap();
        assert_eq!(list.input_value(), "bet");
        assert_eq!(list.selected_row().unwrap().heading, picked);

        // A heading edit rebuilds it.
        sync(&mut panel, Some(3), "# A\n## Beta\n## Bet\n## Gamma\n", &tx);
        assert_eq!(panel.body.list().unwrap().input_value(), "");
        assert_eq!(rows(&panel).len(), 4);
    }

    #[test]
    fn outline_sync_skips_the_parse_at_an_unchanged_revision() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut panel = outline_panel();
        let note = VaultPath::note_path_from("doc");
        let rev = NonZeroU64::new(7);
        panel.sync(&note, rev, || "# A\n".to_string(), &tx);
        panel.sync(
            &note,
            rev,
            || panic!("parsed at an unchanged revision"),
            &tx,
        );
        // An unknown revision always re-reads.
        let mut read = false;
        panel.sync(
            &note,
            None,
            || {
                read = true;
                "# A\n".to_string()
            },
            &tx,
        );
        assert!(read);
    }
}
