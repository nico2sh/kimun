//! The **folder chooser** (M8c, shared since M9b-2): a panel inside a
//! full-window view (the setup assistant, Preferences > Workspaces), not a
//! dialog over it. The folder shown with Up, its subfolders (a click enters
//! one, a double-click or `Choose this folder` takes the folder shown) and
//! an inline `New folder…` field.
//!
//! **Behaviour owner.** The chooser entity: the folder shown, its listing,
//! the selection, the new folder field and the work under way. Listings
//! and the new folder run on tokio's blocking pool
//! (`logic::onboarding::list_subdirs`, `create_subdir`, through core); a
//! stale listing is dropped by revision.
//!
//! **Its host** draws it ([`FolderChooser`] renders the panel, [`actions`]
//! the command row the host places in its footer), keeps the keyboard on
//! its own root and hands the keys over ([`FolderChooser::key`]); Enter and
//! Esc in the new folder field come through the host's field actions
//! ([`FolderChooser::new_folder_focused`]). The chooser says what was picked
//! with [`FolderChooserEvent`]. Element ids carry the host's prefix
//! (`onboarding-chooser`, `preferences-chooser`).
//!
//! **Files mode** ([`FolderChooser::files`], off by default): the list also
//! shows the files with one extension (`logic::onboarding::list_files`:
//! hidden folders listed, hidden files left out). A folder works as before; a click or Enter on
//! a file picks it ([`FolderChooserEvent::ChosenFile`]). There is no
//! `Choose this folder` then.

use crate::tasks::blocking;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable as _, InteractiveElement, IntoElement, ParentElement, Render, Role, ScrollStrategy,
    SharedString, StatefulInteractiveElement, Styled, Task, UniformListScrollHandle, Window, div,
    rems, uniform_list,
};
use kimun_core::system::{self, SystemPath};
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::onboarding as onb;
use kimun_notes::settings::themes::Theme;

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
    /// `Choose this folder` (Ctrl+Enter, a double-click): this folder.
    Chosen(SystemPath),
    /// A click or Enter on a file (files mode only): this file.
    ChosenFile(SystemPath),
    /// `Cancel` (Esc): nothing chosen.
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
    /// Where the keyboard goes back to when the new folder field closes:
    /// the host's root.
    host_focus: FocusHandle,
    dir: SystemPath,
    listing: Listing,
    selected: Option<usize>,
    /// Bumped by every listing asked for: an older one landing is dropped.
    revision: u64,
    task: Option<Task<()>>,
    new_folder: Option<NewFolder>,
    scroll: UniformListScrollHandle,
    /// Files mode: the extension of the files listed beside the folders.
    files: Option<&'static str>,
    /// Which rows of the listing are files, by index.
    is_file: Vec<bool>,
}

impl EventEmitter<FolderChooserEvent> for FolderChooser {}

impl FolderChooser {
    /// A chooser showing `start` (listed in the background), its ids under
    /// `prefix`, the keyboard going back to `host_focus`. Needs the tokio
    /// runtime entered.
    pub(crate) fn new(
        prefix: &'static str,
        start: SystemPath,
        host_focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut chooser = Self {
            prefix,
            host_focus,
            dir: start.clone(),
            listing: Listing::Loading,
            selected: None,
            revision: 0,
            task: None,
            new_folder: None,
            scroll: UniformListScrollHandle::new(),
            files: None,
            is_file: Vec::new(),
        };
        chooser.browse(start, window, cx);
        chooser
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
    /// the host hands its Enter and Esc here).
    pub(crate) fn new_folder_focused(&self, window: &Window, cx: &App) -> bool {
        self.new_folder
            .as_ref()
            .is_some_and(|f| f.input.read(cx).focus_handle(cx).is_focused(window))
    }

    fn rows(&self) -> &[SystemPath] {
        match &self.listing {
            Listing::Loaded(rows) => rows,
            _ => &[],
        }
    }

    /// Whether row `ix` is a file (files mode).
    fn is_file(&self, ix: usize) -> bool {
        self.is_file.get(ix).copied().unwrap_or(false)
    }

    /// Show `dir`, listed in the background.
    pub(crate) fn browse(&mut self, dir: SystemPath, window: &mut Window, cx: &mut Context<Self>) {
        self.revision += 1;
        let revision = self.revision;
        self.dir = dir.clone();
        self.listing = Listing::Loading;
        self.selected = None;
        self.new_folder = None;
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
                self.selected = (!rows.is_empty()).then_some(0);
                let (rows, is_file) = rows.into_iter().unzip();
                self.is_file = is_file;
                self.listing = Listing::Loaded(rows);
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            Err(e) => self.listing = Listing::Failed(e),
        }
        cx.notify();
    }

    /// `Up` (Left, Backspace): the parent folder.
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

    /// Enter (Right): into the selected subfolder. Enter on a file (files
    /// mode) picks it; Right does nothing there.
    fn enter_selected(&mut self, pick: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating() {
            return;
        }
        let Some(ix) = self.selected else {
            return;
        };
        let Some(target) = self.rows().get(ix).cloned() else {
            return;
        };
        if !self.is_file(ix) {
            self.browse(target, window, cx);
        } else if pick {
            cx.emit(FolderChooserEvent::ChosenFile(target));
        }
    }

    fn move_selection(&mut self, down: bool, cx: &mut Context<Self>) {
        let count = self.rows().len();
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

    /// `Cancel` (Esc): nothing chosen.
    pub(crate) fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(FolderChooserEvent::Cancelled);
    }

    /// `New folder…` (n): the inline name field, focused.
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
    /// host.
    pub(crate) fn close_new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_folder = None;
        window.focus(&self.host_focus, cx);
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
                window.focus(&self.host_focus, cx);
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

    /// The chooser's keys, handed over by the host (never while the new
    /// folder field has the keyboard). True when taken.
    pub(crate) fn key(
        &mut self,
        key: KeyInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let plain = key.mods == Mods::NONE;
        match key.key {
            Key::Enter if key.mods.ctrl || key.mods.sup => self.choose(cx),
            Key::Enter if plain => self.enter_selected(true, window, cx),
            Key::Right if plain => self.enter_selected(false, window, cx),
            Key::Left | Key::Backspace if plain => self.up(window, cx),
            Key::Up if plain => self.move_selection(false, cx),
            Key::Down if plain => self.move_selection(true, cx),
            Key::Esc => self.cancel(cx),
            Key::Char('n') if plain => self.open_new_folder(window, cx),
            _ => return false,
        }
        true
    }

    /// The key hints a host shows while the chooser is up.
    pub(crate) const HINTS: &'static str =
        "↑/↓: select · Enter: open · ←: up · Ctrl+Enter: choose · N: new folder · Esc: back";

    /// [`Self::HINTS`] in files mode.
    pub(crate) const FILE_HINTS: &'static str =
        "↑/↓: select · Enter: open a folder or pick a file · ←: up · N: new folder · Esc: back";

    /// The key hints for this chooser's mode.
    pub(crate) fn hints(&self) -> &'static str {
        if self.files.is_some() {
            Self::FILE_HINTS
        } else {
            Self::HINTS
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let prefix = self.prefix;
        let path = system::path_to_string(&self.dir);
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
            Listing::Loaded(rows) => uniform_list(
                id(prefix, "list"),
                rows.len(),
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    let t = theme::current(cx);
                    let hover = theme::hover_background(&t);
                    range
                        .filter_map(|ix| this.rows().get(ix).map(|d| (ix, d.clone())))
                        .map(|(ix, dir)| {
                            let selected = this.selected == Some(ix);
                            let name = folder_name(&dir);
                            let file = this.is_file(ix);
                            div()
                                .id(if file {
                                    file_row_id(prefix, &dir)
                                } else {
                                    row_id(prefix, &dir)
                                })
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
                                .text_color(if file {
                                    theme::foreground(&t)
                                } else {
                                    theme::directory(&t)
                                })
                                .when(selected, |row| row.bg(theme::selected_background(&t)))
                                .when(!selected, |row| row.hover(move |style| style.bg(hover)))
                                .child(div().truncate().child(name))
                                .on_click(cx.listener(
                                    move |this, event: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
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
                        div()
                            .id(id(prefix, "path"))
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
                    .id(id(prefix, "rows"))
                    .role(Role::ListBox)
                    .aria_label(if self.files.is_some() {
                        "Folders and files"
                    } else {
                        "Folders"
                    })
                    .flex_1()
                    .min_h(rems(8.))
                    .h(rems(14.))
                    .p_1()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::divider(t))
                    .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                        if event.click_count() >= 2 {
                            this.choose(cx);
                        }
                    }))
                    .child(body),
            )
    }
}

/// The chooser's commands (`New folder…`, `Cancel`, `Choose this folder`),
/// as a row of buttons for the host's footer, with `spacer` between the
/// first and the other two.
/// In files mode there is no `Choose this folder`.
pub(crate) fn actions(chooser: &Entity<FolderChooser>, cx: &App) -> Vec<AnyElement> {
    let prefix = chooser.read(cx).prefix;
    let creating = chooser.read(cx).creating();
    let files = chooser.read(cx).files.is_some();
    let new = chooser.clone();
    let cancel = chooser.clone();
    let choose = chooser.clone();
    let mut buttons = vec![
        Button::new(id(prefix, "new"))
            .label("New folder…")
            .disabled(creating)
            .on_click(move |_: &ClickEvent, window, cx| {
                new.update(cx, |c, cx| c.open_new_folder(window, cx))
            })
            .into_any_element(),
        div().flex_1().into_any_element(),
        Button::new(id(prefix, "cancel"))
            .label("Cancel")
            .on_click(move |_: &ClickEvent, _, cx| cancel.update(cx, |c, cx| c.cancel(cx)))
            .into_any_element(),
    ];
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
