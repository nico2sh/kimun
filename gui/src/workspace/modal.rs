//! The window's **modal host**: the TUI OverlayHost's rules on top of the
//! kit's dialog layer (`WindowExt::open_dialog` on `Root`), which already
//! owns the stack, Escape, the backdrop click, the focus trap and giving the
//! keyboard back to whatever had it on close.
//!
//! What this adds is the single slot and what the window does around it:
//!
//! - **One modal at a time.** Opening while one is open is refused; the
//!   only exception is the modal's own chord, which toggles it closed (the
//!   TUI's palette toggle).
//! - **Close before running.** A command chosen in a modal runs with the
//!   modal already gone, so it sees the window's own focus and layout.
//! - **Nothing steals the keyboard from it.** [`Workspace::apply_focus`]
//!   does nothing while a dialog is open: async results (a note opening, a
//!   listing landing) reach the layout, and the kit restores the opener's
//!   focus when the modal closes.
//! - **File operations close it.** A note or folder created, renamed, moved
//!   or deleted under an open modal leaves it showing stale data.
//! - **App-channel messages get a window.** The channel's dispatch has no
//!   `Window`; [`Workspace::with_window`] runs a step with the one captured
//!   when the workspace was made.
//!
//! The *open* modal is the kit's to say (`has_active_dialog`): a modal the
//! kit closed on its own (Escape, a backdrop click) is closed here too.

use gpui_kit::component::WindowExt as _;
use gpui_kit::{Context, KeystrokeEvent, Window};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::message::FileOp;

use super::Workspace;
use crate::input;
use crate::keymap;

/// Whether a kit dialog holds the keyboard. `false` while the window is
/// still being built (no `Root` yet), where the kit's own query would panic.
pub(crate) fn dialog_open(window: &mut Window, cx: &mut gpui_kit::App) -> bool {
    window.root::<gpui_kit::base::Root>().flatten().is_some() && window.has_active_dialog(cx)
}

/// Which modal the window's one slot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModalKind {
    /// The command palette.
    Palette,
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

    /// Close the open modal; the kit gives the keyboard back to its opener.
    pub(crate) fn close_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if dialog_open(window, cx) {
            window.close_dialog(cx);
        }
        self.modal = None;
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
                workspace.modal = None;
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
        let action = keymap::active(cx).resolve(key, self.key_ctx(cx)).cloned();
        match (open, action) {
            (ModalKind::Palette, Some(ActionShortcuts::OpenCommandPalette)) => {
                self.close_modal(window, cx);
                true
            }
            _ => false,
        }
    }
}
