//! The **tab strip** (ADR-0058): a row of labels over the shown tab, one
//! per tab, with a plus button that opens a tab. Hidden while the region
//! holds one tab, so a window that never opens a second tab looks as it
//! always did. A label names the note of its tab's active group
//! ([`tab_label`]) and carries the Footer's unsaved dot while any group of
//! the tab has unsaved text ([`tab_dirty`]). A click on a label shows its
//! tab; its close button closes it. The strip scrolls sideways when the
//! labels do not fit, and keeps the shown tab in view.
//!
//! The tab commands live here too: a new tab (one empty group, which takes
//! the keyboard), the next and previous tab (wrapping), and the close. A
//! close writes the unsaved text of every group of the tab first (all at
//! once, as the window's close does); when a write fails or cannot start
//! the tab stays and the flash says why. Closing the last tab leaves one
//! empty group: the region never has zero tabs or zero groups.

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
use super::file_ops::run_all;
use super::note::Closing;
use super::note_flush::FlushBlock;
use crate::{keymap, theme};

/// The strip's element id.
pub(crate) const TAB_STRIP_ID: &str = "tab-strip";
/// The plus button's element id.
pub(crate) const TAB_NEW_ID: &str = "tab-strip-new";
/// The unsaved dot's id, with the tab's position.
pub(crate) const TAB_UNSAVED_ID: &str = "tab-unsaved";
/// A close button's id, with the tab's position.
pub(crate) const TAB_CLOSE_ID: &str = "tab-close";
/// What a tab whose active group shows nothing is called.
pub(crate) const EMPTY_LABEL: &str = "Empty";
/// A tab's close glyph: the kit's own close icon (its dialogs and toasts
/// draw it), carried by the default bundle. Lucide's `x` is not bundled.
const CLOSE_ICON: IconName = IconName::Close;
/// The plus button's glyph.
const NEW_ICON: IconName = IconName::Plus;
/// Every glyph the strip draws, for the bundle test: an icon the asset
/// source does not carry paints nothing.
#[cfg(test)]
pub(crate) const ICONS: [IconName; 2] = [CLOSE_ICON, NEW_ICON];
/// The widest a label grows before its name is cut with an ellipsis.
const LABEL_MAX_REMS: f32 = 14.;

/// The element id of the label at `index` (its position in the strip).
pub(crate) fn tab_label_id(index: usize) -> ElementId {
    ElementId::NamedInteger("tab-label".into(), index as u64)
}

/// What `group` shows, named as the Footer names it: the attachment in
/// place of the editor, else the note. `None` for an empty group.
fn group_name(workspace: &Workspace, group: GroupId, cx: &App) -> Option<String> {
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
    /// its editor with the keyboard (the empty group offers the finder).
    /// Nothing without a vault, or while the window closes.
    pub(crate) fn new_tab(&mut self, cx: &mut Context<Self>) {
        if matches!(self.closing, Closing::Requested | Closing::Flushing) {
            return;
        }
        let Some(state) = self.new_group_state(cx) else {
            return;
        };
        let (_, group) = self.layout.region_mut().new_tab();
        self.insert_group(group, state, cx);
        self.focus_group(group, cx);
        self.scroll_to_shown_tab();
    }

    /// `NextTab` / `PrevTab`: show the next (previous) tab, wrapping; its
    /// active group takes the keyboard. Nothing with one tab.
    pub(crate) fn step_tab(&mut self, forward: bool, cx: &mut Context<Self>) {
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
    /// keyboard; the strip scrolls to its label.
    fn show_active_group(&mut self, cx: &mut Context<Self>) {
        let group = self.layout.active_group();
        self.focus_group(group, cx);
        self.scroll_to_shown_tab();
    }

    /// Bring the shown tab's label into the strip's view.
    fn scroll_to_shown_tab(&self) {
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
        if self.tab_closing.is_some() || !self.layout.region().tabs().contains(&tab) {
            return;
        }
        let groups = self.layout.region().groups(tab);
        let refusal = groups.iter().find_map(|group| {
            self.flush_block_in(*group, cx).map(|block| match block {
                FlushBlock::Closing => None,
                FlushBlock::Opening => {
                    Some("A note in this tab is still opening. Try again in a moment.".to_string())
                }
                FlushBlock::Held => Some(
                    "Another operation on the note is still running. The tab stays open."
                        .to_string(),
                ),
                FlushBlock::Unsaveable => Some(format!(
                    "{} The tab stays open.",
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
                    // dropped unrun.
                    for (begun, _) in flushes {
                        self.restart_tick_unless_read_only_in(begun);
                        self.block_input_in(begun, false, cx);
                    }
                    self.flash_error("Another save is still running. The tab stays open.", cx);
                    return;
                }
            }
        }
        if flushes.is_empty() {
            self.remove_tab(tab, cx);
            return;
        }
        self.tab_closing = Some(cx.spawn(async move |this, cx| {
            let flushed = run_all(flushes).await;
            this.update(cx, |workspace, cx| workspace.tab_flushed(tab, flushed, cx))
                .ok();
        }));
        cx.notify();
    }

    /// The flushes before a tab closes ended: close it when everything is
    /// written, else keep it (each note keeps its edits, its tick and the
    /// keyboard) and say why.
    fn tab_flushed(
        &mut self,
        tab: TabId,
        flushed: Vec<(GroupId, Result<FlushResult, JoinError>)>,
        cx: &mut Context<Self>,
    ) {
        self.tab_closing = None;
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
                format!("Couldn't save {note}: {problem}. The tab stays open."),
                cx,
            );
        } else {
            self.remove_tab(tab, cx);
        }
        // Opens parked while the notes were held go on.
        self.file_op_ended(cx);
    }

    /// Take `tab`'s groups away, last first, by the region's rules: the
    /// tab goes with its last group, except the region's last tab, whose
    /// first group stays as an empty group. Nothing is written here: the
    /// caller flushed them.
    fn remove_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
        let mut groups = self.layout.region().groups(tab);
        groups.reverse();
        for group in groups {
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
                }
                None => {}
            }
        }
        // The tab shown now (or the emptied one): the window follows its
        // active group; the keyboard stays where it was.
        self.sync_active_status(cx);
        self.refresh_note_counts();
        self.focus_shown_content(cx);
        self.scroll_to_shown_tab();
        cx.notify();
    }

    // ---- Drawing -------------------------------------------------------------

    /// The strip over the shown tab, or `None` while the region holds one
    /// tab (ADR-0058).
    pub(super) fn render_tab_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let region = self.layout.region();
        let tabs = region.tabs();
        if tabs.len() < 2 {
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
            .size(rems(1.5))
            .mx_1()
            .rounded_md()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(
                Icon::new(NEW_ICON)
                    .size_4()
                    .text_color(theme::muted(&theme)),
            )
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.new_tab(cx)));
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
                .w_full()
                .bg(theme::panel_background(&theme))
                .border_b_1()
                .border_color(theme::divider(&theme))
                .child(
                    div()
                        .id("tab-strip-labels")
                        .test_support()
                        .flex()
                        .flex_row()
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

    /// The label of `tab`, at `at` in the strip. The shown tab sits on the
    /// page's colour with a bar in the focus colour along its top; the
    /// others are muted and light up under the pointer.
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
        let close_tip: SharedString = match keymap::chord_label(cx, &ActionShortcuts::CloseTab) {
            Some(chord) => format!("Close tab ({chord})").into(),
            None => "Close tab".into(),
        };
        let close = div()
            .id(ElementId::NamedInteger(TAB_CLOSE_ID.into(), at as u64))
            .test_support()
            .role(Role::Button)
            .aria_label("Close tab")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rems(1.125))
            .rounded_sm()
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .child(Icon::new(CLOSE_ICON).size_3())
            .tooltip(move |window, cx| Tooltip::new(close_tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.close_tab(tab, cx);
            }));
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
            .h(rems(1.875))
            .max_w(rems(LABEL_MAX_REMS))
            .pl_3()
            .pr_1()
            .text_sm()
            .whitespace_nowrap()
            .cursor_pointer()
            .border_r_1()
            .border_color(theme::divider(&theme))
            .when(shown, |row| {
                row.bg(theme::background(&theme))
                    .text_color(theme::foreground(&theme))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h_0p5()
                            .bg(theme::focus_border(&theme)),
                    )
            })
            .when(!shown, |row| {
                row.text_color(theme::muted(&theme))
                    .hover(move |style| style.bg(hover))
            })
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
            .when(dirty, |row| {
                row.child(
                    div()
                        .id(ElementId::NamedInteger(TAB_UNSAVED_ID.into(), at as u64))
                        .test_support()
                        .flex_none()
                        .size(rems(0.375))
                        .rounded_full()
                        .bg(theme::unsaved(&theme)),
                )
            })
            .child(close)
            .into_any_element()
    }
}
