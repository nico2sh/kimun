//! The **FILES** drawer view (M5b-2 G3): one directory of the vault, its
//! breadcrumb, a filter, and its rows.
//!
//! **State.** The rules are the logic [`FilesModel`]'s (the listing, its
//! order, the filter and the `Create:` row, the selection, the open-note
//! marker, what activating a row means); this view draws its
//! [`ListCore`](kimun_notes::logic::list::ListCore) and turns pointer and
//! keyboard input into model calls. The filter text lives in the kit's
//! [`InputState`] and is handed to the model on every change.
//!
//! **Keys.** The filter has the keyboard while FILES has it, so typing
//! filters. Up/Down move the selection and Enter activates it; Left goes up a folder while the filter is empty (it moves the caret otherwise);
//! Escape clears the filter, then gives the keyboard back to the editor.
//! These are the filter's own actions, caught on their way to it.
//!
//! **Pointer.** A click selects a row and activates it, as Enter does
//! ([`ClickPolicy::Open`]: the pointing hand over every row); a right
//! click shows an entry's menu. The sort chip beside the breadcrumb opens the sort dialog
//! ([`super::sort`]).
//!
//! **Activation.** A folder (or `..`) is shown here; everything else is
//! the workspace's to do ([`FilesEvent`]): open a note, show an attachment
//! in the editor Panel. `Create:` makes the note through core and opens it.
//!
//! The [`Workspace`] glue at the bottom keeps the view in step with the
//! window's app channel: the listing's redraws, file operations, and the
//! note the editor opens and saves.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::breadcrumb::{Breadcrumb, BreadcrumbItem};
use gpui_kit::component::input::{
    Enter, Escape, InputEvent as FilterEvent, InputState, MoveDown, MoveLeft, MoveUp,
};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, Context, CursorStyle, Div, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, Role,
    ScrollStrategy, SharedString, Stateful, StatefulInteractiveElement, Styled, Subscription,
    UniformListScrollHandle, WeakEntity, Window, div, rems, uniform_list,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::files::{Activation, FileListEntry, FilesModel, spawn_create_and_open};
use kimun_notes::logic::layout::{Panel, Region};
use kimun_notes::logic::message::SortTarget;
use kimun_notes::logic::message::{AppTx, FileOp};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::session::Saved;
use kimun_notes::logic::sort::SortState;
use kimun_notes::settings::AppSettings;
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::file_dialogs::FileOpKind;
use super::query_field::query_field;
use super::sort::{chip_label, sort_chip};
use crate::theme;

/// Element ids, for tests.
pub(crate) const FILES_ID: &str = "files";
pub(crate) const FILTER_ID: &str = "files-filter";
pub(crate) const LIST_ID: &str = "files-list";
pub(crate) const CRUMBS_ID: &str = "files-breadcrumb";
pub(crate) const FILES_SORT_ID: &str = "files-sort";

/// A note row's height, in rems: two lines of small text and their gap
/// (a title over its filename). Every FILES row has it (the list is
/// virtualised over uniform rows; a folder's one line sits centred in it),
/// and so do the other lists of notes over [`lane_row`]: the pickers,
/// LINKS, the saved searches, the pinned notes, the Sources view (a
/// source's heading over its path).
pub(super) const NOTE_ROW_REMS: f32 = 2.75;

/// The most breadcrumb parts shown: deeper, the root, an ellipsis and the
/// last two folders.
const CRUMBS_SHOWN: usize = 4;

/// The element id of `row`: its path, so a row keeps its identity across a
/// re-sort or a filter (never its position).
pub(crate) fn row_id(row: &FileListEntry) -> SharedString {
    match row {
        FileListEntry::Up { .. } => "files-row-up".into(),
        FileListEntry::CreateNote { .. } => "files-row-create".into(),
        other => format!("files-row:{}", other.path()).into(),
    }
}

/// The Lucide icon of `row`.
pub(crate) fn row_icon(row: &FileListEntry) -> IconName {
    match row {
        FileListEntry::Up { .. } => IconName::CornerLeftUp,
        FileListEntry::Directory { .. } => IconName::Folder,
        FileListEntry::Note {
            journal_date: Some(_),
            ..
        } => IconName::CalendarDays,
        FileListEntry::Note { .. } => IconName::FileText,
        FileListEntry::Attachment { .. } => IconName::Paperclip,
        FileListEntry::CreateNote { .. } => IconName::Plus,
    }
}

/// Every icon a row or the breadcrumb can show (the asset test checks they
/// are bundled).
#[cfg(test)]
pub(crate) const ICONS: [IconName; 7] = [
    IconName::CornerLeftUp,
    IconName::Folder,
    IconName::CalendarDays,
    IconName::FileText,
    IconName::Paperclip,
    IconName::Plus,
    IconName::Circle,
];

/// What the workspace does for FILES.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FilesEvent {
    /// Open this note in the editor and give it the keyboard.
    Open(VaultPath),
    /// A `Create:` row made this note; it opens when core has it: give the
    /// editor the keyboard.
    Creating(VaultPath),
    /// Show this attachment in the editor Panel and give it the keyboard.
    OpenAttachment(VaultPath),
    /// Escape with an empty filter: back to the editor.
    Dismiss,
    /// A row's context menu asked to rename, move or delete its entry.
    FileOp(FileOpKind, VaultPath),
    /// The sort chip: open the sort dialog on FILES.
    Sort,
}

/// The FILES view. See the module docs.
pub(crate) struct FilesPane {
    model: FilesModel,
    filter: Entity<InputState>,
    scroll: UniformListScrollHandle,
    tx: AppTx,
    theme: Theme,
    /// The workspace's name: the breadcrumb's root.
    root_name: SharedString,
    _filter_events: Subscription,
}

impl EventEmitter<FilesEvent> for FilesPane {}

impl FilesPane {
    pub(crate) fn new(
        vault: Arc<NoteVault>,
        root_name: SharedString,
        settings: &AppSettings,
        tx: AppTx,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter or create"));
        let filter_events = cx.subscribe(&filter, |this, input, event: &FilterEvent, cx| {
            if matches!(event, FilterEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.set_query(query, cx);
            }
        });
        Self {
            model: FilesModel::new(vault, settings),
            filter,
            scroll: UniformListScrollHandle::new(),
            tx,
            theme,
            root_name,
            _filter_events: filter_events,
        }
    }

    /// The listing's rules, for reads (tests).
    #[cfg(test)]
    pub(crate) fn model(&self) -> &FilesModel {
        &self.model
    }

    /// Nothing listed yet (no directory navigated to).
    pub(crate) fn is_empty(&self) -> bool {
        self.model.is_empty()
    }

    pub(crate) fn current_dir(&self) -> &VaultPath {
        self.model.current_dir()
    }

    /// The filter's text.
    pub(crate) fn query(&self, cx: &App) -> String {
        self.filter.read(cx).value().to_string()
    }

    // ---- The listing -----------------------------------------------------------

    /// Show `dir`, keeping the filter (for a navigation the user did not
    /// ask for here: the first note opened, a folder opened elsewhere).
    pub(crate) fn navigate(&mut self, dir: VaultPath, cx: &mut Context<Self>) {
        self.model.navigate(dir, &self.tx);
        self.after_rebuild(cx);
    }

    /// Show `dir` with the filter cleared: the user went there.
    pub(crate) fn enter_dir(
        &mut self,
        dir: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_filter(window, cx);
        self.navigate(dir, cx);
    }

    /// Something changed in `dir`: list it again if it is the one shown.
    pub(crate) fn refresh_if_showing(&mut self, dir: &VaultPath, cx: &mut Context<Self>) {
        if dir.is_like(self.model.current_dir()) {
            self.model.refresh_if_showing(dir, &self.tx);
            self.after_rebuild(cx);
        }
    }

    /// A fresh listing starts with no query: give it the filter's, and start
    /// at its top.
    fn after_rebuild(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        if let Some(core) = self.model.core_mut() {
            core.set_query(query);
        }
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Apply what the load delivered (on the redraw it sends).
    pub(crate) fn poll(&mut self, cx: &mut Context<Self>) {
        let polled = self.model.poll();
        if polled.changed {
            if polled.restarted {
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            cx.notify();
        }
    }

    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        if let Some(core) = self.model.core_mut() {
            core.set_query(query);
        }
        self.reveal_selection();
        cx.notify();
    }

    fn clear_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query(cx).is_empty() {
            return;
        }
        // `set_value` reports no change: hand the model the empty query too.
        self.filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.set_query(String::new(), cx);
    }

    // ---- The sort -----------------------------------------------------------------

    /// What the sort dialog opens on: the listing's field, order and
    /// grouping.
    pub(crate) fn sort_state(&self) -> SortState {
        self.model.sort_state()
    }

    /// A sort dialog pick: the rows re-sort in place (a property sort once
    /// its values land, read off the UI thread; see
    /// `FilesModel::apply_sort_state`).
    pub(crate) fn apply_sort(&mut self, state: &SortState, cx: &mut Context<Self>) {
        self.model.apply_sort_state(state, &self.tx);
        self.reveal_selection();
        cx.notify();
    }

    // ---- The open note ---------------------------------------------------------

    /// The editor opened `note`, or shows this attachment (the marker
    /// follows it).
    pub(crate) fn set_open_note(&mut self, note: Option<VaultPath>, cx: &mut Context<Self>) {
        self.model.set_open_note(note);
        cx.notify();
    }

    /// `saved` was written: its row shows the title it was saved with.
    pub(crate) fn note_saved(&mut self, saved: &Saved, cx: &mut Context<Self>) {
        let title = FileListEntry::display_title(saved.title.clone());
        self.model.update_note_row(&saved.path, &title);
        cx.notify();
    }

    /// Keep the listing in step with a file operation that finished (the
    /// TUI's row effects; dialogs and the open note are M6's).
    pub(crate) fn file_op(&mut self, op: &FileOp, cx: &mut Context<Self>) {
        match op {
            FileOp::Created(path) | FileOp::Deleted(path) => {
                self.refresh_if_showing(&path.get_parent_path().0, cx);
            }
            // Gone from one folder, arrived in another: either may be shown.
            FileOp::Moved { from, to } => {
                self.refresh_if_showing(&from.get_parent_path().0, cx);
                self.refresh_if_showing(&to.get_parent_path().0, cx);
            }
            FileOp::Renamed { from, to } if from.is_note() => {
                self.model.rename_note_row(from, to);
                cx.notify();
            }
            FileOp::Renamed { from, .. } => {
                self.refresh_if_showing(&from.get_parent_path().0, cx);
            }
            _ => {}
        }
    }

    // ---- Selection and activation ----------------------------------------------

    fn select_next(&mut self, cx: &mut Context<Self>) {
        if let Some(core) = self.model.core_mut() {
            core.select_next();
        }
        self.reveal_selection();
        cx.notify();
    }

    fn select_prev(&mut self, cx: &mut Context<Self>) {
        if let Some(core) = self.model.core_mut() {
            core.select_prev();
        }
        self.reveal_selection();
        cx.notify();
    }

    fn reveal_selection(&self) {
        if let Some(pos) = self.model.core().and_then(|c| c.selected()) {
            self.scroll.scroll_to_item(pos, ScrollStrategy::Nearest);
        }
    }

    /// Do what the selected row asks for.
    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(activation) = self.model.activate() else {
            return;
        };
        match activation {
            Activation::Navigate(dir) => self.enter_dir(dir, window, cx),
            Activation::Open(path) => cx.emit(FilesEvent::Open(path)),
            Activation::OpenAttachment(path) => cx.emit(FilesEvent::OpenAttachment(path)),
            Activation::Create(path) => {
                spawn_create_and_open(self.model.vault().clone(), path.clone(), self.tx.clone());
                // The note is made: the filter that named it has done its job.
                self.clear_filter(window, cx);
                cx.emit(FilesEvent::Creating(path));
            }
        }
    }

    /// Up one folder (Left with an empty filter). True when
    /// there was one to go up to.
    fn go_up(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let dir = self.model.current_dir();
        if self.model.is_empty() || dir.is_root_or_empty() {
            return false;
        }
        let parent = dir.get_parent_path().0;
        self.enter_dir(parent, window, cx);
        true
    }

    // ---- Rendering --------------------------------------------------------------

    /// The breadcrumb: the vault root, then each folder down to the shown
    /// one, each a way back up. Built from the path's slices, so nothing
    /// here knows the separator.
    fn render_breadcrumb(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dir = self.model.current_dir().clone();
        let mut crumbs: Vec<(SharedString, VaultPath)> =
            vec![(self.root_name.clone(), VaultPath::root())];
        let mut at = VaultPath::root();
        for slice in dir.get_slices() {
            at = at.append(&VaultPath::new(&slice));
            crumbs.push((slice.into(), at.clone()));
        }
        // Deep down, the middle folders give way to an ellipsis, so the
        // folder shown (the end) stays in view.
        let mut elided = None;
        if crumbs.len() > CRUMBS_SHOWN {
            let tail = crumbs.split_off(crumbs.len() - (CRUMBS_SHOWN - 2));
            crumbs.truncate(1);
            elided = Some(1);
            crumbs.push(("\u{2026}".into(), VaultPath::root()));
            crumbs.extend(tail);
        }
        let last = crumbs.len() - 1;
        let muted = theme::muted(&self.theme);
        let current = theme::foreground(&self.theme);
        let items = crumbs.into_iter().enumerate().map(|(i, (label, target))| {
            let item = BreadcrumbItem::new(label).whitespace_nowrap();
            if elided == Some(i) {
                item.text_color(muted).disabled(true)
            } else if i == last {
                item.text_color(current)
            } else {
                item.text_color(muted).on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.enter_dir(target.clone(), window, cx)
                    },
                ))
            }
        });
        let (field, order) = self.model.current_sort();
        let chip = sort_chip(FILES_SORT_ID, chip_label(&field, order), &self.theme)
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(FilesEvent::Sort)))
            .test_support();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_3()
            .pb_1()
            .child(
                div()
                    .id(CRUMBS_ID)
                    .test_support()
                    .role(Role::Navigation)
                    .aria_label("Folder")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(Breadcrumb::new().text_color(muted).children(items)),
            )
            .child(chip)
    }

    fn render_filter(&self) -> impl IntoElement {
        query_field(
            &self.filter,
            FILTER_ID,
            "Filter notes, or name a new one",
            &self.theme,
        )
    }

    fn render_row(
        &self,
        pos: usize,
        row: &FileListEntry,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Notes, folders and attachments have a context menu; `..` and
        // `Create:` are not entries.
        let entry = match row {
            FileListEntry::Directory { .. }
            | FileListEntry::Note { .. }
            | FileListEntry::Attachment { .. } => Some(row.path().clone()),
            FileListEntry::Up { .. } | FileListEntry::CreateNote { .. } => None,
        };
        let row = entry_row(row_id(row), row, selected, false, &self.theme);
        let row = with_click_cursor(row, ClickPolicy::Open, true)
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                ClickPolicy::Open.apply(event, this, pos, window, cx)
            }))
            .test_support();
        match entry {
            Some(path) => with_file_menu(row, path, cx.entity().downgrade(), FilesEvent::FileOp),
            None => row.into_any_element(),
        }
    }

    /// What the list area says when it has no row to show.
    fn render_note(&self, text: &'static str) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .text_sm()
            .text_color(theme::muted(&self.theme))
            .child(text)
    }
}

/// One listing row as FILES draws it, shared with the note pickers so a
/// note, a folder or `Create:` looks the same wherever it is listed: the
/// icon lane, the title (a note's filename and journal date under it), and
/// the open-note lane. `with_path` puts the note's full vault path under the
/// title instead of its filename: FILES lists one folder, so the filename
/// says where it is, but a result list (finder, links, SEM) mixes folders. Every row has the same height ([`NOTE_ROW_REMS`]): the
/// lists over these rows are uniform. The caller adds the pointer handling
/// and `test_support`.
pub(super) fn entry_row(
    id: SharedString,
    row: &FileListEntry,
    selected: bool,
    with_path: bool,
    t: &Theme,
) -> Stateful<Div> {
    let fg = theme::foreground(t);
    let muted = theme::muted(t);
    let (primary, secondary, glyph, text): (SharedString, Option<AnyElement>, _, _) = match row {
        FileListEntry::Up { .. } => ("..".into(), None, muted, muted),
        FileListEntry::Directory { name, .. } => {
            let c = theme::directory(t);
            (name.clone().into(), None, c, c)
        }
        FileListEntry::Note {
            path,
            title,
            filename,
            journal_date,
            is_open,
        } => {
            let place = if with_path {
                path.to_string()
            } else {
                filename.clone()
            };
            let glyph = if *is_open { theme::accent(t) } else { fg };
            let line = div()
                .flex()
                .gap_1()
                .min_w_0()
                .text_xs()
                .child(ellipsis(place).text_color(theme::secondary(t)))
                .when_some(journal_date.clone(), |line, date| {
                    line.child(
                        div()
                            .flex_none()
                            .whitespace_nowrap()
                            .text_color(theme::journal_date(t))
                            .child(date),
                    )
                })
                .into_any_element();
            (title.clone().into(), Some(line), glyph, fg)
        }
        FileListEntry::Attachment {
            filename, is_open, ..
        } => {
            let c = theme::secondary(t);
            let glyph = if *is_open { theme::accent(t) } else { c };
            (filename.clone().into(), None, glyph, c)
        }
        FileListEntry::CreateNote { filename, .. } => {
            let c = theme::accent(t);
            (format!("Create: {filename}").into(), None, c, c)
        }
    };
    let is_open = matches!(
        row,
        FileListEntry::Note { is_open: true, .. } | FileListEntry::Attachment { is_open: true, .. }
    );
    let label = match row {
        FileListEntry::Up { .. } => SharedString::from("Parent folder"),
        _ if is_open => format!("{primary}, open").into(),
        _ => primary.clone(),
    };
    let icon = Icon::new(row_icon(row)).size_4().text_color(glyph);
    lane_row(
        id,
        label,
        selected,
        NOTE_ROW_REMS,
        Some(icon.into_any_element()),
        [ellipsis(primary).text_color(text).into_any_element()]
            .into_iter()
            .chain(secondary),
        t,
    )
    // The open-note lane: a dot as well as the accent glyph, so the
    // open note is not told by colour alone.
    .child(
        div()
            .flex()
            .flex_none()
            .justify_center()
            .w_3()
            .when(is_open, |lane| {
                lane.child(
                    Icon::new(IconName::Circle)
                        .size_2()
                        .text_color(theme::accent(t)),
                )
            }),
    )
}

/// How a list's rows take a left click — one policy for every list (C1).
/// Lists name theirs; the rows' handler asks [`Self::click`] what a click
/// does and [`with_click_cursor`] gives them the pointer that says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ClickPolicy {
    /// A click selects the row and activates it, as Enter does (FILES, the
    /// drawer lists, the pickers, the dialogs). The pointing hand says so.
    #[default]
    Open,
    /// A click selects (the selection drives a preview or a reader); a
    /// double-click opens the row's note (the search browser, the Sources
    /// view). The default arrow: a double-click is the manual's to tell.
    Select,
}

/// What one left click on a row does, under its list's [`ClickPolicy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowClick {
    /// Nothing at all: not a left click, or a press `Open` already acted on.
    Ignore,
    /// Select the row.
    Select,
    /// Select the row, then activate it.
    Activate,
}

/// A list whose rows take clicks ([`ClickPolicy::apply`]).
pub(crate) trait ClickRows: Sized + 'static {
    /// Select the row at visible position `pos`.
    fn select_row(&mut self, pos: usize, cx: &mut Context<Self>);
    /// Do what Enter does on the row just selected.
    fn activate_row(&mut self, window: &mut Window, cx: &mut Context<Self>);
}

impl RowClick {
    /// Carry the click out on the row at `pos` of `list`: nothing, select,
    /// or select then activate.
    pub(crate) fn run<L: ClickRows>(
        self,
        list: &mut L,
        pos: usize,
        window: &mut Window,
        cx: &mut Context<L>,
    ) {
        if self == Self::Ignore {
            return;
        }
        list.select_row(pos, cx);
        if self == Self::Activate {
            list.activate_row(window, cx);
        }
    }
}

impl ClickPolicy {
    /// A click on the row at `pos` of `list`, answered under this policy:
    /// [`Self::click`], then [`RowClick::run`].
    pub(crate) fn apply<L: ClickRows>(
        self,
        event: &ClickEvent,
        list: &mut L,
        pos: usize,
        window: &mut Window,
        cx: &mut Context<L>,
    ) {
        self.click(event).run(list, pos, window, cx);
    }

    /// What `event`, a click on a row, does. A right click is the row's
    /// menu's, never this. Under `Open` only a double-click's first press
    /// counts: the first one activated and may have changed what is under
    /// the pointer (a folder entered, a dialog closed, another view shown),
    /// so the second must not act on whatever it lands on. Under `Select`
    /// the second press opens; a third is a click again.
    pub(crate) fn click(self, event: &ClickEvent) -> RowClick {
        if !event.standard_click() {
            return RowClick::Ignore;
        }
        match (self, event.click_count()) {
            (Self::Open, 1) => RowClick::Activate,
            (Self::Open, _) => RowClick::Ignore,
            (Self::Select, 2) => RowClick::Activate,
            (Self::Select, _) => RowClick::Select,
        }
    }

    /// The pointer over a row of this policy: the pointing hand where a
    /// click opens it, the default arrow (`None`) where a click only
    /// selects or the row cannot be activated now (`enabled` false: a
    /// missing pinned note, a picker waiting on its `Create:`).
    pub(crate) fn cursor(self, enabled: bool) -> Option<CursorStyle> {
        match self {
            Self::Open if enabled => Some(CursorStyle::PointingHand),
            Self::Open | Self::Select => None,
        }
    }
}

/// `row` with the pointer its list's `policy` gives it ([`ClickPolicy::cursor`]).
/// Every list row that takes clicks goes through here, so none is missed.
pub(super) fn with_click_cursor(
    row: Stateful<Div>,
    policy: ClickPolicy,
    enabled: bool,
) -> Stateful<Div> {
    row.when_some(policy.cursor(enabled), |row, cursor| row.cursor(cursor))
}

/// The frame every list row shares (FILES, the pickers, the drawer lists):
/// one option of a list box, `height_rems` tall and the list's full width,
/// its selection a fill of its own surface, a quieter fill under the
/// pointer. The caller lays out its content inside.
pub(super) fn row_frame(
    id: SharedString,
    label: SharedString,
    selected: bool,
    height_rems: f32,
    t: &Theme,
) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    div()
        .id(id)
        .role(Role::ListBoxOption)
        .aria_label(label)
        .aria_selected(selected)
        .flex()
        .items_center()
        .w_full()
        .h(rems(height_rems))
        .px_2()
        .rounded_md()
        .when(selected, |row| row.bg(theme::selected_background(t)))
        .when(!selected, |row| row.hover(move |style| style.bg(hover)))
}

/// A list row of [`row_frame`] with a leading lane and lines beside it:
/// the lane (an icon, a number, or nothing) centred in a fixed width so
/// every row's text starts on one edge, then `lines` stacked in small text,
/// each taking the rest of the width. FILES, the pickers, LINKS, FIND,
/// TAGS, the saved searches and the pinned notes all draw their rows with
/// it; a caller may add a trailing lane (FILES' open-note dot, TAGS'
/// count). OUTLINE has no lane (its headings indent from the edge), so it
/// uses [`row_frame`] itself.
pub(super) fn lane_row(
    id: SharedString,
    label: SharedString,
    selected: bool,
    height_rems: f32,
    lane: Option<AnyElement>,
    lines: impl IntoIterator<Item = AnyElement>,
    t: &Theme,
) -> Stateful<Div> {
    row_frame(id, label, selected, height_rems, t)
        .gap_2()
        .child(
            div()
                .flex()
                .flex_none()
                .justify_center()
                .w_5()
                .children(lane),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .text_sm()
                .children(lines),
        )
}

/// The title a note row shows: its filename when it has no title (the
/// TUI's rule).
pub(super) fn display_title<'a>(title: &'a str, filename: &'a str) -> &'a str {
    if title.trim().is_empty() {
        filename
    } else {
        title
    }
}

/// `row` with the FILES entry menu for `path`: Rename…, Move…, Delete….
/// A pick emits `event(kind, path)` from `owner` once the menu has given
/// the keyboard back, so the dialog it opens returns it there too.
pub(super) fn with_file_menu<R, T, E>(
    row: R,
    path: VaultPath,
    owner: WeakEntity<T>,
    event: fn(FileOpKind, VaultPath) -> E,
) -> AnyElement
where
    R: InteractiveElement + ParentElement + Styled + IntoElement + 'static,
    T: EventEmitter<E>,
    E: 'static,
{
    row.context_menu(move |menu, _, _| {
        let item = |kind: FileOpKind, label: &'static str| {
            let owner = owner.clone();
            let path = path.clone();
            PopupMenuItem::new(label).on_click(move |_, window, cx| {
                let event = event(kind, path.clone());
                let owner = owner.clone();
                window.defer(cx, move |_, cx| {
                    owner.update(cx, |_, cx| cx.emit(event)).ok();
                });
            })
        };
        menu.item(item(FileOpKind::Rename, "Rename…"))
            .item(item(FileOpKind::Move, "Move…"))
            .separator()
            .item(item(FileOpKind::Delete, "Delete…"))
    })
    .into_any_element()
}

/// One line of text that ends in an ellipsis when it does not fit.
pub(super) fn ellipsis(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text.into())
}

impl ClickRows for FilesPane {
    fn select_row(&mut self, pos: usize, cx: &mut Context<Self>) {
        if let Some(core) = self.model.core_mut() {
            core.select(pos);
        }
        cx.notify();
    }

    fn activate_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(window, cx);
    }
}

impl Focusable for FilesPane {
    /// The filter: typing in FILES filters.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.filter.read(cx).focus_handle(cx)
    }
}

impl Render for FilesPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (count, selected, loading) = self.model.core().map_or((0, None, false), |core| {
            (core.visible_len(), core.selected(), core.is_loading())
        });
        let list = if count > 0 {
            uniform_list(
                LIST_ID,
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    let rows: Vec<(usize, FileListEntry)> = range
                        .filter_map(|pos| {
                            this.model
                                .core()
                                .and_then(|core| core.visible_row(pos))
                                .map(|row| (pos, row.clone()))
                        })
                        .collect();
                    rows.iter()
                        .map(|(pos, row)| this.render_row(*pos, row, selected == Some(*pos), cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full()
            .into_any_element()
        } else if self.model.is_empty() || loading {
            self.render_note("Loading…").into_any_element()
        } else {
            self.render_note("This folder is empty").into_any_element()
        };
        div()
            .id(FILES_ID)
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            // The filter's keys that mean something else here, taken before
            // the filter sees them.
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.select_prev(cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.select_next(cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                this.activate(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                if this.query(cx).is_empty() {
                    cx.emit(FilesEvent::Dismiss);
                } else {
                    this.clear_filter(window, cx);
                }
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveLeft, window, cx| {
                if this.query(cx).is_empty() && this.go_up(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(self.render_breadcrumb(cx))
            .child(self.render_filter())
            .child(
                div()
                    .id("files-rows")
                    .role(Role::ListBox)
                    .aria_label("Files")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(list),
            )
    }
}

// ---- The workspace's side ----------------------------------------------------

impl Workspace {
    /// Make the FILES view over the vault the window opened (the drawer
    /// shows it; nothing is listed until it is first needed).
    pub(super) fn start_files(
        &mut self,
        vault: Arc<NoteVault>,
        name: String,
        settings: &AppSettings,
        tx: AppTx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = self.theme.clone();
        let name = SharedString::from(name);
        let pane = cx.new(|cx| FilesPane::new(vault, name, settings, tx, theme, window, cx));
        self._files_events = Some(cx.subscribe_in(&pane, window, Self::on_files_event));
        self.files = Some(pane);
    }

    fn on_files_event(
        &mut self,
        _: &Entity<FilesPane>,
        event: &FilesEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FilesEvent::Open(path) => {
                self.focus_panel(Panel::Editor, window, cx);
                self.open_note(path.clone(), cx);
            }
            FilesEvent::Creating(_) => self.focus_panel(Panel::Editor, window, cx),
            FilesEvent::OpenAttachment(path) => {
                self.focus_panel(Panel::Editor, window, cx);
                self.open_attachment(path, cx);
            }
            FilesEvent::Dismiss => self.focus_panel(Panel::Editor, window, cx),
            FilesEvent::FileOp(kind, path) => {
                self.open_file_dialog(*kind, path.clone(), window, cx)
            }
            FilesEvent::Sort => self.open_sort(SortTarget::Sidebar, window, cx),
        }
    }

    /// Apply what the listing's load delivered.
    pub(super) fn poll_files(&mut self, cx: &mut Context<Self>) {
        if let Some(files) = self.files.as_ref() {
            files.update(cx, |pane, cx| pane.poll(cx));
        }
    }

    pub(super) fn files_file_op(&mut self, op: &FileOp, cx: &mut Context<Self>) {
        if let Some(files) = self.files.as_ref() {
            files.update(cx, |pane, cx| pane.file_op(op, cx));
        }
    }

    /// A folder was asked to be opened (`OpenPath` of a directory): FILES
    /// shows it, as the TUI's sidebar does.
    pub(super) fn files_show_dir(&mut self, dir: VaultPath, cx: &mut Context<Self>) {
        if let Some(files) = self.files.as_ref() {
            files.update(cx, |pane, cx| pane.navigate(dir, cx));
        }
    }

    /// The editor opened `note`: the marker follows, and a listing that was
    /// never loaded starts in the note's folder.
    pub(super) fn files_note_opened(&mut self, note: &VaultPath, cx: &mut Context<Self>) {
        if let Some(files) = self.files.as_ref() {
            files.update(cx, |pane, cx| {
                pane.set_open_note(Some(note.clone()), cx);
                if pane.is_empty() {
                    pane.navigate(note.get_parent_path().0, cx);
                }
            });
        }
    }

    /// `saved` was written: retitle its row.
    pub(super) fn files_note_saved(&mut self, saved: &Saved, cx: &mut Context<Self>) {
        if let Some(files) = self.files.as_ref() {
            files.update(cx, |pane, cx| pane.note_saved(saved, cx));
        }
    }

    /// Whether the first note of the session (the restore, or the one named
    /// on the command line) may still be on its way.
    fn opening_first(&self) -> bool {
        self.first_open_pending
            || self
                .notes
                .as_ref()
                .is_some_and(|h| h.is_switching() || h.is_open_wanted())
    }

    /// List something once nothing will choose the folder any more: the
    /// open note's, or the root. A no-op when FILES already lists one, or
    /// while the first open may still land (it picks the note's folder).
    pub(super) fn ensure_files_listing(&mut self, cx: &mut Context<Self>) {
        if self.opening_first() {
            return;
        }
        let dir = self.open_note_dir();
        if let Some(files) = self.files.as_ref()
            && files.read(cx).is_empty()
        {
            files.update(cx, |pane, cx| pane.navigate(dir, cx));
        }
    }

    /// The folder of the note in the editor, or the root.
    pub(super) fn open_note_dir(&self) -> VaultPath {
        self.notes
            .as_ref()
            .and_then(|h| h.note.as_ref())
            .map_or_else(VaultPath::root, |note| note.get_parent_path().0)
    }

    /// `OpenFileBrowser`: show FILES in the drawer where it lives (its home
    /// side when it is in no slot), on the open note's folder when it shows
    /// another one (a filter and a selection in the right folder stay), and
    /// give it the keyboard.
    pub(super) fn open_file_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (side, index) = self.layout.open_view(DrawerView::Files);
        self.place_rail_cursor(Some(DrawerView::Files));
        self.layout.focus_region(Region::Slot { side, index });
        if !self.opening_first() {
            let dir = self.open_note_dir();
            if let Some(files) = self.files.as_ref() {
                files.update(cx, |pane, cx| {
                    if pane.is_empty() || !dir.is_like(pane.current_dir()) {
                        pane.enter_dir(dir, window, cx);
                    }
                });
            }
        }
        self.apply_focus(window, cx);
        self.save_layout(cx);
        cx.notify();
    }
}
