//! **New note / new directory** (`NewNote`, `NewDirectory`, leader `n n` /
//! `n f`, FILES' `+` menu and its `Create:` row): one dialog, whose kind
//! picks the wording and what the confirm makes (`NewEntryKind`, shared with
//! the TUI so the two cannot drift).
//!
//! The typed name is a path relative to the chosen folder, which starts as
//! the one FILES shows (else the open note's, else the root); a leading `/`
//! starts from the vault root, and the folders in the name are made with the
//! entry. `Change…` opens the [folder choice](super::directory_choice) under
//! the folder line. The resolved target shows as the user types, a name core
//! refuses says why, and a check marks a name already taken. Enter creates
//! unless the name is taken or refused: the create is exclusive, so a stale
//! check can never overwrite anything.
//!
//! A made note opens in the editor (which takes the keyboard); a made
//! directory opens nothing, so FILES points it out instead
//! ([`FilesModel::reveal`](kimun_notes::logic::files::FilesModel::reveal)).
//! A failure stays in the dialog, which stays open; while an entry is being
//! made its buttons wait and Escape does nothing.

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Task, WeakEntity, Window,
    div,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::file_ops::{self, NewEntry, NewEntryKind, ValidationState};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::message::{AppMessage, FileOp};
use kimun_notes::logic::nav::DrawerView;

use super::Workspace;
use super::directory_choice::{DirectoryChoice, DirectoryChoiceEvent, directory_label};
use super::file_dialogs::{OpStatus, check_free, error_row, footer, hint};
use super::modal::{self, ModalKind, Slot};
use super::route::NO_WORKSPACE;
use crate::tasks;
use crate::theme;

/// Element ids, for tests.
pub(crate) const NEW_ENTRY_ID: &str = "new-entry";
pub(crate) const NEW_ENTRY_INPUT_ID: &str = "new-entry-input";
pub(crate) const NEW_ENTRY_TARGET_ID: &str = "new-entry-target";
pub(crate) const NEW_ENTRY_DIR_ID: &str = "new-entry-directory";
pub(crate) const NEW_ENTRY_CHANGE_ID: &str = "new-entry-change";

/// The dialog's width, in rems.
const DIALOG_REMS: f32 = 32.;

/// The dialog's body. The title is the kit dialog's.
pub(crate) struct NewEntryDialog {
    kind: NewEntryKind,
    /// The folder the typed name is relative to: the one the dialog opened
    /// on until another is chosen.
    base: VaultPath,
    pub(crate) input: Entity<InputState>,
    vault: Arc<NoteVault>,
    workspace: WeakEntity<Workspace>,
    status: Entity<OpStatus>,
    /// What the name resolves to: `None` while it is empty.
    target: Option<Result<VaultPath, String>>,
    state: ValidationState,
    /// The availability check under way; a newer one replaces it.
    check: Option<Task<()>>,
    /// The folder choice, while it is open.
    choice: Option<Entity<DirectoryChoice>>,
    _choice_events: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

/// What a dialog starts with.
struct Start {
    kind: NewEntryKind,
    /// The folder the dialog opens on.
    base: VaultPath,
    /// A name typed in already (a `Create:` row's).
    initial: Option<String>,
}

impl NewEntryDialog {
    fn new(
        start: Start,
        vault: Arc<NoteVault>,
        workspace: WeakEntity<Workspace>,
        status: Entity<OpStatus>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let Start {
            kind,
            base,
            initial,
        } = start;
        let input = cx.new(|cx| {
            let state = InputState::new(window, cx).placeholder(match kind {
                NewEntryKind::Note => "Name, or folder/name",
                NewEntryKind::Directory => "Name, or folder/name",
            });
            match initial.clone() {
                Some(text) => state.default_value(text),
                None => state,
            }
        });
        let subscriptions = vec![
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.edited(cx);
                }
            }),
            cx.observe(&status, |_, _, cx| cx.notify()),
        ];
        let mut dialog = Self {
            kind,
            base,
            input,
            vault,
            workspace,
            status,
            target: None,
            state: ValidationState::Idle,
            check: None,
            choice: None,
            _choice_events: None,
            _subscriptions: subscriptions,
        };
        if initial.is_some() {
            dialog.resolve(cx);
        }
        dialog
    }

    #[cfg(test)]
    pub(crate) fn target(&self) -> Option<&Result<VaultPath, String>> {
        self.target.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn base(&self) -> &VaultPath {
        &self.base
    }

    #[cfg(test)]
    pub(crate) fn state(&self) -> ValidationState {
        self.state
    }

    #[cfg(test)]
    pub(crate) fn choice(&self) -> Option<&Entity<DirectoryChoice>> {
        self.choice.as_ref()
    }

    /// The name changed: a shown error goes, and the target is worked out
    /// again.
    fn edited(&mut self, cx: &mut Context<Self>) {
        self.status.update(cx, |status, cx| {
            if status.error.take().is_some() {
                cx.notify();
            }
        });
        self.resolve(cx);
    }

    /// Work out what the name resolves to and, when it names something,
    /// check whether that is taken (the latest check wins).
    fn resolve(&mut self, cx: &mut Context<Self>) {
        self.check = None;
        let typed = self.input.read(cx).value().to_string();
        if typed.trim().is_empty() {
            self.target = None;
            self.state = ValidationState::Idle;
            cx.notify();
            return;
        }
        let target = self.kind.target(&self.base, &typed);
        self.target = Some(target.clone());
        match target {
            Ok(path) => {
                self.state = ValidationState::Pending;
                let checked = path.clone();
                self.check = Some(check_free(
                    cx,
                    self.vault.clone(),
                    checked,
                    move |dialog: &mut Self, state, cx| {
                        if dialog.target == Some(Ok(path)) {
                            dialog.state = state;
                            cx.notify();
                        }
                    },
                ));
            }
            // Core's reason shows in place of the check.
            Err(_) => self.state = ValidationState::Idle,
        }
        cx.notify();
    }

    /// The path Enter would make, when it may be made now.
    fn confirmable(&self) -> Option<VaultPath> {
        match (&self.target, self.state) {
            (Some(Ok(_)), ValidationState::Taken) => None,
            (Some(Ok(path)), _) => Some(path.clone()),
            _ => None,
        }
    }

    pub(crate) fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status.read(cx).busy {
            return;
        }
        let Some(path) = self.confirmable() else {
            return;
        };
        let kind = self.kind;
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.new_entry_confirmed(kind, path, window, cx)
            })
            .ok();
    }

    // ---- The folder choice -----------------------------------------------------

    /// `Change…`: open the folder choice (or close it), the keyboard in its
    /// filter.
    pub(crate) fn toggle_choice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.choice.is_some() {
            self.close_choice(window, cx);
            return;
        }
        let (vault, base) = (self.vault.clone(), self.base.clone());
        let choice = cx.new(|cx| DirectoryChoice::new(vault, base, window, cx));
        self._choice_events = Some(cx.subscribe_in(
            &choice,
            window,
            |this, _, event: &DirectoryChoiceEvent, window, cx| {
                let DirectoryChoiceEvent::Chosen(dir) = event;
                this.base = dir.clone();
                this.close_choice(window, cx);
                this.resolve(cx);
            },
        ));
        choice.update(cx, |choice, cx| choice.focus(window, cx));
        self.choice = Some(choice);
        cx.notify();
    }

    /// Close the folder choice, the keyboard back in the name.
    fn close_choice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.choice = None;
        self._choice_events = None;
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
}

impl Render for NewEntryDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let status = self.status.read(cx);
        let busy = status.busy;
        let refused = match &self.target {
            Some(Err(reason)) => Some(reason.as_str()),
            _ => None,
        };
        let hint = hint(self.state, refused, t);
        let enabled = self.confirmable().is_some();
        let choosing = self.choice.is_some();
        let place: SharedString = match &self.target {
            Some(Ok(path)) => path.to_string().into(),
            _ => format!("in {}", self.base).into(),
        };
        let confirm = cx.entity().downgrade();
        let workspace = self.workspace.clone();
        div()
            .id(NEW_ENTRY_ID)
            .flex()
            .flex_col()
            .gap_3()
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                match this.choice.clone() {
                    Some(choice) => choice.update(cx, |choice, cx| choice.choose(cx)),
                    None => this.confirm(window, cx),
                }
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.choice.is_some() {
                    this.close_choice(window, cx);
                } else {
                    this.workspace
                        .update(cx, |workspace, cx| {
                            workspace.dismiss_note_dialog(window, cx)
                        })
                        .ok();
                }
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                if let Some(choice) = this.choice.clone() {
                    choice.update(cx, |choice, cx| choice.step(false, cx));
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                if let Some(choice) = this.choice.clone() {
                    choice.update(cx, |choice, cx| choice.step(true, cx));
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .id(NEW_ENTRY_TARGET_ID)
                    .test_support()
                    .text_sm()
                    .text_color(theme::foreground(t))
                    .truncate()
                    .child(place),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_xs().text_color(theme::muted(t)).child("Folder"))
                            .child(
                                div()
                                    .id(NEW_ENTRY_DIR_ID)
                                    .test_support()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .text_color(theme::directory(t))
                                    .truncate()
                                    .child(directory_label(&self.base)),
                            )
                            .child(
                                Button::new(NEW_ENTRY_CHANGE_ID)
                                    .label(if choosing { "Done" } else { "Change…" })
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.toggle_choice(window, cx)
                                    })),
                            ),
                    )
                    .children(self.choice.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(t))
                            .child(self.kind.field_label()),
                    )
                    .child(
                        Input::new(&self.input)
                            .id(NEW_ENTRY_INPUT_ID)
                            .aria_label(self.kind.field_label()),
                    ),
            )
            .child(
                div()
                    .h(gpui_kit::rems(1.25))
                    .text_sm()
                    .when_some(hint, |row, (text, color)| row.text_color(color).child(text)),
            )
            .children(error_row(status, t))
            .child(footer(
                "Create",
                enabled,
                busy,
                move |_, window, cx| {
                    confirm
                        .update(cx, |dialog, cx| dialog.confirm(window, cx))
                        .ok();
                },
                move |_, window, cx| {
                    workspace
                        .update(cx, |workspace, cx| {
                            workspace.dismiss_note_dialog(window, cx)
                        })
                        .ok();
                },
            ))
    }
}

// ---- The workspace's side ----------------------------------------------------------

impl Workspace {
    /// The folder a new note or directory goes into: the one FILES shows
    /// while it is shown, else the open note's folder, else the root.
    pub(crate) fn new_entry_base(&self, cx: &gpui_kit::App) -> VaultPath {
        let browsed = self
            .is_shown(DrawerView::Files)
            .then(|| {
                self.files
                    .as_ref()
                    .map(|f| f.read(cx).current_dir().clone())
            })
            .flatten();
        let note = self
            .active_state()
            .notes
            .as_ref()
            .and_then(|h| h.note.clone());
        file_ops::new_entry_base(browsed.as_ref(), note.as_ref())
    }

    /// `NewNote` / `NewDirectory` (the chords, leader `n n` / `n f`, the
    /// palette, FILES' `+`): the dialog on [`Self::new_entry_base`].
    pub(crate) fn open_new_entry_here(
        &mut self,
        kind: NewEntryKind,
        initial: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let base = self.new_entry_base(cx);
        self.open_new_entry(kind, base, initial, window, cx);
    }

    /// Open the dialog for `kind` on `base`, `initial` typed in. Refused
    /// while a modal is open; says so without a workspace.
    pub(crate) fn open_new_entry(
        &mut self,
        kind: NewEntryKind,
        base: VaultPath,
        initial: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) {
            return;
        }
        let Some(vault) = self.vault() else {
            self.flash(NO_WORKSPACE, cx);
            return;
        };
        if self.active_state().notes.is_none() {
            self.flash(NO_WORKSPACE, cx);
            return;
        }
        let status = cx.new(|_| OpStatus::default());
        self.note_status = Some(status.clone());
        let workspace = cx.entity().downgrade();
        let view = {
            let status = status.clone();
            let start = Start {
                kind,
                base,
                initial,
            };
            cx.new(|cx| NewEntryDialog::new(start, vault, workspace, status, window, cx))
        };
        self.new_entry = Some(view.downgrade());
        let input = view.read(cx).input.clone();
        let slot = match kind {
            NewEntryKind::Note => ModalKind::NewNote,
            NewEntryKind::Directory => ModalKind::NewDirectory,
        };
        self.show_modal(
            Slot::Own(slot),
            DIALOG_REMS,
            move |window, cx| input.update(cx, |input, cx| input.focus(window, cx)),
            move |dialog, _, _| {
                let running = status.clone();
                let ok = view.downgrade();
                dialog
                    .title(kind.title())
                    // Enter reaches the body's own capture first; this is
                    // the kit's fallback for a click-focused dialog.
                    .on_ok(move |_, window, cx| {
                        ok.update(cx, |dialog, cx| dialog.confirm(window, cx)).ok();
                        false
                    })
                    .on_cancel(move |_, _, cx| !running.read(cx).busy)
                    .child(view.clone())
            },
            window,
            cx,
        );
    }

    /// `Create` in the dialog: core makes the entry, then a note opens and a
    /// directory is pointed out in FILES; a failure stays in the dialog.
    pub(crate) fn new_entry_confirmed(
        &mut self,
        kind: NewEntryKind,
        path: VaultPath,
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
        let made = tokio::spawn(async move { kind.create(&vault, &path, None).await });
        self.note_task = Some(cx.spawn_in(window, async move |this, cx| {
            let made = tasks::joined(made).await;
            this.update_in(cx, |workspace, window, cx| {
                workspace.note_task = None;
                match made {
                    Ok(entry) => {
                        workspace.close_note_dialog(&mine, window, cx);
                        workspace.new_entry_made(entry, window, cx);
                    }
                    Err(error) => workspace.note_failed(&mine, error, cx),
                }
            })
            .ok();
        }));
    }

    /// An entry was made: a note opens in the editor, a directory is
    /// pointed out in FILES.
    fn new_entry_made(&mut self, entry: NewEntry, window: &mut Window, cx: &mut Context<Self>) {
        match entry {
            NewEntry::Note(outcome) => {
                self.focus_panel(Panel::Editor, window, cx);
                if let Some(tx) = self.tx.as_ref() {
                    outcome.announce_and_open(tx);
                }
            }
            NewEntry::Directory(path) => {
                if let Some(tx) = self.tx.as_ref() {
                    tx.send(AppMessage::FileOp(FileOp::Created(path))).ok();
                }
            }
        }
    }
}
