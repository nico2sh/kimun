//! The **note picker** (M6 G4): one component for choosing a note, in the
//! window's modal slot. Two doors open it:
//!
//! - the **finder** (`OpenNote` chord, leader `f f`): every note, most
//!   recently changed first, fuzzy-ranked as you type, with a `Create:` row
//!   for whatever was typed (`FileFinderProvider`);
//! - the **link chooser**: a followed link that several notes answer to
//!   lists every one of them, narrowed by what you type
//!   (`LinkResultsProvider`).
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

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, UniformListScrollHandle,
    WeakEntity, Window, div, rems, uniform_list,
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
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::files::entry_row;
use super::modal::{self, ModalKind};
use super::route::NO_WORKSPACE;
use crate::theme;

/// Element ids, for tests.
pub(crate) const PICKER_ID: &str = "note-picker";
pub(crate) const PICKER_INPUT_ID: &str = "note-picker-input";
pub(crate) const PICKER_LIST_ID: &str = "note-picker-list";
pub(crate) const PICKER_ERROR_ID: &str = "note-picker-error";

/// The picker's width, in rems.
const WIDTH_REMS: f32 = 36.;
/// The list's height, in rems: about seven rows.
const LIST_REMS: f32 = 20.;

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
    _input_events: Subscription,
}

impl NotePicker {
    fn new(
        kind: PickerKind,
        source: Arc<dyn RowSource<FileListEntry>>,
        redraw: kimun_notes::logic::message::Notifier,
        workspace: WeakEntity<Workspace>,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = match kind {
            PickerKind::Finder => "Find a note, or name a new one",
            PickerKind::Links => "Filter the matches",
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let input_events = cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.set_query(query, cx);
            }
        });
        Self {
            kind,
            core: ListCore::new(source, redraw, ListSetup::default()),
            input,
            scroll: UniformListScrollHandle::new(),
            workspace,
            theme,
            error: None,
            busy: false,
            _input_events: input_events,
        }
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
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            cx.notify();
        }
    }

    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.error = None;
        self.core.set_query(query);
        self.reveal_selection();
        cx.notify();
    }

    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if down {
            self.core.select_next();
        } else {
            self.core.select_prev();
        }
        self.reveal_selection();
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
        let creating = self
            .workspace
            .update(cx, |workspace, cx| workspace.picker_chose(open, window, cx))
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                _ => self.render_note("No notes match"),
            }
        };
        let label = match self.kind {
            PickerKind::Finder => "Find a note, or name a new one",
            PickerKind::Links => "Filter the matching notes",
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
            .child(
                Input::new(&self.input)
                    .id(PICKER_INPUT_ID)
                    .aria_label(label),
            )
            .child(
                div()
                    .id("note-picker-rows")
                    .role(Role::ListBox)
                    .aria_label("Notes")
                    .h(rems(LIST_REMS))
                    .child(list),
            )
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
        self.open_picker(PickerKind::Finder, source, None, window, cx);
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
        self.open_picker(PickerKind::Links, source, Some(title), window, cx);
    }

    fn open_picker(
        &mut self,
        kind: PickerKind,
        source: Arc<dyn RowSource<FileListEntry>>,
        title: Option<String>,
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
                redraw_callback(tx),
                workspace,
                theme,
                window,
                cx,
            )
        });
        let width = window.rem_size() * WIDTH_REMS;
        let first = Cell::new(true);
        self.modal = Some(match kind {
            PickerKind::Finder => ModalKind::Finder,
            PickerKind::Links => ModalKind::LinkChooser,
        });
        self.picker = Some(picker.downgrade());
        let busy = picker.downgrade();
        let on_cancel = move |_: &ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
            !busy.upgrade().is_some_and(|p| p.read(cx).is_busy())
        };
        window.open_dialog(cx, move |dialog, window, cx| {
            if first.replace(false) {
                let input = picker.read(cx).input.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
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
