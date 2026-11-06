//! The **setup assistant** (M8c): the TUI's guided setup
//! (`app_screen/onboarding.rs`) as a full-window view that takes the
//! Workspace body over while it shows. The rail, the drawers and the editor
//! are not drawn, so nothing of theirs can be focused, and the workspace's
//! chords (leader, palette, drawer toggles) do nothing.
//!
//! **Entry.** It opens on its own when the window enters `Start::NoWorkspace`
//! (a first run: leaving it quits the app), and from the leader / palette
//! (`OpenOnboarding`, "guided setup"), with or without a vault (a re-run:
//! the workspaces are shown, never changed).
//!
//! **State.** The steps, the staged draft and the rules (the "choose a
//! folder first" gate, the name that follows the folder until edited) are
//! the logic `OnboardingModel`'s; this view draws it and maps keys and
//! clicks onto it. The model is built off the UI thread (it probes for nvim
//! and reads the theme folder). It is built without the Nerd Fonts step, which only the terminal
//! app uses (the assistant walks six steps), and without the nvim backend,
//! which the desktop app has not enabled (the Backend step offers plain and
//! vim). Both settings stay as configured unless changed here: a configured
//! nvim survives Finish when no other backend is chosen.
//!
//! **The folder chooser** is a panel of its own inside the assistant, not a
//! dialog over it: the current folder with Up, its subfolders (a click
//! enters one, a double-click or `Choose this folder` takes the folder
//! shown) and an inline `New folder…` field. Listings and the new folder run
//! on tokio's blocking pool (`logic::onboarding::list_subdirs`,
//! `create_subdir`, through core); a stale listing is dropped by revision.
//!
//! **Theme preview.** Choosing a theme restyles the whole window at once
//! ([`Workspace::apply_theme`]); leaving without finishing puts the theme
//! the window had back.
//!
//! **Finish** applies the draft off the UI thread (`apply_draft`: the folder
//! is created, the workspace registered), then hands over to the window,
//! which applies the theme, persists the settings through its one settings
//! write ([`Workspace::persist_settings`]) and sends `OnboardingFinished`, so
//! the vault is opened again as at startup (M8b). A refused draft is shown
//! in the assistant, back on the step that can fix it.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ObjectFit, ParentElement, Render,
    Role, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, StyledImage as _,
    Subscription, Task, UniformListScrollHandle, Window, div, img, px, rems, uniform_list,
};
use kimun_core::system::{self, SystemPath};
use kimun_notes::logic::input::{Key, KeyInput};
use kimun_notes::logic::message::AppMessage;
use kimun_notes::logic::onboarding::{self as onb, OnbStep, OnboardingModel};
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{EditorBackendSetting, SharedSettings};

use super::Workspace;
use crate::assets;
use crate::input;
use crate::startup::EmptyState;
use crate::theme;

/// The assistant's root element.
pub(crate) const ONBOARDING_ID: &str = "onboarding";
/// The footer's commands.
pub(crate) const NEXT_ID: &str = "onboarding-next";
pub(crate) const BACK_ID: &str = "onboarding-back";
pub(crate) const CANCEL_ID: &str = "onboarding-cancel";
/// The inline problem line (a refused finish, "choose a folder first").
pub(crate) const MESSAGE_ID: &str = "onboarding-message";
/// The app logo on the Welcome step.
pub(crate) const LOGO_ID: &str = "onboarding-logo";
/// The Workspace step's controls.
pub(crate) const CHOOSE_FOLDER_ID: &str = "onboarding-choose-folder";
pub(crate) const NAME_ID: &str = "onboarding-name";
/// The folder chooser's controls.
pub(crate) const CHOOSER_ID: &str = "onboarding-chooser";
pub(crate) const CHOOSER_UP_ID: &str = "onboarding-chooser-up";
pub(crate) const CHOOSER_NEW_ID: &str = "onboarding-chooser-new";
pub(crate) const CHOOSER_CHOOSE_ID: &str = "onboarding-chooser-choose";
pub(crate) const CHOOSER_CANCEL_ID: &str = "onboarding-chooser-cancel";
pub(crate) const CHOOSER_LIST_ID: &str = "onboarding-chooser-list";
pub(crate) const NEW_FOLDER_ID: &str = "onboarding-new-folder";
pub(crate) const NEW_FOLDER_CREATE_ID: &str = "onboarding-new-folder-create";
/// The confirmation's two answers.
pub(crate) const CONFIRM_YES_ID: &str = "onboarding-confirm-yes";
pub(crate) const CONFIRM_NO_ID: &str = "onboarding-confirm-no";
/// The Theme step's list.
const THEME_LIST_ID: &str = "onboarding-themes";

/// What the GUI says when the first run tries to leave the Workspace step
/// without a folder (the TUI's "choose a directory first").
pub(crate) const CHOOSE_FOLDER_FIRST: &str = "Choose a folder for your notes first.";
/// What the name field says about a name `set_workspace_name` refuses.
pub(crate) const INVALID_NAME: &str =
    "Not a valid workspace name: it must work as a folder name on every system.";

/// The element id of the chooser's row for `dir` (its folder name).
pub(crate) fn chooser_row_id(dir: &SystemPath) -> SharedString {
    format!("onboarding-dir-{}", folder_name(dir)).into()
}

/// The element id of the Theme step's row for `name`.
pub(crate) fn theme_row_id(name: &str) -> SharedString {
    format!("onboarding-theme-{name}").into()
}

/// The last component of `dir`, as shown in the chooser.
fn folder_name(dir: &SystemPath) -> String {
    dir.as_path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| system::path_to_string(dir))
}

/// What the assistant asks of the window.
pub(crate) enum OnboardingEvent {
    /// Show the whole window in this theme (a theme row was chosen).
    Preview(Theme),
    /// Leave without applying anything; the window goes back to this theme.
    Close(Theme),
    /// The first run was left: quit the app.
    Quit,
    /// The draft was applied to the shared settings: show this theme, save
    /// the settings and open the vault they name.
    Finished { theme: Theme, first_run: bool },
}

/// A configured workspace, as the re-run's Workspace step lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceRow {
    name: String,
    path: String,
    current: bool,
}

/// What is read off the UI thread when the assistant opens.
struct Loaded {
    model: OnboardingModel,
    workspaces: Vec<WorkspaceRow>,
}

impl Loaded {
    /// Blocking: the nvim probe and the theme folder.
    fn read(settings: &SharedSettings) -> Self {
        let s = settings.read().unwrap_or_else(|e| e.into_inner());
        // Nerd Fonts only matter to the terminal app, and the desktop app has
        // no nvim engine: neither is offered, and the draft keeps both
        // configured values.
        let model = OnboardingModel::new(&s)
            .without_step(OnbStep::NerdFonts)
            .without_backend(EditorBackendSetting::Nvim);
        let current = s.current_workspace_name().unwrap_or_default();
        let workspaces = s
            .workspace_config
            .as_ref()
            .map(|wc| {
                wc.workspaces
                    .iter()
                    .map(|(name, entry)| WorkspaceRow {
                        name: name.clone(),
                        path: system::path_to_string(entry.effective_path()),
                        current: *name == current,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { model, workspaces }
    }
}

/// The question over the assistant while one is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confirm {
    /// First run: leaving means quitting.
    Quit,
    /// Re-run with changes: leaving drops them.
    Discard,
}

/// A folder listing: under way, done, or refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Listing {
    Loading,
    Loaded(Vec<SystemPath>),
    Failed(String),
}

/// The inline `New folder…` field and the folder being made.
struct NewFolder {
    input: Entity<InputState>,
    error: Option<SharedString>,
    task: Option<Task<()>>,
}

/// The folder chooser panel: the folder shown and its subfolders.
pub(crate) struct Chooser {
    dir: SystemPath,
    listing: Listing,
    selected: Option<usize>,
    /// Bumped by every listing asked for: an older one landing is dropped.
    revision: u64,
    task: Option<Task<()>>,
    new_folder: Option<NewFolder>,
    scroll: UniformListScrollHandle,
}

impl Chooser {
    /// The folder shown.
    #[cfg(test)]
    pub(crate) fn dir(&self) -> &SystemPath {
        &self.dir
    }

    /// Its listing.
    #[cfg(test)]
    pub(crate) fn listing(&self) -> &Listing {
        &self.listing
    }

    /// The selected row.
    #[cfg(test)]
    pub(crate) fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The new folder field's problem, if it shows one.
    #[cfg(test)]
    pub(crate) fn new_folder_error(&self) -> Option<&str> {
        self.new_folder.as_ref()?.error.as_deref()
    }

    /// Whether the new folder field shows.
    #[cfg(test)]
    pub(crate) fn new_folder_open(&self) -> bool {
        self.new_folder.is_some()
    }

    fn rows(&self) -> &[SystemPath] {
        match &self.listing {
            Listing::Loaded(rows) => rows,
            _ => &[],
        }
    }
}

/// The setup assistant. See the module docs.
pub(crate) struct OnboardingView {
    settings: SharedSettings,
    /// The theme the assistant draws with: the preview.
    theme: Theme,
    /// The theme the window had when the assistant opened.
    original: Theme,
    /// `None` while it is being read.
    model: Option<OnboardingModel>,
    workspaces: Vec<WorkspaceRow>,
    focus: FocusHandle,
    name_input: Entity<InputState>,
    name_error: Option<SharedString>,
    /// The inline problem line.
    message: Option<SharedString>,
    chooser: Option<Chooser>,
    confirm: Option<Confirm>,
    /// The draft being applied: every control waits for it.
    finishing: Option<Task<()>>,
    theme_scroll: UniformListScrollHandle,
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<OnboardingEvent> for OnboardingView {}

impl Focusable for OnboardingView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Run `job` on tokio's blocking pool, from the GPUI side: its result, or
/// the task's failure as text. Needs the runtime entered.
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let handle = tokio::spawn(async move { tokio::task::spawn_blocking(job).await });
    match handle.await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) | Err(e) => Err(e.to_string()),
    }
}

impl OnboardingView {
    /// An assistant over `settings`, the window showing `original` now.
    /// Its model is read in the background. Needs the tokio runtime entered.
    pub(crate) fn new(
        settings: SharedSettings,
        original: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Workspace name"));
        let subscriptions = vec![
            cx.subscribe(&name_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.name_changed(cx);
                }
            }),
        ];
        let read = settings.clone();
        let load = cx.spawn_in(window, async move |this, cx| {
            let loaded = blocking(move || Loaded::read(&read)).await;
            this.update_in(cx, |view, window, cx| view.loaded(loaded, window, cx))
                .ok();
        });
        Self {
            settings,
            theme: original.clone(),
            original,
            model: None,
            workspaces: Vec::new(),
            focus: cx.focus_handle(),
            name_input,
            name_error: None,
            message: None,
            chooser: None,
            confirm: None,
            finishing: None,
            theme_scroll: UniformListScrollHandle::new(),
            _load: Some(load),
            _subscriptions: subscriptions,
        }
    }

    fn loaded(
        &mut self,
        loaded: Result<Loaded, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._load = None;
        match loaded {
            Ok(Loaded { model, workspaces }) => {
                if let Some((name, _)) = model.draft.workspace.as_ref() {
                    let name = name.clone();
                    self.name_input
                        .update(cx, |input, cx| input.set_value(name, window, cx));
                }
                self.model = Some(model);
                self.workspaces = workspaces;
            }
            Err(e) => self.message = Some(format!("Couldn't start the setup: {e}").into()),
        }
        cx.notify();
    }

    // ---- reading (the tests) ----------------------------------------------

    /// The model, once read.
    #[cfg(test)]
    pub(crate) fn model(&self) -> Option<&OnboardingModel> {
        self.model.as_ref()
    }

    /// The model, to inject facts in tests (nvim's availability).
    #[cfg(test)]
    pub(crate) fn model_mut(&mut self) -> Option<&mut OnboardingModel> {
        self.model.as_mut()
    }

    /// The step shown, once the model is read.
    pub(crate) fn step(&self) -> Option<OnbStep> {
        self.model.as_ref().map(|m| m.step)
    }

    /// The inline problem line.
    #[cfg(test)]
    pub(crate) fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// The name field's problem.
    #[cfg(test)]
    pub(crate) fn name_error(&self) -> Option<&str> {
        self.name_error.as_deref()
    }

    /// The name field.
    #[cfg(test)]
    pub(crate) fn name_input(&self) -> &Entity<InputState> {
        &self.name_input
    }

    /// The folder chooser, while it shows.
    #[cfg(test)]
    pub(crate) fn chooser(&self) -> Option<&Chooser> {
        self.chooser.as_ref()
    }

    /// The question asked, while one is.
    #[cfg(test)]
    pub(crate) fn confirm(&self) -> Option<Confirm> {
        self.confirm
    }

    /// Whether the draft is being applied.
    #[cfg(test)]
    pub(crate) fn finishing(&self) -> bool {
        self.finishing.is_some()
    }

    /// The theme the assistant draws with.
    #[cfg(test)]
    pub(crate) fn preview_theme(&self) -> &Theme {
        &self.theme
    }

    fn first_run(&self) -> bool {
        self.model.as_ref().is_some_and(|m| m.first_run)
    }

    fn busy(&self) -> bool {
        self.finishing.is_some() || self.model.is_none()
    }

    /// Whether one of the assistant's text fields has the keyboard: its
    /// keys are the field's.
    fn field_focused(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        let focused =
            |input: &Entity<InputState>| input.read(cx).focus_handle(cx).is_focused(window);
        focused(&self.name_input)
            || self
                .chooser
                .as_ref()
                .and_then(|c| c.new_folder.as_ref())
                .is_some_and(|f| focused(&f.input))
    }

    fn new_folder_focused(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        self.chooser
            .as_ref()
            .and_then(|c| c.new_folder.as_ref())
            .is_some_and(|f| f.input.read(cx).focus_handle(cx).is_focused(window))
    }

    // ---- the steps --------------------------------------------------------

    /// `Next` (Enter, Right): advance, or finish on the summary. The first
    /// run stays on the Workspace step until a folder is chosen and the
    /// name is valid.
    pub(crate) fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(model) = self.model.as_mut() else {
            return;
        };
        match model.step {
            OnbStep::Summary => return self.finish(window, cx),
            OnbStep::Workspace if !model.can_advance() => {
                self.message = Some(CHOOSE_FOLDER_FIRST.into());
            }
            OnbStep::Workspace if model.first_run && self.name_error.is_some() => {
                self.message = self.name_error.clone();
            }
            _ => {
                model.go_next();
                self.left_step(window, cx);
            }
        }
        cx.notify();
    }

    /// `Back` (Left).
    pub(crate) fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        if let Some(model) = self.model.as_mut()
            && model.go_prev()
        {
            self.left_step(window, cx);
        }
        cx.notify();
    }

    /// A step was left: its problem line and the chooser go, the keyboard
    /// comes back to the assistant, and the theme list scrolls to its row.
    fn left_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.message = None;
        self.chooser = None;
        window.focus(&self.focus, cx);
        if let Some(model) = self.model.as_ref()
            && model.step == OnbStep::Theme
        {
            self.theme_scroll
                .scroll_to_item(model.theme_idx, ScrollStrategy::Nearest);
        }
    }

    /// Esc, `Cancel`: the first run asks before quitting, a re-run with
    /// changes before dropping them, a clean re-run just closes.
    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.finishing.is_some() {
            return;
        }
        let Some(model) = self.model.as_ref() else {
            return;
        };
        if model.first_run {
            self.confirm = Some(Confirm::Quit);
        } else if model.dirty(&self.settings.read().unwrap_or_else(|e| e.into_inner())) {
            self.confirm = Some(Confirm::Discard);
        } else {
            cx.emit(OnboardingEvent::Close(self.original.clone()));
        }
        cx.notify();
    }

    /// The question's yes: quit, or drop the changes.
    pub(crate) fn confirm_yes(&mut self, cx: &mut Context<Self>) {
        match self.confirm.take() {
            Some(Confirm::Quit) => cx.emit(OnboardingEvent::Quit),
            Some(Confirm::Discard) => cx.emit(OnboardingEvent::Close(self.original.clone())),
            None => {}
        }
        cx.notify();
    }

    /// The question's no: back to the setup.
    pub(crate) fn confirm_no(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Select theme row `idx` and show the window in it.
    pub(crate) fn select_theme(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        if let Some(t) = model.select_theme(idx) {
            let t = t.clone();
            self.theme = t.clone();
            self.theme_scroll
                .scroll_to_item(idx, ScrollStrategy::Nearest);
            cx.emit(OnboardingEvent::Preview(t));
        }
        cx.notify();
    }

    /// Select backend row `idx` (of the model's backends: plain, vim).
    pub(crate) fn select_backend(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(model) = self.model.as_mut()
            && model.select_backend(idx)
        {
            cx.notify();
        }
    }

    fn set_update_check(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(model) = self.model.as_mut() {
            model.draft.update_check = on;
        }
        cx.notify();
    }

    /// The name field changed: a valid name becomes the draft's (and is no
    /// longer replaced by a folder's suggestion).
    fn name_changed(&mut self, cx: &mut Context<Self>) {
        let typed = self.name_input.read(cx).value().to_string();
        let Some(model) = self.model.as_mut() else {
            return;
        };
        if !model.first_run || model.draft.workspace.is_none() {
            return;
        }
        self.name_error = (!model.set_workspace_name(&typed)).then(|| INVALID_NAME.into());
        if self.name_error.is_none() && self.message.as_deref() == Some(INVALID_NAME) {
            self.message = None;
        }
        cx.notify();
    }

    /// Adopt `dir` as the workspace folder: the name follows it unless it
    /// was edited. The chooser closes.
    pub(crate) fn choose_directory(
        &mut self,
        dir: SystemPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        model.choose_directory(dir.into_path_buf());
        if !model.name_edited
            && let Some((name, _)) = model.draft.workspace.as_ref()
        {
            let name = name.clone();
            self.name_input
                .update(cx, |input, cx| input.set_value(name, window, cx));
            self.name_error = None;
        }
        self.chooser = None;
        self.message = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    // ---- the folder chooser -----------------------------------------------

    /// `Choose folder…` (b): the chooser, on the chosen folder's parent (or
    /// the suggestion's, or the home folder).
    pub(crate) fn open_chooser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(model) = self.model.as_ref() else {
            return;
        };
        if !model.first_run || self.busy() {
            return;
        }
        let start = model
            .browse_start()
            .and_then(|p| SystemPath::try_absolute(p).ok())
            .or_else(|| system::browse_root().ok());
        let Some(start) = start else {
            self.message = Some("Couldn't find a folder to start browsing from.".into());
            cx.notify();
            return;
        };
        self.chooser = Some(Chooser {
            dir: start.clone(),
            listing: Listing::Loading,
            selected: None,
            revision: 0,
            task: None,
            new_folder: None,
            scroll: UniformListScrollHandle::new(),
        });
        self.message = None;
        window.focus(&self.focus, cx);
        self.browse(start, window, cx);
    }

    /// Show `dir` in the chooser, listed in the background.
    pub(crate) fn browse(&mut self, dir: SystemPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chooser) = self.chooser.as_mut() else {
            return;
        };
        chooser.revision += 1;
        let revision = chooser.revision;
        chooser.dir = dir.clone();
        chooser.listing = Listing::Loading;
        chooser.selected = None;
        chooser.new_folder = None;
        chooser.task = Some(cx.spawn_in(window, async move |this, cx| {
            let listed = blocking(move || onb::list_subdirs(&dir).map_err(|e| e.to_string())).await;
            this.update(cx, |view, cx| {
                view.listed(revision, listed.and_then(|r| r), cx)
            })
            .ok();
        }));
        cx.notify();
    }

    fn listed(
        &mut self,
        revision: u64,
        listed: Result<Vec<SystemPath>, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(chooser) = self.chooser.as_mut() else {
            return;
        };
        if chooser.revision != revision {
            return;
        }
        chooser.task = None;
        match listed {
            Ok(rows) => {
                chooser.selected = (!rows.is_empty()).then_some(0);
                chooser.listing = Listing::Loaded(rows);
                chooser.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            Err(e) => chooser.listing = Listing::Failed(e),
        }
        cx.notify();
    }

    /// `Up` (Left, Backspace): the parent folder.
    pub(crate) fn chooser_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(parent) = self.chooser.as_ref().and_then(|c| c.dir.parent()) {
            self.browse(parent, window, cx);
        }
    }

    /// Enter (Right): into the selected subfolder.
    fn chooser_enter_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .chooser
            .as_ref()
            .and_then(|c| c.selected.and_then(|ix| c.rows().get(ix).cloned()));
        if let Some(target) = target {
            self.browse(target, window, cx);
        }
    }

    fn chooser_move(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some(chooser) = self.chooser.as_mut() else {
            return;
        };
        let count = chooser.rows().len();
        if count == 0 {
            return;
        }
        let ix = match (chooser.selected, down) {
            (None, _) => 0,
            (Some(ix), true) => (ix + 1).min(count - 1),
            (Some(ix), false) => ix.saturating_sub(1),
        };
        chooser.selected = Some(ix);
        chooser.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// `Choose this folder` (Ctrl+Enter, a double-click): the folder shown.
    pub(crate) fn chooser_choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dir) = self.chooser.as_ref().map(|c| c.dir.clone()) {
            self.choose_directory(dir, window, cx);
        }
    }

    /// `Cancel` (Esc): back to the Workspace step, nothing chosen.
    pub(crate) fn close_chooser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chooser = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// `New folder…` (n): the inline name field, focused.
    pub(crate) fn open_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chooser) = self.chooser.as_mut() else {
            return;
        };
        if chooser.new_folder.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("New folder name"));
            chooser.new_folder = Some(NewFolder {
                input,
                error: None,
                task: None,
            });
        }
        if let Some(folder) = chooser.new_folder.as_ref() {
            let handle = folder.input.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    /// The new folder field's text, for tests.
    #[cfg(test)]
    pub(crate) fn new_folder_input(&self) -> Option<Entity<InputState>> {
        Some(self.chooser.as_ref()?.new_folder.as_ref()?.input.clone())
    }

    fn close_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(chooser) = self.chooser.as_mut() {
            chooser.new_folder = None;
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// `Create` (Enter in the field): make the folder in the background and
    /// enter it.
    pub(crate) fn create_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chooser) = self.chooser.as_mut() else {
            return;
        };
        let dir = chooser.dir.clone();
        let Some(folder) = chooser.new_folder.as_mut() else {
            return;
        };
        if folder.task.is_some() {
            return;
        }
        let name = folder.input.read(cx).value().to_string();
        folder.error = None;
        folder.task = Some(cx.spawn_in(window, async move |this, cx| {
            let made = blocking(move || onb::create_subdir(&dir, &name).map_err(|e| e.to_string()))
                .await
                .and_then(|r| r);
            this.update_in(cx, |view, window, cx| view.folder_made(made, window, cx))
                .ok();
        }));
        cx.notify();
    }

    fn folder_made(
        &mut self,
        made: Result<SystemPath, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match made {
            Ok(dir) => {
                window.focus(&self.focus, cx);
                self.browse(dir, window, cx);
            }
            Err(e) => {
                if let Some(folder) = self.chooser.as_mut().and_then(|c| c.new_folder.as_mut()) {
                    folder.task = None;
                    folder.error = Some(format!("Couldn't create the folder: {e}").into());
                }
            }
        }
        cx.notify();
    }

    // ---- finishing --------------------------------------------------------

    /// The theme the draft names, as the window will show it: the one the
    /// window had when the draft names a theme that is not installed.
    fn chosen_theme(&self) -> Theme {
        self.model
            .as_ref()
            .and_then(|m| m.themes.iter().find(|t| t.name == m.draft.theme_name))
            .cloned()
            .unwrap_or_else(|| self.original.clone())
    }

    /// `Finish` (Enter on the summary): apply the draft off the UI thread.
    pub(crate) fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(model) = self.model.as_ref() else {
            return;
        };
        if self.finishing.is_some() {
            return;
        }
        let (draft, first_run) = (model.draft.clone(), model.first_run);
        let settings = self.settings.clone();
        self.message = None;
        self.finishing = Some(cx.spawn_in(window, async move |this, cx| {
            let applied = blocking(move || {
                onb::apply_draft(&settings, &draft, first_run)
                    .map_err(|e| (e.to_string(), e.return_step()))
            })
            .await;
            let applied = match applied {
                Ok(result) => result,
                Err(e) => Err((e, OnbStep::Summary)),
            };
            this.update_in(cx, |view, window, cx| view.finished(applied, window, cx))
                .ok();
        }));
        cx.notify();
    }

    fn finished(
        &mut self,
        applied: Result<(), (String, OnbStep)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finishing = None;
        match applied {
            Ok(()) => {
                let first_run = self.first_run();
                cx.emit(OnboardingEvent::Finished {
                    theme: self.chosen_theme(),
                    first_run,
                });
            }
            Err((text, step)) => {
                if let Some(model) = self.model.as_mut() {
                    model.step = step;
                }
                self.message = Some(format!("Couldn't finish the setup: {text}").into());
                window.focus(&self.focus, cx);
            }
        }
        cx.notify();
    }

    // ---- keys -------------------------------------------------------------

    /// Enter in a text field (the kit's action, seen first): create the new
    /// folder, or leave the name field forward.
    fn on_field_enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_folder_focused(window, cx) {
            self.create_folder(window, cx);
            cx.stop_propagation();
        } else if self.name_input.read(cx).focus_handle(cx).is_focused(window) {
            self.next(window, cx);
            cx.stop_propagation();
        }
    }

    /// Esc in a text field: close the new folder field, or leave the name
    /// field for the assistant (Esc again cancels).
    fn on_field_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_folder_focused(window, cx) {
            self.close_new_folder(window, cx);
            cx.stop_propagation();
        } else if self.name_input.read(cx).focus_handle(cx).is_focused(window) {
            window.focus(&self.focus, cx);
            cx.notify();
            cx.stop_propagation();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.field_focused(window, cx) {
            return;
        }
        let Some(key) = input::key_down_input(event) else {
            return;
        };
        if self.key(key, window, cx) {
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// The assistant's keys. True when taken.
    pub(crate) fn key(
        &mut self,
        key: KeyInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let plain = key.mods == kimun_notes::logic::input::Mods::NONE;
        if self.finishing.is_some() {
            return true;
        }
        if self.confirm.is_some() {
            match key.key {
                Key::Enter if plain => self.confirm_yes(cx),
                Key::Esc => self.confirm_no(window, cx),
                _ => return false,
            }
            return true;
        }
        if self.chooser.is_some() {
            match key.key {
                Key::Enter if key.mods.ctrl => self.chooser_choose(window, cx),
                Key::Enter | Key::Right if plain => self.chooser_enter_selected(window, cx),
                Key::Left | Key::Backspace if plain => self.chooser_up(window, cx),
                Key::Up if plain => self.chooser_move(false, cx),
                Key::Down if plain => self.chooser_move(true, cx),
                Key::Esc => self.close_chooser(window, cx),
                Key::Char('n') if plain => self.open_new_folder(window, cx),
                _ => return false,
            }
            return true;
        }
        let Some(step) = self.step() else {
            return false;
        };
        if !plain {
            return false;
        }
        match (step, key.key) {
            (_, Key::Esc) => self.cancel(cx),
            (_, Key::Enter | Key::Right) => self.next(window, cx),
            (_, Key::Left) => self.back(window, cx),
            (OnbStep::Workspace, Key::Char('b')) if self.first_run() => {
                self.open_chooser(window, cx)
            }
            (OnbStep::Workspace, Key::Char('e')) if self.first_run() => {
                let handle = self.name_input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
            (OnbStep::Updates, Key::Up) => self.set_update_check(true, cx),
            (OnbStep::Updates, Key::Down) => self.set_update_check(false, cx),
            (OnbStep::Updates, Key::Char(' ')) => {
                let on = self.model.as_ref().is_some_and(|m| m.draft.update_check);
                self.set_update_check(!on, cx)
            }
            (OnbStep::Theme, Key::Up | Key::Down) => {
                let (idx, len) = self
                    .model
                    .as_ref()
                    .map_or((0, 0), |m| (m.theme_idx, m.themes.len()));
                let next = if key.key == Key::Up {
                    idx.checked_sub(1)
                } else {
                    (idx + 1 < len).then_some(idx + 1)
                };
                if let Some(next) = next {
                    self.select_theme(next, cx);
                }
            }
            (OnbStep::Backend, Key::Up) => {
                if let Some(m) = self.model.as_mut() {
                    m.move_backend(-1)
                }
            }
            (OnbStep::Backend, Key::Down) => {
                if let Some(m) = self.model.as_mut() {
                    m.move_backend(1)
                }
            }
            _ => return false,
        }
        true
    }
}

// ---- Rendering ---------------------------------------------------------------

/// A step's title (the TUI's). The desktop app has no Nerd Fonts step
/// (the model is built without it), so that one is never shown.
fn step_title(step: OnbStep) -> &'static str {
    match step {
        OnbStep::Welcome => "Welcome",
        OnbStep::Workspace => "Workspace",
        OnbStep::NerdFonts => "Setup",
        OnbStep::Theme => "Theme",
        OnbStep::Backend => "Editor backend",
        OnbStep::Updates => "Updates",
        OnbStep::Summary => "Summary",
    }
}

/// The Summary's backend value. A configured nvim (not offered here) is
/// kept as it is, and the desktop app runs the plain editor for it.
fn backend_summary(model: &OnboardingModel) -> String {
    match model.selected_backend() {
        Some(_) => model.backend_name().to_string(),
        None => format!(
            "{} (unchanged; the desktop app uses the plain editor)",
            model.backend_name()
        ),
    }
}

/// A paragraph of body text.
fn para(text: impl Into<SharedString>, t: &Theme) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(theme::foreground(t))
        .child(text.into())
}

/// A quieter note under a control.
fn note(text: impl Into<SharedString>, t: &Theme) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(theme::muted(t))
        .child(text.into())
}

impl OnboardingView {
    fn render_header(&self, step: Option<OnbStep>) -> impl IntoElement {
        let t = &self.theme;
        let index = self.model.as_ref().map(OnboardingModel::step_index);
        let (title, index) = match (self.confirm, step) {
            (Some(Confirm::Quit), _) => ("Quit kimün?", index),
            (Some(Confirm::Discard), _) => ("Discard your changes?", index),
            (None, _) if self.chooser.is_some() => ("Choose a folder", index),
            (None, Some(step)) => (step_title(step), index),
            (None, None) => ("Setup", None),
        };
        let total = self.model.as_ref().map_or(0, OnboardingModel::step_count);
        let focus = theme::focus_border(t);
        let rest = theme::divider(t);
        let dots = (0..total).map(move |i| {
            div()
                .flex_none()
                .size(px(8.))
                .rounded_full()
                .bg(if Some(i) == index { focus } else { rest })
        });
        let progress = index.map(|i| format!("Step {} of {total}", i + 1));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::bright(t))
                    .child(title),
            )
            .child(
                div()
                    .id("onboarding-progress")
                    .test_support()
                    .role(Role::Group)
                    .aria_label(SharedString::from(progress.clone().unwrap_or_default()))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .children(dots)
                    .when_some(progress, |row, progress| {
                        row.child(
                            div()
                                .ml_2()
                                .text_xs()
                                .text_color(theme::muted(t))
                                .child(progress),
                        )
                    }),
            )
    }

    fn render_welcome(&self) -> AnyElement {
        let t = &self.theme;
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                // A fixed square, so the layout holds before the PNG is
                // decoded (and in tests, where it may never be); `Contain`
                // keeps the whole mark inside it. The wrapper carries the
                // id and the label: `img` exposes no accessibility node.
                div()
                    .id(LOGO_ID)
                    .test_support()
                    .role(Role::Image)
                    .aria_label("kimün logo")
                    .size(px(112.))
                    .flex_none()
                    .child(
                        img(assets::logo())
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    ),
            )
            .child(
                div()
                    .text_size(rems(3.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme::accent(t))
                    .child("Kimün"),
            )
            .child(
                div()
                    .text_base()
                    .text_color(theme::bright(t))
                    .child("Your notes as plain Markdown files, on your own disk."),
            )
            .child(para(
                "This setup walks you through the essentials: where your notes live, how \
                 the app looks and which editor drives it.",
                t,
            ))
            .child(para(
                "Nothing is applied until you finish the summary. You can run this setup \
                 again from the command palette (\u{201c}guided setup\u{201d}).",
                t,
            ))
            .into_any_element()
    }

    fn render_workspace(&self, model: &OnboardingModel, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        if !model.first_run {
            let rows = self.workspaces.iter().map(|row| {
                div()
                    .id(SharedString::from(format!("onboarding-ws-{}", row.name)))
                    .test_support()
                    .role(Role::ListItem)
                    .aria_label(SharedString::from(row.name.clone()))
                    .flex()
                    .flex_col()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(row.current, |r| r.bg(theme::selected_background(t)))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::foreground(t))
                            .child(if row.current {
                                format!("{} (current)", row.name)
                            } else {
                                row.name.clone()
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(t))
                            .truncate()
                            .child(row.path.clone()),
                    )
            });
            return div()
                .flex()
                .flex_col()
                .gap_3()
                .child(para(
                    "Your workspaces. This setup leaves them unchanged; the window stays on \
                     the current one.",
                    t,
                ))
                .child(
                    div()
                        .id("onboarding-workspaces")
                        .test_support()
                        .role(Role::List)
                        .aria_label("Workspaces")
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(rows),
                )
                .into_any_element();
        }
        let chosen = model.draft.workspace.as_ref();
        let folder = chosen.map_or_else(
            || "No folder chosen".to_string(),
            |(_, path)| system::path_to_string(path),
        );
        let label = |text: &'static str| {
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::muted(t))
                .child(text)
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(para(
                "A workspace is where your notes live: one folder on disk, holding plain \
                 Markdown files. kimün indexes it for search and links.",
                t,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label("Folder"))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("onboarding-folder")
                                    .test_support()
                                    .role(Role::Label)
                                    .aria_label(SharedString::from(folder.clone()))
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .text_color(if chosen.is_some() {
                                        theme::foreground(t)
                                    } else {
                                        theme::muted(t)
                                    })
                                    .child(folder),
                            )
                            .child(
                                Button::new(CHOOSE_FOLDER_ID)
                                    .label("Choose folder…")
                                    .disabled(self.busy())
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.open_chooser(window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label("Name"))
                    .child(
                        Input::new(&self.name_input)
                            .id(NAME_ID)
                            .aria_label("Workspace name")
                            .disabled(chosen.is_none() || self.busy())
                            .bg(theme::background(t))
                            .border_color(theme::divider(t))
                            .text_color(theme::foreground(t)),
                    )
                    .child(match &self.name_error {
                        Some(error) => div()
                            .text_xs()
                            .text_color(theme::error(t))
                            .child(error.clone())
                            .into_any_element(),
                        None => note("It follows the folder's name until you edit it.", t)
                            .into_any_element(),
                    }),
            )
            .into_any_element()
    }

    fn render_theme(&self, model: &OnboardingModel, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let count = model.themes.len();
        let list = uniform_list(
            THEME_LIST_ID,
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let Some(model) = this.model.as_ref() else {
                    return Vec::new();
                };
                let t = this.theme.clone();
                let hover = theme::hover_background(&t);
                range
                    .filter_map(|ix| model.themes.get(ix).map(|theme| (ix, theme.name.clone())))
                    .map(|(ix, name)| {
                        let selected = model.theme_idx == ix;
                        div()
                            .id(theme_row_id(&name))
                            .test_support()
                            .role(Role::ListBoxOption)
                            .aria_label(SharedString::from(name.clone()))
                            .aria_selected(selected)
                            .flex()
                            .items_center()
                            .w_full()
                            .h(rems(1.75))
                            .px_2()
                            .rounded_md()
                            .text_sm()
                            .text_color(theme::foreground(&t))
                            .when(selected, |row| {
                                row.bg(theme::selected_background(&t))
                                    .font_weight(FontWeight::SEMIBOLD)
                            })
                            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
                            .child(name)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.select_theme(ix, cx)
                            }))
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.theme_scroll)
        .size_full();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .min_h_0()
            .child(para(
                "The colour theme for the whole app. The window shows your selection as \
                 you choose; custom themes go in the themes folder of kimün's config.",
                t,
            ))
            .child(
                div()
                    .id("onboarding-theme-box")
                    .role(Role::ListBox)
                    .aria_label("Themes")
                    .flex_1()
                    .min_h(rems(8.))
                    .h(rems(14.))
                    .p_1()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::divider(t))
                    .child(list),
            )
            .into_any_element()
    }

    fn render_backend(&self, model: &OnboardingModel, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let radios = model
            .backends()
            .iter()
            .map(|(_, name, desc)| Radio::new(*name).label(*name).child(note(*desc, t)));
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(para("Which engine drives the note editor.", t))
            .child(
                RadioGroup::vertical("onboarding-backends")
                    .selected_index(model.selected_backend())
                    .on_click(cx.listener(|this, ix: &usize, _, cx| this.select_backend(*ix, cx)))
                    .children(radios),
            )
            .into_any_element()
    }

    fn render_updates(&self, model: &OnboardingModel, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(para(
                "kimün can check GitHub for a newer release on startup and show a small \
                 notice. Nothing is sent: it only reads the public list of releases. The \
                 terminal app checks today; the desktop app does not check yet.",
                t,
            ))
            .child(
                RadioGroup::vertical("onboarding-updates")
                    .selected_index(Some(if model.draft.update_check { 0 } else { 1 }))
                    .on_click(
                        cx.listener(|this, ix: &usize, _, cx| this.set_update_check(*ix == 0, cx)),
                    )
                    .child(Radio::new("on").label("On — check for updates on startup"))
                    .child(Radio::new("off").label("Off — never check")),
            )
            .into_any_element()
    }

    fn render_summary(&self, model: &OnboardingModel) -> AnyElement {
        let t = &self.theme;
        let current = self
            .workspaces
            .iter()
            .find(|w| w.current)
            .map(|w| w.name.clone())
            .unwrap_or_default();
        let workspace = match (&model.draft.workspace, model.first_run) {
            (Some((name, path)), _) => format!("{name} — {}", system::path_to_string(path)),
            (None, false) => format!("{current} (unchanged)"),
            (None, true) => "Not set up — go back to the Workspace step".to_string(),
        };
        let on_off = |on: bool| if on { "On" } else { "Off" };
        let rows = [
            ("Workspace", workspace),
            ("Theme", model.draft.theme_name.clone()),
            ("Editor backend", backend_summary(model)),
            ("Updates", on_off(model.draft.update_check).to_string()),
        ];
        let rows = rows.into_iter().map(|(key, value)| {
            div()
                .id(SharedString::from(format!("onboarding-summary-{key}")))
                .test_support()
                .role(Role::Label)
                .aria_label(SharedString::from(format!("{key}: {value}")))
                .flex()
                .flex_row()
                .gap_3()
                .child(
                    div()
                        .flex_none()
                        .w(rems(8.))
                        .text_sm()
                        .text_color(theme::muted(t))
                        .child(key),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(theme::foreground(t))
                        .child(value),
                )
        });
        let mut notes = vec![
            "Finish applies everything at once. The theme and the editor take effect \
             right away."
                .to_string(),
            "The update check is used by the terminal app.".to_string(),
        ];
        if model.first_run {
            notes.push("The folder is created if it does not exist yet.".to_string());
        }
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(para("Review your choices.", t))
            .child(div().flex().flex_col().gap_2().children(rows))
            .children(notes.into_iter().map(|n| note(n, t)))
            .into_any_element()
    }

    fn render_confirm(&self, confirm: Confirm) -> AnyElement {
        let t = &self.theme;
        let text = match confirm {
            Confirm::Quit => "No workspace is set up yet, and kimün cannot run without one.",
            Confirm::Discard => "Your setup changes have not been applied.",
        };
        para(text, t).into_any_element()
    }

    fn render_chooser(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let Some(chooser) = self.chooser.as_ref() else {
            return div().into_any_element();
        };
        let path = system::path_to_string(&chooser.dir);
        let has_parent = chooser.dir.parent().is_some();
        let body = match &chooser.listing {
            Listing::Loading => note("Loading…", t).into_any_element(),
            Listing::Failed(e) => div()
                .id("onboarding-chooser-error")
                .test_support()
                .role(Role::Label)
                .aria_label(SharedString::from(e.clone()))
                .text_sm()
                .text_color(theme::error(t))
                .child(format!("Couldn't list this folder: {e}"))
                .into_any_element(),
            Listing::Loaded(rows) if rows.is_empty() => {
                note("No folders here. Choose this one, or make a new one.", t).into_any_element()
            }
            Listing::Loaded(rows) => uniform_list(
                CHOOSER_LIST_ID,
                rows.len(),
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    let Some(chooser) = this.chooser.as_ref() else {
                        return Vec::new();
                    };
                    let t = this.theme.clone();
                    let hover = theme::hover_background(&t);
                    range
                        .filter_map(|ix| chooser.rows().get(ix).map(|d| (ix, d.clone())))
                        .map(|(ix, dir)| {
                            let selected = chooser.selected == Some(ix);
                            let name = folder_name(&dir);
                            div()
                                .id(chooser_row_id(&dir))
                                .test_support()
                                .role(Role::ListBoxOption)
                                .aria_label(SharedString::from(name.clone()))
                                .aria_selected(selected)
                                .flex()
                                .items_center()
                                .w_full()
                                .h(rems(1.75))
                                .px_2()
                                .rounded_md()
                                .text_sm()
                                .text_color(theme::directory(&t))
                                .when(selected, |row| row.bg(theme::selected_background(&t)))
                                .when(!selected, |row| row.hover(move |style| style.bg(hover)))
                                .child(div().truncate().child(name))
                                .on_click(cx.listener(
                                    move |this, event: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        if event.click_count() >= 2 {
                                            // The first click entered it: take it.
                                            this.chooser_choose(window, cx);
                                        } else {
                                            this.browse(dir.clone(), window, cx);
                                        }
                                    },
                                ))
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&chooser.scroll)
            .size_full()
            .into_any_element(),
        };
        let new_folder = chooser.new_folder.as_ref().map(|folder| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&folder.input)
                                    .id(NEW_FOLDER_ID)
                                    .aria_label("New folder name")
                                    .disabled(folder.task.is_some())
                                    .bg(theme::background(t))
                                    .border_color(theme::divider(t))
                                    .text_color(theme::foreground(t)),
                            ),
                        )
                        .child(
                            Button::new(NEW_FOLDER_CREATE_ID)
                                .label("Create")
                                .loading(folder.task.is_some())
                                .disabled(folder.task.is_some())
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.create_folder(window, cx)
                                })),
                        ),
                )
                .when_some(folder.error.clone(), |col, error| {
                    col.child(div().text_xs().text_color(theme::error(t)).child(error))
                })
        });
        div()
            .id(CHOOSER_ID)
            .test_support()
            .role(Role::Group)
            .aria_label("Choose a folder")
            .flex()
            .flex_col()
            .gap_3()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new(CHOOSER_UP_ID)
                            .label("Up")
                            .disabled(!has_parent)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.chooser_up(window, cx)
                            })),
                    )
                    .child(
                        div()
                            .id("onboarding-chooser-path")
                            .test_support()
                            .role(Role::Label)
                            .aria_label(SharedString::from(path.clone()))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(theme::foreground(t))
                            .child(path),
                    ),
            )
            .children(new_folder)
            .child(
                div()
                    .id("onboarding-chooser-rows")
                    .role(Role::ListBox)
                    .aria_label("Folders")
                    .flex_1()
                    .min_h(rems(8.))
                    .h(rems(14.))
                    .p_1()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::divider(t))
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            this.chooser_choose(window, cx);
                        }
                    }))
                    .child(body),
            )
            .into_any_element()
    }

    /// The key hints under the body: what the keyboard does here.
    fn hints(&self, step: Option<OnbStep>) -> &'static str {
        if self.confirm.is_some() {
            return "Enter: yes · Esc: back to the setup";
        }
        if self.chooser.is_some() {
            return "↑/↓: select · Enter: open · ←: up · Ctrl+Enter: choose · N: new folder · Esc: back";
        }
        match step {
            Some(OnbStep::Welcome) => "Enter: start · ←/→: steps · Esc: cancel",
            Some(OnbStep::Workspace) if self.first_run() => {
                "Enter: next · B: choose folder · E: edit name · ←/→: steps · Esc: cancel"
            }
            Some(OnbStep::Summary) => "Enter: finish · ←: back · Esc: cancel",
            Some(OnbStep::Updates) => {
                "↑/↓: select · Space: toggle · Enter/→: next · ←: back · Esc: cancel"
            }
            Some(OnbStep::Theme | OnbStep::Backend) => {
                "↑/↓: select · Enter/→: next · ←: back · Esc: cancel"
            }
            _ => "Enter/→: next · ←: back · Esc: cancel",
        }
    }

    fn render_footer(&self, step: Option<OnbStep>, cx: &mut Context<Self>) -> AnyElement {
        let row = div().flex().flex_row().items_center().gap_2().flex_wrap();
        if let Some(confirm) = self.confirm {
            let yes = match confirm {
                Confirm::Quit => "Quit kimün",
                Confirm::Discard => "Discard changes",
            };
            return row
                .justify_end()
                .child(Button::new(CONFIRM_NO_ID).label("Back to setup").on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.confirm_no(window, cx)),
                ))
                .child(
                    Button::new(CONFIRM_YES_ID)
                        .danger()
                        .label(yes)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.confirm_yes(cx))),
                )
                .into_any_element();
        }
        if self.chooser.is_some() {
            let creating = self
                .chooser
                .as_ref()
                .and_then(|c| c.new_folder.as_ref())
                .is_some_and(|f| f.task.is_some());
            return row
                .child(
                    Button::new(CHOOSER_NEW_ID)
                        .label("New folder…")
                        .disabled(creating)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.open_new_folder(window, cx)
                        })),
                )
                .child(div().flex_1())
                .child(Button::new(CHOOSER_CANCEL_ID).label("Cancel").on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.close_chooser(window, cx)),
                ))
                .child(
                    Button::new(CHOOSER_CHOOSE_ID)
                        .primary()
                        .label("Choose this folder")
                        .disabled(creating)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.chooser_choose(window, cx)
                        })),
                )
                .into_any_element();
        }
        let busy = self.busy();
        let finishing = self.finishing.is_some();
        let summary = step == Some(OnbStep::Summary);
        let blocked = self.model.as_ref().is_some_and(|m| {
            !m.can_advance()
                || (m.step == OnbStep::Workspace && m.first_run && self.name_error.is_some())
        });
        row.child(
            Button::new(CANCEL_ID)
                .ghost()
                .label("Cancel")
                .disabled(busy)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel(cx))),
        )
        .child(div().flex_1())
        .child(
            Button::new(BACK_ID)
                .label("Back")
                .disabled(busy || step == Some(OnbStep::Welcome))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.back(window, cx))),
        )
        .child(
            Button::new(NEXT_ID)
                .primary()
                .label(if summary { "Finish" } else { "Next" })
                .loading(finishing)
                .disabled(busy || blocked)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.next(window, cx))),
        )
        .into_any_element()
    }
}

impl Render for OnboardingView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.clone();
        let step = self.step();
        let body = match (self.confirm, self.model.as_ref()) {
            (Some(confirm), _) => self.render_confirm(confirm),
            (None, _) if self.chooser.is_some() => self.render_chooser(cx),
            (None, None) => note("Getting the setup ready…", &t).into_any_element(),
            (None, Some(model)) => match model.step {
                OnbStep::Welcome => self.render_welcome(),
                OnbStep::Workspace => self.render_workspace(model, cx),
                // Never shown: the desktop app's model has no Nerd Fonts step.
                OnbStep::NerdFonts => div().into_any_element(),
                OnbStep::Theme => self.render_theme(model, cx),
                OnbStep::Backend => self.render_backend(model, cx),
                OnbStep::Updates => self.render_updates(model, cx),
                OnbStep::Summary => self.render_summary(model),
            },
        };
        let message = self.message.clone();
        div()
            .id(ONBOARDING_ID)
            .test_support()
            .role(Role::Group)
            .aria_label("Setup")
            .key_context("KimunOnboarding")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .capture_action(cx.listener(Self::on_field_enter))
            .capture_action(cx.listener(Self::on_field_escape))
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            // The 16 px side gutter of a narrow window.
            .p_4()
            .bg(theme::background(&t))
            .text_color(theme::foreground(&t))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .w_full()
                    .max_w(rems(40.))
                    .h_full()
                    .max_h(rems(38.))
                    .p_6()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme::divider(&t))
                    .bg(theme::panel_background(&t))
                    .child(self.render_header(step))
                    .child(
                        div()
                            .id("onboarding-body")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(body),
                    )
                    .when_some(message, |card, message| {
                        card.child(
                            div()
                                .id(MESSAGE_ID)
                                .test_support()
                                .role(Role::Label)
                                .aria_label(message.clone())
                                .text_sm()
                                .text_color(theme::error(&t))
                                .child(message),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(&t))
                            .child(self.hints(step)),
                    )
                    .child(self.render_footer(step, cx)),
            )
    }
}

// ---- The window's side ---------------------------------------------------------

/// What the editor Panel says while the vault the assistant set up opens.
fn opening_state() -> EmptyState {
    EmptyState {
        title: "Opening the workspace…".into(),
        detail: String::new(),
        failed: false,
    }
}

impl Workspace {
    /// Whether the setup assistant has the window.
    pub(crate) fn onboarding_open(&self) -> bool {
        self.onboarding.is_some()
    }

    /// Open the setup assistant over the window (a first run when the
    /// settings name no workspace), or give it the keyboard when it is
    /// already open. Modals and a pending leader sequence go. Needs the
    /// tokio runtime entered.
    pub(crate) fn open_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.onboarding.is_none() {
            let Some(settings) = self.settings.clone() else {
                return;
            };
            if super::modal::dialog_open(window, cx) {
                self.close_modal(window, cx);
            }
            self.cancel_leader(cx);
            let original = self.theme.clone();
            let view = cx.new(|cx| OnboardingView::new(settings, original, window, cx));
            self._onboarding_events =
                Some(cx.subscribe_in(&view, window, Self::on_onboarding_event));
            self.onboarding = Some(view);
        }
        self.apply_focus(window, cx);
        // A modal closing gives the keyboard back to its opener once it is
        // gone: take it again after that.
        cx.defer_in(window, |this, window, cx| this.apply_focus(window, cx));
        cx.notify();
    }

    /// Take the assistant down; the keyboard goes back to the layout.
    fn close_onboarding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.onboarding = None;
        self._onboarding_events = None;
        self.apply_focus(window, cx);
        cx.notify();
    }

    /// The window entered a vault: a setup assistant still up has nothing
    /// left to set up.
    pub(super) fn drop_onboarding(&mut self) {
        self.onboarding = None;
        self._onboarding_events = None;
    }

    fn on_onboarding_event(
        &mut self,
        _: &Entity<OnboardingView>,
        event: &OnboardingEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            OnboardingEvent::Preview(t) => self.apply_theme(t.clone(), cx),
            OnboardingEvent::Close(t) => {
                self.apply_theme(t.clone(), cx);
                self.close_onboarding(window, cx);
            }
            OnboardingEvent::Quit => self.request_close(window, cx),
            OnboardingEvent::Finished {
                theme: t,
                first_run,
            } => {
                self.apply_theme(t.clone(), cx);
                self.close_onboarding(window, cx);
                if *first_run {
                    self.empty = Some(opening_state());
                }
                self.persist_settings(cx);
                match self.tx.clone() {
                    Some(tx) if tx.send(AppMessage::OnboardingFinished).is_ok() => {}
                    _ => self.reopen(cx),
                }
            }
        }
    }

    /// Show the whole window in `t` from now on: the workspace and the
    /// editor redraw in it, and the kit's components (dialogs, buttons,
    /// fields) take its colours. The drawer views built over the vault
    /// keep theirs until they are built again (a vault change rebuilds
    /// them); while the assistant shows they are not drawn.
    pub(crate) fn apply_theme(&mut self, t: Theme, cx: &mut Context<Self>) {
        self.editor.update(cx, |view, cx| {
            view.theme = t.clone();
            cx.notify();
        });
        gpui_kit::component::Theme::update(cx, |kit| theme::apply_to_kit(&t, kit));
        self.theme = t;
        cx.notify();
    }
}
