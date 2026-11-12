//! Semantic search surface (P4): the drawer view over the logic's server-backed
//! [`SemanticSource`], which queries the RAG server for similar chunks and
//! lists the matching notes. Only usable when a server is configured and
//! reachable. This file keeps the panel (render + keys).

use std::ops::Range;
use std::sync::{Arc, Mutex};

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{ListItem, Paragraph};

use crate::ask::locate;
use crate::components::event_state::EventState;
use crate::components::events::InputEvent;
use crate::components::expand_chevron::ExpandControl;
use crate::components::file_list::FileListEntry;
use crate::components::panel::panel_block;
use crate::components::preview_pane::{Highlight, PreviewPane, Step};
use crate::components::query_list_panel::{HostEvent, ListPanelSpec, QueryListPanel};
use crate::components::rich_row::RichRow;
use crate::components::search_list::SearchRow;
use crate::keys::key_combo::KeyCombo;
use crate::logic::message::{AppMessage, AppTx, FileOp};
#[cfg(test)]
use crate::logic::views::chunks_to_entries;
pub use crate::logic::views::{SemanticHit, SemanticSource};
#[cfg(test)]
use crate::server_client::ChunkResult;
use crate::settings::SharedSettings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

impl SearchRow for SemanticHit {
    /// The note's row (glyph, open marker, journal date, file name) with the
    /// matched section's title and the score in place of its title: `Title ·
    /// 87%`. A blank title shows the file name.
    fn to_list_item(&self, theme: &Theme, icons: &Icons, _selected: bool) -> ListItem<'static> {
        let FileListEntry::Note {
            filename,
            journal_date,
            is_open,
            ..
        } = &self.entry
        else {
            return self.entry.to_list_item(theme, icons);
        };
        let title = if self.title.trim().is_empty() {
            filename.as_str()
        } else {
            self.title.as_str()
        };
        let percent = (self.score * 100.0).round().clamp(0.0, 100.0) as u32;
        let glyph = if journal_date.is_some() {
            icons.journal
        } else {
            icons.note
        };
        let mut row =
            RichRow::new(glyph, format!("{title} \u{b7} {percent}%")).filename(filename.clone());
        if *is_open {
            row = row.glyph_style(Style::default().fg(theme.accent.to_ratatui()));
        }
        if let Some(date) = journal_date {
            row = row.secondary(
                date.clone(),
                Some(Style::default().fg(theme.color_journal_date.to_ratatui())),
            );
        }
        row.into_list_item(theme)
    }

    fn visual_height(&self) -> u16 {
        self.entry.visual_height()
    }
}

/// Spec for the semantic drawer view: a query input over server-ranked note
/// results; Enter/click opens the note, right-click opens the file-ops menu.
pub struct SemanticSpec;

impl ListPanelSpec for SemanticSpec {
    type Row = SemanticHit;
    const TITLE: &'static str = "Semantic";
    const HAS_FILTER: bool = true;
    // Server-backed query: draw a bordered search box separated from the results
    // and a "Searching…" indicator while the request is in flight.
    const BORDERED_INPUT: bool = true;
    // The server already ranks/filters by the query; don't re-filter its
    // semantic results locally by the literal query text (that discards
    // conceptually-relevant notes whose titles lack the words).
    const LOCAL_FILTER: bool = false;

    fn submit(row: &SemanticHit, tx: &AppTx) {
        if let FileListEntry::Note { path, .. } = &row.entry {
            tx.send(AppMessage::open(path.clone())).ok();
        }
    }

    fn context_event(row: &SemanticHit) -> Option<AppMessage> {
        match &row.entry {
            FileListEntry::Note { path, .. } => {
                Some(AppMessage::FileOp(FileOp::ShowMenu(path.clone())))
            }
            _ => None,
        }
    }

    fn hints() -> Vec<(String, String)> {
        vec![("Enter".into(), "Open".into())]
    }
}

/// The SEMANTIC drawer view: type a query, see the notes the RAG server ranks
/// most similar. Only meaningful when a server is configured + reachable.
/// Alt+Right / Alt+Left step a preview of the selected note, scrolled to the
/// matched section with that section highlighted (the Sources reader's
/// preview, over the same [`PreviewPane`]).
pub struct SemanticPanel {
    vault: Arc<NoteVault>,
    settings: SharedSettings,
    body: QueryListPanel<SemanticSpec>,
    source_installed: bool,
    preview: PreviewPane,
    /// The note the preview shows. `None` until a preview first opens.
    loaded: Option<LoadedNote>,
    /// Where spawned note loads leave their text (stale ones are dropped on
    /// read: their path is no longer the loaded one).
    inbox: Inbox,
    /// The app channel, kept from the last input so a load that a render
    /// starts can still wake the render loop when it lands.
    tx: Option<AppTx>,
    /// The row chevron and its Alt+Right / Alt+Left chords. The engine
    /// intercepts the chords, so they work while typing.
    expand: ExpandControl,
}

/// Where the preview's note text stands.
enum PreviewText {
    /// The note load is in flight.
    Loading,
    /// The note text, and its matched section's byte range, if found.
    Loaded {
        text: String,
        highlight: Option<Range<usize>>,
    },
    /// The note load failed.
    Failed,
}

/// The note behind the preview: the hit it was read for (its path and the
/// matched section that locates the highlight) and its text.
struct LoadedNote {
    path: VaultPath,
    ordinal: usize,
    heading: String,
    chunk: String,
    text: PreviewText,
}

/// Note loads that have landed, `(path, text)`, until the panel reads them.
type Inbox = Arc<Mutex<Vec<(VaultPath, Option<String>)>>>;

/// The matched section of a hit in its note's `text`, as the Sources reader
/// locates it. A hit without an ordinal or a heading has no section: the
/// preview opens the note from the top.
fn section_of(text: &str, heading: &str, chunk: &str, ordinal: usize) -> Option<Range<usize>> {
    if ordinal == 0 || heading.trim().is_empty() {
        return None;
    }
    locate::section_range(text, heading, chunk)
}

impl SemanticPanel {
    pub fn new(
        vault: Arc<NoteVault>,
        settings: SharedSettings,
        icons: Icons,
        yank_combos: Vec<KeyCombo>,
    ) -> Self {
        let expand = ExpandControl::new();
        let mut body = QueryListPanel::new(icons, yank_combos);
        body.set_intercept(expand.combos().to_vec());
        Self {
            vault,
            settings,
            body,
            source_installed: false,
            preview: PreviewPane::new(),
            loaded: None,
            inbox: Arc::new(Mutex::new(Vec::new())),
            tx: None,
            expand,
        }
    }

    /// Installs the server-backed source the first time the view is opened.
    pub fn ensure_source(&mut self, tx: &AppTx) {
        self.tx = Some(tx.clone());
        if !self.source_installed {
            self.body.set_source(
                SemanticSource::new(self.vault.clone(), self.settings.clone()),
                tx,
            );
            self.source_installed = true;
        }
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        let mut hints = self.body.hint_shortcuts();
        hints.extend(
            self.expand
                .hint(&self.preview, self.body.selected_row().is_some()),
        );
        hints
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        self.tx = Some(tx.clone());
        self.poll_load();
        self.sync_preview();
        let state = match event {
            InputEvent::Key(key) => self.handle_key(event, key, tx),
            InputEvent::Mouse(mouse) => self.handle_mouse(event, mouse, tx),
            _ => self.body.handle_input(event, tx),
        };
        self.sync_preview();
        self.ensure_loaded();
        state
    }

    fn handle_key(&mut self, event: &InputEvent, key: &KeyEvent, tx: &AppTx) -> EventState {
        // Full hides the list: Up/Down scroll the preview instead (as FIND).
        if self.preview.is_full() && key.modifiers.is_empty() {
            match key.code {
                KeyCode::Up => {
                    self.preview.scroll_up();
                    return EventState::Consumed;
                }
                KeyCode::Down => {
                    self.preview.scroll_down();
                    return EventState::Consumed;
                }
                _ => {}
            }
        }
        let state = self.body.handle_input(event, tx);
        match self.body.take_host_event() {
            Some(HostEvent::Intercepted(c)) if let Some(dir) = self.expand.dir_of(&c) => {
                self.step(dir.into());
                EventState::Consumed
            }
            _ => state,
        }
    }

    /// The chevrons step the preview and never open the note; in Full a
    /// click on the header title collapses it (as FIND). The wheel over the
    /// preview scrolls its content; everything else is the list's.
    fn handle_mouse(&mut self, event: &InputEvent, mouse: &MouseEvent, tx: &AppTx) -> EventState {
        if self.preview.is_full() {
            if let Some(hit) = self.expand.hit_header(&self.preview, mouse) {
                self.step(hit.step());
                return EventState::Consumed;
            }
        } else if self.expand.hit_row(mouse) {
            self.step(Step::Grow);
            return EventState::Consumed;
        }
        let state = self.body.handle_input(event, tx);
        match self.body.take_host_event() {
            Some(HostEvent::ContentScrollUp) => self.preview.scroll_up(),
            Some(HostEvent::ContentScrollDown) => self.preview.scroll_down(),
            _ => {}
        }
        state
    }

    /// The one path every expand step takes (keys, clicks).
    fn step(&mut self, step: Step) {
        if self.preview.step(step, self.selected_path()) {
            self.after_step();
        }
    }

    /// Drop the last frame's regions: the layout changes with the state, so
    /// until the next render no click or wheel may land on them.
    fn after_step(&mut self) {
        self.expand.clear();
        self.body.set_list_rect(Rect::default());
        self.body.set_content_rect(Rect::default());
    }

    fn selected_path(&self) -> Option<VaultPath> {
        self.body.selected_row().map(|hit| hit.entry.path().clone())
    }

    /// Keep the preview on the selected row (Context stays open across
    /// moves, Full collapses: see [`PreviewPane::sync`]).
    fn sync_preview(&mut self) {
        if self.preview.sync(self.selected_path()) {
            self.after_step();
        }
    }

    /// Back an open preview with the selected hit's note: the same section
    /// does nothing, another section of the loaded note moves the highlight,
    /// another note starts a load.
    fn ensure_loaded(&mut self) {
        if self.preview.is_collapsed() {
            // Drop the text so a reopen after an edit loads fresh.
            self.loaded = None;
            return;
        }
        let Some(hit) = self.body.selected_row() else {
            return;
        };
        let path = hit.entry.path().clone();
        let (ordinal, heading, chunk) = (hit.ordinal, hit.heading.clone(), hit.chunk.clone());
        if let Some(l) = &mut self.loaded
            && l.path == path
        {
            if l.ordinal != ordinal || l.heading != heading || l.chunk != chunk {
                if let PreviewText::Loaded { text, highlight } = &mut l.text {
                    *highlight = section_of(text, &heading, &chunk, ordinal);
                }
                (l.ordinal, l.heading, l.chunk) = (ordinal, heading, chunk);
                self.preview.re_anchor();
            }
            return;
        }
        self.loaded = Some(LoadedNote {
            path: path.clone(),
            ordinal,
            heading,
            chunk,
            text: PreviewText::Loading,
        });
        let vault = self.vault.clone();
        let inbox = self.inbox.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let text = vault.get_note_text(&path).await.ok();
            inbox.lock().unwrap().push((path, text));
            if let Some(tx) = tx {
                tx.send(AppMessage::Redraw).ok();
            }
        });
    }

    /// Take the landed note loads; only the loaded path's is kept.
    fn poll_load(&mut self) {
        let landed = std::mem::take(&mut *self.inbox.lock().unwrap());
        for (path, text) in landed {
            let Some(l) = &mut self.loaded else {
                continue;
            };
            if l.path != path {
                continue;
            }
            l.text = match text {
                Some(text) => {
                    let highlight = section_of(&text, &l.heading, &l.chunk, l.ordinal);
                    PreviewText::Loaded { text, highlight }
                }
                None => PreviewText::Failed,
            };
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        // New rows can move the selection: apply them before the preview
        // follows the selection, so this frame's rows and preview agree.
        self.body.poll();
        self.poll_load();
        self.sync_preview();
        self.ensure_loaded();
        // Each frame records the regions it draws anew.
        self.body.set_content_rect(Rect::default());
        self.preview.clear_header();
        self.expand.clear();

        let block = panel_block(SemanticSpec::TITLE, theme, focused);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        if self.preview.is_collapsed() || self.body.selected_row().is_none() {
            self.body.render_in(f, inner, rect, theme, focused);
            self.paint_chevron(f, theme);
            return;
        }

        // The search box stays on top; the list and the preview share the
        // rest. Full gives the preview all of it.
        let input = inner.height.min(3);
        let results = inner.height - input;
        let list = if self.preview.is_full() {
            0
        } else {
            (results / 2).max(1).min(results)
        };
        let top = Rect {
            height: input + list,
            ..inner
        };
        self.body.render_in(f, top, rect, theme, focused);
        let mut below = Rect {
            y: top.bottom(),
            height: inner.height - top.height,
            ..inner
        };
        if self.preview.is_full() {
            // The wheel scrolls the content from anywhere in the panel.
            self.body.set_content_rect(rect);
            self.render_preview(f, below, true, theme);
            return;
        }
        self.paint_chevron(f, theme);
        if below.height > 0 {
            let divider = Rect { height: 1, ..below };
            f.render_widget(
                Paragraph::new("\u{2500}".repeat(divider.width as usize)).style(
                    Style::default()
                        .fg(theme.gray.to_ratatui())
                        .bg(theme.bg_panel.to_ratatui()),
                ),
                divider,
            );
            below.y += 1;
            below.height -= 1;
        }
        self.render_preview(f, below, false, theme);
        self.body.set_content_rect(below);
    }

    /// The expand chevron ends the selected row, where the list is drawn.
    fn paint_chevron(&mut self, f: &mut Frame, theme: &Theme) {
        let row = self.body.selected_row_rect();
        self.expand.paint(f, row, &self.preview, theme);
    }

    /// Feed the loaded note into the preview (Context or Full), or show the
    /// load's placeholder.
    fn render_preview(&mut self, f: &mut Frame, area: Rect, full: bool, theme: &Theme) {
        let (title, filename) = match self.body.selected_row() {
            Some(SemanticHit {
                entry: FileListEntry::Note { filename, .. },
                title,
                ..
            }) => (title.trim().to_string(), filename.clone()),
            _ => (String::new(), String::new()),
        };
        let selected = self.selected_path();
        let Self {
            loaded, preview, ..
        } = self;
        // Only the selected note's text is painted; another note's shows as
        // still loading.
        let text = loaded
            .as_ref()
            .filter(|l| selected.as_ref() == Some(&l.path))
            .map(|l| &l.text);
        match text {
            Some(PreviewText::Loaded { text, highlight }) => {
                let highlight = Highlight::Range(highlight.as_ref());
                if full {
                    preview.render_full(f, area, &title, &filename, text, highlight, theme);
                } else {
                    preview.render_context(f, area, text, highlight, theme);
                }
            }
            Some(PreviewText::Failed) => {
                let red = Style::default().fg(theme.red.to_ratatui());
                f.render_widget(Paragraph::new("failed to load note").style(red), area);
            }
            None | Some(PreviewText::Loading) => {
                let dim = Style::default().fg(theme.gray.to_ratatui());
                f.render_widget(Paragraph::new("loading\u{2026}").style(dim), area);
            }
        }
    }

    #[cfg(test)]
    fn loaded_text(&self) -> Option<&str> {
        match self.loaded.as_ref().map(|l| &l.text) {
            Some(PreviewText::Loaded { text, .. }) => Some(text),
            _ => None,
        }
    }

    #[cfg(test)]
    fn loaded_highlight(&self) -> Option<Range<usize>> {
        match self.loaded.as_ref().map(|l| &l.text) {
            Some(PreviewText::Loaded { highlight, .. }) => highlight.clone(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::VaultConfig;
    use tempfile::TempDir;

    fn chunk(path: &str, title: &str, score: f64) -> ChunkResult {
        ChunkResult {
            path: path.to_string(),
            title: title.to_string(),
            date: None,
            content: String::new(),
            hash: String::new(),
            similarity_score: score,
            ordinal: 0,
        }
    }

    #[tokio::test]
    async fn dedups_by_note_keeping_first_and_preserves_order() {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(crate::test_support::sys(dir.path())))
            .await
            .unwrap();

        // Two chunks of "/a.md" (best first) then one of "/b.md".
        let chunks = vec![
            chunk("/a.md", "A > Intro", 0.9),
            chunk("/a.md", "A > Details", 0.7),
            chunk("/b.md", "B", 0.5),
        ];
        let entries = chunks_to_entries(chunks, &vault);
        assert_eq!(entries.len(), 2);
        match (&entries[0], &entries[1]) {
            (
                FileListEntry::Note {
                    path: pa,
                    title: ta,
                    ..
                },
                FileListEntry::Note { path: pb, .. },
            ) => {
                assert_eq!(pa.to_string(), "/a.md");
                assert_eq!(ta, "A > Intro"); // the higher-ranked chunk won
                assert_eq!(pb.to_string(), "/b.md");
            }
            _ => panic!("expected note entries"),
        }
    }

    fn hit(title: &str, journal_date: Option<&str>, score: f64) -> SemanticHit {
        SemanticHit {
            entry: FileListEntry::Note {
                path: kimun_core::nfs::VaultPath::note_path_from("a.md"),
                title: if title.is_empty() {
                    "<no title>".into()
                } else {
                    title.into()
                },
                filename: "a.md".into(),
                journal_date: journal_date.map(str::to_string),
                is_open: false,
            },
            title: title.into(),
            heading: title.into(),
            chunk: "the matched chunk".into(),
            ordinal: 1,
            score,
        }
    }

    fn render_text(row: &SemanticHit) -> String {
        use crate::components::search_list::SearchRow;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::widgets::List;
        let item = row.to_list_item(&Theme::default(), &Icons::new(false), false);
        let mut term = Terminal::new(TestBackend::new(40, 4)).unwrap();
        term.draw(|f| f.render_widget(List::new(vec![item]), f.area()))
            .unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_row_shows_the_title_the_score_and_the_filename() {
        let text = render_text(&hit("Intro", None, 0.874));
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].ends_with("Intro \u{b7} 87%"), "{text}");
        assert_eq!(lines[1], "  a.md");
    }

    #[test]
    fn a_blank_title_shows_the_file_name() {
        let text = render_text(&hit("", None, 0.5));
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].ends_with("a.md \u{b7} 50%"), "{text}");
        assert!(!text.contains("<no title>"), "{text}");
    }

    #[test]
    fn a_journal_hit_shows_the_stripped_title_and_its_date_line() {
        let text = render_text(&hit("Afternoon", Some("Wednesday, April 8, 2026"), 0.9));
        assert!(
            text.lines()
                .next()
                .unwrap()
                .ends_with("Afternoon \u{b7} 90%"),
            "{text}"
        );
        assert!(text.contains("Wednesday, April 8, 2026"), "{text}");
        assert!(!text.contains("2026-04-08Afternoon"), "{text}");
    }

    #[test]
    fn a_hit_row_is_as_tall_as_its_note_row() {
        use crate::components::search_list::SearchRow;
        assert_eq!(hit("A", None, 0.5).visual_height(), 2);
        assert_eq!(hit("A", Some("d"), 0.5).visual_height(), 3);
    }

    #[test]
    fn enter_opens_the_note_and_right_click_opens_its_file_menu() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let row = hit("Intro", None, 0.8);
        SemanticSpec::submit(&row, &tx);
        match rx.try_recv().unwrap() {
            AppMessage::OpenPath { path, .. } => {
                assert_eq!(path, kimun_core::nfs::VaultPath::note_path_from("a.md"))
            }
            other => panic!("unexpected {other:?}"),
        }
        match SemanticSpec::context_event(&row) {
            Some(AppMessage::FileOp(FileOp::ShowMenu(path))) => {
                assert_eq!(path, kimun_core::nfs::VaultPath::note_path_from("a.md"))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    // ── The preview ───────────────────────────────────────────────────

    use kimun_core::nfs::VaultPath;
    use ratatui::buffer::Buffer;
    use ratatui::crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use tokio::sync::mpsc::UnboundedReceiver;

    /// Filler above the matched section, so a Context preview must scroll
    /// to show it.
    fn note_text() -> String {
        let mut text = String::from("# Top\n");
        for i in 0..40 {
            text.push_str(&format!("filler line {i}\n"));
        }
        text.push_str("\n## Details\nthe matched chunk body\n\n## Tail\nlast words\n");
        text
    }

    /// A hit on the note `name`, matched in its `Details` section.
    fn note_hit(name: &str, heading: &str, ordinal: usize) -> SemanticHit {
        let path = VaultPath::note_path_from(name);
        let filename = path.get_parent_path().1;
        SemanticHit {
            entry: FileListEntry::Note {
                path,
                title: heading.into(),
                filename,
                journal_date: None,
                is_open: false,
            },
            title: heading.into(),
            heading: heading.into(),
            chunk: "the matched chunk body".into(),
            ordinal,
            score: 0.8,
        }
    }

    /// A panel over `hits`, whose notes all hold [`note_text`], with the
    /// first hit selected.
    async fn panel_with(
        tag: &str,
        hits: Vec<SemanticHit>,
    ) -> (SemanticPanel, AppTx, UnboundedReceiver<AppMessage>) {
        let vault = crate::test_support::temp_vault(tag).await;
        vault.validate_and_init().await.unwrap();
        for hit in &hits {
            if let FileListEntry::Note { path, .. } = &hit.entry {
                vault.create_note(path, &note_text()).await.unwrap();
            }
        }
        let settings = crate::settings::AppSettings::default();
        let mut panel = SemanticPanel::new(
            vault,
            std::sync::Arc::new(std::sync::RwLock::new(settings.clone())),
            settings.icons(),
            settings.yank_combos(),
        );
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        panel.body.set_rows(hits, &tx);
        (panel, tx, rx)
    }

    fn alt(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent::new(code, KeyModifiers::ALT))
    }

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn mouse(kind: MouseEventKind, x: u16, y: u16) -> InputEvent {
        InputEvent::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn click(x: u16, y: u16) -> InputEvent {
        mouse(MouseEventKind::Down(MouseButton::Left), x, y)
    }

    fn draw(panel: &mut SemanticPanel, w: u16, h: u16) -> Buffer {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::default();
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| panel.render(f, f.area(), &theme, true))
            .unwrap();
        term.backend().buffer().clone()
    }

    fn cells(buf: &Buffer, glyph: &str) -> Vec<(u16, u16)> {
        let a = buf.area;
        (a.y..a.bottom())
            .flat_map(|y| (a.x..a.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| buf[(x, y)].symbol() == glyph)
            .collect()
    }

    fn opened(rx: &mut UnboundedReceiver<AppMessage>) -> bool {
        crate::test_support::drain(rx)
            .iter()
            .any(|m| matches!(m, AppMessage::OpenPath { .. }))
    }

    /// Wait until the selected hit's note text has landed.
    async fn settle_load(panel: &mut SemanticPanel) {
        for _ in 0..200 {
            panel.poll_load();
            if panel.loaded_text().is_some() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("the note text never loaded");
    }

    /// The byte range of the `Details` section's chunk in [`note_text`].
    fn details_range() -> std::ops::Range<usize> {
        let text = note_text();
        let start = text.find("the matched chunk body").unwrap();
        start..start + "the matched chunk body".len()
    }

    /// Alt+Right grows the preview one step, Alt+Left shrinks it one step,
    /// and neither touches the query text.
    #[tokio::test(flavor = "multi_thread")]
    async fn alt_arrows_step_the_preview() {
        let (mut panel, tx, _rx) = panel_with("sem-alt", vec![note_hit("a", "Details", 1)]).await;
        assert!(panel.preview.is_collapsed());
        assert_eq!(
            panel.handle_input(&alt(KeyCode::Right), &tx),
            EventState::Consumed
        );
        assert!(panel.preview.is_context(), "Collapsed -> Context");
        panel.handle_input(&alt(KeyCode::Right), &tx);
        assert!(panel.preview.is_full(), "Context -> Full");
        panel.handle_input(&alt(KeyCode::Right), &tx);
        assert!(panel.preview.is_full(), "Full stays Full, no wrap");
        panel.handle_input(&alt(KeyCode::Left), &tx);
        assert!(panel.preview.is_context(), "Full -> Context");
        panel.handle_input(&alt(KeyCode::Left), &tx);
        assert!(panel.preview.is_collapsed(), "Context -> Collapsed");
        panel.handle_input(&alt(KeyCode::Left), &tx);
        assert!(panel.preview.is_collapsed(), "stops at Collapsed");
        assert_eq!(panel.body.list().unwrap().query(), "");
    }

    /// Collapsing drops the loaded text, so a reopen after an edit reads the
    /// note again.
    #[tokio::test(flavor = "multi_thread")]
    async fn collapsing_drops_the_loaded_text() {
        let (mut panel, tx, _rx) = panel_with("sem-drop", vec![note_hit("a", "Details", 1)]).await;
        panel.handle_input(&alt(KeyCode::Right), &tx);
        settle_load(&mut panel).await;
        assert!(panel.loaded_text().is_some());
        panel.handle_input(&alt(KeyCode::Left), &tx);
        draw(&mut panel, 40, 30);
        assert_eq!(panel.loaded_text(), None, "text dropped on collapse");
        assert!(panel.loaded.is_none());
    }

    /// The Context preview opens on the hit's note with its matched section
    /// as the highlight, and scrolls to it.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_preview_opens_on_the_matched_section() {
        let (mut panel, tx, _rx) = panel_with("sem-range", vec![note_hit("a", "Details", 1)]).await;
        panel.handle_input(&alt(KeyCode::Right), &tx);
        settle_load(&mut panel).await;
        assert_eq!(panel.loaded_text(), Some(note_text().as_str()));
        assert_eq!(panel.loaded_highlight(), Some(details_range()));
        assert_eq!(
            panel.loaded_highlight(),
            crate::ask::locate::section_range(&note_text(), "Details", "the matched chunk body")
        );
        let buf = draw(&mut panel, 40, 30);
        assert!(
            panel.preview.scroll_offset() > 0,
            "the Context preview scrolls to the section"
        );
        let shown = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(shown.contains("the matched chunk body"), "{shown}");
    }

    /// A hit with no ordinal opens the note from the top, unhighlighted.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_hit_without_an_ordinal_opens_the_note_from_the_top() {
        let (mut panel, tx, _rx) = panel_with("sem-ord0", vec![note_hit("a", "Details", 0)]).await;
        panel.handle_input(&alt(KeyCode::Right), &tx);
        settle_load(&mut panel).await;
        assert_eq!(panel.loaded_highlight(), None);
        draw(&mut panel, 40, 30);
        assert_eq!(panel.preview.scroll_offset(), 0);
    }

    /// A hit with a blank heading opens the note from the top, unhighlighted.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_hit_with_a_blank_heading_opens_the_note_from_the_top() {
        let (mut panel, tx, _rx) = panel_with("sem-blank", vec![note_hit("a", " ", 1)]).await;
        panel.handle_input(&alt(KeyCode::Right), &tx);
        settle_load(&mut panel).await;
        assert_eq!(panel.loaded_highlight(), None);
        draw(&mut panel, 40, 30);
        assert_eq!(panel.preview.scroll_offset(), 0);
    }

    /// The Full preview renders the note with the same range highlight.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_full_preview_shows_the_note_and_the_shrink_chevron() {
        let (mut panel, tx, _rx) = panel_with("sem-full", vec![note_hit("a", "Details", 1)]).await;
        panel.handle_input(&alt(KeyCode::Right), &tx);
        panel.handle_input(&alt(KeyCode::Right), &tx);
        settle_load(&mut panel).await;
        let buf = draw(&mut panel, 40, 30);
        let found = cells(&buf, "\u{2039}");
        assert_eq!(found.len(), 1, "{found:?}");
        let header = panel.preview.full_header_rect();
        assert_eq!(found[0], (header.right() - 1, header.y));
        assert!(cells(&buf, "\u{203A}").is_empty());
    }

    /// Enter still opens the note, from any preview state.
    #[tokio::test(flavor = "multi_thread")]
    async fn enter_still_opens_the_note() {
        let (mut panel, tx, mut rx) =
            panel_with("sem-enter", vec![note_hit("a", "Details", 1)]).await;
        panel.handle_input(&key(KeyCode::Enter), &tx);
        assert!(opened(&mut rx), "Enter opens while collapsed");
        panel.handle_input(&alt(KeyCode::Right), &tx);
        panel.handle_input(&key(KeyCode::Enter), &tx);
        assert!(opened(&mut rx), "Enter opens with the preview open");
    }

    /// Plain letters are query text in SEM, never preview or list verbs.
    #[tokio::test(flavor = "multi_thread")]
    async fn plain_letters_stay_query_text_in_sem() {
        let (mut panel, tx, _rx) =
            panel_with("sem-letters", vec![note_hit("a", "Details", 1)]).await;
        for ch in ['l', 'h', 'o', 'y'] {
            panel.handle_input(&key(KeyCode::Char(ch)), &tx);
        }
        assert_eq!(panel.body.list().unwrap().query(), "lhoy");
        assert!(panel.preview.is_collapsed());
    }

    /// Only the selected row ends in the grow chevron; a click on it steps
    /// the preview and never opens the note. The Full header's chevron
    /// shrinks one step.
    #[tokio::test(flavor = "multi_thread")]
    async fn clicking_the_chevron_steps_the_preview_without_opening() {
        let (mut panel, tx, mut rx) = panel_with(
            "sem-chev",
            vec![note_hit("a", "Details", 1), note_hit("b", "Details", 1)],
        )
        .await;
        let buf = draw(&mut panel, 40, 30);
        let found = cells(&buf, "\u{203A}");
        assert_eq!(
            found.len(),
            1,
            "one chevron, on the selected row: {found:?}"
        );
        let rect = panel.body.selected_row_rect().unwrap();
        assert_eq!(found[0], (rect.right() - 1, rect.y));

        panel.handle_input(&click(found[0].0, found[0].1), &tx);
        assert!(panel.preview.is_context(), "click grows the preview");
        let buf = draw(&mut panel, 40, 30);
        let (x, y) = cells(&buf, "\u{203A}")[0];
        panel.handle_input(&click(x, y), &tx);
        assert!(panel.preview.is_full(), "second click grows to Full");
        assert!(!opened(&mut rx), "the chevron never opens the note");

        let buf = draw(&mut panel, 40, 30);
        let (x, y) = cells(&buf, "\u{2039}")[0];
        panel.handle_input(&click(x, y), &tx);
        assert!(
            panel.preview.is_context(),
            "header chevron: Full -> Context"
        );
        assert!(!opened(&mut rx));
    }

    /// The wheel over the Context preview scrolls its content and leaves
    /// the list where it is.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_wheel_scrolls_the_preview_not_the_list() {
        let (mut panel, tx, _rx) = panel_with(
            "sem-wheel",
            vec![note_hit("a", "Details", 0), note_hit("b", "Details", 0)],
        )
        .await;
        panel.handle_input(&alt(KeyCode::Right), &tx);
        settle_load(&mut panel).await;
        draw(&mut panel, 40, 30);
        let before = panel.body.selected_row().map(|h| h.entry.path().clone());
        assert_eq!(panel.preview.scroll_offset(), 0);
        // The bottom inner row is the preview's.
        panel.handle_input(&mouse(MouseEventKind::ScrollDown, 10, 28), &tx);
        draw(&mut panel, 40, 30);
        assert!(panel.preview.scroll_offset() > 0, "the preview scrolled");
        assert_eq!(
            panel.body.selected_row().map(|h| h.entry.path().clone()),
            before,
            "the list selection did not move"
        );
        assert!(panel.preview.is_context());
    }

    /// The server's answer per query: `a` for a blank query, `b` otherwise.
    struct SwapSource;
    #[async_trait::async_trait]
    impl crate::components::search_list::RowSource<SemanticHit> for SwapSource {
        async fn load(&self, q: &str, emit: crate::components::search_list::Emit<SemanticHit>) {
            let name = if q.is_empty() { "a" } else { "b" };
            emit.replace(vec![note_hit(name, "Details", 0)]);
        }
    }

    fn screen(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// New results under an open Context preview move the selection to
    /// another note: the frame that applies them never paints the old
    /// note's text, and the new note's text shows once it loads.
    #[tokio::test(flavor = "multi_thread")]
    async fn new_results_never_paint_the_old_notes_text() {
        let (mut panel, tx, _rx) = panel_with("sem-swap", Vec::new()).await;
        for (name, body) in [("a", "alpha body text"), ("b", "bravo body text")] {
            panel
                .vault
                .create_note(&VaultPath::note_path_from(name), body)
                .await
                .unwrap();
        }
        panel.body.set_source(SwapSource, &tx);
        panel.body.list_mut().unwrap().poll_until_idle().await;
        panel.handle_input(&alt(KeyCode::Right), &tx);
        settle_load(&mut panel).await;
        let shown = screen(&draw(&mut panel, 40, 30));
        assert!(shown.contains("alpha body text"), "{shown}");

        // The new rows land unpolled; the next frame applies them.
        panel.body.list_mut().unwrap().set_query("more");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let shown = screen(&draw(&mut panel, 40, 30));
        assert_eq!(
            panel.body.selected_row().map(|h| h.entry.path().clone()),
            Some(VaultPath::note_path_from("b")),
            "the new rows were applied"
        );
        assert!(!shown.contains("alpha body text"), "A under B:\n{shown}");

        settle_load(&mut panel).await;
        assert_eq!(panel.loaded_text(), Some("bravo body text"));
        let shown = screen(&draw(&mut panel, 40, 30));
        assert!(shown.contains("bravo body text"), "{shown}");
        assert!(!shown.contains("alpha body text"), "{shown}");
    }

    /// With no rows the preview keys and the list keys do nothing, and no
    /// chevron shows.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_list_has_no_chevron_and_inert_keys() {
        let (mut panel, tx, _rx) = panel_with("sem-empty", Vec::new()).await;
        let buf = draw(&mut panel, 40, 30);
        for code in [KeyCode::Right, KeyCode::Left] {
            panel.handle_input(&alt(code), &tx);
        }
        for code in [KeyCode::Up, KeyCode::Down] {
            panel.handle_input(&key(code), &tx);
        }
        assert!(panel.preview.is_collapsed());
        let after = draw(&mut panel, 40, 30);
        for b in [&buf, &after] {
            assert!(cells(b, "\u{203A}").is_empty());
            assert!(cells(b, "\u{2039}").is_empty());
        }
    }

    /// The hint row names Enter Open and the chevron's key for the state.
    #[tokio::test(flavor = "multi_thread")]
    async fn hints_name_open_and_the_preview_keys() {
        let (mut panel, tx, _rx) = panel_with("sem-hints", vec![note_hit("a", "Details", 1)]).await;
        let has = |p: &SemanticPanel, k: &str| p.hint_shortcuts().iter().any(|(key, _)| key == k);
        assert!(has(&panel, "Enter"));
        assert!(has(&panel, "Alt+\u{2192}"));
        panel.handle_input(&alt(KeyCode::Right), &tx);
        panel.handle_input(&alt(KeyCode::Right), &tx);
        assert!(has(&panel, "Alt+\u{2190}"));
        assert!(!has(&panel, "Alt+\u{2192}"));
        assert!(has(&panel, "Enter"));
    }

    #[test]
    fn yank_and_match_text_are_the_note_rows() {
        use crate::logic::list::ListRow;
        let row = hit("Intro", None, 0.8);
        assert_eq!(row.match_text(), Some("a.md"));
        assert!(row.yank_target().is_some());
    }
}
