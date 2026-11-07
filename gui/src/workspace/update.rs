//! The **update flow** (M10b): the startup check, the manual check, the
//! toast that announces a newer release and the update dialog. The rules
//! (what to offer, the flow's states, the words) are the logic's
//! (`logic::update`); the release machinery is the TUI's `update` module,
//! reached through an [`UpdateBackend`] whose every call runs on tokio's
//! blocking pool, never on the UI thread.
//!
//! - **Startup check.** When the settings' `update_check` is on (the TUI's
//!   default: on), once per launch, after the window entered what it starts
//!   on and never while the setup assistant has the window (it runs once
//!   the assistant finishes and the vault is entered). Throttled by the
//!   update state file (`update::check` without `force`); silent when
//!   offline or up to date; never delays the window. Under `cfg(test)` the
//!   window has no backend unless a test gives it a fake, so no test checks.
//! - **The toast.** A newer version that was not skipped: one toast
//!   "Kimün `<version>` is available" with a `View` action (the dialog) and
//!   the kit's close button. It stays until dismissed or for
//!   [`ACTION_TOAST_LIFETIME`] (ten seconds), and a startup check does not
//!   announce the same version twice in a session. No Footer item: a missed
//!   toast leaves the palette's `+vault · check for updates` and Preferences >
//!   Display's `Check now`.
//! - **Manual check** (the palette row, leader `v u`, Preferences' `Check
//!   now`): bypasses the throttle and always reports, as a toast: up to
//!   date, available (with `View`), or the error. The palette row shows the
//!   dialog instead when a version is already known. A check asked for
//!   while one runs joins it (and makes it report).
//! - **The dialog** (the window's one modal slot; refused under the setup
//!   assistant, another modal or the window's close; allowed over
//!   Preferences, whose `Check now` leads to it). The versions; then, by
//!   what the install allows ([`UpdateOffer`]): `Install update`, or the
//!   package manager's words (and its command), or the releases page's; and
//!   `Release notes` (the releases page through the window's opener),
//!   `Skip this version` (persisted by the backend's `dismiss`), `Later`.
//!   Enter runs the primary button, Escape is `Later`, Tab walks the
//!   buttons.
//! - **Install.** The open note is flushed first, as before any operation
//!   that lets it go ([`super::note_flush`]); a flush that cannot run or
//!   fails refuses the install inline. Then the backend installs off the UI
//!   thread while the dialog shows its progress with every button waiting,
//!   and the window's close is refused. Success: "Updated to `<v>`. Restart
//!   Kimün to use it." with `Close` (the TUI only asks for a restart too,
//!   and there is no safe relaunch of a replaced binary). Failure: the error
//!   inline and `Retry`.

use std::ffi::OsStr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Task, WeakEntity, Window, div,
};
use kimun_notes::logic::session::FlushResult;
use kimun_notes::logic::update::{
    self as rules, CheckReport, CheckStart, UpdateBackend, UpdateMachine, UpdateOffer, UpdatePhase,
};
use tokio::task::JoinError;

use super::modal::{self, ModalKind, Slot};
use super::note::Closing;
use super::{ToastAction, Workspace};
use crate::tasks::blocking;
use crate::theme;

/// Element ids, for tests.
pub(crate) const UPDATE_DIALOG_ID: &str = "update-dialog";
pub(crate) const UPDATE_VERSIONS_ID: &str = "update-versions";
pub(crate) const UPDATE_TEXT_ID: &str = "update-text";
pub(crate) const UPDATE_COMMAND_ID: &str = "update-command";
pub(crate) const UPDATE_ERROR_ID: &str = "update-error";
pub(crate) const UPDATE_INSTALL_ID: &str = "update-install";
pub(crate) const UPDATE_NOTES_ID: &str = "update-release-notes";
pub(crate) const UPDATE_SKIP_ID: &str = "update-skip";
pub(crate) const UPDATE_LATER_ID: &str = "update-later";
pub(crate) const UPDATE_CLOSE_ID: &str = "update-close";

/// What the toast's `View` says when another dialog is in the way.
const DIALOG_OPEN_TEXT: &str = "Close this dialog first, then check for updates from the palette.";

/// The dialog's title while it offers a version, and once it is installed.
pub(crate) const TITLE: &str = "Update available";
pub(crate) const TITLE_INSTALLED: &str = "Update installed";
/// The install button, and its label after a failure.
pub(crate) const INSTALL_LABEL: &str = "Install update";
pub(crate) const RETRY_LABEL: &str = "Retry";
/// The toast's action.
pub(crate) const VIEW_LABEL: &str = "View";
/// What the dialog says while the open note is written before the install.
pub(crate) const SAVING_NOTE_TEXT: &str = "Saving the open note…";

/// How long a toast with an action stays up when nobody dismisses it (the
/// kit keeps such a toast up for good; the window takes it down).
pub(crate) const ACTION_TOAST_LIFETIME: Duration = Duration::from_secs(10);

/// The dialog's width, in rems.
const DIALOG_REMS: f32 = 30.;

/// The window's update flow: the backend, the state machine and the work
/// under way.
pub(crate) struct UpdateHost {
    /// Where releases come from; `None` (the tests' default): no checks.
    backend: Option<Arc<dyn UpdateBackend>>,
    pub(crate) machine: UpdateMachine,
    /// The startup check ran (or was skipped by the setting) this launch.
    startup_done: bool,
    check: Option<Task<()>>,
    install: Option<Task<()>>,
    /// The open note's flush before the install: it holds the note.
    pub(super) flushing: Option<Task<()>>,
    /// The dialog in the modal slot, while it is open.
    pub(crate) dialog: Option<WeakEntity<UpdateDialog>>,
}

impl UpdateHost {
    /// The window's flow on `backend` (`None`: no checks at all).
    pub(crate) fn new(backend: Option<Arc<dyn UpdateBackend>>) -> Self {
        Self {
            backend,
            machine: UpdateMachine::new(),
            startup_done: false,
            check: None,
            install: None,
            flushing: None,
            dialog: None,
        }
    }
}

/// The backend a window gets: the real one (the desktop app's, see
/// `logic::update::DESKTOP_BINARY_PUBLISHED`), or none under `cfg(test)`.
pub(crate) fn default_backend() -> Option<Arc<dyn UpdateBackend>> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        Some(Arc::new(rules::SystemUpdates::new(
            rules::Frontend::Desktop,
        )))
    }
}

/// The manual gate's fake release (`KIMUN_GUI_FAKE_UPDATE`, see
/// `ScriptedUpdates::from_spec`), when the variable names one. Only built
/// with the `fake-update` feature.
#[cfg(feature = "fake-update")]
pub(crate) fn fake_backend_from_env() -> Option<Arc<dyn UpdateBackend>> {
    let spec = std::env::var("KIMUN_GUI_FAKE_UPDATE").ok()?;
    let fake = rules::ScriptedUpdates::from_spec(&spec);
    if fake.is_none() {
        tracing::warn!("KIMUN_GUI_FAKE_UPDATE={spec:?} is not a known spec; ignored");
    }
    fake.map(|f| Arc::new(f) as Arc<dyn UpdateBackend>)
}

/// What the dialog shows: the flow's phase, what the install allows, and
/// whether the open note is being written before the install.
#[derive(Debug, Clone)]
pub(crate) struct DialogState {
    pub(crate) phase: UpdatePhase,
    pub(crate) offer: Option<UpdateOffer>,
    pub(crate) saving_note: bool,
}

impl DialogState {
    /// Whether everything waits (the flush or the install runs).
    fn busy(&self) -> bool {
        self.saving_note || matches!(self.phase, UpdatePhase::Installing(_))
    }
}

/// The update dialog's view. See the module docs.
pub(crate) struct UpdateDialog {
    focus: FocusHandle,
    /// The non-primary buttons' focus: Enter on one of them is that
    /// button's own (the dialog's `on_ok` must not also run the primary).
    secondary: [FocusHandle; 3],
    workspace: WeakEntity<Workspace>,
    pub(crate) state: DialogState,
}

impl UpdateDialog {
    fn new(state: DialogState, workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            secondary: [cx.focus_handle(), cx.focus_handle(), cx.focus_handle()],
            workspace,
            state,
        }
    }

    /// Whether `Release notes`, `Skip this version` or `Later` has focus.
    fn secondary_focused(&self, window: &Window) -> bool {
        self.secondary.iter().any(|h| h.is_focused(window))
    }

    fn set_state(&mut self, state: DialogState, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }

    /// The dialog's title now.
    pub(crate) fn title(&self) -> &'static str {
        if matches!(self.state.phase, UpdatePhase::Installed { .. }) {
            TITLE_INSTALLED
        } else {
            TITLE
        }
    }

    fn line(
        id: &'static str,
        text: SharedString,
        color: gpui_kit::Rgba,
        strong: bool,
    ) -> impl IntoElement {
        div()
            .id(id)
            .test_support()
            .role(Role::Label)
            .aria_label(text.clone())
            .text_sm()
            .text_color(color)
            .when(strong, |line| {
                line.font_weight(gpui_kit::FontWeight::SEMIBOLD)
            })
            .child(text)
    }

    fn render_buttons(&self) -> impl IntoElement {
        let state = &self.state;
        let busy = state.busy();
        let workspace = self.workspace.clone();
        let on = move |f: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)| {
            let workspace = workspace.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                workspace.update(cx, |w, cx| f(w, window, cx)).ok();
            }
        };
        let row = modal::footer_row().flex_wrap();
        if let UpdatePhase::Installed { .. } = state.phase {
            return row.child(
                Button::new(UPDATE_CLOSE_ID)
                    .primary()
                    .small()
                    .label("Close")
                    .on_click(on(Workspace::update_later)),
            );
        }
        let can_install = state.offer == Some(UpdateOffer::Install);
        let failed = matches!(state.phase, UpdatePhase::Failed { .. });
        let install = can_install.then(|| {
            Button::new(UPDATE_INSTALL_ID)
                .primary()
                .small()
                .label(if failed { RETRY_LABEL } else { INSTALL_LABEL })
                .loading(busy)
                .disabled(busy)
                .on_click(on(Workspace::install_update))
        });
        let [notes_focus, skip_focus, later_focus] = &self.secondary;
        let notes = Button::new(UPDATE_NOTES_ID)
            .track_focus(notes_focus)
            .small()
            .label("Release notes")
            .disabled(busy)
            .on_click(on(Workspace::open_release_notes_from_dialog));
        // Without an install, the release notes are the way forward.
        let notes = if can_install {
            notes.outline()
        } else {
            notes.primary()
        };
        row.children(install)
            .child(notes)
            .child(
                Button::new(UPDATE_SKIP_ID)
                    .track_focus(skip_focus)
                    .small()
                    .ghost()
                    .label("Skip this version")
                    .disabled(busy)
                    .on_click(on(Workspace::skip_update)),
            )
            .child(
                Button::new(UPDATE_LATER_ID)
                    .track_focus(later_focus)
                    .small()
                    .ghost()
                    .label("Later")
                    .disabled(busy)
                    .on_click(on(Workspace::update_later)),
            )
    }
}

impl Focusable for UpdateDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for UpdateDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let state = self.state.clone();
        let mut body = div()
            .id(UPDATE_DIALOG_ID)
            .test_support()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap_2();
        let status = match &state.phase {
            UpdatePhase::Available(s)
            | UpdatePhase::Installing(s)
            | UpdatePhase::Failed { status: s, .. } => Some(s.clone()),
            _ => None,
        };
        if let Some(status) = status.as_ref() {
            body = body.child(Self::line(
                UPDATE_VERSIONS_ID,
                rules::versions_text(status).into(),
                theme::bright(t),
                true,
            ));
        }
        let text: SharedString = match (&state.phase, &state.offer) {
            (UpdatePhase::Installed { version }, _) => rules::installed_text(version).into(),
            _ if state.saving_note => SAVING_NOTE_TEXT.into(),
            (UpdatePhase::Installing(_), _) => rules::INSTALLING_TEXT.into(),
            (_, Some(UpdateOffer::PackageManager { .. })) => rules::PACKAGE_MANAGER_TEXT.into(),
            (_, Some(UpdateOffer::Download)) => rules::DOWNLOAD_TEXT.into(),
            (UpdatePhase::Available(_) | UpdatePhase::Failed { .. }, _) => {
                "Installs in place of this copy, then asks for a restart.".into()
            }
            _ => "No update is known.".into(),
        };
        body = body.child(Self::line(
            UPDATE_TEXT_ID,
            text,
            theme::foreground(t),
            false,
        ));
        if let (Some(UpdateOffer::PackageManager { command: Some(cmd) }), false) = (
            &state.offer,
            matches!(state.phase, UpdatePhase::Installed { .. }),
        ) {
            body = body.child(Self::line(
                UPDATE_COMMAND_ID,
                format!("Run: {cmd}").into(),
                theme::muted(t),
                false,
            ));
        }
        if let UpdatePhase::Failed { error, .. } = &state.phase {
            let error: SharedString = rules::install_failed_text(error).into();
            body = body.child(
                div()
                    .id(UPDATE_ERROR_ID)
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(error.clone())
                    .text_sm()
                    .text_color(theme::error(t))
                    .child(error),
            );
        }
        body.child(div().border_t_1().border_color(theme::divider(t)))
            .child(self.render_buttons())
    }
}

// ---- The workspace's side ----------------------------------------------------------

impl Workspace {
    /// Use `backend` for the update flow (`main`'s manual-gate fake, the
    /// tests' fakes). Before [`Self::start`], so the startup check uses it.
    #[cfg(any(test, feature = "fake-update"))]
    pub(crate) fn set_update_backend(&mut self, backend: Arc<dyn UpdateBackend>) {
        self.updates.backend = Some(backend);
    }

    /// Whether the install (or the note's flush before it) runs: the
    /// window's close waits for it.
    pub(crate) fn update_busy(&self) -> bool {
        self.updates.machine.is_installing() || self.updates.flushing.is_some()
    }

    /// The startup check, once per launch, when the setting is on and the
    /// setup assistant does not have the window (see the module docs).
    pub(super) fn check_updates_on_start(&mut self, cx: &mut Context<Self>) {
        if self.updates.startup_done || self.onboarding_open() {
            return;
        }
        let Some(settings) = self.settings.clone() else {
            return;
        };
        self.updates.startup_done = true;
        let on = settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .update_check();
        if on {
            self.run_update_check(false, cx);
        }
    }

    /// The palette row (leader `v u`): the dialog when a version is known
    /// (or installed), else a manual check.
    pub(crate) fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let machine = &self.updates.machine;
        if machine.shown().is_some() || machine.is_installed() {
            self.open_update_dialog(window, cx);
        } else {
            self.check_updates_now(cx);
        }
    }

    /// A manual check (Preferences' `Check now`, the palette without a known
    /// version): bypasses the throttle, reports whatever it finds.
    pub(crate) fn check_updates_now(&mut self, cx: &mut Context<Self>) {
        if let UpdatePhase::Installed { version } = self.updates.machine.phase() {
            let text = rules::installed_text(version);
            self.flash(text, cx);
            return;
        }
        self.run_update_check(true, cx);
    }

    /// Start a check (`manual`: it reports), unless one runs (it joins it)
    /// or there is no backend.
    fn run_update_check(&mut self, manual: bool, cx: &mut Context<Self>) {
        let Some(backend) = self.updates.backend.clone() else {
            return;
        };
        let force = match self.updates.machine.begin_check(manual) {
            CheckStart::Run { force } => force,
            // A manual ask joins what runs: say what that is, so the click
            // is not dead.
            CheckStart::Busy => {
                if manual {
                    let text = if self.updates.machine.is_installing() {
                        rules::INSTALLING_TEXT
                    } else {
                        rules::CHECKING_TEXT
                    };
                    self.flash(text, cx);
                }
                return;
            }
            CheckStart::Installed => return,
        };
        if manual {
            self.flash(rules::CHECKING_TEXT, cx);
        }
        let job = blocking(move || backend.check(force));
        self.updates.check = Some(cx.spawn(async move |this, cx| {
            let result = job.await.and_then(|r| r);
            this.update(cx, |w, cx| w.update_check_landed(result, cx))
                .ok();
        }));
    }

    fn update_check_landed(
        &mut self,
        result: Result<Option<kimun_notes::update::UpdateStatus>, String>,
        cx: &mut Context<Self>,
    ) {
        self.updates.check = None;
        if let Err(e) = &result {
            tracing::debug!("update check failed: {e}");
        }
        match self.updates.machine.check_landed(result) {
            CheckReport::Available(status) => {
                let view =
                    ToastAction::new(VIEW_LABEL, |w, window, cx| w.open_update_dialog(window, cx));
                self.flash_with_action(rules::available_text(&status.latest), view, cx);
            }
            CheckReport::UpToDate(text) => self.flash(text, cx),
            CheckReport::Failed(text) => self.flash_error(text, cx),
            CheckReport::Silent => {}
        }
        self.refresh_update_dialog(cx);
    }

    /// What the dialog shows now.
    fn update_dialog_state(&self) -> DialogState {
        let machine = &self.updates.machine;
        let offer = match (&self.updates.backend, machine.shown()) {
            (Some(backend), Some(status)) => Some(backend.offer(status)),
            _ => None,
        };
        DialogState {
            phase: machine.shown_phase().clone(),
            offer,
            saving_note: self.updates.flushing.is_some(),
        }
    }

    /// Bring the open dialog up to date with the flow.
    fn refresh_update_dialog(&mut self, cx: &mut Context<Self>) {
        let state = self.update_dialog_state();
        if let Some(view) = self.updates.dialog.as_ref().and_then(|d| d.upgrade()) {
            view.update(cx, |view, cx| view.set_state(state, cx));
        }
        cx.notify();
    }

    /// Open the update dialog on the known version (the toast's `View`, the
    /// palette row). Refused under the setup assistant, another modal and
    /// the window's close; nothing to show without a known or installed
    /// version.
    pub(crate) fn open_update_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.onboarding_open() || self.closing != Closing::Open {
            return;
        }
        if self.open_modal(window, cx) == Some(ModalKind::Update) {
            self.refresh_update_dialog(cx);
            return;
        }
        if modal::dialog_open(window, cx) {
            if self.updates.machine.shown().is_some() {
                self.flash(DIALOG_OPEN_TEXT, cx);
            }
            return;
        }
        let machine = &self.updates.machine;
        if machine.shown().is_none() && !machine.is_installed() {
            return;
        }
        let workspace = cx.entity().downgrade();
        let state = self.update_dialog_state();
        let view = cx.new(|cx| UpdateDialog::new(state, workspace.clone(), cx));
        self.updates.dialog = Some(view.downgrade());
        self.show_modal(
            Slot::Own(ModalKind::Update),
            DIALOG_REMS,
            modal::focus_view(view.clone()),
            move |dialog, _, cx| {
                let title = view.read(cx).title();
                let ok = view.downgrade();
                let cancel = workspace.clone();
                dialog
                    .title(title)
                    // Enter: the primary button, unless a secondary one has
                    // focus (it runs itself).
                    .on_ok(move |_, window, cx| {
                        if let Some(view) = ok.upgrade() {
                            if view.read(cx).secondary_focused(window) {
                                return false;
                            }
                            let workspace = view.read(cx).workspace.clone();
                            workspace
                                .update(cx, |w, cx| w.update_primary(window, cx))
                                .ok();
                        }
                        false
                    })
                    // Escape and a click outside: Later (not while busy).
                    .on_cancel(move |_, window, cx| {
                        cancel.update(cx, |w, cx| w.update_later(window, cx)).ok();
                        false
                    })
                    .child(view.clone())
            },
            window,
            cx,
        );
    }

    /// Enter in the dialog: install (or retry), else the release notes, or
    /// close once installed. Nothing while busy.
    fn update_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.update_dialog_state();
        if state.busy() {
            return;
        }
        match (&state.phase, &state.offer) {
            (UpdatePhase::Installed { .. }, _) => self.update_later(window, cx),
            (_, Some(UpdateOffer::Install)) => self.install_update(window, cx),
            (_, Some(_)) => self.open_release_notes(cx),
            _ => {}
        }
    }

    /// `Later`, `Close`, Escape: close the dialog (a failure is offered
    /// clean next time). Not while the install runs.
    pub(crate) fn update_later(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.update_busy() {
            return;
        }
        self.updates.machine.later();
        if self.open_modal(window, cx) == Some(ModalKind::Update) {
            self.close_modal(window, cx);
        } else {
            self.forget_update_dialog();
        }
    }

    /// `Skip this version`: persisted through the backend (off the UI
    /// thread), the dialog closed; no startup check announces it again.
    pub(crate) fn skip_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.update_busy() {
            return;
        }
        let (Some(backend), Some(version)) =
            (self.updates.backend.clone(), self.updates.machine.dismiss())
        else {
            return;
        };
        let job = blocking(move || backend.dismiss(&version));
        cx.spawn(async move |this, cx| {
            if let Err(e) = job.await.and_then(|r| r) {
                tracing::warn!("could not save the skipped version: {e}");
                this.update(cx, |w, cx| {
                    w.flash_error(format!("Couldn't save the skipped version: {e}"), cx)
                })
                .ok();
            }
        })
        .detach();
        if self.open_modal(window, cx) == Some(ModalKind::Update) {
            self.close_modal(window, cx);
        }
        cx.notify();
    }

    /// `Release notes`: the releases page through the window's opener.
    fn open_release_notes_from_dialog(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.open_release_notes(cx);
    }

    fn open_release_notes(&mut self, cx: &mut Context<Self>) {
        let Some(backend) = self.updates.backend.as_ref() else {
            return;
        };
        let url = backend.releases_url();
        if let Err(e) = (self.opener)(OsStr::new(&url)) {
            tracing::warn!("could not open {url}: {e}");
            self.flash_error(format!("Couldn't open the release notes: {e}"), cx);
        }
    }

    /// `Install update` / `Retry`: flush the open note, then install (see
    /// the module docs). Only where the offer is an install.
    pub(crate) fn install_update(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(backend) = self.updates.backend.clone() else {
            return;
        };
        let Some(status) = self.updates.machine.known().cloned() else {
            return;
        };
        // Never while the window closes (`flush_block` says so for an open
        // note; this covers no note, or a read-only vault).
        if backend.offer(&status) != UpdateOffer::Install
            || self.update_busy()
            || self.closing != Closing::Open
        {
            return;
        }
        let has_note = self.notes.as_ref().is_some_and(|h| h.note.is_some());
        if !has_note || self.status.read_only {
            self.start_install(cx);
            return;
        }
        if let Some(block) = self.flush_block(cx) {
            self.refuse_install(block.reason(), cx);
            return;
        }
        let Some(flush) = self.begin_note_flush(cx) else {
            self.refuse_install("another save of the note is still running", cx);
            return;
        };
        self.updates.flushing = Some(cx.spawn(async move |this, cx| {
            let flushed = tokio::spawn(flush.run()).await;
            this.update(cx, |w, cx| w.update_note_flushed(flushed, cx))
                .ok();
        }));
        self.refresh_update_dialog(cx);
    }

    fn update_note_flushed(
        &mut self,
        flushed: Result<FlushResult, JoinError>,
        cx: &mut Context<Self>,
    ) {
        self.updates.flushing = None;
        let outcome = self.apply_flush(flushed, cx);
        let problem = self.flush_failure(&outcome, cx);
        self.restart_tick_unless_read_only();
        self.block_input(false, cx);
        self.file_op_ended(cx);
        match problem {
            Some(problem) => {
                tracing::warn!("update not installed: {problem}");
                self.refuse_install(&problem, cx);
            }
            None => self.start_install(cx),
        }
    }

    /// The install cannot start: the open note could not be saved first.
    fn refuse_install(&mut self, reason: &str, cx: &mut Context<Self>) {
        self.updates
            .machine
            .install_refused(format!("the open note couldn't be saved first ({reason})"));
        self.refresh_update_dialog(cx);
    }

    fn start_install(&mut self, cx: &mut Context<Self>) {
        let Some(backend) = self.updates.backend.clone() else {
            return;
        };
        let offer = match self.updates.machine.known() {
            Some(status) => backend.offer(status),
            None => return,
        };
        let Some(status) = self.updates.machine.begin_install(&offer) else {
            return;
        };
        let job = blocking(move || backend.install(&status));
        self.updates.install = Some(cx.spawn(async move |this, cx| {
            let result = job.await.and_then(|r| r);
            this.update(cx, |w, cx| w.update_install_landed(result, cx))
                .ok();
        }));
        self.refresh_update_dialog(cx);
    }

    fn update_install_landed(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
        self.updates.install = None;
        if let Err(e) = &result {
            tracing::warn!("update install failed: {e}");
        }
        let shown = self
            .updates
            .dialog
            .as_ref()
            .is_some_and(|d| d.upgrade().is_some());
        // The dialog went meanwhile (a modal reset): the toast says it.
        if !shown {
            match &result {
                Ok(version) => self.flash(rules::installed_text(version), cx),
                Err(e) => self.flash_error(rules::install_failed_text(e), cx),
            }
        }
        self.updates.machine.install_landed(result);
        self.refresh_update_dialog(cx);
    }

    /// The dialog's handle goes with the modal ([`Self::reset_modal_state`]);
    /// the flow and any install under way stay.
    pub(super) fn forget_update_dialog(&mut self) {
        self.updates.dialog = None;
    }
}

/// The toast action's run, boxed (see [`ToastAction`]).
pub(crate) type ToastRun = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;
