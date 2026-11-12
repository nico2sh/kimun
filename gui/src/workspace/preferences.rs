//! **Preferences** (M9b): the TUI's Preferences screen
//! (`app_screen/preferences.rs`) as a full-window view that takes the
//! Workspace body over while it shows, as the setup assistant does
//! ([`onboarding`](super::onboarding)): the rail, the slots and the editor
//! are not drawn, and the window's chords (leader, palette, side toggles)
//! do nothing.
//!
//! **Layout.** A section list on the left (Workspaces, Appearance, Display,
//! Sorting, Indexing, Editor, Server: the TUI's order, and like the TUI they
//! open on Workspaces), the selected section's controls on the right, a
//! footer with `Cancel` and `Save`. In a narrow window the section list
//! becomes a row of section tabs above the controls, so the controls keep
//! the width.
//!
//! **Workspaces and Indexing** ([`workspaces`]) act at once, outside the
//! draft: a workspace change is written as it is made, a reindex runs as it
//! is asked for. A vault whose files differ only by case (found by a
//! reindex, or by the window opening it) is shown in a panel over the
//! sections.
//!
//! **State.** The draft and its rules (ranges, steps, cycles, which settings
//! the desktop app shows) are the logic `PreferencesModel`'s; this view
//! draws it and maps keys and clicks onto it. The model is built off the UI
//! thread (it reads the theme folder), without the nvim backend (the
//! desktop app has no nvim engine: a configured nvim survives untouched
//! until another backend is chosen). Nerd Fonts and mouse capture are
//! terminal-only and not shown; the draft carries them unchanged. Display's
//! `Reduce motion` is the desktop app's own (`[gui] reduce_motion`, which the
//! TUI never shows): applied as soon as it is saved, no restart.
//!
//! **Save** is disabled while the draft is clean or invalid (a server
//! address that is not `http://` / `https://`). It writes the settings with
//! the draft applied through the window's one settings write (off the UI
//! thread, `[gui]` kept, an unparsable config never replaced). Only once the
//! file is written does the draft go into the shared settings and the view
//! close; a failed write is shown in the view, which stays open with the
//! settings untouched. What changes then, and how it takes effect, is the
//! window's ([`Workspace::apply_saved_preferences`]). A save whose plan
//! needs the vault indexed first takes the vault-path reindex flow.
//!
//! **Cancel**, Esc and the window's own close: at once when the draft is
//! clean; otherwise an inline "Discard your changes?" (Keep editing /
//! Discard changes). Esc never saves. Leaving without saving puts the theme
//! the window had back (the Appearance section previews live).

use crate::tasks::blocking;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState};
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role,
    ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Subscription, Task,
    UniformListScrollHandle, Window, div, rems, uniform_list,
};
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout_tree::RailPosition;
use kimun_notes::logic::message::AppMessage;
use kimun_notes::logic::preferences::{
    AUTOSAVE_MAX_SECS, AUTOSAVE_MIN_SECS, DEFAULT_SERVER_URL_HINT, LEADER_DELAY_MAX_MS,
    LEADER_DELAY_MIN_MS, PreferencesModel, SortSlot, backend_label, configured_server_url,
};
use kimun_notes::settings::themes::Theme;
use kimun_notes::settings::{AppSettings, EditorBackendSetting, SharedSettings};
use std::cell::Cell;

use super::note::Closing;
use super::{LayoutStore, Workspace, reopen};
use kimun_notes::logic::preferences::SavePlan;

pub(crate) mod workspaces;
use crate::input;
use crate::theme;

/// The view's root element.
pub(crate) const PREFERENCES_ID: &str = "preferences";
/// The footer's commands.
pub(crate) const SAVE_ID: &str = "preferences-save";
pub(crate) const CANCEL_ID: &str = "preferences-cancel";
/// The discard question's answers.
pub(crate) const KEEP_EDITING_ID: &str = "preferences-keep-editing";
pub(crate) const DISCARD_ID: &str = "preferences-discard";
/// The inline problem line (a failed save).
pub(crate) const MESSAGE_ID: &str = "preferences-message";
/// The section list.
pub(crate) const SECTIONS_ID: &str = "preferences-sections";
/// The controls.
pub(crate) const THEMES_ID: &str = "preferences-themes";
pub(crate) const UPDATE_CHECK_ID: &str = "preferences-update-check";
/// Display's `Check now`: a manual update check.
pub(crate) const CHECK_UPDATES_ID: &str = "preferences-check-updates";
/// The update check's help text, and its element.
pub(crate) const UPDATE_CHECK_HELP_ID: &str = "preferences-update-check-help";
pub(crate) const UPDATE_CHECK_HELP: &str =
    "Checks GitHub for a newer release on startup. Check now looks right away.";
/// Display's `Reduce motion` (`[gui] reduce_motion`), its help text and the
/// help's element. GPUI reads no system hint, hence the second sentence.
pub(crate) const REDUCE_MOTION_ID: &str = "preferences-reduce-motion";
pub(crate) const REDUCE_MOTION_HELP_ID: &str = "preferences-reduce-motion-help";
pub(crate) const REDUCE_MOTION_HELP: &str =
    "Stops the bouncing logo and loading spinners. Not read from your system settings.";
/// Display's `Rail position` (`[gui] rail`, ADR-0055) and its help text.
pub(crate) const RAIL_HELP_ID: &str = "preferences-rail-help";
pub(crate) const RAIL_HELP: &str =
    "Where the activity rail sits. Hide or show it with the button at the start of the footer.";
pub(crate) const DELAY_ID: &str = "preferences-leader-delay";
pub(crate) const DELAY_DOWN_ID: &str = "preferences-leader-delay-down";
pub(crate) const DELAY_UP_ID: &str = "preferences-leader-delay-up";
pub(crate) const AUTOSAVE_ID: &str = "preferences-autosave";
pub(crate) const AUTOSAVE_DOWN_ID: &str = "preferences-autosave-down";
pub(crate) const AUTOSAVE_UP_ID: &str = "preferences-autosave-up";
pub(crate) const BACKENDS_ID: &str = "preferences-backends";
/// The note that a configured nvim is kept.
pub(crate) const BACKEND_KEPT_ID: &str = "preferences-backend-kept";
pub(crate) const SERVER_ID: &str = "preferences-server";
pub(crate) const SERVER_ERROR_ID: &str = "preferences-server-error";
const THEME_LIST_ID: &str = "preferences-theme-list";

/// Below this window width (in rems) the section list becomes a row of
/// section tabs above the controls.
const NARROW_REMS: f32 = 44.;

/// What a configured nvim reads as: the desktop app runs the plain editor
/// for it (as the setup assistant says).
pub(crate) const NVIM_UNCHANGED: &str = "nvim (unchanged; the desktop app uses the plain editor)";

/// One section of the view, in the TUI's sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Workspaces,
    Appearance,
    Display,
    Sorting,
    Indexing,
    Editor,
    Server,
}

impl Section {
    pub(crate) const ALL: [Section; 7] = [
        Section::Workspaces,
        Section::Appearance,
        Section::Display,
        Section::Sorting,
        Section::Indexing,
        Section::Editor,
        Section::Server,
    ];

    fn title(self) -> &'static str {
        match self {
            Section::Workspaces => "Workspaces",
            Section::Indexing => "Indexing",
            Section::Appearance => "Appearance",
            Section::Display => "Display",
            Section::Sorting => "Sorting",
            Section::Editor => "Editor",
            Section::Server => "Server",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Section::Workspaces => "workspaces",
            Section::Indexing => "indexing",
            Section::Appearance => "appearance",
            Section::Display => "display",
            Section::Sorting => "sorting",
            Section::Editor => "editor",
            Section::Server => "server",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// The element id of `section`'s row in the section list.
pub(crate) fn section_id(section: Section) -> SharedString {
    format!("preferences-section-{}", section.slug()).into()
}

/// The element id of the Appearance section's row for `name`.
pub(crate) fn theme_row_id(name: &str) -> SharedString {
    format!("preferences-theme-{name}").into()
}

/// Whether saving the preferences from `before` to `after` needs the vault
/// entered again ([`Workspace::reopen`], through `PreferencesSaved`, as the
/// TUI does on every save). **Only** for what the window cannot change in
/// place:
///
/// - the server address: the server sync is per vault entry and restarts
///   with it;
/// - the editor engine (plain ↔ vim): a new engine starts empty, so the open
///   note is saved first and opened again, which is what the vault change
///   does. A configured nvim and plain are the same engine here.
///
/// Every other setting applies in place, and nothing is reopened for it (no
/// flush, no restore, the views stay).
pub(crate) fn needs_reopen(before: &AppSettings, after: &AppSettings) -> bool {
    configured_server_url(before) != configured_server_url(after)
        || crate::uses_vim(before.editor_backend) != crate::uses_vim(after.editor_backend)
}

/// What the view asks of the window.
pub(crate) enum PreferencesEvent {
    /// Show the whole window in this theme (a theme was selected).
    Preview(Theme),
    /// Leave without saving: the window goes back to `theme`; then close the
    /// window when its own close asked for this.
    Close { theme: Theme, close_window: bool },
    /// The draft is in the shared settings and on disk: show `theme`, apply
    /// what changed, and reopen the vault when `reopen` ([`needs_reopen`]).
    Saved { theme: Theme, reopen: bool },
    /// Leave (the draft goes; the window back to `theme`) and switch to
    /// workspace `name` (M8b).
    Switch { theme: Theme, name: String },
    /// A workspace change is in the shared settings and on disk that moves
    /// the window to another vault (the current workspace's new folder, the
    /// first workspace): leave, back to `theme`, and enter it.
    Reopen { theme: Theme },
    /// The open workspace was reindexed: pick the new index up.
    Reindexed,
    /// The current workspace is called `0` now.
    Renamed(String),
    /// Leave, back to `theme`, and run the setup assistant.
    RunSetup { theme: Theme },
    /// Display's `Check now`: check for an update now (the view stays).
    CheckForUpdates,
}

/// The "Discard your changes?" question, while it is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Discard {
    /// The window's close asked: discarding closes the window too.
    pub(crate) close_window: bool,
}

/// The Preferences view. See the module docs.
pub(crate) struct PreferencesView {
    settings: SharedSettings,
    store: LayoutStore,
    /// The theme the window had when the view opened.
    original: Theme,
    /// `None` while it is being read.
    model: Option<PreferencesModel>,
    /// The settings as the draft was read from them: a save whose vault is
    /// not this one's any more takes the vault-path reindex flow.
    opened: Option<AppSettings>,
    /// [`Self::is_dirty`]'s answer until the draft or the settings change
    /// (rendering asks every frame): cleared by [`Self::model_mut`], a new
    /// model and a save.
    dirty: Cell<Option<bool>>,
    section: Section,
    /// The root's handle: the keyboard rests here while the question is
    /// asked. Not a Tab stop.
    focus: FocusHandle,
    /// The section list's, the theme list's and the two steppers' handles:
    /// Tab stops, each with its own arrow keys.
    sections_focus: FocusHandle,
    themes_focus: FocusHandle,
    delay_focus: FocusHandle,
    autosave_focus: FocusHandle,
    server_input: Entity<InputState>,
    server_error: Option<SharedString>,
    /// The inline problem line.
    message: Option<SharedString>,
    confirm: Option<Discard>,
    /// The write under way: every control waits for it.
    saving: Option<Task<()>>,
    theme_scroll: UniformListScrollHandle,
    /// Preferences > Workspaces ([`workspaces`]).
    ws: workspaces::Workspaces,
    /// Preferences > Indexing, the reindex under way and its panels.
    index: workspaces::Indexing,
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PreferencesEvent> for PreferencesView {}

impl Focusable for PreferencesView {
    /// Where the keyboard goes when the view gets it: the section list, or
    /// the root while a question or the progress panel holds the view.
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        let holding = self.confirm.is_some()
            || self.ws.ask.is_some()
            || self.index.confirm_full
            || self.index.running.is_some()
            || matches!(self.ws.form, Some(workspaces::Form::Delete(_)));
        if holding {
            self.focus.clone()
        } else {
            self.sections_focus.clone()
        }
    }
}

impl PreferencesView {
    /// The view over `settings`, the window showing `original` now, saving
    /// to `store`. Its model is read in the background. Needs the tokio
    /// runtime entered.
    pub(crate) fn new(
        settings: SharedSettings,
        original: Theme,
        store: LayoutStore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let server_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(DEFAULT_SERVER_URL_HINT));
        let subscriptions = vec![
            cx.subscribe(&server_input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.server_changed(cx);
                }
            }),
        ];
        let read = settings.clone();
        let load = cx.spawn_in(window, async move |this, cx| {
            let model = blocking(move || {
                let s = read.read().unwrap_or_else(|e| e.into_inner());
                // The desktop app has no nvim engine: not offered.
                PreferencesModel::new(&s).without_backend(EditorBackendSetting::Nvim)
            })
            .await;
            this.update_in(cx, |view, window, cx| view.loaded(model, window, cx))
                .ok();
        });
        Self {
            settings,
            store,
            original,
            model: None,
            opened: None,
            dirty: Cell::new(None),
            section: Section::Workspaces,
            focus: cx.focus_handle(),
            sections_focus: cx.focus_handle().tab_stop(true),
            themes_focus: cx.focus_handle().tab_stop(true),
            delay_focus: cx.focus_handle().tab_stop(true),
            autosave_focus: cx.focus_handle().tab_stop(true),
            server_input,
            server_error: None,
            message: None,
            confirm: None,
            saving: None,
            theme_scroll: UniformListScrollHandle::new(),
            ws: workspaces::Workspaces::new(cx.focus_handle().tab_stop(true)),
            index: workspaces::Indexing::new(),
            _load: Some(load),
            _subscriptions: subscriptions,
        }
    }

    fn loaded(
        &mut self,
        model: Result<PreferencesModel, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._load = None;
        match model {
            Ok(model) => {
                let url = model.server_url.clone().unwrap_or_default();
                self.theme_scroll
                    .scroll_to_item(model.theme_idx, ScrollStrategy::Nearest);
                self.model = Some(model);
                self.opened = Some(self.initial());
                self.ws.selected =
                    kimun_notes::logic::workspaces::initial_selection(&self.ws_rows()).unwrap_or(0);
                self.dirty.set(None);
                self.server_input
                    .update(cx, |input, cx| input.set_value(url, window, cx));
                self.server_error = None;
            }
            Err(e) => self.message = Some(format!("Couldn't read the preferences: {e}").into()),
        }
        cx.notify();
    }

    // ---- reading (the tests) ----------------------------------------------

    /// The draft, once read.
    #[cfg(test)]
    pub(crate) fn model(&self) -> Option<&PreferencesModel> {
        self.model.as_ref()
    }

    /// The section shown.
    #[cfg(test)]
    pub(crate) fn section(&self) -> Section {
        self.section
    }

    /// The question asked, while one is.
    #[cfg(test)]
    pub(crate) fn confirm(&self) -> Option<Discard> {
        self.confirm
    }

    /// The inline problem line.
    #[cfg(test)]
    pub(crate) fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// The server field's problem.
    #[cfg(test)]
    pub(crate) fn server_error(&self) -> Option<&str> {
        self.server_error.as_deref()
    }

    /// The server field.
    #[cfg(test)]
    pub(crate) fn server_input(&self) -> &Entity<InputState> {
        &self.server_input
    }

    /// Whether the settings are being written.
    #[cfg(test)]
    pub(crate) fn saving(&self) -> bool {
        self.saving.is_some()
    }

    /// The handles of the section list, the theme list, the two steppers
    /// and the workspace list, in that order.
    #[cfg(test)]
    pub(crate) fn handles(&self) -> [FocusHandle; 5] {
        [
            self.sections_focus.clone(),
            self.themes_focus.clone(),
            self.delay_focus.clone(),
            self.autosave_focus.clone(),
            self.ws.focus.clone(),
        ]
    }

    /// The root's handle (the questions and the progress panel hold it).
    #[cfg(test)]
    pub(crate) fn root_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn initial(&self) -> AppSettings {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The theme the window had when the view opened.
    pub(crate) fn original_theme(&self) -> &Theme {
        &self.original
    }

    /// Whether the draft differs from the settings.
    pub(crate) fn is_dirty(&self) -> bool {
        if let Some(dirty) = self.dirty.get() {
            return dirty;
        }
        let dirty = self
            .model
            .as_ref()
            .is_some_and(|m| m.dirty(&self.settings.read().unwrap_or_else(|e| e.into_inner())));
        self.dirty.set(Some(dirty));
        dirty
    }

    /// The draft, for editing: whatever the edit does, [`Self::is_dirty`]
    /// is worked out again. Every change to the draft goes through here.
    fn model_mut(&mut self) -> Option<&mut PreferencesModel> {
        self.dirty.set(None);
        self.model.as_mut()
    }

    /// Whether `Save` would save now: a changed, valid draft, no write under
    /// way.
    pub(crate) fn can_save(&self) -> bool {
        !self.busy() && self.server_error.is_none() && self.is_dirty()
    }

    fn server_focused(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        self.server_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }

    // ---- editing the draft ------------------------------------------------

    /// Show `section`; the keyboard goes to the section list.
    pub(crate) fn show_section(
        &mut self,
        section: Section,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.section = section;
        window.focus(&self.sections_focus, cx);
        if section == Section::Appearance
            && let Some(model) = self.model.as_ref()
        {
            self.theme_scroll
                .scroll_to_item(model.theme_idx, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    /// Up / Down in the section list: the section before or after (clamped).
    fn move_section(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.section.index();
        let next = if down {
            (ix + 1).min(Section::ALL.len() - 1)
        } else {
            ix.saturating_sub(1)
        };
        self.show_section(Section::ALL[next], window, cx);
    }

    /// Select theme row `idx` and show the window in it.
    pub(crate) fn select_theme(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(model) = self.model_mut() else {
            return;
        };
        if let Some(t) = model.select_theme(idx) {
            let t = t.clone();
            self.theme_scroll
                .scroll_to_item(idx, ScrollStrategy::Nearest);
            cx.emit(PreferencesEvent::Preview(t));
        }
        cx.notify();
    }

    /// Up / Down in the theme list (wrapping, as the TUI's).
    fn move_theme(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some(model) = self.model_mut() else {
            return;
        };
        if model.move_theme(if down { 1 } else { -1 }).is_some() {
            let idx = model.theme_idx;
            self.select_theme(idx, cx);
        }
    }

    pub(crate) fn set_update_check(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(model) = self.model_mut() {
            model.update_check = on;
        }
        cx.notify();
    }

    /// Display's `Reduce motion`.
    pub(crate) fn set_reduce_motion(&mut self, on: bool, cx: &mut Context<Self>) {
        if let Some(model) = self.model_mut() {
            model.reduce_motion = on;
        }
        cx.notify();
    }

    /// Display's `Rail position`.
    pub(crate) fn set_rail_position(&mut self, rail: RailPosition, cx: &mut Context<Self>) {
        if let Some(model) = self.model_mut() {
            model.rail = rail;
        }
        cx.notify();
    }

    /// One step of the which-key delay.
    pub(crate) fn step_delay(&mut self, up: bool, cx: &mut Context<Self>) {
        if let Some(model) = self.model_mut() {
            model.step_leader_delay(up);
        }
        cx.notify();
    }

    /// One step of the autosave interval.
    pub(crate) fn step_autosave(&mut self, up: bool, cx: &mut Context<Self>) {
        if let Some(model) = self.model_mut() {
            model.step_autosave(up);
        }
        cx.notify();
    }

    /// The choice `ix` of one sort setting (the model's `sort_choices`).
    pub(crate) fn set_sort(&mut self, slot: SortSlot, ix: usize, cx: &mut Context<Self>) {
        if let Some(model) = self.model_mut() {
            model.set_sort(slot, ix);
            cx.notify();
        }
    }

    /// Backend row `ix` of the model's backends (plain, vim).
    pub(crate) fn select_backend(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(model) = self.model_mut() else {
            return;
        };
        if let Some(backend) = model.backends().get(ix).copied() {
            model.editor_backend = backend;
        }
        cx.notify();
    }

    /// The server field changed: the draft takes a valid address; an
    /// invalid one is shown and blocks Save.
    fn server_changed(&mut self, cx: &mut Context<Self>) {
        let typed = self.server_input.read(cx).value().to_string();
        let Some(model) = self.model_mut() else {
            return;
        };
        self.server_error = model
            .try_set_server_url(&typed)
            .err()
            .map(|e| e.to_string().into());
        cx.notify();
    }

    // ---- leaving ----------------------------------------------------------

    /// `Cancel`, Esc (and the window's close, `close_window`): close at once
    /// when nothing changed, else ask first. Never saves.
    pub(crate) fn cancel(
        &mut self,
        close_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        if self.is_dirty() {
            self.confirm = Some(Discard { close_window });
            window.focus(&self.focus, cx);
        } else {
            cx.emit(PreferencesEvent::Close {
                theme: self.original.clone(),
                close_window,
            });
        }
        cx.notify();
    }

    /// The window's close asked: true when it may close now (nothing to
    /// lose); else the question is asked (the window gives it the keyboard)
    /// and the window stays.
    pub(crate) fn window_close_requested(&mut self, cx: &mut Context<Self>) -> bool {
        // A reindex or a write running: the close is refused with a
        // message, and not queued; the user closes again once it is done.
        if self.busy() {
            self.close_refused(cx);
            return false;
        }
        if !self.is_dirty() {
            return true;
        }
        self.confirm = Some(Discard { close_window: true });
        cx.notify();
        false
    }

    /// `Discard changes`: the theme the window had comes back; the view
    /// closes (and the window, when it was its close that asked).
    pub(crate) fn discard(&mut self, cx: &mut Context<Self>) {
        if let Some(Discard { close_window }) = self.confirm.take() {
            cx.emit(PreferencesEvent::Close {
                theme: self.original.clone(),
                close_window,
            });
        }
        cx.notify();
    }

    /// `Keep editing`: back to the draft.
    pub(crate) fn keep_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm = None;
        window.focus(&self.sections_focus, cx);
        cx.notify();
    }

    /// The theme the draft names, as the window will show it: the one the
    /// window had while the draft keeps the configured name.
    fn chosen_theme(&self, initial: &AppSettings) -> Theme {
        match self.model.as_ref() {
            Some(m) if m.theme_name != initial.theme => m.selected_theme().clone(),
            _ => self.original.clone(),
        }
    }

    /// `Save` (Ctrl+S, Enter in the server field): write the settings with
    /// the draft applied, off the UI thread; see the module docs.
    pub(crate) fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_save() {
            return;
        }
        let Some(model) = self.model.clone() else {
            return;
        };
        let before = self.initial();
        let mut after = before.clone();
        model.apply(&mut after);
        let reopen = needs_reopen(&before, &after);
        let theme = self.chosen_theme(&before);
        self.message = None;
        let opened = self.opened.clone().unwrap_or_else(|| before.clone());
        match kimun_notes::logic::preferences::save_plan(&after, &opened) {
            Err(e) => {
                self.message = Some(e.to_string().into());
                cx.notify();
            }
            // The vault moved under the draft: indexed first, then written
            // and entered again (the vault-path reindex flow).
            Ok(SavePlan::NeedsReindex { workspace, cache }) => {
                self.save_after_reindex(model, theme, workspace, cache, window, cx)
            }
            Ok(SavePlan::SaveOnly) => {
                let indexed = !after.needs_indexing();
                self.write_save(model, indexed, theme, reopen, window, cx)
            }
        }
    }

    /// Write the settings with the draft (`model`) applied, as they are when
    /// the write's turn comes ([`reopen::write_settings`]); written, the
    /// shared settings take the draft in that turn (`indexed`: a pending
    /// reindex is done too), then the view hears of it (`reopen`: the window
    /// enters the vault again).
    fn write_save(
        &mut self,
        model: PreferencesModel,
        indexed: bool,
        theme: Theme,
        reopen: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let write = reopen::write_settings(self.store.clone(), self.settings.clone(), move |s| {
            model.apply(s);
            if indexed {
                s.report_indexed();
            }
        });
        self.saving = Some(cx.spawn_in(window, async move |this, cx| {
            let written = write.await;
            this.update_in(cx, |view, window, cx| {
                view.saved(written, theme, reopen, window, cx)
            })
            .ok();
        }));
        cx.notify();
    }

    fn saved(
        &mut self,
        written: Result<(), String>,
        theme: Theme,
        reopen: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.saving = None;
        match (written, self.model.as_ref()) {
            // The shared settings took the draft in the write.
            (Ok(()), Some(_)) => {
                self.dirty.set(None);
                cx.emit(PreferencesEvent::Saved { theme, reopen });
            }
            (Ok(()), None) => {}
            (Err(e), _) => {
                tracing::warn!("could not save the preferences: {e}");
                self.message = Some(format!("Couldn't save the preferences: {e}").into());
                window.focus(&self.sections_focus, cx);
            }
        }
        cx.notify();
    }

    // ---- keys -------------------------------------------------------------

    /// Enter in the server field (the kit's action, seen first): Save.
    fn on_field_enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.server_focused(window, cx) {
            self.save(window, cx);
            cx.stop_propagation();
        } else if self.ws_field_focused(window, cx) {
            self.ws_field_enter(window, cx);
            cx.stop_propagation();
        }
    }

    /// Esc in the server field: Cancel, as everywhere in the view.
    fn on_field_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.server_focused(window, cx) {
            self.cancel(false, window, cx);
            cx.stop_propagation();
        } else if self.ws_field_focused(window, cx) {
            self.ws_field_escape(window, cx);
            cx.stop_propagation();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = input::key_down_input(event) else {
            return;
        };
        if self.key(key, window, cx) {
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// The view's keys. True when taken. The arrows belong to the control
    /// with the keyboard: the section list, the theme list, a stepper; the
    /// kit's controls (buttons, checkbox, radios, the field) keep their own.
    pub(crate) fn key(
        &mut self,
        key: KeyInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let plain = key.mods == Mods::NONE;
        let primary = (key.mods.ctrl || key.mods.sup) && !key.mods.alt && !key.mods.shift;
        // A write or a reindex holds the view; Tab may still walk it.
        if self.busy() {
            return !matches!(key.key, Key::Tab | Key::BackTab);
        }
        let on_root = self.focus.is_focused(window);
        if self.confirm.is_some() {
            match key.key {
                Key::Esc => self.keep_editing(window, cx),
                Key::Enter if plain && on_root => self.discard(cx),
                _ => return false,
            }
            return true;
        }
        if self.ws.ask.is_some() {
            match key.key {
                Key::Esc => self.ask_keep(window, cx),
                Key::Enter if plain && on_root => self.ask_go(window, cx),
                _ => return false,
            }
            return true;
        }
        if self.index.confirm_full {
            match key.key {
                Key::Esc => self.full_cancel(window, cx),
                Key::Enter if plain && on_root => self.full_confirm(window, cx),
                _ => return false,
            }
            return true;
        }
        if matches!(self.ws.form, Some(workspaces::Form::Delete(_))) {
            match key.key {
                Key::Esc => self.ws_close_form(window, cx),
                Key::Enter if plain && on_root => self.ws_delete_confirm(window, cx),
                _ => return false,
            }
            return true;
        }
        if primary && key.key == Key::Char('s') {
            self.save(window, cx);
            return true;
        }
        if self.server_focused(window, cx) || self.ws_field_focused(window, cx) {
            return false;
        }
        // The chooser up in Workspaces has the arrows, Enter and Esc.
        if let Some(chooser) = self.ws_chooser()
            && on_root
        {
            return chooser.update(cx, |c, cx| c.key(key, window, cx));
        }
        if key.key == Key::Esc {
            if self.ws.form.is_some() {
                self.ws_close_form(window, cx);
            } else {
                self.cancel(false, window, cx);
            }
            return true;
        }
        if !plain {
            return false;
        }
        let on = |handle: &FocusHandle| handle.is_focused(window);
        if on(&self.ws.focus) {
            match key.key {
                Key::Up => self.ws_move(false, cx),
                Key::Down => self.ws_move(true, cx),
                Key::Enter => self.ws_switch(window, cx),
                Key::F(2) | Key::Char('r') => self.ws_rename(window, cx),
                Key::Delete | Key::Char('d') => self.ws_delete(window, cx),
                Key::Char('n') => self.ws_new(window, cx),
                Key::Char('b') => self.ws_change_path(window, cx),
                _ => return false,
            }
            return true;
        }
        if on(&self.sections_focus) || on(&self.focus) {
            match key.key {
                Key::Up => self.move_section(false, window, cx),
                Key::Down => self.move_section(true, window, cx),
                _ => return false,
            }
        } else if on(&self.themes_focus) {
            match key.key {
                Key::Up => self.move_theme(false, cx),
                Key::Down => self.move_theme(true, cx),
                _ => return false,
            }
        } else if on(&self.delay_focus) || on(&self.autosave_focus) {
            let up = match key.key {
                Key::Right | Key::Up => true,
                Key::Left | Key::Down => false,
                _ => return false,
            };
            if on(&self.delay_focus) {
                self.step_delay(up, cx);
            } else {
                self.step_autosave(up, cx);
            }
        } else {
            return false;
        }
        true
    }
}

// ---- Rendering ---------------------------------------------------------------

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

/// A control's label.
fn label(text: impl Into<SharedString>, t: &Theme) -> impl IntoElement {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::foreground(t))
        .child(text.into())
}

/// One labelled control with its notes: label, control, notes, tight.
fn field(
    title: &'static str,
    control: impl IntoElement,
    notes: Vec<SharedString>,
    t: &Theme,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(label(title, t))
        .child(control)
        .children(notes.into_iter().map(|n| note(n, t)))
}

/// What a stepper shows and how it reads.
struct StepperSpec {
    id: &'static str,
    down_id: &'static str,
    up_id: &'static str,
    name: &'static str,
    value: u64,
    min: u64,
    max: u64,
    unit: &'static str,
}

impl PreferencesView {
    fn render_section_row(&self, section: Section, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let selected = self.section == section;
        let hover = theme::hover_background(t);
        div()
            .id(section_id(section))
            .test_support()
            .role(Role::Tab)
            .aria_label(section.title())
            .aria_selected(selected)
            .flex()
            .items_center()
            .h(rems(1.75))
            .px_2()
            .rounded_md()
            .text_sm()
            .text_color(theme::foreground(t))
            .when(selected, |row| {
                row.bg(theme::selected_background(t))
                    .font_weight(FontWeight::SEMIBOLD)
            })
            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
            .child(section.title())
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.show_section(section, window, cx)
            }))
            .into_any_element()
    }

    /// The section list: a column beside the controls, or (`narrow`) a row
    /// of tabs above them. It holds the keyboard for Up / Down.
    fn render_sections(&self, narrow: bool, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let focused = self.sections_focus.is_focused(window);
        let rows: Vec<AnyElement> = Section::ALL
            .into_iter()
            .map(|s| self.render_section_row(s, cx))
            .collect();
        let list = div()
            .id(SECTIONS_ID)
            .test_support()
            .role(Role::TabList)
            .aria_label("Sections")
            .track_focus(&self.sections_focus)
            .flex()
            .gap_1()
            .p_1()
            .rounded_md()
            .border_1()
            .border_color(if focused {
                theme::focus_border(t)
            } else {
                gpui_kit::transparent_black().into()
            })
            .children(rows);
        if narrow {
            div()
                .flex_none()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(theme::divider(t))
                .bg(theme::panel_background(t))
                .child(list.flex_row().flex_wrap())
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .flex_none()
                .w(rems(13.))
                .gap_3()
                .p_3()
                .border_r_1()
                .border_color(theme::divider(t))
                .bg(theme::panel_background(t))
                .child(
                    div()
                        .px_2()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::bright(t))
                        .child("Preferences"),
                )
                .child(list.flex_col())
                .into_any_element()
        }
    }

    fn render_stepper(
        &self,
        spec: StepperSpec,
        focus: &FocusHandle,
        on_step: fn(&mut Self, bool, &mut Context<Self>),
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let focused = focus.is_focused(window);
        let text = format!("{} {}", spec.value, spec.unit);
        h_row()
            .gap_2()
            .child(
                Button::new(spec.down_id)
                    .icon(IconName::Minus)
                    .small()
                    .tab_stop(false)
                    .accessibility_label(format!("Decrease {}", spec.name))
                    .disabled(spec.value <= spec.min || self.saving.is_some())
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| on_step(this, false, cx)),
                    ),
            )
            .child(
                div()
                    .id(spec.id)
                    .test_support()
                    .role(Role::SpinButton)
                    .aria_label(spec.name)
                    .aria_value(SharedString::from(text.clone()))
                    .aria_numeric_value(spec.value as f64)
                    .aria_min_numeric_value(spec.min as f64)
                    .aria_max_numeric_value(spec.max as f64)
                    .track_focus(focus)
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_w(rems(6.))
                    .h(rems(1.75))
                    .px_2()
                    .rounded_md()
                    .border_1()
                    .border_color(if focused {
                        theme::focus_border(t)
                    } else {
                        theme::divider(t)
                    })
                    .bg(theme::background(t))
                    .text_sm()
                    .text_color(theme::foreground(t))
                    // The value changes in place: tabular figures hold it still.
                    .font_features(crate::ui_font::tabular_figures())
                    .child(text),
            )
            .child(
                Button::new(spec.up_id)
                    .icon(IconName::Plus)
                    .small()
                    .tab_stop(false)
                    .accessibility_label(format!("Increase {}", spec.name))
                    .disabled(spec.value >= spec.max || self.saving.is_some())
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| on_step(this, true, cx)),
                    ),
            )
            .into_any_element()
    }

    fn render_appearance(
        &self,
        model: &PreferencesModel,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let count = model.themes.len();
        let focused = self.themes_focus.is_focused(window);
        let list = uniform_list(
            THEME_LIST_ID,
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let Some(model) = this.model.as_ref() else {
                    return Vec::new();
                };
                let t = theme::current(cx);
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
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                window.focus(&this.themes_focus, cx);
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
            .gap_1()
            .child(label("Theme", t))
            .child(
                div()
                    .id(THEMES_ID)
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label("Themes")
                    .track_focus(&self.themes_focus)
                    .h(rems(16.))
                    .p_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if focused {
                        theme::focus_border(t)
                    } else {
                        theme::divider(t)
                    })
                    .child(list),
            )
            .child(note(
                "The window shows each theme as you select it. Cancel puts the previous one \
                 back; custom themes go in the themes folder of kimün's config.",
                t,
            ))
            .into_any_element()
    }

    fn render_display(
        &self,
        model: &PreferencesModel,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let delay = self.render_stepper(
            StepperSpec {
                id: DELAY_ID,
                down_id: DELAY_DOWN_ID,
                up_id: DELAY_UP_ID,
                name: "Which-key delay",
                value: model.leader_timeout_ms,
                min: LEADER_DELAY_MIN_MS,
                max: LEADER_DELAY_MAX_MS,
                unit: "ms",
            },
            &self.delay_focus,
            Self::step_delay,
            window,
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .child(
                                Checkbox::new(UPDATE_CHECK_ID)
                                    .label("Check for updates on startup")
                                    .checked(model.update_check)
                                    .disabled(self.saving.is_some())
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.set_update_check(*checked, cx)
                                    })),
                            )
                            .child(
                                // Acts now, saved or not: it is not a setting.
                                Button::new(CHECK_UPDATES_ID)
                                    .small()
                                    .outline()
                                    .label("Check now")
                                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                        cx.emit(PreferencesEvent::CheckForUpdates)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id(UPDATE_CHECK_HELP_ID)
                            .test_support()
                            .role(Role::Label)
                            .aria_label(UPDATE_CHECK_HELP)
                            .child(note(UPDATE_CHECK_HELP, t)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        Checkbox::new(REDUCE_MOTION_ID)
                            .label("Reduce motion")
                            .checked(model.reduce_motion)
                            .disabled(self.saving.is_some())
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_reduce_motion(*checked, cx)
                            })),
                    )
                    .child(
                        div()
                            .id(REDUCE_MOTION_HELP_ID)
                            .test_support()
                            .role(Role::Label)
                            .aria_label(REDUCE_MOTION_HELP)
                            .child(note(REDUCE_MOTION_HELP, t)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(note("Rail position", t))
                    .child(
                        RadioGroup::horizontal("preferences-rail")
                            .selected_index(
                                RailPosition::ALL
                                    .iter()
                                    .position(|rail| *rail == model.rail),
                            )
                            .disabled(self.saving.is_some())
                            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                                if let Some(rail) = RailPosition::ALL.get(*ix) {
                                    this.set_rail_position(*rail, cx)
                                }
                            }))
                            .children(
                                RailPosition::ALL
                                    .into_iter()
                                    .map(|rail| Radio::new(rail.label()).label(rail.label())),
                            ),
                    )
                    .child(
                        div()
                            .id(RAIL_HELP_ID)
                            .test_support()
                            .role(Role::Label)
                            .aria_label(RAIL_HELP)
                            .child(note(RAIL_HELP, t)),
                    ),
            )
            .child(field(
                "Which-key delay",
                delay,
                vec![
                    "How long the leader waits before it lists the keys that can follow. \
                     Takes effect when you save."
                        .into(),
                ],
                t,
            ))
            .into_any_element()
    }

    fn sort_group(
        &self,
        title: &'static str,
        id: &'static str,
        field_slot: SortSlot,
        order_slot: SortSlot,
        model: &PreferencesModel,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let field_ix = model.sort_selected(field_slot);
        let order_ix = model.sort_selected(order_slot);
        let busy = self.saving.is_some();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::bright(t))
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(note("Sort by", t))
                    .child(
                        RadioGroup::horizontal(SharedString::from(format!("{id}-field")))
                            .selected_index(field_ix)
                            .disabled(busy)
                            .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                                this.set_sort(field_slot, *ix, cx)
                            }))
                            .children(
                                model
                                    .sort_choices(field_slot)
                                    .into_iter()
                                    .map(|name| Radio::new(name).label(name)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(note("Order", t))
                    .child(
                        RadioGroup::horizontal(SharedString::from(format!("{id}-order")))
                            .selected_index(order_ix)
                            .disabled(busy)
                            .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                                this.set_sort(order_slot, *ix, cx)
                            }))
                            .children(
                                model
                                    .sort_choices(order_slot)
                                    .into_iter()
                                    .map(|name| Radio::new(name).label(name)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_sorting(&self, model: &PreferencesModel, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.sort_group(
                "Notes",
                "preferences-sort-default",
                SortSlot::DefaultField,
                SortSlot::DefaultOrder,
                model,
                cx,
            ))
            .child(self.sort_group(
                "Journal",
                "preferences-sort-journal",
                SortSlot::JournalField,
                SortSlot::JournalOrder,
                model,
                cx,
            ))
            .child(note(
                "Takes effect when you save: FILES re-sorts the folder it shows (unless you \
                 sorted it by hand) and sorts every folder you open.",
                t,
            ))
            .into_any_element()
    }

    fn render_editor(
        &self,
        model: &PreferencesModel,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let autosave = self.render_stepper(
            StepperSpec {
                id: AUTOSAVE_ID,
                down_id: AUTOSAVE_DOWN_ID,
                up_id: AUTOSAVE_UP_ID,
                name: "Autosave interval",
                value: model.autosave_interval_secs,
                min: AUTOSAVE_MIN_SECS,
                max: AUTOSAVE_MAX_SECS,
                unit: "s",
            },
            &self.autosave_focus,
            Self::step_autosave,
            window,
            cx,
        );
        let selected = model
            .backends()
            .iter()
            .position(|b| *b == model.editor_backend);
        let radios = model
            .backends()
            .iter()
            .map(|b| Radio::new(backend_label(*b)).label(backend_label(*b)));
        // A configured nvim (not offered): no row is selected, and this
        // says what stays.
        let kept = selected.is_none().then(|| {
            div()
                .id(BACKEND_KEPT_ID)
                .test_support()
                .role(Role::Label)
                .aria_label(NVIM_UNCHANGED)
                .text_sm()
                .text_color(theme::foreground(t))
                .child(NVIM_UNCHANGED)
        });
        let backend = div().flex().flex_col().gap_2().children(kept).child(
            RadioGroup::vertical(BACKENDS_ID)
                .selected_index(selected)
                .disabled(self.saving.is_some())
                .on_click(cx.listener(|this, ix: &usize, _, cx| this.select_backend(*ix, cx)))
                .children(radios),
        );
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(field(
                "Autosave interval",
                autosave,
                vec![
                    "How often the open note is written while it has unsaved changes. Takes \
                     effect when you save."
                        .into(),
                ],
                t,
            ))
            .child(field(
                "Editor backend",
                backend,
                vec![
                    "Changing it reopens the workspace when you save; the open note is saved \
                     first."
                        .into(),
                ],
                t,
            ))
            .into_any_element()
    }

    fn render_server(&self, cx: &gpui_kit::App) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let input = Input::new(&self.server_input)
            .id(SERVER_ID)
            .aria_label("kimün server address")
            .disabled(self.saving.is_some())
            .bg(theme::background(t))
            .border_color(if self.server_error.is_some() {
                theme::error(t)
            } else {
                theme::divider(t)
            })
            .text_color(theme::foreground(t));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(label("kimün server address", t))
            .child(input)
            .when_some(self.server_error.clone(), |col, error| {
                col.child(
                    div()
                        .id(SERVER_ERROR_ID)
                        .test_support()
                        .role(Role::Label)
                        .aria_label(error.clone())
                        .text_xs()
                        .text_color(theme::error(t))
                        .child(error),
                )
            })
            .child(note(
                format!(
                    "URL with port, e.g. {DEFAULT_SERVER_URL_HINT}. Leave it empty to turn \
                     the server features off."
                ),
                t,
            ))
            .child(note(
                "Changing it reopens the workspace when you save, so the server sync \
                 restarts; the open note is saved first.",
                t,
            ))
            .into_any_element()
    }

    /// The selected section's title and controls (or the question, or the
    /// wait for the draft).
    fn render_content(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let conflict = self
            .index
            .conflict
            .clone()
            .filter(|_| self.confirm.is_none())
            .map(|text| self.render_conflict(text, cx));
        let running = self.index.running.as_ref().map(|r| r.label);
        let ask = self.ask_text();
        let (title, body) = match (self.confirm, self.model.as_ref()) {
            (Some(_), _) => (
                "Discard your changes?",
                para("Your changes to the preferences have not been saved.", t).into_any_element(),
            ),
            (None, _) if running.is_some() => (
                "Indexing",
                self.render_progress(running.unwrap_or_default(), cx),
            ),
            (None, _) if ask.is_some() => {
                let (title, text) = ask.unwrap_or_default();
                (title, para(text, t).into_any_element())
            }
            (None, None) => (
                "Preferences",
                note("Reading the preferences…", t).into_any_element(),
            ),
            (None, Some(model)) => (
                self.section.title(),
                match self.section {
                    Section::Workspaces => self.render_workspaces(window, cx),
                    Section::Indexing => self.render_indexing(cx),
                    Section::Appearance => self.render_appearance(model, window, cx),
                    Section::Display => self.render_display(model, window, cx),
                    Section::Sorting => self.render_sorting(model, cx),
                    Section::Editor => self.render_editor(model, window, cx),
                    Section::Server => self.render_server(cx),
                },
            ),
        };
        div()
            .id("preferences-content")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .w_full()
                    .max_w(rems(40.))
                    .px_6()
                    .py_5()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::bright(t))
                            .child(title),
                    )
                    .children(conflict)
                    .child(body),
            )
            .into_any_element()
    }

    /// The key hints, or the problem line, and the commands.
    fn render_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_2()
            .flex_none()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(theme::divider(t))
            .bg(theme::panel_background(t));
        let lead = match (&self.message, self.confirm) {
            (Some(message), None) => div()
                .id(MESSAGE_ID)
                .test_support()
                .role(Role::Label)
                .aria_label(message.clone())
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(theme::error(t))
                .child(message.clone())
                .into_any_element(),
            (_, Some(_)) => div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(theme::muted(t))
                .child("Enter: discard · Esc: keep editing")
                .into_any_element(),
            (None, None) if self.ws.ask.is_some() => div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(theme::muted(t))
                .child("Enter: discard and go on · Esc: keep editing")
                .into_any_element(),
            (None, None) => div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(theme::muted(t))
                .child("↑/↓: sections and lists · ←/→: steps · Tab: next control · Ctrl+S: save · Esc: cancel")
                .into_any_element(),
        };
        if let Some(buttons) = self.ask_buttons(cx) {
            return row.child(lead).children(buttons).into_any_element();
        }
        if self.confirm.is_some() {
            return row
                .child(lead)
                .child(Button::new(KEEP_EDITING_ID).label("Keep editing").on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.keep_editing(window, cx)),
                ))
                .child(
                    Button::new(DISCARD_ID)
                        .danger()
                        .label("Discard changes")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.discard(cx))),
                )
                .into_any_element();
        }
        let saving = self.saving.is_some();
        let busy = self.busy();
        row.child(lead)
            .child(
                Button::new(CANCEL_ID)
                    .label("Cancel")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.cancel(false, window, cx)
                    })),
            )
            .child(
                Button::new(SAVE_ID)
                    .primary()
                    .label("Save")
                    .loading(saving)
                    .disabled(!self.can_save())
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| this.save(window, cx)),
                    ),
            )
            .into_any_element()
    }
}

/// A row of controls, centred on its cross axis.
fn h_row() -> gpui_kit::Div {
    div().flex().flex_row().items_center()
}

impl Render for PreferencesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        let narrow = window.viewport_size().width < window.rem_size() * NARROW_REMS;
        let sections = self.render_sections(narrow, window, cx);
        let content = self.render_content(window, cx);
        let body = div().flex().flex_1().min_h_0().min_w_0();
        let body = if narrow {
            body.flex_col().child(sections).child(content)
        } else {
            body.flex_row()
                .items_stretch()
                .child(sections)
                .child(content)
        };
        div()
            .id(PREFERENCES_ID)
            .test_support()
            .role(Role::Group)
            .aria_label("Preferences")
            .key_context("KimunPreferences")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .capture_action(cx.listener(Self::on_field_enter))
            .capture_action(cx.listener(Self::on_field_escape))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::background(&t))
            .text_color(theme::foreground(&t))
            .child(body)
            .child(self.render_footer(cx))
    }
}

// ---- The window's side ---------------------------------------------------------

impl Workspace {
    /// Whether Preferences have the window.
    pub(crate) fn preferences_open(&self) -> bool {
        self.preferences.is_some()
    }

    /// `OpenPreferences` (its chord, leader `v p`, the palette): Preferences
    /// over the window, in every window state (with no vault too), or the
    /// keyboard given back to them when they are open. Refused under the
    /// setup assistant, another modal, a vault change or the window's close.
    /// A pending leader sequence ends. Needs the tokio runtime entered.
    pub(crate) fn open_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preferences.is_none() {
            if self.onboarding_open()
                || super::modal::dialog_open(window, cx)
                || self.reopen_running()
                || self.closing != Closing::Open
            {
                return;
            }
            let Some(settings) = self.settings.clone() else {
                return;
            };
            self.cancel_leader(cx);
            let original = (*theme::current(cx)).clone();
            let store = self.store.clone();
            let view = cx.new(|cx| PreferencesView::new(settings, original, store, window, cx));
            self._preferences_events =
                Some(cx.subscribe_in(&view, window, Self::on_preferences_event));
            self.preferences = Some(view);
        }
        self.apply_focus(window, cx);
        cx.notify();
    }

    /// Take Preferences down; the keyboard goes back to the layout.
    fn close_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drop_preferences();
        self.apply_focus(window, cx);
        cx.notify();
    }

    /// Preferences go without a word (the vault the draft was made over is
    /// being left).
    pub(super) fn drop_preferences(&mut self) {
        self.preferences = None;
        self._preferences_events = None;
    }

    /// The window's close asked while Preferences show: true when it may go
    /// on (no Preferences, or nothing changed in them); otherwise they ask
    /// "Discard your changes?" and the window stays.
    pub(crate) fn preferences_allow_close(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.preferences.clone() else {
            return true;
        };
        let allowed = view.update(cx, |view, cx| view.window_close_requested(cx));
        if !allowed {
            // The question has the keyboard (Enter discards, Esc goes back).
            self.with_window(cx, |workspace, window, cx| {
                workspace.apply_focus(window, cx)
            });
        }
        allowed
    }

    fn on_preferences_event(
        &mut self,
        _: &Entity<PreferencesView>,
        event: &PreferencesEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PreferencesEvent::Preview(t) => self.apply_theme(t.clone(), cx),
            PreferencesEvent::Close {
                theme: t,
                close_window,
            } => {
                self.apply_theme(t.clone(), cx);
                self.close_preferences(window, cx);
                if *close_window {
                    self.request_close(window, cx);
                }
            }
            PreferencesEvent::Saved { theme: t, reopen } => {
                self.apply_theme(t.clone(), cx);
                self.close_preferences(window, cx);
                self.apply_saved_preferences(*reopen, cx);
            }
            PreferencesEvent::Switch { theme: t, name } => {
                self.apply_theme(t.clone(), cx);
                self.close_preferences(window, cx);
                self.switch_workspace(name.clone(), cx);
            }
            PreferencesEvent::Reopen { theme: t } => {
                self.apply_theme(t.clone(), cx);
                self.close_preferences(window, cx);
                match self.tx.clone() {
                    Some(tx) if tx.send(AppMessage::PreferencesSaved).is_ok() => {}
                    _ => self.reopen(cx),
                }
            }
            PreferencesEvent::Reindexed => self.index_rebuilt(cx),
            PreferencesEvent::Renamed(name) => self.workspace_renamed(name, window, cx),
            PreferencesEvent::RunSetup { theme: t } => {
                self.apply_theme(t.clone(), cx);
                self.close_preferences(window, cx);
                self.open_onboarding(window, cx);
            }
            PreferencesEvent::CheckForUpdates => self.check_updates_now(cx),
        }
    }

    /// The open workspace's index was rebuilt on a throwaway vault (the same
    /// index file): the views over it load again, as after the startup's
    /// indexing (`IndexingDone`), and FILES lists its folder again.
    fn index_rebuilt(&mut self, cx: &mut Context<Self>) {
        self.refresh_note_counts();
        self.invalidate_views(cx);
        if let Some(files) = self.files.clone() {
            files.update(cx, |pane, cx| {
                let dir = pane.current_dir().clone();
                pane.refresh_if_showing(&dir, cx);
            });
        }
        self.refresh_views(cx);
        cx.notify();
    }

    /// The current workspace was renamed (its files keep their names): the
    /// window title and FILES' root follow.
    fn workspace_renamed(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.vault().is_none() {
            return;
        }
        self.set_workspace_name(name, window);
        if let Some(files) = self.files.clone() {
            let name = SharedString::from(name.to_string());
            files.update(cx, |pane, cx| pane.set_root_name(name, cx));
        }
        cx.notify();
    }

    /// Opening the vault found files whose names differ only by case
    /// (`VaultConflict`): Preferences open on Workspaces with the vault's
    /// message in a panel, so another workspace or folder can be picked.
    /// The workspace config stays as it is. Not over the setup assistant;
    /// a modal open closes first. Needs the tokio runtime entered.
    pub(crate) fn show_vault_conflict(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.onboarding_open() {
            return;
        }
        if self.preferences.is_none() && super::modal::dialog_open(window, cx) {
            self.close_modal(window, cx);
        }
        self.open_preferences(window, cx);
        if let Some(view) = self.preferences.clone() {
            view.update(cx, |view, cx| view.show_conflict(text, window, cx));
            self.apply_focus(window, cx);
        }
    }

    /// The preferences were saved (the shared settings hold them): apply
    /// them without a restart. The theme is applied by the caller.
    ///
    /// - which-key delay: read by every leader sequence as it starts;
    /// - the leader tree: rebuilt from the settings;
    /// - autosave interval: a running tick restarts on the new period;
    /// - sort defaults: FILES takes them (its folder re-sorts unless sorted
    ///   by hand);
    /// - update check: read by the next launch's startup check ([`super::update`]);
    /// - reduce motion: GPUI's switch is set at once
    ///   ([`crate::apply_reduce_motion`]), every animation honours it on the
    ///   next frame;
    /// - server address, editor engine: `reopen` ([`needs_reopen`]) sends
    ///   `PreferencesSaved`, so the vault is entered again (M8b). Nothing else
    ///   reopens.
    pub(crate) fn apply_saved_preferences(&mut self, reopen: bool, cx: &mut Context<Self>) {
        let Some(settings) = self.settings.clone() else {
            return;
        };
        let snapshot = settings.read().unwrap_or_else(|e| e.into_inner()).clone();
        self.leader.set_tree(snapshot.leader_tree());
        crate::apply_reduce_motion(snapshot.gui.reduce_motion(), cx);
        // The saved position moves the rail in this window now; the next
        // layout save writes the same value.
        self.rail = snapshot.gui.rail();
        if let Some(files) = self.files.clone() {
            files.update(cx, |pane, cx| pane.set_sort_defaults(&snapshot, cx));
        }
        if let (Some(host), Some(tx)) = (self.notes.as_mut(), self.tx.as_ref()) {
            host.session.retime_autosave(tx);
        }
        if reopen {
            match self.tx.clone() {
                Some(tx) if tx.send(AppMessage::PreferencesSaved).is_ok() => {}
                _ => self.reopen(cx),
            }
        }
        cx.notify();
    }
}
