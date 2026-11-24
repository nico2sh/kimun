//! The **folder chooser** (M8c, shared since M9b-2): a panel inside a
//! full-window view (the setup assistant, Preferences > Workspaces), not a
//! dialog over it. Up, the Location field, the subfolders of the folder
//! shown (a click enters one, a double-click or `Choose this folder` takes
//! the folder shown) and an inline `New folder…` field.
//!
//! **The Location field** shows the folder as its placeholder. A name typed
//! there ranks the rows, best first, the best selected
//! (`logic::file_ops::rank_names`); an empty field shows the listing in
//! order; a path (`logic::onboarding::classify_location`) filters nothing
//! and selects nothing. Enter there ([`FolderChooser::go_to_location`])
//! looks the path up off the UI thread (`resolve_location`): a folder opens
//! in the list, a file is picked in files mode, and anything else shows its
//! message under the field until the next key. Browsing to another folder
//! empties the field. The list's accessible name names the folder shown.
//!
//! **The keys.** The field has the keyboard from the moment the chooser
//! opens, and drives it: Up and Down move the selection, Enter opens the
//! row or goes to the typed path, Ctrl+Enter (Cmd+Enter) chooses the
//! folder, Alt+Up (Cmd+Up on macOS, read from the keymap's platform) and
//! Backspace in the empty field go up, Ctrl+N (Cmd+N) opens `New folder…`,
//! Esc clears the text and then cancels. A click on a row leaves the
//! keyboard in the field. Left, Right, a plain N and Backspace
//! with text are the field's. The chooser catches the kit's field actions
//! (`Enter`, `Escape`, `MoveUp`, `MoveDown`, `Backspace`) on its own root
//! in the capture phase, after the host's capture handlers and before the
//! field's own, and stops them there.
//!
//! **Height.** The list takes [`LIST_FRACTION`] of the window's height,
//! between [`LIST_MIN_REMS`] and [`LIST_MAX_REMS`] ([`list_height`]), in
//! every host. A host of fixed height shrinks it to fit, never under the
//! floor; a host too short for that scrolls.
//!
//! **Behaviour owner.** The chooser entity: the folder shown, its listing,
//! the rows shown and the selection, the fields and the work under way.
//! Listings and the new folder run on tokio's blocking pool
//! (`logic::onboarding::list_subdirs`, `create_subdir`, through core); a
//! stale listing is dropped by revision.
//!
//! **Its host** draws it ([`FolderChooser`] renders the panel, [`actions`]
//! the command row the host places in its footer and
//! [`FolderChooser::hints`] the key hints). The host's field actions and
//! keys leave a chooser field alone ([`FolderChooser::field_focused`]):
//! the chooser handles Enter and Esc in both of its fields. When the
//! host's own root has the keyboard (after a click outside the fields), it
//! hands the keys over ([`FolderChooser::key`]). The chooser says what was
//! picked with [`FolderChooserEvent`]. Element ids carry the host's prefix
//! (`onboarding-chooser`, `preferences-chooser`).
//!
//! **Files mode** ([`FolderChooser::files`], off by default): the list also
//! shows the files with one extension (`logic::onboarding::list_files`:
//! hidden folders listed, hidden files left out). A folder works as before; a click or Enter on
//! a file picks it ([`FolderChooserEvent::ChosenFile`]). There is no
//! `Choose this folder` then, and no `New folder…` (nor its Ctrl+N key).
//!
//! Each row shows a folder or file icon before its name, and its label
//! names the kind, so the kind is not told by colour alone.

use crate::tasks::blocking;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Icon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    Backspace, Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveToStart, MoveUp,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, Focusable as _,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels, Render, Role,
    ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Subscription, Task,
    UniformListScrollHandle, Window, div, rems, uniform_list,
};
use kimun_core::system::{self, SystemPath};
use kimun_notes::logic::file_ops;
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::onboarding::{self as onb, LocationInput};
use kimun_notes::settings::themes::Theme;

use crate::keymap::{self, Platform};
use crate::theme;

/// The last component of `dir`, as shown in the chooser.
pub(crate) fn folder_name(dir: &SystemPath) -> String {
    dir.as_path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| system::path_to_string(dir))
}

/// The element id of a chooser's row for `dir` (its folder name), under the
/// host's `prefix`.
pub(crate) fn row_id(prefix: &str, dir: &SystemPath) -> SharedString {
    format!("{prefix}-dir-{}", folder_name(dir)).into()
}

/// The element id of a chooser's row for the file `path` (its name), in
/// files mode.
pub(crate) fn file_row_id(prefix: &str, path: &SystemPath) -> SharedString {
    format!("{prefix}-file-{}", folder_name(path)).into()
}

/// The Location field's accessible name.
const LOCATION_LABEL: &str = "Location";

/// The list's least height, in rems: a short window keeps it.
pub(crate) const LIST_MIN_REMS: f32 = 14.0;

/// The list's greatest height, in rems: a tall window stops there.
pub(crate) const LIST_MAX_REMS: f32 = 36.0;

/// The share of the window's height the list takes between the two.
pub(crate) const LIST_FRACTION: f32 = 0.45;

/// The list's height in a window `viewport_height` tall whose rem is `rem`:
/// [`LIST_FRACTION`] of the window, kept between [`LIST_MIN_REMS`] and
/// [`LIST_MAX_REMS`]. One rule for every host.
pub(crate) fn list_height(viewport_height: Pixels, rem: Pixels) -> Pixels {
    (viewport_height * LIST_FRACTION).clamp(rem * LIST_MIN_REMS, rem * LIST_MAX_REMS)
}

/// The chooser's element ids under the host's `prefix`: `{prefix}-chooser`,
/// `{prefix}-chooser-<part>`.
fn id(prefix: &str, part: &str) -> SharedString {
    if part.is_empty() {
        format!("{prefix}-chooser").into()
    } else {
        format!("{prefix}-chooser-{part}").into()
    }
}

/// What the chooser tells its host.
pub(crate) enum FolderChooserEvent {
    /// `Choose this folder` (Ctrl+Enter or Cmd+Enter, a double-click):
    /// this folder.
    Chosen(SystemPath),
    /// A click or Enter on a file (files mode only): this file.
    ChosenFile(SystemPath),
    /// `Cancel` (Esc in the empty field): nothing chosen.
    Cancelled,
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

/// The folder chooser. See the module docs.
pub(crate) struct FolderChooser {
    /// The host's id prefix.
    prefix: &'static str,
    dir: SystemPath,
    listing: Listing,
    /// The rows shown, as indexes into the listing: all of them in order,
    /// or the ones the Location field's filter keeps, best first.
    shown: Vec<usize>,
    /// The selected row, as an index into [`Self::shown`].
    selected: Option<usize>,
    /// Bumped by every listing asked for: an older one landing is dropped.
    revision: u64,
    task: Option<Task<()>>,
    new_folder: Option<NewFolder>,
    scroll: UniformListScrollHandle,
    /// Files mode: the extension of the files listed beside the folders.
    files: Option<&'static str>,
    /// Which rows of the listing are files, by index into the listing.
    is_file: Vec<bool>,
    /// The Location field: a name filters the list, a path is gone to
    /// (Enter). Its placeholder is the folder shown.
    location: Entity<InputState>,
    _location_events: Subscription,
    /// Why the path typed there cannot be gone to, under the field until
    /// the next key or another folder.
    location_error: Option<SharedString>,
    /// The typed path being looked up (Enter).
    resolving: Option<Task<()>>,
    /// Bumped by every lookup, every browse and every key in the field: an
    /// older lookup landing is dropped.
    location_revision: u64,
}

impl EventEmitter<FolderChooserEvent> for FolderChooser {}

impl FolderChooser {
    /// A chooser showing `start` (listed in the background), its ids under
    /// `prefix`. Its Location field takes the keyboard. Needs the tokio
    /// runtime entered.
    pub(crate) fn new(
        prefix: &'static str,
        start: SystemPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let location = cx.new(|cx| InputState::new(window, cx));
        let location_events = cx.subscribe(&location, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                // The next key: the message goes, and a lookup of the old
                // text is no longer wanted.
                this.location_error = None;
                this.location_revision += 1;
                this.resolving = None;
                this.refilter(false, cx);
            }
        });
        let mut chooser = Self {
            prefix,
            dir: start.clone(),
            listing: Listing::Loading,
            shown: Vec::new(),
            selected: None,
            revision: 0,
            task: None,
            new_folder: None,
            scroll: UniformListScrollHandle::new(),
            files: None,
            is_file: Vec::new(),
            location,
            _location_events: location_events,
            location_error: None,
            resolving: None,
            location_revision: 0,
        };
        chooser.browse(start, window, cx);
        chooser.focus_location(window, cx);
        chooser
    }

    /// Give the Location field the keyboard.
    fn focus_location(&self, window: &mut Window, cx: &mut App) {
        let handle = self.location.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// Files mode: the files with `extension` (`toml`) are listed too, and
    /// one is picked with a click or Enter. Set it as the chooser is made:
    /// the first listing reads it when it runs.
    pub(crate) fn files(mut self, extension: &'static str) -> Self {
        self.files = Some(extension);
        self
    }

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

    /// How far the list is scrolled down (zero at its top).
    #[cfg(test)]
    pub(crate) fn list_scroll(&self) -> gpui_kit::Pixels {
        -self.scroll.0.borrow().base_handle.offset().y
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

    /// The new folder field.
    #[cfg(test)]
    pub(crate) fn new_folder_input(&self) -> Option<Entity<InputState>> {
        Some(self.new_folder.as_ref()?.input.clone())
    }

    /// Whether a new folder is being made: the commands wait for it.
    pub(crate) fn creating(&self) -> bool {
        self.new_folder.as_ref().is_some_and(|f| f.task.is_some())
    }

    /// Whether the new folder field has the keyboard (its keys are its own;
    /// the chooser takes its Enter and Esc).
    pub(crate) fn new_folder_focused(&self, window: &Window, cx: &App) -> bool {
        self.new_folder
            .as_ref()
            .is_some_and(|f| f.input.read(cx).focus_handle(cx).is_focused(window))
    }

    /// The Location field.
    #[cfg(test)]
    pub(crate) fn location_input(&self) -> Entity<InputState> {
        self.location.clone()
    }

    /// The rows shown, in their order, each with whether it is a file.
    #[cfg(test)]
    pub(crate) fn shown_rows(&self) -> Vec<(SystemPath, bool)> {
        (0..self.shown.len())
            .filter_map(|ix| Some((self.row(ix)?.clone(), self.is_file(ix))))
            .collect()
    }

    /// Whether a typed path is being looked up.
    #[cfg(test)]
    pub(crate) fn resolving(&self) -> bool {
        self.resolving.is_some()
    }

    /// The message under the Location field, if it shows one.
    #[cfg(test)]
    pub(crate) fn location_error(&self) -> Option<&str> {
        self.location_error.as_deref()
    }

    /// Whether the Location field has the keyboard.
    pub(crate) fn location_focused(&self, window: &Window, cx: &App) -> bool {
        self.location.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Whether one of the chooser's fields (the Location field, the new
    /// folder field) has the keyboard: its keys are the field's, so the
    /// host does not take them.
    pub(crate) fn field_focused(&self, window: &Window, cx: &App) -> bool {
        self.location_focused(window, cx) || self.new_folder_focused(window, cx)
    }

    /// The whole listing, in its alphabetical order.
    fn loaded(&self) -> &[SystemPath] {
        match &self.listing {
            Listing::Loaded(rows) => rows,
            _ => &[],
        }
    }

    /// The row shown at `ix` (an index into the rows shown).
    fn row(&self, ix: usize) -> Option<&SystemPath> {
        self.loaded().get(*self.shown.get(ix)?)
    }

    /// Whether the row shown at `ix` is a file (files mode).
    fn is_file(&self, ix: usize) -> bool {
        self.shown
            .get(ix)
            .and_then(|&row| self.is_file.get(row))
            .copied()
            .unwrap_or(false)
    }

    /// The Location field changed, or a listing landed (`landed`): show
    /// the rows a name keeps, best first, or every row in order for an
    /// empty field, and select the first. A path filters nothing: the list
    /// stays as it was until Enter, a fresh listing shows whole. Nothing is
    /// selected then, since Enter goes to the path, not to a row.
    fn refilter(&mut self, landed: bool, cx: &mut Context<Self>) {
        let text = self.location.read(cx).value().to_string();
        let all = || (0..self.loaded().len()).collect::<Vec<_>>();
        let input = onb::classify_location(&text);
        let path = matches!(input, LocationInput::Path(_));
        self.shown = match input {
            LocationInput::Filter(query) => {
                let names: Vec<String> = self.loaded().iter().map(folder_name).collect();
                file_ops::rank_names(&names, &query)
            }
            LocationInput::Path(_) if !landed => {
                self.selected = None;
                cx.notify();
                return;
            }
            LocationInput::Empty | LocationInput::Path(_) => all(),
        };
        self.selected = (!path && !self.shown.is_empty()).then_some(0);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Show `dir`, listed in the background.
    pub(crate) fn browse(&mut self, dir: SystemPath, window: &mut Window, cx: &mut Context<Self>) {
        self.revision += 1;
        let revision = self.revision;
        self.dir = dir.clone();
        self.listing = Listing::Loading;
        self.shown.clear();
        self.selected = None;
        self.new_folder = None;
        // Another folder: a lookup under way and its message are for the
        // old one.
        self.location_revision += 1;
        self.resolving = None;
        self.location_error = None;
        // Another folder: the field empties (no Change event), and its
        // placeholder names the new folder.
        let placeholder = system::path_to_string(&dir);
        self.location.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            // Read here, not at the call: the builder sets it after `new`.
            let files = this.read_with(cx, |c, _| c.files).ok().flatten();
            let listed = blocking(move || list(&dir, files)).await;
            this.update(cx, |chooser, cx| {
                chooser.listed(revision, listed.and_then(|r| r), cx)
            })
            .ok();
        }));
        cx.notify();
    }

    fn listed(
        &mut self,
        revision: u64,
        listed: Result<Vec<(SystemPath, bool)>, String>,
        cx: &mut Context<Self>,
    ) {
        if self.revision != revision {
            return;
        }
        self.task = None;
        match listed {
            Ok(rows) => {
                let (rows, is_file) = rows.into_iter().unzip();
                self.is_file = is_file;
                self.listing = Listing::Loaded(rows);
                // Text typed while it ran stays, and ranks it.
                self.refilter(true, cx);
            }
            Err(e) => self.listing = Listing::Failed(e),
        }
        cx.notify();
    }

    /// `Up` (Alt+Up or Cmd+Up, Backspace in the empty field): the parent
    /// folder.
    pub(crate) fn up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Not while a folder is being made: leaving drops the task, and
        // with it a failure the user should see.
        if self.creating() {
            return;
        }
        if let Some(parent) = self.dir.parent() {
            self.browse(parent, window, cx);
        }
    }

    /// Enter on a name or an empty field: into the selected subfolder, or
    /// pick the selected file (files mode).
    fn enter_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating() {
            return;
        }
        let Some(ix) = self.selected else {
            return;
        };
        let Some(target) = self.row(ix).cloned() else {
            return;
        };
        if self.is_file(ix) {
            cx.emit(FolderChooserEvent::ChosenFile(target));
        } else {
            self.browse(target, window, cx);
        }
    }

    /// Enter in the Location field. A path (`logic::onboarding::
    /// classify_location`) is looked up off the UI thread: a folder opens in
    /// the list (it is not chosen), a file with the files-mode extension is
    /// picked, anything else shows why under the field. A name or an empty
    /// field opens the selected row, as Enter on the list does.
    pub(crate) fn go_to_location(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating() {
            return;
        }
        let text = self.location.read(cx).value().to_string();
        let LocationInput::Path(path) = onb::classify_location(&text) else {
            self.enter_selected(window, cx);
            return;
        };
        self.location_revision += 1;
        let revision = self.location_revision;
        let current = self.dir.clone();
        let files = self.files;
        self.location_error = None;
        // The listing stays as it is while this runs.
        self.resolving = Some(cx.spawn_in(window, async move |this, cx| {
            let resolved = blocking(move || onb::resolve_location(&path, &current, files)).await;
            this.update_in(cx, |chooser, window, cx| {
                chooser.resolved(revision, resolved, window, cx)
            })
            .ok();
        }));
        cx.notify();
    }

    fn resolved(
        &mut self,
        revision: u64,
        resolved: Result<Result<onb::Resolved, onb::LocationError>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.location_revision != revision {
            return;
        }
        self.resolving = None;
        match resolved {
            Ok(Ok(onb::Resolved::Folder(dir))) => self.browse(dir, window, cx),
            Ok(Ok(onb::Resolved::File(path))) => cx.emit(FolderChooserEvent::ChosenFile(path)),
            Ok(Err(e)) => self.location_error = Some(e.to_string().into()),
            Err(e) => self.location_error = Some(e.into()),
        }
        cx.notify();
    }

    fn move_selection(&mut self, down: bool, cx: &mut Context<Self>) {
        let count = self.shown.len();
        if count == 0 {
            return;
        }
        let ix = match (self.selected, down) {
            (None, _) => 0,
            (Some(ix), true) => (ix + 1).min(count - 1),
            (Some(ix), false) => ix.saturating_sub(1),
        };
        self.selected = Some(ix);
        self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// `Choose this folder` (Ctrl+Enter, a double-click): the folder shown.
    /// Not in files mode, which picks a file.
    pub(crate) fn choose(&mut self, cx: &mut Context<Self>) {
        if self.creating() || self.files.is_some() {
            return;
        }
        cx.emit(FolderChooserEvent::Chosen(self.dir.clone()));
    }

    /// `Cancel` (Esc in the empty field): nothing chosen.
    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(FolderChooserEvent::Cancelled);
    }

    /// Whether the Location field holds no text.
    fn location_empty(&self, cx: &App) -> bool {
        self.location.read(cx).value().is_empty()
    }

    /// Esc: the Location field's text goes first (and its message, and a
    /// lookup under way, as at any key there); from the empty field the
    /// chooser is cancelled.
    pub(crate) fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.location_empty(cx) {
            return self.cancel(cx);
        }
        // `set_value` sends no Change event: do what a key there does.
        self.location
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.location_error = None;
        self.location_revision += 1;
        self.resolving = None;
        self.refilter(false, cx);
    }

    /// `New folder…` (Ctrl+N): the inline name field, focused.
    pub(crate) fn open_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_folder.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("New folder name"));
            self.new_folder = Some(NewFolder {
                input,
                error: None,
                task: None,
            });
        }
        if let Some(folder) = self.new_folder.as_ref() {
            let handle = folder.input.read(cx).focus_handle(cx);
            window.focus(&handle, cx);
        }
        cx.notify();
    }

    /// Esc in the new folder field: it closes; the keyboard goes back to the
    /// Location field.
    pub(crate) fn close_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_folder = None;
        self.focus_location(window, cx);
        cx.notify();
    }

    /// `Create` (Enter in the field): make the folder in the background and
    /// enter it.
    pub(crate) fn create_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.dir.clone();
        let Some(folder) = self.new_folder.as_mut() else {
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
            this.update_in(cx, |chooser, window, cx| {
                chooser.folder_made(made, window, cx)
            })
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
                self.focus_location(window, cx);
                self.browse(dir, window, cx);
            }
            Err(e) => {
                if let Some(folder) = self.new_folder.as_mut() {
                    folder.task = None;
                    folder.error = Some(format!("Couldn't create the folder: {e}").into());
                }
            }
        }
        cx.notify();
    }

    /// The modifiers of the key that goes up with Up: Cmd on macOS, Alt
    /// elsewhere, read from the keymap in effect (its platform), as the
    /// hints are.
    fn up_mods(cx: &App) -> Mods {
        match keymap::platform(cx) {
            Platform::MacOs => Mods::SUPER,
            Platform::LinuxWindows => Mods::ALT,
        }
    }

    /// The chooser's keys: those the kit's field gives no action for (Alt+Up,
    /// Ctrl+N, Ctrl+Enter or Cmd+Enter where the kit binds the other), seen
    /// on the chooser's root while the Location field has the keyboard, and
    /// every key the host hands over while its root has it. True when
    /// taken. A plain character is never taken: it is typed.
    pub(crate) fn key(
        &mut self,
        key: KeyInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let plain = key.mods == Mods::NONE;
        let primary = key.mods == Mods::CTRL || key.mods == Mods::SUPER;
        match key.key {
            Key::Enter if primary => self.choose(cx),
            Key::Enter if plain => self.go_to_location(window, cx),
            Key::Up if key.mods == Self::up_mods(cx) => self.up(window, cx),
            Key::Up if plain => self.move_selection(false, cx),
            Key::Down if plain => self.move_selection(true, cx),
            Key::Backspace if plain && self.location_empty(cx) => self.up(window, cx),
            Key::Esc if plain => self.escape(window, cx),
            // No new folder in files mode: a file is picked there.
            Key::Char('n') if primary && self.files.is_none() => self.open_new_folder(window, cx),
            _ => return false,
        }
        true
    }

    /// The kit's field actions in the chooser's fields (see the module
    /// docs). Each acts and stops the action, or lets it go on to the field.
    fn on_enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_folder_focused(window, cx) {
            self.create_folder(window, cx);
        } else if self.location_focused(window, cx) {
            if action.secondary {
                self.choose(cx);
            } else {
                self.go_to_location(window, cx);
            }
        } else {
            return;
        }
        cx.stop_propagation();
    }

    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.new_folder_focused(window, cx) {
            self.close_new_folder(window, cx);
        } else if self.location_focused(window, cx) {
            self.escape(window, cx);
        } else {
            return;
        }
        cx.stop_propagation();
    }

    fn on_move(&mut self, down: bool, window: &Window, cx: &mut Context<Self>) {
        if self.location_focused(window, cx) {
            self.move_selection(down, cx);
            cx.stop_propagation();
        }
    }

    /// Backspace edits the text; in the empty field it goes up.
    fn on_backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.location_focused(window, cx) && self.location_empty(cx) {
            self.up(window, cx);
            cx.stop_propagation();
        }
    }

    /// The kit binds Cmd+Up on macOS to the field's caret (start of the
    /// text): in the chooser it goes up, as Alt+Up does elsewhere. Cmd+Up is
    /// the kit's only macOS key for it; elsewhere it is Ctrl+Home, which
    /// stays the caret's. The action carries no keystroke, and the window's
    /// modifiers follow only modifier-change events, so neither is read.
    fn on_move_to_start(&mut self, _: &MoveToStart, window: &mut Window, cx: &mut Context<Self>) {
        if self.location_focused(window, cx) && keymap::platform(cx) == Platform::MacOs {
            self.up(window, cx);
            cx.stop_propagation();
        }
    }

    /// A key in the Location field that is not a kit action there.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.location_focused(window, cx) {
            return;
        }
        let Some(key) = crate::input::key_down_input(event) else {
            return;
        };
        if self.key(key, window, cx) {
            cx.stop_propagation();
        }
    }

    /// The key hints a host shows while a folder chooser is up.
    pub(crate) const HINTS: &'static str = "Up and Down: select · Enter: open or go to the path \
         · Alt+Up: up · Ctrl+Enter: choose · Ctrl+N: new folder · Esc: clear or back";

    /// [`Self::HINTS`] on macOS.
    pub(crate) const HINTS_MAC: &'static str = "Up and Down: select · Enter: open or go to the \
         path · Cmd+Up: up · Cmd+Enter: choose · Cmd+N: new folder · Esc: clear or back";

    /// [`Self::HINTS`] in files mode: no choose, no new folder.
    pub(crate) const FILE_HINTS: &'static str = "Up and Down: select · Enter: open a folder, \
         pick a file or go to the path · Alt+Up: up · Esc: clear or back";

    /// [`Self::FILE_HINTS`] on macOS.
    pub(crate) const FILE_HINTS_MAC: &'static str = "Up and Down: select · Enter: open a folder, \
         pick a file or go to the path · Cmd+Up: up · Esc: clear or back";

    /// The key hints for this chooser's mode, on the keymap's platform.
    pub(crate) fn hints(&self, cx: &App) -> &'static str {
        let mac = keymap::platform(cx) == Platform::MacOs;
        match (self.files.is_some(), mac) {
            (false, false) => Self::HINTS,
            (false, true) => Self::HINTS_MAC,
            (true, false) => Self::FILE_HINTS,
            (true, true) => Self::FILE_HINTS_MAC,
        }
    }
}

/// The listing of `dir`: its subfolders, or in files mode (`files`) its
/// folders and the files with that extension. Each with whether it is a
/// file. Blocking.
fn list(dir: &SystemPath, files: Option<&'static str>) -> Result<Vec<(SystemPath, bool)>, String> {
    match files {
        None => onb::list_subdirs(dir)
            .map(|dirs| dirs.into_iter().map(|d| (d, false)).collect())
            .map_err(|e| e.to_string()),
        Some(extension) => onb::list_files(dir, extension).map(|rows| {
            rows.into_iter()
                .map(|p| {
                    let file = !p.is_dir();
                    (p, file)
                })
                .collect()
        }),
    }
}

/// A quieter note.
fn note(text: impl Into<SharedString>, t: &Theme) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(theme::muted(t))
        .child(text.into())
}

impl Render for FolderChooser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let prefix = self.prefix;
        let list_height = list_height(window.viewport_size().height, window.rem_size());
        let has_parent = self.dir.parent().is_some();
        let body = match &self.listing {
            Listing::Loading => note("Loading…", t).into_any_element(),
            Listing::Failed(e) => div()
                .id(id(prefix, "error"))
                .test_support()
                .role(Role::Label)
                .aria_label(SharedString::from(e.clone()))
                .text_sm()
                .text_color(theme::error(t))
                .child(format!("Couldn't list this folder: {e}"))
                .into_any_element(),
            Listing::Loaded(rows) if rows.is_empty() => match self.files {
                Some(ext) => note(
                    format!("No folders or .{} files here.", ext.trim_start_matches('.')),
                    t,
                )
                .into_any_element(),
                None => note("No folders here. Choose this one, or make a new one.", t)
                    .into_any_element(),
            },
            Listing::Loaded(_) if self.shown.is_empty() => match self.files {
                Some(_) => note("No folders or files match.", t).into_any_element(),
                None => note("No folders match.", t).into_any_element(),
            },
            Listing::Loaded(_) => uniform_list(
                id(prefix, "list"),
                self.shown.len(),
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    let t = theme::current(cx);
                    let hover = theme::hover_background(&t);
                    range
                        .filter_map(|ix| this.row(ix).map(|d| (ix, d.clone())))
                        .map(|(ix, dir)| {
                            let selected = this.selected == Some(ix);
                            let name = folder_name(&dir);
                            let file = this.is_file(ix);
                            // The kind is told by an icon and in the
                            // label, not by the colour alone.
                            let (kind, icon, color) = if file {
                                ("file", IconName::FileText, theme::foreground(&t))
                            } else {
                                ("folder", IconName::Folder, theme::directory(&t))
                            };
                            div()
                                .id(if file {
                                    file_row_id(prefix, &dir)
                                } else {
                                    row_id(prefix, &dir)
                                })
                                .test_support()
                                .role(Role::ListBoxOption)
                                .aria_label(SharedString::from(format!("{name}, {kind}")))
                                .aria_selected(selected)
                                .flex()
                                .items_center()
                                .gap_2()
                                .w_full()
                                .h(rems(1.75))
                                .px_2()
                                .rounded_md()
                                .text_sm()
                                .text_color(color)
                                .when(selected, |row| row.bg(theme::selected_background(&t)))
                                .when(!selected, |row| row.hover(move |style| style.bg(hover)))
                                .child(Icon::new(icon).size_4().flex_none().text_color(color))
                                .child(div().min_w_0().truncate().child(name))
                                .on_click(cx.listener(
                                    move |this, event: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        // A row takes no focus: the press gave
                                        // it to the host's root. The field keeps
                                        // the keyboard, so typing goes on.
                                        this.focus_location(window, cx);
                                        if file {
                                            cx.emit(FolderChooserEvent::ChosenFile(dir.clone()));
                                        } else if event.click_count() >= 2 {
                                            // The first click entered it: take it.
                                            this.choose(cx);
                                        } else if !this.creating() {
                                            this.browse(dir.clone(), window, cx);
                                        }
                                    },
                                ))
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            // GPUI hands the wheel to every scroll container under the
            // pointer, so the host's pane scrolled with the list (and on
            // its own at the list's edge). The list's offset moves before
            // this bubble listener runs: stopping here keeps the wheel in
            // the list.
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .size_full()
            .into_any_element(),
        };
        let new_folder = self.new_folder.as_ref().map(|folder| {
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
                                    .id(SharedString::from(format!("{prefix}-new-folder")))
                                    .aria_label("New folder name")
                                    .disabled(folder.task.is_some())
                                    .bg(theme::background(t))
                                    .border_color(theme::divider(t))
                                    .text_color(theme::foreground(t)),
                            ),
                        )
                        .child(
                            Button::new(SharedString::from(format!("{prefix}-new-folder-create")))
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
            .id(id(prefix, ""))
            .test_support()
            .role(Role::Group)
            // Capture: ahead of the field's own handlers (it would take the
            // arrows to its caret), after the host's (which leave a chooser
            // field alone).
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(
                cx.listener(|this, _: &MoveUp, window, cx| this.on_move(false, window, cx)),
            )
            .capture_action(
                cx.listener(|this, _: &MoveDown, window, cx| this.on_move(true, window, cx)),
            )
            .capture_action(cx.listener(Self::on_backspace))
            .capture_action(cx.listener(Self::on_move_to_start))
            .on_key_down(cx.listener(Self::on_key_down))
            .aria_label(if self.files.is_some() {
                "Choose a file"
            } else {
                "Choose a folder"
            })
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
                        Button::new(id(prefix, "up"))
                            .label("Up")
                            .disabled(!has_parent)
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| this.up(window, cx)),
                            ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.location)
                                .id(id(prefix, "location"))
                                .aria_label(LOCATION_LABEL)
                                .bg(theme::background(t))
                                .border_color(theme::divider(t))
                                .text_color(theme::foreground(t)),
                        ),
                    ),
            )
            .when_some(self.location_error.clone(), |col, message| {
                col.child(
                    div()
                        .id(id(prefix, "location-error"))
                        .test_support()
                        .role(Role::Status)
                        .aria_label(message.clone())
                        .text_xs()
                        .text_color(theme::error(t))
                        .child(message),
                )
            })
            .children(new_folder)
            .child(
                div()
                    .id(id(prefix, "rows"))
                    .test_support()
                    .role(Role::ListBox)
                    // The folder is named here: the placeholder that shows
                    // it hides once text is typed.
                    .aria_label(SharedString::from(format!(
                        "{} in {}",
                        if self.files.is_some() {
                            "Folders and files"
                        } else {
                            "Folders"
                        },
                        system::path_to_string(&self.dir)
                    )))
                    // Its own height, from the window's, and no taller: a
                    // tall host leaves the rest empty. A host of fixed
                    // height (the setup card) shrinks it to fit, never
                    // under the floor: a short host scrolls instead.
                    .flex_shrink(1.)
                    .min_h(rems(LIST_MIN_REMS))
                    .h(list_height)
                    .p_1()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::divider(t))
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        // As on a row: the field keeps the keyboard.
                        this.focus_location(window, cx);
                        if event.click_count() >= 2 {
                            this.choose(cx);
                        }
                    }))
                    .child(body),
            )
    }
}

/// The chooser's commands (`New folder…`, `Cancel`, `Choose this folder`),
/// as a row of buttons for the host's footer, with a spacer before
/// `Cancel`. Files mode has only the spacer and `Cancel`: no `New folder…`
/// and no `Choose this folder`.
pub(crate) fn actions(chooser: &Entity<FolderChooser>, cx: &App) -> Vec<AnyElement> {
    let prefix = chooser.read(cx).prefix;
    let creating = chooser.read(cx).creating();
    let files = chooser.read(cx).files.is_some();
    let new = chooser.clone();
    let cancel = chooser.clone();
    let choose = chooser.clone();
    let mut buttons = Vec::new();
    // Files mode picks a file: a new, empty folder is no use there.
    if !files {
        buttons.push(
            Button::new(id(prefix, "new"))
                .label("New folder…")
                .disabled(creating)
                .on_click(move |_: &ClickEvent, window, cx| {
                    new.update(cx, |c, cx| c.open_new_folder(window, cx))
                })
                .into_any_element(),
        );
    }
    // The spacer stays, so Cancel keeps its side.
    buttons.push(div().flex_1().into_any_element());
    buttons.push(
        Button::new(id(prefix, "cancel"))
            .label("Cancel")
            .on_click(move |_: &ClickEvent, _, cx| cancel.update(cx, |c, cx| c.cancel(cx)))
            .into_any_element(),
    );
    if !files {
        buttons.push(
            Button::new(id(prefix, "choose"))
                .primary()
                .label("Choose this folder")
                .disabled(creating)
                .on_click(move |_: &ClickEvent, _, cx| choose.update(cx, |c, cx| c.choose(cx)))
                .into_any_element(),
        );
    }
    buttons
}
