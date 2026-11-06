//! The **LINKS** drawer view (M7a G1): the open note's backlinks, outgoing
//! note links and unlinked mentions, one tab at a time, in the shared
//! [`DrawerList`].
//!
//! **Rows** are the logic's (`logic::views::link_entries`, loaded off the
//! UI thread); each is a note, drawn as FILES draws one (title over
//! filename), with the FILES entry menu. Enter or a click opens it
//! through the note session (the open note is saved first).
//!
//! **Tabs.** A segmented control over the list; `b`, `o` and `u`, or Left
//! and Right, switch tabs while the list has the keyboard.
//!
//! **When it loads.** Only while shown: when it is revealed, when the open
//! note changes (an open, a rename followed, the note gone: "No note open"),
//! when the tab changes, and after a file operation landed or the index
//! caught up. Typing and autosaves never reload it. A newer load replaces an
//! older one, whose rows never land.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Icon;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::{
    AppContext as _, Context, Div, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString, Stateful, Styled as _,
    Window, div,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::input::{Key, Mods};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::message::{AppTx, Notifier, redraw_callback};
use kimun_notes::logic::nav::{DrawerView, LinksTab};
use kimun_notes::logic::views::{LinkEntry, LinksSource};
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::drawer_list::{DrawerList, DrawerListEvent, DrawerRow};
use super::files::{NOTE_ROW_REMS, display_title, ellipsis, lane_row};
use crate::input;
use crate::theme;

/// Element ids, for tests.
pub(crate) const LINKS_ID: &str = "links";
pub(crate) const LINKS_LIST_ID: &str = "links-list";
pub(crate) const LINKS_TABS_ID: &str = "links-tabs";

/// The element id of the row for `path`.
pub(crate) fn link_row_id(path: &VaultPath) -> SharedString {
    format!("links-row:{path}").into()
}

/// A tab's name as the segmented control shows it.
pub(crate) fn tab_name(tab: LinksTab) -> &'static str {
    match tab {
        LinksTab::Backlinks => "Backlinks",
        LinksTab::Outgoing => "Outgoing",
        LinksTab::Unlinked => "Unlinked",
    }
}

/// What a tab says when it lists nothing for a note.
fn empty_text(tab: LinksTab) -> &'static str {
    match tab {
        LinksTab::Backlinks => "No notes link to this one",
        LinksTab::Outgoing => "This note links to no other note",
        LinksTab::Unlinked => "No other note mentions it by name",
    }
}

/// What every tab says without an open note.
pub(crate) const NO_NOTE_TEXT: &str = "No note open";

impl DrawerRow for LinkEntry {
    fn element_id(&self) -> SharedString {
        link_row_id(&self.path)
    }

    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        let title = SharedString::from(display_title(&self.title, &self.filename).to_string());
        let icon = Icon::new(IconName::FileText)
            .size_4()
            .text_color(theme::foreground(t));
        lane_row(
            self.element_id(),
            title.clone(),
            selected,
            NOTE_ROW_REMS,
            Some(icon.into_any_element()),
            [
                ellipsis(title)
                    .text_color(theme::foreground(t))
                    .into_any_element(),
                ellipsis(self.path.to_string())
                    .text_xs()
                    .text_color(theme::secondary(t))
                    .into_any_element(),
            ],
            t,
        )
    }

    fn menu_target(&self) -> Option<VaultPath> {
        Some(self.path.clone())
    }
}

/// The LINKS view. See the module docs.
pub(crate) struct LinksView {
    pub(crate) list: Entity<DrawerList<LinkEntry>>,
    vault: Arc<NoteVault>,
    tab: LinksTab,
    /// The note the rows are (being) listed for; `None` until first listed,
    /// and again when the tab changed while hidden.
    note: Option<VaultPath>,
    /// Whether the view was shown at the last refresh: a reveal reloads.
    shown: bool,
    /// A file operation landed since the rows were listed.
    stale: bool,
    theme: Theme,
}

impl LinksView {
    fn new(vault: Arc<NoteVault>, redraw: Notifier, theme: Theme, cx: &mut Context<Self>) -> Self {
        let list_theme = theme.clone();
        let list = cx.new(|cx| DrawerList::new(LINKS_LIST_ID, "Links", redraw, list_theme, cx));
        Self {
            list,
            vault,
            tab: LinksTab::Backlinks,
            note: None,
            shown: false,
            stale: false,
            theme,
        }
    }

    /// The tab shown (tests).
    #[cfg(test)]
    pub(crate) fn tab(&self) -> LinksTab {
        self.tab
    }

    /// The note the rows are listed for (tests).
    #[cfg(test)]
    pub(crate) fn listed_note(&self) -> Option<&VaultPath> {
        self.note.as_ref()
    }

    /// Bring the rows up to `note` (the open note, empty for none) if the
    /// view is `shown` and they are not (module docs). Hidden: nothing.
    pub(crate) fn refresh(&mut self, shown: bool, note: &VaultPath, cx: &mut Context<Self>) {
        let revealed = shown && !self.shown;
        self.shown = shown;
        if shown && (revealed || self.stale || self.note.as_ref() != Some(note)) {
            self.load(note.clone(), cx);
        }
    }

    /// The vault changed under the rows (a file operation landed, the
    /// index caught up): they may name paths that moved or miss links.
    /// Reloaded now if shown, else when next revealed.
    pub(crate) fn invalidate(&mut self, cx: &mut Context<Self>) {
        self.stale = true;
        if self.shown
            && let Some(note) = self.note.clone()
        {
            self.load(note, cx);
        }
    }

    /// Show `tab`; its rows load now if the view is shown, else when it is.
    pub(crate) fn set_tab(&mut self, tab: LinksTab, cx: &mut Context<Self>) {
        if tab == self.tab {
            return;
        }
        self.tab = tab;
        match self.note.clone() {
            Some(note) if self.shown => self.load(note, cx),
            _ => self.note = None,
        }
        cx.notify();
    }

    fn load(&mut self, note: VaultPath, cx: &mut Context<Self>) {
        self.stale = false;
        let tab = self.tab;
        if note.is_root_or_empty() {
            self.list
                .update(cx, |list, cx| list.set_rows(Vec::new(), NO_NOTE_TEXT, cx));
        } else {
            let source = Arc::new(LinksSource::new(self.vault.clone(), note.clone(), tab));
            self.list
                .update(cx, |list, cx| list.load(source, empty_text(tab), cx));
        }
        self.note = Some(note);
    }

    /// The tab keys (module docs), on their way out of the list. True when
    /// the key was one.
    fn key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(key) = input::key_down_input(event) else {
            return false;
        };
        if key.mods != Mods::NONE {
            return false;
        }
        let tab = match key.key {
            Key::Char('b') => LinksTab::Backlinks,
            Key::Char('o') => LinksTab::Outgoing,
            Key::Char('u') => LinksTab::Unlinked,
            Key::Left => self.tab.cycled(-1),
            Key::Right => self.tab.cycled(1),
            _ => return false,
        };
        self.set_tab(tab, cx);
        true
    }
}

impl Focusable for LinksView {
    /// The list: the tab keys arrive through it.
    fn focus_handle(&self, cx: &gpui_kit::App) -> FocusHandle {
        self.list.read(cx).focus_handle(cx)
    }
}

impl Render for LinksView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = LinksTab::ORDER
            .iter()
            .position(|t| *t == self.tab)
            .unwrap_or(0);
        let this = cx.entity().downgrade();
        let tabs = TabBar::new(LINKS_TABS_ID)
            .segmented()
            .small()
            .w_full()
            .selected_index(selected)
            .on_click(move |ix: &usize, window, cx| {
                let Some(tab) = LinksTab::ORDER.get(*ix).copied() else {
                    return;
                };
                this.update(cx, |view, cx| {
                    view.set_tab(tab, cx);
                    // The keyboard stays on the list, where the tab keys
                    // work.
                    let handle = view.list.read(cx).focus_handle(cx);
                    window.focus(&handle, cx);
                })
                .ok();
            })
            .children(
                LinksTab::ORDER
                    .iter()
                    .map(|tab| Tab::new().flex_1().label(tab_name(*tab))),
            );
        div()
            .id(LINKS_ID)
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if this.key(event, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(div().flex_none().px_2().pb_2().child(tabs))
            .child(self.list.clone())
            .text_color(theme::foreground(&self.theme))
    }
}

// ---- The workspace's side ----------------------------------------------------

impl Workspace {
    /// Make the LINKS view over the vault the window opened (nothing is
    /// listed until it is shown).
    pub(super) fn start_links(
        &mut self,
        vault: Arc<NoteVault>,
        tx: AppTx,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = self.theme.clone();
        let view = cx.new(|cx| LinksView::new(vault, redraw_callback(tx), theme, cx));
        let list = view.read(cx).list.clone();
        self._views_events
            .push(cx.subscribe_in(&list, window, Self::on_links_event));
        self.links = Some(view);
    }

    fn on_links_event(
        &mut self,
        _: &Entity<DrawerList<LinkEntry>>,
        event: &DrawerListEvent<LinkEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Saved first, as every open is (the note session's switch).
            DrawerListEvent::Activate(row) => {
                self.focus_panel(Panel::Editor, window, cx);
                self.open_note(row.path.clone(), cx);
            }
            DrawerListEvent::FileOp(kind, path) => {
                self.open_file_dialog(*kind, path.clone(), window, cx)
            }
            DrawerListEvent::Verb(..) => {}
            DrawerListEvent::Dismiss => self.focus_panel(Panel::Editor, window, cx),
        }
    }

    /// Leader `l b/o/u`, the palette's links rows, the Footer's backlinks:
    /// LINKS on `tab`, shown and given the keyboard.
    pub(crate) fn show_links_tab(
        &mut self,
        tab: LinksTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(links) = self.links.as_ref() {
            links.update(cx, |view, cx| view.set_tab(tab, cx));
        }
        self.show_view(DrawerView::Links, window, cx);
    }
}
