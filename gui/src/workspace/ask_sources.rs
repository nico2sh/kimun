//! The **Sources view** (M7c G10b): the drawer view ASK, beside the Thread
//! (CONTEXT.md: **Sources view**, **Source reader**). It lists the selected
//! turn's sources, best first, and reads the one picked in the **Source
//! reader** below the list: the whole note, its retrieved section in the
//! accent colour and scrolled into view.
//!
//! **Rows** are the turn's sources, built here and now (no load): the rank
//! in the lane, the heading (a journal source's date beside it, a bare
//! date when it has no heading), the score, the path under it. The filter
//! above the list narrows them by heading and path (the TUI's fuzzy filter).
//! No sources: the TUI's prompt alone.
//!
//! **The reader** is FIND's preview ([`SearchPreview`]) marking a section
//! instead of needles, driven by the same `PreviewModel`: Collapsed (the
//! list only) → Context (under the list) → Full (the whole view, the list
//! keeping the keyboard). While Collapsed each row shows `Expand →` at its
//! right end under the pointer (the list's row action): a click selects
//! that row and grows the reader to Context, as Right does, and the list
//! has the keyboard after. Open, a one-line **header** sits on top of the
//! reader: the source's title (its heading, else its path), cut short with
//! an ellipsis, and the size action, `Expand →` in Context and `Collapse ←`
//! in Full; a click on it does what Right / Left do, and the list keeps (or
//! gets) the keyboard. The note is read on tokio, keyed by path and the
//! source's ordinal: a newer pick replaces a read under way and its result
//! never shows; another section of the note on screen is laid out again
//! from the text already read.
//!
//! **Keys** (the list has the keyboard, as in the TUI): Up/Down/Home/End
//! move (Up from the first row goes to the filter), Enter cycles the
//! reader, Right grows it, Left shrinks it, Space cycles it; in Full,
//! Up/Down scroll it, `o` opens the source's note in the editor. Escape steps the reader back, then gives the Thread's
//! composer the keyboard. In the filter: Down, Enter or Escape go back to
//! the list. **Pointer** ([`ClickPolicy::Select`]): a click selects (the
//! reader follows), a double-click opens the source's note as `o` does, a
//! right-click offers `Open note`.
//!
//! **Sync** with the Thread is the workspace's (`AskFlow`): the view only
//! answers `set_turn` (a no-op for the turn shown), `refresh` (always
//! rebuilds and collapses), `reset`, `focus_source` (a citation's ordinal,
//! matched against the rows, never taken as a position) and `open_reader`.

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Enter, Escape, InputEvent, InputState, MoveDown};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render,
    Role, SharedString, Stateful, StatefulInteractiveElement as _, StyleRefinement, Styled as _,
    Subscription, Window, div,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::ask::AskSource;
use kimun_notes::logic::input::{Key, Mods};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::list::{Filter, ListRow, ListSetup, YankTarget};
use kimun_notes::logic::message::AppTx;
use kimun_notes::logic::preview::PreviewModel;
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::drawer_list::{
    DrawerList, DrawerListEvent, DrawerRow, MenuItem, OPEN_NOTE, RowAction, VerbId,
};
use super::files::{ClickPolicy, NOTE_ROW_REMS, ellipsis, lane_row};
use super::query_field::{self, query_field};
use super::search::{Mark, SearchPreview, SectionMark};
use crate::input;
use crate::theme;

/// Element ids, for tests.
pub(crate) const SOURCES_ID: &str = "ask-sources";
pub(crate) const SOURCES_FILTER_ID: &str = "ask-sources-filter";
pub(crate) const SOURCES_LIST_ID: &str = "ask-sources-list";
pub(crate) const READER_ID: &str = "ask-reader";
pub(crate) const READER_ROW_ID: &str = "ask-reader-row";
pub(crate) const READER_HEADER_ID: &str = "ask-reader-header";
pub(crate) const READER_ACTION_ID: &str = "ask-reader-action";

/// The reader's size action (a row's while Collapsed, the header's open):
/// its label and key hint growing, then shrinking from Full.
pub(crate) const EXPAND: &str = "Expand";
pub(crate) const COLLAPSE: &str = "Collapse";
/// The verb of the row action `Expand →`.
const EXPAND_VERB: VerbId = "expand";
const EXPAND_KEY: &str = "→";
const COLLAPSE_KEY: &str = "←";
const EXPAND_TIP: &str = "Expand the reader";
const COLLAPSE_TIP: &str = "Collapse the reader";

/// What the view says with no source to list, and with none left by the
/// filter (the TUI's words).
pub(crate) const NO_SOURCES: &str = "no sources — ask something";
pub(crate) const NO_RESULTS: &str = "No results";
/// The filter's name (the TUI's box title).
const FILTER_LABEL: &str = "filter";

/// The element id of the row of the source ranked `rank` (1-based) in its
/// turn.
pub(crate) fn source_row_id(rank: usize) -> SharedString {
    format!("ask-source:{rank}").into()
}

/// The element id of the Expand action on the row of the source ranked
/// `rank` (shown while the reader is Collapsed).
pub(crate) fn source_row_expand_id(rank: usize) -> SharedString {
    format!("ask-source-expand:{rank}").into()
}

/// The similarity as a whole percentage (the TUI's: the server's
/// `0.0..=1.0` score, clamped).
fn score_percent(score: f64) -> u32 {
    (score.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// One row: a turn's source and its 1-based rank in the turn's ranked list
/// (kept on the row, so a filter does not renumber it).
#[derive(Debug, Clone)]
pub(crate) struct SourceRow {
    pub(crate) rank: usize,
    pub(crate) source: AskSource,
    /// What the filter matches: the heading and the path (the TUI's).
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

impl ListRow for SourceRow {
    fn match_text(&self) -> Option<&str> {
        Some(&self.filter_text)
    }

    fn yank_target(&self) -> Option<YankTarget> {
        Some(YankTarget::path(self.source.path.to_string()))
    }
}

impl DrawerRow for SourceRow {
    fn element_id(&self) -> SharedString {
        source_row_id(self.rank)
    }

    fn menu_items(&self) -> Vec<MenuItem> {
        vec![MenuItem::open_note()]
    }

    /// `Expand →`, which the view shows only while its reader is Collapsed.
    fn hover_action(&self) -> Option<RowAction> {
        Some(
            RowAction::new(
                EXPAND_VERB,
                source_row_expand_id(self.rank),
                EXPAND,
                EXPAND_TIP,
            )
            .with_key_hint(EXPAND_KEY),
        )
    }

    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        let source = &self.source;
        let label = SharedString::from(format!("{} {}", self.rank, source.display_heading()));
        // A dated source with no heading shows its date as the title.
        let (title, title_color, date) = match (&source.date, source.heading.is_empty()) {
            (Some(date), true) => (date.clone(), theme::journal_date(t), None),
            (date, _) => (source.heading.clone(), theme::foreground(t), date.clone()),
        };
        let rank = div()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::accent(t))
            .child(self.rank.to_string());
        let heading = div()
            .flex()
            .gap_1()
            .min_w_0()
            .child(
                ellipsis(title)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(title_color),
            )
            .when_some(date, |line, date| {
                line.child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(theme::journal_date(t))
                        .child(date),
                )
            });
        lane_row(
            self.element_id(),
            label,
            selected,
            NOTE_ROW_REMS,
            Some(rank.into_any_element()),
            [
                heading.into_any_element(),
                ellipsis(source.path.to_string())
                    .text_xs()
                    .text_color(theme::secondary(t))
                    .into_any_element(),
            ],
            t,
        )
        .child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_xs()
                .text_color(theme::muted(t))
                .child(format!("{}%", score_percent(source.score))),
        )
    }
}

/// The section a source's reader marks.
fn section_of(source: &AskSource) -> Mark {
    Mark::Section(SectionMark {
        ordinal: source.ordinal,
        heading: source.match_heading().into(),
        chunk: source.text.as_str().into(),
    })
}

/// The reader header's title for `source`: the heading the row's label
/// reads (a journal source's date joined in), else its path.
fn reader_title(source: &AskSource) -> String {
    let heading = source.display_heading();
    if heading.is_empty() {
        source.path.to_string()
    } else {
        heading
    }
}

/// The Sources view. See the module docs.
pub(crate) struct SourcesView {
    pub(crate) list: Entity<DrawerList<SourceRow>>,
    pub(crate) reader: Entity<SearchPreview>,
    pub(crate) filter: Entity<InputState>,
    /// The turn listed, `None` before any (and after New).
    turn_id: Option<u64>,
    /// How much of the picked source's note shows, and for which row.
    model: PreviewModel,
    /// The source the reader was last pointed at (its turn, note and
    /// ordinal), `None` while it shows nothing.
    reading: Option<(Option<u64>, VaultPath, usize)>,
    /// How many times the reader was pointed (tests).
    #[cfg(test)]
    pub(crate) reader_pointed: usize,
    _subscriptions: Vec<Subscription>,
}

/// What the Sources view asks of the workspace.
#[derive(Debug, Clone)]
pub(crate) enum SourcesEvent {
    /// `o`: open the selected source's note in the editor.
    OpenNote(VaultPath),
}

impl EventEmitter<SourcesEvent> for SourcesView {}

impl SourcesView {}

impl SourcesView {
    fn new(
        vault: Arc<NoteVault>,
        list: Entity<DrawerList<SourceRow>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(FILTER_LABEL));
        let reader = cx.new(|cx| SearchPreview::new(vault, cx).with_ids(READER_ID, READER_ROW_ID));
        let subscriptions = vec![
            cx.subscribe(&filter, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().to_string();
                    this.list.update(cx, |list, cx| list.set_query(query, cx));
                    this.sync_preview(cx);
                }
            }),
            // A selection move: the reader follows (Context stays open,
            // Full collapses, the TUI's rule).
            cx.observe(&list, |this, _, cx| this.sync_preview(cx)),
        ];
        let mut view = Self {
            list,
            reader,
            filter,
            turn_id: None,
            model: PreviewModel::new(),
            reading: None,
            #[cfg(test)]
            reader_pointed: 0,
            _subscriptions: subscriptions,
        };
        view.rebuild(Vec::new(), cx);
        view.sync_row_actions(cx);
        view
    }

    /// The turn listed (tests).
    #[cfg(test)]
    pub(crate) fn turn_id(&self) -> Option<u64> {
        self.turn_id
    }

    /// The reader's state (tests).
    #[cfg(test)]
    pub(crate) fn reader_model(&self) -> &PreviewModel {
        &self.model
    }

    /// List `turn_id`'s `sources`, unless that turn is listed already: then
    /// the selection, the filter and the reader stay as they are.
    pub(crate) fn set_turn(
        &mut self,
        turn_id: u64,
        sources: Vec<AskSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.turn_id != Some(turn_id) {
            self.refresh(turn_id, sources, window, cx);
        }
    }

    /// List `turn_id`'s `sources` even when it is the turn listed (its
    /// answer landed with new sources): the filter cleared, the first row
    /// selected, the reader collapsed.
    pub(crate) fn refresh(
        &mut self,
        turn_id: u64,
        sources: Vec<AskSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.relist(Some(turn_id), sources, window, cx);
    }

    /// Back to no turn (New): no rows, the reader collapsed.
    pub(crate) fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.relist(None, Vec::new(), window, cx);
    }

    /// List `sources` as `turn_id`'s ([`Self::refresh`], [`Self::reset`]):
    /// the filter cleared, the first row selected, the reader collapsed.
    fn relist(
        &mut self,
        turn_id: Option<u64>,
        sources: Vec<AskSource>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.turn_id = turn_id;
        self.clear_filter_text(window, cx);
        self.rebuild(sources, cx);
        self.model.reset();
        self.sync_preview(cx);
        cx.notify();
    }

    /// Select the source a citation `[ordinal]` names, matched by ordinal
    /// against the rows (never `ordinal - 1`); the filter is cleared so it
    /// is never hidden, and the reader collapses. An ordinal no row has
    /// changes only that.
    pub(crate) fn focus_source(
        &mut self,
        ordinal: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_filter(window, cx);
        self.model.reset();
        let pos = self
            .list
            .read(cx)
            .position(|row| row.source.ordinal == ordinal);
        if let Some(pos) = pos {
            self.list.update(cx, |list, cx| list.select_at(pos, cx));
        }
        self.sync_preview(cx);
        cx.notify();
    }

    /// Read the source ranked `index + 1` (leader `a s`: the top one): it
    /// is selected and the reader shows it, opened to Context when
    /// collapsed, at its size otherwise (the TUI's `open_reader`). Nothing
    /// for an index past the rows.
    pub(crate) fn open_reader(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_filter(window, cx);
        let Some(path) = self
            .list
            .read(cx)
            .all_rows()
            .get(index)
            .map(|row| row.source.path.clone())
        else {
            return;
        };
        self.list.update(cx, |list, cx| list.select_at(index, cx));
        if self.model.is_collapsed() {
            self.model.toggle(Some(path));
        } else {
            self.model.repoint(Some(path));
        }
        self.sync_preview(cx);
        cx.notify();
    }

    /// The rows for `sources`, ranked in their order, under the fuzzy
    /// filter.
    fn rebuild(&mut self, sources: Vec<AskSource>, cx: &mut Context<Self>) {
        let empty = if sources.is_empty() {
            NO_SOURCES
        } else {
            NO_RESULTS
        };
        let rows: Vec<SourceRow> = sources
            .into_iter()
            .enumerate()
            .map(|(i, source)| SourceRow::new(i + 1, source))
            .collect();
        let setup = ListSetup {
            filter: Filter::Fuzzy,
            ..ListSetup::default()
        };
        self.list
            .update(cx, |list, cx| list.set_rows_with(rows, setup, empty, cx));
    }

    /// Empty the filter and show every row again.
    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_filter_text(window, cx);
        self.list.update(cx, |list, cx| list.set_query("", cx));
    }

    /// Empty the filter field (`set_value` reports no change).
    fn clear_filter_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.filter.read(cx).value().is_empty() {
            self.filter
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    pub(crate) fn selected_path(&self, cx: &App) -> Option<VaultPath> {
        self.list
            .read(cx)
            .selected_row()
            .map(|row| row.source.path.clone())
    }

    /// Keep the reader on the selected source; collapsed, nothing is read.
    /// The reader is told only when what it should show changed (another
    /// turn, note or section, or it opened or collapsed): every list notify
    /// lands here, and most leave the reader as it is.
    fn sync_preview(&mut self, cx: &mut Context<Self>) {
        let changed = self.model.sync(self.selected_path(cx));
        let wanted = (!self.model.is_collapsed())
            .then(|| {
                let list = self.list.read(cx);
                let source = &list.selected_row()?.source;
                Some((self.turn_id, source.path.clone(), source.ordinal))
            })
            .flatten();
        if wanted != self.reading {
            let (path, mark) = match wanted
                .as_ref()
                .and_then(|_| self.list.read(cx).selected_row())
            {
                Some(row) => (Some(row.source.path.clone()), section_of(&row.source)),
                None => (None, Mark::Needles(Arc::from(Vec::new()))),
            };
            self.reading = wanted;
            #[cfg(test)]
            {
                self.reader_pointed += 1;
            }
            self.reader
                .update(cx, |reader, cx| reader.show_marked(path, mark, cx));
        }
        self.sync_row_actions(cx);
        if changed {
            cx.notify();
        }
    }

    /// The rows' Expand action shows exactly while the reader is Collapsed
    /// (open, the header carries the size action). Every model change goes
    /// through [`Self::sync_preview`], which calls this; the list redraws
    /// only when the answer flips.
    fn sync_row_actions(&mut self, cx: &mut Context<Self>) {
        let collapsed = self.model.is_collapsed();
        self.list
            .update(cx, |list, cx| list.set_row_actions_enabled(collapsed, cx));
    }

    /// A click on a row's Expand action (the list selected the row): the
    /// reader grows to Context as Right does, and the list has the
    /// keyboard.
    pub(crate) fn expand_from_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.grow(cx);
        self.focus_list(window, cx);
    }

    /// Right: Collapsed → Context → Full (Full stays).
    fn grow(&mut self, cx: &mut Context<Self>) {
        if !self.model.is_full() {
            self.model.toggle(self.selected_path(cx));
        }
        self.sync_preview(cx);
        cx.notify();
    }

    /// Left (and Escape): Full → Context → Collapsed.
    fn shrink(&mut self, cx: &mut Context<Self>) {
        self.model.collapse_step(self.selected_path(cx));
        self.sync_preview(cx);
        cx.notify();
    }

    /// Enter, Space: Collapsed → Context → Full → Collapsed
    /// (the TUI's Enter).
    pub(crate) fn cycle(&mut self, cx: &mut Context<Self>) {
        self.model.toggle(self.selected_path(cx));
        self.sync_preview(cx);
        cx.notify();
    }

    /// A click on the reader header's action: Context grows to Full as
    /// Right does, Full shrinks to Context as Left does; the list has the
    /// keyboard after, so the arrows the hint names keep working.
    fn header_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.is_full() {
            self.shrink(cx);
        } else {
            self.grow(cx);
        }
        self.focus_list(window, cx);
    }

    /// The open reader's one-line header: the selected source's title and
    /// the size action. `None` with no source selected.
    fn reader_header(&self, t: &Theme, cx: &Context<Self>) -> Option<impl IntoElement + use<>> {
        let title = reader_title(&self.list.read(cx).selected_row()?.source);
        let (label, key, tip) = if self.model.is_full() {
            (COLLAPSE, COLLAPSE_KEY, COLLAPSE_TIP)
        } else {
            (EXPAND, EXPAND_KEY, EXPAND_TIP)
        };
        let hover = theme::foreground(t);
        let action = div()
            .id(READER_ACTION_ID)
            .test_support()
            .role(Role::Button)
            .aria_label(label)
            .flex()
            .flex_none()
            .gap_1()
            .whitespace_nowrap()
            .cursor_pointer()
            .text_color(theme::secondary(t))
            .hover(move |style| style.text_color(hover))
            .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                if event.standard_click() {
                    this.header_action(window, cx);
                }
            }))
            .child(label)
            .child(div().text_color(theme::muted(t)).child(key));
        Some(
            div()
                .id(READER_HEADER_ID)
                .test_support()
                .aria_label(SharedString::from(title.clone()))
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .px_2()
                .py_0p5()
                .text_xs()
                .child(ellipsis(title).flex_1().text_color(theme::muted(t)))
                .child(action),
        )
    }

    fn focus_list(&self, window: &mut Window, cx: &mut Context<Self>) {
        query_field::focus_list(&self.list, window, cx);
    }

    fn list_focused(&self, window: &Window, cx: &App) -> bool {
        query_field::list_focused(&self.list, window, cx)
    }

    /// Whether the filter has the keyboard (its copy chord is its own).
    pub(crate) fn filter_focused(&self, window: &Window, cx: &App) -> bool {
        self.filter.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Keys the list must not see first: in Full, Up/Down scroll the
    /// reader; Up from the first row goes to the filter; Escape with the
    /// reader open steps it back (the TUI's ladder).
    fn capture_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.list_focused(window, cx) {
            return false;
        }
        let Some(key) = input::key_down_input(event) else {
            return false;
        };
        if key.mods != Mods::NONE {
            return false;
        }
        match key.key {
            Key::Up | Key::Down if self.model.is_full() => {
                let rows = if key.key == Key::Up { -1 } else { 1 };
                self.reader
                    .update(cx, |reader, cx| reader.scroll_rows(rows, cx));
                true
            }
            // The filter is drawn only while there are sources (the same
            // test as the render's); a filter that matches none still gets
            // the key.
            Key::Up
                if !self.list.read(cx).all_rows().is_empty()
                    && query_field::at_top(&self.list, cx) =>
            {
                query_field::focus_input(&self.filter, window, cx);
                true
            }
            Key::Esc if !self.model.is_collapsed() => {
                self.shrink(cx);
                true
            }
            _ => false,
        }
    }

    /// The reader's keys the list left alone.
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.list_focused(window, cx) {
            return false;
        }
        let Some(key) = input::key_down_input(event) else {
            return false;
        };
        if key.mods != Mods::NONE {
            return false;
        }
        match key.key {
            Key::Right => self.grow(cx),
            Key::Left => self.shrink(cx),
            Key::Char(' ') => self.cycle(cx),
            // The TUI's list verb `o`: the note, in the editor.
            Key::Char('o') => match self.selected_path(cx) {
                Some(path) => cx.emit(SourcesEvent::OpenNote(path)),
                None => return false,
            },
            _ => return false,
        }
        true
    }
}

impl Focusable for SourcesView {
    /// The list (the TUI's Sources view opens on it).
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.list.read(cx).focus_handle(cx)
    }
}

impl Render for SourcesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        let has_sources = !self.list.read(cx).all_rows().is_empty();
        let full = self.model.is_full();
        let list = div()
            .flex()
            .flex_col()
            .min_h_0()
            // In Full the list keeps the keyboard out of sight.
            .map(|area| {
                if full {
                    area.h_0().overflow_hidden()
                } else {
                    area.flex_1()
                }
            })
            .child(self.list.clone());
        // Collapsed: the list alone (its rows carry Expand); open, the
        // header tops the reader.
        let pane = (!self.model.is_collapsed()).then(|| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .when(!full, |pane| {
                    pane.border_t_1().border_color(theme::divider(&t))
                })
                .children(self.reader_header(&t, cx))
                .child(
                    self.reader
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                )
        });
        div()
            .id(SOURCES_ID)
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .text_color(theme::foreground(&t))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.capture_key(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.key(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            // The filter's keys that mean something else here: back to the
            // list.
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                this.focus_list(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                this.focus_list(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                this.focus_list(window, cx);
                cx.stop_propagation();
            }))
            .when(has_sources, |view| {
                view.child(query_field(
                    &self.filter,
                    SOURCES_FILTER_ID,
                    FILTER_LABEL,
                    &t,
                ))
            })
            .child(list)
            .children(pane)
    }
}

// ---- The workspace's side ----------------------------------------------------

impl Workspace {
    /// Make the Sources view over the vault the window opened (no turn).
    pub(super) fn start_ask_sources(
        &mut self,
        vault: Arc<NoteVault>,
        tx: AppTx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A click picks the source the reader reads; a double-click opens
        // its note.
        let list = DrawerList::spawn_with(SOURCES_LIST_ID, "Sources", tx, cx, |list| {
            list.with_click_policy(ClickPolicy::Select)
        });
        let view = cx.new(|cx| SourcesView::new(vault, list.clone(), window, cx));
        self._views_events
            .push(cx.subscribe_in(&list, window, Self::on_sources_list_event));
        self._views_events.push(cx.subscribe_in(
            &view,
            window,
            |this, _, event: &SourcesEvent, window, cx| match event {
                SourcesEvent::OpenNote(path) => this.open_source_note(path.clone(), window, cx),
            },
        ));
        self.ask_sources = Some(view);
    }

    fn on_sources_list_event(
        &mut self,
        _: &Entity<DrawerList<SourceRow>>,
        event: &DrawerListEvent<SourceRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            DrawerListEvent::Activate(_) => {
                if let Some(view) = self.ask_sources.clone() {
                    view.update(cx, |view, cx| view.cycle(cx));
                }
            }
            DrawerListEvent::Verb(row, OPEN_NOTE) => {
                self.open_source_note(row.source.path.clone(), window, cx)
            }
            DrawerListEvent::Verb(_, EXPAND_VERB) => {
                if let Some(view) = self.ask_sources.clone() {
                    view.update(cx, |view, cx| view.expand_from_row(window, cx));
                }
            }
            DrawerListEvent::Verb(..) => {}
            DrawerListEvent::Dismiss => self.focus_ask_composer(window, cx),
            // A source is not offered for rename / move / delete here.
            DrawerListEvent::FileOp(..) => {}
        }
    }

    /// Open a source's note in the editor (the `o` key, the row menu's
    /// `Open note`, a double-click on the row): the Thread goes down, the note
    /// opens and the editor has the keyboard.
    pub(super) fn open_source_note(
        &mut self,
        path: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_chosen(path, None, window, cx);
    }

    /// Escape on the Sources list: the Thread's composer has the keyboard
    /// (the editor Panel, whatever shows there, when the Thread does not).
    /// Esc never leaves zoom: with a slot zoomed the keyboard stays.
    fn focus_ask_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.zoom_keeps_keyboard() {
            return;
        }
        if let Some(view) = self.ask.clone().filter(|_| self.ask_shown()) {
            view.update(cx, |view, _| view.want_composer());
        }
        self.focus_panel(Panel::Editor, window, cx);
    }

    /// Point the Sources view at the Thread's selected turn (no-op for the
    /// turn it lists), and record that it did (`AskFlow::synced`). Nothing
    /// without a selected turn.
    pub(super) fn sync_sources(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, sources)) = self
            .ask
            .as_ref()
            .and_then(|v| v.read(cx).selected_sources())
        else {
            return;
        };
        if let Some(view) = self.ask_sources.clone() {
            view.update(cx, |view, cx| view.set_turn(id, sources, window, cx));
        }
        self.ask_flow.synced(id);
    }

    /// After an input on the Thread (a selection moved, a question sent, a
    /// citation clicked): what `AskFlow::after_input` says — the Sources
    /// view follows the selected turn, and a clicked `[n]` selects its
    /// source.
    pub(super) fn after_ask_input(
        &mut self,
        citation: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self.ask.as_ref().and_then(|v| v.read(cx).selected_id());
        let step = self.ask_flow.after_input(selected, citation);
        if step.sync_sources {
            self.sync_sources(window, cx);
        }
        if let Some(ordinal) = step.focus_citation
            && let Some(view) = self.ask_sources.clone()
        {
            view.update(cx, |view, cx| view.focus_source(ordinal, window, cx));
        }
    }

    /// Whether the Sources view's filter has the keyboard.
    pub(super) fn sources_filter_focused(&self, window: &Window, cx: &App) -> bool {
        self.ask_sources
            .as_ref()
            .is_some_and(|view| view.read(cx).filter_focused(window, cx))
    }
}
