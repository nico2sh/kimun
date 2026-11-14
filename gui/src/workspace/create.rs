//! **Making notes** (M6 G4): the create dialog, the quick note, and today's
//! journal entry. Each is one core call through `logic::file_ops`; the note
//! made opens through the note session like any other (the open note is
//! saved first) and is announced (`FileOp::Created`) so FILES lists it.
//!
//! - **Create dialog** (kit `Dialog`): offered when a followed link names
//!   no note, and when a note asked for is not there. `Create "name"?`,
//!   then `Create` makes it (an existing note at that path is opened, never
//!   overwritten) and opens it.
//! - **Quick note** (`QuickNote` chord, the palette): one line of text into
//!   a timestamped note in the inbox. Enter on an empty line closes without
//!   making anything; Enter saves and closes; Shift+Enter saves and opens
//!   the note. Typing clears a shown error.
//! - **Journal** (`NewJournal` chord, leader `n d`, the palette): today's
//!   entry, made with its date heading the first time, opened every time.
//!
//! A failure stays in the dialog, which stays open; while a note is being
//! made its buttons wait and Escape does nothing.

use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState};
use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, WeakEntity, Window, div,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::file_ops;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::message::{AppMessage, FileOp};

use super::Workspace;
use super::file_dialogs::{CANCEL_ID, CONFIRM_ID, OpStatus, error_row, footer, path_line};
use super::modal::{self, ModalKind, Slot};
use super::route::NO_WORKSPACE;
use crate::tasks;
use crate::theme;

/// Element ids, for tests.
pub(crate) const CREATE_ID: &str = "create-note";
pub(crate) const QUICK_NOTE_ID: &str = "quick-note";
pub(crate) const QUICK_NOTE_INPUT_ID: &str = "quick-note-input";
pub(crate) const QUICK_NOTE_OPEN_ID: &str = "quick-note-open";

/// The dialogs' width, in rems.
const DIALOG_REMS: f32 = 30.;

// ---- The create dialog -----------------------------------------------------------

/// The create dialog's body: where the note would go, why the last attempt
/// failed, and `Cancel` / `Create`. The title is the dialog's.
pub(crate) struct CreateNoteBody {
    path: VaultPath,
    /// What the note starts with (a saved Ask answer); `None`: empty.
    content: Option<String>,
    workspace: WeakEntity<Workspace>,
    status: Entity<OpStatus>,
    _status_watch: Subscription,
}

impl CreateNoteBody {
    fn new(
        path: VaultPath,
        content: Option<String>,
        workspace: WeakEntity<Workspace>,
        status: Entity<OpStatus>,
        cx: &mut Context<Self>,
    ) -> Self {
        let watch = cx.observe(&status, |_, _, cx| cx.notify());
        Self {
            path,
            content,
            workspace,
            status,
            _status_watch: watch,
        }
    }
}

impl Render for CreateNoteBody {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status.read(cx);
        let busy = status.busy;
        let confirm = self.workspace.clone();
        let path = self.path.clone();
        let content = self.content.clone();
        let cancel = self.workspace.clone();
        div()
            .id(CREATE_ID)
            .flex()
            .flex_col()
            .gap_3()
            .child(path_line(
                "There is no note at",
                &self.path,
                &theme::current(cx),
            ))
            .children(error_row(status, &theme::current(cx)))
            .child(footer(
                "Create",
                true,
                busy,
                move |_, window, cx| {
                    let path = path.clone();
                    let content = content.clone();
                    confirm
                        .update(cx, |workspace, cx| {
                            workspace.create_confirmed(path, content, window, cx)
                        })
                        .ok();
                },
                move |_, window, cx| {
                    cancel
                        .update(cx, |workspace, cx| {
                            workspace.dismiss_note_dialog(window, cx)
                        })
                        .ok();
                },
            ))
    }
}

// ---- The quick note ----------------------------------------------------------------

/// The quick note: one line of text, saved into the inbox.
pub(crate) struct QuickNoteDialog {
    pub(crate) input: Entity<InputState>,
    workspace: WeakEntity<Workspace>,
    status: Entity<OpStatus>,
    _subscriptions: Vec<Subscription>,
}

impl QuickNoteDialog {
    fn new(
        workspace: WeakEntity<Workspace>,
        status: Entity<OpStatus>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type your thought"));
        let subscriptions = vec![
            // Typing clears a shown error (the TUI's rule).
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.status.update(cx, |status, cx| {
                        if status.error.take().is_some() {
                            cx.notify();
                        }
                    });
                }
            }),
            cx.observe(&status, |_, _, cx| cx.notify()),
        ];
        Self {
            input,
            workspace,
            status,
            _subscriptions: subscriptions,
        }
    }

    /// Enter (`open`: Shift+Enter, or the `Save and open` button): save the
    /// text, or close on an empty line.
    fn submit(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.status.read(cx).busy {
            return;
        }
        let text = self.input.read(cx).value().to_string();
        self.workspace
            .update(cx, |workspace, cx| {
                if text.trim().is_empty() {
                    workspace.dismiss_note_dialog(window, cx);
                } else {
                    workspace.quick_note_confirmed(text, open, window, cx);
                }
            })
            .ok();
    }
}

impl Render for QuickNoteDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let status = self.status.read(cx);
        let busy = status.busy;
        let save = cx.entity().downgrade();
        let save_open = save.clone();
        let cancel = self.workspace.clone();
        div()
            .id(QUICK_NOTE_ID)
            .flex()
            .flex_col()
            .gap_3()
            .capture_action(cx.listener(|this, action: &Enter, window, cx| {
                this.submit(action.shift, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                this.workspace
                    .update(cx, |workspace, cx| {
                        workspace.dismiss_note_dialog(window, cx)
                    })
                    .ok();
                cx.stop_propagation();
            }))
            .child(
                Input::new(&self.input)
                    .id(QUICK_NOTE_INPUT_ID)
                    .aria_label("Quick note text"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::muted(t))
                    .child("Saved in the inbox. Shift+Enter saves and opens it."),
            )
            .children(error_row(status, t))
            .child(
                modal::footer_row()
                    .child(
                        Button::new(CANCEL_ID)
                            .label("Cancel")
                            .disabled(busy)
                            .on_click(move |_: &ClickEvent, window, cx| {
                                cancel
                                    .update(cx, |workspace, cx| {
                                        workspace.dismiss_note_dialog(window, cx)
                                    })
                                    .ok();
                            }),
                    )
                    .child(
                        Button::new(QUICK_NOTE_OPEN_ID)
                            .label("Save and open")
                            .disabled(busy)
                            .on_click(move |_: &ClickEvent, window, cx| {
                                save_open
                                    .update(cx, |dialog, cx| dialog.submit(true, window, cx))
                                    .ok();
                            }),
                    )
                    .child(
                        Button::new(CONFIRM_ID)
                            .primary()
                            .label("Save")
                            .loading(busy)
                            .disabled(busy)
                            .on_click(move |_: &ClickEvent, window, cx| {
                                save.update(cx, |dialog, cx| dialog.submit(false, window, cx))
                                    .ok();
                            }),
                    ),
            )
    }
}

// ---- The workspace's side ----------------------------------------------------------

/// A note's name as the dialogs say it.
fn name_of(path: &VaultPath) -> String {
    path.get_parent_path().1
}

impl Workspace {
    /// Offer to create the note at `path`, which is not there (a followed
    /// link, an open). With a modal already open it only says so.
    pub(crate) fn offer_create(&mut self, path: VaultPath, cx: &mut Context<Self>) {
        self.offer_create_with(path, None, cx);
    }

    /// [`Self::offer_create`], the note to start with `content` (a saved
    /// Ask answer; `None`: empty). With a modal already open it only says so,
    /// by what was offered: a missing note, or an answer left unsaved.
    pub(crate) fn offer_create_with(
        &mut self,
        path: VaultPath,
        content: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.with_window(cx, move |workspace, window, cx| {
            if modal::dialog_open(window, cx) {
                // A saved answer is not "not found": say what to do.
                let why = if content.is_some() {
                    "Couldn't save the answer: close the open dialog and save again.".to_string()
                } else {
                    format!("{path} not found")
                };
                workspace.flash_error(why, cx);
            } else {
                workspace.open_create_dialog(path, content, window, cx);
            }
        });
    }

    /// Open the create dialog for `path`, the note to hold `content` once
    /// made (`None`: empty). Refused while a modal is open, and without a
    /// workspace.
    pub(crate) fn open_create_dialog(
        &mut self,
        path: VaultPath,
        content: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) || self.active_state().notes.is_none() {
            return;
        }
        let status = cx.new(|_| OpStatus::default());
        self.note_status = Some(status.clone());
        let workspace = cx.entity().downgrade();
        let title = format!("Create \"{}\"?", name_of(&path));
        let body = {
            let path = path.clone();
            let content = content.clone();
            let status = status.clone();
            cx.new(|cx| CreateNoteBody::new(path, content, workspace.clone(), status, cx))
        };
        self.show_modal(
            Slot::Own(ModalKind::CreateNote),
            DIALOG_REMS,
            |_, _| {},
            move |dialog, _, _| {
                let confirm = workspace.clone();
                let path = path.clone();
                let content = content.clone();
                let running = status.clone();
                dialog
                    .title(title.clone())
                    // Enter: the dialog's own confirm.
                    .on_ok(move |_, window, cx| {
                        let path = path.clone();
                        let content = content.clone();
                        confirm
                            .update(cx, |workspace, cx| {
                                workspace.create_confirmed(path, content, window, cx)
                            })
                            .ok();
                        false
                    })
                    .on_cancel(move |_, _, cx| !running.read(cx).busy)
                    .child(body.clone())
            },
            window,
            cx,
        );
    }

    /// `Create` in the create dialog: core makes the note (or finds one
    /// already there; with `content` it always makes a fresh one, a taken
    /// name getting a suffix), then it opens; a failure stays in the
    /// dialog.
    pub(crate) fn create_confirmed(
        &mut self,
        path: VaultPath,
        content: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.vault() else {
            return;
        };
        if self.note_task.is_some() {
            return;
        }
        self.set_note_status(true, None, cx);
        let mine = self.note_status.clone();
        let made = tokio::spawn(async move { file_ops::create_note(&vault, &path, content).await });
        self.note_task = Some(cx.spawn_in(window, async move |this, cx| {
            let made = tasks::joined(made).await;
            this.update_in(cx, |workspace, window, cx| {
                workspace.note_task = None;
                match made {
                    Ok(outcome) => {
                        workspace.close_note_dialog(&mine, window, cx);
                        workspace.focus_panel(Panel::Editor, window, cx);
                        if let Some(tx) = workspace.tx.as_ref() {
                            outcome.announce_and_open(tx);
                        }
                    }
                    Err(error) => workspace.note_failed(&mine, error, cx),
                }
            })
            .ok();
        }));
    }

    /// Open the quick note, its line focused. Refused while a modal is
    /// open; says so without a workspace.
    pub(crate) fn open_quick_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if modal::dialog_open(window, cx) {
            return;
        }
        if self.active_state().notes.is_none() {
            self.flash(NO_WORKSPACE, cx);
            return;
        }
        let status = cx.new(|_| OpStatus::default());
        self.note_status = Some(status.clone());
        let workspace = cx.entity().downgrade();
        let view = cx.new(|cx| QuickNoteDialog::new(workspace, status.clone(), window, cx));
        self.quick_note = Some(view.downgrade());
        let input = view.read(cx).input.clone();
        self.show_modal(
            Slot::Own(ModalKind::QuickNote),
            DIALOG_REMS,
            move |window, cx| input.update(cx, |input, cx| input.focus(window, cx)),
            move |dialog, _, _| {
                let running = status.clone();
                dialog
                    .title("Quick note")
                    .on_cancel(move |_, _, cx| !running.read(cx).busy)
                    .child(view.clone())
            },
            window,
            cx,
        );
    }

    /// Save `text` as a quick note; then close, or open it (`open`). A
    /// failure stays in the dialog.
    pub(crate) fn quick_note_confirmed(
        &mut self,
        text: String,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.vault() else {
            return;
        };
        if self.note_task.is_some() {
            return;
        }
        self.set_note_status(true, None, cx);
        let mine = self.note_status.clone();
        let made = tokio::spawn(async move { file_ops::quick_note(&vault, &text).await });
        self.note_task = Some(cx.spawn_in(window, async move |this, cx| {
            let made = tasks::joined(made).await;
            this.update_in(cx, |workspace, window, cx| {
                workspace.note_task = None;
                match made {
                    Ok(path) => {
                        workspace.close_note_dialog(&mine, window, cx);
                        if let Some(tx) = workspace.tx.as_ref() {
                            // Always a fresh note: FILES lists it.
                            tx.send(AppMessage::FileOp(FileOp::Created(path.clone())))
                                .ok();
                        }
                        if open {
                            workspace.open_chosen(path, None, window, cx);
                        } else {
                            workspace.flash(format!("Saved {}", name_of(&path)), cx);
                        }
                    }
                    Err(error) => workspace.note_failed(&mine, error, cx),
                }
            })
            .ok();
        }));
    }

    /// The `NewJournal` chord, leader `n d`: today's journal entry, made
    /// the first time, opened in the editor (which takes the keyboard).
    pub(crate) fn open_journal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.vault() else {
            self.flash(NO_WORKSPACE, cx);
            return;
        };
        let made = {
            let vault = vault.clone();
            tokio::spawn(async move { file_ops::journal(&vault).await })
        };
        cx.spawn_in(window, async move |this, cx| {
            let made = tasks::joined(made).await;
            this.update_in(cx, |workspace, window, cx| match made {
                // Another vault was entered meanwhile: not its journal.
                _ if !workspace.is_vault(&vault) => {}
                Ok(outcome) => {
                    workspace.focus_panel(Panel::Editor, window, cx);
                    if let Some(tx) = workspace.tx.as_ref() {
                        outcome.announce_and_open(tx);
                    }
                }
                Err(error) => {
                    workspace.flash_error(format!("Couldn't open the journal: {error}"), cx)
                }
            })
            .ok();
        })
        .detach();
    }

    /// Cancel or Escape in the create dialog or the quick note: close it,
    /// unless a note is being made.
    pub(crate) fn dismiss_note_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note_task.is_some() {
            return;
        }
        self.close_modal(window, cx);
    }

    /// Whether the dialog whose status is `mine` (the create dialog or the
    /// quick note that started the note task) is still the one up. A file
    /// operation landing meanwhile closes every modal and forgets its
    /// status ([`Self::reset_modal_state`]); by the time the task lands the
    /// slot may hold another modal, or a new note dialog.
    fn is_note_dialog_up(&self, mine: &Option<Entity<OpStatus>>) -> bool {
        mine.is_some() && self.note_status == *mine
    }

    /// The note task from the dialog `mine` made its note: close that
    /// dialog — not whatever modal took its place.
    fn close_note_dialog(
        &mut self,
        mine: &Option<Entity<OpStatus>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_note_dialog_up(mine) {
            self.close_modal(window, cx);
        }
    }

    /// The note task from the dialog `mine` failed: the dialog says why,
    /// or, when it is gone, a flash does.
    fn note_failed(
        &mut self,
        mine: &Option<Entity<OpStatus>>,
        error: String,
        cx: &mut Context<Self>,
    ) {
        if self.is_note_dialog_up(mine) {
            self.set_note_status(false, Some(error), cx);
        } else {
            self.flash_error(error, cx);
        }
    }

    fn set_note_status(&self, busy: bool, error: Option<String>, cx: &mut Context<Self>) {
        if let Some(status) = self.note_status.as_ref() {
            status.update(cx, |status, cx| {
                status.busy = busy;
                status.error = error.map(Into::into);
                cx.notify();
            });
        }
    }

    /// The error the create dialog or the quick note shows (tests).
    #[cfg(test)]
    pub(crate) fn note_error(&self, cx: &gpui_kit::App) -> Option<String> {
        let status = self.note_status.as_ref()?;
        status.read(cx).error.as_ref().map(|e| e.to_string())
    }
}
