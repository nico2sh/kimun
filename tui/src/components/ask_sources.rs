//! `SourcesPanel` — the Ask workspace's drawer view (CONTEXT.md: **Sources
//! view**, **Source reader**; adr/0030): a ranked per-turn source list that
//! reveals the full note — the retrieved section highlighted — in an inline
//! preview, without leaving the answer.
//!
//! Composes the shared list engine ([`SearchList`]) the same way the FIND
//! drawer (`query_panel.rs`) does — the panel no longer hand-rolls
//! `List`/`ListState`, a cursor, selection styling, plain-letter matching, or a
//! chord pre-intercept. The engine owns navigation, the (new) filter input,
//! selection, list scroll, the list-focus verbs (`l`/`h`/`o`/`y`), the
//! FollowLink / `Ctrl+Y` intercepts, and mouse hit-testing; on top of it the
//! panel composes the shared [`PreviewPane`] reveal (the **Source reader**) and
//! the per-turn note-load lifecycle.
//!
//! Unlike FIND (which opens on its query input), the Sources view opens on the
//! list ([`Focus::List`]) — the first production user of `opening_focus`. Its
//! rows are per-turn in-memory sources, so it composes `SearchList` directly
//! (over an in-memory [`RowSource`]) rather than through `QueryListPanel`:
//! `QueryListPanel` is a bare list with no `PreviewPane` and it swallows the
//! `ListVerb`/`Intercepted` reactions the reveal is driven by.
//!
//! The per-turn sources live in the engine's row set (there is no parallel
//! copy): `set_turn`/`refresh`/`reset` rebuild the engine over the turn's rows,
//! and the directed reveals (`open_reader`/`focus_source`) and note-load
//! resolution all read them back through the engine.

use std::ops::Range;
use std::sync::{Arc, Mutex};

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{ListItem, Paragraph};

use crate::ask::{AskSource, locate};
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, AskData, InputEvent};
use crate::components::panel::panel_block;
use crate::components::preview_pane::{Highlight, PreviewPane};
use crate::components::rich_row::RichRow;
use crate::components::search_list::{
    Emit, Filter, Focus, KeyReaction, RowSource, SearchList, SearchMouse, SearchRow,
};
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::keys::key_combo::KeyCombo;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// Rows a PageUp/PageDown leaves visible from the previous view (shared
/// convention with the note preview and the Ask thread).
const PAGE_OVERLAP: u16 = 2;

/// The load state for the currently-anchored source's note text.
enum ReaderContent {
    /// The note load is in flight.
    Loading,
    /// The note loaded successfully. `highlight` is the byte range
    /// `locate::section_range` resolved, if any.
    Loaded {
        text: String,
        highlight: Option<Range<usize>>,
    },
    /// The note load failed.
    Failed,
}

/// The async note load backing the preview. Keyed by `path` for stale-drop of
/// an in-flight vault load (a selection change, or a new turn, before the load
/// lands must not clobber the note anchored now), and additionally by the
/// source `ordinal` so that selecting a *different section of the same note*
/// re-resolves the highlight against the new heading without a refetch.
struct LoadedNote {
    path: VaultPath,
    /// The anchored source's citation ordinal — the section identity within the
    /// note. Distinguishes two sources sharing a `path` but a different heading.
    ordinal: usize,
    content: ReaderContent,
}

/// One list-engine row: a per-turn source plus its 1-based rank (its position
/// in the turn's ranked list — kept on the row so it survives filtering). The
/// [`SearchRow`] bridge draws it as the shared [`RichRow`] and exposes the
/// heading + path as the fuzzy-filter haystack.
#[derive(Clone)]
struct SourceRow {
    rank: usize,
    source: AskSource,
    /// The `heading path` haystack the list's `Filter::Fuzzy` matches, so the
    /// new filter input narrows the turn's sources by heading or path text.
    filter_text: String,
}

impl SourceRow {
    fn new(rank: usize, source: AskSource) -> Self {
        let filter_text = format!("{} {}", source.heading, source.path);
        Self {
            rank,
            source,
            filter_text,
        }
    }
}

impl SearchRow for SourceRow {
    fn to_list_item(&self, theme: &Theme, _icons: &Icons, _selected: bool) -> ListItem<'static> {
        source_row(self.rank, &self.source, theme).into_list_item(theme)
    }

    fn visual_height(&self) -> u16 {
        // Title line + dim filename line (the date is inline on the title).
        2
    }

    fn match_text(&self) -> Option<&str> {
        Some(&self.filter_text)
    }
}

/// The in-memory [`RowSource`] for one turn: it delivers the fixed source rows
/// once and never reloads on the query — the query is a *local* fuzzy filter
/// over the loaded rows (`reload_on_query() == false`).
struct TurnSource {
    rows: Vec<SourceRow>,
}

#[async_trait::async_trait]
impl RowSource<SourceRow> for TurnSource {
    async fn load(&self, _query: &str, emit: Emit<SourceRow>) {
        emit.replace(self.rows.clone());
    }
    fn reload_on_query(&self) -> bool {
        false
    }
}

/// The Ask workspace's Sources drawer view: a ranked source list (on the shared
/// [`SearchList`]) with the shared [`PreviewPane`] revealing the selected
/// source's note below/over it.
pub struct SourcesPanel {
    turn_id: Option<u64>,
    /// The shared list engine: query/filter input, result list, selection,
    /// list scroll, list-focus verbs and intercepts. Rebuilt per turn over a
    /// fresh in-memory [`TurnSource`].
    list: SearchList<SourceRow>,
    /// The note-preview surface (expand cycle + content scroll + content
    /// render), shared with the FIND drawer. Anchored by the located section
    /// byte range as the highlight.
    preview: PreviewPane,
    /// The note load for the currently-anchored source. `None` until a preview
    /// first opens.
    loaded: Option<LoadedNote>,
    /// Vault handle for the preview's note load (`ensure_note_load` spawns a
    /// `vault.get_note_text`). Owned here so `handle_input` needs no vault
    /// passed in.
    vault: Arc<NoteVault>,
    icons: Icons,
    /// Combos the engine intercepts: FollowLink (open) plus `Ctrl+Y` (yank).
    /// Registered on every rebuilt list.
    intercept: Vec<KeyCombo>,
    /// The `Ctrl+Y` combo, kept to route an [`KeyReaction::Intercepted`] to
    /// yank (any other intercepted combo is a FollowLink → open).
    ctrl_y_combo: Option<KeyCombo>,
    /// Shared sender, filled the first time a `tx` arrives; the engine's redraw
    /// callback reads it so an async row load wakes the render loop.
    redraw_tx: Arc<Mutex<Option<AppTx>>>,
    /// The preview content viewport height from the last render — the page size
    /// for PageUp/PageDown content scrolling in the Full preview.
    preview_page: u16,
}

impl SourcesPanel {
    pub fn new(vault: Arc<NoteVault>, key_bindings: &KeyBindings) -> Self {
        let map = key_bindings.to_hashmap();
        let follow = map.get(&ActionShortcuts::FollowLink).cloned().unwrap_or_default();
        let ctrl_y_combo = crate::keys::key_event_to_combo(&KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::CONTROL,
        ));
        let mut intercept = follow;
        if let Some(c) = ctrl_y_combo {
            intercept.push(c);
        }
        let icons = Icons::new(false);
        let redraw_tx: Arc<Mutex<Option<AppTx>>> = Arc::new(Mutex::new(None));
        let list = build_list(Vec::new(), &intercept, &icons, &redraw_tx);
        Self {
            turn_id: None,
            list,
            preview: PreviewPane::new(),
            loaded: None,
            vault,
            icons,
            intercept,
            ctrl_y_combo,
            redraw_tx,
            preview_page: 0,
        }
    }

    /// Repopulates the list for `turn_id` and collapses the preview. A repeated
    /// call with the same `turn_id` is a no-op — it keeps the selection (and the
    /// preview state) exactly as-is when a selection sync re-points the drawer
    /// at the already-shown turn. Regeneration replaces a turn's sources with
    /// the fresh ones on completion, but that goes through
    /// [`refresh`](Self::refresh) (which never short-circuits), not here.
    pub fn set_turn(&mut self, turn_id: u64, sources: Vec<AskSource>) {
        if self.turn_id == Some(turn_id) {
            return;
        }
        self.refresh(turn_id, sources);
    }

    /// Force the source list for `turn_id` to `sources`, even when it's the
    /// turn already shown — the answer-completion path, where a `Thinking`
    /// turn (empty sources) gains its sources once the answer lands. Unlike
    /// [`set_turn`](Self::set_turn), it never short-circuits on a matching id.
    /// Collapses the preview and resets to the top.
    pub fn refresh(&mut self, turn_id: u64, sources: Vec<AskSource>) {
        self.turn_id = Some(turn_id);
        self.rebuild_list(sources);
        self.preview.reset();
        self.loaded = None;
    }

    /// Clear the panel back to its empty, collapsed state — the "new
    /// conversation" action (leader `a n`) drops the old turn's sources.
    pub fn reset(&mut self) {
        self.turn_id = None;
        self.rebuild_list(Vec::new());
        self.preview.reset();
        self.loaded = None;
    }

    /// (Re)build the list engine over `sources` (rank = 1-based position), the
    /// engine-per-turn pattern: the turn's rows live only in the engine.
    fn rebuild_list(&mut self, sources: Vec<AskSource>) {
        let rows: Vec<SourceRow> = sources
            .into_iter()
            .enumerate()
            .map(|(i, s)| SourceRow::new(i + 1, s))
            .collect();
        self.list = build_list(rows, &self.intercept, &self.icons, &self.redraw_tx);
    }

    /// Whether the current turn has any sources — read from the engine's row
    /// set (the panel keeps no parallel copy).
    fn has_sources(&self) -> bool {
        !self.list.rows().is_empty()
    }

    /// The source at rank position `index` (0-based), from the engine's full
    /// (unfiltered) row set.
    fn source_at(&self, index: usize) -> Option<&AskSource> {
        self.list.rows().get(index).map(|r| &r.source)
    }

    /// Point the list selection at the source with citation `ordinal` and
    /// collapse the preview — a citation click in the thread asks the drawer to
    /// reveal that exact source in the list. This is the ordinal→row boundary:
    /// the panel lists sources in rank order, so it resolves the ordinal to a
    /// position by matching the engine's rows, never by assuming `ordinal - 1`.
    /// An ordinal with no matching source is ignored. Clears any active filter
    /// so the target is never hidden.
    pub fn focus_source(&mut self, ordinal: usize) {
        self.list.set_query("");
        self.list.poll();
        if let Some(pos) = self
            .list
            .visible_rows()
            .iter()
            .position(|r| r.source.ordinal == ordinal)
        {
            self.list.select(pos);
            self.preview.reset();
            self.loaded = None;
        }
    }

    /// Reveal `sources[source_index]` in the preview (leader `a s`): point the
    /// selection at it and make sure the preview ends *revealed* on that source.
    /// Collapsed opens to the half-height Context preview; an already-open
    /// Context/Full stays at its expand level and re-points onto the new source
    /// (never collapsing the way a plain selection move would). Spawns/refreshes
    /// the note load. No-op for an out-of-range index.
    pub fn open_reader(&mut self, source_index: usize, tx: &AppTx) {
        self.ensure_redraw_tx(tx);
        self.list.set_query("");
        self.list.poll();
        let Some(source) = self.source_at(source_index).cloned() else {
            return;
        };
        self.list.select(source_index);
        let sel = Some(source.path.clone());
        if self.preview.is_collapsed() {
            self.preview.toggle(sel); // Collapsed -> Context
        } else {
            self.preview.repoint(sel); // keep the expand level, re-anchor here
        }
        self.ensure_note_load(
            source.path.clone(),
            source.ordinal,
            source.match_heading().to_string(),
            source.text.clone(),
            tx,
        );
    }

    /// Accepts a `ReaderNote` only when the panel is currently awaiting that
    /// exact path (stale-drop: a source switch, or a new turn, before the load
    /// lands must not clobber whatever is anchored now). Any other `AskData`
    /// variant is addressed elsewhere and ignored.
    pub fn handle_data(&mut self, data: AskData) {
        let AskData::ReaderNote { path, text } = data else {
            return;
        };
        if self.loaded.as_ref().map(|l| &l.path) != Some(&path) {
            return;
        }
        // Resolve the highlight against the anchored source (prefer the one with
        // the loaded ordinal; fall back to any source with this path) — read
        // back from the engine's row set.
        let ord = self.loaded.as_ref().map(|l| l.ordinal);
        let rows = self.list.rows();
        let hl_src = rows
            .iter()
            .map(|r| &r.source)
            .find(|s| s.path == path && Some(s.ordinal) == ord)
            .or_else(|| rows.iter().map(|r| &r.source).find(|s| s.path == path))
            .map(|s| (s.match_heading().to_string(), s.text.clone()));
        let content = match text {
            Some(loaded) => {
                let highlight = hl_src
                    .and_then(|(heading, chunk)| locate::section_range(&loaded, &heading, &chunk));
                ReaderContent::Loaded {
                    text: loaded,
                    highlight,
                }
            }
            None => ReaderContent::Failed,
        };
        if let Some(l) = &mut self.loaded {
            l.content = content;
        }
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        if self.list.focus() == Focus::Input {
            return vec![
                ("Esc".into(), "list".into()),
                ("type".into(), "filter".into()),
            ];
        }
        if self.preview.is_collapsed() {
            vec![
                ("j/k".into(), "Select".into()),
                ("Enter/l".into(), "Preview".into()),
                ("o/^N".into(), "Open".into()),
                ("y".into(), "Yank".into()),
                ("i".into(), "Filter".into()),
            ]
        } else {
            vec![
                ("j/k".into(), "Select".into()),
                ("Enter/l".into(), "Expand".into()),
                ("h/Esc".into(), "Back".into()),
                ("o/^N".into(), "Open".into()),
            ]
        }
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let key = match event {
            InputEvent::Key(key) => key,
            // The mouse wheel scrolls the open preview's content or the list
            // (converged with FIND, where the engine routes the wheel).
            InputEvent::Mouse(mouse) => return self.handle_mouse(mouse, tx),
            _ => return EventState::NotConsumed,
        };
        self.ensure_redraw_tx(tx);

        // Full takes over the arrow/page keys for content scroll BEFORE the
        // engine sees them (mirrors FIND). `j`/`k` reach the engine so the list
        // cursor stays reachable under the full preview.
        if self.preview.is_full() {
            match key.code {
                KeyCode::Up => {
                    self.preview.scroll_up();
                    return EventState::Consumed;
                }
                KeyCode::Down => {
                    self.preview.scroll_down();
                    return EventState::Consumed;
                }
                KeyCode::PageUp => {
                    self.scroll_preview_page(true);
                    return EventState::Consumed;
                }
                KeyCode::PageDown => {
                    self.scroll_preview_page(false);
                    return EventState::Consumed;
                }
                _ => {}
            }
        }

        // Esc ladder: in list focus with the preview revealed, Esc steps the
        // reveal back (Full → Context → Collapsed) and is consumed; from a
        // collapsed list the engine's `Cancel` bubbles so the drawer host
        // returns focus to the thread. (In input focus, Esc first returns to
        // the list — the engine handles that.)
        if key.code == KeyCode::Esc
            && self.list.focus() == Focus::List
            && !self.preview.is_collapsed()
        {
            self.preview.collapse_step(self.selected_path());
            return EventState::Consumed;
        }

        match self.list.handle_key(key) {
            // FollowLink opens; `Ctrl+Y` yanks — the canonical chords, now via
            // the engine's intercept mechanism instead of a hand-rolled
            // pre-check. From any focus / reveal state.
            KeyReaction::Intercepted(c) => {
                if Some(c) == self.ctrl_y_combo {
                    self.yank_selected_path(tx);
                } else {
                    self.open_selected(tx);
                }
                EventState::Consumed
            }
            // Enter (with no autocomplete open) cycles the reveal, like `l`.
            KeyReaction::Submit => {
                if self.has_sources() {
                    self.preview.toggle(self.selected_path());
                    self.ensure_loaded(tx);
                }
                EventState::Consumed
            }
            // List-focus verbs: `l`/`h` cycle the reveal, `o` opens, `y` yanks.
            KeyReaction::ListVerb(c) => {
                match c {
                    'l' => {
                        if self.has_sources() {
                            self.preview.toggle(self.selected_path());
                            self.ensure_loaded(tx);
                        }
                    }
                    'h' => self.preview.collapse_step(self.selected_path()),
                    'o' => self.open_selected(tx),
                    'y' => self.yank_selected_path(tx),
                    _ => {}
                }
                EventState::Consumed
            }
            // A consumed navigation / filter keystroke: re-anchor the preview on
            // the new selection and refresh its note load (Context sticks across
            // moves, Full collapses — see [`PreviewPane::sync`]).
            KeyReaction::Consumed => {
                self.sync_preview();
                self.ensure_loaded(tx);
                EventState::Consumed
            }
            // Esc from a collapsed list bubbles so the host returns focus to the
            // thread.
            KeyReaction::Cancel | KeyReaction::Unhandled => EventState::NotConsumed,
        }
    }

    /// Route a mouse event through the engine (converged with FIND): the wheel
    /// scrolls the open preview's content (inside its region) or the list
    /// (elsewhere in the panel); a click selects a row, a second click on the
    /// selected row cycles the reveal.
    fn handle_mouse(&mut self, mouse: &MouseEvent, tx: &AppTx) -> EventState {
        self.ensure_redraw_tx(tx);
        let was_full = self.preview.is_full();
        self.sync_preview();
        // In Full the list is not rendered (its recorded rect is stale), so only
        // the wheel may reach the engine; a click on the header collapses the
        // reveal, anything else is swallowed.
        if was_full {
            match mouse.kind {
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {}
                MouseEventKind::Down(MouseButton::Left)
                    if self.preview.full_header_rect().contains(Position {
                        x: mouse.column,
                        y: mouse.row,
                    }) =>
                {
                    self.preview.toggle(self.selected_path());
                    return EventState::Consumed;
                }
                _ => return EventState::Consumed,
            }
        }
        match self.list.handle_mouse(mouse) {
            SearchMouse::ContentScrollUp => {
                self.preview.scroll_up();
                EventState::Consumed
            }
            SearchMouse::ContentScrollDown => {
                self.preview.scroll_down();
                EventState::Consumed
            }
            SearchMouse::Activated(_) => {
                self.preview.toggle(self.selected_path());
                self.ensure_loaded(tx);
                EventState::Consumed
            }
            SearchMouse::Selected(_) | SearchMouse::Scrolled | SearchMouse::Context(_) => {
                self.sync_preview();
                self.ensure_loaded(tx);
                EventState::Consumed
            }
            SearchMouse::None => EventState::NotConsumed,
        }
    }

    /// Scroll the Full preview by a page (last render's content viewport, less
    /// a small overlap), `up` toward the top. Single-tick scrolls under the
    /// hood so the anchor-takeover/clamp rules hold.
    fn scroll_preview_page(&mut self, up: bool) {
        let page = self.preview_page.saturating_sub(PAGE_OVERLAP).max(1);
        for _ in 0..page {
            if up {
                self.preview.scroll_up();
            } else {
                self.preview.scroll_down();
            }
        }
    }

    /// The selected source (read from the engine's selected row, so it is
    /// correct even under an active filter).
    fn selected_source(&self) -> Option<&AskSource> {
        self.list.selected_row().map(|r| &r.source)
    }

    /// The selected source's path, for preview anchoring and open/yank.
    fn selected_path(&self) -> Option<VaultPath> {
        self.selected_source().map(|s| s.path.clone())
    }

    /// Re-anchor the preview onto the current selection (Context sticks across
    /// moves, Full collapses — see [`PreviewPane::sync`]).
    fn sync_preview(&mut self) {
        let sel = self.selected_path();
        self.preview.sync(sel);
    }

    /// Ensure the preview is backed by the *selected* source's note (the
    /// interactive path — `open_reader` calls [`ensure_note_load`] directly with
    /// its directed source). No-op while collapsed or with nothing selected.
    fn ensure_loaded(&mut self, tx: &AppTx) {
        if self.preview.is_collapsed() {
            return;
        }
        let Some(source) = self.selected_source() else {
            return;
        };
        let path = source.path.clone();
        let ordinal = source.ordinal;
        let heading = source.match_heading().to_string();
        let chunk = source.text.clone();
        self.ensure_note_load(path, ordinal, heading, chunk, tx);
    }

    /// Ensure the preview is backed by the given source's note. Three cases,
    /// keyed on the source identity (`path` + `ordinal`), not `path` alone:
    ///
    /// - **Same source** (same path and ordinal): nothing to do.
    /// - **Same note, different section** (same path, new ordinal): reuse the
    ///   already-loaded text, re-resolve the highlight against the new heading,
    ///   and re-anchor — no vault refetch.
    /// - **Different note**: spawn the load, re-keying `loaded` so an earlier
    ///   path's late `ReaderNote` is dropped on arrival.
    fn ensure_note_load(
        &mut self,
        path: VaultPath,
        ordinal: usize,
        heading: String,
        chunk: String,
        tx: &AppTx,
    ) {
        match &self.loaded {
            Some(l) if l.path == path && l.ordinal == ordinal => return,
            Some(l) if l.path == path => {
                // Same note, new section: re-resolve the highlight in place and
                // re-anchor the preview, without a fresh vault load.
                if let Some(l) = &mut self.loaded {
                    l.ordinal = ordinal;
                    if let ReaderContent::Loaded { text, highlight } = &mut l.content {
                        *highlight = locate::section_range(text, &heading, &chunk);
                    }
                }
                self.preview.re_anchor();
                return;
            }
            _ => {}
        }

        self.loaded = Some(LoadedNote {
            path: path.clone(),
            ordinal,
            content: ReaderContent::Loading,
        });
        let vault = self.vault.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let text = vault.get_note_text(&path).await.ok();
            let _ = tx.send(AppEvent::Ask(AskData::ReaderNote { path, text }));
        });
    }

    /// Open the selected source's note in the editor (plain `o`, or the
    /// FollowLink intercept) — from any reveal state.
    fn open_selected(&self, tx: &AppTx) {
        if let Some(source) = self.selected_source() {
            tx.send(AppEvent::open(source.path.clone())).ok();
        }
    }

    /// Copy the selected source's path to the OS clipboard, reusing the same
    /// `arboard` seam `ThreadPanel` and the FIND drawer use.
    fn yank_selected_path(&self, tx: &AppTx) {
        let Some(source) = self.selected_source() else {
            return;
        };
        let text = source.path.to_string();
        let msg = match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
            Ok(()) => "path copied".to_string(),
            Err(e) => format!("clipboard: {e}"),
        };
        tx.send(AppEvent::FlashMessage(msg)).ok();
    }

    fn ensure_redraw_tx(&self, tx: &AppTx) {
        let mut slot = self.redraw_tx.lock().unwrap();
        if slot.is_none() {
            *slot = Some(tx.clone());
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.list.poll();
        // Keep the preview anchored to the selection every frame (Context sticks
        // across moves; Full collapses on a change) before laying anything out.
        self.sync_preview();
        // The whole panel is wheel-scrollable; the content sub-region is
        // re-recorded (or cleared) by the branch that draws a preview.
        self.list.set_panel_rect(rect);
        self.list.set_content_rect(Rect::default());
        self.preview.clear_header();

        let block = panel_block("Sources", theme, focused);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        // Empty row set: while the turn's initial load is still in flight show
        // nothing (it lands within a frame); once settled empty, the prompt.
        if !self.has_sources() {
            if !self.list.is_loading() {
                let style = Style::default().fg(theme.gray.to_ratatui());
                f.render_widget(
                    Paragraph::new("no sources — ask something").style(style),
                    inner,
                );
            }
            return;
        }

        // A one-row filter input on top when the list half has been left for the
        // input (typing filters the turn's sources by heading/path).
        let body = if self.list.focus() == Focus::Input {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(inner);
            self.list.render_query(f, rows[0], theme, focused);
            rows[1]
        } else {
            inner
        };

        // Full: the preview takes the whole body, no list visible. The wheel
        // scrolls the content from anywhere in the panel.
        if self.preview.is_full() {
            self.list.set_content_rect(rect);
            self.render_preview(f, body, true, theme);
            return;
        }

        // Context: list on top, half-height preview below, divider between.
        if self.preview.is_context() {
            let max_list = body.height / 2;
            // Rows are two lines each; cap the list at half the panel but shrink
            // for a short list so the preview gets the rest.
            let list_height = (self.list.rows().len() as u16 * 2).min(max_list).max(1);
            let areas = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(list_height),
                    Constraint::Length(1),
                    Constraint::Min(0),
                ])
                .split(body);
            self.list.render(f, areas[0], theme, focused);
            self.list.set_list_rect(areas[0]);
            let gray = theme.gray.to_ratatui();
            let bg = theme.bg_panel.to_ratatui();
            f.render_widget(
                Paragraph::new("\u{2500}".repeat(areas[1].width as usize))
                    .style(Style::default().fg(gray).bg(bg)),
                areas[1],
            );
            self.render_preview(f, areas[2], false, theme);
            self.list.set_content_rect(areas[2]);
            return;
        }

        // Collapsed: list only.
        self.list.render(f, body, theme, focused);
        self.list.set_list_rect(body);
    }

    /// Feed the anchored source's loaded note into the preview surface (Context
    /// or Full), or show the load's placeholder.
    fn render_preview(&mut self, f: &mut Frame, area: Rect, full: bool, theme: &Theme) {
        // Record the content viewport for page scrolling: Full spends two rows
        // on the fixed title + divider chrome; Context uses the whole area.
        self.preview_page = area.height.saturating_sub(if full { 2 } else { 0 });

        let title_fn = self
            .list
            .selected_row()
            .map(|r| (r.source.display_heading(), r.source.path.to_string()));
        let Self {
            loaded, preview, ..
        } = self;
        match loaded {
            Some(LoadedNote {
                content: ReaderContent::Loaded { text, highlight },
                ..
            }) => {
                if full {
                    let (title, filename) =
                        title_fn.unwrap_or_else(|| ("Source".to_string(), String::new()));
                    preview.render_full(
                        f,
                        area,
                        &title,
                        &filename,
                        text,
                        Highlight::Range(highlight.as_ref()),
                        theme,
                    );
                } else {
                    preview.render_context(f, area, text, Highlight::Range(highlight.as_ref()), theme);
                }
            }
            Some(LoadedNote {
                content: ReaderContent::Failed,
                ..
            }) => {
                let red = Style::default().fg(theme.red.to_ratatui());
                f.render_widget(Paragraph::new("failed to load note").style(red), area);
            }
            None
            | Some(LoadedNote {
                content: ReaderContent::Loading,
                ..
            }) => {
                let dim = Style::default().fg(theme.gray.to_ratatui());
                f.render_widget(Paragraph::new("loading\u{2026}").style(dim), area);
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn settle(&mut self) {
        self.list.poll_until_idle().await;
    }

    #[cfg(test)]
    pub(crate) fn match_count(&self) -> usize {
        self.list.match_count()
    }
}

/// (Re)build a [`SearchList`] over the given per-turn rows, wired the same way
/// on every turn: fuzzy local filter, opening on the list, the `l`/`h`/`o`/`y`
/// verbs, and the FollowLink / `Ctrl+Y` intercepts.
fn build_list(
    rows: Vec<SourceRow>,
    intercept: &[KeyCombo],
    icons: &Icons,
    redraw_tx: &Arc<Mutex<Option<AppTx>>>,
) -> SearchList<SourceRow> {
    let slot = redraw_tx.clone();
    let redraw: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if let Some(tx) = slot.lock().unwrap().as_ref() {
            let _ = tx.send(AppEvent::Redraw);
        }
    });
    SearchList::builder(TurnSource { rows }, redraw)
        .icons(icons.clone())
        .filter(Filter::Fuzzy)
        .opening_focus(Focus::List)
        .intercept(intercept.to_vec())
        .list_verb('l')
        .list_verb('h')
        .list_verb('o')
        .list_verb('y')
        .build()
}

/// The similarity as a whole-percent integer (`score` is the server's
/// normalized `0.0..=1.0` similarity — clamped defensively).
fn score_percent(score: f64) -> u32 {
    (score.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// Build the shared [`RichRow`] for a source: the 1-based `rank` as the leading
/// glyph, the journal date and heading kept as distinct spaced elements (never
/// the wire's glued `2026-04-08Afternoon`), the score percentage as dim meta,
/// and the path on the dim filename line.
fn source_row(rank: usize, source: &AskSource, theme: &Theme) -> RichRow {
    let bold = Style::default()
        .fg(theme.fg_bright.to_ratatui())
        .add_modifier(Modifier::BOLD);
    let date_style = Style::default().fg(theme.color_journal_date.to_ratatui());
    let rank_style = Style::default()
        .fg(theme.accent.to_ratatui())
        .add_modifier(Modifier::BOLD);
    let pct = format!("{}%", score_percent(source.score));

    let mut row = if source.heading.is_empty() {
        // A bare-date chunk (empty heading) shows just the date as its title,
        // in the date color, so there is no dangling separator.
        match &source.date {
            Some(date) => RichRow::new(rank.to_string(), date.clone()).title_style(date_style),
            None => RichRow::new(rank.to_string(), String::new()).title_style(bold),
        }
    } else {
        let mut r = RichRow::new(rank.to_string(), source.heading.clone()).title_style(bold);
        if let Some(date) = &source.date {
            r = r.date(date.clone(), Some(date_style));
        }
        r
    };
    row = row.glyph_style(rank_style).meta(pct);
    row.filename(source.path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::VaultConfig;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tempfile::TempDir;

    fn source(path: &str, heading: &str, score: f64, text: &str) -> AskSource {
        AskSource {
            path: VaultPath::new(path),
            heading: heading.to_string(),
            date: None,
            score,
            text: text.to_string(),
            ordinal: 0,
        }
    }

    fn dated_source(path: &str, heading: &str, date: &str, score: f64) -> AskSource {
        AskSource {
            path: VaultPath::new(path),
            heading: heading.to_string(),
            date: Some(date.to_string()),
            score,
            text: String::new(),
            ordinal: 0,
        }
    }

    async fn test_vault() -> (TempDir, NoteVault) {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(dir.path())).await.unwrap();
        (dir, vault)
    }

    fn key_bindings() -> KeyBindings {
        crate::settings::AppSettings::default().key_bindings.clone()
    }

    /// A panel over a throwaway vault, for tests that never touch the note load.
    /// The backing dir is leaked so the vault stays valid for the test's
    /// lifetime.
    async fn test_panel() -> SourcesPanel {
        let (dir, vault) = test_vault().await;
        std::mem::forget(dir);
        SourcesPanel::new(Arc::new(vault), &key_bindings())
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    /// Populate `p` with two sources and drain the engine's initial load so the
    /// rows (and the seeded selection) are live.
    async fn two_source_panel(p: &mut SourcesPanel) {
        p.set_turn(
            1,
            vec![
                source("a.md", "A", 0.9, "alpha body"),
                source("b.md", "B", 0.5, "beta body"),
            ],
        );
        p.settle().await;
    }

    /// Move the list selection to visible index `i` by driving the engine.
    async fn select_index(p: &mut SourcesPanel, i: usize) {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        for _ in 0..i {
            p.handle_input(&InputEvent::Key(key(KeyCode::Char('j'))), &tx);
        }
    }

    fn selected_heading(p: &SourcesPanel) -> Option<String> {
        p.selected_source().map(|s| s.heading.clone())
    }

    /// Heading of the source at rank position `i` (0-based) in the engine's row
    /// set — the panel keeps no parallel sources copy.
    fn nth_heading(p: &SourcesPanel, i: usize) -> Option<String> {
        p.source_at(i).map(|s| s.heading.clone())
    }

    #[test]
    fn score_percent_rounds_and_clamps() {
        assert_eq!(score_percent(0.874), 87);
        assert_eq!(score_percent(1.5), 100);
        assert_eq!(score_percent(-0.2), 0);
    }

    #[test]
    fn dated_source_display_heading_separates_date_and_heading() {
        let s = dated_source("journal/2026-04-08.md", "Afternoon", "2026-04-08", 0.9);
        assert_eq!(s.display_heading(), "2026-04-08 \u{b7} Afternoon");
        assert_eq!(source("n.md", "Ideas", 0.5, "").display_heading(), "Ideas");
    }

    #[tokio::test]
    async fn new_panel_starts_empty_and_collapsed() {
        let p = test_panel().await;
        assert_eq!(p.match_count(), 0);
        assert!(p.preview.is_collapsed());
    }

    #[tokio::test]
    async fn set_turn_populates_and_collapses() {
        let mut p = test_panel().await;
        p.set_turn(1, vec![source("a.md", "A", 0.9, "text a")]);
        p.settle().await;
        assert_eq!(p.turn_id, Some(1));
        assert_eq!(p.match_count(), 1, "the engine mirrors the turn's rows");
        assert!(p.preview.is_collapsed());
    }

    #[tokio::test]
    async fn set_turn_same_id_is_a_noop_and_keeps_selection() {
        let mut p = test_panel().await;
        two_source_panel(&mut p).await;
        select_index(&mut p, 1).await;
        assert_eq!(selected_heading(&p).as_deref(), Some("B"));
        p.set_turn(1, vec![source("c.md", "C", 0.1, "text c")]);
        p.settle().await;
        assert_eq!(
            selected_heading(&p).as_deref(),
            Some("B"),
            "selection must survive a same-id set_turn"
        );
        assert_eq!(p.match_count(), 2, "rows must not be replaced");
        assert_eq!(nth_heading(&p, 0).as_deref(), Some("A"));
    }

    #[tokio::test]
    async fn set_turn_new_id_resets_selection_and_collapses() {
        let mut p = test_panel().await;
        two_source_panel(&mut p).await;
        select_index(&mut p, 1).await;
        p.preview.toggle(Some(VaultPath::new("a.md")));
        p.set_turn(2, vec![source("c.md", "C", 0.1, "text c")]);
        p.settle().await;
        assert_eq!(selected_heading(&p).as_deref(), Some("C"));
        assert_eq!(p.match_count(), 1);
        assert!(p.preview.is_collapsed());
    }

    #[tokio::test]
    async fn focus_source_points_selection_by_ordinal_through_the_engine() {
        let mut p = test_panel().await;
        let mut a = source("a.md", "A", 0.9, "a");
        a.ordinal = 3;
        let mut b = source("b.md", "B", 0.5, "b");
        b.ordinal = 7;
        p.set_turn(1, vec![a, b]);
        p.settle().await;
        p.preview.toggle(Some(VaultPath::new("a.md")));
        p.focus_source(7);
        assert_eq!(
            p.selected_source().map(|s| s.ordinal),
            Some(7),
            "resolved ordinal 7 to its row through the engine, not ordinal-1"
        );
        assert_eq!(selected_heading(&p).as_deref(), Some("B"));
        assert!(p.preview.is_collapsed());
        // An unknown ordinal is ignored.
        p.focus_source(99);
        assert_eq!(p.selected_source().map(|s| s.ordinal), Some(7));
    }

    // ── New filter input (in-memory, heading/path text) ───────────────────

    #[tokio::test]
    async fn filter_input_narrows_sources_by_heading_or_path_text() {
        let mut p = test_panel().await;
        p.set_turn(
            1,
            vec![
                source("alpha.md", "Alpha section", 0.9, "a"),
                source("beta.md", "Beta section", 0.5, "b"),
                source("gamma.md", "Gamma section", 0.3, "g"),
            ],
        );
        p.settle().await;
        assert_eq!(p.match_count(), 3, "no filter shows every source");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        // `i` reveals the filter input; typing filters by heading text.
        assert_eq!(p.list.focus(), Focus::List);
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('i'))), &tx);
        assert_eq!(p.list.focus(), Focus::Input, "`i` reveals the filter input");
        for c in ['B', 'e', 't', 'a'] {
            p.handle_input(&InputEvent::Key(key(KeyCode::Char(c))), &tx);
        }
        p.settle().await;
        assert_eq!(p.match_count(), 1, "typed filter narrows to the match");
        assert_eq!(selected_heading(&p).as_deref(), Some("Beta section"));
    }

    #[tokio::test]
    async fn slash_also_reveals_the_filter_and_matches_path_text() {
        let mut p = test_panel().await;
        p.set_turn(
            1,
            vec![
                source("notes/alpha.md", "One", 0.9, "a"),
                source("journal/beta.md", "Two", 0.5, "b"),
            ],
        );
        p.settle().await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('/'))), &tx);
        assert_eq!(p.list.focus(), Focus::Input, "`/` reveals the filter input");
        for c in ['j', 'o', 'u', 'r'] {
            p.handle_input(&InputEvent::Key(key(KeyCode::Char(c))), &tx);
        }
        p.settle().await;
        assert_eq!(p.match_count(), 1, "path text filters too");
        assert_eq!(selected_heading(&p).as_deref(), Some("Two"));
    }

    // ── Reveal cycle (Enter / l / h) ──────────────────────────────────────

    #[tokio::test]
    async fn enter_and_l_cycle_forward_h_cycles_back() {
        let mut p = test_panel().await;
        two_source_panel(&mut p).await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        assert!(p.preview.is_collapsed());

        p.handle_input(&InputEvent::Key(key(KeyCode::Enter)), &tx);
        assert!(p.preview.is_context(), "Enter: Collapsed -> Context");
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('l'))), &tx);
        assert!(p.preview.is_full(), "l: Context -> Full");
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('l'))), &tx);
        assert!(p.preview.is_collapsed(), "l: Full -> Collapsed (wraps)");

        // Back cycle with h stops at Collapsed.
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('l'))), &tx); // -> Context
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('l'))), &tx); // -> Full
        assert!(p.preview.is_full());
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('h'))), &tx);
        assert!(p.preview.is_context(), "h: Full -> Context");
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('h'))), &tx);
        assert!(p.preview.is_collapsed(), "h: Context -> Collapsed");
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('h'))), &tx);
        assert!(p.preview.is_collapsed(), "h at Collapsed stays Collapsed");
    }

    #[tokio::test]
    async fn esc_steps_back_then_bubbles_to_thread() {
        let mut p = test_panel().await;
        two_source_panel(&mut p).await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        p.preview.toggle(Some(VaultPath::new("a.md"))); // Context

        let st = p.handle_input(&InputEvent::Key(key(KeyCode::Esc)), &tx);
        assert_eq!(st, EventState::Consumed);
        assert!(p.preview.is_collapsed(), "Esc steps back one reveal state");

        // From Collapsed (list focus), Esc bubbles so the host returns focus to
        // the thread.
        let st = p.handle_input(&InputEvent::Key(key(KeyCode::Esc)), &tx);
        assert_eq!(st, EventState::NotConsumed, "Collapsed Esc -> back to thread");
    }

    #[tokio::test]
    async fn jk_moves_selection_within_bounds() {
        let mut p = test_panel().await;
        two_source_panel(&mut p).await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        p.handle_input(&InputEvent::Key(key(KeyCode::Char('j'))), &tx);
        assert_eq!(selected_heading(&p).as_deref(), Some("B"));
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('j'))), &tx);
        assert_eq!(selected_heading(&p).as_deref(), Some("B"), "clamped at the last row");
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('k'))), &tx);
        assert_eq!(selected_heading(&p).as_deref(), Some("A"));
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('k'))), &tx);
        assert_eq!(selected_heading(&p).as_deref(), Some("A"), "clamped at the first row");
    }

    // ── Open (o / FollowLink) — from any reveal state ─────────────────────

    async fn assert_opens_selected(setup: impl Fn(&mut SourcesPanel), open: KeyEvent) {
        let mut p = test_panel().await;
        two_source_panel(&mut p).await;
        select_index(&mut p, 1).await;
        setup(&mut p);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let st = p.handle_input(&InputEvent::Key(open), &tx);
        assert_eq!(st, EventState::Consumed);
        let mut opened = None;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::OpenPath { path, .. } = ev {
                opened = Some(path);
            }
        }
        assert_eq!(
            opened,
            Some(VaultPath::new("b.md")),
            "opened the selected source"
        );
    }

    #[tokio::test]
    async fn o_opens_selected_from_every_reveal_state() {
        // Collapsed, Context, Full — `o` opens the selected source each time.
        assert_opens_selected(|_p| {}, key(KeyCode::Char('o'))).await;
        assert_opens_selected(
            |p| p.preview.toggle(Some(VaultPath::new("b.md"))),
            key(KeyCode::Char('o')),
        )
        .await;
        assert_opens_selected(
            |p| {
                p.preview.toggle(Some(VaultPath::new("b.md")));
                p.preview.toggle(Some(VaultPath::new("b.md")));
            },
            key(KeyCode::Char('o')),
        )
        .await;
    }

    #[tokio::test]
    async fn followlink_ctrl_n_opens_selected() {
        assert_opens_selected(|_p| {}, ctrl(KeyCode::Char('n'))).await;
        // Also from Full.
        assert_opens_selected(
            |p| {
                p.preview.toggle(Some(VaultPath::new("b.md")));
                p.preview.toggle(Some(VaultPath::new("b.md")));
            },
            ctrl(KeyCode::Char('n')),
        )
        .await;
    }

    // ── Yank (y / Ctrl+Y) ─────────────────────────────────────────────────

    async fn assert_yanks(k: KeyEvent) {
        let mut p = test_panel().await;
        two_source_panel(&mut p).await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let st = p.handle_input(&InputEvent::Key(k), &tx);
        assert_eq!(st, EventState::Consumed);
        let mut flashed = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, AppEvent::FlashMessage(_)) {
                flashed = true;
            }
        }
        assert!(flashed, "yank emits a flash message (ok or clipboard error)");
    }

    #[tokio::test]
    async fn plain_y_and_ctrl_y_both_yank() {
        assert_yanks(key(KeyCode::Char('y'))).await;
        assert_yanks(ctrl(KeyCode::Char('y'))).await;
    }

    // ── Async note load + stale-drop ──────────────────────────────────────

    #[tokio::test]
    async fn reader_note_for_the_wrong_path_is_dropped() {
        let mut p = test_panel().await;
        p.set_turn(1, vec![source("a.md", "A", 0.9, "alpha body")]);
        p.loaded = Some(LoadedNote {
            path: VaultPath::new("a.md"),
            ordinal: 0,
            content: ReaderContent::Loading,
        });
        p.handle_data(AskData::ReaderNote {
            path: VaultPath::new("other.md"),
            text: Some("nope".to_string()),
        });
        assert!(
            matches!(p.loaded.as_ref().unwrap().content, ReaderContent::Loading),
            "wrong-path ReaderNote must be dropped, not accepted"
        );
    }

    #[tokio::test]
    async fn reader_note_for_the_right_path_loads_and_highlights() {
        let mut p = test_panel().await;
        p.set_turn(1, vec![source("a.md", "b", 0.9, "beta body")]);
        p.settle().await;
        p.loaded = Some(LoadedNote {
            path: VaultPath::new("a.md"),
            ordinal: 0,
            content: ReaderContent::Loading,
        });
        p.handle_data(AskData::ReaderNote {
            path: VaultPath::new("a.md"),
            text: Some("# a\nalpha body\n# b\nbeta body\n".to_string()),
        });
        match &p.loaded.as_ref().unwrap().content {
            ReaderContent::Loaded { text, highlight } => {
                let r = highlight.clone().expect("chunk resolves");
                assert_eq!(&text[r], "beta body");
            }
            _ => panic!("expected Loaded"),
        }
    }

    #[tokio::test]
    async fn reader_note_load_failure_is_recorded() {
        let mut p = test_panel().await;
        p.set_turn(1, vec![source("a.md", "A", 0.9, "alpha body")]);
        p.loaded = Some(LoadedNote {
            path: VaultPath::new("a.md"),
            ordinal: 0,
            content: ReaderContent::Loading,
        });
        p.handle_data(AskData::ReaderNote {
            path: VaultPath::new("a.md"),
            text: None,
        });
        assert!(matches!(
            p.loaded.as_ref().unwrap().content,
            ReaderContent::Failed
        ));
    }

    #[tokio::test]
    async fn handle_data_ignores_answer_ready() {
        let mut p = test_panel().await;
        p.set_turn(1, vec![source("a.md", "A", 0.9, "alpha body")]);
        p.loaded = Some(LoadedNote {
            path: VaultPath::new("a.md"),
            ordinal: 0,
            content: ReaderContent::Loading,
        });
        p.handle_data(AskData::AnswerReady {
            turn_id: 1,
            result: Ok(("x".into(), vec![])),
        });
        assert!(matches!(
            p.loaded.as_ref().unwrap().content,
            ReaderContent::Loading
        ));
    }

    #[tokio::test]
    async fn open_reader_opens_preview_and_round_trips_a_real_vault() {
        let (_dir, vault) = test_vault().await;
        let path = VaultPath::new("note.md");
        vault.create_note(&path, "# h\nbody text\n").await.unwrap();

        let mut p = SourcesPanel::new(Arc::new(vault), &key_bindings());
        p.set_turn(1, vec![source("note.md", "h", 0.9, "body text")]);
        p.settle().await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        p.open_reader(0, &tx);
        assert!(p.preview.is_context(), "open_reader opens the Context preview");

        let event = rx.recv().await.expect("open_reader spawns a ReaderNote");
        let AppEvent::Ask(data) = event else {
            panic!("expected an Ask event");
        };
        p.handle_data(data);
        match &p.loaded.as_ref().unwrap().content {
            ReaderContent::Loaded { text, .. } => assert_eq!(text, "# h\nbody text\n"),
            _ => panic!("expected Loaded"),
        }
    }

    #[tokio::test]
    async fn navigating_in_context_reloads_for_the_new_source() {
        let (_dir, vault) = test_vault().await;
        std::mem::forget(_dir);
        let mut p = SourcesPanel::new(Arc::new(vault), &key_bindings());
        two_source_panel(&mut p).await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        p.preview.toggle(Some(VaultPath::new("a.md"))); // Context
        p.ensure_loaded(&tx);
        assert_eq!(p.loaded.as_ref().unwrap().path, VaultPath::new("a.md"));
        // Move down while the preview is open: the load re-keys to b.md, so a
        // late a.md ReaderNote would now be dropped.
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('j'))), &tx);
        assert_eq!(p.loaded.as_ref().unwrap().path, VaultPath::new("b.md"));
    }

    // ── Rendering ─────────────────────────────────────────────────────────

    fn buffer_text(p: &mut SourcesPanel, w: u16, h: u16) -> String {
        let theme = Theme::default();
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            let area = f.area();
            p.render(f, area, &theme, true);
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn row_render_carries_rank_and_score() {
        let mut p = test_panel().await;
        p.set_turn(
            1,
            vec![
                dated_source("journal/2026-04-08.md", "Afternoon", "2026-04-08", 0.9),
                source("b.md", "Beta section", 0.42, "beta body"),
            ],
        );
        p.settle().await;
        let text = buffer_text(&mut p, 60, 8);
        assert!(text.contains("1 "), "rank 1 leads the first row: {text}");
        assert!(text.contains("2 "), "rank 2 leads the second row: {text}");
        assert!(text.contains("90%"), "score percent shown: {text}");
        assert!(text.contains("42%"), "second score shown: {text}");
        assert!(text.contains("2026-04-08"), "date kept: {text}");
        assert!(
            text.contains('\u{b7}'),
            "date \u{b7} heading separation: {text}"
        );
        assert!(text.contains("Afternoon"), "heading kept: {text}");
    }

    #[tokio::test]
    async fn render_does_not_panic_across_states_and_sizes() {
        let mut p = test_panel().await;
        buffer_text(&mut p, 40, 10); // empty list

        p.set_turn(
            1,
            vec![
                dated_source("journal/2026-04-08.md", "Afternoon", "2026-04-08", 0.9),
                source("b.md", "Beta section", 0.4, "beta body"),
            ],
        );
        p.settle().await;
        buffer_text(&mut p, 40, 10); // collapsed list
        select_index(&mut p, 1).await;
        buffer_text(&mut p, 40, 3); // tiny viewport

        // Context with a loaded note.
        p.preview.toggle(Some(VaultPath::new("b.md")));
        p.loaded = Some(LoadedNote {
            path: VaultPath::new("b.md"),
            ordinal: 0,
            content: ReaderContent::Loaded {
                text: "# Beta\nbeta body\nmore\n".to_string(),
                highlight: Some(7..16),
            },
        });
        buffer_text(&mut p, 40, 12); // context + preview
        p.preview.toggle(Some(VaultPath::new("b.md"))); // -> Full
        buffer_text(&mut p, 40, 12); // full preview

        // Loading / Failed placeholders.
        p.loaded = Some(LoadedNote {
            path: VaultPath::new("b.md"),
            ordinal: 0,
            content: ReaderContent::Loading,
        });
        buffer_text(&mut p, 40, 12);
        p.loaded = Some(LoadedNote {
            path: VaultPath::new("b.md"),
            ordinal: 0,
            content: ReaderContent::Failed,
        });
        buffer_text(&mut p, 40, 12);

        buffer_text(&mut p, 3, 3); // degenerate
        buffer_text(&mut p, 0, 0); // zero rect
    }

    #[tokio::test]
    async fn full_preview_anchors_scroll_to_the_highlighted_section() {
        let mut p = test_panel().await;
        p.set_turn(1, vec![source("a.md", "b", 0.9, "beta body")]);
        p.settle().await;
        // Open to Full and load a note where the section is several lines down.
        p.preview.toggle(Some(VaultPath::new("a.md"))); // Context
        p.preview.toggle(Some(VaultPath::new("a.md"))); // Full
        // Section is deep enough that anchoring scrolls past the top (the "two
        // lines of context above the section" rule needs room above it).
        let mut body = String::new();
        for i in 0..8 {
            body.push_str(&format!("line{i}\n"));
        }
        body.push_str("beta body\n");
        for i in 0..8 {
            body.push_str(&format!("tail{i}\n"));
        }
        let start = body.find("beta body").unwrap();
        p.loaded = Some(LoadedNote {
            path: VaultPath::new("a.md"),
            ordinal: 0,
            content: ReaderContent::Loaded {
                text: body,
                highlight: Some(start..start + "beta body".len()),
            },
        });
        // Full mode: title(1) + divider(1) + content; a short content viewport so
        // the section (line 2) is scrollable into view.
        buffer_text(&mut p, 40, 6);
        assert!(
            p.preview.scroll_offset() > 0,
            "preview anchored the scroll to the section, offset={}",
            p.preview.scroll_offset()
        );
    }

    // ── Full-preview content scroll (F1) ──────────────────────────────────

    #[tokio::test]
    async fn full_down_scrolls_content_not_the_list() {
        let mut p = test_panel().await;
        two_source_panel(&mut p).await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        p.preview.toggle(Some(VaultPath::new("a.md"))); // Context
        p.preview.toggle(Some(VaultPath::new("a.md"))); // Full
        // A note taller than the viewport with the section at the very top, so
        // the anchor sits at offset 0 with room to scroll down.
        let mut body = String::from("alpha body\n");
        for i in 0..20 {
            body.push_str(&format!("line{i}\n"));
        }
        p.loaded = Some(LoadedNote {
            path: VaultPath::new("a.md"),
            ordinal: 0,
            content: ReaderContent::Loaded {
                text: body,
                highlight: Some(0.."alpha body".len()),
            },
        });
        buffer_text(&mut p, 40, 6); // render sets max; anchor at the top
        assert_eq!(p.preview.scroll_offset(), 0);
        // Down scrolls the preview content; the list selection stays put.
        p.handle_input(&InputEvent::Key(key(KeyCode::Down)), &tx);
        assert_eq!(
            selected_heading(&p).as_deref(),
            Some("A"),
            "Down in Full scrolls content, not the list"
        );
        assert!(
            p.preview.scroll_offset() > 0,
            "Full + Down scrolled the content, offset={}",
            p.preview.scroll_offset()
        );
    }

    #[tokio::test]
    async fn full_j_still_moves_the_list_selection() {
        let mut p = test_panel().await;
        two_source_panel(&mut p).await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        p.preview.toggle(Some(VaultPath::new("a.md"))); // Context
        p.preview.toggle(Some(VaultPath::new("a.md"))); // Full
        assert!(p.preview.is_full());
        // `j` is left for list navigation even under the full preview.
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('j'))), &tx);
        assert_eq!(
            selected_heading(&p).as_deref(),
            Some("B"),
            "j moves the list selection in Full"
        );
    }

    #[tokio::test]
    async fn wheel_scrolls_the_open_preview_and_is_ignored_when_collapsed() {
        let mut p = test_panel().await;
        two_source_panel(&mut p).await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        // Collapsed with no list rect recorded yet: the wheel misses everything
        // and is left unconsumed for the host.
        let wheel = |kind| {
            InputEvent::Mouse(MouseEvent {
                kind,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            })
        };
        assert_eq!(
            p.handle_input(&wheel(MouseEventKind::ScrollDown), &tx),
            EventState::NotConsumed,
            "collapsed preview with no recorded rect does not eat the wheel"
        );
        // Open to Full with scrollable content.
        p.preview.toggle(Some(VaultPath::new("a.md")));
        p.preview.toggle(Some(VaultPath::new("a.md")));
        let mut body = String::from("alpha body\n");
        for i in 0..20 {
            body.push_str(&format!("line{i}\n"));
        }
        p.loaded = Some(LoadedNote {
            path: VaultPath::new("a.md"),
            ordinal: 0,
            content: ReaderContent::Loaded {
                text: body,
                highlight: Some(0.."alpha body".len()),
            },
        });
        buffer_text(&mut p, 40, 6);
        assert_eq!(
            p.handle_input(&wheel(MouseEventKind::ScrollDown), &tx),
            EventState::Consumed,
            "open preview consumes the wheel"
        );
        assert!(p.preview.scroll_offset() > 0, "wheel scrolled the content");
    }

    // ── Same-note, different-section re-anchor (F2) ───────────────────────

    #[tokio::test]
    async fn same_note_different_heading_recomputes_highlight_without_reload() {
        let (_dir, vault) = test_vault().await;
        std::mem::forget(_dir);
        let mut p = SourcesPanel::new(Arc::new(vault), &key_bindings());
        // Two sources in the SAME note, different sections (distinct ordinals).
        let mut s0 = source("doc.md", "Alpha", 0.9, "alpha body");
        s0.ordinal = 1;
        let mut s1 = source("doc.md", "Beta", 0.8, "beta body");
        s1.ordinal = 2;
        p.set_turn(1, vec![s0, s1]);
        p.settle().await;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        p.preview.toggle(Some(VaultPath::new("doc.md"))); // Context
        p.ensure_loaded(&tx); // spawns a load for doc.md, ordinal 1
        // Deliver the note text (simulating the load landing).
        let note = "# Alpha\nalpha body\n# Beta\nbeta body\n".to_string();
        p.handle_data(AskData::ReaderNote {
            path: VaultPath::new("doc.md"),
            text: Some(note),
        });
        let first = match &p.loaded.as_ref().unwrap().content {
            ReaderContent::Loaded { text, highlight } => {
                let r = highlight.clone().expect("section resolves");
                assert_eq!(&text[r.clone()], "alpha body");
                r
            }
            _ => panic!("expected Loaded"),
        };
        // Move to the second source (same note): the highlight re-resolves to
        // the new section and the loaded note is REUSED (no drop to Loading).
        p.handle_input(&InputEvent::Key(key(KeyCode::Char('j'))), &tx);
        match &p.loaded.as_ref().unwrap().content {
            ReaderContent::Loaded { text, highlight } => {
                let r = highlight.clone().expect("re-resolved");
                assert_eq!(&text[r.clone()], "beta body");
                assert_ne!(r, first, "highlight moved to the new section");
            }
            _ => panic!("must reuse the loaded note, not reload"),
        }
        assert_eq!(
            p.loaded.as_ref().unwrap().ordinal,
            2,
            "re-keyed to the new source"
        );
    }

    // ── open_reader keeps the reveal (F5) ─────────────────────────────────

    #[tokio::test]
    async fn open_reader_stays_full_and_re_points_to_the_source() {
        let (_dir, vault) = test_vault().await;
        vault
            .create_note(&VaultPath::new("a.md"), "# ha\nalpha text\n")
            .await
            .unwrap();
        vault
            .create_note(&VaultPath::new("b.md"), "# hb\nbeta text\n")
            .await
            .unwrap();
        let mut p = SourcesPanel::new(Arc::new(vault), &key_bindings());
        let mut s0 = source("a.md", "ha", 0.9, "alpha text");
        s0.ordinal = 1;
        let mut s1 = source("b.md", "hb", 0.8, "beta text");
        s1.ordinal = 2;
        p.set_turn(1, vec![s0, s1]);
        p.settle().await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        // Reveal source 1 in Full.
        select_index(&mut p, 1).await;
        p.preview.toggle(Some(VaultPath::new("b.md"))); // Context
        p.preview.toggle(Some(VaultPath::new("b.md"))); // Full
        assert!(p.preview.is_full());
        // Directed reveal of source 0 must STAY Full (not collapse) and re-point.
        p.open_reader(0, &tx);
        assert!(p.preview.is_full(), "open_reader keeps the Full reveal");
        assert_eq!(selected_heading(&p).as_deref(), Some("ha"));
        // It spawned a load for source 0's note; deliver it and check the section.
        let ev = rx.recv().await.expect("open_reader spawns a ReaderNote");
        let AppEvent::Ask(data) = ev else {
            panic!("expected an Ask event");
        };
        p.handle_data(data);
        match &p.loaded.as_ref().unwrap().content {
            ReaderContent::Loaded { text, highlight } => {
                assert_eq!(text, "# ha\nalpha text\n", "source 0's note is shown");
                let r = highlight.clone().expect("section resolves");
                assert_eq!(&text[r], "alpha text");
            }
            _ => panic!("expected Loaded for source 0"),
        }
    }
}
