//! The **Ask workspace** (M7c G10a/b, ADR-0053): the view ASK, one
//! view in a slot (right by default, `view_registry`'s `home_right`) that
//! shows the **Thread** with the selected turn's sources and the **Source
//! reader** folded in (CONTEXT.md: **Ask workspace**, **Thread**, **Turn**,
//! **Sources view**, **Source reader**). The thread model and its rules are
//! the logic's (`kimun_notes::ask::Thread`, `ask::jobs`, `ask::save`,
//! `logic::ask_flow::AskFlow`); this file draws it and drives it.
//!
//! **The view.** The turns, oldest first, in one scrolling column: each
//! turn's question, then its answer, or a status line while it is pending
//! (`… thinking`) or failed (`✗ {message}` and the retry hint). The selected
//! turn (the one Copy, Save as note and Regenerate act on) has the selected
//! fill and, once answered with sources, a `Sources (n)` toggle. Under the
//! turns: the turn actions, then the composer (one line, the TUI's) with
//! Send. The actions are enabled by the TUI's rule (`sync_actions`): Send
//! needs a server and a question, the turn actions a turn; Regenerate and
//! New also need the server (the leader gates them so).
//!
//! **Sources, beside or instead.** The toggle (or `s` on the turn list,
//! leader `a s`, a citation) opens the selected turn's sources: the
//! [`super::ask_sources`] list and its reader. In a wide slot (zoomed, or
//! [`WIDE_REMS`] and up) they are a pane at the view's right, the Thread
//! beside them. In a narrow slot they replace the Thread (drill-in): a
//! back header, and Esc once the reader is back, go up. The part follows
//! the selected turn (`AskFlow`, the one record) and stays open across
//! selection moves until toggled shut.
//!
//! **Answers** are drawn by the shared answer rules (`kimun_notes::ask::
//! markdown`, the TUI's too, over the editor's markdown model; mapped by
//! [`answer_lines`]), once, when the answer lands: code lines in the monospace face,
//! headings semibold, quotes muted, and on ordinary lines bold, italic and
//! struck-through runs with their sigils hidden and inline code verbatim on
//! a tint; the `[n]` citations in the accent colour. A citation that names
//! one of the turn's sources is a link (a pointing hand over it): a click
//! selects the turn, expands its sources and opens the reader on that
//! source (the TUI's `click_turns`, the reader entered from a citation).
//!
//! **Keys** (the TUI's). The composer: Enter sends; Escape gives the turn
//! list the keyboard; with the composer empty, Up/Down move the selection;
//! PageUp/PageDown scroll the turns. The turn list: Up/Down (k/j) move the
//! selection, PageUp/PageDown/Home/End scroll, i or / go back to the
//! composer, s expands (or shuts) the sources and gives their list the
//! keyboard, y copies, e saves as a note, r regenerates; Escape gives the
//! composer the keyboard back (never leaving the slot, so never a zoom).
//! The sources list's own keys are [`super::ask_sources`]'.
//!
//! **Asking.** Send pushes a pending turn and spawns the logic's ask job on
//! tokio; its `AppMessage::Ask(AnswerReady)` completes or fails the turn
//! (a stale id is dropped by `Thread`). Regenerate rewinds the selected
//! turn and asks again. Both follow the bottom: a new turn, and an answer
//! that lands on the selected turn, scroll into view.
//!
//! **The workspace's side** (below): ASK is shown and hidden like any view
//! (rail, leader, side toggles, presets, the saved layout); it never
//! replaces the editor area, so the open note stays open and untouched
//! while Ask shows. The rail entry is capability-gated (server reachable
//! with an LLM); losing the capability keeps the view readable, asking
//! disabled. The thread lives in memory for the vault session, one at a
//! time, shown or not.

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
    Window, div, point, px, rems,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::ask::{AskSource, Thread, Turn, TurnStatus, jobs, markdown, save};
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout_tree::{Focus, Leaf};
use kimun_notes::logic::leader_route::AskStep;
use kimun_notes::logic::message::{AppTx, AskData};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::rag_caps::{self, Caps};
use kimun_notes::logic::view_registry;
use kimun_notes::rag::RagStatus;
use kimun_notes::server_client::RagClient;

use super::Workspace;
use super::ask_sources::SourcesView;
use super::note::Closing;
use super::route::NO_WORKSPACE;
use super::tree_view::{SlotAware, SlotContext};
use crate::input;
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
/// The zoom / restore button: one id, its label follows the state.
pub(crate) const ZOOM_ID: &str = "ask-zoom";
/// The selected turn's `Sources (n)` toggle, and the part it expands.
pub(crate) const SOURCES_TOGGLE_ID: &str = "ask-sources-toggle";
pub(crate) const SOURCES_PART_ID: &str = "ask-sources-part";

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
/// The sources pane's width beside the Thread (wide layout).
const SOURCES_PANE_REMS: f32 = 26.0;
/// The slot width from which the sources sit beside the Thread; under it
/// they take the whole view (drill-in).
pub(crate) const WIDE_REMS: f32 = 48.0;
/// The drill-in header's back button.
pub(crate) const BACK_ID: &str = "ask-sources-back";
/// The key that expands the sources (the toggle's hint).
const SOURCES_KEY: &str = "s";
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
    /// The zoom button: zoom the Thread's slot, or leave zoom.
    ToggleZoom,
    /// Escape on the turn list: the composer gets the keyboard back
    /// (the workspace's one door for it, `focus_ask_composer`).
    Dismiss,
    /// The selection may have moved (a key, a click, a question sent, a
    /// regenerate), and a click on a citation names the source `[n]` of
    /// the selected turn: the Sources view follows.
    Input(Option<usize>),
}

/// Where the keyboard goes when the view gets it (the TUI's
/// `ThreadFocus`, plus the sources part).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Want {
    Composer,
    Turns,
    Sources,
}

/// The Ask view. See the module docs.
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
    /// What takes the keyboard when the view gets it: the composer (while
    /// a server can answer), the turn list, or the sources part (while it
    /// is drawn).
    want: Want,
    scroll: ScrollHandle,
    /// The selected turn's sources and reader (the workspace makes it over
    /// the vault), drawn in place under the selected turn while open.
    sources: Option<Entity<SourcesView>>,
    sources_open: bool,
    /// Whether this view's slot is zoomed (the workspace says, each time
    /// it draws the slot): the zoom button's label.
    zoomed: bool,
    /// Whether the slot is wide enough for the sources beside the Thread
    /// (the workspace says, as for `zoomed`).
    wide: bool,
    /// Each done turn's answer as drawn, by turn id: made once when the
    /// answer landed, dropped when the turn is asked again or the thread
    /// cleared.
    answers: HashMap<u64, DrawnAnswer>,
    tx: AppTx,
    _composer_events: Subscription,
}

impl EventEmitter<AskEvent> for AskView {}

impl AskView {
    pub(crate) fn new(tx: AppTx, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            want: Want::Composer,
            scroll: ScrollHandle::new(),
            sources: None,
            sources_open: false,
            zoomed: false,
            wide: false,
            answers: HashMap::new(),
            tx,
            _composer_events: composer_events,
        }
    }

    /// Fold `sources` (the workspace's, over the vault) in: drawn beside
    /// the Thread, or in place of it, while open.
    pub(crate) fn set_sources(&mut self, sources: Entity<SourcesView>, cx: &mut Context<Self>) {
        self.sources = Some(sources);
        cx.notify();
    }

    /// Whether the sources part is expanded (drawn when the selected turn
    /// has sources). Tests.
    #[cfg(test)]
    pub(crate) fn sources_open(&self) -> bool {
        self.sources_open
    }

    /// Whether the sources part is drawn: expanded, and the selected turn
    /// answered with sources.
    pub(crate) fn sources_drawn(&self) -> bool {
        self.sources_open
            && self.sources.is_some()
            && self.thread.selected().is_some_and(has_sources)
    }

    /// Expand the sources part, its list to get the keyboard when the view
    /// does (leader `a s`).
    pub(crate) fn open_sources(&mut self, cx: &mut Context<Self>) {
        self.sources_open = true;
        self.want = Want::Sources;
        cx.notify();
    }

    /// Expand the sources part without moving the keyboard (a citation).
    fn reveal_sources(&mut self, cx: &mut Context<Self>) {
        self.sources_open = true;
        cx.notify();
    }

    /// The toggle, `s` on the turn list: expand the part and give its list
    /// the keyboard, or shut it (the turn list keeps or gets the keyboard).
    fn toggle_sources(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sources_open {
            self.sources_open = false;
            self.focus_turns(window, cx);
        } else if self.thread.selected().is_some_and(has_sources) {
            self.open_sources(cx);
            let handle = self.focus_handle(cx);
            window.focus(&handle, cx);
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

    /// The composer gets the keyboard when the view does (leader `a a`,
    /// F6, New, Escape on the turn list or the sources list).
    pub(crate) fn want_composer(&mut self, cx: &mut Context<Self>) {
        self.want = Want::Composer;
        cx.notify();
    }

    /// Escape up from the turn list or the sources list: the composer is
    /// wanted, and drilled into the sources (the composer behind them)
    /// going up is leaving them. Only this path closes them: focus cycling
    /// through [`Self::want_composer`] leaves the reader as it is.
    pub(crate) fn dismiss_to_composer(&mut self, cx: &mut Context<Self>) {
        if self.drilled() {
            self.sources_open = false;
        }
        self.want_composer(cx);
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

    /// Start over: no turns, the sources shut, the composer wanted.
    pub(crate) fn clear(&mut self, cx: &mut Context<Self>) {
        self.thread.clear();
        self.answers.clear();
        self.sources_open = false;
        self.want = Want::Composer;
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
    /// answers to `n` (by ordinal), the sources part expands and the
    /// workspace opens the reader on it. An unknown `[n]` is no link.
    fn click_citation(&mut self, id: u64, n: usize, cx: &mut Context<Self>) {
        let Some(ix) = self.thread.turns().iter().position(|t| t.id == id) else {
            return;
        };
        self.thread.select_index(ix);
        let known = self
            .thread
            .selected()
            .is_some_and(|turn| turn.source_for_citation(n).is_some());
        if known {
            self.reveal_sources(cx);
        }
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
        self.want = Want::Composer;
        let handle = self.focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    /// Whether the composer has the keyboard.
    fn composer_focused(&self, window: &Window, cx: &App) -> bool {
        self.composer.read(cx).focus_handle(cx).is_focused(window)
    }

    fn focus_turns(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.want = Want::Turns;
        window.focus(&self.turns_focus, cx);
        cx.notify();
    }

    /// A key on the turn list (the TUI's `handle_turns_key`). True when
    /// taken.
    fn turns_key(&mut self, key: KeyInput, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // Only the turn list's own keys: the sources part sits inside the
        // turn column, and its filter and list must never fire Thread
        // commands (Ruling 21).
        if !self.turns_focus.is_focused(window) {
            return false;
        }
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
            Key::Char('s') => self.toggle_sources(window, cx),
            Key::Char('y') => cx.emit(AskEvent::Copy),
            Key::Char('e') => cx.emit(AskEvent::Save),
            Key::Char('r') => self.regenerate(cx),
            Key::Esc => cx.emit(AskEvent::Dismiss),
            _ => return false,
        }
        true
    }

    fn render_turn(&self, turn: &Turn, selected: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
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
        let toggle = (selected && has_sources(turn)).then(|| self.render_sources_toggle(turn, cx));
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
            .children(toggle)
            .into_any_element()
    }

    /// The selected turn's `Sources (n)` toggle: expands the part under the
    /// turn (its list then has the keyboard), or shuts it.
    fn render_sources_toggle(&self, turn: &Turn, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let label: SharedString = format!("Sources ({})", turn.sources.len()).into();
        let hover = theme::foreground(t);
        div()
            .id(SOURCES_TOGGLE_ID)
            .test_support()
            .role(Role::Button)
            .aria_label(label.clone())
            .aria_expanded(self.sources_open)
            .flex()
            .flex_none()
            .gap_1()
            .text_xs()
            .whitespace_nowrap()
            .cursor_pointer()
            .text_color(theme::secondary(t))
            .hover(move |style| style.text_color(hover))
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                if event.standard_click() {
                    this.toggle_sources(window, cx);
                }
            }))
            .child(label)
            .child(div().text_color(theme::muted(t)).child(SOURCES_KEY))
            .into_any_element()
    }

    /// The drill-in header: back to the Thread, and the zoom toggle.
    fn render_back_header(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_none()
            .flex_row()
            .gap_2()
            .px_3()
            .py_1()
            .child(
                Button::new(BACK_ID)
                    .small()
                    .label("← Thread")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.sources_open = false;
                        this.focus_turns(window, cx);
                    })),
            )
            .child(self.render_zoom_button(cx))
            .into_any_element()
    }

    /// The sources part: the list and the reader, beside the Thread when
    /// the slot is wide, else in place of it (with a back header). A press in it is the part's (the list or
    /// the filter takes the keyboard), never the turn list's.
    fn render_sources_part(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.sources_drawn() {
            return None;
        }
        let sources = self.sources.clone()?;
        let theme = theme::current(cx);
        let t = &*theme;
        Some(
            div()
                .id(SOURCES_PART_ID)
                .test_support()
                .role(Role::Group)
                .aria_label("Sources")
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .py_1()
                .when(self.wide, |part| {
                    part.flex_none().w(rems(SOURCES_PANE_REMS)).border_l_1()
                })
                .border_color(theme::divider(t))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.want = Want::Sources;
                        cx.stop_propagation();
                    }),
                )
                .child(sources)
                .into_any_element(),
        )
    }

    /// A done turn's answer, line by line, as [`DrawnAnswer`] made it when
    /// it landed (render only reads it). A line with citations naming one
    /// of the turn's sources is interactive text: a click on one is
    /// hit-tested on the shaped line ([`Self::click_citation`]) and shows
    /// the pointing hand.
    fn render_answer(&self, turn: &Turn, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
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

    /// Whether the sources replace the Thread (drilled in): drawn, in a
    /// slot too narrow to hold both.
    fn drilled(&self) -> bool {
        self.sources_drawn() && !self.wide
    }

    /// Drilling in unmounts the Thread column (turn list and composer):
    /// a keyboard on either would be on a handle no longer in the tree and
    /// keys would stop arriving. It goes to the sources list instead.
    /// Keyboard elsewhere (the editor, say) is left alone.
    fn rehome_focus_when_drilled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.drilled() {
            return;
        }
        let in_thread = self.turns_focus.is_focused(window) || self.composer_focused(window, cx);
        if !in_thread {
            return;
        }
        self.want = Want::Sources;
        let handle = self.focus_handle(cx);
        window.focus(&handle, cx);
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
            .child(self.render_zoom_button(cx))
    }

    /// The slot's zoom toggle. Drawn in the Thread's actions row and, with
    /// the Thread gone, in the drill-in header (never both): zooming is how
    /// a narrow slot gets the sources beside the Thread.
    fn render_zoom_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new(ZOOM_ID)
            .small()
            .label(if self.zoomed { "Zoom out" } else { "Zoom" })
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(AskEvent::ToggleZoom)))
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
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
                                    this.want = Want::Composer;
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

/// Whether `turn` is answered with sources to list.
fn has_sources(turn: &Turn) -> bool {
    matches!(turn.status, TurnStatus::Done) && !turn.sources.is_empty()
}

impl SlotAware for AskView {
    /// Zoomed (the button's label) and wide enough for the sources beside
    /// the Thread ([`WIDE_REMS`]); redraws only on a change.
    fn set_slot(&mut self, slot: SlotContext, cx: &mut Context<Self>) {
        let wide = slot.width >= f64::from(WIDE_REMS) * slot.rem;
        if self.zoomed != slot.zoomed || self.wide != wide {
            self.zoomed = slot.zoomed;
            self.wide = wide;
            cx.notify();
        }
    }
}

impl Focusable for AskView {
    /// The sources list while it is wanted and drawn; the composer while it
    /// is wanted and a server can answer; else the turn list.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match (self.want, self.sources.as_ref()) {
            (Want::Sources, Some(sources)) if self.sources_drawn() => {
                sources.read(cx).focus_handle(cx)
            }
            (Want::Composer, _) if self.client.is_some() => self.composer.read(cx).focus_handle(cx),
            _ => self.turns_focus.clone(),
        }
    }
}

impl AskView {
    /// The Thread column: the turn list, then the actions and the composer.
    /// Not drawn while the sources are drilled into, so not built then.
    fn render_thread(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let selected = self.thread.selected().map(|turn| turn.id);
        let mut turns: Vec<AnyElement> = Vec::with_capacity(self.thread.turns().len() + 1);
        for turn in self.thread.turns() {
            let is_selected = Some(turn.id) == selected;
            turns.push(self.render_turn(turn, is_selected, cx));
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
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
            .into_any_element()
    }
}

impl Render for AskView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.rehome_focus_when_drilled(window, cx);
        let drilled = self.drilled();
        let wide_pane = self.sources_drawn() && self.wide;
        // Drawn only when drilled in or beside the Thread, never else.
        let part = self.render_sources_part(cx);
        let (drilled_part, pane_part) = if drilled { (part, None) } else { (None, part) };
        let thread = (!drilled).then(|| self.render_thread(cx));
        let root = if wide_pane {
            div().flex_row()
        } else {
            div().flex_col()
        };
        root.id(ASK_ID)
            .test_support()
            .role(Role::Group)
            .aria_label(view_registry::spec(DrawerView::Ask).name)
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            // The composer's keys that mean something else here (only the
            // composer's: the sources filter under a turn has its own).
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                if this.composer_focused(window, cx) {
                    this.send(window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.composer_focused(window, cx) {
                    this.focus_turns(window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                if this.composer_focused(window, cx) && this.composer_text(cx).is_empty() {
                    this.select_step(true, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                if this.composer_focused(window, cx) && this.composer_text(cx).is_empty() {
                    this.select_step(false, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MovePageUp, window, cx| {
                if this.composer_focused(window, cx) {
                    this.scroll_by_key(Key::PageUp, window);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MovePageDown, window, cx| {
                if this.composer_focused(window, cx) {
                    this.scroll_by_key(Key::PageDown, window);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .when(drilled, |root| root.child(self.render_back_header(cx)))
            .when_some(drilled_part, |root, part| root.child(part))
            .children(thread)
            .when_some(pane_part, |root, part| root.child(part))
    }
}

// ---- The workspace's side ----------------------------------------------------

/// What showing the view goes on to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AskThen {
    /// Give the composer the keyboard (leader `a a`, F6).
    Composer,
    /// Start over, the composer then (leader `a n`, New).
    New,
    /// Regenerate the selected turn (leader `a r`).
    Regenerate,
    /// Read the selected turn's top source, the sources part expanded and
    /// its list given the keyboard (leader `a s`).
    Source,
}

impl Workspace {
    /// Build the Thread for the vault the window opened (empty, no server
    /// until the status says one can answer). The sources part follows the
    /// selected turn whether the view shows or not (`AskFlow` held showing:
    /// the part is the view's own, never a second surface to enter).
    pub(super) fn start_ask(&mut self, tx: AppTx, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.new(|cx| AskView::new(tx, window, cx));
        self._ask_events = Some(cx.subscribe_in(&view, window, Self::on_ask_event));
        self.ask = Some(view);
        self.ask_flow.enter();
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
            AskEvent::ToggleZoom => self.toggle_zoom(window, cx),
            AskEvent::Dismiss => self.focus_ask_composer(window, cx),
            AskEvent::Input(citation) => self.after_ask_input(*citation, window, cx),
        }
    }

    /// Whether the view ASK is shown in a slot (with a vault to ask about).
    pub(crate) fn ask_shown(&self) -> bool {
        self.ask.is_some() && self.is_shown(DrawerView::Ask)
    }

    /// Why the server cannot answer questions now, in the TUI's words.
    pub(crate) fn ask_unavailable(&self) -> Option<&'static str> {
        rag_caps::ask_unavailable_message(self.rag_status)
    }

    /// Why the leader's `+ask` `step` cannot run now, or `None` when it
    /// can. Copy and Save only read the thread; focusing the view or
    /// reading a source while it already shows needs nothing (the TUI's
    /// `open_ask_workspace`); New and Regenerate, and showing it, need a
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

    /// Why the leader's `drawer.ask` (a leader id shared with the TUI, hence
    /// "drawer") cannot show the view now: the rail's
    /// rule, a server that can answer; a view already shown just takes
    /// the keyboard.
    pub(crate) fn ask_view_block(&self) -> Option<&'static str> {
        if self.ask.is_none() {
            return Some(NO_WORKSPACE);
        }
        if self.ask_shown() {
            return None;
        }
        self.ask_unavailable()
    }

    /// The leader's `drawer.ask`: the view shown where it is placed (its
    /// home, the right slot, when placed nowhere) with the keyboard, as any
    /// view the leader opens. Gated here too: a leaf sent on the channel
    /// skips the route block.
    pub(crate) fn open_ask_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(why) = self.ask_view_block() {
            self.flash(why, cx);
            return;
        }
        self.show_view(DrawerView::Ask, window, cx);
        cx.notify();
    }

    /// Run the leader's `+ask` `step` (also F6 = Focus, and the view's
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

    /// Show the view in its slot (where it is placed, else its home on the
    /// right) with the keyboard, then do `then`. The open note is not
    /// touched: Ask sits beside it.
    pub(crate) fn enter_ask(&mut self, then: AskThen, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.ask.clone() else {
            return;
        };
        if self.closing != Closing::Open {
            return;
        }
        match then {
            AskThen::Composer => view.update(cx, |view, cx| view.want_composer(cx)),
            AskThen::New => {
                view.update(cx, |view, cx| view.clear(cx));
                if let Some(sources) = self.ask_sources.clone() {
                    sources.update(cx, |sources, cx| sources.reset(window, cx));
                }
            }
            AskThen::Regenerate => view.update(cx, |view, cx| view.regenerate(cx)),
            AskThen::Source => {
                // The TUI's `a s`: the part on the selected turn, its top
                // source in the reader, its list with the keyboard.
                self.sync_sources(window, cx);
                if let Some(sources) = self.ask_sources.clone() {
                    sources.update(cx, |sources, cx| sources.open_reader(0, window, cx));
                }
                view.update(cx, |view, cx| view.open_sources(cx));
            }
        }
        self.place_rail_cursor(Some(DrawerView::Ask));
        self.open_view_focused(DrawerView::Ask);
        self.tidy_layout(cx);
        self.apply_focus(window, cx);
        self.save_layout(cx);
        cx.notify();
    }

    /// Escape on the turn list or the sources list: the composer gets the
    /// keyboard. The one door for it: the keyboard stays in ASK's slot, so
    /// a zoom on it never ends (Esc never leaves zoom, ADR-0053), and with
    /// another leaf zoomed (ASK out of sight) nothing moves.
    pub(super) fn focus_ask_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.ask.clone() else {
            return;
        };
        let Some(slot) = self
            .layout
            .view_slot(&DrawerView::Ask)
            .filter(|slot| self.layout.shown(*slot) == Some(DrawerView::Ask))
        else {
            return;
        };
        if self
            .layout
            .zoomed()
            .is_some_and(|leaf| leaf != Leaf::Slot(slot))
        {
            return;
        }
        view.update(cx, |view, cx| view.dismiss_to_composer(cx));
        self.layout.focus(Focus::Leaf(Leaf::Slot(slot)));
        self.apply_focus(window, cx);
        cx.notify();
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
    /// shown or not; when it is the selected turn's, the sources part
    /// lists its new sources (`AskFlow::answer_ready`).
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

    /// List `turn_id`'s sources in the sources part even when it lists
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
}
