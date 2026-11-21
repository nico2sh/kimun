//! The **Facet view**'s bar rows and their lists ([`super`]).
//!
//! **The chips.** Row 1 holds one menu chip per kind with something left to
//! pick: `Tags N`, `Links N` and `Sections N`, where N counts the unpicked
//! facets of that kind that give results in the current view ([`menus`]).
//! A kind with nothing left has no chip, unless its list is open: the list
//! hangs from it. A chip whose N is 0 is drawn dimmed, with a fainter
//! border, and still opens its list. A spacer follows, then the search chip
//! (a search icon and `Find`, its key `/` in its tooltip and name), drawn while any chip
//! is. Row 1 never wraps. The chip labels are in the text colour; a dimmed
//! chip's is muted.
//!
//! **The picks.** Row 2 holds the picked pills in pick order ([`picked`]),
//! each with its remove mark. It wraps onto as many lines as it needs. The
//! bar scrolls inside its height cap, and a stop of the bar that takes the
//! keyboard scrolls into view. Row 2 is drawn only while there are picks.
//!
//! **The lists.** A kind chip opens the list of that kind's facets
//! ([`kind_list`]). The search chip and `/` open the list of every facet
//! under a small label per kind ([`all_grouped`]). The chip of the open list
//! is drawn selected and reports itself expanded. Each list starts with its
//! filter field, then one row per facet: a tick when picked, its text, and
//! at the right end the pieces it gives in the current view. Picked rows
//! come first. In the tag and link lists the others follow by that count,
//! so rows with no result come last. Sections stay in note order and the
//! dimmed ones stay in place. A row that gives nothing is dimmed but can
//! still be picked. The order is set when the list opens or its filter
//! changes, and stays while you pick: a press on a row toggles it in its
//! place, the list stays open, and only the counts change. A list draws at most [`LIST_CAP`] rows, and a muted last
//! line says how many more the filter can reveal. A filter that matches
//! nothing leaves one muted line that says so. A list opens only while its
//! scope has an unpicked facet. Once open, it stays open while its scope
//! holds any facet, so the last pick shows ticked. Only new text that
//! leaves the scope empty closes it. The keyboard highlight runs over the
//! drawn rows in order, picked rows too, and the list scrolls the lit row
//! into view. Going up onto the first row of a group shows its label too.
//! A new query scrolls the list back to the top.
//!
//! [`menus`]: kimun_notes::logic::facet_bar::FacetBar::menus
//! [`picked`]: kimun_notes::logic::facet_bar::FacetBar::picked
//! [`kind_list`]: kimun_notes::logic::facet_bar::FacetBar::kind_list
//! [`all_grouped`]: kimun_notes::logic::facet_bar::FacetBar::all_grouped

use std::collections::{HashMap, HashSet};

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, ClickEvent, Context, Div, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div,
};
use kimun_core::note::{Facet, FacetKind};
use kimun_notes::logic::facet_bar::{KindCounts, Pill};
use kimun_notes::settings::themes::Theme;

use super::keys::Stop;
use super::{FacetView, LINE_ID, ListKind, list_group_id, menu_id, more_item_id, pill_id};
use crate::theme;
use crate::workspace::facet_list::{ListGroup, OpenList, list_popover};
use crate::workspace::facet_pill::{pill, row};
use crate::workspace::facet_ui::{CARET_ICON, SEARCH_ICON, chip, icon_label_chip};

/// The most matches a list draws.
pub(crate) const LIST_CAP: usize = 200;

/// The search chip's text, after its icon ([`SEARCH_ICON`]).
const SEARCH_TEXT: &str = "Find";
/// The search chip's name and tooltip: its text and its key.
const SEARCH_LABEL: &str = "Find (/)";

/// A list's groups: each with its kind label (`None` for a kind list) and
/// the pills of its rows.
pub(super) type Groups = Vec<(Option<FacetKind>, Vec<Pill>)>;

/// A list's row order: each group with its kind label and its facets.
pub(super) type Order = Vec<(Option<FacetKind>, Vec<Facet>)>;

/// The row order of `groups`.
pub(super) fn order_of(groups: &Groups) -> Order {
    groups
        .iter()
        .map(|(kind, pills)| (*kind, pills.iter().map(|p| p.facet.clone()).collect()))
        .collect()
}

impl FacetView {
    /// Row 1: the kind chips and the search chip (see the module docs).
    /// A kind's chip shows while the kind has an unpicked facet, or while
    /// its list is open. A chip with no result is drawn dimmed. `None` when
    /// no kind has anything left to pick and no list is open.
    pub(super) fn render_chips(
        &self,
        counts: KindCounts,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let kinds = ListKind::chip_order().map(|kind| {
            let (n, any) = list_counts(counts, kind);
            (kind, kind.name(), n, any)
        });
        #[cfg(test)]
        {
            self.open_chip.set(None);
            self.dimmed_chips.borrow_mut().clear();
        }
        // An open list keeps its chip: it hangs from it.
        if kinds.iter().all(|(_, _, _, any)| !any) && self.more.is_none() {
            return None;
        }
        let chips: Vec<AnyElement> = kinds
            .into_iter()
            .filter(|(kind, _, _, any)| *any || self.list_open(*kind))
            .map(|(kind, name, n, _)| {
                let dimmed = n == 0;
                #[cfg(test)]
                if dimmed {
                    self.dimmed_chips.borrow_mut().push(kind);
                }
                let el = chip(menu_id(kind), format!("{name} {n}"), dimmed, t);
                let el = self
                    .open_chip(kind, el, t)
                    .gap_0p5()
                    .child(
                        div()
                            .flex()
                            .text_color(theme::muted(t))
                            .child(Icon::new(CARET_ICON).size_3()),
                    )
                    .test_support();
                let el = self.track(Stop::Menu(kind), el, t, cx);
                self.render_list(kind, Anchor::TopLeft, el.into_any_element(), t, cx)
            })
            .collect();
        let search = self
            .open_chip(
                ListKind::All,
                icon_label_chip(menu_id(ListKind::All), SEARCH_ICON, SEARCH_TEXT, t),
                t,
            )
            .aria_label(SEARCH_LABEL)
            .tooltip(|window, cx| Tooltip::new(SEARCH_LABEL).build(window, cx))
            .test_support();
        let search = self.track(Stop::Menu(ListKind::All), search, t, cx);
        // The search chip ends the row: its list opens leftward from it.
        let search = self.render_list(
            ListKind::All,
            Anchor::TopRight,
            search.into_any_element(),
            t,
            cx,
        );
        Some(
            div()
                .id(LINE_ID)
                .flex()
                .flex_none()
                .w_full()
                .items_center()
                .gap_1()
                .children(chips)
                .child(div().flex_1().min_w_0())
                .child(search)
                .test_support(),
        )
    }

    /// `chip`, the chip of `kind`'s list, drawn selected while that list is
    /// open. It reports its open state as expanded.
    fn open_chip(&self, kind: ListKind, chip: Stateful<Div>, t: &Theme) -> Stateful<Div> {
        let open = self.list_open(kind);
        #[cfg(test)]
        if open {
            self.open_chip.set(Some(kind));
        }
        chip.aria_expanded(open)
            .when(open, |chip| chip.bg(theme::selected_background(t)))
    }

    /// Row 2: the picked pills, each a button that unpicks it, with the
    /// count its list row shows. They are
    /// children of the bar itself, so the bar can scroll each one into view
    /// ([`Self::reveal_arrived`]). Empty without picks.
    pub(super) fn render_picks(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.bar
            .picked(&self.members.merged)
            .into_iter()
            .map(|mut p| {
                // The count its list row shows: the pieces it gives in view.
                p.in_view = self.members.in_view_of(&p.facet);
                let el = pill(pill_id(&p.facet), &p, t);
                self.button(Stop::Pill(p.facet), el, t, cx)
                    .into_any_element()
            })
            .collect()
    }

    /// When a stop of the bar took the keyboard since the last frame (by
    /// Tab, or after an unpick moved it on), scroll the bar so that stop
    /// shows. The bar's children are row 1 (`chips`), the picks (`picks`
    /// of them), then the summary line. The kit scrolls in this frame's
    /// layout, by the least amount that shows the child.
    pub(super) fn reveal_arrived(&self, chips: bool, picks: usize) {
        let first_pick = usize::from(chips);
        let ix = match self.stops.borrow().arrived() {
            Some(Stop::Menu(_)) if chips => 0,
            Some(Stop::Pill(facet)) => {
                let picked = self.bar.picked(&self.members.merged);
                match picked.iter().position(|p| p.facet == *facet) {
                    Some(at) => first_pick + at,
                    None => return,
                }
            }
            Some(Stop::Clear) if picks > 0 => first_pick + picks,
            // The summary line after a clear: no picks before it.
            Some(Stop::Undo) => first_pick + picks,
            _ => return,
        };
        self.bar_scroll.scroll_to_item(ix);
    }

    /// `trigger` with `kind`'s list ([`list_popover`]) hung under it, open
    /// while the view holds it open: the field, then the groups of
    /// [`Self::list_groups`].
    fn render_list(
        &self,
        kind: ListKind,
        anchor: Anchor,
        trigger: AnyElement,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let list = self
            .more
            .as_ref()
            .filter(|more| more.kind == kind)
            .map(|more| {
                let query = more.input.read(cx).value().to_string();
                let (groups, rest) = cap(self.ordered(kind, &query, &more.order));
                let len = groups.iter().map(|(_, pills)| pills.len()).sum::<usize>();
                let lit = more.highlight.min(len.saturating_sub(1));
                let empty = (groups.is_empty() && !query.is_empty())
                    .then(|| SharedString::from(empty_text(kind, &query)));
                let mut at = 0;
                // The scroll container's child of the lit row: the rows and
                // the group labels before it.
                let mut lit_child = 0;
                // The lit row is the first under a group label.
                let mut lit_first = false;
                let mut labels = 0;
                let groups = groups
                    .into_iter()
                    .map(|(group, pills)| {
                        labels += usize::from(group.is_some());
                        let first = at;
                        let rows = pills
                            .into_iter()
                            .map(|p| {
                                let lit = at == lit;
                                if lit {
                                    lit_child = at + labels;
                                    lit_first = group.is_some() && at == first;
                                }
                                at += 1;
                                let facet = p.facet.clone();
                                row(more_item_id(&p.facet), &p, lit, t)
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, window, cx| {
                                            this.toggle_row(facet.clone(), window, cx)
                                        },
                                    ))
                                    .test_support()
                                    .into_any_element()
                            })
                            .collect();
                        ListGroup {
                            label: group.map(|kind| (list_group_id(kind), group_label(kind))),
                            rows,
                        }
                    })
                    .collect::<Vec<_>>();
                if more.reveal.take() {
                    // Up onto a group's first row: its label shows too.
                    let label = more.up && lit_first;
                    more.scroll.scroll_to_item(lit_child - usize::from(label));
                }
                let rest = (rest > 0).then(|| SharedString::from(rest_text(rest)));
                OpenList {
                    input: more.input.clone(),
                    name: kind.name(),
                    scroll: more.scroll.clone(),
                    groups,
                    empty,
                    rest,
                }
            });
        let changed = cx.listener(move |this, open: &bool, window, cx| {
            this.list_changed(kind, *open, window, cx)
        });
        list_popover(kind, anchor, trigger, list, changed, t, cx)
    }

    /// The groups of `kind`'s list, filtered by `query`: one group with no
    /// label for a kind, the kinds under their labels for
    /// [`ListKind::All`]. Picked and unpicked facets, picked first. Every
    /// match, not capped. Empty groups are left out.
    pub(super) fn scope(&self, kind: ListKind, query: &str) -> Groups {
        let merged = &self.members.merged;
        let in_view = |f: &Facet| self.members.in_view_of(f);
        let groups: Groups = match kind {
            ListKind::All => self
                .bar
                .all_grouped(merged, query, &in_view)
                .into_iter()
                .map(|(kind, pills)| (Some(kind), pills))
                .collect(),
            // A kind list: its one kind, with no group label.
            kind => kind
                .facet_kinds()
                .iter()
                .map(|&k| (None, self.bar.kind_list(merged, k, query, &in_view)))
                .collect(),
        };
        groups
            .into_iter()
            .filter(|(_, pills)| !pills.is_empty())
            .collect()
    }

    /// [`Self::scope`] with its rows in `order`, each row with its tick and
    /// count as they are now. A row whose facet the scope no longer holds
    /// is left out. A facet `order` does not know (new text) sorts the
    /// whole list again, as [`Self::scope`] does.
    fn ordered(&self, kind: ListKind, query: &str, order: &Order) -> Groups {
        let fresh = self.scope(kind, query);
        let known: HashSet<&Facet> = order.iter().flat_map(|(_, facets)| facets).collect();
        if fresh
            .iter()
            .flat_map(|(_, pills)| pills)
            .any(|p| !known.contains(&p.facet))
        {
            return fresh;
        }
        let mut pills: HashMap<Facet, Pill> = fresh
            .into_iter()
            .flat_map(|(_, pills)| pills)
            .map(|p| (p.facet.clone(), p))
            .collect();
        order
            .iter()
            .map(|(kind, facets)| {
                (
                    *kind,
                    facets.iter().filter_map(|f| pills.remove(f)).collect(),
                )
            })
            .filter(|(_, pills): &(_, Vec<Pill>)| !pills.is_empty())
            .collect()
    }

    /// The open list's groups, filtered by its query, in its row order and
    /// cut to [`LIST_CAP`] rows, and how many matches the cut left out.
    fn list_groups(&self, cx: &App) -> (Groups, usize) {
        let Some(more) = &self.more else {
            return (Vec::new(), 0);
        };
        let query = more.input.read(cx).value().to_string();
        cap(self.ordered(more.kind, &query, &more.order))
    }

    /// The pills of the open list's drawn rows, in order: what the
    /// highlight runs over.
    pub(super) fn list_rows(&self, cx: &App) -> Vec<Pill> {
        self.list_groups(cx)
            .0
            .into_iter()
            .flat_map(|(_, pills)| pills)
            .collect()
    }
}

/// `groups` cut to [`LIST_CAP`] rows in order (a group left empty is
/// dropped), and how many rows the cut left out.
fn cap(groups: Groups) -> (Groups, usize) {
    let mut room = LIST_CAP;
    let mut rest = 0;
    let mut kept = Vec::new();
    for (kind, mut pills) in groups {
        if pills.len() > room {
            rest += pills.len() - room;
            pills.truncate(room);
        }
        room -= pills.len();
        if !pills.is_empty() {
            kept.push((kind, pills));
        }
    }
    (kept, rest)
}

/// The muted last line of a list that left `rest` matches out.
fn rest_text(rest: usize) -> String {
    format!("{rest} more. Type to filter.")
}

/// The muted line of `kind`'s list when `query` matches nothing.
fn empty_text(kind: ListKind, query: &str) -> String {
    match kind {
        ListKind::All => format!("No matches for \"{query}\""),
        kind => format!("No {} match \"{query}\"", kind.name().to_lowercase()),
    }
}

/// The label of `kind`'s group in the all-kinds list: the name of the kind
/// list that holds it.
fn group_label(kind: FacetKind) -> &'static str {
    ListKind::of(kind).name()
}

/// What the chip of `list` counts: the unpicked facets of its kinds that
/// give results, and whether it has any unpicked facet at all.
pub(super) fn list_counts(counts: KindCounts, list: ListKind) -> (usize, bool) {
    list.facet_kinds()
        .iter()
        .map(|&kind| match kind {
            FacetKind::Tag => (counts.tags, counts.any.tags),
            FacetKind::NoteLink => (counts.links, counts.any.links),
            FacetKind::Heading => (counts.sections, counts.any.sections),
        })
        .fold((0, false), |(n, any), (m, more)| (n + m, any || more))
}
