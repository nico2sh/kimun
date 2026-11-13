//! The **tab strip** (ADR-0058): a row of pill buttons over the shown tab,
//! one per tab, with a round plus button after them that opens a tab.
//! Hidden while the region holds one tab, so a window that never opens a
//! second tab looks as it always did. A pill names the note of its tab's
//! active group ([`tab_label`]). The shown tab's pill is filled with the
//! selected tint; the others are bare until the pointer is over them. Each
//! pill has one trailing slot of a fixed size ([`Trail`]): the close button
//! on the shown pill and on the pill under the pointer, else the Footer's
//! unsaved dot while any group of the tab has unsaved text
//! ([`tab_dirty`]). A click on a pill shows its tab; its close button
//! closes it. The pills scroll sideways when they do not fit, and the shown
//! tab is kept in view.
//!
//! The tab commands live here too: a new tab (one empty group, which takes
//! the keyboard), the next and previous tab (wrapping), and the close.
//!
//! While a group header is dragged ([`super::group_drop`]) the strip shows
//! even with one tab: each other tab's label and the plus button are drop
//! targets, with the drop zones' look ([`Workspace::drop_zone`]). A
//! close writes the unsaved text of every group of the tab first (all at
//! once, as the window's close does); when a write fails or cannot start
//! the tab stays and the flash says why. Closing the last tab leaves one
//! empty group: the region never has zero tabs or zero groups. Closing one
//! group of a tab ([`Workspace::close_editor_group`]) goes the same way,
//! for that group alone. While a tab's close writes its notes, no group is
//! added to it ([`Workspace::tab_mid_close`]): it would go with the tab.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement, IntoElement,
    ParentElement, Role, SharedString, StatefulInteractiveElement, Styled, Window, div, rems,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::editor_region::{CloseOutcome, TabId};
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::session::FlushResult;
use tokio::task::JoinError;

use super::Workspace;
use super::drop_zones::DropTarget;
use super::file_ops::run_all;
use super::note::Closing;
use super::note_flush::FlushBlock;
use super::session::Queued;
use crate::{keymap, theme};

/// The strip's element id.
pub(crate) const TAB_STRIP_ID: &str = "tab-strip";
/// The plus button's element id.
pub(crate) const TAB_NEW_ID: &str = "tab-strip-new";
/// The plus button's drop zone's element id, during a group's drag.
pub(crate) const TAB_NEW_DROP_ID: &str = "drop-tab-new";
/// The unsaved dot's id, with the tab's position.
pub(crate) const TAB_UNSAVED_ID: &str = "tab-unsaved";
/// A close button's id, with the tab's position.
pub(crate) const TAB_CLOSE_ID: &str = "tab-close";
/// What a tab whose active group shows nothing is called.
pub(crate) const EMPTY_LABEL: &str = "Empty";
/// Why no group is added to a tab whose close is under way.
pub(crate) const TAB_CLOSING: &str = "This tab is closing. Its notes are being saved.";
/// A tab's close glyph: the kit's own close icon (its dialogs and toasts
/// draw it), carried by the default bundle. Lucide's `x` is not bundled.
const CLOSE_ICON: IconName = IconName::Close;
/// The plus button's glyph.
const NEW_ICON: IconName = IconName::Plus;
/// Every glyph the strip draws, for the bundle test: an icon the asset
/// source does not carry paints nothing.
#[cfg(test)]
pub(crate) const ICONS: [IconName; 2] = [CLOSE_ICON, NEW_ICON];
/// The widest a pill grows before its name is cut with an ellipsis.
const LABEL_MAX_REMS: f32 = 14.;
/// A pill's height (and the plus button's size): a comfortable click
/// target that keeps the strip about as tall as a slot header.
const PILL_HEIGHT_REMS: f32 = 1.625;
/// The trailing slot of a pill: the close button, the unsaved dot or
/// nothing, always this size.
const TRAIL_REMS: f32 = 1.125;
/// The scrolling row of pills.
pub(crate) const TAB_LABELS_ID: &str = "tab-strip-labels";

/// What sits in the trailing slot of a tab's pill. The close button shows
/// on the pill under the pointer; else the unsaved dot while any group of
/// the tab has unsaved text (it turns into the close button under the
/// pointer); else the close button on the shown tab; else nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trail {
    Close,
    Unsaved,
    Nothing,
}

/// The element id of the trailing slot of the pill at `index`.
pub(crate) fn tab_trail_id(index: usize) -> ElementId {
    ElementId::NamedInteger("tab-trail".into(), index as u64)
}

/// What a running close takes away once its notes are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseTarget {
    /// A tab, with every group it holds.
    Tab(TabId),
    /// One group of a tab that holds others.
    Group(GroupId),
}

impl CloseTarget {
    /// What a flash calls it.
    fn noun(self) -> &'static str {
        match self {
            CloseTarget::Tab(_) => "tab",
            CloseTarget::Group(_) => "group",
        }
    }
}

/// The element id of the label at `index` (its position in the strip).
pub(crate) fn tab_label_id(index: usize) -> ElementId {
    ElementId::NamedInteger("tab-label".into(), index as u64)
}

/// The element id of the drop zone over the label at `index`, during a
/// group's drag.
pub(crate) fn tab_drop_id(index: usize) -> ElementId {
    ElementId::NamedInteger("drop-tab".into(), index as u64)
}

/// What `group` shows, named as the Footer names it: the attachment in
/// place of the editor, else the note. `None` for an empty group.
pub(super) fn group_name(workspace: &Workspace, group: GroupId, cx: &App) -> Option<String> {
    let state = workspace.state(group)?;
    let shown: Option<VaultPath> = match state.attachment.as_ref() {
        Some(view) => Some(view.read(cx).path().clone()),
        None => state.notes.as_ref().and_then(|h| h.note.clone()),
    };
    shown.map(|path| path.get_name())
}

/// `tab`'s label: the name of the note its active group shows (the
/// Footer's name for it), `Empty` for an empty group, plus ` +N` for the
/// tab's other groups.
pub(crate) fn tab_label(workspace: &Workspace, tab: TabId, cx: &App) -> SharedString {
    let region = workspace.layout.region();
    let groups = region.groups(tab);
    let Some(active) = region
        .active_group_of(tab)
        .or_else(|| groups.first().copied())
    else {
        return SharedString::default();
    };
    let name = group_name(workspace, active, cx).unwrap_or_else(|| EMPTY_LABEL.to_string());
    match groups.len() {
        0 | 1 => name.into(),
        n => format!("{name} +{}", n - 1).into(),
    }
}

/// Whether any group of `tab` has unsaved text: its label carries the dot.
pub(crate) fn tab_dirty(workspace: &Workspace, tab: TabId, cx: &App) -> bool {
    workspace
        .layout
        .region()
        .groups(tab)
        .into_iter()
        .any(|group| workspace.editor_dirty_in(group, cx))
}

impl Workspace {
    // ---- Commands ------------------------------------------------------------

    /// `NewTab`: a tab after the last one holding one empty group, shown,
    /// its editor with the keyboard (the empty group offers the finder). A
    /// zoom ends.
    /// Nothing without a vault, or while the window closes. While a stored
    /// record restores, it waits for it ([`super::session`]).
    pub(crate) fn new_tab(&mut self, cx: &mut Context<Self>) {
        if matches!(self.closing, Closing::Requested | Closing::Flushing) {
            return;
        }
        // A stored record still restoring would replace the tab: it waits.
        if self.hold_while_restoring(Queued::NewTab) {
            return;
        }
        let Some(state) = self.new_group_state(cx) else {
            return;
        };
        let (_, group) = self.layout.region_mut().new_tab();
        // Another tab shows: a zoom ends with the one it showed.
        self.layout.zoom(None);
        self.insert_group(group, state, cx);
        self.focus_group(group, cx);
        self.scroll_to_shown_tab();
    }

    /// `NextTab` / `PrevTab`: show the next (previous) tab, wrapping; its
    /// active group takes the keyboard. Nothing with one tab. While a
    /// stored record restores, it waits for it ([`super::session`]).
    pub(crate) fn step_tab(&mut self, forward: bool, cx: &mut Context<Self>) {
        // Steps among the restored tabs (and those asked for meanwhile).
        if self.hold_while_restoring(Queued::StepTab { forward }) {
            return;
        }
        if self.layout.region().tabs().len() < 2 {
            return;
        }
        let region = self.layout.region_mut();
        if forward {
            region.next_tab();
        } else {
            region.prev_tab();
        }
        self.show_active_group(cx);
    }

    /// Show `tab` (a click on its label); its active group takes the
    /// keyboard.
    pub(crate) fn show_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
        if self.layout.region_mut().show(tab) {
            self.show_active_group(cx);
        }
    }

    /// The shown tab changed: its active group becomes the window's (the
    /// Footer with fresh counts, FILES' mark, the views) and takes the
    /// keyboard; the strip scrolls to its label. A zoom ends.
    fn show_active_group(&mut self, cx: &mut Context<Self>) {
        // Another tab shows: a zoom ends with the one it showed.
        self.layout.zoom(None);
        let group = self.layout.active_group();
        self.focus_group(group, cx);
        self.scroll_to_shown_tab();
    }

    /// Bring the shown tab's label into the strip's view.
    pub(super) fn scroll_to_shown_tab(&self) {
        let region = self.layout.region();
        if let Some(at) = region.tabs().iter().position(|t| *t == region.shown()) {
            self.tab_scroll.scroll_to_item(at);
        }
    }

    /// `CloseTab` on the shown tab.
    pub(crate) fn close_shown_tab(&mut self, cx: &mut Context<Self>) {
        let tab = self.layout.region().shown();
        self.close_tab(tab, cx);
    }

    /// Close `tab`: write the unsaved text of every group it holds, then
    /// take its groups away (the last tab keeps one, emptied). Refused,
    /// with a flash, while one of its notes cannot be let go (opening, held
    /// by another operation, edits that cannot be saved); ignored while
    /// another close runs or the window closes.
    pub(crate) fn close_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
        if self.groups_closing.is_some() || !self.layout.region().tabs().contains(&tab) {
            return;
        }
        let groups = self.layout.region().groups(tab);
        self.close_flushing(CloseTarget::Tab(tab), groups, cx);
    }

    /// Close `group` (the `CloseSlot` chord on the editor, its header's
    /// close button): write its unsaved text first, then take it away; a
    /// neighbour takes its space and, when it was active, the group before
    /// it (or the first) becomes active. The last group of a tab closes the
    /// tab ([`Self::close_tab`]: the last tab keeps one empty group). The
    /// same refusals as a tab's close; ignored while another close runs.
    /// While a stored record restores, it waits for it
    /// ([`super::session`]).
    pub(crate) fn close_editor_group(&mut self, group: GroupId, cx: &mut Context<Self>) {
        // While a stored record restores, the one group there is is the
        // active one: the close waits, and closes the active group of then.
        if self.hold_while_restoring(Queued::CloseGroup) {
            return;
        }
        if self.groups_closing.is_some() {
            return;
        }
        let region = self.layout.region();
        let Some(tab) = region.group_tab(group) else {
            return;
        };
        if region.groups(tab).len() == 1 {
            self.close_tab(tab, cx);
            return;
        }
        self.close_flushing(CloseTarget::Group(group), vec![group], cx);
    }

    /// Whether the close of `tab` is under way: its groups are being
    /// written before the tab goes. A group added to it now would go with
    /// it, so nothing is added ([`TAB_CLOSING`]).
    pub(crate) fn tab_mid_close(&self, tab: TabId) -> bool {
        matches!(self.groups_closing, Some((CloseTarget::Tab(t), _)) if t == tab)
    }

    /// Write the unsaved text of `groups` (all at once, as the window's
    /// close does), then take `target` away. Refused, with a flash, while
    /// one of their notes cannot be let go.
    fn close_flushing(
        &mut self,
        target: CloseTarget,
        groups: Vec<GroupId>,
        cx: &mut Context<Self>,
    ) {
        let noun = target.noun();
        let refusal = groups.iter().find_map(|group| {
            self.flush_block_in(*group, cx).map(|block| match block {
                FlushBlock::Closing => None,
                FlushBlock::Opening => Some(match target {
                    CloseTarget::Tab(_) => {
                        "A note in this tab is still opening. Try again in a moment.".to_string()
                    }
                    CloseTarget::Group(_) => {
                        "The note in this group is still opening. Try again in a moment."
                            .to_string()
                    }
                }),
                FlushBlock::Held => Some(format!(
                    "Another operation on the note is still running. The {noun} stays open."
                )),
                FlushBlock::Unsaveable => Some(format!(
                    "{} The {noun} stays open.",
                    self.unsaveable_in(*group)
                )),
            })
        });
        match refusal {
            Some(Some(text)) => {
                self.flash_error(text, cx);
                return;
            }
            Some(None) => return,
            None => {}
        }
        let saving: Vec<GroupId> = groups
            .into_iter()
            .filter(|g| self.can_save_in(*g))
            .collect();
        let mut flushes = Vec::new();
        for group in saving {
            match self.begin_note_flush_in(group, cx) {
                Some(request) => flushes.push((group, request)),
                None => {
                    // Its tick is back already; the groups begun before it
                    // get theirs and their input back, their flushes
                    // given back unrun.
                    for (begun, flush) in flushes {
                        self.give_back_flush_in(begun, flush);
                        self.restart_tick_unless_read_only_in(begun);
                        self.block_input_in(begun, false, cx);
                    }
                    self.flash_error(
                        format!("Another save is still running. The {noun} stays open."),
                        cx,
                    );
                    return;
                }
            }
        }
        if flushes.is_empty() {
            self.remove_closed(target, cx);
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            let flushed = run_all(flushes).await;
            this.update(cx, |workspace, cx| {
                workspace.groups_flushed(target, flushed, cx)
            })
            .ok();
        });
        self.groups_closing = Some((target, task));
        cx.notify();
    }

    /// The flushes before a close ended: take `target` away when everything
    /// is written, else keep it (each note keeps its edits, its tick and
    /// the keyboard) and say why.
    fn groups_flushed(
        &mut self,
        target: CloseTarget,
        flushed: Vec<(GroupId, Result<FlushResult, JoinError>)>,
        cx: &mut Context<Self>,
    ) {
        self.groups_closing = None;
        let touched: Vec<GroupId> = flushed.iter().map(|(group, _)| *group).collect();
        let mut failure = None;
        for (group, result) in flushed {
            let outcome = self.apply_flush(group, result, cx);
            if failure.is_none()
                && let Some(problem) = self.flush_failure_in(group, &outcome, cx)
            {
                failure = Some((group, problem));
            }
        }
        for group in &touched {
            self.block_input_in(*group, false, cx);
        }
        if let Some((group, problem)) = failure {
            for group in &touched {
                self.restart_tick_unless_read_only_in(*group);
            }
            let note = self
                .state(group)
                .and_then(|s| s.notes.as_ref())
                .and_then(|h| h.note.clone())
                .unwrap_or_else(VaultPath::root);
            self.flash_error(
                format!(
                    "Couldn't save {note}: {problem}. The {} stays open.",
                    target.noun()
                ),
                cx,
            );
        } else {
            self.remove_closed(target, cx);
        }
        // Opens parked while the notes were held go on.
        self.file_op_ended(cx);
    }

    /// Take `target` away, its notes written.
    fn remove_closed(&mut self, target: CloseTarget, cx: &mut Context<Self>) {
        match target {
            CloseTarget::Tab(tab) => self.remove_tab(tab, cx),
            CloseTarget::Group(group) => {
                let shown = self.layout.region().shown();
                self.take_group(group, cx);
                self.groups_removed(shown, cx);
            }
        }
    }

    /// Take `tab`'s groups away, last first, by the region's rules: the
    /// tab goes with its last group, except the region's last tab, whose
    /// first group stays as an empty group. Nothing is written here: the
    /// caller flushed them.
    fn remove_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
        let shown = self.layout.region().shown();
        let mut groups = self.layout.region().groups(tab);
        groups.reverse();
        for group in groups {
            self.take_group(group, cx);
        }
        self.groups_removed(shown, cx);
    }

    /// Take `group` out of the region ([`EditorRegion::close_group`]): its
    /// state goes, its autosave stopped; a group the region keeps (the last
    /// of the last tab) is emptied instead. Already flushed.
    ///
    /// [`EditorRegion::close_group`]: kimun_notes::logic::editor_region::EditorRegion::close_group
    fn take_group(&mut self, group: GroupId, cx: &mut Context<Self>) {
        self.forget_note_menu_of(group, cx);
        self.group_bounds.borrow_mut().remove(&group);
        match self.layout.region_mut().close_group(group) {
            Some(CloseOutcome::Emptied) => {
                self.forget_attachment_request(group);
                self.drop_attachment_in(group);
                self.release_note_in(group, cx);
            }
            Some(CloseOutcome::Group | CloseOutcome::Tab(_)) => {
                if let Some(mut state) = self.groups.remove(&group)
                    && let Some(host) = state.notes.as_mut()
                {
                    host.session.stop_autosave();
                    host.session.abort_autosave();
                }
                self.forget_note_gone(group);
            }
            None => {}
        }
    }

    /// Groups were taken away; `shown` was the shown tab before. The window
    /// follows the active group now (a zoom ends when another tab shows);
    /// the keyboard stays where it was.
    fn groups_removed(&mut self, shown: TabId, cx: &mut Context<Self>) {
        if self.layout.region().shown() != shown {
            self.layout.zoom(None);
        }
        self.sync_active_status(cx);
        self.refresh_note_counts();
        self.focus_shown_content(cx);
        self.scroll_to_shown_tab();
        cx.notify();
    }

    // ---- Drawing -------------------------------------------------------------

    /// The strip over the shown tab, or `None` while the region holds one
    /// tab and no group is dragged (ADR-0058). A row of pill buttons that
    /// scrolls sideways, and the round plus button after it, outside the
    /// scroll so it always shows.
    pub(super) fn render_tab_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let region = self.layout.region();
        let tabs = region.tabs();
        if tabs.len() < 2 && self.dragged_group(cx).is_none() {
            return None;
        }
        let shown = region.shown();
        let labels: Vec<AnyElement> = tabs
            .iter()
            .enumerate()
            .map(|(at, tab)| self.render_tab(at, *tab, *tab == shown, cx))
            .collect();
        let theme = theme::current(cx);
        let hover = theme::hover_background(&theme);
        let tip: SharedString = match keymap::chord_label(cx, &ActionShortcuts::NewTab) {
            Some(chord) => format!("New tab ({chord})").into(),
            None => "New tab".into(),
        };
        let new_tab = div()
            .id(TAB_NEW_ID)
            .test_support()
            .role(Role::Button)
            .aria_label("New tab")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .relative()
            .size(rems(PILL_HEIGHT_REMS))
            .rounded_full()
            .text_color(theme::muted(&theme))
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(Icon::new(NEW_ICON).size_4())
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.new_tab(cx)))
            // During a group's drag: the group goes to a new tab. The zone
            // follows the round button.
            .children(
                self.drop_zone(TAB_NEW_DROP_ID, DropTarget::NewTab, cx)
                    .map(|zone| zone.inset_0().rounded_full().test_support()),
            );
        Some(
            div()
                .id(TAB_STRIP_ID)
                .test_support()
                .role(Role::TabList)
                .aria_label("Tabs")
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .gap_1()
                .w_full()
                .px_1()
                .py_1()
                .bg(theme::panel_background(&theme))
                .border_b_1()
                .border_color(theme::divider(&theme))
                .child(
                    div()
                        .id(TAB_LABELS_ID)
                        .test_support()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .flex_1()
                        .min_w_0()
                        .overflow_x_scroll()
                        .track_scroll(&self.tab_scroll)
                        .children(labels),
                )
                .child(new_tab)
                .into_any_element(),
        )
    }

    /// What the trailing slot of `tab`'s pill holds (see [`Trail`]).
    pub(crate) fn tab_trail(&self, tab: TabId, shown: bool, cx: &App) -> Trail {
        if self.tab_hover == Some(tab) {
            Trail::Close
        } else if tab_dirty(self, tab, cx) {
            Trail::Unsaved
        } else if shown {
            Trail::Close
        } else {
            Trail::Nothing
        }
    }

    /// The pointer entered or left `tab`'s pill.
    fn hover_tab(&mut self, tab: TabId, hovered: bool, cx: &mut Context<Self>) {
        let now = match (hovered, self.tab_hover) {
            (true, _) => Some(tab),
            (false, Some(t)) if t == tab => None,
            (false, other) => other,
        };
        if now != self.tab_hover {
            self.tab_hover = now;
            cx.notify();
        }
    }

    /// The pill of `tab`, at `at` in the strip. The shown tab is filled
    /// with the selected tint (the rail's shown view uses it) and reads in
    /// the foreground colour; the others have no fill, muted text, and the
    /// hover fill under the pointer. One trailing slot of a fixed size
    /// holds the close button, the unsaved dot or nothing, so a pill never
    /// changes width as the pointer moves.
    fn render_tab(&self, at: usize, tab: TabId, shown: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let label = tab_label(self, tab, cx);
        let dirty = tab_dirty(self, tab, cx);
        let hover = theme::hover_background(&theme);
        let spoken: SharedString = if dirty {
            format!("{label}, unsaved").into()
        } else {
            label.clone()
        };
        let trail_content: Option<AnyElement> = match self.tab_trail(tab, shown, cx) {
            Trail::Close => {
                let close_tip: SharedString =
                    match keymap::chord_label(cx, &ActionShortcuts::CloseTab) {
                        Some(chord) => format!("Close tab ({chord})").into(),
                        None => "Close tab".into(),
                    };
                Some(
                    div()
                        .id(ElementId::NamedInteger(TAB_CLOSE_ID.into(), at as u64))
                        .test_support()
                        .role(Role::Button)
                        .aria_label("Close tab")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size_full()
                        .rounded_full()
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .child(Icon::new(CLOSE_ICON).size_3())
                        .tooltip(move |window, cx| {
                            Tooltip::new(close_tip.clone()).build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.close_tab(tab, cx);
                        }))
                        .into_any_element(),
                )
            }
            Trail::Unsaved => Some(
                div()
                    .id(ElementId::NamedInteger(TAB_UNSAVED_ID.into(), at as u64))
                    .test_support()
                    .size(rems(0.375))
                    .rounded_full()
                    .bg(theme::unsaved(&theme))
                    .into_any_element(),
            ),
            Trail::Nothing => None,
        };
        let trail = div()
            .id(tab_trail_id(at))
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rems(TRAIL_REMS))
            .children(trail_content);
        div()
            .id(tab_label_id(at))
            .test_support()
            .role(Role::Tab)
            .aria_label(spoken)
            .aria_selected(shown)
            .relative()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap_1()
            .h(rems(PILL_HEIGHT_REMS))
            .max_w(rems(LABEL_MAX_REMS))
            .pl_3()
            .pr_1()
            .rounded_full()
            .text_sm()
            .whitespace_nowrap()
            .cursor_pointer()
            .when(shown, |pill| {
                pill.bg(theme::selected_background(&theme))
                    .text_color(theme::foreground(&theme))
            })
            .when(!shown, |pill| {
                pill.text_color(theme::muted(&theme))
                    .hover(move |style| style.bg(hover))
            })
            .on_hover(
                cx.listener(move |this, hovered: &bool, _: &mut Window, cx| {
                    this.hover_tab(tab, *hovered, cx)
                }),
            )
            .on_click(
                cx.listener(move |this, _: &ClickEvent, _: &mut Window, cx| this.show_tab(tab, cx)),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(label),
            )
            .child(trail)
            // During a group's drag: the group joins this tab. The zone
            // covers the whole pill and follows its round ends.
            .children(
                self.drop_zone(tab_drop_id(at), DropTarget::Tab(tab), cx)
                    .map(|zone| zone.inset_0().rounded_full().test_support()),
            )
            .into_any_element()
    }
}
