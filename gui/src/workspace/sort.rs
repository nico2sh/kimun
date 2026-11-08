//! The **sort dialog** (M7b G7): one list's [`SortState`] — what it sorts
//! by (name, title or a property), the direction, and for FILES whether
//! folders come first — in a kit `Dialog`. Three lists open it:
//!
//! - **FILES** (`logic::files` `sort_state` / `apply_sort_state`): the rows
//!   already listed re-sort in place; a property sort reads its values from
//!   the index off the UI thread and re-sorts when they land (newest key
//!   wins, notes without the key last). It holds until FILES opens another
//!   folder, which starts on its default, unless it is saved as the default
//!   (below).
//! - **FIND** and the **search browser**: the sort is the query's order
//!   directive (`logic::sort::query_with_sort`), so a change rewrites the
//!   query and runs it again. In FIND that is an edit like any other: a
//!   saved search's name reads "• edited" after it. The browser's sort
//!   stacks over the browser (decision 3's one allowed stacked case, as its
//!   query syntax does); Escape returns to it.
//!
//! **Opening.** The `OpenSortDialog` chord (primary+R) from the rail or a
//! drawer sorts the focused view (FILES or FIND), and inside the search
//! browser the browser; it is never live in the editor, where Ctrl+R is
//! vim's redo. The palette row sorts the focused view (FILES when it is
//! none); each list's sort chip opens it with a click. The chord again
//! closes it.
//!
//! **Changes apply live**: each pick reaches the list at once. A property
//! sort applies once it has a key: picked from the vault's known keys
//! (loaded off the UI thread), or typed and confirmed with Enter. Escape or
//! `Close` keep what was applied; the kit gives the keyboard back to the
//! opener.
//!
//! **Save as default** (FILES only, the TUI's `s`): a checkbox, "Save as
//! default for notes" or "…for the journal" after the folder shown
//! (`FilesModel::is_current_journal`), ticked and cleared by click, Space on
//! it, or `s` anywhere in the dialog but the Key field. Ticked, the `Close`
//! button reads `Save`; pressing it (or Enter closing the Key field) is the
//! confirm: the sort stays applied, and the field and order of that context
//! (plus the grouping) are written with the shared candidate-then-apply
//! seam (`reopen::write_settings`, off the UI thread). Only once the write
//! is on disk do the shared settings take it (the write itself, so a write
//! queued behind it starts from it), and FILES' remembered default once it
//! lands (later folder opens use it); a failed write is flashed and nothing
//! is remembered. Escape, a click outside and the chord again never save. A
//! property sort can't be a default (the TUI's rule): the box is disabled.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, IndexPath, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Div, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, Role,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity,
    Window, div, rems,
};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::files::save_sort_default;
use kimun_notes::logic::input::{Key, KeyInput};
use kimun_notes::logic::message::SortTarget;
use kimun_notes::logic::sort::{
    SortField, SortOrder, SortState, is_blank_property, order_of_query, property_key,
    sort_chip_label,
};
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::key_suggestions::KeySuggestions;
use super::modal::{self, ModalKind, Slot};
use super::reopen;
use crate::input;
use crate::keymap;
use crate::theme;

/// Element ids, for tests.
pub(crate) const SORT_DIALOG_ID: &str = "sort-dialog";
pub(crate) const SORT_FIELD_ID: &str = "sort-field";
pub(crate) const SORT_ORDER_ID: &str = "sort-order";
pub(crate) const SORT_KEY_ID: &str = "sort-key";
pub(crate) const SORT_GROUP_ID: &str = "sort-group-dirs";
pub(crate) const SORT_HINT_ID: &str = "sort-hint";
pub(crate) const SORT_CLOSE_ID: &str = "sort-close";
pub(crate) const SORT_SAVE_ID: &str = "sort-save-default";

/// A known key suggested under the Key field.
pub(crate) fn sort_suggestion_id(key: &str) -> SharedString {
    format!("sort-suggestion:{key}").into()
}

/// The dialog's width, in rems.
const DIALOG_REMS: f32 = 26.;
/// The labels' column, in rems.
const LABEL_REMS: f32 = 7.;

/// The Field select's options, in the TUI's cycle order.
const FIELDS: [&str; 3] = ["Name", "Title", "Property"];
/// The Order select's options.
const ORDERS: [&str; 2] = ["Ascending ↑", "Descending ↓"];

/// What FILES' dialog says about how long its sort holds.
pub(crate) const FILES_HINT: &str = "Applies until FILES opens another folder.";
/// What FILES' dialog says with "save as default" ticked.
pub(crate) const FILES_SAVE_HINT: &str = "Save keeps it as the default. Esc closes without saving.";
/// What FILES' dialog says on a property sort.
pub(crate) const FILES_PROPERTY_HINT: &str =
    "Applies until FILES opens another folder. A property sort can't be a default.";
/// The checkbox, in the notes' context and in the journal's.
pub(crate) const SAVE_NOTES_LABEL: &str = "Save as default for notes";
pub(crate) const SAVE_JOURNAL_LABEL: &str = "Save as default for the journal";
/// What the confirm button reads, plain and with the box ticked.
pub(crate) const CLOSE_LABEL: &str = "Close";
pub(crate) const SAVE_LABEL: &str = "Save";

/// The checkbox's label for the `journal` context or the notes'.
pub(crate) fn save_label(journal: bool) -> &'static str {
    if journal {
        SAVE_JOURNAL_LABEL
    } else {
        SAVE_NOTES_LABEL
    }
}
/// What a query's dialog says about where its sort goes.
pub(crate) const QUERY_HINT: &str = "Written into the query as its order.";

fn field_label(field: &SortField) -> &'static str {
    match field {
        SortField::Name => FIELDS[0],
        SortField::Title => FIELDS[1],
        SortField::Property(_) => FIELDS[2],
    }
}

fn order_label(order: SortOrder) -> &'static str {
    match order {
        SortOrder::Ascending => ORDERS[0],
        SortOrder::Descending => ORDERS[1],
    }
}

/// The chip a sortable list shows its order with (FIND's bar, FILES'
/// header, the search browser's query row): small, muted, bordered, a
/// button that opens the sort dialog. `label` is the order as the chip
/// says it; the caller adds the click, then `test_support`.
pub(super) fn sort_chip(id: &'static str, label: SharedString, t: &Theme) -> Stateful<Div> {
    let hover = theme::hover_background(t);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(SharedString::from(format!("Sorted by {label}")))
        .flex_none()
        .whitespace_nowrap()
        .px_1p5()
        .rounded_md()
        .border_1()
        .border_color(theme::divider(t))
        .text_xs()
        .text_color(theme::muted(t))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(label)
        .tooltip(|window, cx| Tooltip::new(sort_tip(cx)).build(window, cx))
}

/// The chip's label for `field` / `order` (`Name ↑`, `due ↓`).
pub(super) fn chip_label(field: &SortField, order: SortOrder) -> SharedString {
    sort_chip_label(field, order).trim().to_string().into()
}

/// The chip's tooltip: what a click does, and the chord that does it.
fn sort_tip(cx: &App) -> SharedString {
    match keymap::chord_label(cx, &ActionShortcuts::OpenSortDialog) {
        Some(chord) => format!("Change the sort ({chord})").into(),
        None => "Change the sort".into(),
    }
}

/// The sort dialog's view. See the module docs.
pub(crate) struct SortDialog {
    target: SortTarget,
    state: SortState,
    /// The list is in its natural order (the browser's recent notes): the
    /// Field select shows "Unsorted" until a Field or Order pick.
    unsorted: bool,
    field: Entity<SelectState<Vec<&'static str>>>,
    order: Entity<SelectState<Vec<&'static str>>>,
    key: Entity<InputState>,
    known_keys: Vec<String>,
    suggestions: KeySuggestions,
    /// FILES' "save as default": the context it saves to (`true`: the
    /// journal's), taken when the dialog opened (FILES cannot move under a
    /// modal). `None` for the query lists, which never save.
    save_context: Option<bool>,
    /// The "save as default" box is ticked.
    save_default: bool,
    workspace: WeakEntity<Workspace>,
    _subscriptions: Vec<Subscription>,
}

impl SortDialog {
    fn new(
        target: SortTarget,
        state: SortState,
        unsorted: bool,
        save_context: Option<bool>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field_ix = (!unsorted).then(|| {
            let at = FIELDS.iter().position(|f| *f == field_label(&state.field));
            IndexPath::new(at.unwrap_or(0))
        });
        let field = cx.new(|cx| SelectState::new(FIELDS.to_vec(), field_ix, window, cx));
        let order_ix = ORDERS.iter().position(|o| *o == order_label(state.order));
        let order = cx
            .new(|cx| SelectState::new(ORDERS.to_vec(), order_ix.map(IndexPath::new), window, cx));
        let typed = property_key(&state.field).unwrap_or_default().to_string();
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Property key")
                .default_value(typed)
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &field,
                window,
                |this, _, event: &SelectEvent<Vec<&'static str>>, window, cx| {
                    let SelectEvent::Confirm(Some(label)) = event else {
                        return;
                    };
                    this.pick_field(label, window, cx);
                },
            ),
            cx.subscribe_in(
                &order,
                window,
                |this, _, event: &SelectEvent<Vec<&'static str>>, window, cx| {
                    let SelectEvent::Confirm(Some(label)) = event else {
                        return;
                    };
                    let order = if *label == ORDERS[1] {
                        SortOrder::Descending
                    } else {
                        SortOrder::Ascending
                    };
                    this.pick_order(order, window, cx);
                },
            ),
            cx.subscribe(&key, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh_suggestions(cx);
                }
            }),
        ];
        Self {
            target,
            state,
            unsorted,
            field,
            order,
            key,
            known_keys: Vec::new(),
            suggestions: KeySuggestions::default(),
            save_context: save_context.filter(|_| target == SortTarget::Sidebar),
            save_default: false,
            workspace,
            _subscriptions: subscriptions,
        }
    }

    /// The state the dialog shows (tests).
    #[cfg(test)]
    pub(crate) fn state(&self) -> &SortState {
        &self.state
    }

    /// The list the dialog sorts (tests).
    #[cfg(test)]
    pub(crate) fn target(&self) -> SortTarget {
        self.target
    }

    /// Whether the dialog says "Unsorted" (tests).
    #[cfg(test)]
    pub(crate) fn is_unsorted(&self) -> bool {
        self.unsorted
    }

    /// The "save as default" checkbox's label, when the dialog has one
    /// (tests).
    #[cfg(test)]
    pub(crate) fn save_default_label(&self) -> Option<&'static str> {
        self.save_context.map(save_label)
    }

    /// Whether a confirm now saves the sort as the default (tests).
    #[cfg(test)]
    pub(crate) fn saves_default(&self) -> bool {
        self.will_save()
    }

    /// The known keys suggested now (tests).
    #[cfg(test)]
    pub(crate) fn suggestions(&self) -> Vec<String> {
        self.suggestions.shown().to_vec()
    }

    /// The vault's property keys landed (read off the UI thread).
    fn keys_loaded(&mut self, keys: Vec<String>, cx: &mut Context<Self>) {
        self.known_keys = keys;
        self.refresh_suggestions(cx);
    }

    fn typed_key(&self, cx: &App) -> String {
        self.key.read(cx).value().trim().to_string()
    }

    fn refresh_suggestions(&mut self, cx: &mut Context<Self>) {
        let typed = self.typed_key(cx);
        self.suggestions.refresh(&self.known_keys, &typed);
        cx.notify();
    }

    /// Hand the state to the list. A property sort with no key yet is not
    /// an order: nothing is sent.
    fn emit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        if is_blank_property(&self.state.field) {
            return;
        }
        let (target, state) = (self.target, self.state.clone());
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.apply_sort(target, &state, window, cx)
            })
            .ok();
    }

    /// A Field pick (the select, or a test). Property takes the typed key,
    /// and the keyboard goes to the Key field.
    pub(crate) fn pick_field(&mut self, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.unsorted = false;
        self.state.field = match label {
            l if l == FIELDS[1] => SortField::Title,
            l if l == FIELDS[2] => SortField::Property(self.typed_key(cx)),
            _ => SortField::Name,
        };
        self.sync_selects(window, cx);
        if matches!(self.state.field, SortField::Property(_)) {
            // A property sort can't be a default: the tick goes with it,
            // and doesn't come back on a later Name or Title.
            self.save_default = false;
            self.key.update(cx, |input, cx| input.focus(window, cx));
        }
        self.emit(window, cx);
    }

    /// An Order pick. Leaving "Unsorted" sorts by name in that order (the
    /// TUI's rule).
    pub(crate) fn pick_order(
        &mut self,
        order: SortOrder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.order = order;
        if std::mem::take(&mut self.unsorted) {
            self.state.field = SortField::Name;
        }
        self.sync_selects(window, cx);
        self.emit(window, cx);
    }

    /// The switch: folders first, or among the rest (FILES).
    pub(crate) fn set_group_dirs(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.group_dirs.is_none() {
            return;
        }
        self.state.group_dirs = Some(on);
        self.emit(window, cx);
    }

    /// The sort can be saved as a default: FILES' dialog, not a property
    /// sort.
    fn can_save(&self) -> bool {
        self.save_context.is_some() && !matches!(self.state.field, SortField::Property(_))
    }

    /// A confirm saves: the box is ticked and the sort can be a default.
    fn will_save(&self) -> bool {
        self.save_default && self.can_save()
    }

    /// The "save as default" box (a click, Space on it, `s`, or a test).
    /// Nothing for the query lists or a property sort.
    pub(crate) fn set_save_default(&mut self, on: bool, cx: &mut Context<Self>) {
        if !self.can_save() {
            return;
        }
        self.save_default = on;
        cx.notify();
    }

    /// `s` outside the Key field: tick or clear the box (the TUI's key).
    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(KeyInput { key, mods }) = input::key_down_input(event) else {
            return;
        };
        if mods.ctrl || mods.alt || mods.sup || mods.shift || key != Key::Char('s') {
            return;
        }
        if !self.can_save() || self.key_focused(window, cx) {
            return;
        }
        self.set_save_default(!self.save_default, cx);
        cx.stop_propagation();
    }

    /// Sort by the property `key` (a suggestion taken, or the typed key
    /// confirmed). A blank key does nothing.
    pub(crate) fn pick_key(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        let key = key.trim().to_string();
        if key.is_empty() {
            return;
        }
        if self.typed_key(cx) != key {
            // `set_value` reports no change: the suggestions follow here.
            self.key
                .update(cx, |input, cx| input.set_value(key.clone(), window, cx));
        }
        self.unsorted = false;
        self.state.field = SortField::Property(key);
        self.save_default = false;
        self.refresh_suggestions(cx);
        self.emit(window, cx);
    }

    /// The selects show the state (after a pick moved Unsorted to Name).
    fn sync_selects(&self, window: &mut Window, cx: &mut Context<Self>) {
        let field = field_label(&self.state.field);
        self.field.update(cx, |select, cx| {
            select.set_selected_value(&field, window, cx)
        });
        let order = order_label(self.state.order);
        self.order.update(cx, |select, cx| {
            select.set_selected_value(&order, window, cx)
        });
    }

    fn key_focused(&self, window: &Window, cx: &App) -> bool {
        self.key.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Enter in the Key field: take the highlighted suggestion, else sort
    /// by the typed key; with nothing new to apply, close (the TUI's Enter).
    fn confirm_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self
            .suggestions
            .highlighted()
            .and_then(|ix| self.suggestions.get(ix))
            .cloned()
        {
            self.pick_key(key, window, cx);
            return;
        }
        let typed = self.typed_key(cx);
        if !typed.is_empty() && self.state.field != SortField::Property(typed.clone()) {
            self.pick_key(typed, window, cx);
        } else {
            self.confirm(window, cx);
        }
    }

    fn take_suggestion(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.suggestions.get(ix).cloned() {
            self.pick_key(key, window, cx);
        }
    }

    /// The confirm (`Close` / `Save`, Enter closing the Key field): with
    /// "save as default" ticked the sort is written as its context's
    /// default; the dialog closes either way.
    pub(crate) fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let save = self
            .save_context
            .filter(|_| self.will_save())
            .map(|journal| (self.state.clone(), journal));
        self.workspace
            .update(cx, |workspace, cx| {
                if let Some((state, journal)) = save {
                    workspace.save_sort_default(state, journal, cx);
                }
                workspace.dismiss_sort(window, cx);
            })
            .ok();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| workspace.dismiss_sort(window, cx))
            .ok();
    }

    fn row(&self, label: &'static str, control: impl IntoElement, cx: &gpui_kit::App) -> Div {
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .w(rems(LABEL_REMS))
                    .flex_none()
                    .text_sm()
                    .text_color(theme::muted(&theme::current(cx)))
                    .child(label),
            )
            .child(div().flex_1().min_w_0().child(control))
    }
}

impl Render for SortDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        let property = matches!(self.state.field, SortField::Property(_));
        let suggestions = if property && self.key_focused(window, cx) {
            self.suggestions.render(
                "sort-suggestions",
                "Known keys",
                sort_suggestion_id,
                &t,
                cx,
                |this: &mut Self, ix, window, cx| this.take_suggestion(ix, window, cx),
            )
        } else {
            None
        };
        let key_row = property.then(|| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                // The Key field's keys: the suggestions first.
                .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                    if this.suggestions.step(false) {
                        cx.notify();
                        cx.stop_propagation();
                    }
                }))
                .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                    if this.suggestions.step(true) {
                        cx.notify();
                        cx.stop_propagation();
                    }
                }))
                .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                    this.confirm_key(window, cx);
                    cx.stop_propagation();
                }))
                .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                    // A highlighted suggestion lets go first.
                    if this.suggestions.release() {
                        cx.notify();
                    } else {
                        this.close(window, cx);
                    }
                    cx.stop_propagation();
                }))
                .child(
                    self.row(
                        "Key",
                        Input::new(&self.key)
                            .id(SORT_KEY_ID)
                            .aria_label("Property key"),
                        cx,
                    ),
                )
                .when_some(suggestions, |column, list| {
                    column.child(div().pl(rems(LABEL_REMS + 0.75)).child(list))
                })
        });
        let group = self.state.group_dirs.map(|on| {
            let this = cx.entity().downgrade();
            self.row(
                "Folders first",
                Switch::new(SORT_GROUP_ID)
                    .checked(on)
                    .accessibility_label("Group folders first")
                    .on_change(move |checked, window, cx| {
                        let checked = *checked;
                        this.update(cx, |dialog, cx| dialog.set_group_dirs(checked, window, cx))
                            .ok();
                    }),
                cx,
            )
        });
        let save_row = self.save_context.map(|journal| {
            let this = cx.entity().downgrade();
            let can_save = self.can_save();
            let tip = if can_save {
                "Keep this sort for every such folder (S)"
            } else {
                "A property sort can't be a default"
            };
            self.row(
                "",
                Checkbox::new(SORT_SAVE_ID)
                    .label(save_label(journal))
                    .checked(self.will_save())
                    .disabled(!can_save)
                    .tooltip(tip)
                    .on_click(move |checked, _, cx| {
                        let checked = *checked;
                        this.update(cx, |dialog, cx| dialog.set_save_default(checked, cx))
                            .ok();
                    }),
                cx,
            )
        });
        let saving = self.will_save();
        let hint = match self.target {
            SortTarget::Sidebar if saving => FILES_SAVE_HINT,
            SortTarget::Sidebar if !self.can_save() => FILES_PROPERTY_HINT,
            SortTarget::Sidebar => FILES_HINT,
            _ => QUERY_HINT,
        };
        let confirm = Button::new(SORT_CLOSE_ID)
            .small()
            .label(if saving { SAVE_LABEL } else { CLOSE_LABEL })
            .when(saving, |button| button.primary())
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.confirm(window, cx)));
        div()
            .id(SORT_DIALOG_ID)
            .test_support()
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                self.row(
                    "Sort by",
                    Select::new(&self.field)
                        .id(SORT_FIELD_ID)
                        .placeholder("Unsorted")
                        .accessibility_label("Sort by"),
                    cx,
                ),
            )
            .child(
                self.row(
                    "Order",
                    Select::new(&self.order)
                        .id(SORT_ORDER_ID)
                        .accessibility_label("Order"),
                    cx,
                ),
            )
            .children(key_row)
            .children(group)
            .children(save_row)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_1()
                    .child(
                        div()
                            .id(SORT_HINT_ID)
                            .test_support()
                            .role(Role::Note)
                            .aria_label(hint)
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(theme::muted(&t))
                            .child(hint),
                    )
                    .child(confirm),
            )
    }
}

/// The open sort dialog, while one is.
pub(crate) struct SortOpen {
    pub(crate) dialog: WeakEntity<SortDialog>,
    /// Over the search browser (the browser keeps the modal slot).
    pub(crate) stacked: bool,
}

// ---- The workspace's side ----------------------------------------------------

impl Workspace {
    /// Whether the sort dialog is stacked over the search browser.
    pub(crate) fn sort_stacked(&self) -> bool {
        self.sort.as_ref().is_some_and(|open| open.stacked)
    }

    /// The `OpenSortDialog` chord (`key`: pressed) or palette row: sort the
    /// focused view — FILES or FIND (the rail's cell when the rail has the
    /// keyboard); the palette's row sorts FILES when no view is focused.
    /// False when the chord means nothing here (it goes on).
    pub(crate) fn sort_command(
        &mut self,
        pressed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        use kimun_notes::logic::layout_tree::Focus;
        use kimun_notes::logic::nav::DrawerView;
        let view = match self.layout.focused() {
            Focus::Rail => self.rail_cursor_view(),
            _ => self.focused_view(),
        };
        let target = match view {
            Some(DrawerView::Files) => SortTarget::Sidebar,
            Some(DrawerView::Find) => SortTarget::Query,
            _ if !pressed => SortTarget::Sidebar,
            _ => return false,
        };
        self.open_sort(target, window, cx);
        true
    }

    /// The sort and its "Unsorted" flag `target` opens on; `None` when the
    /// list is not there.
    fn sort_state_of(&self, target: SortTarget, cx: &App) -> Option<(SortState, bool)> {
        let query_state = |query: &str| {
            let (field, order) = order_of_query(query);
            SortState {
                field,
                order,
                group_dirs: None,
            }
        };
        match target {
            SortTarget::Sidebar => self
                .files
                .as_ref()
                .map(|files| (files.read(cx).sort_state(), false)),
            SortTarget::Query => self
                .find
                .as_ref()
                .map(|find| (query_state(&find.read(cx).query(cx)), false)),
            SortTarget::Browser => {
                let picker = self.picker.as_ref()?.upgrade()?;
                let query = picker.read(cx).input.read(cx).value().to_string();
                // The empty query lists the recent notes, most recent first.
                let unsorted = query.trim().is_empty();
                Some((query_state(&query), unsorted))
            }
        }
    }

    /// Open the sort dialog on `target`'s sort. The browser's stacks over
    /// the open browser; the others take the modal slot (refused under
    /// another modal). Open already: closed (the chord's toggle).
    pub(crate) fn open_sort(
        &mut self,
        target: SortTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sort.is_some() && modal::dialog_open(window, cx) {
            self.dismiss_sort(window, cx);
            return;
        }
        let stacked = target == SortTarget::Browser;
        let allowed = if stacked {
            self.open_modal(window, cx) == Some(ModalKind::Search)
                && !self.query_help
                && !self.picker_busy(cx)
        } else {
            !modal::dialog_open(window, cx)
        };
        if !allowed {
            return;
        }
        let Some(vault) = self.vault_or_flash(cx) else {
            return;
        };
        let Some((state, unsorted)) = self.sort_state_of(target, cx) else {
            return;
        };
        // FILES' "save as default" saves to the context of the folder shown.
        let save_context = match target {
            SortTarget::Sidebar => self
                .files
                .as_ref()
                .map(|files| files.read(cx).is_current_journal()),
            _ => None,
        };
        let workspace = cx.entity().downgrade();
        let view = cx.new(|cx| {
            SortDialog::new(
                target,
                state,
                unsorted,
                save_context,
                workspace.clone(),
                window,
                cx,
            )
        });
        // The known keys, read off the UI thread.
        let keys = tokio::spawn(async move { vault.property_keys().await });
        let loading = view.downgrade();
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(keys)) = keys.await {
                loading
                    .update(cx, |dialog, cx| dialog.keys_loaded(keys, cx))
                    .ok();
            }
        })
        .detach();
        self.sort = Some(SortOpen {
            dialog: view.downgrade(),
            stacked,
        });
        let closed = workspace.clone();
        let mine = view.downgrade();
        let field = view.read(cx).field.clone();
        self.show_modal(
            // Over the browser the slot stays `Search`.
            if stacked {
                Slot::Stacked
            } else {
                Slot::Own(ModalKind::Sort)
            },
            DIALOG_REMS,
            move |window, cx| field.update(cx, |select, cx| select.focus(window, cx)),
            move |dialog, _, _| {
                let closed = closed.clone();
                let mine = mine.clone();
                dialog
                    .title("Sort")
                    .on_close(move |_, _, cx| {
                        closed
                            .update(cx, |workspace, _| {
                                // Only this dialog's record (a newer one may
                                // be open by now).
                                if workspace
                                    .sort
                                    .as_ref()
                                    .is_some_and(|open| open.dialog == mine)
                                {
                                    workspace.sort = None;
                                }
                            })
                            .ok();
                    })
                    .child(view.clone())
            },
            window,
            cx,
        );
    }

    /// Close the sort dialog (Escape, `Close`, the chord again): over the
    /// browser only it goes, and the browser has the keyboard again.
    pub(crate) fn dismiss_sort(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.sort.take() {
            Some(open) if open.stacked => {
                if modal::dialog_open(window, cx) {
                    window.close_dialog(cx);
                }
            }
            Some(_) => self.close_modal(window, cx),
            None => {}
        }
        cx.notify();
    }

    /// A pick in the dialog: hand `state` to `target`'s list now.
    pub(crate) fn apply_sort(
        &mut self,
        target: SortTarget,
        state: &SortState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match target {
            SortTarget::Sidebar => {
                if let Some(files) = self.files.as_ref() {
                    files.update(cx, |pane, cx| pane.apply_sort(state, cx));
                }
            }
            SortTarget::Query => {
                if let Some(find) = self.find.as_ref() {
                    find.update(cx, |view, cx| {
                        view.apply_sort(&state.field, state.order, window, cx)
                    });
                }
            }
            SortTarget::Browser => {
                if let Some(picker) = self.picker.as_ref().and_then(|p| p.upgrade()) {
                    picker.update(cx, |picker, cx| {
                        picker.apply_sort(&state.field, state.order, window, cx)
                    });
                }
            }
        }
    }

    /// FILES' "save as default" confirmed: `state` as the default of its
    /// context (`journal`), written with the shared candidate-then-apply
    /// seam ([`reopen::write_settings`]) — the shared settings as they are
    /// when the write's turn comes, plus this default, off the UI thread;
    /// the shared settings take it once it is on disk, so a write queued
    /// behind it starts from it. Must run with the tokio runtime entered.
    pub(crate) fn save_sort_default(
        &mut self,
        state: SortState,
        journal: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(settings) = self.settings.clone() else {
            return;
        };
        let written_state = state.clone();
        let write = reopen::write_settings(self.store.clone(), settings, move |s| {
            save_sort_default(s, &written_state, journal);
        });
        cx.spawn(async move |this, cx| {
            let written = write.await;
            this.update(cx, |workspace, cx| {
                workspace.sort_default_written(state, journal, written, cx)
            })
            .ok();
        })
        .detach();
    }

    /// The default's write landed. Written: the shared settings already
    /// have it (the write put it there) and FILES remembers it for later
    /// folder opens; the folder shown already has that sort. Failed: the
    /// sort stays on the folder shown, nothing is remembered, a flash says
    /// so.
    fn sort_default_written(
        &mut self,
        state: SortState,
        journal: bool,
        written: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = written {
            tracing::warn!("could not save the sort default: {error}");
            self.flash_error(format!("Couldn't save the sort default: {error}"), cx);
            return;
        }
        if let Some(files) = self.files.as_ref() {
            files.update(cx, |pane, _| {
                pane.remember_default_for(journal, state.field.clone(), state.order)
            });
        }
        let context = if journal { "the journal" } else { "notes" };
        self.flash(format!("Sort saved as the default for {context}"), cx);
    }

    /// The sort dialog's view, while open (tests).
    #[cfg(test)]
    pub(crate) fn sort_dialog(&self) -> Option<Entity<SortDialog>> {
        self.sort.as_ref().and_then(|open| open.dialog.upgrade())
    }
}
