//! The **view chooser** (ADR-0054 §1, §4): a modal list of rows, each a
//! label, the chord that does the same where one does, and what picking
//! it means, on the kit list the palette uses (`Command` over a
//! `CommandState`, unbordered). Split asks with it which view the new slot
//! holds ([`Workspace::open_split_chooser`]); "Move view to…" where the
//! focused view goes ([`Workspace::open_move_chooser`]).
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
use gpui_kit::component::kbd::Kbd;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div,
};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::input::KeyInput;
use kimun_notes::logic::layout::Dir;
use kimun_notes::logic::layout_tree::{Focus, Leaf, SlotId};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;

use super::Workspace;
use super::files::ClickPolicy;
use super::modal::{ModalKind, Slot};
use crate::{input, keymap};

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

/// One row of the chooser: its label, the chord that does the same (for
/// the trailing slot), and what picking it means.
pub(crate) struct ChooserRow<P> {
    label: SharedString,
    hint: Option<KeyInput>,
    payload: P,
}

impl<P> ChooserRow<P> {
    pub(crate) fn new(label: impl Into<SharedString>, payload: P) -> Self {
        Self {
            label: label.into(),
            hint: None,
            payload,
        }
    }

    /// The chord shown in the row's trailing slot.
    pub(crate) fn hint(mut self, hint: Option<KeyInput>) -> Self {
        self.hint = hint;
        self
    }
}

/// The chooser's view: the kit list's state and its rows.
pub(crate) struct ViewChooser<P: 'static> {
    state: Entity<CommandState>,
    rows: Vec<ChooserRow<P>>,
    placeholder: SharedString,
}

impl<P: 'static> EventEmitter<ChooserEvent> for ViewChooser<P> {}

impl<P: 'static> ViewChooser<P> {
    pub(crate) fn new(
        rows: Vec<ChooserRow<P>>,
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
        self.rows.get(index).map(|row| &row.payload)
    }

    /// Every row's payload, in list order.
    #[cfg(test)]
    pub(crate) fn payloads(&self) -> impl Iterator<Item = &P> {
        self.rows.iter().map(|row| &row.payload)
    }

    /// Every row's label, in list order.
    #[cfg(test)]
    pub(crate) fn labels(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|row| row.label.as_ref())
    }

    /// Every row's chord, in list order.
    #[cfg(test)]
    pub(crate) fn hints(&self) -> impl Iterator<Item = Option<KeyInput>> {
        self.rows.iter().map(|row| row.hint)
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
            .items(self.rows.iter().map(|row| {
                // A click picks a row (the kit's); the pointing hand says so.
                let text = row.label.clone();
                let hint = row.hint;
                CommandItem::new()
                    .label(row.label.clone())
                    .child(move |_, _| {
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .when_some(cursor, |row, cursor| row.cursor(cursor))
                            .child(div().flex_1().min_w_0().truncate().child(text.clone()))
                            .when_some(hint, |row, chord| {
                                row.child(Kbd::new(input::keystroke(chord)))
                            })
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

/// Where a move chooser row sends the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveTarget {
    /// Into this other drawn slot.
    Into(SlotId),
    /// Into a new slot on this side of the view's slot.
    New(Dir),
}

/// The four sides a move chord points to, in the order the rows name them.
const DIRS: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

/// The move chooser's new-slot rows, one per side.
const NEW_SIDES: [(Dir, &str); 4] = [
    (Dir::Up, "New slot above"),
    (Dir::Down, "New slot below"),
    (Dir::Left, "New slot to the left"),
    (Dir::Right, "New slot to the right"),
];

/// What a slot opened empty is called in its header and in the rows.
pub(crate) const EMPTY_TITLE: &str = "Empty";

/// The move chooser while it is open: whose view moves.
pub(crate) struct MoveChooser {
    chooser: Entity<ViewChooser<MoveTarget>>,
    /// The slot the view is moved out of.
    slot: SlotId,
    /// The view it showed when the chooser opened.
    view: DrawerView,
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
        let rows: Vec<ChooserRow<DrawerView>> = self
            .rail_views()
            .iter()
            .filter(|view| !self.is_shown(**view))
            .map(|view| ChooserRow::new(view_registry::spec(*view).name, *view))
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

    /// "Move view to…" (ADR-0054 §4): ask where the view `slot` shows goes.
    /// The rows ([`Self::move_rows`]): each other drawn slot, then, when
    /// the view shares `slot` with others, a new slot on each side of it.
    /// With no row the command says so and opens nothing. Nothing happens
    /// while zoomed (the other slots are not on screen), for a slot showing
    /// no view, or under another modal.
    pub(crate) fn open_move_chooser(
        &mut self,
        slot: SlotId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if super::modal::dialog_open(window, cx) || self.layout.zoomed().is_some() {
            return;
        }
        let Some(view) = self.layout.shown(slot) else {
            return;
        };
        let rows = self.move_rows(view, slot, cx);
        if rows.is_empty() {
            self.flash("Nowhere else to move this view", cx);
            return;
        }
        let chooser = cx.new(|cx| ViewChooser::new(rows, "Move the view to", window, cx));
        let events = cx.subscribe_in(&chooser, window, Self::on_move_chooser);
        self.move_chooser = Some(MoveChooser {
            chooser: chooser.clone(),
            slot,
            view,
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

    /// Where `view`, shown in `slot`, can go: `Into <title> slot` for each
    /// other drawn slot in reading order (titled by the view it shows),
    /// then a new slot above, below, left and right of `slot` when `slot`
    /// holds other views (alone there, there is nothing to leave behind).
    /// A row names the move chord that does the same: the slot a chord
    /// reaches, or the new slot on a side with nothing there.
    fn move_rows(
        &self,
        view: DrawerView,
        slot: SlotId,
        cx: &gpui_kit::App,
    ) -> Vec<ChooserRow<MoveTarget>> {
        let keymap = keymap::active(cx);
        let chord = |dir: Dir| keymap.chord_for(&ActionShortcuts::MoveViewToNeighbour(dir));
        // Where the move chord toward `dir` takes the view; `None`: a new
        // slot on that side (up and down make one rather than land on a
        // slot that is not drawn).
        let reach = |dir: Dir| {
            let mut tree = self.layout.clone();
            let to = tree
                .move_view_dir(&view, dir)
                .then(|| tree.view_slot(&view))
                .flatten()?;
            match dir {
                Dir::Up | Dir::Down => self.slot_drawn(to).then_some(to),
                Dir::Left | Dir::Right => Some(to),
            }
        };
        let mut rows: Vec<ChooserRow<MoveTarget>> = self
            .layout
            .slots()
            .into_iter()
            .filter(|other| *other != slot && self.slot_drawn(*other))
            .map(|other| {
                let title = self
                    .layout
                    .shown(other)
                    .map_or(EMPTY_TITLE, |shown| view_registry::spec(shown).title);
                let hint = DIRS
                    .into_iter()
                    .find(|dir| reach(*dir) == Some(other))
                    .and_then(chord);
                ChooserRow::new(format!("Into {title} slot"), MoveTarget::Into(other)).hint(hint)
            })
            .collect();
        if self.layout.slot_views(slot).len() > 1 {
            rows.extend(NEW_SIDES.into_iter().map(|(dir, label)| {
                let hint = reach(dir).is_none().then(|| chord(dir)).flatten();
                ChooserRow::new(label, MoveTarget::New(dir)).hint(hint)
            }));
        }
        rows
    }

    /// The move chooser answered: on a pick, close it and move the view
    /// ([`Self::move_picked`]).
    fn on_move_chooser(
        &mut self,
        _: &Entity<ViewChooser<MoveTarget>>,
        event: &ChooserEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.move_chooser.take() else {
            return;
        };
        let ChooserEvent::Picked(index) = *event else {
            return;
        };
        let Some(target) = open.chooser.read(cx).payload(index).copied() else {
            return;
        };
        self.close_modal(window, cx);
        self.move_picked(open.view, open.slot, target, window, cx);
    }

    /// Move `view` out of `from` to `target`. Into a slot: shown there,
    /// the view shown there staying placed, hidden (as the move chord
    /// does); `from` closes when that emptied it
    /// ([`Self::close_if_vacated`]). A new slot: on that side of `from`
    /// ([`Self::spawn_beside`]). The keyboard follows the view. Persisted.
    /// Nothing changes when the view left `from` meanwhile, or while
    /// zoomed.
    fn move_picked(
        &mut self,
        view: DrawerView,
        from: SlotId,
        target: MoveTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.layout.view_slot(&view) != Some(from) || self.layout.zoomed().is_some() {
            return;
        }
        match target {
            MoveTarget::New(dir) => {
                self.spawn_beside(view, from, dir, window, cx);
            }
            MoveTarget::Into(slot) => {
                if !self.layout.move_view(view, slot) {
                    return;
                }
                self.close_if_vacated(from, view);
                self.layout.focus(Focus::Leaf(Leaf::Slot(slot)));
                self.ensure_files_listing(cx);
                self.tidy_layout(cx);
                self.apply_focus(window, cx);
                self.save_layout(cx);
                cx.notify();
            }
        }
    }

    /// The labels the open move chooser offers, in its order.
    #[cfg(test)]
    pub(crate) fn move_chooser_rows(&self, cx: &gpui_kit::App) -> Option<Vec<String>> {
        let open = self.move_chooser.as_ref()?;
        Some(open.chooser.read(cx).labels().map(str::to_owned).collect())
    }

    /// The chords the open move chooser's rows name, in its order.
    #[cfg(test)]
    pub(crate) fn move_chooser_hints(&self, cx: &gpui_kit::App) -> Option<Vec<Option<KeyInput>>> {
        let open = self.move_chooser.as_ref()?;
        Some(open.chooser.read(cx).hints().collect())
    }
}
