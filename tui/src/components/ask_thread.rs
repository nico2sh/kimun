//! `ThreadPanel` — the editor area's Ask-workspace content (see CONTEXT.md:
//! **Ask workspace**, **Thread**; adr/0030). Owns the conversation `Thread`
//! and the docked question composer.
//!
//! `PanelSet` hands this back to its caller via `take_ask` (rather than
//! dropping it, the way `clear_attachment` drops an `AttachmentView`) so the
//! conversation survives the user switching to another editor-area view.
//!
//! Input runs through the inherent [`ThreadPanel::handle_input`], not the
//! `Component` trait method: submitting/regenerating a turn needs an
//! optional `&Arc<RagClient>` that the generic `dyn Component` dispatch
//! (`PanelSet::editor_area_mut`) has no way to thread through. Wiring the
//! editor screen to call this method directly (mirroring the typed
//! `ask_mut()` accessor) is Task 11's job; the `Component` impl here only
//! covers `render` (and default no-op input), same boundary Task 8 left it
//! at.

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
use crate::settings::themes::Theme;

/// Height (in rows) of the docked composer box, borders included.
const COMPOSER_HEIGHT: u16 = 3;

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
    /// One word-wrapped line of a turn's answer body: the turn it belongs to
    /// plus the line's byte range into `turn.answer`, so a click's column can
    /// resolve to a specific citation (`citation_at_column`).
    Answer { turn_id: u64, range: Range<usize> },
}

/// The Ask workspace's editor-area content: the conversation `Thread` plus
/// the docked question composer. See the module doc for lifetime notes.
pub struct ThreadPanel {
    thread: Thread,
    composer: SingleLineInput,
    /// Whether the Ask capability (Kimün server reachable with an LLM
    /// configured) is currently available. Losing it disables the composer
    /// without evicting the thread — the thread's answers are already local
    /// (CONTEXT.md: **Ask workspace**).
    capability: bool,
    focus: ThreadFocus,
    /// Topmost visible row of the flattened turn-lines list, auto-adjusted on
    /// selection change to keep the selected turn in view.
    scroll: u16,
    /// Source row index a citation click asked the Sources drawer to focus —
    /// cleared on read via `take_citation_target`.
    citation_target: Option<usize>,
    /// The turn list's rect from the last render — mouse hit-testing base.
    turns_rect: Rect,
    /// The composer's rect from the last render — mouse hit-testing base.
    composer_rect: Rect,
    /// Row → data mapping from the last render, scoped to `turns_rect`.
    row_map: Vec<RowSlot>,
}

impl ThreadPanel {
    pub fn new() -> Self {
        Self {
            thread: Thread::default(),
            composer: SingleLineInput::new(),
            capability: true,
            focus: ThreadFocus::Composer,
            scroll: 0,
            citation_target: None,
            turns_rect: Rect::default(),
            composer_rect: Rect::default(),
            row_map: Vec::new(),
        }
    }

    /// Update whether the Ask capability is currently available. See
    /// adr/0030: losing capability disables the composer but never evicts
    /// the thread.
    pub fn set_capability(&mut self, on: bool) {
        self.capability = on;
    }

    /// Whether the Ask capability is currently on — the wiring seam reads this
    /// to assert capability⇔client lockstep (Task 11).
    pub fn capability(&self) -> bool {
        self.capability
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

    /// Handle an input event. `client` is the live RAG client when the Ask
    /// capability is up; `None` disables submission/regeneration (their
    /// synchronous half — pushing/rewinding a turn — still runs, matching
    /// `capability`, but nothing is ever spawned without a client).
    pub fn handle_input(
        &mut self,
        event: &InputEvent,
        tx: &AppTx,
        client: Option<&Arc<RagClient>>,
    ) -> EventState {
        match event {
            InputEvent::Key(key) => self.handle_key(key, tx, client),
            InputEvent::Mouse(mouse) => self.handle_mouse(mouse, tx),
            InputEvent::Paste(_) => EventState::NotConsumed,
        }
    }

    pub fn handle_data(&mut self, data: AskData) {
        if let AskData::AnswerReady { turn_id, result } = data {
            match result {
                Ok((answer, sources)) => {
                    self.thread.complete(turn_id, answer, sources);
                }
                Err(e) => {
                    self.thread.fail(turn_id, e);
                }
            }
        }
        // `ReaderNote` is addressed to the source reader (Task 10), not here.
    }

    fn handle_key(
        &mut self,
        key: &KeyEvent,
        tx: &AppTx,
        client: Option<&Arc<RagClient>>,
    ) -> EventState {
        match self.focus {
            ThreadFocus::Composer => self.handle_composer_key(key, tx, client),
            ThreadFocus::Turns => self.handle_turns_key(key, tx, client),
        }
    }

    fn handle_composer_key(
        &mut self,
        key: &KeyEvent,
        tx: &AppTx,
        client: Option<&Arc<RagClient>>,
    ) -> EventState {
        if key.code == KeyCode::Esc {
            self.focus = ThreadFocus::Turns;
            return EventState::Consumed;
        }
        match self.composer.handle_key(key) {
            InputOutcome::Submit => {
                self.submit(tx, client);
                EventState::Consumed
            }
            InputOutcome::NotConsumed => EventState::NotConsumed,
            _ => EventState::Consumed,
        }
    }

    fn handle_turns_key(
        &mut self,
        key: &KeyEvent,
        tx: &AppTx,
        client: Option<&Arc<RagClient>>,
    ) -> EventState {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.thread.select_prev();
                EventState::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.thread.select_next();
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
                self.regenerate_selected(tx, client);
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
                self.scroll = self.scroll.saturating_sub(1);
                EventState::Consumed
            }
            MouseEventKind::ScrollDown if self.turns_rect.contains(pos) => {
                self.scroll = self.scroll.saturating_add(1);
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
            RowSlot::Answer { turn_id, range } => (*turn_id, Some(range.clone())),
        });
        let Some((turn_id, answer_range)) = hit else {
            return;
        };
        self.select_turn(turn_id);
        let Some(range) = answer_range else {
            return;
        };
        let col = mouse.column.saturating_sub(self.turns_rect.x);
        let Some(turn) = self.thread.selected() else {
            return;
        };
        let Some(citation_idx) = citation_at_column(&turn.answer, range, col) else {
            return;
        };
        let source_idx = citation_idx.saturating_sub(1);
        if source_idx < turn.sources.len() {
            self.citation_target = Some(source_idx);
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

    /// Pre-spawn half of `submit`, factored out for testability (no
    /// `RagClient` needed): validates the Ask capability and non-empty
    /// composer text, pushes a `Thinking` turn, and returns what the spawn
    /// needs. `None` — and no thread mutation — when the capability is off
    /// or the composer is (effectively) empty.
    fn begin_turn(&mut self) -> Option<PendingTurn> {
        if !self.capability {
            return None;
        }
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
        Some((question, history, turn_id))
    }

    /// Submit the composer's question. With a live `client` this spawns the
    /// ask job, delivering `AppEvent::Ask(AskData::AnswerReady)` on
    /// completion; without one the turn stays `Thinking` (an
    /// capability/client mismatch is a caller bug, not something resolved
    /// here — see the `handle_input` doc comment).
    fn submit(&mut self, tx: &AppTx, client: Option<&Arc<RagClient>>) {
        let Some((question, history, turn_id)) = self.begin_turn() else {
            return;
        };
        let Some(client) = client else {
            return;
        };
        Self::spawn_ask(tx, client, question, history, turn_id);
    }

    /// Rewind the selected turn back to `Thinking` and re-ask its question.
    /// The server always re-retrieves context, so the completion carries fresh
    /// sources — the `[n]` markers in the new answer are numbered against that
    /// fresh context, and replacing the turn's sources keeps citations, reader
    /// targets, and saved-note wikilinks aligned. (`Thread::regenerate` leaves
    /// the old sources in place while the turn is `Thinking`, so the previous
    /// evidence stays visible during regeneration; only completion swaps them.)
    /// No-op without capability, without a client, or when the selected turn is
    /// currently in flight (`Thread::regenerate` rejects that case). Leader `a r`.
    pub(crate) fn regenerate_selected(&mut self, tx: &AppTx, client: Option<&Arc<RagClient>>) {
        if !self.capability {
            return;
        }
        let Some(id) = self.thread.selected().map(|t| t.id) else {
            return;
        };
        let Some(question) = self.thread.regenerate(id) else {
            return;
        };
        let Some(client) = client else {
            return;
        };
        let history = self.thread.history();
        Self::spawn_ask(tx, client, question, history, id);
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
                    (
                        a.answer,
                        a.sources.into_iter().map(AskSource::from).collect(),
                    )
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

        let base = Style::default().fg(theme.fg.to_ratatui());
        let bold = Style::default()
            .fg(theme.fg_bright.to_ratatui())
            .add_modifier(Modifier::BOLD);
        let accent = Style::default().fg(theme.accent.to_ratatui());
        let dim = Style::default().fg(theme.gray.to_ratatui());
        let err = Style::default().fg(theme.red.to_ratatui());

        let mut rows: Vec<(RowSlot, Line<'static>)> = Vec::new();
        let mut turn_start_row: Vec<(u64, u16)> = Vec::new();
        for turn in self.thread.turns() {
            turn_start_row.push((turn.id, rows.len() as u16));
            render_turn(turn, rect.width, base, bold, accent, dim, err, &mut rows);
        }
        let total = rows.len() as u16;
        let height = rect.height;

        // Auto-scroll to keep the selected turn's start row in view.
        if let Some(sel) = self.thread.selected()
            && let Some(&(_, start)) = turn_start_row.iter().find(|(id, _)| *id == sel.id)
        {
            if start < self.scroll {
                self.scroll = start;
            } else if height > 0 && start >= self.scroll + height {
                self.scroll = start.saturating_sub(height - 1);
            }
        }
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

        let title = if self.capability {
            "Ask a question"
        } else {
            "server unavailable"
        };
        let block = panel_block(title, theme, focused);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let style = if self.capability {
            Style::default().fg(theme.fg.to_ratatui())
        } else {
            Style::default()
                .fg(theme.gray.to_ratatui())
                .add_modifier(Modifier::DIM)
        };
        self.composer
            .render(f, inner, style, 0, focused && self.capability);
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
    base: Style,
    bold: Style,
    accent: Style,
    dim: Style,
    err: Style,
    out: &mut Vec<(RowSlot, Line<'static>)>,
) {
    let question = format!("> {}", turn.question);
    for qline in wrap_text(&question, width) {
        out.push((
            RowSlot::Turn(turn.id),
            Line::from(Span::styled(question[qline].to_string(), bold)),
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
        TurnStatus::Done => {
            for aline in wrap_text(&turn.answer, width) {
                out.push((
                    RowSlot::Answer {
                        turn_id: turn.id,
                        range: aline.clone(),
                    },
                    citation_styled_line(&turn.answer, aline, base, accent),
                ));
            }
        }
    }
    out.push((RowSlot::Turn(turn.id), Line::default()));
}

/// Build a styled `Line` for one wrapped line of an answer: plain text in
/// `base`, `[n]` citation markers in `accent`.
fn citation_styled_line(
    text: &str,
    range: Range<usize>,
    base: Style,
    accent: Style,
) -> Line<'static> {
    let slice = &text[range];
    let mut spans = Vec::new();
    let mut last = 0;
    for c in citations::scan(slice) {
        if c.range.start > last {
            spans.push(Span::styled(slice[last..c.range.start].to_string(), base));
        }
        spans.push(Span::styled(slice[c.range.clone()].to_string(), accent));
        last = c.range.end;
    }
    if last < slice.len() {
        spans.push(Span::styled(slice[last..].to_string(), base));
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    Line::from(spans)
}

/// Map a mouse click's column (relative to the wrapped line's own left edge)
/// to the citation it landed on, if any. `range` must be a line byte range
/// produced by `wrap_text` over `text` — the same one `render_turn` sliced to
/// build the line, so clicks resolve against exactly what's on screen.
fn citation_at_column(text: &str, range: Range<usize>, col: u16) -> Option<usize> {
    let slice = &text[range];
    let mut w: u16 = 0;
    for (i, ch) in slice.char_indices() {
        let cw = (ch.width().unwrap_or(0) as u16).max(1);
        if col < w + cw {
            return citations::scan(slice)
                .into_iter()
                .find(|c| c.range.contains(&i))
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

    fn test_panel() -> ThreadPanel {
        let mut p = ThreadPanel::new();
        p.set_capability(false);
        p.composer.set_value("q");
        p
    }

    fn test_panel_online() -> ThreadPanel {
        ThreadPanel::new()
    }

    fn p_handle_enter(p: &mut ThreadPanel) -> EventState {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        p.handle_input(&InputEvent::Key(key), &tx, None)
    }

    #[test]
    fn new_thread_panel_starts_empty_with_capability_and_composer_focus() {
        let panel = ThreadPanel::new();
        assert!(panel.thread().is_empty());
        assert!(panel.capability);
        assert_eq!(panel.focus, ThreadFocus::Composer);
    }

    #[test]
    fn set_capability_toggles_the_flag() {
        let mut panel = ThreadPanel::new();
        panel.set_capability(false);
        assert!(!panel.capability);
        panel.set_capability(true);
        assert!(panel.capability);
    }

    #[test]
    fn thread_mut_allows_mutating_the_conversation() {
        let mut panel = ThreadPanel::new();
        panel.thread_mut().ask("q?".to_string());
        assert_eq!(panel.thread().turns().len(), 1);
    }

    #[test]
    fn enter_submits_only_with_capability() {
        let mut p = test_panel();
        let _ = p_handle_enter(&mut p);
        assert!(p.thread().is_empty(), "no capability → no turn");

        p.set_capability(true);
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
    fn begin_turn_is_none_when_composer_empty() {
        let mut p = ThreadPanel::new();
        p.composer.set_value("   ");
        assert!(p.begin_turn().is_none());
        assert!(p.thread().is_empty());
    }

    #[test]
    fn begin_turn_pushes_a_thinking_turn_and_selects_it() {
        let mut p = ThreadPanel::new();
        p.composer.set_value("hello");
        let (question, history, turn_id) = p.begin_turn().expect("capability + non-empty");
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
        let state = p.handle_input(&InputEvent::Key(key), &tx, None);
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
        p.handle_input(&InputEvent::Key(key), &tx, None);
        assert_eq!(p.thread().selected().unwrap().id, first);
    }

    #[test]
    fn regenerate_without_client_still_rewinds_but_does_not_spawn() {
        let mut p = ThreadPanel::new();
        let id = p.thread_mut().ask("q".into());
        p.thread_mut().complete(id, "a".into(), vec![]);
        p.focus = ThreadFocus::Turns;

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let key = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE);
        p.handle_input(&InputEvent::Key(key), &tx, None);
        assert!(matches!(
            p.thread().selected().unwrap().status,
            TurnStatus::Thinking
        ));
        assert_eq!(p.thread().selected().unwrap().id, id);
    }

    #[test]
    fn i_and_slash_move_focus_to_composer() {
        for ch in ['i', '/'] {
            let mut p = ThreadPanel::new();
            p.focus = ThreadFocus::Turns;
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            p.handle_input(&InputEvent::Key(key), &tx, None);
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

    #[test]
    fn citation_at_column_finds_the_marker_under_the_click() {
        let text = "Fact [1] more";
        let idx = citation_at_column(text, 0..text.len(), 5);
        assert_eq!(idx, Some(1));
        let idx = citation_at_column(text, 0..text.len(), 0);
        assert_eq!(idx, None);
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
                score: 1.0,
                text: String::new(),
            }],
        );
        let second = p.thread_mut().ask("b".into());
        p.thread_mut().complete(second, "b!".into(), vec![]);
        // Currently selected: `second`. Simulate a render so row_map/turns_rect exist.
        p.turns_rect = Rect::new(0, 0, 40, 20);
        p.row_map = vec![
            RowSlot::Turn(first),
            RowSlot::Answer {
                turn_id: first,
                range: 0.."See [1] for it".len(),
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
        assert_eq!(p.take_citation_target(), Some(0));
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

        #[test]
        fn render_does_not_panic_across_states_and_sizes() {
            let theme = Theme::default();
            let mut p = ThreadPanel::new();
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

            p.set_capability(false);
            draw(&mut p, &theme, 40, 10, false); // disabled, unfocused

            draw(&mut p, &theme, 3, 3, true); // degenerate tiny rect
            draw(&mut p, &theme, 0, 0, true); // zero rect
        }
    }
}
