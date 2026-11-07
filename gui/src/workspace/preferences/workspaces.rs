//! Preferences > **Workspaces** and **Indexing** (M9b-2), and the vault-path
//! reindex flow they share with Save.
//!
//! **Workspaces** lists the configured workspaces (`workspace_rows`: sorted,
//! the current one tagged). Its edits are made at once, as the TUI makes
//! them, not kept in the draft: each one ([`WorkspaceEdit`]) is applied to a
//! copy of the shared settings, the copy is written through the window's one
//! settings write (off the UI thread), and only once it is on disk is the
//! same edit applied to the shared settings. A failed write is shown in the
//! section and changes nothing. Workspace edits never make the draft dirty.
//!
//! - **New:** a name (checked as it is typed) and a folder from the shared
//!   [`FolderChooser`]; the folder is made through core when it does not
//!   exist (the first workspace's suggested folder). The first workspace
//!   becomes the current one, and the window enters it.
//! - **Rename:** in place; the window title follows a renamed current one.
//! - **Delete:** asks first, never the current workspace; its index and
//!   history are removed off the UI thread, the notes stay.
//! - **Switch:** Preferences close and the window switches (M8b).
//! - **Change path:** the chooser. Another workspace's folder is saved at
//!   once; the current one's runs the vault-path reindex flow.
//!
//! **The vault-path reindex flow** (the TUI's): the new folder is indexed
//! from scratch on a throwaway vault, off the UI thread, while a progress
//! panel holds the view. Only when that worked are the settings written and
//! applied, and the window enters the vault again. A case conflict is shown
//! in a panel and nothing changes; any other failure likewise. A Save whose
//! plan needs a reindex takes the same flow.
//!
//! A change that makes the window enter a vault (Switch, the current
//! workspace's new folder, the first workspace) drops an unsaved draft: it
//! asks first when there is one.
//!
//! **Indexing** runs a fast or (confirmed) full reindex of the open
//! workspace on a throwaway vault, one at a time, then tells the window to
//! pick the new index up in place. Inert without a workspace.

use std::path::PathBuf;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable as _,
    FontWeight, InteractiveElement, IntoElement, ParentElement, Role, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
};
use kimun_core::IndexFile;
use kimun_core::system::{self, SystemPath};
use kimun_notes::logic::preferences::{
    self as prefs, FIRST_WORKSPACE_NAME, ReindexOutcome, WorkspaceEdit, WorkspaceError,
};
use kimun_notes::logic::workspaces::{WorkspaceRow, workspace_rows};
use kimun_notes::settings::AppSettings;
use kimun_notes::settings::themes::Theme;

use super::super::folder_chooser::{self, FolderChooser, FolderChooserEvent};
use super::super::reopen;
use super::{PreferencesEvent, PreferencesView, note, para};
use crate::tasks::blocking;
use crate::theme;

/// The Workspaces section's controls.
pub(crate) const WS_LIST_ID: &str = "preferences-workspaces";
pub(crate) const WS_NEW_ID: &str = "preferences-ws-new";
pub(crate) const WS_SWITCH_ID: &str = "preferences-ws-switch";
pub(crate) const WS_RENAME_ID: &str = "preferences-ws-rename";
pub(crate) const WS_PATH_ID: &str = "preferences-ws-path";
pub(crate) const WS_DELETE_ID: &str = "preferences-ws-delete";
pub(crate) const WS_SETUP_ID: &str = "preferences-ws-setup";
pub(crate) const WS_EMPTY_ID: &str = "preferences-ws-empty";
/// The New form.
pub(crate) const WS_NAME_ID: &str = "preferences-ws-name";
pub(crate) const WS_NAME_ERROR_ID: &str = "preferences-ws-name-error";
pub(crate) const WS_FOLDER_ID: &str = "preferences-ws-folder";
pub(crate) const WS_CHOOSE_ID: &str = "preferences-ws-choose";
pub(crate) const WS_CREATE_ID: &str = "preferences-ws-create";
pub(crate) const WS_FORM_CANCEL_ID: &str = "preferences-ws-form-cancel";
/// The Rename form.
pub(crate) const WS_RENAME_INPUT_ID: &str = "preferences-ws-rename-input";
pub(crate) const WS_RENAME_SAVE_ID: &str = "preferences-ws-rename-save";
/// The delete question.
pub(crate) const WS_DELETE_YES_ID: &str = "preferences-ws-delete-yes";
pub(crate) const WS_DELETE_NO_ID: &str = "preferences-ws-delete-no";
/// The section's result line.
pub(crate) const WS_MESSAGE_ID: &str = "preferences-ws-message";
/// The "discard your changes?" question before the window enters a vault.
pub(crate) const ASK_KEEP_ID: &str = "preferences-ask-keep";
pub(crate) const ASK_GO_ID: &str = "preferences-ask-go";
/// The case-conflict panel.
pub(crate) const CONFLICT_ID: &str = "preferences-conflict";
pub(crate) const CONFLICT_DISMISS_ID: &str = "preferences-conflict-dismiss";
/// The reindex progress panel.
pub(crate) const PROGRESS_ID: &str = "preferences-progress";
/// The Indexing section.
pub(crate) const FAST_ID: &str = "preferences-fast-reindex";
pub(crate) const FULL_ID: &str = "preferences-full-reindex";
pub(crate) const FULL_YES_ID: &str = "preferences-full-yes";
pub(crate) const FULL_NO_ID: &str = "preferences-full-no";
pub(crate) const INDEX_MESSAGE_ID: &str = "preferences-index-message";
/// Why the Indexing buttons are off.
pub(crate) const INDEX_INERT_ID: &str = "preferences-index-inert";

/// The id prefix of the section's folder chooser.
pub(crate) const CHOOSER_PREFIX: &str = "preferences";

/// What the Indexing section says without a workspace.
pub(crate) const NEEDS_WORKSPACE: &str = "Needs an open workspace.";
/// The full reindex question.
pub(crate) const FULL_QUESTION: &str = "Rebuild the whole index? This can take a while.";
/// The progress panel's text while a moved vault is indexed.
pub(crate) const INDEXING_NEW: &str = "Indexing the new location…";
/// The progress panel's text while the open workspace is indexed.
pub(crate) const INDEXING: &str = "Indexing the workspace…";
/// The case-conflict panel's title.
pub(crate) const CONFLICT_TITLE: &str = "Two files differ only by letter case";

/// The element id of the workspace list's row for `name`.
pub(crate) fn workspace_row_id(name: &str) -> SharedString {
    format!("preferences-workspace-{name}").into()
}

/// The delete question for workspace `name`: what goes and what stays
/// (`take_workspace_for_delete` + `remove_workspace_artifacts` remove the
/// index and the history only).
pub(crate) fn delete_question(name: &str) -> String {
    format!(
        "Delete workspace '{name}'? Its index and history are removed; your notes stay on disk."
    )
}

/// A result line: a confirmation or a problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub(crate) text: SharedString,
    pub(crate) error: bool,
}

impl Notice {
    fn info(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            error: false,
        }
    }

    fn error(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            error: true,
        }
    }
}

/// The New form.
pub(crate) struct NewForm {
    input: Entity<InputState>,
    error: Option<SharedString>,
    /// The folder chosen (or suggested for the first workspace).
    folder: Option<PathBuf>,
    /// The chooser, while it is up.
    chooser: Option<Entity<FolderChooser>>,
}

/// What the Workspaces section shows under its list.
pub(crate) enum Form {
    New(NewForm),
    Rename {
        name: String,
        input: Entity<InputState>,
        error: Option<SharedString>,
    },
    /// The delete question.
    Delete(String),
    /// The chooser for a workspace's new folder.
    Path {
        name: String,
        chooser: Entity<FolderChooser>,
    },
}

/// A change that makes the window enter a vault, waiting for the answer to
/// "discard your changes?" (asked when the draft is dirty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ask {
    Switch(String),
    Path { name: String, path: PathBuf },
    New { name: String, path: PathBuf },
}

impl Ask {
    fn question(&self) -> &'static str {
        match self {
            Ask::Switch(_) => "Switching reopens the workspace; discard your changes?",
            Ask::Path { .. } => "Moving the open workspace reopens it; discard your changes?",
            Ask::New { .. } => "The new workspace opens at once; discard your changes?",
        }
    }

    fn go(&self) -> &'static str {
        match self {
            Ask::Switch(_) => "Discard and switch",
            Ask::Path { .. } => "Discard and move",
            Ask::New { .. } => "Discard and create",
        }
    }
}

/// The Workspaces section's state.
pub(crate) struct Workspaces {
    /// The selected row of `workspace_list` (clamped when drawn).
    pub(crate) selected: usize,
    /// The list's handle: a Tab stop with its own keys.
    pub(crate) focus: FocusHandle,
    pub(crate) form: Option<Form>,
    pub(crate) message: Option<Notice>,
    /// A workspace write (or a new folder, or a deleted workspace's files)
    /// under way: the section waits for it.
    pub(crate) task: Option<Task<()>>,
    pub(crate) ask: Option<Ask>,
    _form_events: Option<Subscription>,
    /// The open chooser's subscription. Its own slot: sharing `_form_events`
    /// would drop the New name field's live validation.
    _chooser_events: Option<Subscription>,
}

impl Workspaces {
    pub(crate) fn new(focus: FocusHandle) -> Self {
        Self {
            selected: 0,
            focus,
            form: None,
            message: None,
            task: None,
            ask: None,
            _form_events: None,
            _chooser_events: None,
        }
    }
}

/// What a reindex is for, and what follows it.
enum Job {
    /// The current workspace's new folder: write `candidate` (the edit
    /// applied), then the edit on the shared settings, then the window
    /// enters the vault.
    Move {
        candidate: Box<AppSettings>,
        edit: WorkspaceEdit,
    },
    /// A Save whose plan needs a reindex: write `after`, as a Save does.
    Save {
        after: Box<AppSettings>,
        theme: Theme,
    },
    /// Indexing's buttons, on the open workspace.
    Fast,
    Full,
}

/// A reindex (and the write after it) under way.
pub(crate) struct Running {
    pub(crate) label: &'static str,
    _task: Task<()>,
}

/// The Indexing section's state, and the panels a reindex shows.
pub(crate) struct Indexing {
    pub(crate) running: Option<Running>,
    pub(crate) confirm_full: bool,
    pub(crate) message: Option<Notice>,
    /// A case conflict: a reindex found one, or the window opened a vault
    /// that has one (`VaultConflict`). Dismissable.
    pub(crate) conflict: Option<SharedString>,
}

impl Indexing {
    pub(crate) fn new() -> Self {
        Self {
            running: None,
            confirm_full: false,
            message: None,
            conflict: None,
        }
    }
}

impl PreferencesView {
    // ---- reading ----------------------------------------------------------

    /// The workspaces, as listed (`workspace_rows`, from the shared
    /// settings in memory: nothing is read from disk).
    pub(crate) fn ws_rows(&self) -> Vec<WorkspaceRow> {
        workspace_rows(&self.settings.read().unwrap_or_else(|e| e.into_inner()))
    }

    fn ws_selected_row(&self) -> Option<WorkspaceRow> {
        let rows = self.ws_rows();
        let ix = self.ws.selected.min(rows.len().saturating_sub(1));
        rows.into_iter().nth(ix)
    }

    /// The section's result line.
    #[cfg(test)]
    pub(crate) fn ws_message(&self) -> Option<&Notice> {
        self.ws.message.as_ref()
    }

    /// The form shown.
    #[cfg(test)]
    pub(crate) fn ws_form(&self) -> Option<&Form> {
        self.ws.form.as_ref()
    }

    /// The question asked before the window enters a vault.
    #[cfg(test)]
    pub(crate) fn ws_ask(&self) -> Option<&Ask> {
        self.ws.ask.as_ref()
    }

    /// The selected row's name.
    #[cfg(test)]
    pub(crate) fn ws_selected(&self) -> Option<String> {
        self.ws_selected_row().map(|r| r.name)
    }

    /// The New form's name field.
    #[cfg(test)]
    pub(crate) fn ws_new_input(&self) -> Option<Entity<InputState>> {
        match self.ws.form.as_ref()? {
            Form::New(f) => Some(f.input.clone()),
            _ => None,
        }
    }

    /// The New form's name problem.
    #[cfg(test)]
    pub(crate) fn ws_new_error(&self) -> Option<String> {
        match self.ws.form.as_ref()? {
            Form::New(f) => f.error.as_ref().map(|e| e.to_string()),
            _ => None,
        }
    }

    /// The chooser of a Change path, while up.
    #[cfg(test)]
    pub(crate) fn ws_path_chooser(&self) -> Option<Entity<FolderChooser>> {
        match self.ws.form.as_ref()? {
            Form::Path { chooser, .. } => Some(chooser.clone()),
            _ => None,
        }
    }

    /// The Rename form's field.
    #[cfg(test)]
    pub(crate) fn ws_rename_input(&self) -> Option<Entity<InputState>> {
        match self.ws.form.as_ref()? {
            Form::Rename { input, .. } => Some(input.clone()),
            _ => None,
        }
    }

    /// The Indexing section's result line.
    #[cfg(test)]
    pub(crate) fn index_message(&self) -> Option<&Notice> {
        self.index.message.as_ref()
    }

    /// The case conflict shown.
    #[cfg(test)]
    pub(crate) fn conflict(&self) -> Option<&str> {
        self.index.conflict.as_deref()
    }

    /// Whether a reindex (or the write after it) runs.
    #[cfg(test)]
    pub(crate) fn reindexing(&self) -> bool {
        self.index.running.is_some()
    }

    /// Whether the full reindex question is asked.
    #[cfg(test)]
    pub(crate) fn confirm_full(&self) -> bool {
        self.index.confirm_full
    }

    /// Whether a workspace change is being written.
    #[cfg(test)]
    pub(crate) fn ws_writing(&self) -> bool {
        self.ws.task.is_some()
    }

    /// Something runs that every control waits for.
    pub(crate) fn busy(&self) -> bool {
        self.saving.is_some() || self.index.running.is_some() || self.ws.task.is_some()
    }

    /// Whether one of the section's text fields has the keyboard (the New
    /// name, the Rename field, the chooser's new folder).
    pub(crate) fn ws_field_focused(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        let focused =
            |input: &Entity<InputState>| input.read(cx).focus_handle(cx).is_focused(window);
        match self.ws.form.as_ref() {
            Some(Form::New(f)) => {
                focused(&f.input)
                    || f.chooser
                        .as_ref()
                        .is_some_and(|c| c.read(cx).new_folder_focused(window, cx))
            }
            Some(Form::Rename { input, .. }) => focused(input),
            Some(Form::Path { chooser, .. }) => chooser.read(cx).new_folder_focused(window, cx),
            _ => false,
        }
    }

    /// The folder chooser up in the section, if one is.
    pub(crate) fn ws_chooser(&self) -> Option<Entity<FolderChooser>> {
        match self.ws.form.as_ref()? {
            Form::New(f) => f.chooser.clone(),
            Form::Path { chooser, .. } => Some(chooser.clone()),
            _ => None,
        }
    }

    // ---- the list ---------------------------------------------------------

    /// Select row `ix` of the list (a click), giving it the keyboard.
    pub(crate) fn ws_select(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.ws.selected = ix;
        window.focus(&self.ws.focus, cx);
        cx.notify();
    }

    /// Up / Down in the list (clamped).
    pub(crate) fn ws_move(&mut self, down: bool, cx: &mut Context<Self>) {
        let count = self.ws_rows().len();
        if count == 0 {
            return;
        }
        let ix = self.ws.selected.min(count - 1);
        self.ws.selected = if down {
            (ix + 1).min(count - 1)
        } else {
            ix.saturating_sub(1)
        };
        cx.notify();
    }

    /// Select the row of workspace `name`, if listed.
    fn ws_select_name(&mut self, name: &str) {
        if let Some(ix) = self.ws_rows().iter().position(|r| r.name == name) {
            self.ws.selected = ix;
        }
    }

    // ---- switch -----------------------------------------------------------

    /// `Switch` (Enter on a row): the window moves onto the selected
    /// workspace and Preferences close. Not on the current one; asks first
    /// when the draft is dirty.
    pub(crate) fn ws_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(name) = self
            .ws_selected_row()
            .and_then(|r| r.switch_target().map(str::to_string))
        else {
            return;
        };
        self.go_or_ask(Ask::Switch(name), window, cx);
    }

    /// Do `ask`'s change now when the draft is clean, else ask first.
    fn go_or_ask(&mut self, ask: Ask, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dirty() {
            self.ws.ask = Some(ask);
            window.focus(&self.focus, cx);
            cx.notify();
        } else {
            self.ask_do(ask, window, cx);
        }
    }

    /// The question's "keep editing": nothing happens.
    pub(crate) fn ask_keep(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ws.ask = None;
        window.focus(&self.ws.focus, cx);
        cx.notify();
    }

    /// The question's "discard and …": the change goes ahead, the draft
    /// with it.
    pub(crate) fn ask_go(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ask) = self.ws.ask.take() {
            self.ask_do(ask, window, cx);
        }
        cx.notify();
    }

    fn ask_do(&mut self, ask: Ask, window: &mut Window, cx: &mut Context<Self>) {
        match ask {
            Ask::Switch(name) => cx.emit(PreferencesEvent::Switch {
                theme: self.original.clone(),
                name,
            }),
            Ask::Path { name, path } => self.run_move(name, path, window, cx),
            Ask::New { name, path } => self.create(name, path, window, cx),
        }
    }

    // ---- new --------------------------------------------------------------

    /// `New workspace…` (n): the New form, its name field focused. With no
    /// workspace yet, the first one's name and folder are suggested.
    pub(crate) fn ws_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let first = self.ws_rows().is_empty();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Workspace name"));
        if first {
            input.update(cx, |i, cx| i.set_value(FIRST_WORKSPACE_NAME, window, cx));
        }
        let events = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.ws_name_changed(cx);
            }
        });
        let handle = input.read(cx).focus_handle(cx);
        self.ws.form = Some(Form::New(NewForm {
            input,
            error: None,
            folder: first
                .then(AppSettings::default_workspace_suggestion)
                .flatten(),
            chooser: None,
        }));
        self.ws._form_events = Some(events);
        self.ws.message = None;
        window.focus(&handle, cx);
        cx.notify();
    }

    /// The New name changed: checked at once.
    fn ws_name_changed(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        let Some(Form::New(form)) = self.ws.form.as_mut() else {
            return;
        };
        let typed = form.input.read(cx).value().to_string();
        let checked = prefs::check_new_workspace_name(
            &settings.read().unwrap_or_else(|e| e.into_inner()),
            &typed,
        );
        form.error = checked.err().map(|e| e.to_string().into());
        cx.notify();
    }

    /// Where a chooser starts: `path` when it is an absolute folder path,
    /// else its parent, else the home folder.
    fn chooser_start(path: Option<&PathBuf>, parent: bool) -> Option<SystemPath> {
        let from = path.and_then(|p| SystemPath::try_absolute(p).ok());
        let from = if parent {
            from.and_then(|p| p.parent())
        } else {
            from
        };
        from.or_else(|| system::browse_root().ok())
    }

    fn open_chooser(
        &mut self,
        start: SystemPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<FolderChooser> {
        let host = self.focus.clone();
        let chooser = cx.new(|cx| FolderChooser::new(CHOOSER_PREFIX, start, host, window, cx));
        self.ws._chooser_events = Some(cx.subscribe_in(&chooser, window, Self::on_chooser_event));
        window.focus(&self.focus, cx);
        chooser
    }

    /// `Choose folder…` in the New form: the chooser, on the folder's
    /// parent (or the home folder).
    pub(crate) fn ws_choose_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = match self.ws.form.as_ref() {
            Some(Form::New(form)) => Self::chooser_start(form.folder.as_ref(), true),
            _ => return,
        };
        let Some(start) = start else {
            self.ws.message = Some(Notice::error(
                "Couldn't find a folder to start browsing from.",
            ));
            cx.notify();
            return;
        };
        let chooser = self.open_chooser(start, window, cx);
        if let Some(Form::New(form)) = self.ws.form.as_mut() {
            form.chooser = Some(chooser);
        }
        cx.notify();
    }

    fn on_chooser_event(
        &mut self,
        _: &Entity<FolderChooser>,
        event: &FolderChooserEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (self.ws.form.as_mut(), event) {
            (Some(Form::New(form)), FolderChooserEvent::Chosen(dir)) => {
                form.folder = Some(dir.clone().into_path_buf());
                form.chooser = None;
                let handle = form.input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
            (Some(Form::New(form)), FolderChooserEvent::Cancelled) => {
                form.chooser = None;
                let handle = form.input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
            (Some(Form::Path { name, .. }), FolderChooserEvent::Chosen(dir)) => {
                let name = name.clone();
                let path = dir.clone().into_path_buf();
                self.ws.form = None;
                window.focus(&self.ws.focus, cx);
                self.path_chosen(name, path, window, cx);
            }
            (Some(Form::Path { .. }), FolderChooserEvent::Cancelled) => {
                self.ws_close_form(window, cx)
            }
            _ => {}
        }
        cx.notify();
    }

    /// `Create` (Enter in the name field once a folder is chosen; before
    /// that, Enter opens the chooser).
    pub(crate) fn ws_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(Form::New(form)) = self.ws.form.as_ref() else {
            return;
        };
        let typed = form.input.read(cx).value().to_string();
        let folder = form.folder.clone();
        let checked = prefs::check_new_workspace_name(
            &self.settings.read().unwrap_or_else(|e| e.into_inner()),
            &typed,
        );
        let name = match checked {
            Ok(name) => name,
            Err(e) => {
                if let Some(Form::New(form)) = self.ws.form.as_mut() {
                    form.error = Some(e.to_string().into());
                }
                cx.notify();
                return;
            }
        };
        let Some(path) = folder else {
            return self.ws_choose_folder(window, cx);
        };
        if !self.has_vault() {
            // No current workspace: the new one becomes it, and the window
            // enters it.
            self.go_or_ask(Ask::New { name, path }, window, cx);
        } else {
            self.create(name, path, window, cx);
        }
    }

    /// Make the folder when needed (through core), then register the
    /// workspace ([`Self::commit_edit`]).
    fn create(&mut self, name: String, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.ws.message = None;
        self.ws.task = Some(cx.spawn_in(window, async move |this, cx| {
            let made = blocking(move || prefs::prepare_workspace_folder(&path))
                .await
                .and_then(|r| r);
            this.update_in(cx, |view, window, cx| match made {
                Ok(path) => {
                    view.ws.task = None;
                    view.commit_edit(WorkspaceEdit::Add { name, path }, window, cx);
                }
                Err(e) => {
                    view.ws.task = None;
                    view.ws.message = Some(Notice::error(format!("Couldn't make the folder: {e}")));
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    // ---- rename -----------------------------------------------------------

    /// `Rename…` (F2, r): the selected workspace's name in a field.
    pub(crate) fn ws_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(row) = self.ws_selected_row() else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Workspace name"));
        input.update(cx, |i, cx| i.set_value(row.name.clone(), window, cx));
        let handle = input.read(cx).focus_handle(cx);
        self.ws.form = Some(Form::Rename {
            name: row.name,
            input,
            error: None,
        });
        self.ws._form_events = None;
        self.ws._chooser_events = None;
        self.ws.message = None;
        window.focus(&handle, cx);
        cx.notify();
    }

    /// `Rename` (Enter in the field).
    pub(crate) fn ws_rename_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(Form::Rename { name, input, .. }) = self.ws.form.as_ref() else {
            return;
        };
        let edit = WorkspaceEdit::Rename {
            old: name.clone(),
            new: input.read(cx).value().to_string(),
        };
        self.commit_edit(edit, window, cx);
    }

    // ---- delete -----------------------------------------------------------

    /// `Delete…` (Delete, d): the question, or why not (the current one).
    pub(crate) fn ws_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(row) = self.ws_selected_row() else {
            return;
        };
        if row.current {
            let refused = WorkspaceError::IsCurrent(row.name);
            self.ws.message = Some(Notice::error(format!(
                "{refused} Switch to another one first."
            )));
            self.ws.form = None;
        } else {
            self.ws.form = Some(Form::Delete(row.name));
            self.ws.message = None;
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    /// The question's `Delete` (Enter).
    pub(crate) fn ws_delete_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Form::Delete(name)) = self.ws.form.as_ref() else {
            return;
        };
        let edit = WorkspaceEdit::Delete { name: name.clone() };
        self.commit_edit(edit, window, cx);
    }

    // ---- change path ------------------------------------------------------

    /// `Change path…` (b): the chooser, on the workspace's folder.
    pub(crate) fn ws_change_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(row) = self.ws_selected_row() else {
            return;
        };
        let Some(start) = Self::chooser_start(Some(&row.path), false) else {
            return;
        };
        let chooser = self.open_chooser(start, window, cx);
        self.ws.form = Some(Form::Path {
            name: row.name,
            chooser,
        });
        self.ws.message = None;
        cx.notify();
    }

    /// A workspace's new folder was chosen: saved at once for another
    /// workspace; for the current one, the vault-path reindex flow.
    fn path_chosen(
        &mut self,
        name: String,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut candidate = self.initial();
        let edit = WorkspaceEdit::SetPath {
            name: name.clone(),
            path: path.clone(),
        };
        match edit.apply(&mut candidate) {
            Err(e) => self.ws.message = Some(Notice::error(e.to_string())),
            Ok(out) if !out.changed => {
                self.ws.message = Some(Notice::info(format!("'{name}' already uses that folder.")))
            }
            Ok(out) if out.current => self.go_or_ask(Ask::Path { name, path }, window, cx),
            Ok(_) => self.commit_edit(edit, window, cx),
        }
        cx.notify();
    }

    /// The current workspace's new folder: indexed first, on a throwaway
    /// vault; saved and entered only when that worked.
    fn run_move(
        &mut self,
        name: String,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut candidate = self.initial();
        let edit = WorkspaceEdit::SetPath { name, path };
        if let Err(e) = edit.apply(&mut candidate) {
            self.ws.message = Some(Notice::error(e.to_string()));
            cx.notify();
            return;
        }
        let Some((workspace, cache)) = prefs::reindex_target(&candidate) else {
            self.ws.message = Some(Notice::error(prefs::SaveError::NoWorkspace.to_string()));
            cx.notify();
            return;
        };
        self.run_reindex(
            Job::Move {
                candidate: Box::new(candidate),
                edit,
            },
            workspace,
            cache,
            window,
            cx,
        );
    }

    // ---- forms ------------------------------------------------------------

    /// Esc in a form, its `Cancel`: the form goes, the list has the keyboard.
    pub(crate) fn ws_close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ws.form = None;
        self.ws._form_events = None;
        self.ws._chooser_events = None;
        window.focus(&self.ws.focus, cx);
        cx.notify();
    }

    /// Enter in one of the section's fields.
    pub(crate) fn ws_field_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(chooser) = self.ws_chooser()
            && chooser.read(cx).new_folder_focused(window, cx)
        {
            chooser.update(cx, |c, cx| c.create_folder(window, cx));
            return;
        }
        match self.ws.form.as_ref() {
            Some(Form::New(_)) => self.ws_create(window, cx),
            Some(Form::Rename { .. }) => self.ws_rename_submit(window, cx),
            _ => {}
        }
    }

    /// Esc in one of the section's fields.
    pub(crate) fn ws_field_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(chooser) = self.ws_chooser()
            && chooser.read(cx).new_folder_focused(window, cx)
        {
            chooser.update(cx, |c, cx| c.close_new_folder(window, cx));
            return;
        }
        self.ws_close_form(window, cx);
    }

    // ---- the write --------------------------------------------------------

    /// Apply `edit` to a copy of the settings and write it; once written,
    /// apply it to the shared settings too ([module docs](self)). A refused
    /// edit or a failed write is shown, and nothing changes.
    fn commit_edit(&mut self, edit: WorkspaceEdit, window: &mut Window, cx: &mut Context<Self>) {
        let mut candidate = self.initial();
        match edit.apply(&mut candidate) {
            Err(e) => {
                self.edit_refused(e.to_string(), cx);
                return;
            }
            Ok(out) if !out.changed => {
                self.ws_close_form(window, cx);
                return;
            }
            Ok(_) => {}
        }
        let write = reopen::write_settings(self.store.clone(), move || candidate);
        self.ws.message = None;
        self.ws.task = Some(cx.spawn_in(window, async move |this, cx| {
            let written = write.await;
            this.update_in(cx, |view, window, cx| {
                view.edit_written(edit, written, window, cx)
            })
            .ok();
        }));
        cx.notify();
    }

    /// A refused edit: shown in its form when it has a field, else in the
    /// section.
    fn edit_refused(&mut self, text: String, cx: &mut Context<Self>) {
        match self.ws.form.as_mut() {
            Some(Form::New(form)) => form.error = Some(text.into()),
            Some(Form::Rename { error, .. }) => *error = Some(text.into()),
            _ => self.ws.message = Some(Notice::error(text)),
        }
        cx.notify();
    }

    /// A workspace write failed: logged, and said in the section.
    fn write_failed(&mut self, e: &str) {
        tracing::warn!("could not save the workspaces: {e}");
        self.ws.message = Some(Notice::error(format!("Couldn't save the workspaces: {e}")));
    }

    fn edit_written(
        &mut self,
        edit: WorkspaceEdit,
        written: Result<(), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ws.task = None;
        if let Err(e) = written {
            self.write_failed(&e);
            cx.notify();
            return;
        }
        let applied = {
            let mut settings = self.settings.write().unwrap_or_else(|e| e.into_inner());
            edit.apply(&mut settings)
        };
        // A new draft baseline: workspace edits never touch the draft's
        // fields, but the cached answer is worked out again.
        self.dirty.set(None);
        self.ws.form = None;
        self.ws._form_events = None;
        self.ws._chooser_events = None;
        window.focus(&self.ws.focus, cx);
        let out = match applied {
            Ok(out) => out,
            Err(e) => {
                self.ws.message = Some(Notice::error(e.to_string()));
                cx.notify();
                return;
            }
        };
        match edit {
            WorkspaceEdit::Add { name, .. } => {
                let name = name.trim().to_lowercase();
                self.ws_select_name(&name);
                self.ws.message = Some(Notice::info(format!("Workspace '{name}' created.")));
                if out.current {
                    cx.emit(PreferencesEvent::Reopen {
                        theme: self.original.clone(),
                    });
                }
            }
            WorkspaceEdit::Rename { new, .. } => {
                let name = new.trim().to_lowercase();
                self.ws_select_name(&name);
                self.ws.message = Some(Notice::info(format!("Renamed to '{name}'.")));
                if out.current {
                    cx.emit(PreferencesEvent::Renamed(name));
                }
            }
            WorkspaceEdit::SetPath { name, .. } => {
                self.ws_select_name(&name);
                self.ws.message = Some(Notice::info(format!("'{name}' now uses the new folder.")));
            }
            WorkspaceEdit::Delete { name } => {
                self.ws.selected = self.ws.selected.saturating_sub(1);
                self.ws.message = Some(Notice::info(format!("Workspace '{name}' deleted.")));
                if let Some((index, history)) = out.artifacts {
                    self.remove_artifacts(name, index, history, window, cx);
                }
            }
        }
        cx.notify();
    }

    /// A deleted workspace's index and history go, off the UI thread and
    /// outside the settings lock; what stays behind is said.
    fn remove_artifacts(
        &mut self,
        name: String,
        index: IndexFile,
        history: kimun_notes::settings::history::HistoryFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ws.task = Some(cx.spawn_in(window, async move |this, cx| {
            let label = name.clone();
            let left = blocking(move || prefs::remove_workspace_artifacts(&name, &index, &history))
                .await
                .unwrap_or_else(|e| vec![e]);
            this.update(cx, |view, cx| {
                view.ws.task = None;
                if !left.is_empty() {
                    view.ws.message = Some(Notice::error(format!(
                        "Workspace '{label}' deleted, but these files stayed behind: {}",
                        left.join(", ")
                    )));
                }
                cx.notify();
            })
            .ok();
        }));
    }

    // ---- reindex ----------------------------------------------------------

    /// A Save whose plan needs the vault indexed first (the vault moved):
    /// the same flow as a moved workspace; `after` is written once indexed.
    pub(crate) fn save_after_reindex(
        &mut self,
        after: AppSettings,
        theme: Theme,
        workspace: SystemPath,
        cache: Option<IndexFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_reindex(
            Job::Save {
                after: Box::new(after),
                theme,
            },
            workspace,
            cache,
            window,
            cx,
        );
    }

    /// `Fast reindex`.
    pub(crate) fn fast_reindex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reindex_open(Job::Fast, window, cx);
    }

    /// `Full reindex…`: asks first.
    pub(crate) fn full_reindex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || !self.has_vault() {
            return;
        }
        self.index.confirm_full = true;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// The question's `Rebuild` (Enter).
    pub(crate) fn full_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.index.confirm_full = false;
        self.reindex_open(Job::Full, window, cx);
        cx.notify();
    }

    /// The question's `Cancel` (Esc): nothing happens.
    pub(crate) fn full_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.index.confirm_full = false;
        window.focus(&self.sections_focus, cx);
        cx.notify();
    }

    fn has_vault(&self) -> bool {
        prefs::has_vault(&self.settings.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// Reindex the open workspace (`job` Fast or Full).
    fn reindex_open(&mut self, job: Job, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let target =
            prefs::reindex_target(&self.settings.read().unwrap_or_else(|e| e.into_inner()));
        let Some((workspace, cache)) = target else {
            self.index.message = Some(Notice::error(NEEDS_WORKSPACE));
            cx.notify();
            return;
        };
        self.run_reindex(job, workspace, cache, window, cx);
    }

    /// Run the reindex `job` asks for on a throwaway vault, off the UI
    /// thread; the progress panel holds the view meanwhile.
    fn run_reindex(
        &mut self,
        job: Job,
        workspace: SystemPath,
        cache: Option<IndexFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let label = match job {
            Job::Move { .. } | Job::Save { .. } => INDEXING_NEW,
            Job::Fast | Job::Full => INDEXING,
        };
        let fast = matches!(job, Job::Fast);
        self.index.message = None;
        self.index.conflict = None;
        self.ws.message = None;
        self.message = None;
        let task = cx.spawn_in(window, async move |this, cx| {
            let outcome = if fast {
                match tokio::spawn(prefs::fast_reindex(workspace, cache)).await {
                    Ok(Ok(d)) => ReindexOutcome::Done(d),
                    Ok(Err(e)) => ReindexOutcome::Failed(e),
                    Err(e) => ReindexOutcome::Failed(e.to_string()),
                }
            } else {
                match tokio::spawn(prefs::reindex_new_vault(workspace, cache)).await {
                    Ok(outcome) => outcome,
                    Err(e) => ReindexOutcome::Failed(e.to_string()),
                }
            };
            this.update_in(cx, |view, window, cx| {
                view.reindexed(job, outcome, window, cx)
            })
            .ok();
        });
        self.index.running = Some(Running { label, _task: task });
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn reindexed(
        &mut self,
        job: Job,
        outcome: ReindexOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.index.running = None;
        let duration = match outcome {
            ReindexOutcome::Done(duration) => duration,
            ReindexOutcome::Conflict(text) => {
                tracing::warn!("{text}");
                self.index.conflict = Some(text.into());
                self.ws.form = None;
                window.focus(&self.sections_focus, cx);
                cx.notify();
                return;
            }
            ReindexOutcome::Failed(text) => {
                tracing::warn!("reindex failed: {text}");
                let problem = Notice::error(format!("Couldn't index the workspace: {text}"));
                match job {
                    Job::Fast | Job::Full => self.index.message = Some(problem),
                    Job::Move { .. } => self.ws.message = Some(problem),
                    Job::Save { .. } => self.message = Some(problem.text),
                }
                window.focus(&self.sections_focus, cx);
                cx.notify();
                return;
            }
        };
        match job {
            Job::Fast | Job::Full => {
                // The open vault is indexed now: a pending `needs_indexing`
                // (a changed folder, say) is done, as in the TUI.
                self.settings
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .report_indexed();
                self.index.message = Some(Notice::info(prefs::indexed_in(duration)));
                window.focus(&self.sections_focus, cx);
                cx.emit(PreferencesEvent::Reindexed);
            }
            Job::Move { candidate, edit } => {
                let mut candidate = *candidate;
                candidate.report_indexed();
                let write = reopen::write_settings(self.store.clone(), move || candidate);
                let task = cx.spawn_in(window, async move |this, cx| {
                    let written = write.await;
                    this.update_in(cx, |view, window, cx| {
                        view.move_written(edit, written, window, cx)
                    })
                    .ok();
                });
                self.index.running = Some(Running {
                    label: INDEXING_NEW,
                    _task: task,
                });
            }
            Job::Save { after, theme } => {
                let mut after = *after;
                after.report_indexed();
                self.write_save(after, theme, true, window, cx);
            }
        }
        cx.notify();
    }

    /// The moved workspace's settings are on disk: the shared settings take
    /// the move (indexed), and the window enters the vault there.
    fn move_written(
        &mut self,
        edit: WorkspaceEdit,
        written: Result<(), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.index.running = None;
        if let Err(e) = written {
            self.write_failed(&e);
            window.focus(&self.sections_focus, cx);
            cx.notify();
            return;
        }
        let applied = {
            let mut settings = self.settings.write().unwrap_or_else(|e| e.into_inner());
            let applied = edit.apply(&mut settings);
            settings.report_indexed();
            applied
        };
        if let Err(e) = applied {
            // Written, but the shared settings refuse it: entering the vault
            // would disagree with the disk, so say so and stay.
            tracing::warn!("could not apply the moved workspace: {e}");
            self.ws.message = Some(Notice::error(e.to_string()));
            window.focus(&self.sections_focus, cx);
            cx.notify();
            return;
        }
        cx.emit(PreferencesEvent::Reopen {
            theme: self.original.clone(),
        });
        cx.notify();
    }

    /// Show the case conflict `text` (the vault's own words) in its panel.
    pub(crate) fn show_conflict(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.index.conflict = Some(text.into());
        self.show_section(super::Section::Workspaces, window, cx);
    }

    /// The conflict panel's `Dismiss`.
    pub(crate) fn dismiss_conflict(&mut self, cx: &mut Context<Self>) {
        self.index.conflict = None;
        cx.notify();
    }

    /// The window's close while something runs: refused, and said.
    pub(crate) fn close_refused(&mut self, cx: &mut Context<Self>) {
        self.message =
            Some("Wait until the indexing or the save is done, then close again.".into());
        cx.notify();
    }
}

// ---- Rendering ---------------------------------------------------------------

/// A result line under a section's controls.
fn notice_line(id: &'static str, notice: &Notice, t: &Theme) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .role(Role::Label)
        .aria_label(notice.text.clone())
        .text_sm()
        .text_color(if notice.error {
            theme::error(t)
        } else {
            theme::foreground(t)
        })
        .child(notice.text.clone())
        .into_any_element()
}

/// A row of buttons, wrapping in a narrow window.
fn button_row() -> gpui_kit::Div {
    div().flex().flex_row().flex_wrap().items_center().gap_2()
}

impl PreferencesView {
    fn render_ws_row(
        &self,
        ix: usize,
        row: &WorkspaceRow,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let hover = theme::hover_background(t);
        let label = if row.current {
            format!("{}, current", row.name)
        } else {
            row.name.clone()
        };
        div()
            .id(workspace_row_id(&row.name))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(SharedString::from(label))
            .aria_selected(selected)
            .flex()
            .flex_col()
            .w_full()
            .px_2()
            .py_1()
            .rounded_md()
            .when(selected, |r| r.bg(theme::selected_background(t)))
            .when(!selected, |r| r.hover(move |style| style.bg(hover)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .text_color(theme::foreground(t))
                    .child(
                        div()
                            .truncate()
                            .when(selected, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .child(row.name.clone()),
                    )
                    .when(row.current, |d| {
                        d.child(div().text_xs().text_color(theme::muted(t)).child("current"))
                    }),
            )
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(theme::muted(t))
                    .child(row.folder.clone()),
            )
            .on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| this.ws_select(ix, window, cx)),
            )
            .into_any_element()
    }

    fn render_new_form(&self, form: &NewForm, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        if let Some(chooser) = form.chooser.clone() {
            return div()
                .flex()
                .flex_col()
                .gap_3()
                .child(chooser.clone())
                .child(button_row().children(folder_chooser::actions(&chooser, cx)))
                .child(note(FolderChooser::HINTS, t))
                .into_any_element();
        }
        let busy = self.busy();
        let folder = form
            .folder
            .as_ref()
            .map(system::path_to_string)
            .unwrap_or_else(|| "No folder chosen yet".to_string());
        let can_create = form.error.is_none() && form.folder.is_some() && !busy;
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme::divider(t))
            .child(super::label("New workspace", t))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(note("Name", t))
                    .child(
                        Input::new(&form.input)
                            .id(WS_NAME_ID)
                            .aria_label("New workspace name")
                            .disabled(busy)
                            .bg(theme::background(t))
                            .border_color(if form.error.is_some() {
                                theme::error(t)
                            } else {
                                theme::divider(t)
                            })
                            .text_color(theme::foreground(t)),
                    )
                    .when_some(form.error.clone(), |col, error| {
                        col.child(
                            div()
                                .id(WS_NAME_ERROR_ID)
                                .test_support()
                                .role(Role::Label)
                                .aria_label(error.clone())
                                .text_xs()
                                .text_color(theme::error(t))
                                .child(error),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(note("Folder", t))
                    .child(
                        div()
                            .id(WS_FOLDER_ID)
                            .test_support()
                            .role(Role::Label)
                            .aria_label(SharedString::from(folder.clone()))
                            .truncate()
                            .text_sm()
                            .text_color(theme::foreground(t))
                            .child(folder),
                    ),
            )
            .child(
                button_row()
                    .child(
                        Button::new(WS_CHOOSE_ID)
                            .label("Choose folder…")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.ws_choose_folder(window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new(WS_FORM_CANCEL_ID)
                            .label("Cancel")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.ws_close_form(window, cx)
                            })),
                    )
                    .child(
                        Button::new(WS_CREATE_ID)
                            .primary()
                            .label("Create")
                            .loading(self.ws.task.is_some())
                            .disabled(!can_create)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.ws_create(window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_rename_form(
        &self,
        input: &Entity<InputState>,
        error: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let busy = self.busy();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme::divider(t))
            .child(super::label("Rename workspace", t))
            .child(
                Input::new(input)
                    .id(WS_RENAME_INPUT_ID)
                    .aria_label("Workspace name")
                    .disabled(busy)
                    .bg(theme::background(t))
                    .border_color(if error.is_some() {
                        theme::error(t)
                    } else {
                        theme::divider(t)
                    })
                    .text_color(theme::foreground(t)),
            )
            .when_some(error, |col, error| {
                col.child(div().text_xs().text_color(theme::error(t)).child(error))
            })
            .child(
                button_row()
                    .child(div().flex_1())
                    .child(
                        Button::new(WS_FORM_CANCEL_ID)
                            .label("Cancel")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.ws_close_form(window, cx)
                            })),
                    )
                    .child(
                        Button::new(WS_RENAME_SAVE_ID)
                            .primary()
                            .label("Rename")
                            .loading(self.ws.task.is_some())
                            .disabled(busy)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.ws_rename_submit(window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_delete_question(&self, name: &str, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let busy = self.busy();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme::error(t))
            .child(para(delete_question(name), t))
            .child(note("Enter: delete · Esc: keep it", t))
            .child(
                button_row()
                    .child(div().flex_1())
                    .child(
                        Button::new(WS_DELETE_NO_ID)
                            .label("Keep it")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.ws_close_form(window, cx)
                            })),
                    )
                    .child(
                        Button::new(WS_DELETE_YES_ID)
                            .danger()
                            .label("Delete workspace")
                            .loading(self.ws.task.is_some())
                            .disabled(busy)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.ws_delete_confirm(window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    /// The Workspaces section.
    pub(crate) fn render_workspaces(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let rows = self.ws_rows();
        let busy = self.busy();
        let help = note(
            "Changes here are saved as you make them. Save and Cancel are for the other \
             sections.",
            t,
        );
        let message = self
            .ws
            .message
            .as_ref()
            .map(|n| notice_line(WS_MESSAGE_ID, n, t));
        // The chooser of a Change path takes the section's room.
        if let Some(Form::Path { name, chooser }) = self.ws.form.as_ref() {
            return div()
                .flex()
                .flex_col()
                .gap_3()
                .child(para(format!("Choose the folder for '{name}'."), t))
                .child(chooser.clone())
                .child(button_row().children(folder_chooser::actions(chooser, cx)))
                .child(note(FolderChooser::HINTS, t))
                .children(message)
                .into_any_element();
        }
        if rows.is_empty() {
            let form = match self.ws.form.as_ref() {
                Some(Form::New(form)) => Some(self.render_new_form(form, cx)),
                _ => None,
            };
            // Tracks the list's handle, as the list does: closing a form
            // returns focus to it, and it must land on an element.
            return div()
                .track_focus(&self.ws.focus)
                .flex()
                .flex_col()
                .gap_3()
                .child(help)
                .child(
                    div()
                        .id(WS_EMPTY_ID)
                        .test_support()
                        .role(Role::Label)
                        .aria_label("No workspaces yet.")
                        .child(para("No workspaces yet.", t)),
                )
                .when(form.is_none(), |col| {
                    col.child(
                        button_row()
                            .child(
                                Button::new(WS_NEW_ID)
                                    .primary()
                                    .label("New workspace…")
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.ws_new(window, cx)
                                    })),
                            )
                            .child(
                                Button::new(WS_SETUP_ID)
                                    .label("Run guided setup…")
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        cx.emit(PreferencesEvent::RunSetup {
                                            theme: this.original.clone(),
                                        })
                                    })),
                            ),
                    )
                })
                .children(form)
                .children(message)
                .into_any_element();
        }
        let selected = self.ws.selected.min(rows.len() - 1);
        let current_selected = rows[selected].current;
        let focused = self.ws.focus.is_focused(window);
        let list_rows: Vec<AnyElement> = rows
            .iter()
            .enumerate()
            .map(|(ix, row)| self.render_ws_row(ix, row, ix == selected, cx))
            .collect();
        let list = div()
            .id(WS_LIST_ID)
            .test_support()
            .role(Role::ListBox)
            .aria_label("Workspaces")
            .track_focus(&self.ws.focus)
            .flex()
            .flex_col()
            .gap_1()
            .p_1()
            .rounded_md()
            .border_1()
            .border_color(if focused {
                theme::focus_border(t)
            } else {
                theme::divider(t)
            })
            .children(list_rows);
        let form = match self.ws.form.as_ref() {
            Some(Form::New(form)) => Some(self.render_new_form(form, cx)),
            Some(Form::Rename { input, error, .. }) => {
                Some(self.render_rename_form(input, error.clone(), cx))
            }
            Some(Form::Delete(name)) => Some(self.render_delete_question(name, cx)),
            _ => None,
        };
        let actions = button_row()
            .child(
                Button::new(WS_SWITCH_ID)
                    .label("Switch")
                    .disabled(busy || current_selected)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.ws_switch(window, cx)),
                    ),
            )
            .child(
                Button::new(WS_RENAME_ID)
                    .label("Rename…")
                    .disabled(busy)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.ws_rename(window, cx)),
                    ),
            )
            .child(
                Button::new(WS_PATH_ID)
                    .label("Change path…")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.ws_change_path(window, cx)
                    })),
            )
            .child(
                Button::new(WS_DELETE_ID)
                    .label("Delete…")
                    .disabled(busy)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.ws_delete(window, cx)),
                    ),
            )
            .child(div().flex_1())
            .child(
                Button::new(WS_NEW_ID)
                    .label("New workspace…")
                    .disabled(busy)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.ws_new(window, cx)),
                    ),
            );
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(help)
            .child(list)
            .when(form.is_none(), |col| col.child(actions))
            .children(form)
            .children(message)
            .child(note(
                "↑/↓: select · Enter: switch · F2/R: rename · B: change path · Del/D: delete · \
                 N: new",
                t,
            ))
            .into_any_element()
    }

    /// The Indexing section.
    pub(crate) fn render_indexing(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let inert = !self.has_vault();
        let busy = self.busy();
        let question = self.index.confirm_full.then(|| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(theme::divider(t))
                .child(para(FULL_QUESTION, t))
                .child(note("Enter: rebuild · Esc: cancel", t))
                .child(
                    button_row()
                        .child(div().flex_1())
                        .child(
                            Button::new(FULL_NO_ID)
                                .label("Cancel")
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.full_cancel(window, cx)
                                })),
                        )
                        .child(
                            Button::new(FULL_YES_ID)
                                .primary()
                                .label("Rebuild the index")
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.full_confirm(window, cx)
                                })),
                        ),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(note(
                "Fast reindex looks for notes changed outside kimün. Full reindex rebuilds \
                 the whole index from the notes on disk. FILES, LINKS, FIND and TAGS show the \
                 result when it is done.",
                t,
            ))
            .when(question.is_none(), |col| {
                col.child(
                    button_row()
                        .child(
                            Button::new(FAST_ID)
                                .label("Fast reindex")
                                .disabled(inert || busy)
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.fast_reindex(window, cx)
                                })),
                        )
                        .child(
                            Button::new(FULL_ID)
                                .label("Full reindex…")
                                .disabled(inert || busy)
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.full_reindex(window, cx)
                                })),
                        ),
                )
            })
            .children(question)
            .when(inert, |col| {
                col.child(
                    div()
                        .id(INDEX_INERT_ID)
                        .test_support()
                        .role(Role::Label)
                        .aria_label(NEEDS_WORKSPACE)
                        .child(note(NEEDS_WORKSPACE, t)),
                )
            })
            .children(
                self.index
                    .message
                    .as_ref()
                    .map(|n| notice_line(INDEX_MESSAGE_ID, n, t)),
            )
            .into_any_element()
    }

    /// The progress panel while a reindex runs.
    pub(crate) fn render_progress(
        &self,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        div()
            .id(PROGRESS_ID)
            .test_support()
            .role(Role::Status)
            .aria_label(label)
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme::divider(t))
            .child(Spinner::new().small())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(para(label, t))
                    .child(note("The other controls wait until it is done.", t)),
            )
            .into_any_element()
    }

    /// The case-conflict panel, over any section.
    pub(crate) fn render_conflict(&self, text: SharedString, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        div()
            .id(CONFLICT_ID)
            .test_support()
            .role(Role::Alert)
            .aria_label(text.clone())
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme::error(t))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::error(t))
                    .child(CONFLICT_TITLE),
            )
            .child(para(text, t))
            .child(note(
                "Rename one of each pair (outside kimün), or choose another workspace or \
                 folder. Nothing was changed.",
                t,
            ))
            .child(
                button_row().child(div().flex_1()).child(
                    Button::new(CONFLICT_DISMISS_ID)
                        .label("Dismiss")
                        .small()
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.dismiss_conflict(cx)),
                        ),
                ),
            )
            .into_any_element()
    }

    /// The "discard your changes?" question before the window enters a
    /// vault: its title and text.
    pub(crate) fn ask_text(&self) -> Option<(&'static str, &'static str)> {
        self.ws
            .ask
            .as_ref()
            .map(|a| ("Discard your changes?", a.question()))
    }

    /// Its footer commands.
    pub(crate) fn ask_buttons(&self, cx: &mut Context<Self>) -> Option<Vec<AnyElement>> {
        let ask = self.ws.ask.as_ref()?;
        Some(vec![
            Button::new(ASK_KEEP_ID)
                .label("Keep editing")
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.ask_keep(window, cx)))
                .into_any_element(),
            Button::new(ASK_GO_ID)
                .danger()
                .label(ask.go())
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.ask_go(window, cx)))
                .into_any_element(),
        ])
    }
}
