//! The **note menu**: one pointer-anchored menu that holds the actions on
//! a group's note (`Properties…`, `Rename…`, `Move…`, `Delete…`) and the
//! layout actions (split, new tab, close group, close tab).
//!
//! The items are a pure function of a small [`MenuContext`] ([`menu_items`]):
//! an item that cannot act is hidden, never grayed out. The view only emits
//! [`NoteMenuEvent`]s; the workspace that draws it runs the pick. The menu
//! is not modal: a key it does not use goes on to its parent.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, Role, StatefulInteractiveElement, Styled,
    WeakEntity, Window, div,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout_tree::GroupId;

use super::Workspace;
use super::file_dialogs::FileOpKind;
use crate::input;
use crate::keymap;
use crate::theme;

/// The menu's element id, for tests.
pub(crate) const NOTE_MENU_ID: &str = "note-menu";

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
// The group, the target and the workspace are read by the host that draws
// the menu (the workspace wiring).
#[allow(dead_code)]
pub(crate) struct NoteMenu {
    group: GroupId,
    target: Option<VaultPath>,
    items: Vec<MenuItem>,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    selected: usize,
}

impl EventEmitter<NoteMenuEvent> for NoteMenu {}

#[allow(dead_code)]
impl NoteMenu {
    pub(crate) fn new(
        group: GroupId,
        target: Option<VaultPath>,
        ctx: MenuContext,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            group,
            target,
            items: menu_items(&ctx),
            workspace,
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
        let first_layout = self
            .items
            .iter()
            .position(|i| !i.is_note())
            .filter(|&ix| ix > 0);
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
