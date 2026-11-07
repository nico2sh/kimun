//! The **Thread** (M7c G10a): the Ask workspace's conversation, shown in the
//! editor Panel in place of the editor (CONTEXT.md: **Ask workspace**,
//! **Thread**, **Turn**). The thread model and its rules are the logic's
//! (`kimun_notes::ask::Thread`, `ask::jobs`, `ask::save`,
//! `logic::ask_flow::AskFlow`); this file draws it and drives it.
//!
//! **The view.** The turns, oldest first, in one scrolling column: each
//! turn's question, then its answer, or a status line while it is pending
//! (`… thinking`) or failed (`✗ {message}` and the retry hint). The selected
//! turn (the one Copy, Save as note and Regenerate act on) has the selected
//! fill. Under the turns: the turn actions, then the composer (one line, the
//! TUI's) with Send. The actions are enabled by the TUI's rule
//! (`sync_actions`): Send needs a server and a question, the turn actions a
//! turn; Regenerate and New also need the server (the leader gates them so).
//!
//! **Answers** are drawn by the shared answer rules (`kimun_notes::ask::
//! markdown`, the TUI's too, over the editor's markdown model; mapped by
//! [`answer_lines`]), once, when the answer lands: code lines in the monospace face,
//! headings semibold, quotes muted, and on ordinary lines bold, italic and
//! struck-through runs with their sigils hidden and inline code verbatim on
//! a tint; the `[n]` citations in the accent colour. A citation that names
//! one of the turn's sources is a link (a pointing hand over it): a click
//! selects the turn and that source in the Sources view (the TUI's
//! `click_turns`; the reader is leader `a s` or Enter there).
//!
//! **Keys** (the TUI's). The composer: Enter sends; Escape gives the turn
//! list the keyboard; with the composer empty, Up/Down move the selection;
//! PageUp/PageDown scroll the turns. The turn list: Up/Down (k/j) move the
//! selection, PageUp/PageDown/Home/End scroll, i or / go back to the
//! composer, y copies, e saves as a note, r regenerates; Escape leaves the
//! Thread for the drawer (or the rail): the editor it replaced is gone.
//!
//! **Asking.** Send pushes a pending turn and spawns the logic's ask job on
//! tokio; its `AppMessage::Ask(AnswerReady)` completes or fails the turn
//! (a stale id is dropped by `Thread`). Regenerate rewinds the selected
//! turn and asks again. Both follow the bottom: a new turn, and an answer
//! that lands on the selected turn, scroll into view.
//!
//! **The workspace's side** (below): entering the Thread leaves the open
//! note the way the attachment view does (`Workspace::leave_note`: flushed
//! first; a failed flush refuses), and while it shows there is no open note. It shows with the
//! drawer view ASK, the turn's Sources ([`super::ask_sources`]), which
//! follows the selected turn (`AskFlow`, the one record). Any note opening,
//! and any attachment shown, takes it down and puts the drawer back on
//! FILES; another view shown in place of ASK takes it down too. The thread
//! itself stays, and comes back as it was.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    Enter, Escape, Input, InputEvent, InputState, MoveDown, MovePageDown, MovePageUp, MoveUp,
};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, FontStyle, FontWeight, HighlightStyle,
    InteractiveElement as _, InteractiveText, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement as _, Render, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, StrikethroughStyle, Styled as _, StyledText, Subscription,
    Window, div, point, px,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::ask::{AskSource, Thread, Turn, TurnStatus, jobs, markdown, save};
#[cfg(test)]
use kimun_notes::logic::ask_flow::AskFlow;
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout::{Panel, Region};
use kimun_notes::logic::leader_route::AskStep;
use kimun_notes::logic::message::{AppTx, AskData};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::rag_caps::{self, Caps};
use kimun_notes::rag::RagStatus;
use kimun_notes::server_client::RagClient;
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::note::{Closing, LeavingFor, NoteLeft};
use super::route::NO_WORKSPACE;
use crate::input;
use crate::shell_layout;
use crate::tasks;
use crate::theme;

/// Element ids, for tests.
pub(crate) const ASK_ID: &str = "ask";
pub(crate) const TURNS_ID: &str = "ask-turns";
pub(crate) const COMPOSER_ID: &str = "ask-composer";
pub(crate) const COMPOSER_LABEL_ID: &str = "ask-composer-label";
pub(crate) const SEND_ID: &str = "ask-send";
pub(crate) const COPY_ID: &str = "ask-copy";
pub(crate) const SAVE_ID: &str = "ask-save";
pub(crate) const REGENERATE_ID: &str = "ask-regenerate";
pub(crate) const NEW_ID: &str = "ask-new";

/// The element id of the turn `id`.
pub(crate) fn turn_element_id(id: u64) -> SharedString {
    format!("ask-turn:{id}").into()
}

/// The element id of the turn `id`'s status line (pending or failed),
/// named by its text.
pub(crate) fn turn_status_id(id: u64) -> SharedString {
    format!("ask-turn-status:{id}").into()
}

/// The element id of the turn `id`'s answer, named by the text it shows.
pub(crate) fn turn_answer_id(id: u64) -> SharedString {
    format!("ask-turn-answer:{id}").into()
}

/// The element id of line `line` (0-based) of the turn `id`'s answer.
pub(crate) fn answer_line_id(id: u64, line: usize) -> ElementId {
    ElementId::NamedInteger(format!("ask-answer-line:{id}").into(), line as u64)
}

/// The composer's title, and the one it has while no server can answer
/// (the TUI's words).
pub(crate) const COMPOSER_TITLE: &str = "Ask a question";
pub(crate) const COMPOSER_DISABLED: &str = "server unavailable";
/// A pending turn's status line (the TUI's).
pub(crate) const THINKING: &str = "… thinking";
/// The hint under a failed turn (the TUI's, without its indent).
pub(crate) const RETRY_HINT: &str = "[r] retry";
/// The flash after a copy (the TUI's).
pub(crate) const COPIED: &str = "answer copied";

/// The turn list's accessible name (the TUI's hint word for it).
const TURNS_LABEL: &str = "Turns";
/// How much of a page a PageUp/PageDown keeps in view, in rems (two lines).
const PAGE_OVERLAP_REMS: f32 = 2.75;

// ---- An answer's lines ---------------------------------------------------------

/// The block role of one line of an answer (the shared answer rules',
/// `kimun_notes::ask::markdown`, the TUI's too).
pub(crate) use kimun_notes::ask::markdown::BlockKind as LineKind;
/// The inline style of a run of an answer line's shown text (the shared
/// rules'; they combine).
pub(crate) use kimun_notes::ask::markdown::SpanStyle as RunStyle;

/// One line of an answer as the Thread draws it, from the shared
/// [`markdown::AnswerLine::shown`] with the text made shareable: the text shown (emphasis
/// sigils removed), its role, its styled runs (byte ranges into `text`, in
/// order, not overlapping; plain text has none), and each `[n]` citation's
/// range in `text` with its `n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DrawnLine {
    pub(crate) text: SharedString,
    pub(crate) kind: LineKind,
    pub(crate) runs: Vec<(Range<usize>, RunStyle)>,
    pub(crate) citations: Vec<(Range<usize>, usize)>,
}

/// `answer` split into the lines the Thread draws, by the shared answer
/// rules (`ask::markdown::parse_answer`: the editor's markdown model, one
/// buffer-aware pass, so fences, setext headings and lazy quotes resolve
/// as in the editor): this only lays their spans end to end. Code, heading
/// and quote lines are verbatim in their role; on ordinary lines the
/// hidden emphasis sigils are left out, inline code is verbatim (backticks
/// included, never a citation), and every `[n]` citation is marked.
pub(crate) fn answer_lines(answer: &str) -> Vec<DrawnLine> {
    markdown::parse_answer(answer)
        .iter()
        .map(|line| {
            let shown = line.shown(answer);
            DrawnLine {
                text: shown.text.into(),
                kind: shown.kind,
                runs: shown.runs,
                citations: shown.citations,
            }
        })
        .collect()
}

/// One line's links: each citation's range in the shown text and its `n`.
type Links = Rc<[(Range<usize>, usize)]>;

/// A done turn's answer as the Thread draws it, made once when the answer
/// landed (never in a frame): its lines, each line's citations that name
/// one of the turn's sources (the links), and the whole shown text (the
/// answer's accessible name).
struct DrawnAnswer {
    lines: Vec<DrawnLine>,
    links: Vec<Links>,
    label: SharedString,
}

impl DrawnAnswer {
    fn of(turn: &Turn) -> Self {
        let lines = answer_lines(&turn.answer);
        let links = lines
            .iter()
            .map(|line| {
                line.citations
                    .iter()
                    .filter(|(_, n)| turn.source_for_citation(*n).is_some())
                    .cloned()
                    .collect()
            })
            .collect();
        let label = lines
            .iter()
            .map(|line| line.text.as_ref())
            .collect::<Vec<_>>()
            .join("\n")
            .into();
        Self {
            lines,
            links,
            label,
        }
    }
}

// ---- The view ------------------------------------------------------------------

/// Which actions can run now (the TUI's `sync_actions`, plus the server
/// for Regenerate and New).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Actions {
    pub(crate) send: bool,
    pub(crate) copy: bool,
    pub(crate) save: bool,
    pub(crate) regenerate: bool,
    pub(crate) new: bool,
}

/// What the Thread asks of the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AskEvent {
    /// Copy the selected turn's answer.
    Copy,
    /// Save the selected turn as a note (the create dialog).
    Save,
    /// Start over.
    New,
    /// Escape on the turn list: leave the Thread for the drawer.
    Dismiss,
    /// The selection may have moved (a key, a click, a question sent, a
    /// regenerate), and a click on a citation names the source `[n]` of
    /// the selected turn: the Sources view follows.
    Input(Option<usize>),
}

/// The Thread. See the module docs.
pub(crate) struct AskView {
    thread: Thread,
    /// The live client while the server can answer questions; `None`
    /// disables the composer, never the thread.
    client: Option<Arc<RagClient>>,
    /// Why the server cannot answer now (the TUI's words), `None` when it
    /// can: the disabled Send and Regenerate say it.
    unavailable: Option<&'static str>,
    pub(crate) composer: Entity<InputState>,
    /// Whether the composer held non-blank text at its last change: the one
    /// thing of its text the Thread draws (Send's state).
    composer_has_text: bool,
    turns_focus: FocusHandle,
    /// The composer, not the turn list, takes the keyboard when the Thread
    /// gets it (the TUI's `ThreadFocus`).
    wants_composer: bool,
    scroll: ScrollHandle,
    /// Each done turn's answer as drawn, by turn id: made once when the
    /// answer landed, dropped when the turn is asked again or the thread
    /// cleared.
    answers: HashMap<u64, DrawnAnswer>,
    tx: AppTx,
    theme: Theme,
    _composer_events: Subscription,
}

impl EventEmitter<AskEvent> for AskView {}

impl AskView {
    /// Draw in `t` from now on (the window's theme changed).
    pub(crate) fn set_theme(&mut self, t: &Theme, cx: &mut Context<Self>) {
        self.theme = t.clone();
        cx.notify();
    }
}

impl AskView {
    pub(crate) fn new(
        tx: AppTx,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer = cx.new(|cx| InputState::new(window, cx));
        // Send's enabled state follows whether the text is blank, so only
        // that flipping redraws the Thread: a keystroke in a long
        // conversation must not rebuild every turn and answer line.
        let composer_events = cx.subscribe(&composer, |this, composer, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let has_text = !composer.read(cx).value().trim().is_empty();
            if std::mem::replace(&mut this.composer_has_text, has_text) != has_text {
                cx.notify();
            }
        });
        Self {
            thread: Thread::default(),
            client: None,
            unavailable: rag_caps::ask_unavailable_message(RagStatus::Disabled),
            composer,
            composer_has_text: false,
            turns_focus: cx.focus_handle(),
            wants_composer: true,
            scroll: ScrollHandle::new(),
            answers: HashMap::new(),
            tx,
            theme,
            _composer_events: composer_events,
        }
    }

    #[cfg(test)]
    pub(crate) fn thread(&self) -> &Thread {
        &self.thread
    }

    /// Whether the turn `id`'s answer is drawn (tests).
    #[cfg(test)]
    pub(crate) fn has_drawn_answer(&self, id: u64) -> bool {
        self.answers.contains_key(&id)
    }

    #[cfg(test)]
    pub(crate) fn has_client(&self) -> bool {
        self.client.is_some()
    }

    /// The server's capability changed: `client` answers from now on
    /// (`None`: nothing can be asked; the thread stays readable), and
    /// `unavailable` says why not.
    pub(crate) fn set_client(
        &mut self,
        client: Option<Arc<RagClient>>,
        unavailable: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        self.client = client;
        self.unavailable = unavailable;
        cx.notify();
    }

    /// The composer's text.
    pub(crate) fn composer_text(&self, cx: &App) -> String {
        self.composer.read(cx).value().to_string()
    }

    /// Which actions can run now.
    pub(crate) fn actions(&self, cx: &App) -> Actions {
        let selected = self.thread.selected();
        let has_turn = selected.is_some();
        let pending = selected.is_some_and(is_pending);
        let server = self.client.is_some();
        Actions {
            send: server && !self.composer_text(cx).trim().is_empty(),
            copy: has_turn,
            save: has_turn,
            regenerate: has_turn && server && !pending,
            new: has_turn && self.unavailable.is_none(),
        }
    }

    /// The composer gets the keyboard when the Thread does (leader `a a`,
    /// F6, New).
    pub(crate) fn want_composer(&mut self) {
        self.wants_composer = true;
    }

    /// The selected turn's id.
    pub(crate) fn selected_id(&self) -> Option<u64> {
        self.thread.selected().map(|turn| turn.id)
    }

    /// The selected turn's id and sources.
    pub(crate) fn selected_sources(&self) -> Option<(u64, Vec<AskSource>)> {
        self.thread
            .selected()
            .map(|turn| (turn.id, turn.sources.clone()))
    }

    /// The turn `id`, if it is in the thread.
    fn turn(&self, id: u64) -> Option<&Turn> {
        self.thread.turns().iter().find(|turn| turn.id == id)
    }

    /// The sources of the turn `id`, if it is in the thread.
    pub(crate) fn turn_sources(&self, id: u64) -> Option<Vec<AskSource>> {
        self.turn(id).map(|turn| turn.sources.clone())
    }

    /// The selected turn's answer as Copy puts it on the clipboard.
    pub(crate) fn copy_text(&self) -> Option<String> {
        self.thread.selected().map(save::copy_text)
    }

    /// The selected turn as a note: the path offered and its content.
    pub(crate) fn save_payload(&self) -> Option<(VaultPath, String)> {
        let turn = self.thread.selected()?;
        Some((
            save::suggested_path(&turn.question),
            save::note_content(turn),
        ))
    }

    /// Send the composer's question (the TUI's `begin_turn`): nothing
    /// without a server or with a blank question; else a pending turn,
    /// selected and followed, and the ask job on tokio.
    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let question = self.composer_text(cx).trim().to_string();
        if question.is_empty() {
            return;
        }
        // `set_value` emits no `Change`, so the flag that gates Send's
        // redraw is reset here; the notify below redraws now.
        self.composer_has_text = false;
        self.composer
            .update(cx, |composer, cx| composer.set_value("", window, cx));
        let history = self.thread.history();
        let turn_id = self.thread.ask(question.clone());
        self.follow_selected();
        self.spawn_ask(client, question, history, turn_id);
        cx.emit(AskEvent::Input(None));
        cx.notify();
    }

    /// Rewind the selected turn and ask its question again (the TUI's
    /// `regenerate_selected`): the server is checked before the rewind, so
    /// without one the turn stays as it is; a pending turn is not asked
    /// twice.
    pub(crate) fn regenerate(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else {
            return;
        };
        let Some(id) = self.thread.selected().map(|t| t.id) else {
            return;
        };
        let Some(question) = self.thread.regenerate(id) else {
            return;
        };
        self.answers.remove(&id);
        let history = self.thread.history();
        self.follow_selected();
        self.spawn_ask(client, question, history, id);
        cx.emit(AskEvent::Input(None));
        cx.notify();
    }

    fn spawn_ask(
        &self,
        client: Arc<RagClient>,
        question: String,
        history: Vec<(String, String)>,
        turn_id: u64,
    ) {
        tokio::spawn(jobs::ask(
            client,
            question,
            history,
            turn_id,
            self.tx.clone(),
        ));
    }

    /// Start over: no turns, the composer wanted.
    pub(crate) fn clear(&mut self, cx: &mut Context<Self>) {
        self.thread.clear();
        self.answers.clear();
        self.wants_composer = true;
        cx.notify();
    }

    /// An answer landed for `turn_id` (the TUI's `handle_data`): the turn
    /// completes or fails; a stale id changes nothing. The selected turn's
    /// end comes into view.
    pub(crate) fn answer_ready(
        &mut self,
        turn_id: u64,
        result: Result<(String, Vec<AskSource>), String>,
        cx: &mut Context<Self>,
    ) {
        let selected = self.thread.selected().map(|t| t.id) == Some(turn_id);
        let landed = match result {
            Ok((answer, sources)) => {
                let done = self.thread.complete(turn_id, answer, sources);
                // Accepted only: a stale id does no work.
                if done && let Some(turn) = self.turn(turn_id) {
                    let drawn = DrawnAnswer::of(turn);
                    self.answers.insert(turn_id, drawn);
                }
                done
            }
            Err(e) => self.thread.fail(turn_id, e),
        };
        if landed {
            if selected {
                self.follow_selected();
            }
            cx.notify();
        }
    }

    /// Move the selection one turn older (`up`) or newer, keeping it in
    /// view; the Sources view follows.
    fn select_step(&mut self, up: bool, cx: &mut Context<Self>) {
        if up {
            self.thread.select_prev();
        } else {
            self.thread.select_next();
        }
        self.follow_selected();
        cx.emit(AskEvent::Input(None));
        cx.notify();
    }

    /// Select the turn `id` (a click on it); the Sources view follows.
    fn select_turn(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(ix) = self.thread.turns().iter().position(|t| t.id == id) {
            self.thread.select_index(ix);
            cx.emit(AskEvent::Input(None));
            cx.notify();
        }
    }

    /// A click on the citation `[n]` in the turn `id`'s answer (the TUI's
    /// `click_turns`): that turn is selected, and when one of its sources
    /// answers to `n` (by ordinal), the Sources view selects it. An unknown
    /// `[n]` only selects the turn.
    fn click_citation(&mut self, id: u64, n: usize, cx: &mut Context<Self>) {
        let Some(ix) = self.thread.turns().iter().position(|t| t.id == id) else {
            return;
        };
        self.thread.select_index(ix);
        let known = self
            .thread
            .selected()
            .is_some_and(|turn| turn.source_for_citation(n).is_some());
        cx.emit(AskEvent::Input(known.then_some(n)));
        cx.notify();
    }

    /// Bring the selected turn into view: the newest turn by its end (new
    /// content landed there), an older one by its start.
    fn follow_selected(&self) {
        let turns = self.thread.turns();
        let Some(selected) = self.thread.selected() else {
            return;
        };
        match turns.iter().position(|t| t.id == selected.id) {
            Some(ix) if ix + 1 == turns.len() => self.scroll.scroll_to_bottom(),
            Some(ix) => self.scroll.scroll_to_item(ix),
            None => {}
        }
    }

    /// PageUp/PageDown/Home/End over the turns (the TUI's content scroll).
    fn scroll_by_key(&self, key: Key, window: &Window) -> bool {
        let overlap = f32::from(window.rem_size()) * PAGE_OVERLAP_REMS;
        let page = (f32::from(self.scroll.bounds().size.height) - overlap).max(overlap);
        let max = f32::from(self.scroll.max_offset().y).max(0.0);
        let y = -f32::from(self.scroll.offset().y);
        let to = match key {
            Key::PageUp => y - page,
            Key::PageDown => y + page,
            Key::Home => 0.0,
            Key::End => max,
            _ => return false,
        };
        let x = self.scroll.offset().x;
        self.scroll.set_offset(point(x, px(-to.clamp(0.0, max))));
        true
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.wants_composer = true;
        let handle = self.focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    fn focus_turns(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.wants_composer = false;
        window.focus(&self.turns_focus, cx);
        cx.notify();
    }

    /// A key on the turn list (the TUI's `handle_turns_key`). True when
    /// taken.
    fn turns_key(&mut self, key: KeyInput, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if key.mods != Mods::NONE && !(matches!(key.key, Key::Char(_)) && key.mods == Mods::SHIFT) {
            return false;
        }
        match key.key {
            Key::Up | Key::Char('k') => self.select_step(true, cx),
            Key::Down | Key::Char('j') => self.select_step(false, cx),
            Key::PageUp | Key::PageDown | Key::Home | Key::End => {
                self.scroll_by_key(key.key, window);
                cx.notify();
            }
            Key::Char('i' | '/') => self.focus_composer(window, cx),
            Key::Char('y') => cx.emit(AskEvent::Copy),
            Key::Char('e') => cx.emit(AskEvent::Save),
            Key::Char('r') => self.regenerate(cx),
            Key::Esc => cx.emit(AskEvent::Dismiss),
            _ => return false,
        }
        true
    }

    fn render_turn(&self, turn: &Turn, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let id = turn.id;
        let body: AnyElement = match &turn.status {
            TurnStatus::Thinking | TurnStatus::Streaming => div()
                .id(turn_status_id(id))
                .test_support()
                .role(Role::Status)
                .aria_label(THINKING)
                .text_color(theme::muted(t))
                .child(THINKING)
                .into_any_element(),
            TurnStatus::Error(message) => {
                let failed: SharedString = format!("✗ {message}").into();
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .id(turn_status_id(id))
                            .test_support()
                            .role(Role::Status)
                            .aria_label(failed.clone())
                            .text_color(theme::error(t))
                            .child(failed),
                    )
                    .child(div().text_color(theme::muted(t)).child(RETRY_HINT))
                    .into_any_element()
            }
            TurnStatus::Done => self.render_answer(turn, cx),
        };
        div()
            .id(turn_element_id(id))
            .test_support()
            .role(Role::ListItem)
            .aria_label(SharedString::from(turn.question.clone()))
            .aria_selected(selected)
            .flex()
            .flex_col()
            .flex_none()
            .gap_1()
            .px_3()
            .py_2()
            .rounded_md()
            .when(selected, |row| row.bg(theme::selected_background(t)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    this.select_turn(id, cx);
                    this.focus_turns(window, cx);
                }),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::accent(t))
                    .child(turn.question.clone()),
            )
            .child(body)
            .into_any_element()
    }

    /// A done turn's answer, line by line, as [`DrawnAnswer`] made it when
    /// it landed (render only reads it). A line with citations naming one
    /// of the turn's sources is interactive text: a click on one is
    /// hit-tested on the shaped line ([`Self::click_citation`]) and shows
    /// the pointing hand.
    fn render_answer(&self, turn: &Turn, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let id = turn.id;
        let answer = div()
            .id(turn_answer_id(id))
            .test_support()
            .role(Role::Document)
            .flex()
            .flex_col()
            .text_color(theme::foreground(t));
        // Every done turn's answer was drawn when it landed.
        let Some(drawn) = self.answers.get(&id) else {
            return answer.into_any_element();
        };
        let mono = cx.theme().mono_font_family.clone();
        let citation = HighlightStyle {
            color: Some(theme::accent(t).into()),
            ..Default::default()
        };
        let code_tint = theme::hover_background(t);
        let this = cx.entity().downgrade();
        answer
            .aria_label(drawn.label.clone())
            .children(drawn.lines.iter().zip(&drawn.links).enumerate().map(
                |(ix, (line, links))| {
                    if line.text.is_empty() {
                        return div().h_2().into_any_element();
                    }
                    let highlights = line.runs.iter().map(|(range, style)| {
                        let mut h = if style.citation {
                            citation
                        } else {
                            HighlightStyle::default()
                        };
                        if style.bold {
                            h.font_weight = Some(FontWeight::BOLD);
                        }
                        if style.italic {
                            h.font_style = Some(FontStyle::Italic);
                        }
                        if style.strike {
                            // A one-pixel line: a physical stroke, not zoomed.
                            h.strikethrough = Some(StrikethroughStyle {
                                thickness: px(1.),
                                color: None,
                            });
                        }
                        if style.code {
                            h.background_color = Some(code_tint.into());
                        }
                        (range.clone(), h)
                    });
                    let text = StyledText::new(line.text.clone()).with_highlights(highlights);
                    let text = if links.is_empty() {
                        text.into_any_element()
                    } else {
                        let this = this.clone();
                        let links = links.clone();
                        let ranges = links.iter().map(|(range, _)| range.clone()).collect();
                        InteractiveText::new(answer_line_id(id, ix), text)
                            .on_click(ranges, move |at, _, cx| {
                                let n = links[at].1;
                                this.update(cx, |view, cx| view.click_citation(id, n, cx))
                                    .ok();
                            })
                            .into_any_element()
                    };
                    div()
                        .id(answer_line_id(id, ix))
                        .test_support()
                        .map(|row| match line.kind {
                            LineKind::Code => row.font_family(mono.clone()),
                            LineKind::Heading => row.font_weight(FontWeight::SEMIBOLD),
                            LineKind::Quote => row.pl_4().text_color(theme::muted(t)),
                            LineKind::Normal => row,
                        })
                        .child(text)
                        .into_any_element()
                },
            ))
            .into_any_element()
    }

    fn render_actions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let actions = self.actions(cx);
        let reason = self.unavailable;
        let button = |id: &'static str, label: &'static str, enabled: bool| {
            Button::new(id).small().label(label).disabled(!enabled)
        };
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_2()
            .child(
                button(COPY_ID, "Copy", actions.copy)
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(AskEvent::Copy))),
            )
            .child(
                button(SAVE_ID, "Save as note", actions.save)
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(AskEvent::Save))),
            )
            .child(
                button(REGENERATE_ID, "Regenerate", actions.regenerate)
                    .when_some(reason, |b, why| b.tooltip(why))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.regenerate(cx))),
            )
            .child(
                button(NEW_ID, "New", actions.new)
                    .when_some(reason, |b, why| b.tooltip(why))
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(AskEvent::New))),
            )
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let enabled = self.client.is_some();
        let title = if enabled {
            COMPOSER_TITLE
        } else {
            COMPOSER_DISABLED
        };
        let send = self.actions(cx).send;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id(COMPOSER_LABEL_ID)
                    .test_support()
                    .role(Role::Label)
                    .aria_label(title)
                    .text_sm()
                    .text_color(theme::muted(t))
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _, _| {
                                    this.wants_composer = true;
                                }),
                            )
                            .child(
                                Input::new(&self.composer)
                                    .id(COMPOSER_ID)
                                    .aria_label(title)
                                    .disabled(!enabled)
                                    .bg(theme::background(t))
                                    .border_color(theme::divider(t))
                                    .text_color(theme::foreground(t)),
                            ),
                    )
                    .child(
                        Button::new(SEND_ID)
                            .primary()
                            .label("Send")
                            .disabled(!send)
                            .when_some(self.unavailable.filter(|_| !enabled), |b, why| {
                                b.tooltip(why)
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.send(window, cx)
                            })),
                    ),
            )
    }
}

/// Whether `turn` waits for its answer.
fn is_pending(turn: &Turn) -> bool {
    matches!(turn.status, TurnStatus::Thinking | TurnStatus::Streaming)
}

impl Focusable for AskView {
    /// The composer while it is wanted and a server can answer; else the
    /// turn list.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.wants_composer && self.client.is_some() {
            self.composer.read(cx).focus_handle(cx)
        } else {
            self.turns_focus.clone()
        }
    }
}

impl Render for AskView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let selected = self.thread.selected().map(|turn| turn.id);
        let turns: Vec<AnyElement> = self
            .thread
            .turns()
            .iter()
            .map(|turn| self.render_turn(turn, Some(turn.id) == selected, cx))
            .collect();
        div()
            .id(ASK_ID)
            .test_support()
            .role(Role::Group)
            .aria_label(shell_layout::view_name(DrawerView::Ask))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            // The composer's keys that mean something else here.
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                this.send(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                this.focus_turns(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                if this.composer_text(cx).is_empty() {
                    this.select_step(true, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                if this.composer_text(cx).is_empty() {
                    this.select_step(false, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MovePageUp, window, cx| {
                this.scroll_by_key(Key::PageUp, window);
                cx.notify();
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MovePageDown, window, cx| {
                this.scroll_by_key(Key::PageDown, window);
                cx.notify();
                cx.stop_propagation();
            }))
            .child(
                div()
                    .id(TURNS_ID)
                    .test_support()
                    .track_focus(&self.turns_focus)
                    .role(Role::List)
                    .aria_label(TURNS_LABEL)
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .px_3()
                    .py_3()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if let Some(key) = input::key_down_input(event)
                            && this.turns_key(key, window, cx)
                        {
                            cx.stop_propagation();
                        }
                    }))
                    .children(turns),
            )
            .child(
                div()
                    .flex_none()
                    // A hairline: a physical boundary, not zoomed.
                    .h(px(1.))
                    .w_full()
                    .bg(theme::divider(t)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .child(self.render_actions(cx))
                    .child(self.render_composer(cx)),
            )
    }
}

// ---- The workspace's side ----------------------------------------------------

/// What entering the Thread goes on to do once it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AskThen {
    /// Give the composer the keyboard (leader `a a`, F6).
    Composer,
    /// Start over, the composer then (leader `a n`, New).
    New,
    /// Regenerate the selected turn (leader `a r`).
    Regenerate,
    /// Read the selected turn's top source, the Sources view given the
    /// keyboard (leader `a s`).
    Source,
}

impl Workspace {
    /// Build the Thread for the vault the window opened (empty, no server
    /// until the status says one can answer).
    pub(super) fn start_ask(&mut self, tx: AppTx, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.theme.clone();
        let view = cx.new(|cx| AskView::new(tx, theme, window, cx));
        self._ask_events = Some(cx.subscribe_in(&view, window, Self::on_ask_event));
        self.ask = Some(view);
    }

    fn on_ask_event(
        &mut self,
        _: &Entity<AskView>,
        event: &AskEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AskEvent::Copy => self.copy_answer(cx),
            AskEvent::Save => self.save_answer(cx),
            AskEvent::New => self.ask_step(AskStep::New, window, cx),
            AskEvent::Dismiss => self.leave_ask_focus(window, cx),
            AskEvent::Input(citation) => self.after_ask_input(*citation, window, cx),
        }
    }

    /// Whether the Thread holds the editor Panel.
    pub(crate) fn ask_shown(&self) -> bool {
        self.ask_flow.showing() && self.ask.is_some()
    }

    /// Why the server cannot answer questions now, in the TUI's words.
    pub(crate) fn ask_unavailable(&self) -> Option<&'static str> {
        rag_caps::ask_unavailable_message(self.rag_status)
    }

    /// Why the leader's `+ask` `step` cannot run now, or `None` when it
    /// can. Copy and Save only read the thread; focusing the Thread or
    /// reading a source while it already shows needs nothing (the TUI's
    /// `open_ask_workspace`); New and Regenerate, and entering it, need a
    /// server that can answer.
    pub(crate) fn ask_block(&self, step: AskStep) -> Option<&'static str> {
        if self.ask.is_none() {
            return Some(NO_WORKSPACE);
        }
        match step {
            AskStep::Copy | AskStep::Save => None,
            AskStep::Focus | AskStep::Source if self.ask_shown() => None,
            AskStep::Focus | AskStep::Source | AskStep::New | AskStep::Regenerate => {
                self.ask_unavailable()
            }
        }
    }

    /// Why the drawer view ASK cannot be opened now (the TUI gates every
    /// route into it on the server being able to answer).
    pub(crate) fn ask_drawer_block(&self) -> Option<&'static str> {
        if self.ask.is_none() {
            return Some(NO_WORKSPACE);
        }
        self.ask_unavailable()
    }

    /// The ASK rail cell (when not shown) and the `drawer.ask` leaf: the
    /// Thread entered (its rules: flush, refuse) with the Sources view
    /// beside it, or, the Thread already showing, the Sources view shown
    /// again; the Thread has the keyboard (the TUI's). Gated here too: a
    /// leaf sent on the channel skips the route block.
    pub(crate) fn open_ask_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(why) = self.ask_drawer_block() {
            self.flash(why, cx);
            return;
        }
        if !self.ask_shown() {
            self.enter_ask(AskThen::Composer, window, cx);
            return;
        }
        self.layout.open_view(DrawerView::Ask);
        self.place_rail_cursor(Some(DrawerView::Ask));
        self.layout.focus(Panel::Editor);
        self.apply_focus(window, cx);
        self.save_layout(cx);
        cx.notify();
    }

    /// Run the leader's `+ask` `step` (also F6 = Focus, and the Thread's
    /// New): checked again here, since a leaf sent on the app channel skips
    /// the route block; a refused step flashes why.
    pub(crate) fn ask_step(&mut self, step: AskStep, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(why) = self.ask_block(step) {
            self.flash(why, cx);
            return;
        }
        match step {
            AskStep::Focus => self.enter_ask(AskThen::Composer, window, cx),
            AskStep::New => self.enter_ask(AskThen::New, window, cx),
            AskStep::Regenerate => self.enter_ask(AskThen::Regenerate, window, cx),
            AskStep::Copy => self.copy_answer(cx),
            AskStep::Save => self.save_answer(cx),
            AskStep::Source => self.enter_ask(AskThen::Source, window, cx),
        }
    }

    /// Show the Thread in the editor Panel, then do `then`. The open note
    /// is left first, as for an attachment ([`Workspace::leave_note`]):
    /// flushed (the tick stopped, the editor taking no input), and a flush
    /// that fails refuses — the note stays with its edits, its tick and the
    /// keyboard. Already showing: `then` runs now.
    pub(crate) fn enter_ask(&mut self, then: AskThen, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing != Closing::Open || self.ask.is_none() {
            return;
        }
        if self.ask_shown() {
            self.ask_then(then, window, cx);
            return;
        }
        if self.ask_flushing.is_some() {
            return;
        }
        self.leave_note(
            LeavingFor::Ask,
            shell_layout::view_name(DrawerView::Ask).into(),
            move |workspace, left, window, cx| match left {
                NoteLeft::Now => workspace.show_ask(then, window, cx),
                // The window began closing while the flush ran.
                NoteLeft::Flushed if workspace.closing != Closing::Open => {
                    workspace.restart_tick_unless_read_only();
                }
                NoteLeft::Flushed => workspace.show_ask(then, window, cx),
                NoteLeft::Stays => {}
            },
            window,
            cx,
        );
    }

    /// Put the Thread in the editor Panel, the note (already flushed) let
    /// go ([`Workspace::release_note`]), so nothing can save while the
    /// Thread shows; an attachment shown gives way too.
    fn show_ask(&mut self, then: AskThen, window: &mut Window, cx: &mut Context<Self>) {
        self.release_note(cx);
        self.drop_attachment();
        if let Some(files) = self.files.as_ref() {
            files.update(cx, |pane, cx| pane.set_open_note(None, cx));
        }
        self.ask_flow.enter();
        // The drawer shows the Sources view (revealed, the TUI's), pointed
        // at the selected turn whatever it listed before.
        self.layout.open_view(DrawerView::Ask);
        self.place_rail_cursor(Some(DrawerView::Ask));
        self.sync_sources(window, cx);
        self.layout.focus(Panel::Editor);
        // Persisted (never with ASK); LINKS and OUTLINE say "No note open".
        self.save_layout(cx);
        self.ask_then(then, window, cx);
        cx.notify();
    }

    /// The Thread shows: do `then`, and give it the keyboard (the Sources
    /// view, for a source read).
    fn ask_then(&mut self, then: AskThen, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.ask.clone() else {
            return;
        };
        match then {
            AskThen::Composer => view.update(cx, |view, _| view.want_composer()),
            AskThen::New => {
                view.update(cx, |view, cx| view.clear(cx));
                if let Some(sources) = self.ask_sources.clone() {
                    sources.update(cx, |sources, cx| sources.reset(window, cx));
                }
            }
            AskThen::Regenerate => view.update(cx, |view, cx| view.regenerate(cx)),
            AskThen::Source => {
                // The TUI's `a s`: the drawer on the selected turn, its top
                // source in the reader, the drawer focused.
                self.sync_sources(window, cx);
                if let Some(sources) = self.ask_sources.clone() {
                    sources.update(cx, |sources, cx| sources.open_reader(0, window, cx));
                }
                let (side, index) = self.layout.open_view(DrawerView::Ask);
                self.place_rail_cursor(Some(DrawerView::Ask));
                self.layout.focus_region(Region::Slot { side, index });
                self.apply_focus(window, cx);
                self.save_layout(cx);
                cx.notify();
                return;
            }
        }
        self.layout.focus(Panel::Editor);
        self.apply_focus(window, cx);
        cx.notify();
    }

    /// Something else takes the editor Panel (a note opened, an attachment
    /// shows): the Thread goes down, its conversation kept for next time.
    /// True when it was showing.
    pub(super) fn hide_ask(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.ask_flow.hide_if_shown() {
            return false;
        }
        // The drawer goes from ASK to FILES, revealing nothing hidden (a
        // drawer left on ASK would hide on the next ASK click).
        match self.drawer_off_ask(cx) {
            Some(side) if self.layout.focused() == Panel::Drawer(side) => {
                self.with_window(cx, |workspace, window, cx| {
                    workspace.apply_focus(window, cx)
                });
            }
            _ => self.focus_shown_content(cx),
        }
        cx.notify();
        true
    }

    /// Take the drawer view ASK out of its slot for FILES: FILES shows in
    /// its place, or, FILES being shown already, the slot closes. The
    /// drawer's visibility is unchanged. The side it was on, if it was in
    /// a slot. Persisted.
    pub(super) fn drawer_off_ask(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<kimun_notes::logic::layout::Side> {
        let (side, index) = self.layout.view_location(&DrawerView::Ask)?;
        if self.layout.view_location(&DrawerView::Files).is_none() {
            self.layout
                .set_slot_view(side, index, DrawerView::Files)
                .ok()?;
            self.ensure_files_listing(cx);
        } else {
            self.layout.close_slot(side, index).ok()?;
        }
        self.place_rail_cursor(Some(DrawerView::Files));
        self.save_layout(cx);
        Some(side)
    }

    /// The layout changed with the Thread showing: if ASK is in no slot
    /// any more (another view shown in its place, its slot closed), the
    /// Thread goes down, its conversation and the synced turn kept (the
    /// TUI's drawer-view switch away from ASK, `AskFlow::leave`).
    pub(super) fn leave_ask_if_drawer_moved_on(&mut self, cx: &mut Context<Self>) {
        if self.ask_flow.showing() && self.layout.view_location(&DrawerView::Ask).is_none() {
            self.ask_flow.leave();
            self.focus_shown_content(cx);
            cx.notify();
        }
    }

    /// Escape on the turn list: the editor the Thread replaced is gone, so
    /// the keyboard goes to the drawer beside it (the left one, else the
    /// right one), else to the rail.
    fn leave_ask_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel = [
            kimun_notes::logic::layout::Side::Left,
            kimun_notes::logic::layout::Side::Right,
        ]
        .into_iter()
        .map(Panel::Drawer)
        .find(|panel| self.layout.is_visible(*panel))
        .unwrap_or(Panel::Rail);
        self.focus_panel(panel, window, cx);
    }

    /// Copy the selected turn's answer, citations stripped (the TUI's
    /// `copy_selected`, its flash). Nothing selected: nothing.
    pub(crate) fn copy_answer(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.ask.as_ref().and_then(|v| v.read(cx).copy_text()) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.flash(COPIED, cx);
    }

    /// Save the selected turn as a note (the TUI's `save_selected`): the
    /// create dialog, offered on a free path under `ask/`, with the turn as
    /// the note's content. Nothing selected: nothing.
    pub(crate) fn save_answer(&mut self, cx: &mut Context<Self>) {
        let Some((path, content)) = self.ask.as_ref().and_then(|v| v.read(cx).save_payload())
        else {
            return;
        };
        self.offer_saved_answer(path, content, cx);
    }

    /// `FileOp::ShowCreateWithContent` (and Save as note): find the first
    /// free path from `path` on tokio — the dialog then names the note the
    /// create will really write — and offer to create it holding `content`.
    pub(crate) fn offer_saved_answer(
        &mut self,
        path: VaultPath,
        content: String,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.vault_or_flash(cx) else {
            return;
        };
        let free = tokio::spawn(async move { Ok::<_, String>(vault.free_note_path(&path).await) });
        self.ask_saving = Some(cx.spawn(async move |this, cx| {
            let free = tasks::joined(free).await;
            this.update(cx, |workspace, cx| {
                workspace.ask_saving = None;
                match free {
                    Ok(path) => workspace.offer_create_with(path, Some(content), cx),
                    Err(e) => workspace.flash_error(format!("Couldn't save the answer: {e}"), cx),
                }
            })
            .ok();
        }));
    }

    /// The kimün server's status changed (from `on_rag_status`): on an ASK
    /// flip the Thread gets a client (built from the config on tokio: it
    /// reads the vault id) or loses it. A client that arrives after the
    /// server went again is dropped.
    pub(super) fn refresh_ask_client(&mut self, cx: &mut Context<Self>) {
        let unavailable = self.ask_unavailable();
        let Some(view) = self.ask.clone() else {
            return;
        };
        if !Caps::from_status(self.rag_status).ask {
            self.ask_client_task = None;
            view.update(cx, |view, cx| view.set_client(None, unavailable, cx));
            return;
        }
        #[cfg(test)]
        if let Some(client) = self.ask_test_client.clone() {
            view.update(cx, |view, cx| {
                view.set_client(Some(client), unavailable, cx)
            });
            return;
        }
        let (Some(settings), Some(vault)) = (self.settings.clone(), self.vault()) else {
            return;
        };
        let status = self.rag_status;
        let client = tokio::spawn(async move {
            Ok::<_, String>(rag_caps::ask_client(&settings, &vault, status).await)
        });
        self.ask_client_task = Some(cx.spawn(async move |this, cx| {
            let client = tasks::joined(client).await;
            this.update(cx, |workspace, cx| {
                workspace.ask_client_task = None;
                if !Caps::from_status(workspace.rag_status).ask {
                    return;
                }
                let client = client.unwrap_or_else(|e| {
                    tracing::warn!("the Ask client could not be built: {e}");
                    None
                });
                let unavailable = workspace.ask_unavailable();
                if let Some(view) = workspace.ask.clone() {
                    view.update(cx, |view, cx| view.set_client(client, unavailable, cx));
                }
            })
            .ok();
        }));
    }

    /// An Ask result from the app channel: an answer goes to the thread,
    /// shown or not; when it is the selected turn's and the Thread shows,
    /// the Sources view lists its new sources (`AskFlow::answer_ready`).
    pub(super) fn ask_data(&mut self, data: AskData, cx: &mut Context<Self>) {
        match data {
            AskData::AnswerReady { turn_id, result } => {
                let Some(view) = self.ask.clone() else {
                    return;
                };
                view.update(cx, |view, cx| view.answer_ready(turn_id, result, cx));
                let selected = view.read(cx).selected_id();
                if self.ask_flow.answer_ready(turn_id, selected) {
                    self.with_window(cx, move |workspace, window, cx| {
                        workspace.refresh_sources(turn_id, window, cx)
                    });
                }
            }
            // The GUI's reader reads its note itself (the preview's task);
            // nothing sends this here.
            AskData::ReaderNote { .. } => {}
        }
    }

    /// List `turn_id`'s sources in the Sources view even when it lists
    /// that turn already (its answer landed), and record the sync.
    fn refresh_sources(&mut self, turn_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sources) = self
            .ask
            .as_ref()
            .and_then(|view| view.read(cx).turn_sources(turn_id))
        else {
            return;
        };
        if let Some(view) = self.ask_sources.clone() {
            view.update(cx, |view, cx| view.refresh(turn_id, sources, window, cx));
        }
        self.ask_flow.synced(turn_id);
    }

    /// The flow that says whether the Thread shows (tests).
    #[cfg(test)]
    pub(crate) fn ask_flow(&self) -> &AskFlow {
        &self.ask_flow
    }
}

crate::theme::impl_themed!(AskView);
