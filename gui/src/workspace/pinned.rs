//! The **Pinned notes dialog** (M7b G8): the vault's pinned notes by number,
//! and where they are managed (leader `f p`, the palette's
//! `+find · pinned notes`). A kit `Dialog` with no query input — nine rows
//! never need filtering — and a fixed height of [`PINNED_NOTES_CAP`] rows,
//! so it never resizes as pins come and go.
//!
//! - **Opening a pin.** A digit 1–9 opens that row at once; Up/Down (and
//!   `j`/`k`) move the selection and Enter opens it; a click opens a row
//!   (the pointing hand over it). The dialog closes first, then
//!   the note opens the way every other open does (the open note is saved
//!   before the switch). A pin whose note is missing on disk is drawn dimmed
//!   with "(missing)" and refuses to open with a flash; the dialog stays.
//!   A click on it only selects it (no hand: it cannot open).
//! - **Managing pins.** Alt+Up / Alt+Down (and `K` / `J`, the TUI's keys)
//!   raise or lower the selected pin; Delete (and `d`) unpins it. Every
//!   change is written as it is made — there is no commit or cancel: the
//!   rows change at once, core writes on tokio, then the list is read again
//!   (always, so a failed write falls back to what is on disk). One
//!   write-and-reload at a time: an edit key pressed while one runs is
//!   ignored. A chorded key that is not one of these is swallowed, so a
//!   chord meant for something else never unpins.
//! - **What it shows** is disk on open and after each of its own edits; a
//!   pin made elsewhere while it is open (leader `m i`) is not pushed in. A
//!   file operation landing closes it, as it closes every modal.
//!
//! **Where every read and write runs.** The list (with each pin's
//! existence check), the reorder and the unpin are core calls through
//! `logic::pinned`, in `tokio::spawn`; the UI thread only applies their
//! results, and only while the dialog is open and the result is the latest
//! it asked for (a generation). Nothing here touches a file.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, Role, ScrollEvent, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, rems,
};
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, PINNED_NOTES_CAP};
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::list::{ListCore, ListSetup, StaticRowSource};
use kimun_notes::logic::message::PinnedRow;
use kimun_notes::logic::pinned::{self, Outcome};

use super::Workspace;
use super::files::{ClickPolicy, ClickRows, NOTE_ROW_REMS, ellipsis, lane_row, with_click_cursor};
use super::modal::{self, ModalKind, Slot};
use crate::input;
use crate::tasks;
use crate::theme;

/// Element ids, for tests.
pub(crate) const PINNED_ID: &str = "pinned-notes";
pub(crate) const PINNED_EMPTY_ID: &str = "pinned-notes-empty";
pub(crate) const PINNED_OPEN_ID: &str = "pinned-open";
pub(crate) const PINNED_RAISE_ID: &str = "pinned-raise";
pub(crate) const PINNED_LOWER_ID: &str = "pinned-lower";
pub(crate) const PINNED_UNPIN_ID: &str = "pinned-unpin";
pub(crate) const PINNED_CLOSE_ID: &str = "pinned-close";

/// What the dialog says with no pins (once the first read has landed).
pub(crate) const EMPTY_TEXT: &str = "No pinned notes yet — leader m i pins the open note";

/// A pin's row id, by its note.
pub(crate) fn pinned_row_id(path: &VaultPath) -> SharedString {
    format!("pinned-row:{path}").into()
}

/// The dialog's width, in rems.
const DIALOG_REMS: f32 = 32.;
/// How far a precise (trackpad) scroll goes before the selection moves one
/// row, in rems.
pub(crate) const WHEEL_STEP_REMS: f32 = 1.5;

/// The dialog while it is open, as the workspace holds it.
pub(crate) struct PinnedOpen {
    /// The view (tests read it).
    #[cfg(test)]
    pub(crate) dialog: WeakEntity<PinnedNotesDialog>,
    /// Set when the modal closes: a read landing after it is dropped. A
    /// flag, not a call into the dialog, so closing from inside the
    /// dialog's own handler never re-enters it.
    closed: Rc<Cell<bool>>,
}

impl PinnedOpen {
    fn close(&self) {
        self.closed.set(true);
    }
}

/// The pinned-notes dialog's view.
pub(crate) struct PinnedNotesDialog {
    vault: Arc<NoteVault>,
    /// The pins and the selection: the shared list engine, its rows put in
    /// by this dialog's own reads and edits (no loader of its own).
    list: ListCore<PinnedRow>,
    /// `false` until the first read lands: before it the body is blank, not
    /// "no pinned notes".
    loaded: bool,
    /// A write started here, and the read after it, are under way. Cleared
    /// only when that read lands (success or failure).
    pending: bool,
    /// The latest read asked for; an older one landing is dropped.
    generation: u64,
    closed: Rc<Cell<bool>>,
    /// Wheel travel not yet worth a row.
    wheel: input::ScrollAccumulator,
    focus: FocusHandle,
    workspace: WeakEntity<Workspace>,
}

impl PinnedNotesDialog {
    fn new(
        vault: Arc<NoteVault>,
        closed: Rc<Cell<bool>>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            vault,
            list: ListCore::with_rows(
                Arc::new(StaticRowSource),
                Arc::new(|| {}),
                ListSetup::default(),
                Vec::new(),
            ),
            loaded: false,
            pending: false,
            generation: 0,
            closed,
            wheel: input::ScrollAccumulator::default(),
            focus: cx.focus_handle(),
            workspace,
        }
    }

    /// The rows shown, in pin order.
    pub(crate) fn rows(&self) -> &[PinnedRow] {
        self.list.rows()
    }

    /// The selected row's index (`0` on an empty list).
    pub(crate) fn selected(&self) -> usize {
        self.list.selected().unwrap_or(0)
    }

    /// Whether the first read has landed (tests).
    #[cfg(test)]
    pub(crate) fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Whether a write and its read are under way (tests).
    #[cfg(test)]
    pub(crate) fn is_pending(&self) -> bool {
        self.pending
    }

    /// Read the list on tokio; it lands on the UI thread.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let vault = self.vault.clone();
        let reading = tokio::spawn(async move { pinned::load(&vault).await });
        cx.spawn(async move |this, cx| {
            let rows = tasks::joined(reading).await;
            this.update(cx, |dialog, cx| dialog.landed(generation, rows, cx))
                .ok();
        })
        .detach();
    }

    /// Write with `op` on tokio, then read the list again whatever the
    /// write said; its flash (a stale row, a failure) is shown before the
    /// rows land.
    fn persist(
        &mut self,
        op: impl std::future::Future<Output = Outcome> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.pending = true;
        self.generation += 1;
        let generation = self.generation;
        let vault = self.vault.clone();
        let writing = tokio::spawn(async move {
            let outcome = op.await;
            (outcome, pinned::load(&vault).await)
        });
        let workspace = self.workspace.clone();
        cx.spawn(async move |this, cx| {
            let (outcome, rows) = match writing.await {
                Ok(done) => done,
                Err(e) => (Outcome::Failed(e.to_string()), Err(e.to_string())),
            };
            workspace
                .update(cx, |workspace, cx| match &outcome {
                    Outcome::Done => {}
                    Outcome::Stale(text) => workspace.flash(text.clone(), cx),
                    Outcome::Failed(text) => workspace.flash_error(text.clone(), cx),
                })
                .ok();
            this.update(cx, |dialog, cx| dialog.landed(generation, rows, cx))
                .ok();
        })
        .detach();
    }

    /// A read landed: the rows (the cursor clamped), or the failure flashed.
    /// Either way the dialog settles. Dropped when the dialog closed or a
    /// newer read was asked for.
    fn landed(
        &mut self,
        generation: u64,
        rows: Result<Vec<PinnedRow>, String>,
        cx: &mut Context<Self>,
    ) {
        if self.closed.get() || generation != self.generation {
            return;
        }
        self.loaded = true;
        self.pending = false;
        match rows {
            Ok(rows) => self.list.replace_rows(rows),
            Err(error) => {
                self.workspace
                    .update(cx, |workspace, cx| workspace.flash_error(error, cx))
                    .ok();
            }
        }
        cx.notify();
    }

    /// Open row `index` (0-based): the dialog closes and the note opens, or
    /// a flash says why not (no such pin, its note missing) and it stays.
    fn open_index(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let target = pinned::open_target(self.rows(), index);
        self.workspace
            .update(cx, |workspace, cx| match target {
                Ok(path) => {
                    workspace.close_modal(window, cx);
                    workspace.open_chosen(path, None, window, cx);
                }
                Err(flash) => workspace.flash(flash, cx),
            })
            .ok();
    }

    /// Enter, the `Open` button.
    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_index(self.selected(), window, cx);
    }

    /// Move the selection one row down (`true`) or up, within the list.
    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if down {
            self.list.select_next();
        } else {
            self.list.select_prev();
        }
        cx.notify();
    }

    /// Raise (`-1`) or lower (`1`) the selected pin: the rows swap at once
    /// and the cursor follows; core writes it. Ignored while a write runs.
    fn move_selected(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let Some((from, to)) = pinned::swap_selection(self.rows().len(), self.selected(), delta)
        else {
            return;
        };
        self.list.swap_visible(from, to);
        let path = self.rows()[to].path.clone();
        let vault = self.vault.clone();
        self.persist(
            async move { pinned::move_by(&vault, &path, delta).await },
            cx,
        );
        cx.notify();
    }

    /// Unpin the selected pin: its row goes at once (the cursor clamped);
    /// core writes it. Ignored while a write runs.
    fn unpin_selected(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let Some(PinnedRow { path, .. }) = self.list.remove_visible(self.selected()) else {
            return;
        };
        let vault = self.vault.clone();
        self.persist(async move { pinned::unpin(&vault, &path).await }, cx);
        cx.notify();
    }

    /// Close (the `Close` button; Escape is the kit's).
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.close_modal(window, cx))
            .ok();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(KeyInput { key, mods }) = input::key_down_input(event) else {
            return;
        };
        // Shift+j is `J` whether or not the platform sent the typed text.
        let key = match key {
            Key::Char(c) if mods.shift => Key::Char(c.to_ascii_uppercase()),
            key => key,
        };
        match (key, mods) {
            (Key::Up, Mods::ALT) => self.move_selected(-1, cx),
            (Key::Down, Mods::ALT) => self.move_selected(1, cx),
            // Any other chord is not this dialog's: swallowed (it is modal),
            // never read as the bare key.
            _ if mods.ctrl || mods.alt || mods.sup => {}
            (Key::Char(c @ '1'..='9'), _) if !mods.shift => {
                self.open_index(usize::from(c as u8 - b'1'), window, cx)
            }
            (Key::Up | Key::Char('k'), _) => self.step(false, cx),
            (Key::Down | Key::Char('j'), _) => self.step(true, cx),
            (Key::Char('K'), _) => self.move_selected(-1, cx),
            (Key::Char('J'), _) => self.move_selected(1, cx),
            (Key::Delete | Key::Char('d'), _) => self.unpin_selected(cx),
            // Escape, Tab: the kit's.
            (Key::Esc | Key::Tab | Key::BackTab, _) => return,
            _ => {}
        }
        cx.stop_propagation();
    }

    /// The wheel moves the selection: a notched wheel a row per notch, a
    /// trackpad a row per [`WHEEL_STEP_REMS`] (the shared
    /// [`input::wheel_rows`], at most one row per event). Horizontal travel
    /// moves nothing.
    fn on_wheel(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let step = window.rem_size() * WHEEL_STEP_REMS;
        // Positive is up (content down): the selection moves the other way.
        match input::wheel_rows(event, step, &mut self.wheel).signum() {
            0 => return,
            up => self.step(up < 0, cx),
        }
        cx.stop_propagation();
    }

    fn render_row(
        &self,
        index: usize,
        row: &PinnedRow,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let name = row.path.get_clean_name();
        let (name_color, path_color) = if row.missing {
            (theme::muted(t), theme::muted(t))
        } else {
            (theme::foreground(t), theme::secondary(t))
        };
        let label: SharedString = if row.missing {
            format!("{} {name} (missing)", index + 1).into()
        } else {
            format!("{} {name}", index + 1).into()
        };
        let number = div()
            .text_xs()
            .text_color(theme::muted(t))
            .child((index + 1).to_string());
        let name_line = div()
            .flex()
            .gap_2()
            .min_w_0()
            .child(ellipsis(name).text_color(name_color))
            .children(row.missing.then(|| {
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme::muted(t))
                    .child("(missing)")
            }));
        let frame = lane_row(
            pinned_row_id(&row.path),
            label,
            index == self.selected(),
            NOTE_ROW_REMS,
            Some(number.into_any_element()),
            [
                name_line.into_any_element(),
                ellipsis(row.path.to_string())
                    .text_xs()
                    .text_color(path_color)
                    .into_any_element(),
            ],
            t,
        );
        with_click_cursor(frame, ClickPolicy::Open, !row.missing)
            .test_support()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                ClickPolicy::Open.apply(event, this, index, window, cx)
            }))
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.list.selected_row();
        let can_open = selected.is_some_and(|row| !row.missing);
        let can_edit = selected.is_some() && !self.pending;
        let can_raise = can_edit && self.selected() > 0;
        let can_lower = can_edit && self.selected() + 1 < self.rows().len();
        let button = |id: &'static str, label: &'static str, tip: &'static str, enabled: bool| {
            Button::new(id)
                .ghost()
                .xsmall()
                .label(label)
                .tooltip(tip)
                .disabled(!enabled)
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                button(
                    PINNED_OPEN_ID,
                    "Open",
                    "Open the note (Enter, or its number)",
                    can_open,
                )
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.open_selected(window, cx)),
                ),
            )
            .child(
                button(
                    PINNED_RAISE_ID,
                    "Raise",
                    "Move the pin up (Alt+Up or K)",
                    can_raise,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.move_selected(-1, cx))),
            )
            .child(
                button(
                    PINNED_LOWER_ID,
                    "Lower",
                    "Move the pin down (Alt+Down or J)",
                    can_lower,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.move_selected(1, cx))),
            )
            .child(
                button(
                    PINNED_UNPIN_ID,
                    "Unpin",
                    "Unpin the note (Delete or d)",
                    can_edit,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.unpin_selected(cx))),
            )
            .child(div().flex_1())
            .child(
                button(PINNED_CLOSE_ID, "Close", "Close (Escape)", true).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                ),
            )
    }
}

/// A click selects the row and opens it ([`ClickPolicy::Open`]). A pin
/// whose note is missing is only selected: the click is inert (Enter still
/// says why it cannot open).
impl ClickRows for PinnedNotesDialog {
    fn select_row(&mut self, index: usize, cx: &mut Context<Self>) {
        self.list.select(index);
        cx.notify();
    }

    fn activate_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.list.selected() else {
            return;
        };
        if self.rows().get(index).is_some_and(|row| !row.missing) {
            self.open_index(index, window, cx);
        }
    }
}

impl Focusable for PinnedNotesDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PinnedNotesDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let body = div()
            .id("pinned-notes-rows")
            .role(Role::ListBox)
            .aria_label("Pinned notes")
            .flex()
            .flex_col()
            .gap_0p5()
            .h(rems(NOTE_ROW_REMS * PINNED_NOTES_CAP as f32));
        let body = if self.loaded && self.rows().is_empty() {
            body.child(
                div()
                    .id(PINNED_EMPTY_ID)
                    .test_support()
                    .px_2()
                    .py_2()
                    .text_sm()
                    .text_color(theme::muted(t))
                    .child(EMPTY_TEXT),
            )
        } else {
            let rows = self
                .rows()
                .iter()
                .enumerate()
                .map(|(index, row)| self.render_row(index, row, cx).into_any_element())
                .collect::<Vec<_>>();
            body.children(rows)
        };
        div()
            .id(PINNED_ID)
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll(cx.listener(Self::on_wheel))
            .flex()
            .flex_col()
            .gap_2()
            .child(body)
            .child(div().border_t_1().border_color(theme::divider(t)))
            .child(self.render_footer(cx))
    }
}

// ---- The workspace's side ----------------------------------------------------------

impl Workspace {
    /// Leader `f p`, the palette: open the pinned notes, or close them when
    /// they are the open modal. Refused under another modal; says so
    /// without a workspace.
    pub(crate) fn open_pinned_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_modal(window, cx) == Some(ModalKind::PinnedNotes) {
            self.close_modal(window, cx);
            return;
        }
        if modal::dialog_open(window, cx) {
            return;
        }
        let Some(vault) = self.vault_or_flash(cx) else {
            return;
        };
        let closed = Rc::new(Cell::new(false));
        let workspace = cx.entity().downgrade();
        let view = cx.new(|cx| {
            let mut dialog = PinnedNotesDialog::new(vault, closed.clone(), workspace, cx);
            dialog.reload(cx);
            dialog
        });
        self.pinned = Some(PinnedOpen {
            #[cfg(test)]
            dialog: view.downgrade(),
            closed,
        });
        let workspace = cx.entity().downgrade();
        self.show_modal(
            Slot::Own(ModalKind::PinnedNotes),
            DIALOG_REMS,
            modal::focus_view(view.clone()),
            move |dialog, _, _| {
                let opener = view.downgrade();
                let cancel = workspace.clone();
                dialog
                    .title("Pinned notes")
                    // Enter opens the selected pin; the dialog closes itself
                    // only when the note opens.
                    .on_ok(move |_, window, cx| {
                        opener
                            .update(cx, |dialog, cx| dialog.open_selected(window, cx))
                            .ok();
                        false
                    })
                    .on_cancel(move |_, window, cx| {
                        cancel
                            .update(cx, |workspace, cx| workspace.close_modal(window, cx))
                            .ok();
                        false
                    })
                    .child(view.clone())
            },
            window,
            cx,
        );
    }

    /// The dialog is going: a read still under way is dropped when it
    /// lands.
    pub(super) fn forget_pinned_notes(&mut self) {
        if let Some(open) = self.pinned.take() {
            open.close();
        }
    }
}
