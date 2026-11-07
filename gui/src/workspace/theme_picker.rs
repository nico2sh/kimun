//! The **theme picker** (M9c): every theme, filtered as you type, the
//! window restyled live as the selection moves (leader `v t`, the palette's
//! `+vault · theme picker`, the CFG view's `Theme…`). A kit `Dialog` in the
//! window's one modal slot, like the workspace switcher.
//!
//! - **Rows.** `logic::preferences::ThemeChoices`: every theme, the one in
//!   effect marked (a dot and "current") and selected. The list is read on
//!   tokio's blocking pool (it reads the themes folder): "Loading themes…"
//!   until it lands. Typing fuzzy-filters it with the shared list engine
//!   (`ListCore`, `Filter::Fuzzy`, as the TUI picker); the best match is
//!   selected, an emptied filter selects the current theme again.
//! - **Live preview.** Moving the selection (Up/Down, a click) restyles the
//!   whole window through [`Workspace::apply_theme`]; the pointer passing
//!   over a row does not. Nothing is written while previewing. With nothing
//!   selected (the filter matches no theme: "No themes match.") the window
//!   goes back to the theme it had when the picker opened.
//! - **Enter** (or `Apply`, or a double-click) keeps the selected theme:
//!   the settings with only their theme changed (`with_theme`) are written
//!   off the UI thread, as Preferences save (the same writer, the `[gui]`
//!   table kept, an unparsable config never replaced); only then does the
//!   shared settings' theme change and the picker close. Nothing reopens. A
//!   failed write is shown in the dialog, which stays open on the previewed
//!   theme with the settings untouched; Escape still reverts. Keys wait
//!   while the write runs.
//! - **Escape**, `Cancel`, a click outside: the window goes back to the
//!   opening theme and the picker closes. So does every other way the modal
//!   goes (a file operation landing, the setup assistant opening, the vault
//!   being left): [`Workspace::reset_modal_state`] restores a pending
//!   preview.
//! - **Pointer** ([`ClickPolicy::Select`], as the search browser, whose
//!   selection also drives a preview): a click selects (previews), a
//!   double-click keeps the theme.
//! - **Where it opens.** In every window state, a vault or none; never over
//!   the setup assistant, Preferences (which list the themes themselves) or
//!   another modal.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, ScrollStrategy,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    UniformListScrollHandle, WeakEntity, Window, div, rems, uniform_list,
};
use kimun_notes::logic::list::{Filter, ListCore, ListSetup, StaticRowSource};
use kimun_notes::logic::preferences::{ThemeChoices, ThemeRow, with_theme};
use kimun_notes::settings::SharedSettings;
use kimun_notes::settings::themes::Theme;

use super::files::{ClickPolicy, ClickRows, ellipsis, lane_row, with_click_cursor};
use super::modal::{self, ModalKind, Slot};
use super::note::Closing;
use super::{Workspace, reopen};
use crate::tasks::blocking;
use crate::theme;

/// Element ids, for tests.
pub(crate) const THEME_PICKER_ID: &str = "theme-picker";
pub(crate) const THEME_PICKER_INPUT_ID: &str = "theme-picker-input";
pub(crate) const THEME_PICKER_LIST_ID: &str = "theme-picker-list";
pub(crate) const THEME_PICKER_EMPTY_ID: &str = "theme-picker-empty";
pub(crate) const THEME_PICKER_ERROR_ID: &str = "theme-picker-error";
pub(crate) const THEME_PICKER_APPLY_ID: &str = "theme-picker-apply";
pub(crate) const THEME_PICKER_CANCEL_ID: &str = "theme-picker-cancel";

/// The dialog's title.
pub(crate) const TITLE: &str = "Theme";
/// What it says while the list is read.
pub(crate) const LOADING_TEXT: &str = "Loading themes…";
/// What it says when the filter matches no theme.
pub(crate) const EMPTY_TEXT: &str = "No themes match.";

/// A theme's row id, by its name.
pub(crate) fn theme_pick_row_id(name: &str) -> SharedString {
    format!("theme-pick:{name}").into()
}

/// The dialog's width, in rems.
const DIALOG_REMS: f32 = 30.;
/// The list's height, in rems: about nine rows.
const LIST_REMS: f32 = 18.;
/// A row's height, in rems: one line.
const ROW_REMS: f32 = 2.;

/// The picker while it is open, as the workspace holds it.
pub(crate) struct ThemePickerOpen {
    /// The view: a write that lands tells whether it is still the open one.
    pub(crate) view: WeakEntity<ThemePicker>,
    /// The theme the window had when the picker opened: what Escape puts
    /// back.
    original: Theme,
    /// Whether the window shows another theme than `original` now.
    previewing: bool,
}

/// The theme picker's view. See the module docs.
pub(crate) struct ThemePicker {
    /// The themes, once read.
    choices: Option<ThemeChoices>,
    /// The rows, the filter and the selection: the shared list engine.
    list: ListCore<ThemeRow>,
    pub(crate) input: Entity<InputState>,
    scroll: UniformListScrollHandle,
    /// The theme (an index into the choices) the window previews now;
    /// `None`: the theme it had when the picker opened.
    previewed: Option<usize>,
    /// Why the last write failed.
    error: Option<SharedString>,
    /// The settings write running: keys wait for it. The write itself is the
    /// workspace's ([`Workspace::keep_theme`]), so it outlives the picker.
    saving: bool,
    /// Typed while saving: applied only if the write fails, so the
    /// preview and the selection stay on the theme being written.
    held_query: Option<String>,
    _load: Option<Task<()>>,
    workspace: WeakEntity<Workspace>,
    _input_events: Subscription,
}

impl ThemePicker {
    fn new(
        settings: SharedSettings,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter themes"));
        let input_events = cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.set_query(query, cx);
            }
        });
        let load = cx.spawn(async move |this, cx| {
            let choices = blocking(move || {
                ThemeChoices::new(&settings.read().unwrap_or_else(|e| e.into_inner()))
            })
            .await;
            this.update(cx, |picker, cx| picker.loaded(choices, cx))
                .ok();
        });
        Self {
            choices: None,
            list: ListCore::with_rows(
                Arc::new(StaticRowSource),
                Arc::new(|| {}),
                ListSetup {
                    filter: Filter::Fuzzy,
                    ..ListSetup::default()
                },
                Vec::new(),
            ),
            input,
            scroll: UniformListScrollHandle::new(),
            previewed: None,
            error: None,
            saving: false,
            held_query: None,
            _load: Some(load),
            workspace,
            _input_events: input_events,
        }
    }

    // ---- reading (the tests) ----------------------------------------------

    /// Whether the theme list has landed.
    #[cfg(test)]
    pub(crate) fn is_loaded(&self) -> bool {
        self.choices.is_some()
    }

    /// The rows shown, top first: (name, current).
    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<(String, bool)> {
        self.list
            .visible_rows()
            .into_iter()
            .map(|r| (r.name.clone(), r.current))
            .collect()
    }

    /// The selected theme's name.
    #[cfg(test)]
    pub(crate) fn selected(&self) -> Option<String> {
        self.list.selected_row().map(|r| r.name.clone())
    }

    /// The error shown.
    #[cfg(test)]
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether the write runs.
    #[cfg(test)]
    pub(crate) fn is_saving(&self) -> bool {
        self.saving
    }

    // ---- behaviour --------------------------------------------------------

    fn loaded(&mut self, choices: Result<ThemeChoices, String>, cx: &mut Context<Self>) {
        self._load = None;
        match choices {
            Ok(choices) => {
                self.list.replace_rows(choices.rows());
                self.choices = Some(choices);
                self.seed_selection();
                self.sync_preview(cx);
            }
            Err(e) => self.error = Some(format!("Couldn't read the themes: {e}").into()),
        }
        cx.notify();
    }

    /// Where the selection starts for the current filter: on the current
    /// theme with none, else on the best match.
    fn seed_selection(&mut self) {
        let original = self.choices.as_ref().map(|c| c.original);
        let pos = if self.list.query().trim().is_empty() {
            self.list
                .visible_rows()
                .iter()
                .position(|r| Some(r.index) == original)
                .unwrap_or(0)
        } else {
            0
        };
        if self.list.visible_len() > 0 {
            self.list.select(pos);
        }
        self.reveal_selection();
    }

    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        if self.saving {
            self.held_query = Some(query);
            return;
        }
        self.error = None;
        self.list.set_query(query);
        self.seed_selection();
        self.sync_preview(cx);
        cx.notify();
    }

    fn reveal_selection(&self) {
        if let Some(pos) = self.list.selected() {
            self.scroll.scroll_to_item(pos, ScrollStrategy::Nearest);
        }
    }

    /// Show the selected theme on the window if it is not shown yet (the
    /// opening theme with nothing selected).
    fn sync_preview(&mut self, cx: &mut Context<Self>) {
        let Some(choices) = self.choices.as_ref() else {
            return;
        };
        let target = choices.preview_target(self.list.selected_row().map(|r| r.index));
        if target == self.previewed {
            return;
        }
        self.previewed = target;
        let theme = target.map(|index| choices.themes[index].clone());
        self.workspace
            .update(cx, |workspace, cx| workspace.preview_theme(theme, cx))
            .ok();
    }

    /// Move the selection one row down (`true`) or up; waits while saving.
    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        if down {
            self.list.select_next();
        } else {
            self.list.select_prev();
        }
        self.reveal_selection();
        self.sync_preview(cx);
        cx.notify();
    }

    /// Enter, `Apply`, a double-click: write the selected theme, then keep
    /// it and close (see the module docs). Nothing selected: nothing.
    fn pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let (Some(choices), Some(row)) = (self.choices.as_ref(), self.list.selected_row()) else {
            return;
        };
        // The theme the window had on opening: nothing to keep, nothing to
        // write (and an unset theme stays unset).
        if row.index == choices.original {
            self.workspace
                .update(cx, |workspace, cx| workspace.close_modal(window, cx))
                .ok();
            return;
        }
        let theme = choices.themes[row.index].clone();
        let view = cx.weak_entity();
        let started = self
            .workspace
            .update(cx, |workspace, cx| {
                workspace.keep_theme(theme, view, window, cx)
            })
            .unwrap_or(false);
        if started {
            self.error = None;
            self.saving = true;
            cx.notify();
        }
    }

    /// The write failed: shown, the picker open again for another try.
    fn save_failed(&mut self, error: &str, cx: &mut Context<Self>) {
        self.saving = false;
        match self.held_query.take() {
            // The filter typed meanwhile now applies (it clears the error).
            Some(query) => self.set_query(query, cx),
            None => cx.notify(),
        }
        self.error = Some(format!("Couldn't save the theme: {error}").into());
    }

    /// Escape, `Cancel`, a click outside: close, the opening theme back.
    /// Waits while saving.
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.workspace
            .update(cx, |workspace, cx| workspace.close_modal(window, cx))
            .ok();
    }

    fn render_row(&self, pos: usize, row: &ThemeRow, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let label: SharedString = if row.current {
            format!("{}, current", row.name).into()
        } else {
            row.name.clone().into()
        };
        // The current theme: a dot in the lane and the word after the name,
        // so it is not told by colour alone.
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
            theme_pick_row_id(&row.name),
            label,
            self.list.selected() == Some(pos),
            ROW_REMS,
            lane,
            [name_line.into_any_element()],
            t,
        );
        with_click_cursor(frame, ClickPolicy::Select, !self.saving)
            .test_support()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if !this.saving {
                    ClickPolicy::Select.apply(event, this, pos, window, cx)
                }
            }))
            .into_any_element()
    }

    fn render_note(&self, id: &'static str, text: &'static str, cx: &gpui_kit::App) -> AnyElement {
        div()
            .id(id)
            .test_support()
            .px_2()
            .py_2()
            .text_sm()
            .text_color(theme::muted(&theme::current(cx)))
            .child(text)
            .into_any_element()
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_apply = self.list.selected_row().is_some() && self.choices.is_some();
        let button = |id: &'static str, label: &'static str, tip: &'static str| {
            Button::new(id).ghost().xsmall().label(label).tooltip(tip)
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                button(
                    THEME_PICKER_APPLY_ID,
                    "Apply",
                    "Keep the selected theme (Enter)",
                )
                .disabled(!can_apply)
                .loading(self.saving)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.pick(window, cx))),
            )
            .child(div().flex_1())
            .child(
                button(
                    THEME_PICKER_CANCEL_ID,
                    "Cancel",
                    "Put the theme back and close (Escape)",
                )
                .disabled(self.saving)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.cancel(window, cx))),
            )
    }
}

/// A click selects the row (previews it); a double-click keeps it
/// ([`ClickPolicy::Select`]).
impl ClickRows for ThemePicker {
    fn select_row(&mut self, pos: usize, cx: &mut Context<Self>) {
        self.list.select(pos);
        self.sync_preview(cx);
        cx.notify();
    }

    fn activate_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick(window, cx);
    }
}

impl Render for ThemePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let count = self.list.visible_len();
        let body = if count > 0 {
            uniform_list(
                THEME_PICKER_LIST_ID,
                count,
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .filter_map(|pos| {
                            let row = this.list.visible_row(pos)?.clone();
                            Some(this.render_row(pos, &row, cx))
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full()
            .into_any_element()
        } else if self.choices.is_none() && self.error.is_none() {
            self.render_note(THEME_PICKER_EMPTY_ID, LOADING_TEXT, cx)
        } else if self.choices.is_some() {
            self.render_note(THEME_PICKER_EMPTY_ID, EMPTY_TEXT, cx)
        } else {
            div().into_any_element()
        };
        let rows = div()
            .id("theme-picker-rows")
            .role(Role::ListBox)
            .aria_label("Themes")
            .h(rems(LIST_REMS))
            .child(body);
        div()
            .id(THEME_PICKER_ID)
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
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                this.pick(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                this.cancel(window, cx);
                cx.stop_propagation();
            }))
            .child(
                Input::new(&self.input)
                    .id(THEME_PICKER_INPUT_ID)
                    .aria_label("Filter themes"),
            )
            .child(rows)
            .children(self.error.clone().map(|error| {
                div()
                    .id(THEME_PICKER_ERROR_ID)
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(error.clone())
                    .text_sm()
                    .text_color(theme::error(t))
                    .child(error)
            }))
            .child(div().border_t_1().border_color(theme::divider(t)))
            .child(self.render_footer(cx))
    }
}

// ---- The workspace's side ----------------------------------------------------------

impl Workspace {
    /// Leader `v t`, the palette, the CFG view's `Theme…`: open the theme
    /// picker over the window, or close it (the opening theme back) when it
    /// is the open modal. Refused under the setup assistant, Preferences,
    /// another modal or the window's close. Needs the tokio runtime
    /// entered.
    pub(crate) fn open_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.onboarding_open() || self.preferences_open() || self.closing != Closing::Open {
            return;
        }
        if self.open_modal(window, cx) == Some(ModalKind::ThemePicker) {
            self.close_modal(window, cx);
            return;
        }
        if modal::dialog_open(window, cx) {
            return;
        }
        let Some(settings) = self.settings.clone() else {
            return;
        };
        let workspace = cx.entity().downgrade();
        let view = cx.new(|cx| ThemePicker::new(settings, workspace, window, cx));
        self.theme_picker = Some(ThemePickerOpen {
            view: view.downgrade(),
            original: (*theme::current(cx)).clone(),
            previewing: false,
        });
        let input = view.read(cx).input.clone();
        self.show_modal(
            Slot::Own(ModalKind::ThemePicker),
            DIALOG_REMS,
            move |window, cx| {
                let handle = input.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            },
            move |dialog, _, _| {
                let picker = view.downgrade();
                let cancel = view.downgrade();
                dialog
                    .title(TITLE)
                    .on_ok(move |_, window, cx| {
                        picker.update(cx, |picker, cx| picker.pick(window, cx)).ok();
                        false
                    })
                    // Escape and a click outside: the opening theme back.
                    .on_cancel(move |_, window, cx| {
                        cancel
                            .update(cx, |picker, cx| picker.cancel(window, cx))
                            .ok();
                        false
                    })
                    .child(view.clone())
            },
            window,
            cx,
        );
    }

    /// The picker's selection moved: show `theme` on the window (`None`:
    /// the theme it had when the picker opened). Ignored once the picker
    /// is gone.
    fn preview_theme(&mut self, theme: Option<Theme>, cx: &mut Context<Self>) {
        let Some(open) = self.theme_picker.as_mut() else {
            return;
        };
        open.previewing = theme.is_some();
        let theme = theme.unwrap_or_else(|| open.original.clone());
        self.apply_theme(theme, cx);
    }

    /// Keep `theme`: the shared settings with only their theme changed,
    /// written off the UI thread. The task is the workspace's, not the
    /// picker's: a modal reset that drops the picker mid-write cannot leave
    /// the file with a theme the window never took. Whether the write
    /// started (`false` without settings). The settings are read when the
    /// write's turn comes, so a settings change queued before it is not
    /// overwritten by an older copy.
    fn keep_theme(
        &mut self,
        theme: Theme,
        picker: WeakEntity<ThemePicker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(settings) = self.settings.clone() else {
            return false;
        };
        let name = theme.name.clone();
        let write = reopen::write_settings(self.store.clone(), move || {
            with_theme(&settings.read().unwrap_or_else(|e| e.into_inner()), &name)
        });
        cx.spawn_in(window, async move |this, cx| {
            let written = write.await;
            this.update_in(cx, |workspace, window, cx| match written {
                Ok(()) => workspace.theme_kept(theme, picker.entity_id(), window, cx),
                Err(e) => {
                    tracing::warn!("could not save the theme: {e}");
                    let shown = picker
                        .update(cx, |picker, cx| picker.save_failed(&e, cx))
                        .is_ok();
                    if !shown {
                        workspace.flash_error(format!("Couldn't save the theme: {e}"), cx);
                    }
                }
            })
            .ok();
        })
        .detach();
        true
    }

    /// The write landed: the shared settings take the theme and the window
    /// shows it. When `picker` is still the open one it closes with nothing
    /// to put back; when it is gone (its modal was reset, the opening theme
    /// put back) or another picker is open, the modal is left alone and the
    /// theme is shown (the other picker's opening theme is the new one).
    fn theme_kept(
        &mut self,
        theme: Theme,
        picker: gpui_kit::EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(settings) = self.settings.as_ref() {
            settings.write().unwrap_or_else(|e| e.into_inner()).theme = theme.name.clone();
        }
        match self.theme_picker.as_mut() {
            Some(open) if open.view.entity_id() == picker => {
                // Kept: closing the modal must not put the opening theme back.
                self.theme_picker = None;
                self.apply_theme(theme, cx);
                self.close_modal(window, cx);
            }
            Some(open) => {
                open.original = theme.clone();
                if !open.previewing {
                    self.apply_theme(theme, cx);
                }
            }
            None => self.apply_theme(theme, cx),
        }
    }

    /// The picker is going without a pick: the window goes back to the
    /// theme it had when the picker opened, if it previews another.
    pub(super) fn forget_theme_picker(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.theme_picker.take()
            && open.previewing
        {
            self.apply_theme(open.original, cx);
        }
    }
}
