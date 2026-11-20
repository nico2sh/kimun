//! The **Facet view**'s filter line and its lists ([`super`]).
//!
//! **The line.** One row that never wraps: the pills of
//! [`FacetBar::line`](kimun_notes::logic::facet_bar::FacetBar::line)
//! that fit (picked first, each with its remove mark, then tags and links
//! by count), the `+N` chip while some are left out, and the `Sections N`
//! chip while the notes have headings. How many fit is
//! [`fit_count`] over measured widths: each pill's width comes from the
//! window's text system in the pill's own font and size, plus its padding,
//! gap, icons and border ([`pill_width`]), cached per pill until the font
//! or the rem size changes ([`PillWidths`]). The room is the line's width
//! less the `Sections N` chip, as laid out in the last frame (a probe
//! records it while the frame prepaints); a frame that finds it changed asks
//! for one more (see `resize_check`). Before the first layout the window's
//! width stands in. Picked pills are always drawn: when they alone overflow
//! the room, the pills clip at the line's end and `+N` stays. The fit keeps
//! a pixel of slack, and the gap before `+N` belongs to the chip, so the
//! last pill is never cut when every pill fits.
//!
//! **The lists.** `+N` and `/` open the list of every unpicked tag and
//! link ([`grouped`]), under a small label per kind (Tags, then Links), its
//! filter field first; without a `+N` chip it hangs from the end of the
//! line. `Sections N` opens the headings ([`more_list`]), and so does `/`
//! when no tag or link is left unpicked. A list of tags and links left with
//! nothing to hold (its last one picked) closes.
//!
//! [`grouped`]: kimun_notes::logic::facet_bar::FacetBar::grouped
//! [`more_list`]: kimun_notes::logic::facet_bar::FacetBar::more_list
//! The keyboard highlight runs over the list's pills in order.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, ClickEvent, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _,
    TextRun, Window, canvas, div, font,
};
use kimun_core::note::FacetKind;
use kimun_notes::logic::facet_bar::{BarLine, Pill, fit_count};
use kimun_notes::settings::themes::Theme;

use super::keys::Stop;
use super::{
    FacetView, LINE_ID, LINE_PILLS_ID, ListKind, MORE_ID, SECTIONS_ID, list_group_id, more_item_id,
    pill_id,
};
use crate::theme;
use crate::workspace::facet_list::{ListGroup, OpenList, list_popover};
use crate::workspace::facet_ui::{chip, chip_width, pill, pill_width};

/// The `+N` chip's tooltip (its name adds the count).
const MORE_LABEL: &str = "More facets (/)";

/// The size of pill text (`text_xs`), in rems.
const TEXT_REMS: f32 = 0.75;

/// What the line keeps free of the room it measured, so a rounding of the
/// layout never cuts a pixel off the last pill.
const FIT_SLACK_PX: f32 = 1.0;

/// The most pill widths kept before the cache starts over.
const MAX_CACHED: usize = 4096;

/// The measured width of each pill drawn so far, for one font and rem size.
#[derive(Default)]
pub(super) struct PillWidths {
    /// The font family and rem size the widths were measured with.
    font: Option<(SharedString, u32)>,
    by_pill: HashMap<(FacetKind, String, usize, bool), f32>,
}

impl PillWidths {
    /// The cached width of `p`, if it was measured with the current font.
    #[cfg(test)]
    pub(super) fn get(&self, p: &Pill) -> Option<f32> {
        self.by_pill.get(&key(p)).copied()
    }
}

fn key(p: &Pill) -> (FacetKind, String, usize, bool) {
    (p.facet.kind, p.label.clone(), p.count, p.picked)
}

/// The width of a text in the pills' font and size, from the window's text
/// system.
struct TextWidth {
    system: Rc<gpui_kit::WindowTextSystem>,
    font: gpui_kit::Font,
    size: Pixels,
}

impl TextWidth {
    fn new(family: SharedString, rem: f32, window: &Window) -> Self {
        Self {
            system: window.text_system().clone(),
            font: font(family),
            size: gpui_kit::px(TEXT_REMS * rem),
        }
    }

    fn of(&self, text: &str) -> f32 {
        let run = TextRun {
            len: text.len(),
            font: self.font.clone(),
            color: gpui_kit::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = self.system.shape_line(
            SharedString::from(text.to_string()),
            self.size,
            &[run],
            None,
        );
        // A text element lays out at its width rounded up.
        f32::from(line.width).ceil()
    }
}

/// An empty element filling its parent that records the parent's width in
/// `room` each time the frame prepaints.
fn room_probe(room: Rc<Cell<Option<f32>>>) -> impl IntoElement {
    canvas(
        move |bounds, _, _| room.set(Some(f32::from(bounds.size.width))),
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl FacetView {
    /// The filter line (see the module docs).
    pub(super) fn render_line(
        &self,
        line: BarLine,
        window: &Window,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let rem = f32::from(window.rem_size());
        let gap = 0.25 * rem;
        let text = TextWidth::new(cx.theme().font_family.clone(), rem, window);
        let widths = self.pill_widths(&line.pills, rem, &text);
        let total = line.pills.len();
        let more_width = chip_width(&format!("+{total}"), rem, |s| text.of(s));
        let room = self
            .line_room
            .get()
            .unwrap_or_else(|| f32::from(window.viewport_size().width));
        let keep = line.pills.iter().filter(|p| p.picked).count();
        let shown = fit_count(&widths, gap, room - FIT_SLACK_PX, more_width, keep);
        let hidden = total - shown;
        self.more_count.set(hidden);

        let pills: Vec<_> = line
            .pills
            .into_iter()
            .take(shown)
            .map(|p| {
                let el = pill(pill_id(&p.facet), &p, t);
                self.button(Stop::Pill(p.facet), el, t, cx)
            })
            .collect();
        let (trigger, anchor) = if hidden > 0 {
            let el = chip(MORE_ID.into(), format!("+{hidden}"), t)
                .aria_label(SharedString::from(format!("{hidden} more facets (/)")))
                .tooltip(|window, cx| Tooltip::new(MORE_LABEL).build(window, cx))
                .test_support();
            let el = self.track(Stop::More, el, t, cx);
            // The chip ends the room: the list opens leftward from it.
            (el.into_any_element(), Anchor::TopRight)
        } else {
            // Nothing left out: `/` hangs the list from the line's end.
            let end = div().id("facet-line-end").flex_none().w_0().h_6();
            (end.into_any_element(), Anchor::TopLeft)
        };
        let more = self.render_list(ListKind::Facets, anchor, trigger, t, cx);
        // The gap before `+N` is the one `fit_count` counts; the bare anchor
        // takes none, so the pills get the whole room when they all fit.
        let more = div()
            .flex_none()
            .when(hidden > 0, |el| el.pl_1())
            .child(more);
        let sections = (line.sections > 0).then(|| {
            let label = format!("Sections {}", line.sections);
            let el = chip(SECTIONS_ID.into(), label, t).test_support();
            let el = self.track(Stop::Sections, el, t, cx);
            self.render_list(
                ListKind::Sections,
                Anchor::TopLeft,
                el.into_any_element(),
                t,
                cx,
            )
        });
        div()
            .id(LINE_ID)
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .child(
                        div()
                            .id(LINE_PILLS_ID)
                            .flex()
                            .min_w_0()
                            .overflow_hidden()
                            .items_center()
                            .gap_1()
                            .children(pills)
                            .test_support(),
                    )
                    .child(more)
                    .child(room_probe(self.line_room.clone())),
            )
            .children(sections)
            .test_support()
    }

    /// The widths of `pills` ([`pill_width`]), measured once per font and
    /// rem size and then cached.
    fn pill_widths(&self, pills: &[Pill], rem: f32, text: &TextWidth) -> Vec<f32> {
        let mut cache = self.widths.borrow_mut();
        let font = Some((text.font.family.clone(), rem.to_bits()));
        // A new font or rem size, or a cache grown past any one note's
        // facets (counts change as notes are edited): measure again.
        if cache.font != font || cache.by_pill.len() > MAX_CACHED {
            cache.font = font;
            cache.by_pill.clear();
        }
        pills
            .iter()
            .map(|p| {
                *cache
                    .by_pill
                    .entry(key(p))
                    .or_insert_with(|| pill_width(p, rem, |s| text.of(s)).ceil())
            })
            .collect()
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
                let groups = self.list_groups(cx);
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
                OpenList {
                    input: more.input.clone(),
                    groups,
                }
            });
        let changed = cx.listener(move |this, open: &bool, window, cx| {
            this.list_changed(kind, *open, window, cx)
        });
        list_popover(kind, anchor, trigger, list, changed, t, cx)
    }

    /// The open list's groups, filtered by its query: the unpicked tags and
    /// links under their kind, or the headings in one group with no label
    /// (see the module docs).
    fn list_groups(&self, cx: &App) -> Vec<(Option<FacetKind>, Vec<Pill>)> {
        let Some(more) = &self.more else {
            return Vec::new();
        };
        let query = more.input.read(cx).value().to_string();
        let merged = &self.members.merged;
        match more.kind {
            ListKind::Facets => self
                .bar
                .grouped(merged, &query)
                .into_iter()
                .map(|(kind, pills)| (Some(kind), pills))
                .collect(),
            ListKind::Sections => {
                vec![(None, self.bar.more_list(merged, FacetKind::Heading, &query))]
            }
        }
    }

    /// The open list's pills, in order: what the highlight runs over.
    pub(super) fn list_pills(&self, cx: &App) -> Vec<Pill> {
        self.list_groups(cx)
            .into_iter()
            .flat_map(|(_, pills)| pills)
            .collect()
    }
}

fn group_label(kind: FacetKind) -> &'static str {
    match kind {
        FacetKind::Tag => "Tags",
        FacetKind::NoteLink => "Links",
        FacetKind::Heading => "Sections",
    }
}
