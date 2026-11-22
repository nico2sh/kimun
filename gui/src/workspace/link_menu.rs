//! The **Link menu**: the pointer-anchored menu a right-click on a follow
//! target (a link or a label) opens, in an editor group or in a facet view
//! piece. It offers the ways to open the target and to copy it.
//!
//! The items are a pure function of a small [`LinkMenuContext`]
//! ([`link_menu_items`]): what the target is ([`TargetClass`], or a label)
//! and whether the menu opened in a facet view, which has no plain `Open`.
//! An item that cannot act is hidden, never grayed out. The view only emits
//! [`LinkMenuEvent`]s; the workspace that draws it runs the pick
//! ([`Workspace::run_link_menu_pick`]). The menu is not modal: a key it does
//! not use goes on to its parent.
//!
//! **In the window.** The workspace holds at most one open Link menu
//! ([`OpenLinkMenu`]) and draws it as the note menu is drawn
//! ([`super::note_menu`]): a deferred overlay anchored at the pointer, over
//! a window-wide layer that dismisses it on a press. It takes the keyboard.
//! It is refused while a modal is open and gives way to one that opens. At
//! most one of the Link menu and the note menu is open: opening one closes
//! the other. Escape, a press outside, a chord, the menu's note (`from`)
//! renamed, moved or deleted, its group closed or showing another note, or
//! a drag starting close it.
//!
//! **Hand-built on purpose.** The menu is not the kit's `PopupMenu`: it picks
//! by a single letter, hides an item that cannot act, hands a key it does not
//! use to its parent, and anchors at the pointer or a group's corner. The
//! surface comes from [`super::menu_overlay`], which draws the kit's popover
//! look, so it matches the row menus.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels,
    Point, Render, Role, StatefulInteractiveElement, Styled, Subscription, Window, div,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::editor::claim::FollowTarget;
use kimun_notes::logic::follow::{TargetClass, classify};
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout_tree::{Axis as TreeAxis, GroupId};

use super::Workspace;
use super::modal;
use super::note::OpenAt;
use super::note_menu::MENU_NOTE_CHANGED;
use super::tab_strip::TAB_CLOSING;
use crate::input;
use crate::theme;
use kimun_notes::logic::file_ops::path_at_or_below;

/// The menu's element id, for tests.
pub(crate) const LINK_MENU_ID: &str = "link-menu";

/// The window-wide layer under the open menu: a press on it dismisses.
pub(crate) const LINK_MENU_LAYER_ID: &str = "link-menu-layer";

/// The flash after `Copy link`.
pub(crate) const LINK_COPIED: &str = "Link copied";

/// The flash after `Copy label`.
pub(crate) const LABEL_COPIED: &str = "Label copied";

/// The menu's width, in rems.
const MENU_REMS: f32 = 16.;

/// One row of the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkItem {
    Open,
    OpenNewTab,
    OpenSplitRight,
    OpenSplitDown,
    OpenInBrowser,
    OpenInDefaultApp,
    SearchLabel,
    CopyLink,
    CopyLabel,
}

impl LinkItem {
    pub(crate) fn label(self) -> &'static str {
        match self {
            LinkItem::Open => "Open",
            LinkItem::OpenNewTab => "Open in new tab",
            LinkItem::OpenSplitRight => "Open in split right",
            LinkItem::OpenSplitDown => "Open in split down",
            LinkItem::OpenInBrowser => "Open in browser",
            LinkItem::OpenInDefaultApp => "Open in default app",
            LinkItem::SearchLabel => "Search for label",
            LinkItem::CopyLink => "Copy link",
            LinkItem::CopyLabel => "Copy label",
        }
    }

    fn is_copy(self) -> bool {
        matches!(self, LinkItem::CopyLink | LinkItem::CopyLabel)
    }
}

/// A menu item's element id (`link-menu-open-new-tab`).
pub(crate) fn link_item_element_id(item: LinkItem) -> &'static str {
    match item {
        LinkItem::Open => "link-menu-open",
        LinkItem::OpenNewTab => "link-menu-open-new-tab",
        LinkItem::OpenSplitRight => "link-menu-open-split-right",
        LinkItem::OpenSplitDown => "link-menu-open-split-down",
        LinkItem::OpenInBrowser => "link-menu-open-in-browser",
        LinkItem::OpenInDefaultApp => "link-menu-open-in-default-app",
        LinkItem::SearchLabel => "link-menu-search-label",
        LinkItem::CopyLink => "link-menu-copy-link",
        LinkItem::CopyLabel => "link-menu-copy-label",
    }
}

/// What decides which items show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinkMenuContext {
    /// What the link points at; `None` for a label.
    pub class: Option<TargetClass>,
    /// The menu opened in a facet view piece: a follow there opens in a new
    /// tab (ADR-0061), so there is no plain `Open`.
    pub facet: bool,
}

impl LinkMenuContext {
    /// The context of `target`, written in the note `from`, right-clicked
    /// in a facet view or not. A link is classified by the same routing
    /// step a follow uses ([`classify`]), so the items match what a follow
    /// does.
    pub(crate) fn of(target: &FollowTarget, from: &VaultPath, facet: bool) -> Self {
        let class = match target {
            FollowTarget::Link { target, kind } => Some(classify(target, *kind, from)),
            FollowTarget::Label(_) => None,
        };
        Self { class, facet }
    }
}

/// The items to show, in order: the ways to open, then the copy item.
pub(crate) fn link_menu_items(ctx: &LinkMenuContext) -> Vec<LinkItem> {
    let mut items = match ctx.class {
        Some(TargetClass::Note) => vec![
            LinkItem::Open,
            LinkItem::OpenNewTab,
            LinkItem::OpenSplitRight,
            LinkItem::OpenSplitDown,
            LinkItem::CopyLink,
        ],
        Some(TargetClass::Attachment) => vec![
            LinkItem::Open,
            LinkItem::OpenNewTab,
            LinkItem::OpenInDefaultApp,
            LinkItem::CopyLink,
        ],
        Some(TargetClass::Web) => vec![LinkItem::OpenInBrowser, LinkItem::CopyLink],
        // A follow would fail: no open item can act.
        Some(TargetClass::Invalid) => vec![LinkItem::CopyLink],
        None => vec![LinkItem::SearchLabel, LinkItem::CopyLabel],
    };
    if ctx.facet {
        items.retain(|item| *item != LinkItem::Open);
    }
    items
}

/// What a pick acts on: the menu's group, the note that holds the link (a
/// relative link resolves from it), the target, and whether the menu opened
/// in a facet view. [`Workspace::run_link_menu_pick`] takes it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LinkMenuPick {
    pub group: GroupId,
    pub from: VaultPath,
    pub target: FollowTarget,
    pub facet: bool,
}

/// What the menu tells its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkMenuEvent {
    Picked(LinkItem),
    Dismissed,
}

/// The Link menu view.
pub(crate) struct LinkMenu {
    items: Vec<LinkItem>,
    focus: FocusHandle,
    selected: usize,
}

impl EventEmitter<LinkMenuEvent> for LinkMenu {}

impl LinkMenu {
    pub(crate) fn new(ctx: LinkMenuContext, cx: &mut Context<Self>) -> Self {
        Self {
            items: link_menu_items(&ctx),
            focus: cx.focus_handle(),
            selected: 0,
        }
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
                cx.stop_propagation();
                cx.emit(LinkMenuEvent::Picked(self.items[self.selected]));
                return;
            }
            Key::Esc => {
                cx.stop_propagation();
                cx.emit(LinkMenuEvent::Dismissed);
                return;
            }
            // The menu is not modal: every other key goes on.
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Focusable for LinkMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for LinkMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        // A divider above the copy item, when open items come before it.
        let first_copy = self
            .items
            .iter()
            .position(|item| item.is_copy())
            .filter(|&ix| ix > 0);
        let rows = self
            .items
            .iter()
            .enumerate()
            .map(|(ix, &item)| {
                let selected = ix == self.selected;
                let hover = theme::hover_background(t);
                div()
                    .id(link_item_element_id(item))
                    .test_support()
                    .role(Role::MenuItem)
                    .aria_label(item.label())
                    .flex()
                    .flex_row()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(selected, |row| row.bg(theme::selected_background(t)))
                    .when(!selected, |row| row.hover(move |style| style.bg(hover)))
                    .when(first_copy == Some(ix), |row| {
                        row.mt_1().border_t_1().border_color(theme::divider(t))
                    })
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(theme::foreground(t))
                            .child(item.label()),
                    )
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.emit(LinkMenuEvent::Picked(item));
                    }))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        div()
            .id(LINK_MENU_ID)
            .test_support()
            .role(Role::Menu)
            .aria_label("Link menu")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .gap_0p5()
            .children(rows)
    }
}

/// The open Link menu: its view, where it is anchored, what it acts on, and
/// what had the keyboard before it.
pub(crate) struct OpenLinkMenu {
    pub(crate) view: Entity<LinkMenu>,
    pub(crate) at: Point<Pixels>,
    /// The group the menu opened in.
    pub(crate) group: GroupId,
    /// The note that holds the link: the group's note, or in a facet view
    /// the piece's source note.
    pub(crate) from: VaultPath,
    pub(crate) target: FollowTarget,
    /// Where the keyboard goes back when the menu is dismissed.
    pub(crate) restore: Option<FocusHandle>,
    /// The menu opened in a facet view piece.
    pub(crate) facet: bool,
    _events: Subscription,
}

impl OpenLinkMenu {
    fn pick(&self) -> LinkMenuPick {
        LinkMenuPick {
            group: self.group,
            from: self.from.clone(),
            target: self.target.clone(),
            facet: self.facet,
        }
    }
}

impl Workspace {
    /// Open the Link menu for what `pick` names (its group, its note, its
    /// target), right-clicked at `at` (window coordinates), with the
    /// keyboard. Refused while a modal is open, and for a group that is
    /// gone. An open note menu or Link menu gives way to it.
    pub(crate) fn open_link_menu(
        &mut self,
        pick: LinkMenuPick,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let LinkMenuPick {
            group,
            from,
            target,
            facet,
        } = pick;
        if modal::dialog_open(window, cx) || self.state(group).is_none() {
            return;
        }
        self.cancel_leader(cx);
        self.end_drag(window, cx);
        // Where the keyboard was before either menu took it.
        let restore = match (self.link_menu.take(), self.note_menu.take()) {
            (Some(open), _) => open.restore,
            (None, Some(open)) => open.restore,
            (None, None) => window.focused(cx),
        };
        let ctx = LinkMenuContext::of(&target, &from, facet);
        let view = cx.new(|cx| LinkMenu::new(ctx, cx));
        let events = cx.subscribe_in(&view, window, Self::on_link_menu_event);
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.link_menu = Some(OpenLinkMenu {
            view,
            at,
            group,
            from,
            target,
            restore,
            facet,
            _events: events,
        });
        cx.notify();
    }

    /// The editor of `group` asked for the Link menu: `from` is the note the
    /// group holds. Nothing for a group with no note (a link typed in an
    /// empty group resolves from nowhere).
    pub(super) fn open_link_menu_in_editor(
        &mut self,
        group: GroupId,
        at: Point<Pixels>,
        target: FollowTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(from) = self.host_in(group).and_then(|host| host.note.clone()) else {
            return;
        };
        let pick = LinkMenuPick {
            group,
            from,
            target,
            facet: false,
        };
        self.open_link_menu(pick, at, window, cx);
    }

    /// Dismiss the open Link menu: the keyboard goes back where it was.
    pub(crate) fn close_link_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = self.link_menu.take() else {
            return;
        };
        match open.restore {
            Some(handle) => window.focus(&handle, cx),
            None => self.apply_focus(window, cx),
        }
        cx.notify();
    }

    /// Close the open Link menu where there is no window at hand (its note
    /// or its group went, a drag started): the keyboard goes to the focused
    /// leaf once the update is done.
    pub(super) fn forget_link_menu(&mut self, cx: &mut Context<Self>) {
        if self.link_menu.take().is_none() {
            return;
        }
        self.with_window(cx, |workspace, window, cx| {
            if workspace.link_menu.is_none() && workspace.note_menu.is_none() {
                workspace.apply_focus(window, cx);
            }
        });
        cx.notify();
    }

    /// Close the Link menu when its group is `group` (the group is going,
    /// or shows another note).
    pub(super) fn forget_link_menu_of(&mut self, group: GroupId, cx: &mut Context<Self>) {
        if self
            .link_menu
            .as_ref()
            .is_some_and(|open| open.group == group)
        {
            self.forget_link_menu(cx);
        }
    }

    /// Close the Link menu when its note is at or below `path` (renamed,
    /// moved or deleted): a relative link would resolve from a stale path.
    pub(super) fn forget_link_menu_under(&mut self, path: &VaultPath, cx: &mut Context<Self>) {
        if self
            .link_menu
            .as_ref()
            .is_some_and(|open| path_at_or_below(&open.from, path))
        {
            self.forget_link_menu(cx);
        }
    }

    /// While the Link menu is open it keeps the keyboard: what the layout
    /// would focus becomes where a dismiss returns it ([`Self::apply_focus`]).
    /// False when no Link menu is open.
    pub(super) fn keep_focus_on_link_menu(
        &mut self,
        handle: FocusHandle,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(open) = self.link_menu.as_mut() else {
            return false;
        };
        open.restore = Some(handle);
        let menu = open.view.read(cx).focus_handle(cx);
        window.focus(&menu, cx);
        true
    }

    fn on_link_menu_event(
        &mut self,
        menu: &Entity<LinkMenu>,
        event: &LinkMenuEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.link_menu.as_ref().filter(|open| &open.view == menu) else {
            return;
        };
        match event {
            LinkMenuEvent::Dismissed => self.close_link_menu(window, cx),
            LinkMenuEvent::Picked(item) => {
                let item = *item;
                let pick = open.pick();
                self.close_link_menu(window, cx);
                // Once the keyboard is back, so what the pick opens takes
                // it from there.
                let workspace = cx.entity().downgrade();
                window.defer(cx, move |window, cx| {
                    workspace
                        .update(cx, |workspace, cx| {
                            workspace.run_link_menu_pick(pick, item, window, cx)
                        })
                        .ok();
                });
            }
        }
    }

    /// Run `item`, picked in the Link menu `pick` describes. A copy item
    /// puts the target as written on the clipboard (a label with its `#`)
    /// and leaves the active group as it is. Any other item first makes
    /// the menu's group active with the keyboard, then acts: an open item
    /// follows the target from `pick.from` to where it says ([`OpenAt`]);
    /// `Search for label` opens the search on the label.
    ///
    /// Nothing when the group is gone. The pick runs a moment after the
    /// click: an editor group that now shows another note than `from`, or
    /// a group that no longer shows a facet view, would follow a link the
    /// user did not pick, so a flash says so ([`MENU_NOTE_CHANGED`]). A
    /// facet view shows several notes, so `from` (the piece's note) is not
    /// checked against the group there.
    ///
    /// A split pick while the shown tab is closing flashes [`TAB_CLOSING`]
    /// at once. A split pick on a note that is already open also flashes
    /// `TAB_CLOSING` while the tab is closing, instead of focusing that
    /// note's group.
    pub(super) fn run_link_menu_pick(
        &mut self,
        pick: LinkMenuPick,
        item: LinkItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state(pick.group) else {
            return;
        };
        let current = if pick.facet {
            state.facet.is_some()
        } else {
            self.shown_note_in(pick.group, cx).as_ref() == Some(&pick.from)
        };
        if !current {
            self.flash(MENU_NOTE_CHANGED, cx);
            return;
        }
        // A copy acts on nothing in the window: the active group stays.
        if item.is_copy() {
            let (text, flash) = match pick.target {
                FollowTarget::Link { target, .. } => (target, LINK_COPIED),
                // The label as the note writes it.
                FollowTarget::Label(name) => (format!("#{name}"), LABEL_COPIED),
            };
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.flash(flash, cx);
            return;
        }
        // A right-click in a facet piece leaves its group as it was.
        self.focus_group(pick.group, cx);
        self.apply_focus(window, cx);
        let at = match item {
            LinkItem::Open | LinkItem::OpenInBrowser => OpenAt::Here,
            LinkItem::OpenNewTab => OpenAt::NewTab,
            LinkItem::OpenSplitRight => OpenAt::Split(TreeAxis::Horizontal),
            LinkItem::OpenSplitDown => OpenAt::Split(TreeAxis::Vertical),
            LinkItem::OpenInDefaultApp => OpenAt::Externally,
            LinkItem::SearchLabel => {
                if let FollowTarget::Label(name) = &pick.target {
                    self.follow_label(name, cx);
                }
                cx.notify();
                return;
            }
            LinkItem::CopyLink | LinkItem::CopyLabel => return,
        };
        // Said at once, before the lookup: the tab takes no new group.
        if matches!(at, OpenAt::Split(_)) && self.tab_mid_close(self.layout.region().shown()) {
            self.flash_error(TAB_CLOSING, cx);
            return;
        }
        if let FollowTarget::Link { target, kind } = pick.target {
            self.follow_link_from(&pick.from, target, kind, at, cx);
        }
        cx.notify();
    }

    /// The open Link menu over a window-wide layer that dismisses it on a
    /// press, kept inside the window (the note menu's overlay).
    pub(super) fn render_link_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let open = self.link_menu.as_ref()?;
        let workspace = cx.entity().downgrade();
        let dismiss = move || {
            let workspace = workspace.clone();
            move |_: &gpui_kit::MouseDownEvent, window: &mut Window, cx: &mut App| {
                workspace
                    .update(cx, |this, cx| this.close_link_menu(window, cx))
                    .ok();
            }
        };
        Some(super::menu_overlay::overlay(
            super::menu_overlay::Placement {
                layer_id: LINK_MENU_LAYER_ID,
                width_rems: MENU_REMS,
                at: open.at,
                anchor: gpui_kit::Anchor::TopLeft,
            },
            open.view.clone(),
            window,
            cx,
            dismiss(),
        ))
    }
}
