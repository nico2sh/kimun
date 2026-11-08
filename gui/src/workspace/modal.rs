//! The window's **modal host**: the TUI OverlayHost's rules on top of the
//! kit's dialog layer (`WindowExt::open_dialog` on `Root`), which already
//! owns the stack, Escape, the backdrop click, the focus trap and giving the
//! keyboard back to whatever had it on close.
//!
//! What this adds is the single slot and what the window does around it:
//!
//! - **One modal at a time.** Opening while one is open is refused; the
//!   only exception is the modal's own chord, which toggles it closed (the
//!   TUI's palette toggle; the finder's and the quick note's too, unless
//!   they are making a note).
//! - **Close before running.** A command chosen in a modal runs with the
//!   modal already gone, so it sees the window's own focus and layout.
//! - **Nothing steals the keyboard from it.** [`Workspace::apply_focus`]
//!   does nothing while a dialog is open: async results (a note opening, a
//!   listing landing) reach the layout, and the kit restores the opener's
//!   focus when the modal closes.
//! - **File operations close it.** A note or folder created, renamed, moved
//!   or deleted under an open modal leaves it showing stale data. A property
//!   write under way still finishes, and the open note still reloads.
//! - **Two stacked dialogs.** The properties dialog's and the saved
//!   searches' deletes ask in a kit `AlertDialog` over them, and the search
//!   browser shows the query syntax or its sort dialog over itself; the slot
//!   stays the one underneath, and closing it closes both. Asked for from
//!   FIND, the query syntax is the one modal. Saving the search browser's
//!   query is not a stacking case: the browser gives way to the save dialog
//!   and comes back on its query when that closes.
//! - **App-channel messages get a window.** The channel's dispatch has no
//!   `Window`; [`Workspace::with_window`] runs a step with the one captured
//!   when the workspace was made.
//!
//! The *open* modal is the kit's to say (`has_active_dialog`): a modal the
//! kit closed on its own (Escape, a backdrop click) is closed here too.

use std::cell::Cell;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::{App, Context, Entity, Focusable, KeystrokeEvent, Window};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::message::FileOp;

use super::Workspace;
use crate::input;

use crate::keymap;

/// The row of buttons under a dialog's body, right-aligned: the one shape
/// every dialog's footer shares (add `.flex_wrap()` where it must wrap).
pub(crate) fn footer_row() -> gpui_kit::Div {
    use gpui_kit::{Styled as _, div};
    div().flex().flex_row().justify_end().gap_2().pt_2()
}

/// Whether a kit dialog holds the keyboard. `false` while the window is
/// still being built (no `Root` yet), where the kit's own query would panic.
pub(crate) fn dialog_open(window: &mut Window, cx: &mut gpui_kit::App) -> bool {
    window.root::<gpui_kit::base::Root>().flatten().is_some() && window.has_active_dialog(cx)
}

/// A [`Workspace::show_modal`] `focus`: the keyboard to `view`'s own focus
/// handle (a dialog whose keys are its own, not a field's).
pub(crate) fn focus_view<V: Focusable>(view: Entity<V>) -> impl FnOnce(&mut Window, &mut App) {
    move |window, cx| {
        let handle = view.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }
}

/// Which modal the window's one slot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModalKind {
    /// The command palette.
    Palette,
    /// The note menu (rename, move, delete).
    FileOps,
    /// The rename dialog.
    Rename,
    /// The move dialog.
    Move,
    /// The delete confirmation.
    Delete,
    /// The note finder (`OpenNote`, leader `f f`).
    Finder,
    /// The link chooser: a followed link several notes answer to.
    LinkChooser,
    /// The create dialog: a note asked for is not there.
    CreateNote,
    /// The quick note.
    QuickNote,
    /// The open note's properties (with its delete confirmation stacked on
    /// it while one asks).
    Properties,
    /// The search browser (`SearchNotes`, leader `f g` / `f r`), with its
    /// query syntax stacked on it while shown.
    Search,
    /// The query syntax on its own (FIND's F1 and `Syntax` button).
    QueryHelp,
    /// The saved searches (`OpenSavedSearches`, leader `f s`), with its
    /// delete confirmation stacked on it while one asks.
    SavedSearches,
    /// The save-search dialog (`SaveCurrentQuery` in FIND or the search
    /// browser, FIND's `Save…`).
    SaveSearch,
    /// The sort dialog over FILES or FIND (`OpenSortDialog`, a sort chip).
    /// Over the search browser it is stacked and the slot stays `Search`.
    Sort,
    /// The pinned notes (leader `f p`, the palette).
    PinnedNotes,
    /// The workspace switcher (`SwitchWorkspace`, leader `v s`, the
    /// palette).
    WorkspaceSwitcher,
    /// The theme picker (leader `v t`, the palette, the CFG view).
    ThemePicker,
    /// The update dialog (the update toast's `View`, the palette).
    Update,
    /// The keyboard reference (F1, leader `?`, the palette, the CFG view).
    KeyboardHelp,
    /// The split chooser: which view a split's new slot holds.
    ViewChooser,
}

/// Where [`Workspace::show_modal`] puts a dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// In the window's one slot, as this modal.
    Own(ModalKind),
    /// Over the open modal (the browser's query syntax or sort), which
    /// keeps the slot.
    Stacked,
}

impl ModalKind {
    /// The chord that toggles this modal: it opens it when nothing is open
    /// and closes it when it is the open modal ([`Workspace::toggle_modal`]).
    /// `None` for a modal without one.
    pub(crate) fn chord(self) -> Option<ActionShortcuts> {
        match self {
            Self::Palette => Some(ActionShortcuts::OpenCommandPalette),
            Self::Finder => Some(ActionShortcuts::OpenNote),
            Self::QuickNote => Some(ActionShortcuts::QuickNote),
            Self::Search => Some(ActionShortcuts::SearchNotes),
            Self::SavedSearches => Some(ActionShortcuts::OpenSavedSearches),
            Self::WorkspaceSwitcher => Some(ActionShortcuts::SwitchWorkspace),
            Self::KeyboardHelp => Some(ActionShortcuts::KeyboardHelp),
            _ => None,
        }
    }

    /// Whether another dialog can sit over this one in the kit's stack (the
    /// browser's query syntax or sort; a delete confirmation over the saved
    /// searches or the properties). Closing such a modal closes the whole
    /// stack; any other modal is alone in it.
    pub(crate) fn stacks(self) -> bool {
        matches!(self, Self::Search | Self::SavedSearches | Self::Properties)
    }
}

impl Workspace {
    /// The modal that is open now, if any.
    pub(crate) fn open_modal(
        &self,
        window: &mut Window,
        cx: &mut gpui_kit::App,
    ) -> Option<ModalKind> {
        if dialog_open(window, cx) {
            self.modal
        } else {
            None
        }
    }

    /// Open a kit dialog in `slot`, `width_rems` wide and without the kit's
    /// close button (`build` may put it back). A modal takes every key, so a pending leader
    /// sequence ends. `focus` runs on the dialog's first build only (the
    /// builder re-runs whenever the dialog layer rebuilds); `build` adds the
    /// rest — the title, `on_ok` / `on_cancel` / `on_close`, the content.
    /// The caller checks first that nothing else is open and records its
    /// own view.
    pub(crate) fn show_modal(
        &mut self,
        slot: Slot,
        width_rems: f32,
        focus: impl FnOnce(&mut Window, &mut App) + 'static,
        build: impl Fn(Dialog, &mut Window, &mut App) -> Dialog + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_leader(cx);
        if let Slot::Own(kind) = slot {
            self.modal = Some(kind);
        }
        let width = window.rem_size() * width_rems;
        let focus = Cell::new(Some(focus));
        window.open_dialog(cx, move |dialog, window, cx| {
            if let Some(focus) = focus.take() {
                focus(window, cx);
            }
            build(dialog.w(width).close_button(false), window, cx)
        });
        cx.notify();
    }

    /// `kind`'s [chord](ModalKind::chord): close `kind` when it is the open
    /// modal, open it with `open` when nothing is open, and do nothing under
    /// another modal.
    pub(crate) fn toggle_modal(
        &mut self,
        kind: ModalKind,
        open: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.open_modal(window, cx) {
            Some(shown) if shown == kind => self.close_by_chord(kind, window, cx),
            None if !dialog_open(window, cx) => open(self, window, cx),
            _ => {}
        }
    }

    /// The open modal `kind`'s chord again: close it — unless it is making
    /// a note (the finder's or the browser's `Create:`, the quick note's
    /// save), which it waits for.
    fn close_by_chord(&mut self, kind: ModalKind, window: &mut Window, cx: &mut Context<Self>) {
        let making_a_note = match kind {
            ModalKind::Finder | ModalKind::Search => self.picker_busy(cx),
            ModalKind::QuickNote => self.note_task.is_some(),
            _ => false,
        };
        if !making_a_note {
            self.close_modal(window, cx);
        }
    }

    /// Close the open modal; the kit gives the keyboard back to its opener.
    /// A modal that [stacks](ModalKind::stacks) closes with whatever is over
    /// it.
    pub(crate) fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if dialog_open(window, cx) {
            if self.modal.is_some_and(ModalKind::stacks) {
                window.close_all_dialogs(cx);
            } else {
                window.close_dialog(cx);
            }
        }
        self.reset_modal_state(cx);
    }

    /// Forget every modal's state: the slot, what is stacked over it, and
    /// each modal's `…Open` handle (a read still under way for one is
    /// dropped when it lands). The one list of them — every way a modal
    /// goes ([`Self::close_modal`], [`Self::file_op_closes_modals`], a file
    /// dialog's operation finishing) clears it here; each caller closes the
    /// kit's dialogs itself. Kept: the file and note operations' progress
    /// (`file_status`, the running tasks) — they belong to the operation,
    /// which outlives its dialog and clears them when it ends. The note
    /// dialogs' `note_status` does go: a note task landing after its dialog
    /// was closed this way flashes its failure instead, and closes nothing
    /// (`create.rs`, `is_note_dialog_up`). The theme picker's preview goes
    /// with it: the window takes back the theme it had when the picker
    /// opened.
    pub(crate) fn reset_modal_state(&mut self, cx: &mut Context<Self>) {
        self.modal = None;
        self.query_help = false;
        self.palette = None;
        self.split_chooser = None;
        self.side_chooser = None;
        self.move_chooser = None;
        #[cfg(test)]
        {
            self.file_dialog = None;
        }
        self.picker = None;
        self.quick_note = None;
        self.note_status = None;
        // A property write in flight still lands: its reload is the window's
        // own message, not the dialog's.
        self.properties = None;
        self.saved_searches = None;
        self.save_search = None;
        self.sort = None;
        self.forget_pinned_notes();
        self.switcher = None;
        self.forget_theme_picker(cx);
        self.forget_update_dialog();
    }

    /// Run `step` with the window, once the current update is done. For
    /// app-channel messages, whose dispatch has no `Window`. Dropped when the
    /// window is gone.
    pub(crate) fn with_window(
        &self,
        cx: &mut Context<Self>,
        step: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(handle) = self.window else {
            return;
        };
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            handle
                .update(cx, |_, window, cx| {
                    this.update(cx, |workspace, cx| step(workspace, window, cx))
                        .ok();
                })
                .ok();
        });
    }

    /// A file operation landed: whatever a modal showed may be stale, so
    /// every modal closes (the TUI's `dismiss_all_overlays`). Requests to
    /// *show* a dialog are not results and leave it alone.
    pub(crate) fn file_op_closes_modals(&mut self, op: &FileOp, cx: &mut Context<Self>) {
        let landed = matches!(
            op,
            FileOp::Created(_) | FileOp::Deleted(_) | FileOp::Renamed { .. } | FileOp::Moved { .. }
        );
        if landed {
            self.with_window(cx, |workspace, window, cx| {
                if dialog_open(window, cx) {
                    window.close_all_dialogs(cx);
                }
                workspace.reset_modal_state(cx);
            });
        }
    }

    /// A key while a modal is open, seen before the modal's own bindings:
    /// the open modal's chord closes it (the palette's toggle). Every other
    /// key is the modal's. True when the key was taken.
    pub(crate) fn intercept_modal_key(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(open) = self.open_modal(window, cx) else {
            return false;
        };
        let Some(key) = input::key_input(&event.keystroke) else {
            return false;
        };
        let keymap = keymap::active(cx);
        // The sort chord closes the sort dialog, whatever opened it; over the
        // browser every other key is the dialog's.
        if self.sort.is_some() && keymap.is_bound(key, &ActionShortcuts::OpenSortDialog) {
            self.dismiss_sort(window, cx);
            return true;
        }
        if self.sort_stacked() {
            return false;
        }
        // The browser's sort: stacked over it, whatever panel opened it
        // (the chord is scoped to the slots otherwise).
        if open == ModalKind::Search
            && !self.query_help
            && keymap.is_bound(key, &ActionShortcuts::OpenSortDialog)
        {
            self.open_sort(kimun_notes::logic::message::SortTarget::Browser, window, cx);
            return true;
        }
        // The browser's query is saved from the browser whatever panel
        // opened it (the chord is scoped to the slots otherwise).
        if open == ModalKind::Search && keymap.is_bound(key, &ActionShortcuts::SaveCurrentQuery) {
            self.save_browser_query(window, cx);
            return true;
        }
        // Under a modal only the modal says whether a query is typed: the
        // search browser does (FIND behind it no longer has the keyboard).
        let ctx = keymap::KeyCtx {
            query: open == ModalKind::Search,
            browser: open == ModalKind::Search,
            ..self.key_ctx(cx)
        };
        let action = keymap.resolve(key, ctx).cloned();
        match (open, action) {
            // The open modal's own chord closes it.
            (open, Some(action)) if open.chord().as_ref() == Some(&action) => {
                self.close_by_chord(open, window, cx);
                true
            }
            // The browser's query in the FIND view (`Open in FIND`).
            (ModalKind::Search, Some(ActionShortcuts::OpenQueryInSidebar)) => {
                self.promote_browser_query(window, cx);
                true
            }
            // The browser's query syntax (F1 by default, the keymap's
            // `Query` scope), shown over it once.
            (ModalKind::Search, Some(ActionShortcuts::QuerySyntax)) => {
                self.open_query_help(window, cx);
                true
            }
            _ => false,
        }
    }
}
