//! `ThreadPanel` — the editor area's Ask-workspace content (see CONTEXT.md:
//! **Ask workspace**, **Thread**; adr/0030). Owns the conversation `Thread`,
//! the docked question composer, and the live `RagClient` (when the Kimün
//! server can answer questions).
//!
//! The panel is a permanent resident of `PanelSet` (like the note editor —
//! ADR-0017): its conversation survives the user switching the editor area to
//! another view because the panel itself is never dropped or moved. Losing the
//! client (server unreachable / no LLM) disables the composer without evicting
//! the thread — the thread's answers are already local.
//!
//! Input runs through the inherent [`ThreadPanel::handle_input`], not the
//! `Component` trait method: the panel derives everything it needs (enabled
//! state, submission) from its own `client`, so the trait `render` (and its
//! default no-op input) is all the generic `dyn Component` dispatch needs.

use std::ops::Range;
use std::sync::Arc;

use kimun_server_client::RagClient;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ask::{AskSource, Thread, Turn, TurnStatus, citations, save};
use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, AskData, FileOp, InputEvent};
use crate::components::panel::panel_block;
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// Height (in rows) of the docked composer box, borders included.
const COMPOSER_HEIGHT: u16 = 3;

/// Rows a PageUp/PageDown leaves visible from the previous view (shared
/// convention with `AttachmentView`).
const PAGE_OVERLAP: u16 = 2;

/// The synchronous half of a turn kickoff (see `ThreadPanel::begin_turn`):
/// the question, the history to send with it, and the new turn's id.
type PendingTurn = (String, Vec<(String, String)>, u64);

/// Which part of the Ask workspace has keyboard focus within the editor
/// area: the question composer, or the turn list above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadFocus {
    Composer,
    Turns,
}

/// What a single rendered row of the turn list belongs to — the last
/// `render_turns` call's row → data mapping, used for mouse hit-testing
/// (`handle_mouse`). `row_map[i]` describes the row at `turns_rect.y + i`.
enum RowSlot {
    /// A turn's question/status line — clicking anywhere on it selects the
    /// turn.
    Turn(u64),
    /// One word-wrapped line of a turn's answer body: the turn it belongs to,
    /// the line's byte range into `turn.answer`, and the line's column map
    /// (`rendered char index → byte offset within the sliced range`, from
    /// `markdown_lines::style_slice_mapped`). Because emphasis sigils are hidden
    /// in the rendered answer, a click's column no longer maps 1:1 to the source
    /// bytes — the map resolves it back so `citation_at_column` still lands on
    /// the right `[n]`.
    Answer {
        turn_id: u64,
        range: Range<usize>,
        col_map: Vec<usize>,
    },
}

/// The Ask workspace's editor-area content: the conversation `Thread` plus
/// the docked question composer. See the module doc for lifetime notes.
pub struct ThreadPanel {
    thread: Thread,
    composer: SingleLineInput,
    /// The live RAG client when the Kimün server can answer questions, else
    /// `None`. Its presence is the single source of truth for whether the
    /// composer is enabled: losing the client disables submission without
    /// evicting the thread (the answers are already local — CONTEXT.md:
    /// **Ask workspace**).
    client: Option<Arc<RagClient>>,
    focus: ThreadFocus,
    /// Topmost visible row of the flattened turn-lines list. While
    /// `follow_selection` is set the render keeps the selected turn in view;
    /// content-scroll keys (`PageUp`/`PageDown`/`Home`/`End`) and the wheel take
    /// it over.
    scroll: u16,
    /// True while the render owns `scroll` (keep the selected turn in view). A
    /// content-scroll key or wheel tick clears it; a selection move (`j`/`k`)
    /// re-arms it. Mirrors `PreviewPane`'s anchored/user-owned split.
    follow_selection: bool,
    /// One-shot: the next render scrolls so the selected turn's *end* is visible
    /// (bottom-follow), set when a turn is added or its answer completes so new
    /// content comes into view. Cleared by the render that honors it.
    bottom_follow_pending: bool,
    /// The turn list's viewport height from the last render — the page size for
    /// `PageUp`/`PageDown`.
    turns_height: u16,
    /// Citation `[n]` ordinal a click asked the Sources drawer to focus (NOT a
    /// vec position — the drawer resolves ordinal → row). Cleared on read via
    /// `take_citation_target`.
    citation_target: Option<usize>,
    /// The turn list's rect from the last render — mouse hit-testing base.
    turns_rect: Rect,
    /// The composer's rect from the last render — mouse hit-testing base.
    composer_rect: Rect,
    /// Row → data mapping from the last render, scoped to `turns_rect`.
    row_map: Vec<RowSlot>,
    /// Glyph set (question-prompt chevron, …) resolved from `use_nerd_fonts`.
    /// Defaults to the ASCII set; `set_icons` swaps in the configured one.
    icons: Icons,
}

impl ThreadPanel {
    pub fn new() -> Self {
        Self {
            thread: Thread::default(),
            composer: SingleLineInput::new(),
            client: None,
            focus: ThreadFocus::Composer,
            scroll: 0,
            follow_selection: true,
            bottom_follow_pending: false,
            turns_height: 0,
            citation_target: None,
            turns_rect: Rect::default(),
            composer_rect: Rect::default(),
            row_map: Vec::new(),
            icons: Icons::new(false),
        }
    }

    /// Swap in the configured glyph set (nerd-font vs ASCII) — the Ask panel is
    /// resident, so `PanelSet` refreshes it here whenever icons are (re)built.
    pub fn set_icons(&mut self, icons: Icons) {
        self.icons = icons;
    }

    /// Set (or clear) the live RAG client — the single injection point
    /// `PanelSet::set_ask_client` drives. A present client enables the
    /// composer; `None` disables it without touching the thread (adr/0030).
    pub fn set_client(&mut self, client: Option<Arc<RagClient>>) {
        self.client = client;
    }

    /// Whether a live RAG client is set — i.e. the composer can submit.
    pub fn has_client(&self) -> bool {
        self.client.is_some()
    }

    /// Move keyboard focus to the question composer (leader `a a` / the Ask
    /// shortcut land here).
    pub fn focus_composer(&mut self) {
        self.focus = ThreadFocus::Composer;
    }

    pub fn thread(&self) -> &Thread {
        &self.thread
    }

    pub fn thread_mut(&mut self) -> &mut Thread {
        &mut self.thread
    }

    /// The source row a citation click asked to be focused, if any — cleared
    /// on read.
    pub fn take_citation_target(&mut self) -> Option<usize> {
        self.citation_target.take()
    }

    // ── Input ────────────────────────────────────────────────────────────

    /// Handle an input event. Submission/regeneration derive from the panel's
    /// own `client`: with no client the composer is disabled and nothing is
    /// ever spawned (no orphaned `Thinking` turn).
    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Key(key) => self.handle_key(key, tx),
            InputEvent::Mouse(mouse) => self.handle_mouse(mouse, tx),
            InputEvent::Paste(_) => EventState::NotConsumed,
        }
    }

    pub fn handle_data(&mut self, data: AskData) {
        if let AskData::AnswerReady { turn_id, result } = data {
            match result {
                Ok((answer, sources)) => {
                    if self.thread.complete(turn_id, answer, sources) {
                        // The answer landed: bring its (now full) content into
                        // view so a long answer doesn't complete off-screen.
                        self.follow_bottom();
                    }
                }
                Err(e) => {
                    if self.thread.fail(turn_id, e) {
                        self.follow_bottom();
                    }
                }
            }
        }
        // `ReaderNote` is addressed to the source reader (Task 10), not here.
    }

    /// Arm bottom-follow: the next render scrolls the selected turn's end into
    /// view. Also re-arms selection-follow so a prior manual scroll doesn't
    /// suppress it.
    fn follow_bottom(&mut self) {
        self.follow_selection = true;
        self.bottom_follow_pending = true;
    }

    /// Scroll the content by `delta` rows, taking the offset over from the
    /// selection-follow anchor (mirrors `PreviewPane`'s user-owned scroll). The
    /// upper bound is clamped by the next render against the wrapped-row total.
    fn content_scroll_by(&mut self, delta: i32) {
        self.follow_selection = false;
        self.bottom_follow_pending = false;
        self.scroll = if delta < 0 {
            self.scroll.saturating_sub((-delta) as u16)
        } else {
            self.scroll.saturating_add(delta as u16)
        };
    }

    fn handle_key(&mut self, key: &KeyEvent, tx: &AppTx) -> EventState {
        match self.focus {
            ThreadFocus::Composer => self.handle_composer_key(key, tx),
            ThreadFocus::Turns => self.handle_turns_key(key, tx),
        }
    }

    fn handle_composer_key(&mut self, key: &KeyEvent, tx: &AppTx) -> EventState {
        if key.code == KeyCode::Esc {
            self.focus = ThreadFocus::Turns;
            return EventState::Consumed;
        }
        match self.composer.handle_key(key) {
            InputOutcome::Submit => {
                self.submit(tx);
                EventState::Consumed
            }
            InputOutcome::NotConsumed => EventState::NotConsumed,
            _ => EventState::Consumed,
        }
    }

    fn handle_turns_key(&mut self, key: &KeyEvent, tx: &AppTx) -> EventState {
        // Page size for content scrolling, leaving a little overlap (mirrors
        // AttachmentView / the note preview).
        let page = self.turns_height.saturating_sub(PAGE_OVERLAP).max(1) as i32;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.thread.select_prev();
                // A selection move re-arms keep-in-view over any manual scroll.
                self.follow_selection = true;
                EventState::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.thread.select_next();
                self.follow_selection = true;
                EventState::Consumed
            }
            // Content scrolling for reading within a long turn — plain,
            // selection-independent, like the preview/attachment surfaces.
            KeyCode::PageUp => {
                self.content_scroll_by(-page);
                EventState::Consumed
            }
            KeyCode::PageDown => {
                self.content_scroll_by(page);
                EventState::Consumed
            }
            KeyCode::Home => {
                self.content_scroll_by(-(u16::MAX as i32));
                EventState::Consumed
            }
            KeyCode::End => {
                self.content_scroll_by(u16::MAX as i32);
                EventState::Consumed
            }
            KeyCode::Char('i') | KeyCode::Char('/') => {
                self.focus = ThreadFocus::Composer;
                EventState::Consumed
            }
            KeyCode::Char('y') => {
                self.copy_selected(tx);
                EventState::Consumed
            }
            KeyCode::Char('e') => {
                self.save_selected(tx);
                EventState::Consumed
            }
            KeyCode::Char('r') => {
                self.regenerate_selected(tx);
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn handle_mouse(&mut self, mouse: &MouseEvent, _tx: &AppTx) -> EventState {
        let pos = Position {
            x: mouse.column,
            y: mouse.row,
        };
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.composer_rect.contains(pos) {
                    self.focus = ThreadFocus::Composer;
                    return EventState::Consumed;
                }
                if !self.turns_rect.contains(pos) {
                    return EventState::NotConsumed;
                }
                self.focus = ThreadFocus::Turns;
                self.click_turns(mouse);
                EventState::Consumed
            }
            MouseEventKind::ScrollUp if self.turns_rect.contains(pos) => {
                self.content_scroll_by(-1);
                EventState::Consumed
            }
            MouseEventKind::ScrollDown if self.turns_rect.contains(pos) => {
                self.content_scroll_by(1);
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    /// Resolve a click inside `turns_rect` against the last render's
    /// `row_map`: selects the clicked turn, and — for a click landing on an
    /// answer line — resolves the column to a citation, marking
    /// `citation_target` when it's in range of the turn's sources.
    fn click_turns(&mut self, mouse: &MouseEvent) {
        let idx = (mouse.row - self.turns_rect.y) as usize;
        let hit = self.row_map.get(idx).map(|slot| match slot {
            RowSlot::Turn(id) => (*id, None),
            RowSlot::Answer {
                turn_id,
                range,
                col_map,
            } => (*turn_id, Some((range.clone(), col_map.clone()))),
        });
        let Some((turn_id, answer_hit)) = hit else {
            return;
        };
        self.select_turn(turn_id);
        let Some((range, col_map)) = answer_hit else {
            return;
        };
        let col = mouse.column.saturating_sub(self.turns_rect.x);
        let Some(turn) = self.thread.selected() else {
            return;
        };
        let Some(citation_idx) = citation_at_column(&turn.answer[range], &col_map, col) else {
            return;
        };
        // Resolve `[n]` through the pairing seam (by ordinal, not vec position);
        // store the ordinal itself — the Sources panel translates it to a row.
        if turn.source_for_citation(citation_idx).is_some() {
            self.citation_target = Some(citation_idx);
        }
    }

    /// Move the thread's selection to turn `id`. No-op when `id` is already
    /// selected or unknown.
    fn select_turn(&mut self, id: u64) {
        if self.thread.selected().map(|t| t.id) == Some(id) {
            return;
        }
        let Some(target_idx) = self.thread.turns().iter().position(|t| t.id == id) else {
            return;
        };
        self.thread.select_index(target_idx);
    }

    // ── Turn actions ─────────────────────────────────────────────────────

    /// Pre-spawn half of `submit`, factored out for testability: validates
    /// that a client is present and the composer text is non-empty, pushes a
    /// `Thinking` turn, and returns what the spawn needs. `None` — and no
    /// thread mutation — when there is no client or the composer is
    /// (effectively) empty. Checking the client here (not just in `submit`) is
    /// what keeps a clientless submit from orphaning a forever-`Thinking` turn.
    fn begin_turn(&mut self) -> Option<PendingTurn> {
        self.client.as_ref()?;
        let question = self.composer.take_text();
        let question = question.trim().to_string();
        if question.is_empty() {
            return None;
        }
        // Read history before `ask()` pushes the new turn — `Thread::history`
        // already excludes the in-flight turn either way, so this ordering
        // isn't load-bearing, but it matches the eventual spawn's intent.
        let history = self.thread.history();
        let turn_id = self.thread.ask(question.clone());
        // The new turn is selected; bring it (and its incoming answer) into view.
        self.follow_bottom();
        Some((question, history, turn_id))
    }

    /// Submit the composer's question. `begin_turn` already guarantees a
    /// client is present (else it pushes no turn), so this spawns the ask job,
    /// delivering `AppEvent::Ask(AskData::AnswerReady)` on completion.
    fn submit(&mut self, tx: &AppTx) {
        let Some((question, history, turn_id)) = self.begin_turn() else {
            return;
        };
        let Some(client) = self.client.clone() else {
            return;
        };
        Self::spawn_ask(tx, &client, question, history, turn_id);
    }

    /// Rewind the selected turn back to `Thinking` and re-ask its question.
    /// The server always re-retrieves context, so the completion carries fresh
    /// sources — the `[n]` markers in the new answer are numbered against that
    /// fresh context, and replacing the turn's sources keeps citations, reader
    /// targets, and saved-note wikilinks aligned. (`Thread::regenerate` leaves
    /// the old sources in place while the turn is `Thinking`, so the previous
    /// evidence stays visible during regeneration; only completion swaps them.)
    /// No-op without a client, or when the selected turn is currently in
    /// flight (`Thread::regenerate` rejects that case). The client is checked
    /// first, before any rewind, so a clientless regenerate leaves the turn
    /// `Done` rather than orphaning it as `Thinking`. Leader `a r`.
    pub(crate) fn regenerate_selected(&mut self, tx: &AppTx) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let Some(id) = self.thread.selected().map(|t| t.id) else {
            return;
        };
        let Some(question) = self.thread.regenerate(id) else {
            return;
        };
        let history = self.thread.history();
        Self::spawn_ask(tx, &client, question, history, id);
    }

    /// Spawn the async ask job for `turn_id`: call the RAG client with the
    /// question and history, map the answer + freshly retrieved sources, and
    /// deliver an `AskData::AnswerReady` on completion. Shared verbatim by
    /// `submit` and `regenerate_selected` — both re-retrieve, so both take the
    /// fresh sources from the response.
    fn spawn_ask(
        tx: &AppTx,
        client: &Arc<RagClient>,
        question: String,
        history: Vec<(String, String)>,
        turn_id: u64,
    ) {
        let (tx, client) = (tx.clone(), client.clone());
        tokio::spawn(async move {
            let result = client
                .ask(&question, &history, None)
                .await
                .map(|a| {
                    // Normalize the wire ordinal ONCE here (position → 1-based
                    // fallback for an older server); downstream sees real ordinals.
                    let sources = a
                        .sources
                        .into_iter()
                        .enumerate()
                        .map(|(i, c)| AskSource::from_chunk(i, c))
                        .collect();
                    (a.answer, sources)
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::Ask(AskData::AnswerReady { turn_id, result }));
        });
    }

    /// Copy the selected turn's answer (citation markers stripped) to the OS
    /// clipboard, reusing the editor's `arboard` seam. Leader `a y`.
    pub(crate) fn copy_selected(&self, tx: &AppTx) {
        let Some(turn) = self.thread.selected() else {
            return;
        };
        let text = citations::strip(&turn.answer);
        let msg = match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
            Ok(()) => "answer copied".to_string(),
            Err(e) => format!("clipboard: {e}"),
        };
        tx.send(AppEvent::FlashMessage(msg)).ok();
    }

    /// Open the create-note dialog pre-filled with the selected turn saved as
    /// a note (adr/0030: **Saved answer**). The dialog owns validation and
    /// the actual create call — this only supplies the path/content. Leader `a e`.
    pub(crate) fn save_selected(&self, tx: &AppTx) {
        let Some(turn) = self.thread.selected() else {
            return;
        };
        let path = save::suggested_path(&turn.question);
        let content = save::note_content(turn);
        tx.send(AppEvent::FileOp(FileOp::ShowCreateWithContent {
            path,
            content,
        }))
        .ok();
    }

    // ── Render ───────────────────────────────────────────────────────────

    fn render_turns(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.turns_rect = rect;

        // The question line gets a strong identity (accent + bold + a chevron
        // prompt glyph); turns are parted by a theme-dimmed horizontal rule.
        let question = Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::BOLD);
        let separator = Style::default()
            .fg(theme.gray.to_ratatui())
            .add_modifier(Modifier::DIM);
        let dim = Style::default().fg(theme.gray.to_ratatui());
        let err = Style::default().fg(theme.red.to_ratatui());
        let md = crate::components::markdown_lines::MdStyles::from_theme(theme);
        let prompt = self.icons.question_prompt;

        let mut rows: Vec<(RowSlot, Line<'static>)> = Vec::new();
        let mut turn_start_row: Vec<(u64, u16)> = Vec::new();
        for (i, turn) in self.thread.turns().iter().enumerate() {
            turn_start_row.push((turn.id, rows.len() as u16));
            render_turn(
                turn,
                rect.width,
                i == 0,
                prompt,
                question,
                separator,
                dim,
                err,
                &md,
                &mut rows,
            );
        }
        let total = rows.len() as u16;
        let height = rect.height;
        self.turns_height = height;

        // Auto-scroll (while following): bottom-follow pins the selected turn's
        // end to view (new content just landed); otherwise keep its start in
        // view. A manual scroll clears `follow_selection`, leaving the offset
        // alone but for the clamp below.
        if let Some(sel) = self.thread.selected()
            && let Some(&(_, start)) = turn_start_row.iter().find(|(id, _)| *id == sel.id)
        {
            // End row of the selected turn: one before the next turn's start,
            // or the last row for the final turn.
            let end = turn_start_row
                .iter()
                .map(|(_, s)| *s)
                .filter(|s| *s > start)
                .min()
                .unwrap_or(total)
                .saturating_sub(1);
            if self.bottom_follow_pending {
                if height > 0 {
                    self.scroll = end.saturating_sub(height - 1);
                }
            } else if self.follow_selection {
                if start < self.scroll {
                    self.scroll = start;
                } else if height > 0 && start >= self.scroll + height {
                    self.scroll = start.saturating_sub(height - 1);
                }
            }
        }
        self.bottom_follow_pending = false;
        self.scroll = self.scroll.min(total.saturating_sub(height));

        let selected_id = self.thread.selected().map(|t| t.id);
        self.row_map.clear();
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (slot, line) in rows
            .into_iter()
            .skip(self.scroll as usize)
            .take(height as usize)
        {
            let row_turn_id = match &slot {
                RowSlot::Turn(id) => *id,
                RowSlot::Answer { turn_id, .. } => *turn_id,
            };
            let line = if focused && Some(row_turn_id) == selected_id {
                line.style(Style::default().bg(theme.selection_bg.to_ratatui()))
            } else {
                line
            };
            self.row_map.push(slot);
            lines.push(line);
        }
        f.render_widget(Paragraph::new(lines), rect);
    }

    fn render_composer(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.composer_rect = rect;

        let enabled = self.client.is_some();
        let title = if enabled {
            "Ask a question"
        } else {
            "server unavailable"
        };
        let block = panel_block(title, theme, focused);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let style = if enabled {
            Style::default().fg(theme.fg.to_ratatui())
        } else {
            Style::default()
                .fg(theme.gray.to_ratatui())
                .add_modifier(Modifier::DIM)
        };
        self.composer
            .render(f, inner, style, 0, focused && enabled);
    }
}

impl Default for ThreadPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for ThreadPanel {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(COMPOSER_HEIGHT)])
            .split(rect);
        self.render_turns(
            f,
            chunks[0],
            theme,
            focused && self.focus == ThreadFocus::Turns,
        );
        self.render_composer(
            f,
            chunks[1],
            theme,
            focused && self.focus == ThreadFocus::Composer,
        );
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        match self.focus {
            ThreadFocus::Composer => vec![
                ("Enter".into(), "Ask".into()),
                ("Esc".into(), "Turns".into()),
            ],
            ThreadFocus::Turns => vec![
                ("j/k".into(), "Select".into()),
                ("PgUp/PgDn".into(), "Scroll".into()),
                ("i//".into(), "Compose".into()),
                ("y".into(), "Copy".into()),
                ("e".into(), "Save as note".into()),
                ("r".into(), "Regenerate".into()),
            ],
        }
    }

    // `handle_input` keeps the `Component` default (no-op): the real input
    // path is the inherent `ThreadPanel::handle_input` above, which needs a
    // `client` the trait signature has no room for. See the module doc.
}

/// Render one turn's rows (question + status/body), appending to `out`.
/// Free function (no `&self` needed) — the "one method per concern" split
/// `render_turns` delegates to.
#[allow(clippy::too_many_arguments)]
fn render_turn(
    turn: &Turn,
    width: u16,
    is_first: bool,
    prompt: &str,
    question_style: Style,
    sep_style: Style,
    dim: Style,
    err: Style,
    md: &crate::components::markdown_lines::MdStyles,
    out: &mut Vec<(RowSlot, Line<'static>)>,
) {
    // A theme-dimmed rule parts each turn from the one above (never before the
    // first turn). It belongs to this turn — clicking it selects the turn.
    if !is_first {
        out.push((RowSlot::Turn(turn.id), separator_line(width, sep_style)));
    }
    let question = format!("{prompt} {}", turn.question);
    for qline in wrap_text(&question, width) {
        out.push((
            RowSlot::Turn(turn.id),
            Line::from(Span::styled(question[qline].to_string(), question_style)),
        ));
    }
    match &turn.status {
        TurnStatus::Thinking | TurnStatus::Streaming => {
            out.push((
                RowSlot::Turn(turn.id),
                Line::from(Span::styled("… thinking", dim)),
            ));
        }
        TurnStatus::Error(msg) => {
            let text = format!("✗ {msg}");
            for eline in wrap_text(&text, width) {
                out.push((
                    RowSlot::Turn(turn.id),
                    Line::from(Span::styled(text[eline].to_string(), err)),
                ));
            }
            out.push((
                RowSlot::Turn(turn.id),
                Line::from(Span::styled("  [r] retry", dim)),
            ));
        }
        TurnStatus::Done => render_answer(turn, width, md, out),
    }
    out.push((RowSlot::Turn(turn.id), Line::default()));
}

/// A full-width theme-dimmed horizontal rule (`─`) parting two turns.
fn separator_line(width: u16, style: Style) -> Line<'static> {
    Line::from(Span::styled("─".repeat(width as usize), style))
}

/// Render a `Done` turn's answer as styled markdown rows. Each *logical* source
/// line is classified (threading fenced-code state) and word-wrapped; each
/// wrapped slice keeps its byte range into `turn.answer` (so `RowSlot::Answer`
/// hit-testing stays aligned) and is styled by
/// `markdown_lines::style_slice_mapped`. That styler hides balanced emphasis
/// sigils, so the rendered columns no longer map 1:1 to the source — the slice's
/// `col_map` (stored on the `RowSlot`) carries `rendered col → source byte` for
/// the citation hit-test to walk.
fn render_answer(
    turn: &Turn,
    width: u16,
    md: &crate::components::markdown_lines::MdStyles,
    out: &mut Vec<(RowSlot, Line<'static>)>,
) {
    use crate::components::markdown_lines;
    let mut offset = 0usize;
    let mut in_fence = false;
    for logical in turn.answer.split_inclusive('\n') {
        let stripped = logical.strip_suffix('\n').unwrap_or(logical);
        let kind = markdown_lines::classify(stripped, &mut in_fence);
        let line_start = offset;
        for rel in wrap_text(stripped, width) {
            let abs = (line_start + rel.start)..(line_start + rel.end);
            let (line, col_map) = markdown_lines::style_slice_mapped(&turn.answer[abs.clone()], kind, md);
            out.push((
                RowSlot::Answer {
                    turn_id: turn.id,
                    range: abs,
                    col_map,
                },
                line,
            ));
        }
        offset += logical.len();
    }
}

/// Map a mouse click's column (relative to the wrapped line's own left edge) to
/// the citation it landed on, if any. `slice` is the wrapped line's source
/// text and `map` its `rendered char index → byte offset in slice` column map
/// (from `style_slice_mapped`): walking `map` by rendered display width steps
/// past any hidden emphasis sigils, so a click on or after them still resolves
/// to the right source byte — and thence the right `[n]`.
fn citation_at_column(slice: &str, map: &[usize], col: u16) -> Option<usize> {
    let mut w: u16 = 0;
    for &raw in map {
        let ch = slice[raw..].chars().next()?;
        let cw = (ch.width().unwrap_or(0) as u16).max(1);
        if col < w + cw {
            return citations::scan(slice)
                .into_iter()
                .find(|c| c.range.contains(&raw))
                .map(|c| c.index);
        }
        w += cw;
    }
    None
}

/// Greedy word-wrap: break `text` into lines no wider than `width` display
/// columns, wrapping at spaces (a single word wider than `width` overflows
/// its own line rather than being split). Existing newlines force a break.
/// Returns byte ranges into `text`, trimmed of the separating whitespace, so
/// both rendering (`render_turn`) and mouse hit-testing
/// (`citation_at_column`) stay in lock-step by construction — there's no
/// second wrapping pass (e.g. `Paragraph::wrap`) to disagree with this one.
fn wrap_text(text: &str, width: u16) -> Vec<Range<usize>> {
    let width = width.max(1) as usize;
    let mut lines = Vec::new();
    let mut para_start = 0;
    for (i, ch) in text.char_indices() {
        if ch == '\n' {
            wrap_paragraph(text, para_start..i, width, &mut lines);
            para_start = i + 1;
        }
    }
    wrap_paragraph(text, para_start..text.len(), width, &mut lines);
    lines
}

/// Word-wrap a single (newline-free) paragraph range, appending to `out`.
fn wrap_paragraph(text: &str, para: Range<usize>, width: usize, out: &mut Vec<Range<usize>>) {
    let words = word_ranges(text, para.clone());
    let Some(first) = words.first() else {
        out.push(para.start..para.start);
        return;
    };
    let mut line_start = first.start;
    let mut line_end = first.end;
    let mut line_w = text[first.clone()].width();
    for w in &words[1..] {
        let word_w = text[w.clone()].width();
        if line_w + 1 + word_w > width {
            out.push(line_start..line_end);
            line_start = w.start;
            line_end = w.end;
            line_w = word_w;
        } else {
            line_end = w.end;
            line_w += 1 + word_w;
        }
    }
    out.push(line_start..line_end);
}

/// Byte ranges of each space-separated word within `range`. Splits on ASCII
/// space only (`b' '` never appears as a UTF-8 continuation byte, so this is
/// always a safe char-boundary split).
fn word_ranges(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut words = Vec::new();
    let mut i = range.start;
    while i < range.end {
        while i < range.end && bytes[i] == b' ' {
            i += 1;
        }
        if i >= range.end {
            break;
        }
        let start = i;
        while i < range.end && bytes[i] != b' ' {
            i += 1;
        }
        words.push(start..i);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    /// A throwaway RAG client — never actually called (localhost:0), just
    /// present so the composer is enabled.
    fn test_client() -> Arc<RagClient> {
        Arc::new(RagClient::new(
            "http://localhost:0".to_string(),
            None,
            "vault".to_string(),
        ))
    }

    /// An offline panel (no client) with pending composer text.
    fn test_panel() -> ThreadPanel {
        let mut p = ThreadPanel::new();
        p.composer.set_value("q");
        p
    }

    fn test_panel_online() -> ThreadPanel {
        let mut p = ThreadPanel::new();
        p.set_client(Some(test_client()));
        p
    }

    fn p_handle_enter(p: &mut ThreadPanel) -> EventState {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        p.handle_input(&InputEvent::Key(key), &tx)
    }

    #[test]
    fn new_thread_panel_starts_empty_without_client_and_composer_focus() {
        let panel = ThreadPanel::new();
        assert!(panel.thread().is_empty());
        assert!(!panel.has_client());
        assert_eq!(panel.focus, ThreadFocus::Composer);
    }

    #[test]
    fn set_client_toggles_the_composer_enable_signal() {
        let mut panel = ThreadPanel::new();
        assert!(!panel.has_client());
        panel.set_client(Some(test_client()));
        assert!(panel.has_client());
        panel.set_client(None);
        assert!(!panel.has_client());
    }

    #[test]
    fn thread_mut_allows_mutating_the_conversation() {
        let mut panel = ThreadPanel::new();
        panel.thread_mut().ask("q?".to_string());
        assert_eq!(panel.thread().turns().len(), 1);
    }

    #[tokio::test]
    async fn enter_submits_only_with_a_client() {
        let mut p = test_panel(); // "q" pending, no client
        let _ = p_handle_enter(&mut p);
        assert!(p.thread().is_empty(), "no client → no turn");

        // A client enables submission; the composer text (untouched by the
        // clientless attempt) now pushes a Thinking turn. The spawned job runs
        // in the background — we only assert the synchronous half here.
        p.set_client(Some(test_client()));
        let _ = p_handle_enter(&mut p);
        assert_eq!(p.thread().turns().len(), 1);
        assert!(matches!(
            p.thread().selected().unwrap().status,
            TurnStatus::Thinking
        ));
    }

    #[test]
    fn answer_ready_completes_matching_turn_only() {
        let mut p = test_panel_online();
        let id = p.thread_mut().ask("q".into());
        p.handle_data(AskData::AnswerReady {
            turn_id: 999,
            result: Ok(("x".into(), vec![])),
        });
        assert!(matches!(
            p.thread().selected().unwrap().status,
            TurnStatus::Thinking
        ));
        p.handle_data(AskData::AnswerReady {
            turn_id: id,
            result: Ok(("a".into(), vec![])),
        });
        assert!(matches!(
            p.thread().selected().unwrap().status,
            TurnStatus::Done
        ));
    }

    #[test]
    fn begin_turn_is_none_without_a_client() {
        let mut p = ThreadPanel::new(); // no client
        p.composer.set_value("hello");
        assert!(p.begin_turn().is_none());
        assert!(
            p.thread().is_empty(),
            "no client → no orphaned Thinking turn"
        );
    }

    #[test]
    fn begin_turn_is_none_when_composer_empty() {
        let mut p = ThreadPanel::new();
        p.set_client(Some(test_client()));
        p.composer.set_value("   ");
        assert!(p.begin_turn().is_none());
        assert!(p.thread().is_empty());
    }

    #[test]
    fn begin_turn_pushes_a_thinking_turn_and_selects_it() {
        let mut p = ThreadPanel::new();
        p.set_client(Some(test_client()));
        p.composer.set_value("hello");
        let (question, history, turn_id) = p.begin_turn().expect("client + non-empty");
        assert_eq!(question, "hello");
        assert!(history.is_empty());
        assert_eq!(p.thread().turns().len(), 1);
        assert_eq!(p.thread().selected().unwrap().id, turn_id);
    }

    #[test]
    fn esc_in_composer_moves_focus_to_turns() {
        let mut p = ThreadPanel::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let state = p.handle_input(&InputEvent::Key(key), &tx);
        assert_eq!(state, EventState::Consumed);
        assert_eq!(p.focus, ThreadFocus::Turns);
    }

    #[test]
    fn jk_in_turns_moves_selection() {
        let mut p = ThreadPanel::new();
        let first = p.thread_mut().ask("a".into());
        p.thread_mut().complete(first, "a!".into(), vec![]);
        let second = p.thread_mut().ask("b".into());
        p.thread_mut().complete(second, "b!".into(), vec![]);
        p.focus = ThreadFocus::Turns;

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let key = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE);
        p.handle_input(&InputEvent::Key(key), &tx);
        assert_eq!(p.thread().selected().unwrap().id, first);
    }

    #[test]
    fn regenerate_without_a_client_does_nothing() {
        // Without a client, regenerate must not even rewind the turn — the
        // client check comes before the rewind, so no orphaned Thinking turn.
        let mut p = ThreadPanel::new();
        let id = p.thread_mut().ask("q".into());
        p.thread_mut().complete(id, "a".into(), vec![]);
        p.focus = ThreadFocus::Turns;

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let key = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE);
        p.handle_input(&InputEvent::Key(key), &tx);
        assert!(
            matches!(p.thread().selected().unwrap().status, TurnStatus::Done),
            "no client → the completed turn stays Done"
        );
        assert_eq!(p.thread().selected().unwrap().id, id);
    }

    #[test]
    fn i_and_slash_move_focus_to_composer() {
        for ch in ['i', '/'] {
            let mut p = ThreadPanel::new();
            p.focus = ThreadFocus::Turns;
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            p.handle_input(&InputEvent::Key(key), &tx);
            assert_eq!(p.focus, ThreadFocus::Composer);
        }
    }

    #[test]
    fn wrap_text_breaks_on_spaces_within_width() {
        let lines = wrap_text("one two three", 7);
        let text = "one two three";
        let rendered: Vec<&str> = lines.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(rendered, vec!["one two", "three"]);
    }

    #[test]
    fn wrap_text_keeps_an_overlong_word_on_its_own_line() {
        let lines = wrap_text("a superlongword b", 5);
        let text = "a superlongword b";
        let rendered: Vec<&str> = lines.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(rendered, vec!["a", "superlongword", "b"]);
    }

    #[test]
    fn wrap_text_forces_a_break_on_newline() {
        let lines = wrap_text("a\nb", 10);
        let text = "a\nb";
        let rendered: Vec<&str> = lines.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(rendered, vec!["a", "b"]);
    }

    /// Identity column map (rendered col == byte offset) for a slice with no
    /// hidden sigils — the common test fixture.
    fn identity_map(slice: &str) -> Vec<usize> {
        slice.char_indices().map(|(i, _)| i).collect()
    }

    #[test]
    fn citation_at_column_finds_the_marker_under_the_click() {
        let text = "Fact [1] more";
        let map = identity_map(text);
        let idx = citation_at_column(text, &map, 5);
        assert_eq!(idx, Some(1));
        let idx = citation_at_column(text, &map, 0);
        assert_eq!(idx, None);
    }

    /// With emphasis sigils hidden, a click on `[1]`'s *rendered* column must
    /// still resolve to citation 1 — the col_map steps past the dropped `**`.
    /// Exercises a line with an emphasis run before the citation.
    #[test]
    fn citation_hit_test_resolves_through_hidden_emphasis() {
        use crate::components::markdown_lines::{self, LineKind, MdStyles};
        let md = MdStyles::from_theme(&Theme::default());
        let raw = "**bold** then [1] tail";
        let (line, col_map) =
            markdown_lines::style_slice_mapped(raw, LineKind::Normal, &md);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "bold then [1] tail");
        // The rendered `[1]` starts at column 10 ("bold then " = 10 cols).
        let col = text.find("[1]").unwrap() as u16 + 1; // inside the marker
        assert_eq!(citation_at_column(raw, &col_map, col), Some(1));
    }

    #[test]
    fn click_turns_selects_turn_and_resolves_citation_target() {
        let mut p = ThreadPanel::new();
        let first = p.thread_mut().ask("a".into());
        p.thread_mut().complete(
            first,
            "See [1] for it".into(),
            vec![AskSource {
                path: kimun_core::nfs::VaultPath::new("a.md"),
                heading: "h".into(),
                date: None,
                score: 1.0,
                text: String::new(),
                ordinal: 1,
            }],
        );
        let second = p.thread_mut().ask("b".into());
        p.thread_mut().complete(second, "b!".into(), vec![]);
        // Currently selected: `second`. Simulate a render so row_map/turns_rect exist.
        p.turns_rect = Rect::new(0, 0, 40, 20);
        let answer_slice = "See [1] for it";
        p.row_map = vec![
            RowSlot::Turn(first),
            RowSlot::Answer {
                turn_id: first,
                range: 0..answer_slice.len(),
                col_map: identity_map(answer_slice),
            },
            RowSlot::Turn(first),
            RowSlot::Turn(second),
        ];
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 4, // inside "[1]"
            row: 1,
            modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
        };
        p.click_turns(&mouse);
        assert_eq!(p.thread().selected().unwrap().id, first);
        // Stores the citation ordinal (`[1]`), resolved through the pairing seam.
        assert_eq!(p.take_citation_target(), Some(1));
    }

    /// A theme-dimmed rule parts turns (never before the first, never trailing
    /// the last), and the question line stands out — accent+bold with the
    /// prompt glyph.
    #[test]
    fn separators_part_turns_and_question_line_stands_out() {
        use crate::components::markdown_lines::MdStyles;
        let theme = Theme::default();
        let md = MdStyles::from_theme(&theme);
        let qstyle = Style::default()
            .fg(theme.accent.to_ratatui())
            .add_modifier(Modifier::BOLD);
        let sep = Style::default()
            .fg(theme.gray.to_ratatui())
            .add_modifier(Modifier::DIM);
        let dim = Style::default();
        let err = Style::default();

        let mut thread = Thread::default();
        let a = thread.ask("first".into());
        thread.complete(a, "ans a".into(), vec![]);
        let b = thread.ask("second".into());
        thread.complete(b, "ans b".into(), vec![]);

        let mut rows: Vec<(RowSlot, Line<'static>)> = Vec::new();
        for (i, turn) in thread.turns().iter().enumerate() {
            render_turn(turn, 40, i == 0, ">", qstyle, sep, dim, err, &md, &mut rows);
        }

        let is_sep = |l: &Line<'static>| l.spans.iter().any(|s| s.content.contains('─'));
        // Two turns → exactly one divider, opening the second turn, never last.
        assert_eq!(rows.iter().filter(|(_, l)| is_sep(l)).count(), 1);
        assert!(!is_sep(&rows[0].1), "no rule before the first turn");
        let sep_idx = rows.iter().position(|(_, l)| is_sep(l)).unwrap();
        assert!(matches!(rows[sep_idx].0, RowSlot::Turn(id) if id == b));
        assert_ne!(sep_idx, rows.len() - 1, "no rule after the last turn");

        // The question row: prompt glyph + accent-bold styling.
        let (_, qline) = rows
            .iter()
            .find(|(_, l)| l.spans.iter().any(|s| s.content.contains("first")))
            .unwrap();
        assert!(qline.spans[0].content.starts_with('>'), "carries the prompt");
        assert_eq!(qline.spans[0].style, qstyle, "accent + bold");
    }

    mod rendering {
        use super::*;
        use crate::settings::themes::Theme;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        fn draw(p: &mut ThreadPanel, theme: &Theme, width: u16, height: u16, focused: bool) {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|f| {
                    let area = f.area();
                    p.render(f, area, theme, focused);
                })
                .unwrap();
        }

        /// Clicking the separator row that opens a turn selects that turn — the
        /// rule joins `row_map` as a `Turn(id)` slot.
        #[test]
        fn clicking_a_separator_row_selects_its_turn() {
            let theme = Theme::default();
            let mut p = ThreadPanel::new();
            let a = p.thread_mut().ask("first".into());
            p.thread_mut().complete(a, "aaa".into(), vec![]);
            let b = p.thread_mut().ask("second".into());
            p.thread_mut().complete(b, "bbb".into(), vec![]);
            p.focus = ThreadFocus::Turns;
            // Select the first turn so a separator click can move selection.
            p.thread_mut().select_index(0);
            draw(&mut p, &theme, 40, 12, true);

            // The first visible row mapped to `b` is its opening separator.
            let sep_row = p
                .row_map
                .iter()
                .position(|s| matches!(s, RowSlot::Turn(id) if *id == b))
                .expect("turn b has rows on screen");
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            let mouse = MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 0,
                row: p.turns_rect.y + sep_row as u16,
                modifiers: KeyModifiers::NONE,
            };
            p.handle_input(&InputEvent::Mouse(mouse), &tx);
            assert_eq!(p.thread().selected().unwrap().id, b);
        }

        #[test]
        fn render_does_not_panic_across_states_and_sizes() {
            let theme = Theme::default();
            let mut p = ThreadPanel::new();
            p.set_client(Some(test_client())); // enabled composer render path
            draw(&mut p, &theme, 40, 10, true); // empty thread

            let id = p
                .thread_mut()
                .ask("A fairly long question that should wrap across more than one line".into());
            draw(&mut p, &theme, 40, 10, true); // Thinking

            p.thread_mut().complete(
                id,
                "An answer citing [1] a source and [2] another, spanning multiple \
                 wrapped lines to exercise citation styling."
                    .into(),
                vec![],
            );
            draw(&mut p, &theme, 40, 10, true); // Done, focused on Turns
            p.focus = ThreadFocus::Turns;
            draw(&mut p, &theme, 40, 10, true);

            let id2 = p.thread_mut().ask("another".into());
            p.thread_mut().fail(id2, "boom".into());
            draw(&mut p, &theme, 40, 10, true); // Error

            p.set_client(None);
            draw(&mut p, &theme, 40, 10, false); // disabled, unfocused

            draw(&mut p, &theme, 3, 3, true); // degenerate tiny rect
            draw(&mut p, &theme, 0, 0, true); // zero rect
        }

        fn turns_key(p: &mut ThreadPanel, code: KeyCode) {
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            p.handle_input(&InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)), &tx);
        }

        /// A markdown answer (heading + prose citation + fenced code block)
        /// renders through the row map, and the prose citation stays clickable:
        /// the rendered answer slices still map 1:1 to the source, so
        /// `citation_at_column` resolves the marker. (Per-block styling is
        /// unit-tested in `markdown_lines`.) Covers I2's hit-testing constraint.
        #[test]
        fn markdown_answer_keeps_prose_citations_clickable() {
            let theme = Theme::default();
            let mut p = ThreadPanel::new();
            let id = p.thread_mut().ask("q".into());
            let answer =
                "# Title\nSee [1] here.\n```\nlet x = arr[9];\n```".to_string();
            p.thread_mut().complete(
                id,
                answer.clone(),
                vec![AskSource {
                    path: kimun_core::nfs::VaultPath::new("a.md"),
                    heading: "h".into(),
                    date: None,
                    score: 1.0,
                    text: String::new(),
                    ordinal: 1,
                }],
            );
            p.focus = ThreadFocus::Turns;
            draw(&mut p, &theme, 60, 12, true);

            // Find the rendered answer row carrying the prose `[1]` and hit-test
            // the marker's column through its stored col_map.
            let hit = p.row_map.iter().find_map(|slot| match slot {
                RowSlot::Answer {
                    range, col_map, ..
                } if answer[range.clone()].contains("[1]") => {
                    let slice = &answer[range.clone()];
                    let col = slice.find("[1]").unwrap() as u16 + 1;
                    Some(citation_at_column(slice, col_map, col))
                }
                _ => None,
            });
            assert_eq!(
                hit,
                Some(Some(1)),
                "the prose citation resolves through the rendered slice"
            );
        }

        /// A completed answer taller than the viewport scrolls so its end is
        /// visible (bottom-follow), not stuck showing the question.
        #[test]
        fn completion_bottom_follows_to_show_the_answer_end() {
            let theme = Theme::default();
            let mut p = ThreadPanel::new();
            p.set_client(Some(test_client()));
            let id = p.thread_mut().ask("q".into());
            p.focus = ThreadFocus::Turns;
            // 10 answer lines. Rows: question(1) + 10 + trailing blank(1) = 12.
            let answer = (0..10).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
            p.handle_data(AskData::AnswerReady {
                turn_id: id,
                result: Ok((answer, vec![])),
            });
            // Terminal height 8 − composer(3) = 5 turn rows. Rows total 12
            // (question 1 + 10 answer + trailing blank 1) → end pins at 12 − 5 = 7.
            draw(&mut p, &theme, 60, 8, true);
            assert_eq!(p.scroll, 7, "bottom-follow shows the answer's end");
        }

        /// Selecting an off-screen turn brings it into view; content-scroll keys
        /// clamp to the wrapped-row total.
        #[test]
        fn selection_scrolls_into_view_and_content_scroll_clamps() {
            let theme = Theme::default();
            let mut p = ThreadPanel::new();
            for i in 0..8 {
                let id = p.thread_mut().ask(format!("q{i}"));
                p.thread_mut().complete(id, format!("a{i}"), vec![]);
            }
            p.focus = ThreadFocus::Turns;
            // First turn: question(1)+answer(1)+blank(1) = 3 rows. Each later
            // turn adds a leading separator: separator(1)+question(1)+answer(1)+
            // blank(1) = 4 rows. 8 turns → 3 + 7×4 = 31 rows.
            // Terminal height 9 − composer(3) = 6 turn rows.
            draw(&mut p, &theme, 60, 9, true); // selection at the last turn

            // Jump the selection to the first turn: it scrolls to the top.
            for _ in 0..8 {
                turns_key(&mut p, KeyCode::Char('k'));
            }
            draw(&mut p, &theme, 60, 9, true);
            assert_eq!(p.scroll, 0, "selecting the first turn scrolled it into view");

            // End scrolls to the bottom, clamped to total − height (31 − 6 = 25).
            turns_key(&mut p, KeyCode::End);
            draw(&mut p, &theme, 60, 9, true);
            assert_eq!(p.scroll, 25, "content scroll clamps to the last page");
        }
    }
}
