//! The **Facet view**'s bar rows and their lists ([`super`]).
//!
//! **The chips.** Row 1 holds one menu chip per kind with something left to
//! pick: `Tags N`, `Links N` and `Sections N`, where N counts the unpicked
//! facets of that kind ([`menus`]). A kind with nothing left has no chip. A
//! spacer follows, then the search chip (`⌕ /`), drawn while any chip is.
//! Row 1 never wraps.
//!
//! **The picks.** Row 2 holds the picked pills in pick order ([`picked`]),
//! each with its remove mark. It wraps onto as many lines as it needs. The
//! bar scrolls inside its height cap, and a stop of the bar that takes the
//! keyboard scrolls into view. Row 2 is drawn only while there are picks.
//!
//! **The lists.** A kind chip opens the list of that kind's unpicked facets
//! ([`kind_list`]). The search chip and `/` open the list of every unpicked
//! facet under a small label per kind ([`all_grouped`]). Each list starts
//! with its filter field. It draws at most [`LIST_CAP`] matches, and a muted
//! last line says how many more the filter can reveal. A list closes when a
//! pick or new text leaves nothing in its scope. The keyboard highlight runs
//! over the drawn pills in order.
//!
//! [`menus`]: kimun_notes::logic::facet_bar::FacetBar::menus
//! [`picked`]: kimun_notes::logic::facet_bar::FacetBar::picked
//! [`kind_list`]: kimun_notes::logic::facet_bar::FacetBar::kind_list
//! [`all_grouped`]: kimun_notes::logic::facet_bar::FacetBar::all_grouped

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, ClickEvent, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div,
};
use kimun_core::note::FacetKind;
use kimun_notes::logic::facet_bar::{KindCounts, Pill};
use kimun_notes::settings::themes::Theme;

use super::keys::Stop;
use super::{FacetView, LINE_ID, ListKind, list_group_id, menu_id, more_item_id, pill_id};
use crate::theme;
use crate::workspace::facet_list::{ListGroup, OpenList, list_popover};
use crate::workspace::facet_ui::{chip, pill};

/// The most matches a list draws.
pub(crate) const LIST_CAP: usize = 200;

/// The search chip's name and tooltip.
const SEARCH_LABEL: &str = "Search facets (/)";

/// The caret after a menu chip's label.
const CARET: &str = "\u{25BE}";

/// A list's groups: each with its kind label (`None` for a kind list) and
/// its pills.
pub(super) type Groups = Vec<(Option<FacetKind>, Vec<Pill>)>;

impl FacetView {
    /// Row 1: the kind chips and the search chip (see the module docs).
    /// `None` when no kind has anything left to pick.
    pub(super) fn render_chips(
        &self,
        counts: KindCounts,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let kinds = [
            (ListKind::Tags, "Tags", counts.tags),
            (ListKind::Links, "Links", counts.links),
            (ListKind::Sections, "Sections", counts.sections),
        ];
        if kinds.iter().all(|(_, _, n)| *n == 0) {
            return None;
        }
        let chips: Vec<AnyElement> = kinds
            .into_iter()
            .filter(|(_, _, n)| *n > 0)
            .map(|(kind, name, n)| {
                let el = chip(menu_id(kind), format!("{name} {n}"), t)
                    .gap_0p5()
                    .child(div().text_color(theme::muted(t)).child(CARET))
                    .test_support();
                let el = self.track(Stop::Menu(kind), el, t, cx);
                self.render_list(kind, Anchor::TopLeft, el.into_any_element(), t, cx)
            })
            .collect();
        let search = chip(menu_id(ListKind::All), "\u{2315} /", t)
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

    /// Row 2: the picked pills, each a button that unpicks it. They are
    /// children of the bar itself, so the bar can scroll each one into view
    /// ([`Self::reveal_arrived`]). Empty without picks.
    pub(super) fn render_picks(&self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.bar
            .picked(&self.members.merged)
            .into_iter()
            .map(|p| {
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
                let (groups, rest) = self.list_groups(cx);
                let len = groups.iter().map(|(_, pills)| pills.len()).sum::<usize>();
                let lit = more.highlight.min(len.saturating_sub(1));
                let mut at = 0;
                let groups = groups
                    .into_iter()
                    .map(|(group, pills)| {
                        let pills = pills
                            .into_iter()
                            .map(|p| {
                                let lit = at == lit;
                                at += 1;
                                let facet = p.facet.clone();
                                pill(more_item_id(&p.facet), &p, t)
                                    .aria_selected(lit)
                                    .when(lit, |p| p.border_color(theme::focus_border(t)))
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, window, cx| {
                                            this.pick_more(facet.clone(), window, cx)
                                        },
                                    ))
                                    .test_support()
                                    .into_any_element()
                            })
                            .collect();
                        ListGroup {
                            label: group.map(|kind| (list_group_id(kind), group_label(kind))),
                            pills,
                        }
                    })
                    .collect();
                let rest = (rest > 0).then(|| SharedString::from(rest_text(rest)));
                OpenList {
                    input: more.input.clone(),
                    groups,
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
    /// [`ListKind::All`]. Every match, not capped. Empty groups are left
    /// out.
    pub(super) fn scope(&self, kind: ListKind, query: &str) -> Groups {
        let merged = &self.members.merged;
        let one = |kind| vec![(None, self.bar.kind_list(merged, kind, query))];
        let groups = match kind {
            ListKind::Tags => one(FacetKind::Tag),
            ListKind::Links => one(FacetKind::NoteLink),
            ListKind::Sections => one(FacetKind::Heading),
            ListKind::All => self
                .bar
                .all_grouped(merged, query)
                .into_iter()
                .map(|(kind, pills)| (Some(kind), pills))
                .collect(),
        };
        groups
            .into_iter()
            .filter(|(_, pills)| !pills.is_empty())
            .collect()
    }

    /// The open list's groups, filtered by its query and cut to
    /// [`LIST_CAP`] pills in group order, and how many matches the cut left
    /// out.
    fn list_groups(&self, cx: &App) -> (Groups, usize) {
        let Some(more) = &self.more else {
            return (Vec::new(), 0);
        };
        let query = more.input.read(cx).value().to_string();
        cap(self.scope(more.kind, &query))
    }

    /// The open list's drawn pills, in order: what the highlight runs over.
    pub(super) fn list_pills(&self, cx: &App) -> Vec<Pill> {
        self.list_groups(cx)
            .0
            .into_iter()
            .flat_map(|(_, pills)| pills)
            .collect()
    }
}

/// `groups` cut to [`LIST_CAP`] pills in order (a group left empty is
/// dropped), and how many pills the cut left out.
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

fn group_label(kind: FacetKind) -> &'static str {
    match kind {
        FacetKind::Tag => "Tags",
        FacetKind::NoteLink => "Links",
        FacetKind::Heading => "Sections",
    }
}
