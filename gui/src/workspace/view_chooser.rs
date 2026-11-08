//! The **view chooser** (ADR-0054 §1, §4): a modal list of rows, each a
//! label and what picking it means, on the kit list the palette uses
//! (`Command` over a `CommandState`, unbordered). Split asks with it which
//! view the new slot holds ([`Workspace::open_split_chooser`]).
//!
//! **Keys.** Up/Down move the highlight, Enter picks, Escape clears a typed
//! query first and then cancels; typing narrows the rows (the kit's own
//! filter). Every key is the chooser's while it is open (the modal host).
//! A click picks a row too.
//!
//! **Events.** A pick emits [`ChooserEvent::Picked`] with the row's index
//! (its place in the rows given, whatever the query hides), Escape on an
//! empty query [`ChooserEvent::Cancelled`]. The workspace closes the
//! modal and acts on the pick.

use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div,
};
use kimun_notes::logic::layout::Dir;
use kimun_notes::logic::layout_tree::{Focus, Leaf, SlotId};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;

use super::Workspace;
use super::files::ClickPolicy;
use super::modal::{ModalKind, Slot};

/// The chooser's width, in rems.
const WIDTH_REMS: f32 = 24.;

/// What the chooser reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChooserEvent {
    /// The row at this index (into the rows given) was picked.
    Picked(usize),
    /// Escape on an empty query: nothing picked.
    Cancelled,
}

/// The chooser's view: the kit list's state and its rows.
pub(crate) struct ViewChooser<P: 'static> {
    state: Entity<CommandState>,
    rows: Vec<(SharedString, P)>,
    placeholder: SharedString,
}

impl<P: 'static> EventEmitter<ChooserEvent> for ViewChooser<P> {}

impl<P: 'static> ViewChooser<P> {
    pub(crate) fn new(
        rows: Vec<(SharedString, P)>,
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            state: cx.new(|cx| CommandState::new(window, cx)),
            rows,
            placeholder: placeholder.into(),
        }
    }

    /// What the row at `index` stands for.
    pub(crate) fn payload(&self, index: usize) -> Option<&P> {
        self.rows.get(index).map(|(_, payload)| payload)
    }

    /// Every row's payload, in list order.
    #[cfg(test)]
    pub(crate) fn payloads(&self) -> impl Iterator<Item = &P> {
        self.rows.iter().map(|(_, payload)| payload)
    }

    /// The query typed so far.
    #[cfg(test)]
    pub(crate) fn query(&self, cx: &gpui_kit::App) -> String {
        self.state.read(cx).query(cx).to_string()
    }
}

impl<P: 'static> Render for ViewChooser<P> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let picked = cx.entity().downgrade();
        let cancelled = picked.clone();
        let cursor = ClickPolicy::Open.cursor(true);
        Command::new(&self.state)
            .bordered(false)
            .placeholder(self.placeholder.clone())
            .items(self.rows.iter().map(|(label, _)| {
                // A click picks a row (the kit's); the pointing hand says so.
                let text = label.clone();
                CommandItem::new().label(label.clone()).child(move |_, _| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .when_some(cursor, |row, cursor| row.cursor(cursor))
                        .child(text.clone())
                })
            }))
            .on_confirm(move |index, _, cx| {
                picked
                    .update(cx, |_, cx| cx.emit(ChooserEvent::Picked(index.row)))
                    .ok();
            })
            .on_cancel(move |_, cx| {
                cancelled
                    .update(cx, |_, cx| cx.emit(ChooserEvent::Cancelled))
                    .ok();
            })
    }
}

/// The split chooser while it is open: where the picked view goes.
pub(crate) struct SplitChooser {
    chooser: Entity<ViewChooser<DrawerView>>,
    /// The slot split.
    slot: SlotId,
    /// The new slot's side of it.
    dir: Dir,
    _events: Subscription,
}

impl Workspace {
    /// `SplitSlotRight` / `SplitSlotDown` (ADR-0054 §1): ask which view the
    /// new slot beside `slot` holds, `dir` its side. The rows are the views
    /// the rail offers now that no slot shows, in registry order; with none,
    /// the split says so and does nothing. Refused under another modal.
    pub(crate) fn open_split_chooser(
        &mut self,
        slot: SlotId,
        dir: Dir,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if super::modal::dialog_open(window, cx) {
            return;
        }
        let rows: Vec<(SharedString, DrawerView)> = self
            .rail_views()
            .iter()
            .filter(|view| !self.is_shown(**view))
            .map(|view| (view_registry::spec(*view).name.into(), *view))
            .collect();
        if rows.is_empty() {
            self.flash("Every view is already shown", cx);
            return;
        }
        let chooser = cx.new(|cx| ViewChooser::new(rows, "Show in the new slot", window, cx));
        let events = cx.subscribe_in(&chooser, window, Self::on_split_chooser);
        self.split_chooser = Some(SplitChooser {
            chooser: chooser.clone(),
            slot,
            dir,
            _events: events,
        });
        let state = chooser.read(cx).state.clone();
        self.show_modal(
            Slot::Own(ModalKind::ViewChooser),
            WIDTH_REMS,
            move |window, cx| state.update(cx, |state, cx| state.focus(window, cx)),
            move |dialog, _, _| dialog.p_0().child(chooser.clone()),
            window,
            cx,
        );
    }

    /// The split chooser answered: on a pick, close it and place the view
    /// in a new slot beside the one split ([`Self::place_picked`]).
    fn on_split_chooser(
        &mut self,
        _: &Entity<ViewChooser<DrawerView>>,
        event: &ChooserEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.split_chooser.take() else {
            return;
        };
        let ChooserEvent::Picked(index) = *event else {
            return;
        };
        let Some(view) = open.chooser.read(cx).payload(index).copied() else {
            return;
        };
        self.close_modal(window, cx);
        self.place_picked(view, open.slot, open.dir, window, cx);
    }

    /// Put `view` in a new slot on the `dir` side of `slot`
    /// ([`LayoutTree::place_beside`]); the view leaves the slot it was
    /// hidden in, which closes if that emptied it
    /// ([`Self::close_if_vacated`]). The keyboard goes to the new slot.
    /// Persisted. Nothing changes when `slot` went away meanwhile.
    ///
    /// [`LayoutTree::place_beside`]: kimun_notes::logic::layout_tree::LayoutTree::place_beside
    fn place_picked(
        &mut self,
        view: DrawerView,
        slot: SlotId,
        dir: Dir,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from = self.layout.view_slot(&view);
        let Ok(new) = self.layout.place_beside(view, slot, dir) else {
            return;
        };
        if let Some(from) = from {
            self.close_if_vacated(from, view);
        }
        self.layout.focus(Focus::Leaf(Leaf::Slot(new)));
        self.ensure_files_listing(cx);
        self.tidy_layout(cx);
        self.apply_focus(window, cx);
        self.save_layout(cx);
        cx.notify();
    }

    /// The views the open split chooser offers, in its order.
    #[cfg(test)]
    pub(crate) fn split_chooser_views(&self, cx: &gpui_kit::App) -> Option<Vec<DrawerView>> {
        let open = self.split_chooser.as_ref()?;
        Some(open.chooser.read(cx).payloads().copied().collect())
    }

    /// The query typed in the open split chooser.
    #[cfg(test)]
    pub(crate) fn split_chooser_query(&self, cx: &gpui_kit::App) -> Option<String> {
        let open = self.split_chooser.as_ref()?;
        Some(open.chooser.read(cx).query(cx))
    }
}
