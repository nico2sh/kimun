//! The **drawer list** (M7a G1): the one list component behind the
//! note-scoped drawer views — LINKS and OUTLINE now, TAGS and FIND later.
//! A view hands it rows (a [`RowSource`] loaded off the UI thread, or rows it
//! already has) and says what to show when there are none; the list draws
//! them and turns keys and clicks into selection and [`DrawerListEvent`]s.
//!
//! **State.** The rows, their order and the selection are the logic
//! [`ListCore`]'s, as in FILES and the pickers. A new load replaces the core,
//! so a slower load for an earlier note never lands (its channel is gone),
//! and the core's own generation guard covers a reload.
//!
//! **Rows.** Each row type says how it draws ([`DrawerRow`]): one frame for
//! every list ([`row_frame`](super::files::row_frame)), uniform height per
//! list, the element id from its domain key (never its position). Only the
//! visible rows are built (`uniform_list`).
//!
//! **Keys** (while the list has the keyboard; it has no text field): Up and
//! Down move the selection, Home and End jump to the ends, Enter activates
//! the selected row, Escape gives the keyboard back to the editor. Every
//! other key bubbles on to the view around the list.
//!
//! **Pointer.** A click selects a row; a double-click activates it. A row
//! that stands for a vault entry has the FILES entry menu.

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Div, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, Role,
    ScrollStrategy, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    UniformListScrollHandle, Window, div, uniform_list,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::input::{Key, Mods};
use kimun_notes::logic::list::{ListCore, ListRow, ListSetup, RowSource, StaticRowSource};
use kimun_notes::logic::message::Notifier;
use kimun_notes::settings::themes::Theme;

use super::file_dialogs::FileOpKind;
use super::files::with_file_menu;
use crate::input;
use crate::theme;

/// A row a drawer list can draw.
pub(crate) trait DrawerRow: ListRow {
    /// The row's element id, from its domain key: it keeps its identity
    /// across a reload or a re-sort.
    fn element_id(&self) -> SharedString;

    /// The row, drawn in its list's uniform height. The list adds the
    /// pointer handling, the context menu and `test_support`.
    fn render_row(&self, selected: bool, theme: &Theme) -> Stateful<Div>;

    /// The vault entry the row stands for, when it has the FILES entry
    /// menu (Rename…, Move…, Delete…).
    fn menu_target(&self) -> Option<VaultPath> {
        None
    }
}

/// What a drawer list asks of its owner.
#[derive(Debug, Clone)]
pub(crate) enum DrawerListEvent<R> {
    /// Enter, or a double-click: do what the row stands for.
    Activate(R),
    /// A row's context menu asked to rename, move or delete its entry.
    FileOp(FileOpKind, VaultPath),
    /// Escape: back to the editor.
    Dismiss,
}

/// One drawer list. See the module docs.
pub(crate) struct DrawerList<R: DrawerRow> {
    /// `None` until the first rows were asked for.
    core: Option<ListCore<R>>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    /// Wakes the window when a load has rows (the app channel's redraw).
    redraw: Notifier,
    theme: Theme,
    /// The list element's id and its accessible name.
    id: &'static str,
    label: &'static str,
    /// What the list says when it shows no row.
    empty: SharedString,
}

impl<R: DrawerRow> EventEmitter<DrawerListEvent<R>> for DrawerList<R> {}

impl<R: DrawerRow> DrawerList<R> {
    pub(crate) fn new(
        id: &'static str,
        label: &'static str,
        redraw: Notifier,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            core: None,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            redraw,
            theme,
            id,
            label,
            empty: SharedString::default(),
        }
    }

    /// List what `source` loads (off the UI thread); `empty` is what the
    /// list says if it loads nothing. Replaces whatever was listed or
    /// loading.
    pub(crate) fn load(
        &mut self,
        source: Arc<dyn RowSource<R>>,
        empty: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.core = Some(ListCore::new(
            source,
            self.redraw.clone(),
            ListSetup::default(),
        ));
        self.restart(empty, cx);
    }

    /// List `rows`, here and now; `empty` is what the list says when there
    /// are none.
    pub(crate) fn set_rows(
        &mut self,
        rows: Vec<R>,
        empty: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.core = Some(ListCore::with_rows(
            Arc::new(StaticRowSource),
            self.redraw.clone(),
            ListSetup::default(),
            rows,
        ));
        self.restart(empty, cx);
    }

    fn restart(&mut self, empty: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.empty = empty.into();
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Apply what the load delivered (on the redraw it sends).
    pub(crate) fn poll(&mut self, cx: &mut Context<Self>) {
        let Some(core) = self.core.as_mut() else {
            return;
        };
        let polled = core.poll();
        if polled.changed {
            if polled.restarted {
                self.scroll.scroll_to_item(0, ScrollStrategy::Top);
            }
            cx.notify();
        }
    }

    /// Whether rows were asked for and have not landed yet (tests).
    #[cfg(test)]
    pub(crate) fn is_loading(&self) -> bool {
        self.core.as_ref().is_some_and(ListCore::is_loading)
    }

    /// Whether rows were ever asked for.
    #[cfg(test)]
    pub(crate) fn is_started(&self) -> bool {
        self.core.is_some()
    }

    /// The rows shown, top first.
    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<R> {
        self.core
            .as_ref()
            .map(|core| core.visible_rows().into_iter().cloned().collect())
            .unwrap_or_default()
    }

    /// What the list says when it shows no row.
    #[cfg(test)]
    pub(crate) fn empty_text(&self) -> &str {
        &self.empty
    }

    /// The row Enter would activate.
    pub(crate) fn selected_row(&self) -> Option<&R> {
        self.core.as_ref().and_then(ListCore::selected_row)
    }

    // ---- Selection and activation --------------------------------------------

    fn select(&mut self, pick: impl FnOnce(&mut ListCore<R>), cx: &mut Context<Self>) {
        let Some(core) = self.core.as_mut() else {
            return;
        };
        pick(core);
        if let Some(pos) = core.selected() {
            self.scroll.scroll_to_item(pos, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    fn activate(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.selected_row().cloned() {
            cx.emit(DrawerListEvent::Activate(row));
        }
    }

    /// A left click on the row at visible position `pos`: select it, and
    /// activate it on a double-click.
    fn click_row(&mut self, pos: usize, event: &ClickEvent, cx: &mut Context<Self>) {
        self.select(|core| core.select(pos), cx);
        if event.click_count() >= 2 {
            self.activate(cx);
        }
    }

    /// The list's own keys (module docs). True when the key was one.
    fn key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(key) = input::key_down_input(event) else {
            return false;
        };
        if key.mods != Mods::NONE {
            return false;
        }
        match key.key {
            Key::Up => self.select(ListCore::select_prev, cx),
            Key::Down => self.select(ListCore::select_next, cx),
            Key::Home => self.select(|core| core.select(0), cx),
            Key::End => self.select(
                |core| {
                    let last = core.visible_len().saturating_sub(1);
                    core.select(last);
                },
                cx,
            ),
            Key::Enter => self.activate(cx),
            Key::Esc => cx.emit(DrawerListEvent::Dismiss),
            _ => return false,
        }
        true
    }

    fn render_row(
        &self,
        pos: usize,
        row: &R,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let element = row
            .render_row(selected, &self.theme)
            .on_click(
                cx.listener(move |this, event: &ClickEvent, _, cx| this.click_row(pos, event, cx)),
            )
            .test_support();
        match row.menu_target() {
            Some(path) => with_file_menu(
                element,
                path,
                cx.entity().downgrade(),
                DrawerListEvent::FileOp,
            ),
            None => element.into_any_element(),
        }
    }
}

impl<R: DrawerRow> Focusable for DrawerList<R> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<R: DrawerRow> Render for DrawerList<R> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (count, selected, loading) = self.core.as_ref().map_or((0, None, false), |core| {
            (core.visible_len(), core.selected(), core.is_loading())
        });
        let body = if count > 0 {
            uniform_list(
                self.id,
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    // Only the visible rows are built, each from its row.
                    range
                        .filter_map(|pos| {
                            let row = this.core.as_ref()?.visible_row(pos)?;
                            Some(this.render_row(pos, row, selected == Some(pos), cx))
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full()
            .into_any_element()
        } else {
            let text = if loading {
                SharedString::from("Loading…")
            } else {
                self.empty.clone()
            };
            div()
                .px_3()
                .py_2()
                .text_sm()
                .text_color(theme::muted(&self.theme))
                .child(text)
                .into_any_element()
        };
        div()
            .id(SharedString::from(format!("{}-rows", self.id)))
            .test_support()
            .track_focus(&self.focus)
            .role(Role::ListBox)
            .aria_label(self.label)
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if this.key(event, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(body)
    }
}
