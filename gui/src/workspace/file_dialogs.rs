//! The **file-operation dialogs** (M6 G3): the note menu (`Rename…`,
//! `Move…`, `Delete…`; from the editor's right-click also `Properties…`,
//! M7a G2), the rename and move dialogs (kit `Dialog`), and the
//! delete confirmation's body (kit `AlertDialog`). Each is a view the modal
//! host opens in its one slot ([`super::modal`]); none of them touches the
//! vault beyond reading it (the availability checks, the directory list).
//! The operation itself is the workspace's ([`Workspace::request_file_op`]),
//! which flushes the open note first when the operation touches it.
//!
//! **Rules (the TUI dialogs').** Rename: the input starts on the entry's
//! file name; every edit re-checks the name (the latest check wins) and the
//! confirm runs only while the name is available. Move: the vault's folders,
//! fuzzy-filtered, the first row selected; the confirm runs unless the
//! destination is taken. Delete: `Delete "name"?`, a danger `Delete`.
//!
//! **Status.** While an operation runs its confirm is disabled and a second
//! request is ignored; a failure is shown in the dialog, which stays open
//! ([`OpStatus`], shared with the workspace).

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::AlertDialog;
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Rgba, Role,
    ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Subscription, Task,
    UniformListScrollHandle, WeakEntity, Window, div, rems, uniform_list,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::file_ops::{self, ValidationState};
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::file_ops::FileOpRequest;
use crate::input;
use crate::theme;

/// Element ids, for tests.
pub(crate) const MENU_ID: &str = "file-ops-menu";
pub(crate) const RENAME_INPUT_ID: &str = "rename-input";
pub(crate) const MOVE_FILTER_ID: &str = "move-filter";
pub(crate) const MOVE_LIST_ID: &str = "move-list";
pub(crate) const CONFIRM_ID: &str = "file-op-confirm";
pub(crate) const CANCEL_ID: &str = "file-op-cancel";
pub(crate) const ERROR_ID: &str = "file-op-error";

/// A row of the move dialog's list, by its folder.
pub(crate) fn move_row_id(dir: &VaultPath) -> SharedString {
    format!("move-row:{dir}").into()
}

/// A menu item's element id (`file-ops-rename`).
pub(crate) fn menu_item_id(kind: FileOpKind) -> &'static str {
    match kind {
        FileOpKind::Rename => "file-ops-rename",
        FileOpKind::Move => "file-ops-move",
        FileOpKind::Delete => "file-ops-delete",
        FileOpKind::Properties => "file-ops-properties",
    }
}

/// What the note menu offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileOpKind {
    Rename,
    Move,
    Delete,
    /// The note's properties dialog (the editor's menu only; not a file
    /// operation, but the same menu offers it).
    Properties,
}

impl FileOpKind {
    /// Every menu item, in order, with the key that picks each (the TUI
    /// menu's R, M, D; P for properties). The F2 menu shows them without
    /// the first.
    const MENU: [(FileOpKind, &'static str, char); 4] = [
        (FileOpKind::Properties, "Properties…", 'p'),
        (FileOpKind::Rename, "Rename…", 'r'),
        (FileOpKind::Move, "Move…", 'm'),
        (FileOpKind::Delete, "Delete…", 'd'),
    ];

    /// The items a menu shows: with `Properties…` (the editor's right-click)
    /// or the file operations alone (F2, unchanged).
    fn menu(with_properties: bool) -> &'static [(FileOpKind, &'static str, char)] {
        if with_properties {
            &Self::MENU
        } else {
            &Self::MENU[1..]
        }
    }
}

/// An operation's progress as its dialog shows it: running (the confirm is
/// disabled) and why the last one failed. Shared by the workspace, which
/// runs the operation, and the open dialog.
#[derive(Debug, Default)]
pub(crate) struct OpStatus {
    pub(crate) busy: bool,
    pub(crate) error: Option<SharedString>,
}

/// The error row under a dialog's fields, while there is one.
pub(super) fn error_row(status: &OpStatus, t: &Theme) -> Option<AnyElement> {
    let error = status.error.clone()?;
    Some(
        div()
            .id(ERROR_ID)
            .role(Role::Alert)
            .aria_label(error.clone())
            .text_sm()
            .text_color(theme::error(t))
            .child(error)
            .into_any_element(),
    )
}

/// What a rename or move check says, and in which colour.
fn hint(state: ValidationState, invalid: Option<&str>, t: &Theme) -> Option<(SharedString, Rgba)> {
    if let Some(reason) = invalid {
        return Some((SharedString::from(reason.to_string()), theme::error(t)));
    }
    match state {
        ValidationState::Idle => None,
        ValidationState::Pending => Some(("Checking…".into(), theme::muted(t))),
        ValidationState::Available => Some(("Available".into(), theme::available(t))),
        ValidationState::Taken => Some(("Already exists".into(), theme::error(t))),
    }
}

/// The dialog's `Cancel` and its result verb, the verb disabled unless
/// `enabled`.
pub(super) fn footer(
    verb: &'static str,
    enabled: bool,
    busy: bool,
    on_confirm: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .justify_end()
        .gap_2()
        .pt_2()
        .child(
            Button::new(CANCEL_ID)
                .label("Cancel")
                .disabled(busy)
                .on_click(on_cancel),
        )
        .child(
            Button::new(CONFIRM_ID)
                .primary()
                .label(verb)
                .loading(busy)
                .disabled(!enabled || busy)
                .on_click(on_confirm),
        )
}

/// The path line over a dialog's fields: what is being renamed or moved.
pub(super) fn path_line(label: &'static str, target: &VaultPath, t: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(theme::muted(t)).child(label))
        .child(
            div()
                .text_sm()
                .text_color(theme::foreground(t))
                .truncate()
                .child(target.to_string()),
        )
}

/// Ask the workspace to close the open modal (unless an operation runs).
fn close(workspace: &WeakEntity<Workspace>, window: &mut Window, cx: &mut App) {
    workspace
        .update(cx, |workspace, cx| {
            workspace.dismiss_file_dialog(window, cx)
        })
        .ok();
}

// ---- The note menu -----------------------------------------------------------

/// The note menu: what can be done to one entry. Picked with a click, the
/// item's letter, or Up/Down and Enter.
pub(crate) struct FileOpsMenu {
    target: VaultPath,
    /// The items shown ([`FileOpKind::menu`]).
    items: &'static [(FileOpKind, &'static str, char)],
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    selected: usize,
}

impl FileOpsMenu {
    pub(crate) fn new(
        target: VaultPath,
        with_properties: bool,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            target,
            items: FileOpKind::menu(with_properties),
            workspace,
            focus: cx.focus_handle(),
            selected: 0,
        }
    }

    /// Run the item `kind`: this menu gives way to its dialog.
    pub(crate) fn choose(&self, kind: FileOpKind, window: &mut Window, cx: &mut App) {
        let target = self.target.clone();
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.choose_file_op(kind, target, window, cx)
            })
            .ok();
    }

    /// Run the selected item (Enter).
    pub(crate) fn choose_selected(&self, window: &mut Window, cx: &mut App) {
        self.choose(self.items[self.selected].0, window, cx);
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(KeyInput { key, mods }) = input::key_down_input(event) else {
            return;
        };
        if mods != Mods::NONE && mods != Mods::SHIFT {
            return;
        }
        match key {
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => self.selected = (self.selected + 1).min(self.items.len() - 1),
            Key::Char(c) => {
                let c = c.to_ascii_lowercase();
                // Every other letter is swallowed: the menu is modal.
                if let Some((kind, ..)) = self.items.iter().find(|(.., k)| *k == c) {
                    cx.stop_propagation();
                    self.choose(*kind, window, cx);
                    return;
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Focusable for FileOpsMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for FileOpsMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let rows = self
            .items
            .iter()
            .enumerate()
            .map(|(ix, (kind, label, key))| {
                let kind = *kind;
                let selected = ix == self.selected;
                let hover = theme::hover_background(t);
                let text = if kind == FileOpKind::Delete {
                    theme::error(t)
                } else {
                    theme::foreground(t)
                };
                div()
                    .id(menu_item_id(kind))
                    .test_support()
                    .role(Role::MenuItem)
                    .aria_label(*label)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(selected, |row| row.bg(theme::selected_background(t)))
                    .when(!selected, |row| row.hover(move |style| style.bg(hover)))
                    // Delete stands apart from the harmless items, and the
                    // file operations from the properties above them.
                    .when(
                        kind == FileOpKind::Delete || (kind == FileOpKind::Rename && ix > 0),
                        |row| row.mt_1().border_t_1().border_color(theme::divider(t)),
                    )
                    .child(div().flex_1().text_sm().text_color(text).child(*label))
                    .child(Kbd::new(input::keystroke(KeyInput::plain(Key::Char(
                        key.to_ascii_uppercase(),
                    )))))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.choose(kind, window, cx)
                    }))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        div()
            .id(MENU_ID)
            .test_support()
            .role(Role::Menu)
            .aria_label("File operations")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .gap_0p5()
            .children(rows)
    }
}

// ---- Rename -------------------------------------------------------------------

/// The rename dialog: the entry's new name, checked as it is typed.
pub(crate) struct RenameDialog {
    target: VaultPath,
    pub(crate) input: Entity<InputState>,
    vault: Arc<NoteVault>,
    workspace: WeakEntity<Workspace>,
    status: Entity<OpStatus>,
    state: ValidationState,
    /// The path the typed name gives, or why it gives none. `None` before
    /// the first edit.
    candidate: Option<Result<VaultPath, String>>,
    /// The availability check under way; a newer one replaces it.
    check: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl RenameDialog {
    pub(crate) fn new(
        target: VaultPath,
        vault: Arc<NoteVault>,
        status: Entity<OpStatus>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = target.get_parent_path().1;
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let subscriptions = vec![
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.edited(cx);
                }
            }),
            cx.observe(&status, |_, _, cx| cx.notify()),
        ];
        Self {
            target,
            input,
            vault,
            workspace,
            status,
            state: ValidationState::Idle,
            candidate: None,
            check: None,
            _subscriptions: subscriptions,
        }
    }

    #[cfg(test)]
    pub(crate) fn state(&self) -> ValidationState {
        self.state
    }

    #[cfg(test)]
    pub(crate) fn target(&self) -> &VaultPath {
        &self.target
    }

    /// The name changed: check the path it gives (the latest check wins).
    fn edited(&mut self, cx: &mut Context<Self>) {
        self.status.update(cx, |status, cx| {
            if status.error.take().is_some() {
                cx.notify();
            }
        });
        let value = self.input.read(cx).value().to_string();
        let candidate = file_ops::rename_target(&self.target, &value);
        self.candidate = Some(candidate.clone());
        self.check = None;
        match candidate {
            Ok(path) => {
                self.state = ValidationState::Pending;
                let vault = self.vault.clone();
                let checked = path.clone();
                self.check = Some(cx.spawn(async move |this, cx| {
                    let free =
                        tokio::spawn(async move { file_ops::availability(&vault, &checked).await })
                            .await;
                    this.update(cx, |dialog, cx| {
                        if dialog.candidate == Some(Ok(path)) {
                            dialog.state = match free {
                                Ok(free) => free.into(),
                                Err(_) => ValidationState::Taken,
                            };
                            cx.notify();
                        }
                    })
                    .ok();
                }));
            }
            Err(_) => self.state = ValidationState::Taken,
        }
        cx.notify();
    }

    fn confirmable(&self) -> Option<VaultPath> {
        match (&self.candidate, self.state) {
            (Some(Ok(to)), ValidationState::Available) => Some(to.clone()),
            _ => None,
        }
    }

    /// Rename, if the name is available and nothing runs.
    pub(crate) fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status.read(cx).busy {
            return;
        }
        let Some(to) = self.confirmable() else {
            return;
        };
        let request = FileOpRequest::Rename {
            from: self.target.clone(),
            to,
        };
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.request_file_op(request, window, cx)
            })
            .ok();
    }
}

impl Render for RenameDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let status = self.status.read(cx);
        let busy = status.busy;
        let invalid = match &self.candidate {
            Some(Err(reason)) => Some(reason.as_str()),
            _ => None,
        };
        let hint = hint(self.state, invalid, t);
        let enabled = self.confirmable().is_some();
        let confirm = cx.entity().downgrade();
        let workspace = self.workspace.clone();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                this.confirm(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                close(&this.workspace, window, cx);
                cx.stop_propagation();
            }))
            .child(path_line("Renaming", &self.target, t))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(t))
                            .child("New name"),
                    )
                    .child(
                        Input::new(&self.input)
                            .id(RENAME_INPUT_ID)
                            .aria_label("New name"),
                    ),
            )
            .child(
                div()
                    .h(rems(1.25))
                    .text_sm()
                    .when_some(hint, |row, (text, color)| row.text_color(color).child(text)),
            )
            .children(error_row(status, t))
            .child(footer(
                "Rename",
                enabled,
                busy,
                move |_, window, cx| {
                    confirm
                        .update(cx, |dialog, cx| dialog.confirm(window, cx))
                        .ok();
                },
                move |_, window, cx| close(&workspace, window, cx),
            ))
    }
}

// ---- Move ---------------------------------------------------------------------

/// The move dialog: a destination folder, picked from the vault's folders.
pub(crate) struct MoveDialog {
    target: VaultPath,
    pub(crate) filter: Entity<InputState>,
    vault: Arc<NoteVault>,
    workspace: WeakEntity<Workspace>,
    status: Entity<OpStatus>,
    /// Every folder, the root first; `None` while loading.
    dirs: Option<Vec<VaultPath>>,
    /// The folders the filter keeps, best first.
    visible: Vec<VaultPath>,
    selected: Option<usize>,
    state: ValidationState,
    /// Where the selected folder would put the entry, or why it cannot.
    candidate: Option<Result<VaultPath, String>>,
    check: Option<Task<()>>,
    _load: Option<Task<()>>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

/// A folder as the move list shows it.
fn dir_label(dir: &VaultPath) -> SharedString {
    if dir.is_root_or_empty() {
        "/ (vault root)".into()
    } else {
        dir.to_string().into()
    }
}

impl MoveDialog {
    pub(crate) fn new(
        target: VaultPath,
        vault: Arc<NoteVault>,
        status: Entity<OpStatus>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter folders"));
        let subscriptions = vec![
            cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refilter(cx);
                }
            }),
            cx.observe(&status, |_, _, cx| cx.notify()),
        ];
        // The folder walk blocks: off the UI thread, and off the runtime's
        // own thread too. The outer task wakes this one from the runtime.
        let walk = vault.clone();
        let load = cx.spawn(async move |this, cx| {
            let dirs = tokio::spawn(async move {
                tokio::task::spawn_blocking(move || file_ops::list_directories(&walk)).await
            })
            .await;
            let dirs = match dirs {
                Ok(Ok(Ok(dirs))) => Ok(dirs),
                Ok(Ok(Err(e))) => Err(e),
                Ok(Err(e)) | Err(e) => Err(e.to_string()),
            };
            this.update(cx, |dialog, cx| dialog.loaded(dirs, cx)).ok();
        });
        Self {
            target,
            filter,
            vault,
            workspace,
            status,
            dirs: None,
            visible: Vec::new(),
            selected: None,
            state: ValidationState::Idle,
            candidate: None,
            check: None,
            _load: Some(load),
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    #[cfg(test)]
    pub(crate) fn selected_dir(&self) -> Option<VaultPath> {
        self.selected.and_then(|ix| self.visible.get(ix).cloned())
    }

    #[cfg(test)]
    pub(crate) fn state(&self) -> ValidationState {
        self.state
    }

    #[cfg(test)]
    pub(crate) fn is_loading(&self) -> bool {
        self.dirs.is_none()
    }

    fn loaded(&mut self, dirs: Result<Vec<VaultPath>, String>, cx: &mut Context<Self>) {
        match dirs {
            Ok(dirs) => self.dirs = Some(dirs),
            Err(e) => {
                self.dirs = Some(Vec::new());
                self.status.update(cx, |status, cx| {
                    status.error = Some(format!("Couldn't list the folders: {e}").into());
                    cx.notify();
                });
            }
        }
        self.refilter(cx);
    }

    /// The filter changed (or the folders arrived): rank them, select the
    /// first.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value().to_string();
        self.visible = self
            .dirs
            .as_deref()
            .map(|dirs| file_ops::filter_directories(dirs, &query))
            .unwrap_or_default();
        self.select((!self.visible.is_empty()).then_some(0), cx);
    }

    /// Select the row at `ix` and check where it would put the entry.
    fn select(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        self.selected = ix;
        self.check = None;
        let Some(dest) = ix.and_then(|ix| self.visible.get(ix)).cloned() else {
            self.candidate = None;
            self.state = ValidationState::Idle;
            cx.notify();
            return;
        };
        self.scroll
            .scroll_to_item(ix.unwrap_or(0), ScrollStrategy::Nearest);
        let candidate = file_ops::move_target(&self.target, &dest);
        self.candidate = Some(candidate.clone());
        match candidate {
            Ok(path) => {
                self.state = ValidationState::Pending;
                let vault = self.vault.clone();
                let checked = path.clone();
                self.check = Some(cx.spawn(async move |this, cx| {
                    let free =
                        tokio::spawn(async move { file_ops::availability(&vault, &checked).await })
                            .await;
                    this.update(cx, |dialog, cx| {
                        if dialog.candidate == Some(Ok(path)) {
                            dialog.state = match free {
                                Ok(free) => free.into(),
                                Err(_) => ValidationState::Taken,
                            };
                            cx.notify();
                        }
                    })
                    .ok();
                }));
            }
            Err(_) => self.state = ValidationState::Taken,
        }
        cx.notify();
    }

    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        let ix = match (self.selected, down) {
            (None, _) => 0,
            (Some(ix), true) => (ix + 1).min(last),
            (Some(ix), false) => ix.saturating_sub(1),
        };
        if Some(ix) != self.selected {
            self.select(Some(ix), cx);
        }
    }

    /// Where the confirm would move the entry: only a taken (or impossible)
    /// destination blocks it — a check still running does not (TUI rule;
    /// core refuses a destination that exists).
    fn confirmable(&self) -> Option<VaultPath> {
        match (&self.candidate, self.state) {
            (Some(Ok(_)), ValidationState::Taken) => None,
            (Some(Ok(to)), _) => Some(to.clone()),
            _ => None,
        }
    }

    pub(crate) fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.status.read(cx).busy {
            return;
        }
        let Some(to) = self.confirmable() else {
            return;
        };
        let request = FileOpRequest::Move {
            from: self.target.clone(),
            to,
        };
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.request_file_op(request, window, cx)
            })
            .ok();
    }

    fn render_row(&self, ix: usize, dir: &VaultPath, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let selected = self.selected == Some(ix);
        let hover = theme::hover_background(t);
        div()
            .id(move_row_id(dir))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(dir_label(dir))
            .aria_selected(selected)
            .flex()
            .items_center()
            .w_full()
            .h(rems(1.75))
            .px_2()
            .rounded_md()
            .text_sm()
            .text_color(theme::directory(t))
            .when(selected, |row| row.bg(theme::selected_background(t)))
            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
            .child(div().truncate().child(dir_label(dir)))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 && this.selected == Some(ix) {
                    this.confirm(window, cx);
                } else {
                    this.select(Some(ix), cx);
                }
            }))
            .into_any_element()
    }
}

impl Render for MoveDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let status = self.status.read(cx);
        let busy = status.busy;
        let invalid = match &self.candidate {
            Some(Err(reason)) => Some(reason.as_str()),
            _ => None,
        };
        let hint = hint(self.state, invalid, t);
        let enabled = self.confirmable().is_some();
        let error = error_row(status, t);
        let count = self.visible.len();
        let list = if count > 0 {
            uniform_list(
                MOVE_LIST_ID,
                count,
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    let rows: Vec<(usize, VaultPath)> = range
                        .filter_map(|ix| this.visible.get(ix).map(|d| (ix, d.clone())))
                        .collect();
                    rows.iter()
                        .map(|(ix, dir)| this.render_row(*ix, dir, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full()
            .into_any_element()
        } else {
            let note = if self.dirs.is_none() {
                "(loading...)"
            } else {
                "(no matches)"
            };
            div()
                .px_2()
                .text_sm()
                .text_color(theme::muted(t))
                .child(note)
                .into_any_element()
        };
        let confirm = cx.entity().downgrade();
        let workspace = self.workspace.clone();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                this.confirm(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                close(&this.workspace, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.step(false, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.step(true, cx);
                cx.stop_propagation();
            }))
            .child(path_line("Moving", &self.target, t))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(t))
                            .child("Destination"),
                    )
                    .child(
                        Input::new(&self.filter)
                            .id(MOVE_FILTER_ID)
                            .aria_label("Filter folders"),
                    )
                    .child(
                        div()
                            .id("move-rows")
                            .role(Role::ListBox)
                            .aria_label("Folders")
                            .h(rems(14.))
                            .child(list),
                    ),
            )
            .child(
                div()
                    .h(rems(1.25))
                    .text_sm()
                    .when_some(hint, |row, (text, color)| row.text_color(color).child(text)),
            )
            .children(error)
            .child(footer(
                "Move here",
                enabled,
                busy,
                move |_, window, cx| {
                    confirm
                        .update(cx, |dialog, cx| dialog.confirm(window, cx))
                        .ok();
                },
                move |_, window, cx| close(&workspace, window, cx),
            ))
    }
}

// ---- Delete -------------------------------------------------------------------

/// Ask before a delete, in a kit `AlertDialog` over whatever is open: the
/// title (`Delete "name"?`), what goes (`description`), a danger `Delete`
/// and `Cancel`. `confirmed` adds the rest — always `on_ok` (its return
/// value says whether the kit closes the alert), and an `on_cancel` or a
/// body when the delete reports into the alert.
pub(crate) fn confirm_delete(
    window: &mut Window,
    cx: &mut App,
    title: String,
    description: &'static str,
    confirmed: impl Fn(AlertDialog) -> AlertDialog + 'static,
) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        confirmed(
            alert
                .confirm()
                .title(title.clone())
                .description(description)
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel"),
        )
    });
}

/// What the delete confirmation shows under its title and description: the
/// error of a delete that failed. The title, the consequence and the
/// buttons are the kit `AlertDialog`'s.
pub(crate) struct DeleteBody {
    status: Entity<OpStatus>,
    _status_watch: Subscription,
}

impl DeleteBody {
    pub(crate) fn new(status: Entity<OpStatus>, cx: &mut Context<Self>) -> Self {
        let watch = cx.observe(&status, |_, _, cx| cx.notify());
        Self {
            status,
            _status_watch: watch,
        }
    }
}

impl Render for DeleteBody {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().children(error_row(self.status.read(cx), &theme::current(cx)))
    }
}
