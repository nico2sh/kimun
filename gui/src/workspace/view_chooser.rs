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
use kimun_notes::logic::layout_tree::{Leaf, SlotId};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::view_registry;

use super::Workspace;
use super::drop_zones::{NEW_SLOT_DIRS, new_slot_label};
use super::files::ClickPolicy;
use super::modal::{ModalKind, Slot};
use super::placement::Placement;
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

/// A chooser while it is open: its list, what opening it was for (`C`:
/// an [`UnshownFor`], or the slot and view moved), and the subscription to
/// its answer. The unshown-views and move choosers are this with their own
/// payload and context.
pub(crate) struct OpenChooser<P: 'static, C> {
    chooser: Entity<ViewChooser<P>>,
    ctx: C,
    _events: Subscription,
}

impl<P: Clone + 'static, C> OpenChooser<P, C> {
    /// What the chooser answered: the context it opened with and the
    /// picked row's payload; `None` for a cancel (or a row gone).
    fn answer(self, event: &ChooserEvent, cx: &gpui_kit::App) -> Option<(C, P)> {
        let ChooserEvent::Picked(index) = *event else {
            return None;
        };
        let payload = self.chooser.read(cx).payload(index).cloned()?;
        Some((self.ctx, payload))
    }
}

/// What an unshown-views chooser is for, and so what a pick does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnshownFor {
    /// Which view the new slot on the `Dir` side of the `SlotId` holds
    /// (`SplitSlotRight` / `SplitSlotDown`, ADR-0054 §1).
    Split(SlotId, Dir),
    /// Which view the first slot of a side just toggled on shows
    /// (ADR-0054 §5).
    Side(SlotId),
}

impl UnshownFor {
    /// Where a pick places the view.
    fn placement(self) -> Placement {
        match self {
            UnshownFor::Split(slot, dir) => Placement::Beside(slot, dir),
            UnshownFor::Side(slot) => Placement::Into(slot),
        }
    }

    /// The chooser's search placeholder.
    fn placeholder(self) -> &'static str {
        match self {
            UnshownFor::Split(..) => "Show in the new slot",
            UnshownFor::Side(_) => "Show in this side",
        }
    }
}

/// The chooser of the views no slot shows: payload the view, context what
/// the pick is for.
type UnshownChooser = OpenChooser<DrawerView, UnshownFor>;

impl MoveTarget {
    /// Where a pick places the view moved out of `from`.
    fn placement(self, from: SlotId) -> Placement {
        match self {
            MoveTarget::Into(slot) => Placement::Into(slot),
            MoveTarget::New(dir) => Placement::Beside(from, dir),
        }
    }
}

/// The move chooser: where the view goes. Context: the slot the view is
/// moved out of and the view it showed when the chooser opened.
pub(crate) type MoveChooser = OpenChooser<MoveTarget, (SlotId, DrawerView)>;

/// The workspace's one open view chooser. Each takes the modal, so at most
/// one is open; a field per kind would be cleared together anyway
/// ([`Workspace::reset_modal_state`]).
pub(crate) enum OpenViewChooser {
    Unshown(UnshownChooser),
    Move(MoveChooser),
}

/// Where a move chooser row sends the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveTarget {
    /// Into this other drawn slot.
    Into(SlotId),
    /// Into a new slot on this side of the view's slot.
    New(Dir),
}

/// A chooser's answer handler on the workspace.
type AnswerFn<P> = fn(
    &mut Workspace,
    &Entity<ViewChooser<P>>,
    &ChooserEvent,
    &mut Window,
    &mut Context<Workspace>,
);

impl Workspace {
    /// Open a chooser over `rows` as the modal: the list takes the keyboard,
    /// its answer goes to `on_answer`. The caller keeps the result in its
    /// field, which [`Self::reset_modal_state`] clears with the modal.
    fn open_chooser<P: Clone + 'static, C>(
        &mut self,
        rows: Vec<ChooserRow<P>>,
        placeholder: &'static str,
        ctx: C,
        on_answer: AnswerFn<P>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> OpenChooser<P, C> {
        let chooser = cx.new(|cx| ViewChooser::new(rows, placeholder, window, cx));
        let events = cx.subscribe_in(&chooser, window, on_answer);
        let state = chooser.read(cx).state.clone();
        let shown = chooser.clone();
        self.show_modal(
            Slot::Own(ModalKind::ViewChooser),
            WIDTH_REMS,
            move |window, cx| state.update(cx, |state, cx| state.focus(window, cx)),
            move |dialog, _, _| dialog.p_0().child(shown.clone()),
            window,
            cx,
        );
        OpenChooser {
            chooser,
            ctx,
            _events: events,
        }
    }

    /// `SplitSlotRight` / `SplitSlotDown` (ADR-0054 §1): ask which view the
    /// new slot beside `slot` holds, `dir` its side
    /// ([`Self::open_unshown_chooser`]). Refused (false) under another modal.
    pub(crate) fn open_split_chooser(
        &mut self,
        slot: SlotId,
        dir: Dir,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.open_unshown_chooser(UnshownFor::Split(slot, dir), window, cx)
    }

    /// A side toggled on with nothing placed there (ADR-0054 §5): ask which
    /// view `slot`, the side's first, shows
    /// ([`Self::open_unshown_chooser`]). Cancelling opens nothing.
    pub(crate) fn open_side_chooser(
        &mut self,
        slot: SlotId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_unshown_chooser(UnshownFor::Side(slot), window, cx);
    }

    /// Ask which of the views the rail offers that no slot shows (in
    /// registry order) `purpose` takes. With none it says so and opens
    /// nothing. False, nothing done, under another modal.
    fn open_unshown_chooser(
        &mut self,
        purpose: UnshownFor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if super::modal::dialog_open(window, cx) {
            return false;
        }
        let rows = self.unshown_rows();
        if rows.is_empty() {
            self.flash("Every view is already shown", cx);
            return true;
        }
        let open = self.open_chooser(
            rows,
            purpose.placeholder(),
            purpose,
            Self::on_unshown_chooser,
            window,
            cx,
        );
        self.view_chooser = Some(OpenViewChooser::Unshown(open));
        true
    }

    /// The unshown-views chooser answered: on a pick, close it and do what
    /// it was for ([`Self::place`]): nothing when the view got shown
    /// elsewhere meanwhile.
    fn on_unshown_chooser(
        &mut self,
        _: &Entity<ViewChooser<DrawerView>>,
        event: &ChooserEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(OpenViewChooser::Unshown(open)) =
            self.take_view_chooser_if(|c| matches!(c, OpenViewChooser::Unshown(_)))
        else {
            return;
        };
        let Some((purpose, view)) = open.answer(event, cx) else {
            return;
        };
        self.close_modal(window, cx);
        // As `move_picked`: the layout may have changed under the chooser.
        if self.is_shown(view) {
            return;
        }
        self.place(view, purpose.placement(), window, cx);
    }

    /// Take the open chooser when `of_kind` says it is the one an answer is
    /// for; a chooser of another kind stays open.
    fn take_view_chooser_if(
        &mut self,
        of_kind: impl Fn(&OpenViewChooser) -> bool,
    ) -> Option<OpenViewChooser> {
        if self.view_chooser.as_ref().is_some_and(of_kind) {
            self.view_chooser.take()
        } else {
            None
        }
    }

    /// Move `view` into `slot` ([`LayoutTree::move_view`]): shown there,
    /// the keyboard on `slot`. Persisted. False when `slot` is absent.
    ///
    /// A view shown in another slot trades places with the view `slot`
    /// shows: that view takes the slot `view` left, so both stay drawn and
    /// no slot closes. Any other move (a hidden view, a view no slot holds,
    /// a `slot` showing nothing) puts `view` in `slot`, the view shown there
    /// staying placed, hidden; the slot `view` leaves closes if that emptied
    /// it ([`Self::close_if_vacated`]).
    ///
    /// [`LayoutTree::move_view`]: kimun_notes::logic::layout_tree::LayoutTree::move_view
    pub(super) fn move_into(
        &mut self,
        view: DrawerView,
        slot: SlotId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let from = self.layout.view_slot(&view).filter(|from| *from != slot);
        let swap_with = self.swap_partner(view, slot);
        if !self.layout.move_view(view, slot) {
            return false;
        }
        match (from, swap_with) {
            (Some(from), Some(other)) => {
                let moved = self.layout.move_view(other, from);
                debug_assert!(moved, "the slot the view left is still there");
                // The zoom follows the view, as it does when its slot closes.
                if self.layout.zoomed() == Some(Leaf::Slot(from)) {
                    self.layout.zoom(Some(Leaf::Slot(slot)));
                }
            }
            (Some(from), None) => {
                self.close_if_vacated(from, view);
            }
            _ => {}
        }
        self.focus_slot_ending_zoom(slot);
        self.ensure_files_listing(cx);
        self.tidy_layout(cx);
        self.apply_focus(window, cx);
        self.save_layout(cx);
        cx.notify();
        true
    }

    /// The view `view` would trade places with if moved into `slot`: the
    /// one `slot` shows, when `view` is shown in another slot
    /// ([`Self::move_into`]).
    pub(super) fn swap_partner(&self, view: DrawerView, slot: SlotId) -> Option<DrawerView> {
        self.layout
            .view_slot(&view)
            .filter(|from| *from != slot)
            .filter(|from| self.layout.shown(*from) == Some(view))?;
        self.layout.shown(slot)
    }

    /// A rail drag onto a slot's centre: like [`Self::move_into`], but the
    /// view `slot` shows is unplaced instead of swapped or kept hidden
    /// there, so the rail still lists it as any view no slot holds. False
    /// when `slot` is absent.
    pub(super) fn replace_into(
        &mut self,
        view: DrawerView,
        slot: SlotId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.layout.slots().contains(&slot) {
            return false;
        }
        if let Some(shown) = self.layout.shown(slot).filter(|shown| *shown != view) {
            self.layout.unplace(&shown);
        }
        self.move_into(view, slot, window, cx)
    }

    /// The rows of the views the rail offers that no slot shows, in
    /// registry order.
    fn unshown_rows(&self) -> Vec<ChooserRow<DrawerView>> {
        self.rail_views()
            .iter()
            .filter(|view| !self.is_shown(**view))
            .map(|view| ChooserRow::new(view_registry::spec(*view).name, *view))
            .collect()
    }

    /// The open unshown-views chooser, when its purpose is one `want` accepts.
    #[cfg(test)]
    fn unshown_chooser(
        &self,
        want: impl Fn(&UnshownFor) -> bool,
    ) -> Option<(UnshownFor, &Entity<ViewChooser<DrawerView>>)> {
        match self.view_chooser.as_ref()? {
            OpenViewChooser::Unshown(open) if want(&open.ctx) => Some((open.ctx, &open.chooser)),
            _ => None,
        }
    }

    /// The open move chooser's list.
    #[cfg(test)]
    fn open_move_chooser_list(&self) -> Option<&Entity<ViewChooser<MoveTarget>>> {
        match self.view_chooser.as_ref()? {
            OpenViewChooser::Move(open) => Some(&open.chooser),
            OpenViewChooser::Unshown(_) => None,
        }
    }

    /// The views the open split chooser offers, in its order.
    #[cfg(test)]
    pub(crate) fn split_chooser_views(&self, cx: &gpui_kit::App) -> Option<Vec<DrawerView>> {
        let (_, chooser) = self.unshown_chooser(|p| matches!(p, UnshownFor::Split(..)))?;
        Some(chooser.read(cx).payloads().copied().collect())
    }

    /// The views the open side chooser offers, in its order.
    #[cfg(test)]
    pub(crate) fn side_chooser_views(&self, cx: &gpui_kit::App) -> Option<Vec<DrawerView>> {
        let (_, chooser) = self.unshown_chooser(|p| matches!(p, UnshownFor::Side(_)))?;
        Some(chooser.read(cx).payloads().copied().collect())
    }

    /// The query typed in the open split chooser.
    #[cfg(test)]
    pub(crate) fn split_chooser_query(&self, cx: &gpui_kit::App) -> Option<String> {
        let (_, chooser) = self.unshown_chooser(|p| matches!(p, UnshownFor::Split(..)))?;
        Some(chooser.read(cx).query(cx))
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
        let open = self.open_chooser(
            rows,
            "Move the view to",
            (slot, view),
            Self::on_move_chooser,
            window,
            cx,
        );
        self.view_chooser = Some(OpenViewChooser::Move(open));
    }

    /// Where `view`, shown in `slot`, can go: `Into <title> slot` for each
    /// other drawn slot in reading order (titled by the view it shows),
    /// then a new slot above, below, left and right of `slot` when `slot`
    /// holds other views (alone there, there is nothing to leave behind).
    /// A row names the move chord that does the same: the slot an Up or
    /// Down chord reaches (both trade places, as a pick does; Left and
    /// Right join that slot's stack instead), or the new slot on a side
    /// with nothing there.
    fn move_rows(
        &self,
        view: DrawerView,
        slot: SlotId,
        cx: &gpui_kit::App,
    ) -> Vec<ChooserRow<MoveTarget>> {
        let keymap = keymap::active(cx);
        let chord = |dir: Dir| keymap.chord_for(&ActionShortcuts::MoveViewToNeighbour(dir));
        // Where the move chord toward each side takes the view; `None`: a
        // new slot on that side (up and down make one rather than land on
        // a slot that is not drawn). Worked out once, each is a layout pass.
        let reaches = NEW_SLOT_DIRS.map(|dir| {
            let mut tree = self.layout.clone();
            let to = tree
                .move_view_dir(&view, dir)
                .then(|| tree.view_slot(&view))
                .flatten()
                .filter(|to| match dir {
                    Dir::Up | Dir::Down => self.slot_drawn(*to),
                    Dir::Left | Dir::Right => true,
                });
            (dir, to)
        });
        let reach = |dir: Dir| {
            reaches
                .iter()
                .find(|(d, _)| *d == dir)
                .and_then(|(_, to)| *to)
        };
        let mut rows: Vec<ChooserRow<MoveTarget>> = self
            .layout
            .slots()
            .into_iter()
            .filter(|other| *other != slot)
            .filter_map(|other| Some((other, self.layout.shown(other)?)))
            .map(|(other, shown)| {
                let title = view_registry::spec(shown).title;
                // Only Up and Down trade places in the slot they reach, as
                // the pick does; Left and Right join its stack instead.
                let hint = [Dir::Up, Dir::Down]
                    .into_iter()
                    .find(|dir| reach(*dir) == Some(other))
                    .and_then(chord);
                ChooserRow::new(format!("Into {title} slot"), MoveTarget::Into(other)).hint(hint)
            })
            .collect();
        if self.layout.slot_views(slot).len() > 1 {
            rows.extend(NEW_SLOT_DIRS.into_iter().map(|dir| {
                let hint = reach(dir).is_none().then(|| chord(dir)).flatten();
                ChooserRow::new(new_slot_label(dir), MoveTarget::New(dir)).hint(hint)
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
        let Some(OpenViewChooser::Move(open)) =
            self.take_view_chooser_if(|c| matches!(c, OpenViewChooser::Move(_)))
        else {
            return;
        };
        let Some(((slot, view), target)) = open.answer(event, cx) else {
            return;
        };
        self.close_modal(window, cx);
        self.move_picked(view, slot, target, window, cx);
    }

    /// Move `view` out of `from` to `target`. Into a slot: shown there,
    /// trading places with the view shown there ([`Self::move_into`], as
    /// the Up and Down chords do). A new slot: on that side of `from`
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
        self.place(view, target.placement(from), window, cx);
    }

    /// The labels the open move chooser offers, in its order.
    #[cfg(test)]
    pub(crate) fn move_chooser_rows(&self, cx: &gpui_kit::App) -> Option<Vec<String>> {
        let chooser = self.open_move_chooser_list()?;
        Some(chooser.read(cx).labels().map(str::to_owned).collect())
    }

    /// The chords the open move chooser's rows name, in its order.
    #[cfg(test)]
    pub(crate) fn move_chooser_hints(&self, cx: &gpui_kit::App) -> Option<Vec<Option<KeyInput>>> {
        let chooser = self.open_move_chooser_list()?;
        Some(chooser.read(cx).hints().collect())
    }
}
