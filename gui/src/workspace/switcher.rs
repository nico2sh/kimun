//! The **Workspace switcher** (M8d): every configured workspace, by name,
//! and the way to move the window onto another one (`SwitchWorkspace`, F5;
//! leader `v s`; the palette's `+vault · switch vault`). A kit `Dialog` in
//! the window's one modal slot, with no query input: workspace lists are
//! short.
//!
//! - **Rows** are the shared settings' workspaces
//!   (`logic::workspaces::workspace_rows`), read when it opens — never
//!   cached, so a workspace added since appears. The current one is marked
//!   (a dot in the lane and "current" after the name) and selected.
//! - **Picking.** Up/Down (and `j`/`k`) move the selection, clamped at the
//!   ends like every shared list (the TUI wraps); Enter, the `Switch`
//!   button or a single click ([`ClickPolicy::Open`]) picks a row. The
//!   dialog closes first; picking the current workspace does nothing more,
//!   picking another runs [`Workspace::switch_workspace`] (M8b), which
//!   persists it and reopens the window on it — or refuses with an error
//!   flash, the old vault still working. The wheel moves the selection.
//! - **No workspace configured.** It says so and offers the setup assistant
//!   (`Run guided setup…`, also Enter), which closes it first.
//! - **Where it opens.** In every window state (a vault, one that would not
//!   open), never over the setup assistant or another modal; F5 again
//!   closes it (the modal host's toggle).
//!
//! Nothing here reads or writes a file: the rows come from the settings in
//! memory, and the switch is the workspace's.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, Role, ScrollEvent, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div,
};
use kimun_notes::logic::input::{Key, KeyInput};
use kimun_notes::logic::list::{ListCore, ListSetup, StaticRowSource};
use kimun_notes::logic::workspaces::{WorkspaceRow, initial_selection, workspace_rows};
use std::sync::Arc;

use super::Workspace;
use super::files::{ClickPolicy, ClickRows, NOTE_ROW_REMS, ellipsis, lane_row, with_click_cursor};
use super::modal::{self, ModalKind, Slot};
use super::pinned::WHEEL_STEP_REMS;
use crate::input;
use crate::theme;

/// Element ids, for tests.
pub(crate) const SWITCHER_ID: &str = "workspace-switcher";
pub(crate) const SWITCHER_EMPTY_ID: &str = "workspace-switcher-empty";
pub(crate) const SWITCHER_SWITCH_ID: &str = "workspace-switch";
pub(crate) const SWITCHER_SETUP_ID: &str = "workspace-setup";
pub(crate) const SWITCHER_CLOSE_ID: &str = "workspace-switcher-close";

/// What the dialog says with no workspace configured.
pub(crate) const EMPTY_TEXT: &str = "No workspaces configured.";

/// The dialog's title.
pub(crate) const TITLE: &str = "Switch workspace";

/// A workspace's row id, by its name.
pub(crate) fn workspace_row_id(name: &str) -> SharedString {
    format!("workspace-row:{name}").into()
}

/// The dialog's width, in rems.
const DIALOG_REMS: f32 = 32.;

/// The workspace switcher's view.
pub(crate) struct WorkspaceSwitcher {
    /// The workspaces and the selection: the shared list engine, its rows
    /// put in once, when the dialog opens.
    list: ListCore<WorkspaceRow>,
    /// Wheel travel not yet worth a row.
    wheel: input::ScrollAccumulator,
    focus: FocusHandle,
    workspace: WeakEntity<Workspace>,
}

impl WorkspaceSwitcher {
    fn new(
        rows: Vec<WorkspaceRow>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let start = initial_selection(&rows);
        let mut list = ListCore::with_rows(
            Arc::new(StaticRowSource),
            Arc::new(|| {}),
            ListSetup::default(),
            rows,
        );
        if let Some(start) = start {
            list.select(start);
        }
        Self {
            list,
            wheel: input::ScrollAccumulator::default(),
            focus: cx.focus_handle(),
            workspace,
        }
    }

    /// The rows shown, by name.
    pub(crate) fn rows(&self) -> &[WorkspaceRow] {
        self.list.rows()
    }

    /// The selected row's index (`None` on an empty list).
    pub(crate) fn selected(&self) -> Option<usize> {
        self.list.selected()
    }

    /// Enter, the `Switch` button, a click: pick the selected workspace
    /// (see the module docs). With no workspace, Enter runs the setup.
    fn pick_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.list.selected_row() else {
            if self.rows().is_empty() {
                self.run_setup(window, cx);
            }
            return;
        };
        let target = row.switch_target().map(str::to_string);
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.close_modal(window, cx);
                if let Some(name) = target {
                    workspace.switch_workspace(name, cx);
                }
            })
            .ok();
    }

    /// `Run guided setup…`: the dialog closes and the setup assistant opens.
    fn run_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.close_modal(window, cx);
                workspace.open_onboarding(window, cx);
            })
            .ok();
    }

    /// Close (the `Close` button; Escape is the kit's).
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.close_modal(window, cx))
            .ok();
    }

    /// Move the selection one row down (`true`) or up, within the list.
    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if down {
            self.list.select_next();
        } else {
            self.list.select_prev();
        }
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(KeyInput { key, mods }) = input::key_down_input(event) else {
            return;
        };
        match key {
            // A chord is not this dialog's: swallowed (it is modal).
            _ if mods.ctrl || mods.alt || mods.sup => {}
            Key::Up | Key::Char('k') => self.step(false, cx),
            Key::Down | Key::Char('j') => self.step(true, cx),
            // Enter is the dialog's `on_ok`; Escape and Tab are the kit's.
            Key::Enter | Key::Esc | Key::Tab | Key::BackTab => return,
            _ => {}
        }
        cx.stop_propagation();
    }

    /// The wheel moves the selection, as in the pinned notes.
    fn on_wheel(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let step = window.rem_size() * WHEEL_STEP_REMS;
        // Positive is up (content down): the selection moves the other way.
        match input::wheel_rows(event, step, &mut self.wheel).signum() {
            0 => return,
            up => self.step(up < 0, cx),
        }
        cx.stop_propagation();
    }

    fn render_row(
        &self,
        index: usize,
        row: &WorkspaceRow,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let label: SharedString = if row.current {
            format!("{}, current", row.name).into()
        } else {
            row.name.clone().into()
        };
        // The current workspace: a dot in the lane and the word after the
        // name, so it is not told by colour alone.
        let lane = row.current.then(|| {
            Icon::new(IconName::Circle)
                .size_2()
                .text_color(theme::accent(t))
                .into_any_element()
        });
        let name_line = div()
            .flex()
            .gap_2()
            .min_w_0()
            .child(ellipsis(row.name.clone()).text_color(theme::foreground(t)))
            .children(row.current.then(|| {
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme::muted(t))
                    .child("current")
            }));
        let frame = lane_row(
            workspace_row_id(&row.name),
            label,
            self.selected() == Some(index),
            NOTE_ROW_REMS,
            lane,
            [
                name_line.into_any_element(),
                ellipsis(row.folder.clone())
                    .text_xs()
                    .text_color(theme::secondary(t))
                    .into_any_element(),
            ],
            t,
        );
        with_click_cursor(frame, ClickPolicy::Open, true)
            .test_support()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                ClickPolicy::Open.apply(event, this, index, window, cx)
            }))
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let button = |id: &'static str, label: &'static str, tip: &'static str| {
            Button::new(id).ghost().xsmall().label(label).tooltip(tip)
        };
        let action = if self.rows().is_empty() {
            button(
                SWITCHER_SETUP_ID,
                "Run guided setup…",
                "Set up a workspace (Enter)",
            )
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.run_setup(window, cx)))
        } else {
            let other = self.list.selected_row().is_some_and(|row| !row.current);
            button(
                SWITCHER_SWITCH_ID,
                "Switch",
                "Switch to the selected workspace (Enter)",
            )
            .disabled(!other)
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.pick_selected(window, cx)),
            )
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(action)
            .child(div().flex_1())
            .child(
                button(SWITCHER_CLOSE_ID, "Close", "Close (Escape)").on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                ),
            )
    }
}

/// A click selects the row and picks it ([`ClickPolicy::Open`]).
impl ClickRows for WorkspaceSwitcher {
    fn select_row(&mut self, index: usize, cx: &mut Context<Self>) {
        self.list.select(index);
        cx.notify();
    }

    fn activate_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_selected(window, cx);
    }
}

impl Focusable for WorkspaceSwitcher {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for WorkspaceSwitcher {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let body = div()
            .id("workspace-switcher-rows")
            .role(Role::ListBox)
            .aria_label("Workspaces")
            .flex()
            .flex_col()
            .gap_0p5();
        let body = if self.rows().is_empty() {
            body.child(
                div()
                    .id(SWITCHER_EMPTY_ID)
                    .test_support()
                    .px_2()
                    .py_2()
                    .text_sm()
                    .text_color(theme::muted(t))
                    .child(EMPTY_TEXT),
            )
        } else {
            let rows = self
                .rows()
                .iter()
                .enumerate()
                .map(|(index, row)| self.render_row(index, row, cx).into_any_element())
                .collect::<Vec<_>>();
            body.children(rows)
        };
        div()
            .id(SWITCHER_ID)
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll(cx.listener(Self::on_wheel))
            .flex()
            .flex_col()
            .gap_2()
            .child(body)
            .child(div().border_t_1().border_color(theme::divider(t)))
            .child(self.render_footer(cx))
    }
}

// ---- The workspace's side ----------------------------------------------------------

impl Workspace {
    /// `SwitchWorkspace`, leader `v s`, the palette: open the workspace
    /// switcher over the window, or close it when it is the open modal.
    /// Refused under the setup assistant or another modal.
    pub(crate) fn open_workspace_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.onboarding_open() || self.preferences_open() {
            return;
        }
        if self.open_modal(window, cx) == Some(ModalKind::WorkspaceSwitcher) {
            self.close_modal(window, cx);
            return;
        }
        if modal::dialog_open(window, cx) {
            return;
        }
        // Read now, not cached: what the settings hold at this moment.
        let rows = self
            .settings
            .as_ref()
            .map(|settings| workspace_rows(&settings.read().unwrap_or_else(|e| e.into_inner())))
            .unwrap_or_default();
        let workspace = cx.entity().downgrade();
        let view = cx.new(|cx| WorkspaceSwitcher::new(rows, workspace, cx));
        self.switcher = Some(view.downgrade());
        let workspace = cx.entity().downgrade();
        self.show_modal(
            Slot::Own(ModalKind::WorkspaceSwitcher),
            DIALOG_REMS,
            modal::focus_view(view.clone()),
            move |dialog, _, _| {
                let picker = view.downgrade();
                let cancel = workspace.clone();
                dialog
                    .title(TITLE)
                    // Enter picks the selected workspace (or runs the setup
                    // with none); the pick closes the dialog itself.
                    .on_ok(move |_, window, cx| {
                        picker
                            .update(cx, |dialog, cx| dialog.pick_selected(window, cx))
                            .ok();
                        false
                    })
                    .on_cancel(move |_, window, cx| {
                        cancel
                            .update(cx, |workspace, cx| workspace.close_modal(window, cx))
                            .ok();
                        false
                    })
                    .child(view.clone())
            },
            window,
            cx,
        );
    }
}
