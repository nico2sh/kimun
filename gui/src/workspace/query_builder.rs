//! The **query builder**: the current query as a list of conditions — a tag, a
//! folder, a property … each with a "not" box — and a sort, in a kit `Dialog`,
//! with the query it writes and how many notes it matches shown live. `Apply`
//! puts the query in the field it came from and runs it; `Cancel` and Escape
//! change nothing.
//!
//! The query is [`BuilderModel`] (logic `query::builder`): rows over core's
//! `SearchTerms`, so quoting and the operators are core's, and a part of the
//! query no row can say stays as a raw row. This view owns the controls for
//! each row and what is suggested under the field being typed in.
//!
//! **Opening.** From FIND (its `Build…` button, the `QueryBuilder` chord) it
//! takes the modal slot. From the search browser it stacks over it, as the
//! sort dialog and the query syntax do, and Escape returns to the browser.
//!
//! **Suggestions** under the field in use, from lists loaded off the UI
//! thread (tags, property keys, folders) or asked per keystroke (note names).
//! A links row offers "current open note" first: it writes `{note}`, which the
//! query resolves against the note open when it runs.
//!
//! **The count** is the matching notes' number as the Query panel counts them
//! ([`count_query`]), about a quarter of a second after the last edit.
//!
//! **Keys.** Up/Down walk the suggestions and Enter takes one. Enter with none
//! highlighted adds a row after a complete one; Ctrl/Cmd+Enter applies.
//! Escape lets go of the suggestions first, then closes.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Disableable as _, IndexPath, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Div, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity,
    Window, div, rems,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::autocomplete::{OPEN_NOTE_LABEL, offers_open_note};
use kimun_notes::logic::input::{Key, KeyInput};
use kimun_notes::logic::query::{
    BuilderModel, BuilderRow, PROPERTY_OPS, QueryContext, QueryCount, RowKind, VAR_NOTE,
    count_query, property_op_label,
};
use kimun_notes::logic::sort::{SortField, SortOrder};

use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::file_dialogs::footer;
use super::key_suggestions::KeySuggestions;
use super::modal::{self, ModalKind, Slot};
use super::sort::{FIELDS, ORDERS, field_label, field_of_label, order_label, order_of_label};
use crate::input;
use crate::tasks;
use crate::theme;

/// Element ids, for tests.
pub(crate) const BUILDER_ID: &str = "query-builder";
pub(crate) const BUILDER_ADD_ID: &str = "query-builder-add";
pub(crate) const BUILDER_PREVIEW_ID: &str = "query-builder-preview";
pub(crate) const BUILDER_COUNT_ID: &str = "query-builder-count";
pub(crate) const BUILDER_SORT_FIELD_ID: &str = "query-builder-sort-field";
pub(crate) const BUILDER_SORT_ORDER_ID: &str = "query-builder-sort-order";
pub(crate) const BUILDER_SORT_KEY_ID: &str = "query-builder-sort-key";
pub(crate) const BUILDER_SUGGESTIONS_ID: &str = "query-builder-suggestions";

/// A row's value field, by the row's position.
pub(crate) fn builder_value_id(ix: usize) -> SharedString {
    format!("query-builder-value:{ix}").into()
}

/// A suggestion, by what it would write.
pub(crate) fn builder_suggestion_id(key: &str) -> SharedString {
    format!("query-builder-suggestion:{key}").into()
}

/// The dialog's width, in rems.
const DIALOG_REMS: f32 = 44.;
/// How long after the last edit the count is taken.
pub(crate) const COUNT_DEBOUNCE: Duration = Duration::from_millis(250);
/// Notes suggested per keystroke.
const NOTE_SUGGESTIONS: usize = 8;

/// The sort control's first option, before the sort dialog's fields.
const NO_SORT: &str = "No sort";

/// The sort control's field options: no sort, then the sort dialog's own.
fn sort_field_options() -> Vec<&'static str> {
    std::iter::once(NO_SORT).chain(FIELDS).collect()
}

/// What the dialog's "Build…" buttons say on hover, before the chord.
pub(crate) const BUILD_TIP: &str = "Build the query from conditions";

type Choice = Entity<SelectState<Vec<&'static str>>>;

/// Which field of a row, or the sort's key, the suggestions are for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Active {
    Value(u64),
    Key(u64),
    SortKey,
}

/// What a row's controls are: the model row's twin, found by `id` (a row's
/// position moves as others go).
struct RowView {
    id: u64,
    kind: Choice,
    op: Choice,
    value: Entity<InputState>,
    key: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

/// Where the built query goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuilderTarget {
    /// FIND's field.
    Find,
    /// The search browser's field (the dialog is stacked over it).
    Browser,
}

/// The query builder's view. See the module docs.
pub(crate) struct QueryBuilderDialog {
    model: BuilderModel,
    rows: Vec<RowView>,
    next_id: u64,
    sort_field: Choice,
    sort_order: Choice,
    sort_key: Entity<InputState>,
    /// The Order select's pick, kept while there is no sort.
    order_choice: SortOrder,
    /// What the suggestions are for, and the suggestions.
    active: Option<Active>,
    suggestions: KeySuggestions,
    /// The vault's tags, property keys and folders (as typed in a `/` term),
    /// loaded off the UI thread.
    tags: Vec<String>,
    keys: Vec<String>,
    folders: Vec<String>,
    /// The notes named like what is typed, asked per keystroke; `notes_gen`
    /// drops an answer a later keystroke has overtaken.
    notes: Vec<String>,
    notes_gen: u64,
    /// The field `notes` were asked for.
    notes_for: Option<Active>,
    vault: Arc<NoteVault>,
    /// The note open now, `{note}`'s meaning.
    note: Option<VaultPath>,
    /// `None` while the count is being taken (or nothing was asked yet).
    count: Option<QueryCount>,
    counting: Option<Task<()>>,
    target: BuilderTarget,
    workspace: WeakEntity<Workspace>,
    _subscriptions: Vec<Subscription>,
}

impl QueryBuilderDialog {
    fn new(
        query: &str,
        target: BuilderTarget,
        vault: Arc<NoteVault>,
        note: Option<VaultPath>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut model = BuilderModel::from_query(query);
        if model.rows.is_empty() {
            model.rows.push(BuilderRow::new(RowKind::Text));
        }
        let sort_ix = model.sort.as_ref().map_or(0, |(field, _)| {
            1 + FIELDS
                .iter()
                .position(|f| *f == field_label(field))
                .unwrap_or(0)
        });
        let order = model
            .sort
            .as_ref()
            .map_or(SortOrder::Ascending, |(_, o)| *o);
        let order_ix = ORDERS
            .iter()
            .position(|o| *o == order_label(order))
            .unwrap_or(0);
        let sort_key_text = match &model.sort {
            Some((SortField::Property(key), _)) => key.clone(),
            _ => String::new(),
        };
        let sort_field = cx.new(|cx| {
            SelectState::new(
                sort_field_options(),
                Some(IndexPath::new(sort_ix)),
                window,
                cx,
            )
        });
        let sort_order = cx.new(|cx| {
            SelectState::new(ORDERS.to_vec(), Some(IndexPath::new(order_ix)), window, cx)
        });
        let sort_key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Property key")
                .default_value(sort_key_text)
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &sort_field,
                window,
                |this, _, event: &SelectEvent<Vec<&'static str>>, window, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event {
                        this.pick_sort_field(label, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &sort_order,
                window,
                |this, _, event: &SelectEvent<Vec<&'static str>>, _, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event {
                        this.pick_sort_order(order_of_label(label), cx);
                    }
                },
            ),
            cx.subscribe_in(
                &sort_key,
                window,
                |this, input, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => {
                        let key = input.read(cx).value().trim().to_string();
                        this.set_sort_key(key, cx);
                        this.active = Some(Active::SortKey);
                        this.refresh_suggestions(true, cx);
                    }
                    InputEvent::Focus => {
                        this.active = Some(Active::SortKey);
                        this.refresh_suggestions(true, cx);
                    }
                    _ => {}
                },
            ),
        ];
        let mut dialog = Self {
            model: BuilderModel::default(),
            rows: Vec::new(),
            next_id: 0,
            sort_field,
            sort_order,
            sort_key,
            order_choice: order,
            active: None,
            suggestions: KeySuggestions::default(),
            tags: Vec::new(),
            keys: Vec::new(),
            folders: Vec::new(),
            notes: Vec::new(),
            notes_gen: 0,
            notes_for: None,
            vault,
            note,
            count: None,
            counting: None,
            target,
            workspace,
            _subscriptions: subscriptions,
        };
        let rows = std::mem::take(&mut model.rows);
        dialog.model.sort = model.sort;
        for row in rows {
            dialog.push_row(row, window, cx);
        }
        dialog.schedule_count(cx);
        dialog
    }

    // ---- The model, as the tests and the workspace see it ----------------------

    /// The query the dialog writes.
    pub(crate) fn query(&self) -> String {
        self.model.to_query()
    }

    /// Give the first row's value field the keyboard (tests).
    #[cfg(test)]
    pub(crate) fn focus_first_value(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.first() {
            row.value.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    /// What the comparison select of row `id` shows (tests).
    #[cfg(test)]
    pub(crate) fn op_shown(&self, id: u64, cx: &App) -> Option<&'static str> {
        let ix = self.index_of(id)?;
        self.rows[ix].op.read(cx).selected_value().copied()
    }

    /// The id of the row at `ix` (tests).
    #[cfg(test)]
    pub(crate) fn row_id(&self, ix: usize) -> u64 {
        self.rows[ix].id
    }

    /// The rows, as the model has them (tests).
    #[cfg(test)]
    pub(crate) fn rows(&self) -> &[BuilderRow] {
        &self.model.rows
    }

    /// The count, once taken (tests).
    #[cfg(test)]
    pub(crate) fn count(&self) -> Option<&QueryCount> {
        self.count.as_ref()
    }

    /// The suggestions shown now (tests).
    #[cfg(test)]
    pub(crate) fn suggested(&self) -> Vec<String> {
        self.suggestions.shown().to_vec()
    }

    /// The sort the model has (tests).
    #[cfg(test)]
    pub(crate) fn sort(&self) -> Option<&(SortField, SortOrder)> {
        self.model.sort.as_ref()
    }

    // ---- Rows ----------------------------------------------------------------------

    fn index_of(&self, id: u64) -> Option<usize> {
        self.rows.iter().position(|r| r.id == id)
    }

    /// Add `row` to the model with its controls.
    fn push_row(&mut self, row: BuilderRow, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.next_id;
        self.next_id += 1;
        let kinds: Vec<&'static str> = RowKind::ALL.iter().map(|k| k.label()).collect();
        let kind_ix = RowKind::ALL
            .iter()
            .position(|k| *k == row.kind)
            .unwrap_or(0);
        let ops: Vec<&'static str> = PROPERTY_OPS.iter().map(|o| property_op_label(*o)).collect();
        let op_ix = PROPERTY_OPS.iter().position(|o| *o == row.op).unwrap_or(1);
        let kind = cx.new(|cx| SelectState::new(kinds, Some(IndexPath::new(kind_ix)), window, cx));
        let op = cx.new(|cx| SelectState::new(ops, Some(IndexPath::new(op_ix)), window, cx));
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Value")
                .default_value(row.value.clone())
        });
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Property key")
                .default_value(row.key.clone())
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &kind,
                window,
                move |this, _, event: &SelectEvent<Vec<&'static str>>, window, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event {
                        this.pick_kind(id, label, window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &op,
                window,
                move |this, _, event: &SelectEvent<Vec<&'static str>>, _, cx| {
                    if let SelectEvent::Confirm(Some(label)) = event {
                        this.pick_op(id, label, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &value,
                window,
                move |this, input, event: &InputEvent, _, cx| {
                    this.field_event(id, Active::Value(id), input, event, cx)
                },
            ),
            cx.subscribe_in(
                &key,
                window,
                move |this, input, event: &InputEvent, _, cx| {
                    this.field_event(id, Active::Key(id), input, event, cx)
                },
            ),
        ];
        self.model.rows.push(row);
        self.rows.push(RowView {
            id,
            kind,
            op,
            value,
            key,
            _subscriptions: subscriptions,
        });
    }

    /// A row's text field changed or took the keyboard.
    fn field_event(
        &mut self,
        id: u64,
        field: Active,
        input: &Entity<InputState>,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.index_of(id) else { return };
        match event {
            InputEvent::Change => {
                let text = input.read(cx).value().to_string();
                let row = &mut self.model.rows[ix];
                match field {
                    Active::Key(_) => row.key = text,
                    _ => row.value = text,
                }
                self.active = Some(field);
                self.refresh_suggestions(true, cx);
                self.schedule_count(cx);
            }
            InputEvent::Focus => {
                self.active = Some(field);
                self.refresh_suggestions(true, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    /// The "Add condition" button, or Enter after a complete row.
    pub(crate) fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.push_row(BuilderRow::new(RowKind::Text), window, cx);
        if let Some(row) = self.rows.last() {
            let value = row.value.clone();
            value.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// A row's remove button.
    pub(crate) fn remove_row(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(ix) = self.index_of(id) else { return };
        self.model.rows.remove(ix);
        self.rows.remove(ix);
        self.refresh_suggestions(true, cx);
        self.schedule_count(cx);
        cx.notify();
    }

    /// A row's type pick. Raw rows can't be "not".
    pub(crate) fn pick_kind(
        &mut self,
        id: u64,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.index_of(id) else { return };
        let Some(kind) = RowKind::ALL.iter().find(|k| k.label() == label).copied() else {
            return;
        };
        let row = &mut self.model.rows[ix];
        row.kind = kind;
        if !kind.can_exclude() {
            row.exclude = false;
        }
        if kind == RowKind::Property && row.op.is_none() && row.key.is_empty() {
            row.op = Some(kimun_core::PropertyOp::Eq);
        }
        // The comparison select shows what the row now says.
        let shown = property_op_label(row.op);
        self.rows[ix].op.update(cx, |select, cx| {
            select.set_selected_value(&shown, window, cx)
        });
        self.refresh_suggestions(true, cx);
        self.schedule_count(cx);
        cx.notify();
    }

    fn pick_op(&mut self, id: u64, label: &str, cx: &mut Context<Self>) {
        let Some(ix) = self.index_of(id) else { return };
        if let Some(op) = PROPERTY_OPS
            .iter()
            .find(|o| property_op_label(**o) == label)
        {
            self.model.rows[ix].op = *op;
            self.schedule_count(cx);
            cx.notify();
        }
    }

    /// A row's "not" box.
    pub(crate) fn set_exclude(&mut self, id: u64, exclude: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.index_of(id) else { return };
        let row = &mut self.model.rows[ix];
        if row.kind.can_exclude() {
            row.exclude = exclude;
            self.schedule_count(cx);
            cx.notify();
        }
    }

    // ---- The sort ---------------------------------------------------------------------

    fn pick_sort_field(&mut self, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        let order = self.order_choice;
        let typed = self.sort_key.read(cx).value().trim().to_string();
        let field = (label != NO_SORT).then(|| field_of_label(label, typed));
        let property = matches!(field, Some(SortField::Property(_)));
        self.model.sort = field.map(|field| (field, order));
        if property {
            self.sort_key
                .update(cx, |input, cx| input.focus(window, cx));
        }
        self.schedule_count(cx);
    }

    /// The Order pick: the sort's direction, remembered for the next field
    /// picked while there is none.
    fn pick_sort_order(&mut self, order: SortOrder, cx: &mut Context<Self>) {
        self.order_choice = order;
        if let Some((_, current)) = self.model.sort.as_mut() {
            *current = order;
            self.schedule_count(cx);
        }
    }

    /// The sort key field's text, while the sort is by a property.
    fn set_sort_key(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some((SortField::Property(current), _)) = self.model.sort.as_mut() {
            *current = key;
            self.schedule_count(cx);
        }
    }

    // ---- Suggestions -------------------------------------------------------------------

    /// What the active field is: its kind's list, or `None` for no list. The
    /// notes are only those asked for this field (another's are not).
    fn suggestion_source(&self) -> Option<(&[String], String, bool)> {
        let notes: &[String] = if self.notes_for == self.active {
            &self.notes
        } else {
            &[]
        };
        match self.active? {
            Active::SortKey => {
                let typed = self.sort_key_text();
                Some((&self.keys, typed, false))
            }
            Active::Key(id) => {
                let row = self.model.rows.get(self.index_of(id)?)?;
                Some((&self.keys, row.key.clone(), false))
            }
            Active::Value(id) => {
                let row = self.model.rows.get(self.index_of(id)?)?;
                match row.kind {
                    RowKind::Tag => Some((&self.tags, row.value.clone(), false)),
                    RowKind::Folder => Some((&self.folders, row.value.clone(), false)),
                    RowKind::Name => Some((notes, row.value.clone(), false)),
                    RowKind::LinksTo | RowKind::LinkedFrom => {
                        Some((notes, row.value.clone(), true))
                    }
                    _ => None,
                }
            }
        }
    }

    fn sort_key_text(&self) -> String {
        match &self.model.sort {
            Some((SortField::Property(key), _)) => key.clone(),
            _ => String::new(),
        }
    }

    /// Recompute the suggestions for the active field. With `ask`, a note
    /// field also asks for the notes named like what is typed (not when the
    /// notes just landed).
    fn refresh_suggestions(&mut self, ask: bool, cx: &mut Context<Self>) {
        let Some((known, typed, links)) = self.suggestion_source() else {
            self.suggestions = KeySuggestions::default();
            cx.notify();
            return;
        };
        let known = known.to_vec();
        let leading =
            (links && self.note.is_some() && offers_open_note(&typed)).then_some(VAR_NOTE);
        self.suggestions
            .refresh_with_leading(&known, &typed, leading);
        if ask && self.active_takes_notes() && !typed.trim().is_empty() {
            self.ask_notes(typed, cx);
        }
        cx.notify();
    }

    /// Whether the active field is a note's name (a name, a link target).
    fn active_takes_notes(&self) -> bool {
        let Some(Active::Value(id)) = self.active else {
            return false;
        };
        self.index_of(id).is_some_and(|ix| {
            matches!(
                self.model.rows[ix].kind,
                RowKind::Name | RowKind::LinksTo | RowKind::LinkedFrom
            )
        })
    }

    /// The notes named like `typed`, off the UI thread, for the active field.
    fn ask_notes(&mut self, typed: String, cx: &mut Context<Self>) {
        self.notes_gen += 1;
        self.notes_for = self.active;
        let asked = (self.notes_gen, self.active);
        let vault = self.vault.clone();
        let task = tokio::spawn(async move {
            vault
                .suggest_notes_by_prefix(&typed, NOTE_SUGGESTIONS)
                .await
                .map(|found| found.into_iter().map(|n| n.name).collect::<Vec<_>>())
        });
        cx.spawn(async move |this, cx| {
            let Ok(names) = tasks::joined(task).await else {
                return;
            };
            this.update(cx, |dialog, cx| {
                // Only if no later keystroke and no other field overtook it.
                if (dialog.notes_gen, dialog.active) == asked {
                    dialog.notes = names;
                    dialog.refresh_suggestions(false, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// The vault's lists landed.
    fn lists_loaded(
        &mut self,
        tags: Vec<String>,
        keys: Vec<String>,
        folders: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.tags = tags;
        self.keys = keys;
        self.folders = folders;
        self.refresh_suggestions(true, cx);
    }

    /// Take suggestion `ix` into the active field.
    pub(crate) fn take_suggestion(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.suggestions.get(ix).cloned() else {
            return;
        };
        let Some(active) = self.active else { return };
        let input = match active {
            Active::SortKey => self.sort_key.clone(),
            Active::Key(id) | Active::Value(id) => {
                let Some(ix) = self.index_of(id) else { return };
                let row = &self.rows[ix];
                if matches!(active, Active::Key(_)) {
                    row.key.clone()
                } else {
                    row.value.clone()
                }
            }
        };
        // `set_value` reports no change: the model follows here.
        input.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
        match active {
            Active::SortKey => self.set_sort_key(text, cx),
            Active::Key(id) => {
                if let Some(ix) = self.index_of(id) {
                    self.model.rows[ix].key = text;
                }
            }
            Active::Value(id) => {
                if let Some(ix) = self.index_of(id) {
                    self.model.rows[ix].value = text;
                }
            }
        }
        self.suggestions = KeySuggestions::default();
        self.schedule_count(cx);
        cx.notify();
    }

    // ---- The count ---------------------------------------------------------------------

    /// Take the count again, shortly after the last edit.
    fn schedule_count(&mut self, cx: &mut Context<Self>) {
        self.count = None;
        let vault = self.vault.clone();
        let note = self.note.clone();
        self.counting = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COUNT_DEBOUNCE).await;
            let Ok(query) = this.update(cx, |dialog, _| dialog.model.to_query()) else {
                return;
            };
            let ctx = QueryContext::with_note(note);
            let counted = tokio::spawn(async move { count_query(&vault, &query, &ctx).await });
            let count = counted.await.unwrap_or(QueryCount::Failed);
            this.update(cx, |dialog, cx| {
                dialog.count = Some(count);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn count_text(&self) -> String {
        match &self.count {
            None => "Counting…".to_string(),
            Some(QueryCount::Blank) => "No conditions yet".to_string(),
            Some(QueryCount::Unresolvable) => "Needs an open note".to_string(),
            Some(QueryCount::Problem(reason)) => (*reason).to_string(),
            Some(QueryCount::Failed) => "Couldn't search".to_string(),
            Some(QueryCount::Matches(1)) => "1 note".to_string(),
            Some(QueryCount::Matches(n)) => format!("{n} notes"),
        }
    }

    // ---- Apply and close ---------------------------------------------------------------

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (query, target) = (self.query(), self.target);
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.query_builder_applied(query, target, window, cx)
            })
            .ok();
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.dismiss_query_builder(window, cx)
            })
            .ok();
    }

    /// Ctrl/Cmd+Enter applies, wherever the keyboard is in the dialog.
    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(KeyInput { key, mods }) = input::key_down_input(event) else {
            return;
        };
        if key == Key::Enter && (mods.ctrl || mods.sup) {
            self.apply(window, cx);
            cx.stop_propagation();
        }
    }

    /// Whether suggestions are showing under a field that has the keyboard:
    /// the only time Up, Down and Escape are theirs.
    fn suggestions_have_keys(&self, window: &Window, cx: &App) -> bool {
        !self.suggestions.is_empty() && self.active_focused(window, cx)
    }

    /// Whether the active field is a row's value and the row says enough.
    fn active_row_complete(&self) -> bool {
        let Some(Active::Value(id)) = self.active else {
            return false;
        };
        self.index_of(id)
            .is_some_and(|ix| self.model.rows[ix].is_complete())
    }

    /// The field the suggestions are for.
    fn active_input(&self) -> Option<&Entity<InputState>> {
        Some(match self.active? {
            Active::SortKey => &self.sort_key,
            Active::Key(id) => &self.rows[self.index_of(id)?].key,
            Active::Value(id) => &self.rows[self.index_of(id)?].value,
        })
    }

    /// Whether the active field has the keyboard.
    fn active_focused(&self, window: &Window, cx: &App) -> bool {
        self.active_input()
            .is_some_and(|input| input.read(cx).focus_handle(cx).is_focused(window))
    }

    // ---- Rendering ---------------------------------------------------------------------

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> Div {
        let view = &self.rows[ix];
        let row = &self.model.rows[ix];
        let id = view.id;
        let kind_select = div().w(rems(9.)).flex_none().child(
            Select::new(&view.kind)
                .id(("query-builder-kind", id as usize))
                .accessibility_label("Condition type"),
        );
        let not = Checkbox::new(("query-builder-not", id as usize))
            .label("Not")
            .checked(row.exclude && row.kind.can_exclude())
            .disabled(!row.kind.can_exclude())
            .on_click(
                cx.listener(move |this, checked: &bool, _, cx| this.set_exclude(id, *checked, cx)),
            );
        let remove = Button::new(("query-builder-remove", id as usize))
            .ghost()
            .small()
            .label("✕")
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.remove_row(id, cx)));
        let fields = if row.kind == RowKind::Property {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div().w(rems(9.)).flex_none().child(
                        Input::new(&view.key)
                            .id(("query-builder-key", id as usize))
                            .aria_label("Property key"),
                    ),
                )
                .child(
                    div().w(rems(5.)).flex_none().child(
                        Select::new(&view.op)
                            .id(("query-builder-op", id as usize))
                            .accessibility_label("Comparison"),
                    ),
                )
                .when(row.op.is_some(), |fields| {
                    fields.child(
                        div().flex_1().min_w_0().child(
                            Input::new(&view.value)
                                .id(builder_value_id(ix))
                                .aria_label("Value"),
                        ),
                    )
                })
        } else {
            div().flex_1().min_w_0().child(
                Input::new(&view.value)
                    .id(builder_value_id(ix))
                    .aria_label(row.kind.label()),
            )
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(not)
            .child(kind_select)
            .child(fields)
            .child(remove)
    }
}

impl Render for QueryBuilderDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let rows: Vec<Div> = (0..self.rows.len())
            .map(|ix| self.render_row(ix, cx))
            .collect();
        let property_sort = matches!(self.model.sort, Some((SortField::Property(_), _)));
        let sort_row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(row_label("Sort", t))
            .child(
                div().w(rems(9.)).flex_none().child(
                    Select::new(&self.sort_field)
                        .id(BUILDER_SORT_FIELD_ID)
                        .accessibility_label("Sort by"),
                ),
            )
            .child(
                div().w(rems(9.)).flex_none().child(
                    Select::new(&self.sort_order)
                        .id(BUILDER_SORT_ORDER_ID)
                        .accessibility_label("Order"),
                ),
            )
            .when(property_sort, |row| {
                row.child(
                    div().flex_1().min_w_0().child(
                        Input::new(&self.sort_key)
                            .id(BUILDER_SORT_KEY_ID)
                            .aria_label("Sort property key"),
                    ),
                )
            });
        // Floating under the field in use, so the dialog never resizes and
        // the list is plainly that field's.
        let suggestions = self
            .active_input()
            .filter(|input| input.read(cx).focus_handle(cx).is_focused(window))
            .map(|input| input.read(cx).input_bounds())
            .and_then(|field| {
                self.suggestions.render_floating(
                    field,
                    BUILDER_SUGGESTIONS_ID,
                    builder_suggestion_id,
                    suggestion_label,
                    t,
                    window,
                    cx,
                    |this: &mut Self, ix, window, cx| this.take_suggestion(ix, window, cx),
                    |this: &mut Self, down, cx| {
                        if this.suggestions.step(down) {
                            cx.notify();
                        }
                    },
                )
            });
        let query = self.model.to_query();
        let incomplete = self.model.incomplete();
        let count_text = self.count_text();
        let confirm = cx.entity().downgrade();
        let cancel = cx.entity().downgrade();
        div()
            .id(BUILDER_ID)
            .test_support()
            .flex()
            .flex_col()
            .gap_3()
            .on_key_down(cx.listener(Self::on_key))
            // The suggestions' keys first, then the dialog's.
            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                if this.suggestions_have_keys(window, cx) && this.suggestions.step(false) {
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                if this.suggestions_have_keys(window, cx) && this.suggestions.step(true) {
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                // Only where a field has the keyboard: a select's Enter is its
                // own. A highlighted suggestion is taken; otherwise Enter after
                // a complete value adds the next row.
                if !this.active_focused(window, cx) {
                    return;
                }
                if let Some(ix) = this.suggestions.highlighted() {
                    this.take_suggestion(ix, window, cx);
                    cx.stop_propagation();
                } else if this.active_row_complete() {
                    this.add_row(window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                // A highlight lets go first, then the suggestions, then the
                // dialog closes.
                if this.suggestions_have_keys(window, cx) {
                    if !this.suggestions.release() {
                        this.suggestions = KeySuggestions::default();
                    }
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(div().flex().flex_col().gap_2().children(rows))
            .children(suggestions)
            .child(
                div().flex().child(
                    Button::new(BUILDER_ADD_ID)
                        .small()
                        .label("Add condition")
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.add_row(window, cx)
                            }),
                        ),
                ),
            )
            .child(sort_row)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    // What the conditions write, there to read and learn the
                    // syntax from: dimmed, so it never competes with the
                    // controls.
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(row_label("Query", t))
                            .child(
                                div()
                                    .id(BUILDER_PREVIEW_ID)
                                    .test_support()
                                    .role(Role::Status)
                                    .aria_label(SharedString::from(query.clone()))
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme::muted(t))
                                    .child(if query.is_empty() {
                                        SharedString::from("none yet")
                                    } else {
                                        SharedString::from(query.clone())
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(row_label("Matches", t))
                            .child(
                                div()
                                    .id(BUILDER_COUNT_ID)
                                    .test_support()
                                    .role(Role::Status)
                                    .aria_label(SharedString::from(count_text.clone()))
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .text_color(theme::foreground(t))
                                    .child(match incomplete {
                                        0 => count_text,
                                        1 => format!(
                                            "{count_text} · 1 condition is incomplete and left out"
                                        ),
                                        n => format!(
                                            "{count_text} · {n} conditions are incomplete and left out"
                                        ),
                                    }),
                            ),
                    ),
            )
            .child(footer(
                "Apply",
                true,
                false,
                move |_, window, cx| {
                    confirm
                        .update(cx, |dialog, cx| dialog.apply(window, cx))
                        .ok();
                },
                move |_, window, cx| {
                    cancel
                        .update(cx, |dialog, cx| dialog.cancel(window, cx))
                        .ok();
                },
            ))
    }
}

/// The label column of the dialog's rows below the conditions (Sort, Query,
/// Matches), so their values line up.
fn row_label(text: &'static str, t: &Theme) -> impl IntoElement {
    div()
        .w(rems(4.5))
        .flex_none()
        .text_sm()
        .text_color(theme::muted(t))
        .child(text)
}

/// How a suggestion reads: the open note's variable as its label.
fn suggestion_label(key: &str) -> String {
    if key == VAR_NOTE {
        OPEN_NOTE_LABEL.to_string()
    } else {
        key.to_string()
    }
}

// ---- The workspace's side ---------------------------------------------------------------

/// The open builder, while one is.
pub(crate) struct QueryBuilderOpen {
    pub(crate) dialog: WeakEntity<QueryBuilderDialog>,
    /// Over the search browser (the browser keeps the modal slot).
    pub(crate) stacked: bool,
}

impl Workspace {
    /// Whether the builder is stacked over the search browser.
    pub(crate) fn query_builder_stacked(&self) -> bool {
        self.query_builder.as_ref().is_some_and(|open| open.stacked)
    }

    /// The `QueryBuilder` chord or a `Build…` button: build FIND's query, or
    /// the search browser's, over the browser. Open already: closed.
    pub(crate) fn open_query_builder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.query_builder.is_some() && modal::dialog_open(window, cx) {
            self.dismiss_query_builder(window, cx);
            return;
        }
        let stacked = self.open_modal(window, cx) == Some(ModalKind::Search);
        let allowed = if stacked {
            !self.query_help && self.sort.is_none() && !self.picker_busy(cx)
        } else {
            !modal::dialog_open(window, cx)
        };
        if !allowed {
            return;
        }
        let Some(vault) = self.vault_or_flash(cx) else {
            return;
        };
        let (query, target) = if stacked {
            let Some(picker) = self.picker.as_ref().and_then(|p| p.upgrade()) else {
                return;
            };
            (
                picker.read(cx).input.read(cx).value().to_string(),
                BuilderTarget::Browser,
            )
        } else {
            let Some(find) = self.find.clone() else {
                self.flash(super::route::NO_WORKSPACE, cx);
                return;
            };
            (find.read(cx).query(cx), BuilderTarget::Find)
        };
        let note = self
            .active_state()
            .notes
            .as_ref()
            .and_then(|h| h.note.clone());
        let workspace = cx.entity().downgrade();
        let view = cx.new(|cx| {
            QueryBuilderDialog::new(
                &query,
                target,
                vault.clone(),
                note,
                workspace.clone(),
                window,
                cx,
            )
        });
        // The lists, read off the UI thread.
        let lists = {
            let vault = vault.clone();
            tokio::spawn(async move {
                let tags = vault.list_labels().await.unwrap_or_default();
                let keys = vault.property_keys().await.unwrap_or_default();
                (tags, keys)
            })
        };
        let folders = tasks::blocking(move || folder_terms(&vault));
        let loading = view.downgrade();
        cx.spawn(async move |_, cx| {
            let (tags, keys) = lists.await.unwrap_or_default();
            let folders = folders.await.unwrap_or_default();
            loading
                .update(cx, |dialog, cx| {
                    dialog.lists_loaded(tags, keys, folders, cx)
                })
                .ok();
        })
        .detach();
        self.query_builder = Some(QueryBuilderOpen {
            dialog: view.downgrade(),
            stacked,
        });
        let closed = workspace.clone();
        let mine = view.downgrade();
        let first = view.read(cx).rows.first().map(|r| r.value.clone());
        self.show_modal(
            if stacked {
                Slot::Stacked
            } else {
                Slot::Own(ModalKind::QueryBuilder)
            },
            DIALOG_REMS,
            move |window, cx| {
                if let Some(first) = first {
                    first.update(cx, |input, cx| input.focus(window, cx));
                }
            },
            move |dialog, _, _| {
                let closed = closed.clone();
                let mine = mine.clone();
                let cancel = closed.clone();
                dialog
                    .title("Build query")
                    .on_cancel(move |_, window, cx| {
                        cancel
                            .update(cx, |workspace, cx| {
                                workspace.dismiss_query_builder(window, cx)
                            })
                            .ok();
                        false
                    })
                    .on_close(move |_, _, cx| {
                        closed
                            .update(cx, |workspace, _| {
                                // Only this dialog's record (a newer one may
                                // be open by now).
                                if workspace
                                    .query_builder
                                    .as_ref()
                                    .is_some_and(|open| open.dialog == mine)
                                {
                                    workspace.query_builder = None;
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

    /// Cancel, Escape or the backdrop: close the builder; over the browser
    /// only it goes, and the browser has the keyboard again.
    pub(crate) fn dismiss_query_builder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.query_builder.take() {
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

    /// Apply: close the builder, then put `query` in the field it came from
    /// and run it.
    pub(crate) fn query_builder_applied(
        &mut self,
        query: String,
        target: BuilderTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_query_builder(window, cx);
        match target {
            BuilderTarget::Find => {
                if let Some(find) = self.find.clone() {
                    find.update(cx, |view, cx| view.apply_query(query, None, window, cx));
                }
            }
            BuilderTarget::Browser => {
                if let Some(picker) = self.picker.as_ref().and_then(|p| p.upgrade()) {
                    picker.update(cx, |picker, cx| picker.apply_query(query, window, cx));
                }
            }
        }
    }
}

/// The vault's folders as a `/` term writes them: a path without its leading
/// separator, the root left out.
fn folder_terms(vault: &NoteVault) -> Vec<String> {
    kimun_notes::logic::file_ops::list_directories(vault)
        .unwrap_or_default()
        .into_iter()
        .filter(|dir| !dir.is_root_or_empty())
        .map(|mut dir| {
            dir.to_relative();
            dir.to_string()
        })
        .collect()
}
