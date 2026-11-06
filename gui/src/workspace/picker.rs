//! The **note picker** (M6 G4): one component for choosing a note, in the
//! window's modal slot. Two doors open it:
//!
//! - the **finder** (`OpenNote` chord, leader `f f`): every note, most
//!   recently changed first, fuzzy-ranked as you type, with a `Create:` row
//!   for whatever was typed (`FileFinderProvider`);
//! - the **link chooser**: a followed link that several notes answer to
//!   lists every one of them, narrowed by what you type
//!   (`LinkResultsProvider`);
//! - the **search browser** (`SearchNotes` chord, leader `f g` / `f r`, a
//!   followed `#tag`): the vault's query search, with a preview column
//!   ([`super::search`], M7b G4) and a sort chip ([`super::sort`], M7b G7);
//! - the **saved searches** (`OpenSavedSearches` chord, leader `f s`): the
//!   workspace's saved searches under the virtual Backlinks entry, run in
//!   FIND ([`super::saved_searches`], M7b G6).
//!
//! The picker is generic over its row ([`PickerRow`]): notes for the first
//! three doors, saved searches for the last; the input, keys, list and
//! dialog are one.
//!
//! **State.** The rows, their order, the `Create:` row and the selection are
//! the logic [`ListCore`]'s, as in FILES; the sources load and rank off the
//! UI thread and a newer query's rows replace an older one's (the core's
//! generation guard). The query lives in the kit's [`InputState`]. Rows are
//! drawn like FILES' rows ([`entry_row`]) in a virtualised list.
//!
//! **Keys.** Typing filters; Up/Down move the selection; Enter chooses it;
//! Escape closes the picker and gives the keyboard back to its opener. The
//! finder's own chord closes it too (the modal host's toggle).
//!
//! **Pointer** ([`ClickPolicy`]). A click chooses a row as Enter does,
//! under the pointing hand — except in the search browser, whose selection
//! drives the preview: there a click selects and a double-click opens the
//! row it hits. While a `Create:` runs no row can be chosen (no hand).
//!
//! **Choosing** is the logic's rule (`picker_open`): a note opens through the
//! note session (the open note is saved first), and `Create:` makes the note
//! through core before opening it. A create that fails keeps the picker open
//! and says why; the next keystroke clears it.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    Delete, Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp, Position,
};
use gpui_kit::component::window_paddings;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, Div, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, ScrollStrategy, SharedString, Stateful,
    StatefulInteractiveElement as _, StyleRefinement, Styled as _, Subscription,
    UniformListScrollHandle, WeakEntity, Window, div, rems, uniform_list,
};
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;
use kimun_notes::logic::file_ops;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::list::{Filter, ListCore, ListRow, ListSetup, RowSource};
use kimun_notes::logic::message::SortTarget;
use kimun_notes::logic::message::redraw_callback;
use kimun_notes::logic::pickers::{
    FileFinderProvider, LinkResultsProvider, PickerOpen, picker_open,
};
use kimun_notes::logic::query::{QueryContext, emphasis_needles, error_reason, resolve_query};
use kimun_notes::logic::sort::{SortField, SortOrder, order_of_query, query_with_sort};
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::files::{ClickPolicy, ClickRows, entry_row, with_click_cursor};
use super::modal::{self, ModalKind, Slot};
use super::search::SearchPreview;
use super::sort::{chip_label, sort_chip};
use crate::tasks;
use crate::theme;

/// Element ids, for tests.
pub(crate) const PICKER_ID: &str = "note-picker";
pub(crate) const PICKER_INPUT_ID: &str = "note-picker-input";
pub(crate) const PICKER_LIST_ID: &str = "note-picker-list";
pub(crate) const PICKER_ERROR_ID: &str = "note-picker-error";
pub(crate) const QUERY_ERROR_ID: &str = "note-picker-query-error";
pub(crate) const HELP_BUTTON_ID: &str = "note-picker-help";
pub(crate) const PICKER_DELETE_ID: &str = "note-picker-delete";
pub(crate) const BROWSER_SORT_ID: &str = "note-picker-sort";

/// The picker's width, in rems.
const WIDTH_REMS: f32 = 36.;
/// The list's height, in rems: about seven rows.
const LIST_REMS: f32 = 20.;
/// The search browser's width, in rems: results and preview side by side.
const SEARCH_WIDTH_REMS: f32 = 64.;
/// Below this window width (rems) the preview goes under the results.
const SEARCH_NARROW_REMS: f32 = 52.;
/// The results' width beside the preview, in rems.
const SEARCH_LIST_REMS: f32 = 24.;
/// What the search browser's window keeps clear of its body (results and
/// preview), in rems: the dialog's margins and padding (4), the query row
/// and its gap (2.5), room for both error rows (3.5) and slack.
const SEARCH_CHROME_REMS: f32 = 12.;
/// The search browser's body height bounds, in rems: never squashed below
/// a few rows, never taller than reading needs on a big screen.
const SEARCH_MIN_BODY_REMS: f32 = 6.;
const SEARCH_MAX_BODY_REMS: f32 = 44.;
/// The results' share of the body when the preview is under them (narrow
/// window).
const SEARCH_STACKED_LIST_SHARE: f32 = 0.4;

/// The search browser's body height, in rems, in a window whose usable
/// height is `window_rems`: all of it but the dialog's chrome, within
/// [`SEARCH_MIN_BODY_REMS`]..=[`SEARCH_MAX_BODY_REMS`].
pub(crate) fn search_body_rems(window_rems: f32) -> f32 {
    (window_rems - SEARCH_CHROME_REMS).clamp(SEARCH_MIN_BODY_REMS, SEARCH_MAX_BODY_REMS)
}

/// A picker row's element id: its path (or the one `Create:` row), never
/// its position.
pub(crate) fn picker_row_id(row: &FileListEntry) -> SharedString {
    match row {
        FileListEntry::CreateNote { .. } => "note-picker-create".into(),
        other => format!("note-picker-row:{}", other.path()).into(),
    }
}

/// Which door opened the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerKind {
    /// Every note, and `Create:`.
    Finder,
    /// The notes a followed link names.
    Links,
    /// The vault's query search, with a preview (the search browser).
    Search,
    /// The workspace's saved searches (rows: [`SearchItem`](kimun_notes::logic::saved_search::SearchItem)).
    SavedSearches,
}

/// A row the picker lists: its identity, how it is drawn, and what
/// choosing it does.
pub(crate) trait PickerRow: ListRow + Clone + 'static {
    /// The row's element id: from what it stands for, never its position.
    fn element_id(&self) -> SharedString;
    /// The row as the list draws it (the picker adds the pointer handling).
    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div>;
    /// How typing narrows the rows the source loaded: the source's own
    /// order (its load runs the query) unless a row type ranks locally.
    fn filter() -> Filter<Self> {
        Filter::SourceOrder
    }
    /// The note the search browser's preview shows for the row.
    fn preview_path(&self) -> Option<VaultPath> {
        None
    }
    /// The name Delete removes the row by (a saved search; never its
    /// virtual Backlinks entry), if it can go.
    fn deletable_name(&self) -> Option<&str> {
        None
    }
    /// Enter (or a click, by the picker's click policy) on the row, opened
    /// by `kind`; `emphasis`
    /// is the query's needles (search browser). True when the picker must
    /// wait (busy) for what started.
    fn choose(
        &self,
        kind: PickerKind,
        emphasis: Option<Vec<String>>,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> bool;
}

impl PickerRow for FileListEntry {
    fn element_id(&self) -> SharedString {
        picker_row_id(self)
    }

    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        entry_row(self.element_id(), self, selected, true, t)
    }

    fn preview_path(&self) -> Option<VaultPath> {
        match self {
            FileListEntry::Note { path, .. } => Some(path.clone()),
            _ => None,
        }
    }

    fn choose(
        &self,
        kind: PickerKind,
        emphasis: Option<Vec<String>>,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> bool {
        match (kind, picker_open(self)) {
            (PickerKind::Search, PickerOpen::Open(path)) => {
                workspace.search_open(path, emphasis, window, cx)
            }
            (_, open) => workspace.picker_chose(open, window, cx),
        }
    }
}

/// What the search browser adds to the picker: the preview column and what
/// the query's variables resolve against (the open note).
pub(crate) struct SearchParts {
    pub(crate) preview: Entity<SearchPreview>,
    pub(crate) context: QueryContext,
}

/// The picker's view. See the module docs.
pub(crate) struct NotePicker<R: PickerRow = FileListEntry> {
    kind: PickerKind,
    core: ListCore<R>,
    pub(crate) input: Entity<InputState>,
    scroll: UniformListScrollHandle,
    workspace: WeakEntity<Workspace>,
    theme: Theme,
    /// Why the last `Create:` failed; cleared by the next keystroke.
    error: Option<SharedString>,
    /// A `Create:` is running: Enter and Escape wait for it.
    busy: bool,
    /// The search browser's preview and query context.
    search: Option<SearchParts>,
    /// The query's emphasis needles (search browser), made once per query.
    needles: Arc<[String]>,
    _input_events: Subscription,
}

impl<R: PickerRow> NotePicker<R> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        kind: PickerKind,
        source: Arc<dyn RowSource<R>>,
        query: String,
        search: Option<SearchParts>,
        redraw: kimun_notes::logic::message::Notifier,
        workspace: WeakEntity<Workspace>,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = match kind {
            PickerKind::Finder => "Find a note, or name a new one",
            PickerKind::Links => "Filter the matches",
            PickerKind::Search => "Search notes: words, #tag, <note, prop:key=value",
            PickerKind::SavedSearches => "Filter by name or number",
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(query.clone())
        });
        let input_events = cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.set_query(query, cx);
            }
        });
        let needles = Self::needles_of(search.as_ref(), &query);
        let setup = ListSetup {
            query,
            filter: R::filter(),
            ..ListSetup::default()
        };
        Self {
            kind,
            core: ListCore::new(source, redraw, setup),
            input,
            scroll: UniformListScrollHandle::new(),
            workspace,
            theme,
            error: None,
            busy: false,
            search,
            needles,
            _input_events: input_events,
        }
    }

    /// The needles `query` implies, resolved against the open note first so
    /// `{note}` never leaks (search browser only).
    fn needles_of(search: Option<&SearchParts>, query: &str) -> Arc<[String]> {
        match search {
            Some(search) => emphasis_needles(&resolve_query(query, &search.context)).into(),
            None => Arc::from([]),
        }
    }

    /// The preview's entity (tests).
    #[cfg(test)]
    pub(crate) fn preview(&self) -> Option<Entity<SearchPreview>> {
        self.search.as_ref().map(|s| s.preview.clone())
    }

    /// The query's needles (tests).
    #[cfg(test)]
    pub(crate) fn needles(&self) -> Vec<String> {
        self.needles.to_vec()
    }

    /// Point the preview at the selected note (none for `Create:` or no
    /// selection). Cheap when nothing changed.
    fn sync_preview(&self, cx: &mut Context<Self>) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let path = self.core.selected_row().and_then(R::preview_path);
        let needles = self.needles.clone();
        search
            .preview
            .update(cx, |preview, cx| preview.show(path, needles, cx));
    }

    /// The rows shown, top first (tests).
    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<R> {
        self.core.visible_rows().into_iter().cloned().collect()
    }

    /// Whether the rows for the current query have landed (tests).
    #[cfg(test)]
    pub(crate) fn is_loading(&self) -> bool {
        self.core.is_loading()
    }

    /// The query typed (tests).
    #[cfg(test)]
    pub(crate) fn query(&self) -> &str {
        self.core.query()
    }

    /// The row Enter would choose (tests).
    #[cfg(test)]
    pub(crate) fn selected(&self) -> Option<R> {
        self.core.selected_row().cloned()
    }

    /// The error the picker shows (tests).
    #[cfg(test)]
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Apply what the load delivered (on the redraw it sends).
    pub(crate) fn poll(&mut self, cx: &mut Context<Self>) {
        let polled = self.core.poll();
        if polled.changed {
            if polled.restarted {
                // The search browser starts on the best result: Enter opens
                // it; `Create:` is one Up away.
                if self.kind == PickerKind::Search
                    && self.core.leading().is_some()
                    && self.core.match_count() > 0
                    && self.core.selected() == Some(0)
                {
                    self.core.move_selection(Some(1));
                }
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            self.seed_saved_search();
            self.sync_preview(cx);
            cx.notify();
        }
    }

    /// The saved searches' quick select (the TUI modal's ranking, made one
    /// step): a filter puts the best match — the search numbered as typed,
    /// else the first name that holds it — under Enter; an empty filter
    /// the Backlinks entry on top.
    fn seed_saved_search(&mut self) {
        if self.kind != PickerKind::SavedSearches {
            return;
        }
        let leading = usize::from(self.core.leading().is_some());
        let pos = if self.core.query().trim().is_empty() || self.core.match_count() == 0 {
            0
        } else {
            leading
        };
        if self.core.visible_len() > pos {
            self.core.move_selection(Some(pos));
        }
    }

    /// A sort dialog pick (search browser): the query's order directive
    /// becomes `field` / `order` and the rows load again. On the empty
    /// query the recent notes take that order. A property sort with no key
    /// changes nothing.
    pub(crate) fn apply_sort(
        &mut self,
        field: &SortField,
        order: SortOrder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = self.core.query().to_string();
        let Some(sorted) = query_with_sort(&query, field, order) else {
            return;
        };
        if sorted == query {
            return;
        }
        // `set_value` reports no change: the query runs here.
        self.input
            .update(cx, |input, cx| input.set_value(sorted.clone(), window, cx));
        self.set_query(sorted, cx);
    }

    /// The search browser's sort chip: the query's order, or "Unsorted" on
    /// the recent notes.
    fn render_sort_chip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<R> {
        let query = self.core.query();
        let label = if query.trim().is_empty() {
            SharedString::from("Unsorted")
        } else {
            let (field, order) = order_of_query(query);
            chip_label(&field, order)
        };
        sort_chip(BROWSER_SORT_ID, label, &self.theme)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                // After this update: the dialog reads the browser's query.
                let workspace = this.workspace.clone();
                window.defer(cx, move |window, cx| {
                    workspace
                        .update(cx, |workspace, cx| {
                            workspace.open_sort(SortTarget::Browser, window, cx)
                        })
                        .ok();
                });
            }))
            .test_support()
    }

    /// Load the rows again (a saved search was deleted).
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.core.reload();
        cx.notify();
    }

    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.error = None;
        self.needles = Self::needles_of(self.search.as_ref(), &query);
        self.core.set_query(query);
        self.seed_saved_search();
        self.reveal_selection();
        self.sync_preview(cx);
        cx.notify();
    }

    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if down {
            self.core.select_next();
        } else {
            self.core.select_prev();
        }
        self.reveal_selection();
        self.sync_preview(cx);
        cx.notify();
    }

    fn reveal_selection(&self) {
        if let Some(pos) = self.core.selected() {
            self.scroll.scroll_to_item(pos, ScrollStrategy::Nearest);
        }
    }

    /// A `Create:` started (`true`) or failed with `error`.
    pub(crate) fn set_busy(&mut self, busy: bool, error: Option<String>, cx: &mut Context<Self>) {
        self.busy = busy;
        self.error = error.map(Into::into);
        cx.notify();
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.busy
    }

    /// Enter (or a click on a row, by the picker's policy): choose the
    /// selected row.
    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(row) = self.core.selected_row().cloned() else {
            return;
        };
        let emphasis = (!self.needles.is_empty()).then(|| self.needles.to_vec());
        let kind = self.kind;
        let creating = self
            .workspace
            .update(cx, |workspace, cx| {
                row.choose(kind, emphasis, workspace, window, cx)
            })
            .unwrap_or(false);
        if creating {
            self.set_busy(true, None, cx);
        }
    }

    /// What a click on a row does here: the search browser's selection
    /// drives its preview, so a click selects and a double-click opens;
    /// every other picker chooses on a click.
    fn click_policy(&self) -> ClickPolicy {
        match self.kind {
            PickerKind::Search => ClickPolicy::Select,
            PickerKind::Finder | PickerKind::Links | PickerKind::SavedSearches => ClickPolicy::Open,
        }
    }

    /// Delete (the key or the button): ask before the selected row goes.
    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(name) = self
            .core
            .selected_row()
            .and_then(|row| row.deletable_name())
            .map(str::to_string)
        else {
            return;
        };
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.ask_delete_saved_search(name, window, cx)
            })
            .ok();
    }

    /// The saved searches' footer: what Enter does, and `Delete…` for the
    /// selected search (disabled on the Backlinks entry, which is not one).
    fn render_saved_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let deletable = self
            .core
            .selected_row()
            .is_some_and(|row| row.deletable_name().is_some());
        div()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .text_color(theme::muted(&self.theme))
            .child(div().flex_1().min_w_0().child("Enter runs it in FIND"))
            .child(
                Button::new(PICKER_DELETE_ID)
                    .ghost()
                    .xsmall()
                    .label("Delete…")
                    .tooltip(if deletable {
                        "Delete the selected search (Delete)"
                    } else {
                        "The Backlinks entry is built in"
                    })
                    .disabled(!deletable)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.delete_selected(window, cx)
                    })),
            )
            .into_any_element()
    }

    fn render_note(&self, text: &'static str) -> AnyElement {
        div()
            .px_3()
            .py_2()
            .text_sm()
            .text_color(theme::muted(&self.theme))
            .child(text)
            .into_any_element()
    }
}

/// A click on a row selects it (the preview follows), then chooses it as
/// Enter does (which waits while a `Create:` runs), under the picker's
/// policy.
impl<R: PickerRow> ClickRows for NotePicker<R> {
    fn select_row(&mut self, pos: usize, cx: &mut Context<Self>) {
        self.core.select(pos);
        self.sync_preview(cx);
        cx.notify();
    }

    fn activate_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.choose(window, cx);
    }
}

impl<R: PickerRow> Render for NotePicker<R> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let count = self.core.visible_len();
        let selected = self.core.selected();
        let policy = self.click_policy();
        // Nothing can be chosen while a `Create:` runs: no pointing hand.
        let enabled = !self.busy;
        let list = if count > 0 {
            uniform_list(
                PICKER_LIST_ID,
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    // Only the visible rows are built, each from its row.
                    range
                        .filter_map(|pos| {
                            let row = this.core.visible_row(pos)?;
                            let element = with_click_cursor(
                                row.render_row(selected == Some(pos), &this.theme),
                                policy,
                                enabled,
                            )
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                this.click_policy().apply(event, this, pos, window, cx)
                            }))
                            .test_support();
                            Some(element.into_any_element())
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full()
            .into_any_element()
        } else if self.core.is_loading() {
            self.render_note("Loading…")
        } else {
            match self.kind {
                PickerKind::Finder if self.core.query().trim().is_empty() => {
                    self.render_note("No notes yet. Type a name to create one.")
                }
                PickerKind::Search if self.core.query().trim().is_empty() => {
                    self.render_note("No recent notes. Type to search.")
                }
                PickerKind::SavedSearches => self.render_note("No saved search matches"),
                _ => self.render_note("No notes match"),
            }
        };
        let (label, rows_label) = match self.kind {
            PickerKind::Finder => ("Find a note, or name a new one", "Notes"),
            PickerKind::Links => ("Filter the matching notes", "Notes"),
            PickerKind::Search => ("Search notes", "Notes"),
            PickerKind::SavedSearches => ("Filter the saved searches", "Saved searches"),
        };
        let rows = div()
            .id("note-picker-rows")
            .role(Role::ListBox)
            .aria_label(rows_label)
            .child(list);
        let input = Input::new(&self.input)
            .id(PICKER_INPUT_ID)
            .aria_label(label);
        let (input_row, body) = match self.search.as_ref() {
            None => (
                input.into_any_element(),
                rows.h(rems(LIST_REMS)).into_any_element(),
            ),
            Some(search) => {
                let input_row = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(input))
                    .child(self.render_sort_chip(cx))
                    .child(
                        Button::new(HELP_BUTTON_ID)
                            .ghost()
                            .icon(IconName::CircleQuestionMark)
                            .tooltip("Query syntax (F1)")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.workspace
                                    .update(cx, |workspace, cx| {
                                        workspace.open_query_help(window, cx)
                                    })
                                    .ok();
                            })),
                    )
                    .into_any_element();
                // A cached view: rebuilt only when the preview itself changes.
                let preview = search
                    .preview
                    .clone()
                    .cached(StyleRefinement::default().size_full());
                let rem = window.rem_size();
                let narrow = window.viewport_size().width < rem * SEARCH_NARROW_REMS;
                // Most of the window's height (inside any client-side frame),
                // so the dialog grows with the window and fits a small one.
                let frame = window_paddings(window);
                let usable = window.viewport_size().height - frame.top - frame.bottom;
                let height = rem * search_body_rems(usable / rem);
                let pane = div()
                    .min_w_0()
                    .min_h_0()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::divider(t))
                    .overflow_hidden()
                    .child(preview);
                let body = if narrow {
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .h(height)
                        .child(rows.h(height * SEARCH_STACKED_LIST_SHARE).flex_none())
                        .child(pane.flex_1())
                } else {
                    div()
                        .flex()
                        .items_stretch()
                        .gap_2()
                        .h(height)
                        .child(rows.w(rems(SEARCH_LIST_REMS)).flex_none())
                        .child(pane.flex_1())
                };
                (input_row, body.into_any_element())
            }
        };
        let query_error = match self.kind {
            PickerKind::Search => error_reason(self.core.query()),
            _ => None,
        };
        let footer = (self.kind == PickerKind::SavedSearches).then(|| self.render_saved_footer(cx));
        div()
            .id(PICKER_ID)
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            // The input's keys that mean something else here, taken before
            // the input sees them.
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.step(false, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.step(true, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                this.choose(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if !this.busy {
                    this.workspace
                        .update(cx, |workspace, cx| workspace.close_modal(window, cx))
                        .ok();
                }
                cx.stop_propagation();
            }))
            // Delete: the selected saved search (the TUI modal's key; the
            // filter gives up its forward delete there, as in the TUI).
            .when(self.kind == PickerKind::SavedSearches, |picker| {
                picker.capture_action(cx.listener(|this, _: &Delete, window, cx| {
                    this.delete_selected(window, cx);
                    cx.stop_propagation();
                }))
            })
            .child(input_row)
            .child(body)
            .children(footer)
            .when_some(query_error, |picker, reason| {
                let text = query_problem(reason);
                picker.child(
                    div()
                        .id(QUERY_ERROR_ID)
                        .test_support()
                        .role(Role::Alert)
                        .aria_label(text.clone())
                        .text_sm()
                        .text_color(theme::error(t))
                        .child(text),
                )
            })
            .when_some(self.error.clone(), |picker, error| {
                picker.child(
                    div()
                        .id(PICKER_ERROR_ID)
                        .test_support()
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .text_sm()
                        .text_color(theme::error(t))
                        .child(error),
                )
            })
    }
}

/// What the search browser says about a query core cannot read: its
/// reason, as a sentence.
pub(crate) fn query_problem(reason: &str) -> SharedString {
    let mut chars = reason.chars();
    match chars.next() {
        Some(first) => format!("{}{} in the query", first.to_uppercase(), chars.as_str()).into(),
        None => SharedString::default(),
    }
}

// ---- The workspace's side ----------------------------------------------------

impl Workspace {
    /// Whether the picker's `Create:` is running.
    pub(crate) fn picker_busy(&self, cx: &gpui_kit::App) -> bool {
        self.picker
            .as_ref()
            .and_then(|p| p.upgrade())
            .is_some_and(|p| p.read(cx).is_busy())
    }

    /// Open the finder over every note; `Create:` rows resolve in the open
    /// note's folder (the TUI's rule). Refused while a modal is open; says
    /// so without a workspace.
    pub(crate) fn open_finder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if modal::dialog_open(window, cx) {
            return;
        }
        let Some(vault) = self.vault_or_flash(cx) else {
            return;
        };
        let source = Arc::new(FileFinderProvider::new(vault, self.open_note_dir()));
        self.open_note_picker(
            PickerKind::Finder,
            source,
            None,
            None,
            String::new(),
            window,
            cx,
        );
    }

    /// Open the link chooser: every note the link `target` names
    /// (`results`), to pick one. Refused while a modal is open.
    pub(crate) fn open_link_chooser(
        &mut self,
        target: &str,
        results: Vec<(NoteEntryData, NoteContentData)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) || self.notes.is_none() {
            return;
        }
        let title = format!("{} notes match \"{target}\"", results.len());
        let source = Arc::new(LinkResultsProvider::from_results(results));
        self.open_note_picker(
            PickerKind::Links,
            source,
            Some(title),
            None,
            String::new(),
            window,
            cx,
        );
    }

    /// The search browser over `source`, its query `query` (empty: the
    /// recent notes), `open` the note its variables resolve against.
    pub(super) fn open_search_picker(
        &mut self,
        source: Arc<dyn RowSource<FileListEntry>>,
        query: String,
        open: Option<VaultPath>,
        preview: Entity<SearchPreview>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let search = SearchParts {
            preview,
            context: QueryContext::with_note(open),
        };
        self.open_note_picker(
            PickerKind::Search,
            source,
            None,
            Some(search),
            query,
            window,
            cx,
        );
    }

    /// A note picker (finder, link chooser, search browser) in the modal
    /// slot.
    #[allow(clippy::too_many_arguments)]
    fn open_note_picker(
        &mut self,
        kind: PickerKind,
        source: Arc<dyn RowSource<FileListEntry>>,
        title: Option<String>,
        search: Option<SearchParts>,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(picker) = self.open_picker(kind, source, title, search, query, window, cx) {
            self.picker = Some(picker.downgrade());
        }
    }

    /// Open the picker `kind` over `source` in the modal slot, its query
    /// field focused on `query`. The caller keeps the returned view.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn open_picker<R: PickerRow>(
        &mut self,
        kind: PickerKind,
        source: Arc<dyn RowSource<R>>,
        title: Option<String>,
        search: Option<SearchParts>,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<NotePicker<R>>> {
        let tx = self.tx.clone()?;
        let workspace = cx.entity().downgrade();
        let theme = self.theme.clone();
        let picker = cx.new(|cx| {
            NotePicker::new(
                kind,
                source,
                query,
                search,
                redraw_callback(tx),
                workspace,
                theme,
                window,
                cx,
            )
        });
        let width_rems = match kind {
            PickerKind::Search => SEARCH_WIDTH_REMS,
            _ => WIDTH_REMS,
        };
        let modal = match kind {
            PickerKind::Finder => ModalKind::Finder,
            PickerKind::Links => ModalKind::LinkChooser,
            PickerKind::Search => ModalKind::Search,
            PickerKind::SavedSearches => ModalKind::SavedSearches,
        };
        let busy = picker.downgrade();
        let on_cancel = move |_: &ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
            !busy.upgrade().is_some_and(|p| p.read(cx).is_busy())
        };
        let input = picker.read(cx).input.clone();
        let opened = picker.clone();
        self.show_modal(
            Slot::Own(modal),
            width_rems,
            // Focused with the caret after any query it opened on (a
            // followed label), so typing narrows it.
            move |window, cx| {
                input.update(cx, |input, cx| {
                    input.set_cursor_position(Position::new(0, u32::MAX), window, cx)
                })
            },
            move |dialog, _, _| {
                dialog
                    .when_some(title.clone(), |dialog, title| dialog.title(title))
                    .on_cancel(on_cancel.clone())
                    .child(picker.clone())
            },
            window,
            cx,
        );
        Some(opened)
    }

    /// The picker's rows have news (the app channel's redraw).
    pub(super) fn poll_picker(&mut self, cx: &mut Context<Self>) {
        if let Some(picker) = self.picker.as_ref().and_then(|p| p.upgrade()) {
            picker.update(cx, |picker, cx| picker.poll(cx));
        }
        if let Some(picker) = self.saved_searches.as_ref().and_then(|p| p.upgrade()) {
            picker.update(cx, |picker, cx| picker.poll(cx));
        }
    }

    /// A picker row was chosen: open it (the picker closes first, the note
    /// takes the keyboard), or make the `Create:` note through core and open
    /// it once made. True when a create started: the picker waits for it.
    pub(crate) fn picker_chose(
        &mut self,
        open: PickerOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match open {
            PickerOpen::Open(path) => {
                self.close_modal(window, cx);
                self.open_chosen(path, None, window, cx);
                false
            }
            PickerOpen::Create(path) => self.picker_create(path, window, cx),
        }
    }

    /// Open `path`, chosen in a modal that has closed: a note opens in the
    /// editor, which takes the keyboard; anything else is shown in FILES, as
    /// the app channel's `OpenPath` does (the pickers list only notes).
    ///
    /// `emphasis`: the search needles a search result opens with
    /// ([`Self::open_note_with`]); `None` from every other modal.
    pub(crate) fn open_chosen(
        &mut self,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if path.is_note() {
            self.focus_panel(Panel::Editor, window, cx);
            self.open_note_with(path, emphasis, cx);
        } else {
            self.files_show_dir(path, cx);
        }
    }

    /// `Create:` in the finder: core makes the note (or finds it there),
    /// then it opens; a failure stays in the picker. Called from the picker
    /// (which marks itself busy when this returns true), so it never
    /// updates the picker itself before the create lands.
    fn picker_create(
        &mut self,
        path: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let (Some(picker), Some(vault)) =
            (self.picker.as_ref().and_then(|p| p.upgrade()), self.vault())
        else {
            return false;
        };
        if self.note_task.is_some() {
            return false;
        }
        let made = tokio::spawn(async move { file_ops::create_note(&vault, &path, None).await });
        self.note_task = Some(cx.spawn_in(window, async move |this, cx| {
            let made = tasks::joined(made).await;
            this.update_in(cx, |workspace, window, cx| {
                workspace.note_task = None;
                match made {
                    Ok(outcome) => {
                        workspace.close_modal(window, cx);
                        workspace.focus_panel(Panel::Editor, window, cx);
                        if let Some(tx) = workspace.tx.as_ref() {
                            outcome.announce_and_open(tx);
                        }
                    }
                    Err(error) => {
                        picker.update(cx, |picker, cx| picker.set_busy(false, Some(error), cx));
                    }
                }
            })
            .ok();
        }));
        true
    }
}
