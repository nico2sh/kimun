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
//!   ([`super::search`], M7b G4).
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
//! **Choosing** is the logic's rule (`picker_open`): a note opens through the
//! note session (the open note is saved first), and `Create:` makes the note
//! through core before opening it. A create that fails keeps the picker open
//! and says why; the next keystroke clears it.

use std::cell::Cell;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp, Position,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, StyleRefinement, Styled as _, Subscription,
    UniformListScrollHandle, WeakEntity, Window, div, rems, uniform_list,
};
use kimun_core::nfs::{NoteEntryData, VaultPath};
use kimun_core::note::NoteContentData;
use kimun_notes::logic::file_ops;
use kimun_notes::logic::files::FileListEntry;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::list::{ListCore, ListSetup, RowSource};
use kimun_notes::logic::message::redraw_callback;
use kimun_notes::logic::pickers::{
    FileFinderProvider, LinkResultsProvider, PickerOpen, picker_open,
};
use kimun_notes::logic::query::{QueryContext, emphasis_needles, error_reason, resolve_query};
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::files::entry_row;
use super::modal::{self, ModalKind};
use super::route::NO_WORKSPACE;
use super::search::SearchPreview;
use crate::theme;

/// Element ids, for tests.
pub(crate) const PICKER_ID: &str = "note-picker";
pub(crate) const PICKER_INPUT_ID: &str = "note-picker-input";
pub(crate) const PICKER_LIST_ID: &str = "note-picker-list";
pub(crate) const PICKER_ERROR_ID: &str = "note-picker-error";
pub(crate) const QUERY_ERROR_ID: &str = "note-picker-query-error";
pub(crate) const HELP_BUTTON_ID: &str = "note-picker-help";

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
/// The preview's height under the results (narrow window), in rems.
const SEARCH_PREVIEW_REMS: f32 = 14.;
/// The results' height over the preview (narrow window), in rems.
const SEARCH_STACKED_LIST_REMS: f32 = 12.;

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
}

/// What the search browser adds to the picker: the preview column and what
/// the query's variables resolve against (the open note).
pub(crate) struct SearchParts {
    pub(crate) preview: Entity<SearchPreview>,
    pub(crate) context: QueryContext,
}

/// The picker's view. See the module docs.
pub(crate) struct NotePicker {
    kind: PickerKind,
    core: ListCore<FileListEntry>,
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

impl NotePicker {
    #[allow(clippy::too_many_arguments)]
    fn new(
        kind: PickerKind,
        source: Arc<dyn RowSource<FileListEntry>>,
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
        let path = match self.core.selected_row() {
            Some(FileListEntry::Note { path, .. }) => Some(path.clone()),
            _ => None,
        };
        let needles = self.needles.clone();
        search
            .preview
            .update(cx, |preview, cx| preview.show(path, needles, cx));
    }

    /// The rows shown, top first (tests).
    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<FileListEntry> {
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
    pub(crate) fn selected(&self) -> Option<FileListEntry> {
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
            self.sync_preview(cx);
            cx.notify();
        }
    }

    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.error = None;
        self.needles = Self::needles_of(self.search.as_ref(), &query);
        self.core.set_query(query);
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

    /// Enter (or a double-click): choose the selected row.
    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(open) = self.core.selected_row().map(picker_open) else {
            return;
        };
        let emphasis = (!self.needles.is_empty()).then(|| self.needles.to_vec());
        let kind = self.kind;
        let creating = self
            .workspace
            .update(cx, |workspace, cx| match (kind, open) {
                (PickerKind::Search, PickerOpen::Open(path)) => {
                    workspace.search_open(path, emphasis, window, cx)
                }
                (_, open) => workspace.picker_chose(open, window, cx),
            })
            .unwrap_or(false);
        if creating {
            self.set_busy(true, None, cx);
        }
    }

    fn click_row(
        &mut self,
        pos: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.click_count() >= 2 && self.core.selected() == Some(pos) {
            self.choose(window, cx);
        } else {
            self.core.select(pos);
            self.sync_preview(cx);
            cx.notify();
        }
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

impl Render for NotePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let count = self.core.visible_len();
        let selected = self.core.selected();
        let list = if count > 0 {
            uniform_list(
                PICKER_LIST_ID,
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    // Only the visible rows are built, each from its row.
                    range
                        .filter_map(|pos| {
                            let row = this.core.visible_row(pos)?;
                            let id = picker_row_id(row);
                            let element = entry_row(id, row, selected == Some(pos), &this.theme)
                                .on_click(cx.listener(
                                    move |this, event: &ClickEvent, window, cx| {
                                        this.click_row(pos, event, window, cx)
                                    },
                                ))
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
                _ => self.render_note("No notes match"),
            }
        };
        let label = match self.kind {
            PickerKind::Finder => "Find a note, or name a new one",
            PickerKind::Links => "Filter the matching notes",
            PickerKind::Search => "Search notes",
        };
        let rows = div()
            .id("note-picker-rows")
            .role(Role::ListBox)
            .aria_label("Notes")
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
                let narrow = window.viewport_size().width < window.rem_size() * SEARCH_NARROW_REMS;
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
                        .child(rows.h(rems(SEARCH_STACKED_LIST_REMS)))
                        .child(pane.h(rems(SEARCH_PREVIEW_REMS)))
                } else {
                    div()
                        .flex()
                        .items_stretch()
                        .gap_2()
                        .h(rems(LIST_REMS))
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
            .child(input_row)
            .child(body)
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
    /// The `OpenNote` chord: open the finder, or close it when it is the
    /// open modal. Refused while another modal is open.
    pub(crate) fn toggle_finder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.open_modal(window, cx) {
            Some(ModalKind::Finder) => {
                if !self.picker_busy(cx) {
                    self.close_modal(window, cx);
                }
            }
            None if !modal::dialog_open(window, cx) => self.open_finder(window, cx),
            _ => {}
        }
    }

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
        let Some(vault) = self.notes.as_ref().map(|h| h.session.vault().clone()) else {
            self.flash(NO_WORKSPACE, cx);
            return;
        };
        let source = Arc::new(FileFinderProvider::new(vault, self.open_note_dir()));
        self.open_picker(
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
        self.open_picker(
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
        self.open_picker(
            PickerKind::Search,
            source,
            None,
            Some(search),
            query,
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn open_picker(
        &mut self,
        kind: PickerKind,
        source: Arc<dyn RowSource<FileListEntry>>,
        title: Option<String>,
        search: Option<SearchParts>,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tx) = self.tx.clone() else {
            return;
        };
        // A modal takes every key: no sequence survives under it.
        self.cancel_leader(cx);
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
        let width = window.rem_size()
            * match kind {
                PickerKind::Search => SEARCH_WIDTH_REMS,
                _ => WIDTH_REMS,
            };
        let first = Cell::new(true);
        self.modal = Some(match kind {
            PickerKind::Finder => ModalKind::Finder,
            PickerKind::Links => ModalKind::LinkChooser,
            PickerKind::Search => ModalKind::Search,
        });
        self.picker = Some(picker.downgrade());
        let busy = picker.downgrade();
        let on_cancel = move |_: &ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
            !busy.upgrade().is_some_and(|p| p.read(cx).is_busy())
        };
        window.open_dialog(cx, move |dialog, window, cx| {
            if first.replace(false) {
                // Focused with the caret after any query it opened on (a
                // followed label), so typing narrows it.
                let input = picker.read(cx).input.clone();
                input.update(cx, |input, cx| {
                    input.set_cursor_position(Position::new(0, u32::MAX), window, cx)
                });
            }
            dialog
                .close_button(false)
                .w(width)
                .when_some(title.clone(), |dialog, title| dialog.title(title))
                .on_cancel(on_cancel.clone())
                .child(picker.clone())
        });
        cx.notify();
    }

    /// The picker's rows have news (the app channel's redraw).
    pub(super) fn poll_picker(&mut self, cx: &mut Context<Self>) {
        if let Some(picker) = self.picker.as_ref().and_then(|p| p.upgrade()) {
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
                self.open_chosen(path, window, cx);
                false
            }
            PickerOpen::Create(path) => self.picker_create(path, window, cx),
        }
    }

    /// Open `path`, chosen in a modal that has closed: a note opens in the
    /// editor, which takes the keyboard; anything else is shown in FILES, as
    /// the app channel's `OpenPath` does (the pickers list only notes).
    pub(crate) fn open_chosen(
        &mut self,
        path: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if path.is_note() {
            self.focus_panel(Panel::Editor, window, cx);
            self.open_note(path, cx);
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
        let (Some(picker), Some(vault)) = (
            self.picker.as_ref().and_then(|p| p.upgrade()),
            self.notes.as_ref().map(|h| h.session.vault().clone()),
        ) else {
            return false;
        };
        if self.note_task.is_some() {
            return false;
        }
        let made = tokio::spawn(async move { file_ops::create_note(&vault, &path, None).await });
        self.note_task = Some(cx.spawn_in(window, async move |this, cx| {
            let made = made.await.unwrap_or_else(|e| Err(e.to_string()));
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
