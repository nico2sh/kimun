//! The **drawer list** (M7a G1): the one list component behind the
//! drawer views — LINKS, OUTLINE, TAGS, FIND, SEM and the Sources view.
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
//! **Pointer.** The list's [`ClickPolicy`]: `Open` (every list but one) — a
//! click selects the row and activates it, as Enter does, under the
//! pointing hand; `Select` (the Sources view) — a click selects, a
//! double-click opens the row's note ([`DrawerListEvent::Verb`] with
//! [`OPEN_NOTE`]). A
//! right click never activates: a row that stands for a vault entry has
//! the FILES entry menu there.
//!
//! **The expand chevron** ([`super::chevron`]). A list whose rows have a
//! preview shows the chevron ([`DrawerList::set_chevron`], off by default)
//! at the right end of the selected row and of the hovered row (GPUI group
//! hover), over the row's own background, taking no width from the row.
//! A click on it selects the row and emits [`DrawerListEvent::Verb`] with
//! [`EXPAND`], and never reaches the row's own click: no activation, no
//! second select, and the press after it is not taken for a double-click
//! on the row. It is a pointer shortcut only; the preview keeps its keys.

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render,
    Role, ScrollStrategy, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    UniformListScrollHandle, Window, div, uniform_list,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::input::{Key, Mods};
use kimun_notes::logic::list::{ListCore, ListRow, ListSetup, RowSource, StaticRowSource};
use kimun_notes::logic::message::{AppTx, Notifier, redraw_callback};
use kimun_notes::settings::themes::Theme;

use super::chevron::{Reach, chevron};
use super::file_dialogs::FileOpKind;
use super::files::{ClickPolicy, ClickRows, RowClick, with_click_cursor, with_file_menu};
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

    /// The extra items of the row's context menu, for a row that has no
    /// FILES entry menu (an Ask source offers [`MenuItem::open_note`]). Each
    /// reaches the owner as a [`DrawerListEvent::Verb`].
    fn menu_items(&self) -> Vec<MenuItem> {
        Vec::new()
    }
}

/// The group name a row with the chevron carries, for the chevron's hover.
const ROW_GROUP: &str = "drawer-row";

/// A row-level command the owner answers to: the one name a context-menu
/// item, the expand chevron or a double-click carries in
/// [`DrawerListEvent::Verb`]. A new row command is a new verb, not a new
/// event variant.
pub(crate) type VerbId = &'static str;

/// Open the row's note in the editor: the menu's `Open note`, and what a
/// double-click does under [`ClickPolicy::Select`].
pub(crate) const OPEN_NOTE: VerbId = "open-note";

/// Step the row's preview: a click on the row's expand chevron.
pub(crate) const EXPAND: VerbId = "expand";

/// The element id of the expand chevron on the row (or header) `row_id`.
pub(crate) fn chevron_id(row_id: &SharedString) -> SharedString {
    format!("{row_id}:chevron").into()
}

/// An extra item of a row's context menu ([`DrawerRow::menu_items`]).
#[derive(Debug, Clone, Copy)]
pub(crate) struct MenuItem {
    label: &'static str,
    verb: VerbId,
}

impl MenuItem {
    pub(crate) const fn new(label: &'static str, verb: VerbId) -> Self {
        Self { label, verb }
    }

    /// `Open note` ([`OPEN_NOTE`]).
    pub(crate) const fn open_note() -> Self {
        Self::new("Open note", OPEN_NOTE)
    }
}

/// What a drawer list asks of its owner.
#[derive(Debug, Clone)]
pub(crate) enum DrawerListEvent<R> {
    /// Enter, or a click under [`ClickPolicy::Open`]: do what the row
    /// stands for.
    Activate(R),
    /// A row's context menu asked to rename, move or delete its entry.
    FileOp(FileOpKind, VaultPath),
    /// A row command: a context-menu item ([`DrawerRow::menu_items`]), a
    /// click on the row's expand chevron ([`EXPAND`]; the row is selected
    /// already) or a double-click under [`ClickPolicy::Select`]
    /// ([`OPEN_NOTE`]).
    Verb(R, VerbId),
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
    /// The list element's id and its accessible name.
    id: &'static str,
    label: &'static str,
    /// What the list says when it shows no row.
    empty: SharedString,
    /// What it says while its first rows are on their way.
    loading: SharedString,
    /// What a click on a row does.
    click: ClickPolicy,
    /// The expand chevron the rows show, `None` for none.
    chevron: Option<Reach>,
    /// A chevron was the last press: the next row click is not the second
    /// half of a double-click.
    after_chevron: bool,
}

impl<R: DrawerRow> EventEmitter<DrawerListEvent<R>> for DrawerList<R> {}

impl<R: DrawerRow> DrawerList<R> {}

impl<R: DrawerRow> DrawerList<R> {
    pub(crate) fn new(
        id: &'static str,
        label: &'static str,
        redraw: Notifier,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            core: None,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            redraw,
            id,
            label,
            empty: SharedString::default(),
            loading: "Loading…".into(),
            click: ClickPolicy::Open,
            chevron: None,
            after_chevron: false,
        }
    }

    /// A new list as an entity of `cx`'s view, on the app channel `tx`.
    pub(crate) fn spawn<W: 'static>(
        id: &'static str,
        label: &'static str,
        tx: AppTx,
        cx: &mut Context<W>,
    ) -> Entity<Self> {
        Self::spawn_with(id, label, tx, cx, |list| list)
    }

    /// [`Self::spawn`], the list set up by `configure` first (its click
    /// policy, its loading text).
    pub(crate) fn spawn_with<W: 'static>(
        id: &'static str,
        label: &'static str,
        tx: AppTx,
        cx: &mut Context<W>,
        configure: impl FnOnce(Self) -> Self,
    ) -> Entity<Self> {
        cx.new(|cx| configure(Self::new(id, label, redraw_callback(tx), cx)))
    }

    /// Show the expand chevron stepping `reach` on the selected and the
    /// hovered row from now on, or (`None`) none.
    pub(crate) fn set_chevron(&mut self, reach: Option<Reach>, cx: &mut Context<Self>) {
        if self.chevron != reach {
            self.chevron = reach;
            cx.notify();
        }
    }

    /// Take clicks under `policy` instead of [`ClickPolicy::Open`].
    pub(crate) fn with_click_policy(mut self, policy: ClickPolicy) -> Self {
        self.click = policy;
        self
    }

    /// Say `text` instead of "Loading…" while rows are on their way (a
    /// server search says "Searching…").
    pub(crate) fn with_loading_text(mut self, text: impl Into<SharedString>) -> Self {
        self.loading = text.into();
        self
    }

    /// Say `text` when the list shows no row, from now on (until the next
    /// load names its own).
    pub(crate) fn set_empty(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        if self.empty != text {
            self.empty = text;
            cx.notify();
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
        self.load_with(source, ListSetup::default(), empty, cx);
    }

    /// [`Self::load`] with the list's own setup: a local filter over the
    /// rows (TAGS), or the query a reloading source runs first (FIND).
    pub(crate) fn load_with(
        &mut self,
        source: Arc<dyn RowSource<R>>,
        setup: ListSetup<R>,
        empty: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.core = Some(ListCore::new(source, self.redraw.clone(), setup));
        self.restart(empty, cx);
    }

    /// Hand the listing a new query: a source that loads per query runs it
    /// (the load under way is aborted, its rows never land); a loaded set
    /// is filtered again here and now. Nothing before the first load.
    pub(crate) fn set_query(&mut self, query: impl Into<String>, cx: &mut Context<Self>) {
        let Some(core) = self.core.as_mut() else {
            return;
        };
        core.set_query(query);
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    /// Run the current query again (the vault changed under the rows).
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        if let Some(core) = self.core.as_mut() {
            core.reload();
            cx.notify();
        }
    }

    /// The query the rows were asked for.
    pub(crate) fn query(&self) -> &str {
        self.core.as_ref().map_or("", ListCore::query)
    }

    /// List `rows`, here and now; `empty` is what the list says when there
    /// are none.
    pub(crate) fn set_rows(
        &mut self,
        rows: Vec<R>,
        empty: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.set_rows_with(rows, ListSetup::default(), empty, cx);
    }

    /// [`Self::set_rows`] with the list's own setup: a local filter over
    /// the rows (the Sources view's).
    pub(crate) fn set_rows_with(
        &mut self,
        rows: Vec<R>,
        setup: ListSetup<R>,
        empty: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.core = Some(ListCore::with_rows(
            Arc::new(StaticRowSource),
            self.redraw.clone(),
            setup,
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

    /// Whether rows were asked for and have not landed yet.
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

    /// How many rows are shown.
    pub(crate) fn len(&self) -> usize {
        self.core.as_ref().map_or(0, ListCore::visible_len)
    }

    /// The first row shown.
    pub(crate) fn first_row(&self) -> Option<&R> {
        self.core.as_ref().and_then(|core| core.visible_row(0))
    }

    /// The selected row's position.
    pub(crate) fn selected(&self) -> Option<usize> {
        self.core.as_ref().and_then(ListCore::selected)
    }

    /// The row Enter would activate.
    pub(crate) fn selected_row(&self) -> Option<&R> {
        self.core.as_ref().and_then(ListCore::selected_row)
    }

    /// Every row listed, a local filter's misses included, in their order.
    pub(crate) fn all_rows(&self) -> &[R] {
        self.core.as_ref().map_or(&[], ListCore::rows)
    }

    /// The position among the rows shown of the first one `pick` takes.
    pub(crate) fn position(&self, pick: impl Fn(&R) -> bool) -> Option<usize> {
        let core = self.core.as_ref()?;
        (0..core.visible_len()).find(|&pos| core.visible_row(pos).is_some_and(&pick))
    }

    /// Select the row shown at `pos` and keep it in view.
    pub(crate) fn select_at(&mut self, pos: usize, cx: &mut Context<Self>) {
        self.select(|core| core.select(pos), cx);
    }

    // ---- Selection and activation --------------------------------------------

    /// Move the selection one row down (`true`) or up, as the arrows do
    /// (FIND's query field steps the list it heads).
    pub(crate) fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if down {
            self.select(ListCore::select_next, cx);
        } else {
            self.select(ListCore::select_prev, cx);
        }
    }

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

    /// A click on the row at visible position `pos`, under the list's
    /// policy ([`ClickRows`]): select it, then activate it (`Open`) or open
    /// its note (`Select`, a double-click).
    fn click_row(
        &mut self,
        pos: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let after_chevron = std::mem::take(&mut self.after_chevron);
        let mut click = self.click.click(event);
        // The press before was the chevron's (it went away under the
        // pointer): this one is a click of its own, not a double-click.
        if click != RowClick::Ignore && after_chevron && event.click_count() > 1 {
            click = RowClick::Select;
        }
        click.run(self, pos, window, cx);
    }

    /// A click on the chevron of the row at visible position `pos`: select
    /// it, then hand it to the owner.
    fn click_chevron(&mut self, pos: usize, cx: &mut Context<Self>) {
        self.after_chevron = true;
        self.select(|core| core.select(pos), cx);
        if let Some(row) = self.selected_row().cloned() {
            cx.emit(DrawerListEvent::Verb(row, EXPAND));
        }
    }

    /// The chevron stepping `reach` on the row `row_id` at `pos`: drawn on
    /// the selected row, and on any other only while the pointer is over it
    /// ([`ROW_GROUP`]).
    fn render_chevron(
        &self,
        pos: usize,
        row_id: &SharedString,
        selected: bool,
        reach: Reach,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        // It sits over the row's right end, on the row's own background, so
        // it covers what is there rather than narrowing the row for everyone.
        let cover = if selected {
            theme::selected_background(t)
        } else {
            theme::hover_background(t)
        };
        chevron(chevron_id(row_id), reach, t)
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .pl_2()
            .pr_2()
            .rounded_md()
            .bg(cover)
            .text_sm()
            .when(!selected, |chevron| {
                chevron
                    .opacity(0.)
                    .group_hover(ROW_GROUP, |style| style.opacity(1.))
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                // Never the row's click as well; a press that ends a
                // double-click on it acted already with its first.
                cx.stop_propagation();
                if event.standard_click() && event.click_count() == 1 {
                    this.click_chevron(pos, cx);
                }
            }))
            .test_support()
            .into_any_element()
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
        let mut element = row.render_row(selected, &theme::current(cx));
        if let Some(reach) = self.chevron {
            let chevron = self.render_chevron(pos, &row.element_id(), selected, reach, cx);
            element = element.group(ROW_GROUP).relative().child(chevron);
        }
        let element = with_click_cursor(element, self.click, true)
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click_row(pos, event, window, cx)
            }))
            .test_support();
        match row.menu_target() {
            Some(path) => with_file_menu(
                element,
                path,
                cx.entity().downgrade(),
                DrawerListEvent::FileOp,
            ),
            None => {
                let items = row.menu_items();
                if items.is_empty() {
                    return element.into_any_element();
                }
                let owner = cx.entity().downgrade();
                let row = row.clone();
                element
                    .context_menu(move |mut menu, _, _| {
                        for item in &items {
                            let (owner, row, verb) = (owner.clone(), row.clone(), item.verb);
                            menu = menu.item(PopupMenuItem::new(item.label).on_click(
                                move |_, window, cx| {
                                    let (owner, row) = (owner.clone(), row.clone());
                                    window.defer(cx, move |_, cx| {
                                        owner
                                            .update(cx, |_, cx| {
                                                cx.emit(DrawerListEvent::Verb(row, verb))
                                            })
                                            .ok();
                                    });
                                },
                            ));
                        }
                        menu
                    })
                    .into_any_element()
            }
        }
    }
}

impl<R: DrawerRow> ClickRows for DrawerList<R> {
    fn select_row(&mut self, pos: usize, cx: &mut Context<Self>) {
        self.select(|core| core.select(pos), cx);
    }

    fn activate_row(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        match self.click {
            ClickPolicy::Open => self.activate(cx),
            ClickPolicy::Select => {
                if let Some(row) = self.selected_row().cloned() {
                    cx.emit(DrawerListEvent::Verb(row, OPEN_NOTE));
                }
            }
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
                self.loading.clone()
            } else {
                self.empty.clone()
            };
            div()
                .px_3()
                .py_2()
                .text_sm()
                .text_color(theme::muted(&theme::current(cx)))
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
