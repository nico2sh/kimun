//! The drawer views: **TAGS**, **LINKS**, and **OUTLINE** — each a
//! thin adapter (`ListPanelSpec` + a `RowSource`) of the shared
//! [`QueryListPanel`] body, over core's vault API. Rows, sources and the
//! outline model live in `logic::views`; this module draws them. Rebuilt on demand
//! (`refresh`) — the same engine-per-context pattern the sidebar uses per
//! directory.

use std::num::NonZeroU64;
use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{ListItem, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::InputEvent;
use crate::components::panel::panel_block;
use crate::components::query_list_panel::{ListPanelSpec, QueryListPanel};
use crate::components::rich_row::RichRow;
use crate::components::search_list::SearchRow;
use crate::keys::key_combo::KeyCombo;
use crate::logic::message::{AppMessage, AppTx, FileOp};
use crate::logic::nav::{HeadingTarget, LinksTab};
use crate::logic::views::{Changed, LinksSource, OutlineModel, TagSource};
/// The rows moved to `logic::views`; re-exported so the old paths still work.
pub use crate::logic::views::{LinkEntry, OutlineEntry, TagEntry};
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

// ---------------------------------------------------------------------------
// TAGS
// ---------------------------------------------------------------------------

impl SearchRow for TagEntry {
    fn to_list_item(&self, theme: &Theme, _icons: &Icons, _selected: bool) -> ListItem<'static> {
        let aqua = Style::default().fg(theme.aqua.to_ratatui());
        RichRow::new("#", self.label.clone())
            .glyph_style(aqua)
            .title_style(aqua)
            .meta(self.count.to_string())
            .into_list_item(theme)
    }

    fn visual_height(&self) -> u16 {
        1
    }
}

/// Spec: Enter / click runs the tag's query in the FIND drawer.
pub struct TagsSpec;

impl ListPanelSpec for TagsSpec {
    type Row = TagEntry;
    const TITLE: &'static str = "Tags";

    fn submit(row: &TagEntry, tx: &AppTx) {
        tx.send(AppMessage::RunTagQuery(row.label.clone())).ok();
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
        self.body.set_source(TagSource::new(self.vault.clone()), tx);
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

    fn visual_height(&self) -> u16 {
        2
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
        tx.send(AppMessage::open(row.path.clone())).ok();
    }

    fn context_event(row: &LinkEntry) -> Option<AppMessage> {
        Some(AppMessage::FileOp(FileOp::ShowMenu(row.path.clone())))
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
    /// click-to-switch hit-test (keyboard ↔ mouse parity).
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
            LinksSource::new(self.vault.clone(), self.note.clone(), self.tab),
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

impl SearchRow for OutlineEntry {
    fn to_list_item(&self, theme: &Theme, _icons: &Icons, _selected: bool) -> ListItem<'static> {
        let indent = "  ".repeat(self.depth.saturating_sub(1));
        RichRow::new(format!("{indent}≡"), self.target.text.clone())
            .glyph_style(Style::default().fg(theme.gray.to_ratatui()))
            .into_list_item(theme)
    }

    fn visual_height(&self) -> u16 {
        1
    }
}

/// Spec: Enter / click jumps the editor to the heading.
pub struct OutlineSpec;

impl ListPanelSpec for OutlineSpec {
    type Row = OutlineEntry;
    const TITLE: &'static str = "Outline";

    fn submit(row: &OutlineEntry, tx: &AppTx) {
        tx.send(AppMessage::JumpToHeading(row.target.clone())).ok();
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
    /// Which headings are listed and when they change (`logic::views`).
    model: OutlineModel,
    body: QueryListPanel<OutlineSpec>,
}

impl OutlinePanel {
    pub fn new(icons: Icons, yank_combos: Vec<KeyCombo>) -> Self {
        Self {
            model: OutlineModel::new(),
            body: QueryListPanel::new(icons, yank_combos),
        }
    }

    /// Show `note`'s headings from its buffer `text` (asked for only when
    /// needed) at `revision` (`None` when unknown, e.g. a buffer just
    /// replaced). Cheap to call often: an unchanged revision skips the parse,
    /// and unchanged headings keep the list — and so its filter and
    /// selection — as they are. See [`OutlineModel::sync`].
    pub fn sync(
        &mut self,
        note: &VaultPath,
        revision: Option<NonZeroU64>,
        text: impl FnOnce() -> String,
        tx: &AppTx,
    ) {
        if self.model.sync(note, revision, text) == Changed::Rebuilt {
            self.body.set_rows(self.model.entries().to_vec(), tx);
        }
    }

    /// The buffer row `target` stands for; see [`OutlineModel::resolve`].
    pub fn resolve(&self, target: &HeadingTarget) -> Option<usize> {
        self.model.resolve(target)
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
            .map(|l| {
                l.visible_rows()
                    .iter()
                    .map(|r| r.heading().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The panel tests read the model's headings through the panel.
#[cfg(test)]
impl std::ops::Deref for OutlinePanel {
    type Target = OutlineModel;

    fn deref(&self) -> &OutlineModel {
        &self.model
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
            .map(|r| (r.heading().to_string(), r.depth))
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
        let occurrences: Vec<usize> = list
            .visible_rows()
            .iter()
            .map(|r| r.target.occurrence)
            .collect();
        assert_eq!(occurrences, [0, 0, 1]);

        OutlineSpec::submit(list.visible_rows()[2], &tx);
        assert!(matches!(
            rx.try_recv(),
            Ok(AppMessage::JumpToHeading(t)) if t.text == "Notes" && t.occurrence == 1 && t.count == 2 && t.line == 2
        ));
    }

    fn target_of(panel: &OutlinePanel, index: usize) -> HeadingTarget {
        panel.entries[index].target.clone()
    }

    #[test]
    fn resolve_follows_a_heading_through_moved_lines() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut panel = outline_panel();
        sync(&mut panel, Some(1), "# Notes\n## Notes\n", &tx);
        let second = target_of(&panel, 1);
        // Two lines typed above: same headings, both moved down.
        sync(&mut panel, Some(2), "new\nnew\n# Notes\n## Notes\n", &tx);
        assert_eq!(panel.resolve(&second), Some(3));
    }

    #[test]
    fn resolve_takes_the_nearest_when_a_same_named_heading_was_added() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut panel = outline_panel();
        let mut text = String::from("# Notes\n");
        text.push_str(&"x\n".repeat(9));
        text.push_str("## Notes\n");
        sync(&mut panel, Some(1), &text, &tx);
        let second = target_of(&panel, 1);
        assert_eq!(second.line, 10);
        // A new `## Notes` typed at line 5: occurrence 1 is now that one.
        let mut edited = String::from("# Notes\n");
        edited.push_str(&"x\n".repeat(4));
        edited.push_str("## Notes\n");
        edited.push_str(&"x\n".repeat(5));
        edited.push_str("## Notes\n");
        sync(&mut panel, Some(2), &edited, &tx);
        assert_eq!(panel.resolve(&second), Some(11));
    }

    #[test]
    fn resolve_finds_nothing_for_a_renamed_heading() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut panel = outline_panel();
        sync(&mut panel, Some(1), "# A\n## Foo\n", &tx);
        let foo = target_of(&panel, 1);
        sync(&mut panel, Some(2), "# A\n## Bar\n", &tx);
        assert_eq!(panel.resolve(&foo), None);
    }

    #[test]
    fn outline_sync_keeps_the_list_when_headings_only_move() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut panel = outline_panel();
        sync(&mut panel, Some(1), "# A\n## Beta\n", &tx);
        panel.body.list_mut().unwrap().set_query("bet");
        sync(&mut panel, Some(2), "intro\n\n# A\n## Beta\n", &tx);
        assert_eq!(panel.body.list().unwrap().input_value(), "bet");
        // The kept rows' lines are stale; resolve reads the fresh ones.
        assert_eq!(panel.resolve(&target_of(&panel, 1)), Some(3));
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
        let picked = list.selected_row().unwrap().heading().to_string();

        // A body edit: new revision, same headings — the list stays as it is.
        sync(&mut panel, Some(2), "# A\nnew body\n## Beta\n## Bet\n", &tx);
        let list = panel.body.list().unwrap();
        assert_eq!(list.input_value(), "bet");
        assert_eq!(list.selected_row().unwrap().heading(), picked);

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
