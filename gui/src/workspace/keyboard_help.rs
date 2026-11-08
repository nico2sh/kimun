//! The **keyboard reference** (M10d): every chord the window answers to and
//! every leader sequence, read-only, filtered as you type (F1, the palette's
//! `+help · keyboard shortcuts`, leader `?`, the CFG view's `Keyboard
//! shortcuts…`). A kit `Dialog` in the window's one modal slot, like the
//! theme picker.
//!
//! - **Rows.** `logic::keyboard_help`: each shared action with the chords
//!   the *active* keymap binds it to (the `[gui.keys]` overrides and the
//!   platform's chords included), under its heading (`ActionShortcuts::group`,
//!   defined next to the actions); then the configured leader tree's
//!   sequences under "Leader tree", whose heading names the gateway once
//!   (`Ctrl+Shift+B`, and `Space` in vim Normal mode when the editor runs
//!   vim). An action the desktop app binds by default but the overrides
//!   unbound is listed under "Unbound", saying "not bound"; an action the app
//!   ships without a chord (formatting, which lives on the leader) is not
//!   listed. Where the vim rule keeps a chord in the editor the row says so;
//!   a leader leaf with no surface yet says "Coming soon" or "Not in the
//!   desktop app yet". Built once, when it opens: everything is in memory.
//! - **Filter.** Fuzzy over label and keys (the shared list engine's
//!   `ListCore`, its rank kept under the headings); "Nothing matches." when
//!   nothing does.
//! - **Keys.** Typing filters; Up/Down move the selection (the list scrolls
//!   to it); Enter does nothing (the reference runs nothing); Escape, the
//!   reference's own chord again, a click outside or the kit's close button
//!   (help to read keeps it, as the query syntax does) close it. A
//!   click selects a row ([`ClickPolicy::Select`]); a double-click does
//!   nothing either.
//! - **Where it opens.** In every window state, a vault or none; never over
//!   the setup assistant, Preferences or another modal (the theme picker's
//!   rule).

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, UniformListScrollHandle,
    WeakEntity, Window, div, rems, uniform_list,
};
use kimun_notes::logic::actions::{ActionGroup, ActionShortcuts};
use kimun_notes::logic::keyboard_help::{self, HelpRow, HelpTarget, leader_rows};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::list::{Filter, ListCore, ListSetup, StaticRowSource};

use super::Workspace;
use super::files::{ClickPolicy, ClickRows, ellipsis, row_frame, with_click_cursor};
use super::leader::SPACE_GATEWAY;
use super::modal::{self, ModalKind, Slot};
use super::note::Closing;
use super::route::{NOT_YET, SOON};
use crate::keymap;
use crate::theme;

/// Element ids, for tests.
pub(crate) const KEYBOARD_HELP_ID: &str = "keyboard-help";
pub(crate) const KEYBOARD_HELP_INPUT_ID: &str = "keyboard-help-input";
pub(crate) const KEYBOARD_HELP_LIST_ID: &str = "keyboard-help-list";
pub(crate) const KEYBOARD_HELP_EMPTY_ID: &str = "keyboard-help-empty";

/// The dialog's title.
pub(crate) const TITLE: &str = "Keyboard shortcuts";
/// What it says when the filter matches nothing.
pub(crate) const EMPTY_TEXT: &str = "Nothing matches.";
/// What a row says after a chord the vim editor keeps.
const VIM_NOTE: &str = "not in the vim editor";
/// What the query syntax's row says: where its chord is live (elsewhere the
/// same F1 is this reference's).
pub(crate) const QUERY_NOTE: &str = "in FIND and search";

/// A row's element id: the action's config name, or the leader sequence
/// (unique in the tree; one leaf may sit under two sequences).
pub(crate) fn help_row_id(row: &HelpRow) -> SharedString {
    match &row.target {
        HelpTarget::Action(action) => format!("keyboard-help:action:{action}").into(),
        HelpTarget::Leaf(_) => format!("keyboard-help:leader:{}", row.keys).into(),
    }
}

/// A heading's element id.
pub(crate) fn help_heading_id(group: ActionGroup) -> SharedString {
    format!("keyboard-help:heading:{}", group.title()).into()
}

/// The dialog's width, in rems.
const DIALOG_REMS: f32 = 38.;
/// The list's height, in rems: about twelve lines.
const LIST_REMS: f32 = 24.;
/// A line's height (row or heading), in rems.
const ROW_REMS: f32 = 2.;

/// What the desktop app calls an action where the TUI's own label would
/// mislead (the TUI's directional focus is the window's panel cycle here).
fn action_label(action: &ActionShortcuts) -> String {
    use ActionShortcuts as A;
    match action {
        A::ToggleSidebar => "Toggle left side".into(),
        A::ToggleRightDrawer => "Toggle right side".into(),
        A::MoveViewToLeftDrawer => "Move view to left slot".into(),
        A::MoveViewToRightDrawer => "Move view to right slot".into(),
        A::SplitDrawerSlot => "Split slot".into(),
        A::UnsplitDrawerSlot => "Close slot".into(),
        A::FocusEditor => "Next panel".into(),
        A::FocusSidebar => "Previous panel".into(),
        A::Leader => "Leader (start a sequence)".into(),
        other => other.label(),
    }
}

/// One line of the list: a heading, or the row at a visible position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    Heading(ActionGroup),
    Row(usize),
}

/// The reference's view. See the module docs.
pub(crate) struct KeyboardHelp {
    /// The rows, the filter and the selection: the shared list engine.
    list: ListCore<HelpRow>,
    /// The visible rows with their headings, top first.
    lines: Vec<Line>,
    pub(crate) input: Entity<InputState>,
    scroll: UniformListScrollHandle,
    /// The "Leader tree" heading's gateway text.
    gateway: SharedString,
    workspace: WeakEntity<Workspace>,
    _input_events: Subscription,
}

impl KeyboardHelp {
    fn new(
        rows: Vec<HelpRow>,
        gateway: SharedString,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter shortcuts"));
        let input_events = cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.set_query(query, cx);
            }
        });
        let list = ListCore::with_rows(
            Arc::new(StaticRowSource),
            Arc::new(|| {}),
            ListSetup {
                filter: Filter::Rank(Arc::new(|rows: &[HelpRow], _: &[usize], query: &str| {
                    keyboard_help::rank(rows, query)
                })),
                ..ListSetup::default()
            },
            rows,
        );
        let mut help = Self {
            list,
            lines: Vec::new(),
            input,
            scroll: UniformListScrollHandle::new(),
            gateway,
            workspace,
            _input_events: input_events,
        };
        help.relayout();
        help
    }

    // ---- reading (the tests) ----------------------------------------------

    /// The rows shown, top first.
    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<HelpRow> {
        self.list.visible_rows().into_iter().cloned().collect()
    }

    /// The headings shown, top first.
    #[cfg(test)]
    pub(crate) fn headings(&self) -> Vec<ActionGroup> {
        self.lines
            .iter()
            .filter_map(|line| match line {
                Line::Heading(group) => Some(*group),
                Line::Row(_) => None,
            })
            .collect()
    }

    /// The selected row.
    #[cfg(test)]
    pub(crate) fn selected(&self) -> Option<HelpRow> {
        self.list.selected_row().cloned()
    }

    /// The "Leader tree" heading's gateway text.
    #[cfg(test)]
    pub(crate) fn gateway(&self) -> &str {
        &self.gateway
    }

    // ---- behaviour --------------------------------------------------------

    /// Rebuild the lines from the visible rows (a heading before each new
    /// group) and put the selection on the first row.
    fn relayout(&mut self) {
        let mut lines = Vec::new();
        let mut last = None;
        for (pos, row) in self.list.visible_rows().into_iter().enumerate() {
            if last != Some(row.group) {
                lines.push(Line::Heading(row.group));
                last = Some(row.group);
            }
            lines.push(Line::Row(pos));
        }
        self.lines = lines;
        if self.list.visible_len() > 0 {
            self.list.select(0);
        }
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.list.set_query(query);
        self.relayout();
        cx.notify();
    }

    /// The line the selected row is on.
    fn selected_line(&self) -> Option<usize> {
        let pos = self.list.selected()?;
        self.lines.iter().position(|line| *line == Line::Row(pos))
    }

    /// Move the selection one row down (`true`) or up, the list following.
    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if down {
            self.list.select_next();
        } else {
            self.list.select_prev();
        }
        // The first row's heading comes into view with it.
        let line = self.selected_line().map(|line| {
            if self.list.selected() == Some(0) {
                0
            } else {
                line
            }
        });
        if let Some(line) = line {
            self.scroll.scroll_to_item(line, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    /// Escape, a click outside: close.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.close_modal(window, cx))
            .ok();
    }

    fn render_heading(&self, group: ActionGroup, cx: &gpui_kit::App) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let text: SharedString = if group == ActionGroup::LeaderTree {
            format!("{} · {}", group.title(), self.gateway).into()
        } else {
            group.title().into()
        };
        div()
            .id(help_heading_id(group))
            .test_support()
            .role(Role::Heading)
            .aria_label(text.clone())
            .flex()
            .items_end()
            .w_full()
            .h(rems(ROW_REMS))
            .px_2()
            .pb_1()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::muted(t))
            .child(ellipsis(text))
            .into_any_element()
    }

    /// The row at visible position `pos`, borrowed from the list (nothing
    /// is cloned but the strings the elements keep). `None` past the end.
    fn render_row(&self, pos: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = self.list.visible_row(pos)?;
        let theme = theme::current(cx);
        let t = &*theme;
        let mono = cx.theme().mono_font_family.clone();
        let label: SharedString = match &row.note {
            Some(note) => format!("{}, {}, {note}", row.label, row.keys).into(),
            None => format!("{}, {}", row.label, row.keys).into(),
        };
        let frame = row_frame(
            help_row_id(row),
            label,
            self.list.selected() == Some(pos),
            ROW_REMS,
            t,
        )
        .gap_3()
        .text_sm()
        .child(
            ellipsis(row.label.clone())
                .flex_1()
                .text_color(theme::foreground(t)),
        )
        .children(row.note.clone().map(|note| {
            div()
                .flex_none()
                .text_xs()
                .text_color(theme::muted(t))
                .child(note)
        }))
        .child(
            div()
                .flex_none()
                .max_w(rems(16.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .font_family(mono)
                .text_color(theme::bright(t))
                .child(row.keys.clone()),
        );
        Some(
            with_click_cursor(frame, ClickPolicy::Select, true)
                .test_support()
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    ClickPolicy::Select.apply(event, this, pos, window, cx)
                }))
                .into_any_element(),
        )
    }
}

/// A click selects; the reference runs nothing, so a double-click is a
/// selection too.
impl ClickRows for KeyboardHelp {
    fn select_row(&mut self, pos: usize, cx: &mut Context<Self>) {
        self.list.select(pos);
        cx.notify();
    }

    fn activate_row(&mut self, _: &mut Window, _: &mut Context<Self>) {}
}

impl Render for KeyboardHelp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let count = self.lines.len();
        let body = if count > 0 {
            uniform_list(
                KEYBOARD_HELP_LIST_ID,
                count,
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .filter_map(|ix| match *this.lines.get(ix)? {
                            Line::Heading(group) => Some(this.render_heading(group, cx)),
                            Line::Row(pos) => this.render_row(pos, cx),
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full()
            .into_any_element()
        } else {
            div()
                .id(KEYBOARD_HELP_EMPTY_ID)
                .test_support()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(theme::muted(t))
                .child(EMPTY_TEXT)
                .into_any_element()
        };
        let rows = div()
            .id("keyboard-help-rows")
            .role(Role::ListBox)
            .aria_label("Keyboard shortcuts")
            .h(rems(LIST_REMS))
            .child(body);
        div()
            .id(KEYBOARD_HELP_ID)
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            // The field's keys that mean something else here, taken before
            // the field sees them.
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.step(false, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.step(true, cx);
                cx.stop_propagation();
            }))
            // Read-only: Enter runs nothing.
            .capture_action(cx.listener(|_, _: &Enter, _, cx| {
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                this.close(window, cx);
                cx.stop_propagation();
            }))
            .child(
                Input::new(&self.input)
                    .id(KEYBOARD_HELP_INPUT_ID)
                    .aria_label("Filter shortcuts"),
            )
            .child(rows)
    }
}

// ---- The workspace's side ----------------------------------------------------------

impl Workspace {
    /// Every row of the reference, as the window is now: the active keymap's
    /// chords, the configured leader tree, which leaves have no surface yet.
    pub(crate) fn keyboard_help_rows(&self, opener: Panel, cx: &gpui_kit::App) -> Vec<HelpRow> {
        let keymap = keymap::active(cx);
        let vim = self.editor.read(cx).editor.is_vim();
        let mut rows: Vec<HelpRow> = ActionShortcuts::ALL
            .into_iter()
            .filter_map(|action| {
                let chords = keymap.chords_for(&action);
                // Shipped without a chord (formatting, the query drawer):
                // not the reference's to list.
                if chords.is_empty() && !keymap.ships_bound(&action) {
                    return None;
                }
                let labels: Vec<String> = chords.iter().map(|c| keymap::format_chord(*c)).collect();
                let shadowed = vim
                    && chords
                        .iter()
                        .any(|chord| keymap.vim_shadows(&action, *chord));
                let row = HelpRow::action(action.clone(), action_label(&action), &labels);
                Some(if shadowed {
                    row.with_note(VIM_NOTE)
                } else if action == ActionShortcuts::QuerySyntax && !labels.is_empty() {
                    row.with_note(QUERY_NOTE)
                } else {
                    row
                })
            })
            .collect();
        rows.extend(leader_rows(self.leader.tree()).into_iter().map(|row| {
            let HelpTarget::Leaf(action) = row.target else {
                return row;
            };
            match self.route_block(&self.route_of(action), opener, cx) {
                Some(reason @ (SOON | NOT_YET)) => row.with_note(reason),
                _ => row,
            }
        }));
        // The headings' order; each heading's rows keep theirs.
        rows.sort_by_key(|row| row.group);
        rows
    }

    /// The "Leader tree" heading's gateway: the `Leader` chord, and a bare
    /// Space in vim Normal mode when the editor runs vim.
    fn keyboard_help_gateway(&self, cx: &gpui_kit::App) -> SharedString {
        let chord = keymap::chord_label(cx, &ActionShortcuts::Leader);
        let vim = self.editor.read(cx).editor.is_vim();
        match (chord, vim) {
            (Some(chord), true) => format!("{chord}, or {SPACE_GATEWAY} in vim Normal mode").into(),
            (Some(chord), false) => chord.into(),
            (None, true) => format!("{SPACE_GATEWAY} in vim Normal mode").into(),
            (None, false) => "the Leader chord is not bound".into(),
        }
    }

    /// F1, the palette, leader `?`, the CFG view: open the reference over the
    /// window, or close it when it is the open modal. Refused under the
    /// setup assistant, Preferences, another modal or the window's close.
    pub(crate) fn open_keyboard_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.onboarding_open() || self.preferences_open() || self.closing != Closing::Open {
            return;
        }
        if self.open_modal(window, cx) == Some(ModalKind::KeyboardHelp) {
            self.close_modal(window, cx);
            return;
        }
        if modal::dialog_open(window, cx) {
            return;
        }
        let rows = self.keyboard_help_rows(self.focused_panel(), cx);
        let gateway = self.keyboard_help_gateway(cx);
        let workspace = cx.entity().downgrade();
        let view = cx.new(|cx| KeyboardHelp::new(rows, gateway, workspace, window, cx));
        let input = view.read(cx).input.clone();
        self.show_modal(
            Slot::Own(ModalKind::KeyboardHelp),
            DIALOG_REMS,
            move |window, cx| {
                let handle = gpui_kit::Focusable::focus_handle(input.read(cx), cx);
                window.focus(&handle, cx);
            },
            move |dialog, _, _| {
                let cancel = view.downgrade();
                dialog
                    .title(TITLE)
                    // Help to read, nothing to confirm: the kit's close
                    // button stays (the query syntax's rule).
                    .close_button(true)
                    // Escape and a click outside.
                    .on_cancel(move |_, window, cx| {
                        cancel.update(cx, |help, cx| help.close(window, cx)).ok();
                        false
                    })
                    .child(view.clone())
            },
            window,
            cx,
        );
    }
}
