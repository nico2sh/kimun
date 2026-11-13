//! The **note menu**: one pointer-anchored menu that holds the actions on
//! a group's note (`Properties…`, `Rename…`, `Move…`, `Delete…`) and the
//! layout actions (split, new tab, close group, close tab).
//!
//! The items are a pure function of a small [`MenuContext`] ([`menu_items`]):
//! an item that cannot act is hidden, never grayed out. The view only emits
//! [`NoteMenuEvent`]s; the workspace that draws it runs the pick. The menu
//! is not modal: a key it does not use goes on to its parent.
//!
//! **In the window.** The workspace holds at most one open menu
//! ([`OpenNoteMenu`]) and draws it as a deferred, anchored overlay over a
//! window-wide layer that dismisses it on a press. It opens from the
//! editor's right-click (at the pointer, for the clicked group, which the
//! press already made active) and from the `FileOperations` chord (under
//! the active group's top-right corner). It is refused while a modal is
//! open, and gives way to one that opens. A pick closes the menu, makes the
//! menu's group active and runs what the matching chord runs, once the
//! keyboard is back where it was. Escape, a press outside, and the menu's
//! note renamed, moved or deleted, its group closed, or a drag starting
//! close it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Pixels, Point, Render, Role, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, anchored, canvas, deferred, div, point, px, rems,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::file_ops::relocated;
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout_tree::{Axis as TreeAxis, GroupId};

use super::Workspace;
use super::file_dialogs::FileOpKind;
use super::modal;
use crate::input;
use crate::keymap;
use crate::theme;

/// The menu's element id, for tests.
pub(crate) const NOTE_MENU_ID: &str = "note-menu";

/// The window-wide layer under the open menu: a press on it dismisses.
pub(crate) const NOTE_MENU_LAYER_ID: &str = "note-menu-layer";

/// The menu's width, in rems.
const MENU_REMS: f32 = 18.;

/// How far the menu keeps from the window's edges, in px.
const EDGE_MARGIN: f32 = 8.;

/// One row of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuItem {
    Note(FileOpKind),
    SplitRight,
    SplitDown,
    NewTab,
    CloseGroup,
    CloseTab,
}

impl MenuItem {
    fn label(self) -> &'static str {
        match self {
            MenuItem::Note(FileOpKind::Properties) => "Properties…",
            MenuItem::Note(FileOpKind::Rename) => "Rename…",
            MenuItem::Note(FileOpKind::Move) => "Move…",
            MenuItem::Note(FileOpKind::Delete) => "Delete…",
            MenuItem::SplitRight => "Split right",
            MenuItem::SplitDown => "Split down",
            MenuItem::NewTab => "New tab",
            MenuItem::CloseGroup => "Close group",
            MenuItem::CloseTab => "Close tab",
        }
    }

    /// The key that picks a note item (the TUI menu's R, M, D; P for
    /// properties). Layout items have none.
    fn letter(self) -> Option<char> {
        match self {
            MenuItem::Note(FileOpKind::Properties) => Some('p'),
            MenuItem::Note(FileOpKind::Rename) => Some('r'),
            MenuItem::Note(FileOpKind::Move) => Some('m'),
            MenuItem::Note(FileOpKind::Delete) => Some('d'),
            _ => None,
        }
    }

    /// The chord a layout item shows as its hint.
    fn action(self) -> Option<ActionShortcuts> {
        match self {
            MenuItem::SplitRight => Some(ActionShortcuts::SplitSlotRight),
            MenuItem::SplitDown => Some(ActionShortcuts::SplitSlotDown),
            MenuItem::NewTab => Some(ActionShortcuts::NewTab),
            MenuItem::CloseGroup => Some(ActionShortcuts::CloseSlot),
            MenuItem::CloseTab => Some(ActionShortcuts::CloseTab),
            MenuItem::Note(_) => None,
        }
    }

    fn is_note(self) -> bool {
        matches!(self, MenuItem::Note(_))
    }
}

/// What decides which items show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MenuContext {
    /// The group holds a note.
    pub has_note: bool,
    /// How many editor groups the group's tab has.
    pub groups_in_tab: usize,
    /// How many tabs there are.
    pub tabs: usize,
}

/// The items to show, in order. The note items come first; the view draws
/// a divider at the boundary between them and the layout items.
pub(crate) fn menu_items(ctx: &MenuContext) -> Vec<MenuItem> {
    let mut items = Vec::new();
    if ctx.has_note {
        items.extend([
            MenuItem::Note(FileOpKind::Properties),
            MenuItem::Note(FileOpKind::Rename),
            MenuItem::Note(FileOpKind::Move),
            MenuItem::Note(FileOpKind::Delete),
        ]);
    }
    items.extend([MenuItem::SplitRight, MenuItem::SplitDown, MenuItem::NewTab]);
    if ctx.groups_in_tab >= 2 {
        items.push(MenuItem::CloseGroup);
    }
    if ctx.tabs >= 2 {
        items.push(MenuItem::CloseTab);
    }
    items
}

/// Where the divider goes: before the first layout item, when note items
/// come before it.
pub(crate) fn divider_at(items: &[MenuItem]) -> Option<usize> {
    items.iter().position(|i| !i.is_note()).filter(|&ix| ix > 0)
}

/// The element id of a layout item's chord hint (`note-menu-new-tab-hint`).
pub(crate) fn menu_item_hint_id(item: MenuItem) -> SharedString {
    format!("{}-hint", menu_item_element_id(item)).into()
}

/// A menu item's element id (`note-menu-rename`).
pub(crate) fn menu_item_element_id(item: MenuItem) -> &'static str {
    match item {
        MenuItem::Note(FileOpKind::Properties) => "note-menu-properties",
        MenuItem::Note(FileOpKind::Rename) => "note-menu-rename",
        MenuItem::Note(FileOpKind::Move) => "note-menu-move",
        MenuItem::Note(FileOpKind::Delete) => "note-menu-delete",
        MenuItem::SplitRight => "note-menu-split-right",
        MenuItem::SplitDown => "note-menu-split-down",
        MenuItem::NewTab => "note-menu-new-tab",
        MenuItem::CloseGroup => "note-menu-close-group",
        MenuItem::CloseTab => "note-menu-close-tab",
    }
}

/// What the menu tells its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NoteMenuEvent {
    Picked(MenuItem),
    Dismissed,
}

/// The note menu view.
pub(crate) struct NoteMenu {
    group: GroupId,
    target: Option<VaultPath>,
    items: Vec<MenuItem>,
    focus: FocusHandle,
    selected: usize,
}

impl EventEmitter<NoteMenuEvent> for NoteMenu {}

impl NoteMenu {
    pub(crate) fn new(
        group: GroupId,
        target: Option<VaultPath>,
        ctx: MenuContext,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            group,
            target,
            items: menu_items(&ctx),
            focus: cx.focus_handle(),
            selected: 0,
        }
    }

    /// The group the menu acts on.
    pub(crate) fn group(&self) -> GroupId {
        self.group
    }

    /// The note the group holds, if any.
    pub(crate) fn target(&self) -> Option<&VaultPath> {
        self.target.as_ref()
    }

    fn pick(&mut self, item: MenuItem, cx: &mut Context<Self>) {
        cx.emit(NoteMenuEvent::Picked(item));
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(KeyInput { key, mods }) = input::key_down_input(event) else {
            return;
        };
        if mods != Mods::NONE && mods != Mods::SHIFT {
            return;
        }
        match key {
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Down => self.selected = (self.selected + 1).min(self.items.len() - 1),
            Key::Enter => {
                let item = self.items[self.selected];
                cx.stop_propagation();
                self.pick(item, cx);
                return;
            }
            Key::Esc => {
                cx.stop_propagation();
                cx.emit(NoteMenuEvent::Dismissed);
                return;
            }
            Key::Char(c) => {
                let c = c.to_ascii_lowercase();
                // Only a letter of a shown item is used; the menu is not
                // modal, so every other key goes on.
                if let Some(item) = self.items.iter().find(|i| i.letter() == Some(c)) {
                    let item = *item;
                    cx.stop_propagation();
                    self.pick(item, cx);
                }
                return;
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Focusable for NoteMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for NoteMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let first_layout = divider_at(&self.items);
        let rows = self
            .items
            .iter()
            .enumerate()
            .map(|(ix, &item)| {
                let selected = ix == self.selected;
                let hover = theme::hover_background(t);
                let text = if item == MenuItem::Note(FileOpKind::Delete) {
                    theme::error(t)
                } else {
                    theme::foreground(t)
                };
                let hint = match (item.letter(), item.action()) {
                    (Some(letter), _) => Some(
                        Kbd::new(input::keystroke(KeyInput::plain(Key::Char(
                            letter.to_ascii_uppercase(),
                        ))))
                        .into_any_element(),
                    ),
                    (None, Some(action)) => keymap::chord_label(cx, &action).map(|chord| {
                        div()
                            .id(menu_item_hint_id(item))
                            .test_support()
                            .role(Role::Label)
                            .aria_label(chord.clone())
                            .text_xs()
                            .text_color(theme::muted(t))
                            .child(chord)
                            .into_any_element()
                    }),
                    (None, None) => None,
                };
                div()
                    .id(menu_item_element_id(item))
                    .test_support()
                    .role(Role::MenuItem)
                    .aria_label(item.label())
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(selected, |row| row.bg(theme::selected_background(t)))
                    .when(!selected, |row| row.hover(move |style| style.bg(hover)))
                    .when(first_layout == Some(ix), |row| {
                        row.mt_1().border_t_1().border_color(theme::divider(t))
                    })
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(text)
                            .child(item.label()),
                    )
                    .children(hint)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.pick(item, cx)))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        div()
            .id(NOTE_MENU_ID)
            .test_support()
            .role(Role::Menu)
            .aria_label("Note menu")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .gap_0p5()
            .children(rows)
    }
}

/// The open note menu: its view, where it is anchored and by which corner,
/// and what had the keyboard before it.
pub(crate) struct OpenNoteMenu {
    pub(crate) view: Entity<NoteMenu>,
    pub(crate) at: Point<Pixels>,
    anchor: Anchor,
    /// Where the keyboard goes back when the menu is dismissed.
    pub(crate) restore: Option<FocusHandle>,
    _events: Subscription,
}

/// Where each drawn group was last laid out, in window coordinates: the
/// `FileOperations` chord anchors the menu at the active group's top-right
/// corner. Written while the groups are painted.
pub(crate) type GroupBounds = Rc<RefCell<HashMap<GroupId, Bounds<Pixels>>>>;

/// An invisible element over its parent that records the parent's bounds
/// as `group`'s in `bounds`.
pub(super) fn group_bounds_probe(bounds: &GroupBounds, group: GroupId) -> impl IntoElement {
    let bounds = bounds.clone();
    canvas(
        move |rect, _, _| {
            bounds.borrow_mut().insert(group, rect);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl Workspace {
    /// Open the note menu for `group` with its top-left corner at `at`
    /// (window coordinates; the editor's right-click). See
    /// [`Self::open_note_menu_anchored`].
    pub(crate) fn open_note_menu(
        &mut self,
        group: GroupId,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_note_menu_anchored(group, at, Anchor::TopLeft, window, cx);
    }

    /// Open the note menu for `group`, its `anchor` corner at `at`, with
    /// the keyboard. Refused while a modal is open, and for a group with no
    /// note session (no vault). A menu already open gives way to it.
    fn open_note_menu_anchored(
        &mut self,
        group: GroupId,
        at: Point<Pixels>,
        anchor: Anchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) {
            return;
        }
        let Some(host) = self.state(group).and_then(|s| s.notes.as_ref()) else {
            return;
        };
        let target = host.note.clone();
        let region = self.layout.region();
        let Some(tab) = region.group_tab(group) else {
            return;
        };
        let ctx = MenuContext {
            has_note: target.is_some(),
            groups_in_tab: region.groups(tab).len(),
            tabs: region.tabs().len(),
        };
        self.cancel_leader(cx);
        self.end_drag(window, cx);
        let restore = match self.note_menu.take() {
            Some(open) => open.restore,
            None => window.focused(cx),
        };
        let view = cx.new(|cx| NoteMenu::new(group, target, ctx, cx));
        let events = cx.subscribe_in(&view, window, Self::on_note_menu_event);
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.note_menu = Some(OpenNoteMenu {
            view,
            at,
            anchor,
            restore,
            _events: events,
        });
        cx.notify();
    }

    /// The `FileOperations` chord: the note menu for the active group,
    /// under its top-right corner (an empty group gets the layout actions).
    /// Again while the menu is open, it closes it.
    pub(crate) fn file_ops_chord(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.note_menu.is_some() {
            self.close_note_menu(window, cx);
            return;
        }
        let group = self.layout.active_group();
        let corner = self
            .group_bounds
            .borrow()
            .get(&group)
            .map(|b| b.top_right());
        // Before the group's first paint: the window's top-right corner.
        let at = corner.unwrap_or_else(|| point(window.viewport_size().width, px(0.)));
        self.open_note_menu_anchored(group, at, Anchor::TopRight, window, cx);
    }

    /// Dismiss the open menu: the keyboard goes back where it was.
    pub(crate) fn close_note_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.note_menu.take() else {
            return;
        };
        match open.restore {
            Some(handle) => window.focus(&handle, cx),
            None => self.apply_focus(window, cx),
        }
        cx.notify();
    }

    /// Close the open menu where there is no window at hand (its note or
    /// its group went, a drag started): the keyboard goes to the focused
    /// leaf once the update is done.
    pub(super) fn forget_note_menu(&mut self, cx: &mut Context<Self>) {
        if self.note_menu.take().is_none() {
            return;
        }
        self.with_window(cx, |workspace, window, cx| {
            if workspace.note_menu.is_none() {
                workspace.apply_focus(window, cx);
            }
        });
        cx.notify();
    }

    /// Close the menu when its group is `group` (the group is going).
    pub(super) fn forget_note_menu_of(&mut self, group: GroupId, cx: &mut Context<Self>) {
        if self.note_menu_group(cx) == Some(group) {
            self.forget_note_menu(cx);
        }
    }

    /// Close the menu when its note is at or below `path` (renamed, moved
    /// or deleted): it would act on a stale path.
    pub(super) fn forget_note_menu_under(&mut self, path: &VaultPath, cx: &mut Context<Self>) {
        let stale = self.note_menu.as_ref().is_some_and(|open| {
            open.view
                .read(cx)
                .target()
                .is_some_and(|note| relocated(note, path, path).is_some())
        });
        if stale {
            self.forget_note_menu(cx);
        }
    }

    /// The group the open menu acts on.
    fn note_menu_group(&self, cx: &App) -> Option<GroupId> {
        self.note_menu
            .as_ref()
            .map(|open| open.view.read(cx).group())
    }

    /// While the menu is open it keeps the keyboard: what the layout would
    /// focus becomes where a dismiss returns it ([`Self::apply_focus`]).
    /// False when no menu is open.
    pub(super) fn keep_focus_on_note_menu(
        &mut self,
        handle: FocusHandle,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(open) = self.note_menu.as_mut() else {
            return false;
        };
        open.restore = Some(handle);
        let menu = open.view.read(cx).focus_handle(cx);
        window.focus(&menu, cx);
        true
    }

    fn on_note_menu_event(
        &mut self,
        menu: &Entity<NoteMenu>,
        event: &NoteMenuEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .note_menu
            .as_ref()
            .is_none_or(|open| &open.view != menu)
        {
            return;
        }
        match event {
            NoteMenuEvent::Dismissed => self.close_note_menu(window, cx),
            NoteMenuEvent::Picked(item) => {
                let item = *item;
                let (group, target) = {
                    let menu = menu.read(cx);
                    (menu.group(), menu.target().cloned())
                };
                self.close_note_menu(window, cx);
                // Once the keyboard is back, so a dialog the pick opens
                // returns it there when it closes.
                let workspace = cx.entity().downgrade();
                window.defer(cx, move |window, cx| {
                    workspace
                        .update(cx, |workspace, cx| {
                            workspace.run_note_menu_pick(group, target, item, window, cx)
                        })
                        .ok();
                });
            }
        }
    }

    /// Run `item`, picked in `group`'s menu: the group becomes active with
    /// the keyboard, then the item does what its chord does.
    fn run_note_menu_pick(
        &mut self,
        group: GroupId,
        target: Option<VaultPath>,
        item: MenuItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state(group).is_none() {
            return;
        }
        self.focus_group(group, cx);
        self.apply_focus(window, cx);
        match item {
            MenuItem::Note(kind) => {
                if let Some(target) = target {
                    self.open_file_dialog(kind, target, window, cx);
                }
            }
            MenuItem::SplitRight => {
                self.split_focused(TreeAxis::Horizontal, window, cx);
            }
            MenuItem::SplitDown => {
                self.split_focused(TreeAxis::Vertical, window, cx);
            }
            MenuItem::NewTab => self.new_tab(cx),
            MenuItem::CloseGroup => self.close_editor_group(group, cx),
            MenuItem::CloseTab => self.close_shown_tab(cx),
        }
        cx.notify();
    }

    /// The open menu over a window-wide layer that dismisses it on a
    /// press, kept inside the window.
    pub(super) fn render_note_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let open = self.note_menu.as_ref()?;
        let theme = theme::current(cx);
        let t = &*theme;
        let size = window.viewport_size();
        let workspace = cx.entity().downgrade();
        let dismiss = move || {
            let workspace = workspace.clone();
            move |_: &gpui_kit::MouseDownEvent, window: &mut Window, cx: &mut App| {
                workspace
                    .update(cx, |this, cx| this.close_note_menu(window, cx))
                    .ok();
            }
        };
        let menu = div()
            .occlude()
            .w(rems(MENU_REMS))
            .p_1()
            .bg(theme::panel_background(t))
            .border_1()
            .border_color(theme::divider(t))
            .rounded_md()
            .shadow_md()
            .child(open.view.clone());
        Some(
            deferred(
                anchored().position(point(px(0.), px(0.))).child(
                    div()
                        .id(NOTE_MENU_LAYER_ID)
                        .test_support()
                        .occlude()
                        .w(size.width)
                        .h(size.height)
                        .on_mouse_down(MouseButton::Left, dismiss())
                        .on_mouse_down(MouseButton::Right, dismiss())
                        .on_mouse_down(MouseButton::Middle, dismiss())
                        .on_scroll(|_, _, cx| cx.stop_propagation())
                        .child(
                            anchored()
                                .position(open.at)
                                .anchor(open.anchor)
                                .snap_to_window_with_margin(px(EDGE_MARGIN))
                                .child(menu),
                        ),
                ),
            )
            .with_priority(gpui_kit::base::POPUP_PRIORITY)
            .into_any_element(),
        )
    }
}
