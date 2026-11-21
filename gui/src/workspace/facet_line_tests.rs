//! UI tests for the **Facet view**'s bar (Option C of the design review):
//! a row of kind menu chips (`Tags N`, `Links N`, `Sections N`) and a search
//! chip, a wrapping row of picked pills under it, and the lists the chips
//! open. Each test draws a `FacetView` alone in a headless window and reads
//! the bounds it painted (see `facet_tests` for the helpers).
#![cfg(test)]

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{SharedString, TestAppContext, VisualTestContext, px};
use kimun_core::note::{Facet, FacetKind};

use super::facet::{
    CLEAR_ID, COLUMN_ID, EMPTY_ID, FACET_BAR_ID, FACET_VIEW_ID, LINE_ID, LIST_EMPTY_ID,
    LIST_REST_ID, ListKind, MORE_LIST_ID, MORE_QUERY_ID, SUMMARY_ID, chunk_id, list_group_id,
    menu_id, more_item_id, outline_mark_id, pill_count_id, pill_id, remove_mark_id, row_count_id,
    row_tick_id,
};
use super::facet_size_tests::assert_no_correction;
use super::facet_tests::{
    bounds, click, field_focused, focus_view, has_focus, heading, key, label, list_open,
    next_frame, open, picks, picks_changed, query, settle, shows, tab_to, tag, view_focused,
};

fn link(key: &str) -> Facet {
    Facet::new(FacetKind::NoteLink, key)
}

/// The 60 tags of [`crowded`], in list order (every count is one, so by
/// label).
fn crowded_tags() -> Vec<Facet> {
    (0..60).map(|i| tag(&format!("tag{i:02}"))).collect()
}

/// The 20 links of [`crowded`], in list order.
fn crowded_links() -> Vec<Facet> {
    (0..20).map(|i| link(&format!("link{i:02}"))).collect()
}

/// A note with a heading, then 60 tags and 20 links in one paragraph.
fn crowded() -> String {
    let tags: String = (0..60).map(|i| format!("#tag{i:02} ")).collect();
    let links: String = (0..20).map(|i| format!("[[link{i:02}]] ")).collect();
    format!("# Head\n\nintro\n\n{tags}{links}\n")
}

/// Whether the element `id` was drawn in the last frame, seen or not (a
/// list's pills scroll inside it).
fn exists(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> bool {
    let id: SharedString = id.into();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
}

fn results(
    view: &gpui_kit::Entity<super::facet::FacetView>,
    cx: &mut VisualTestContext,
) -> Vec<Facet> {
    view.read_with(cx, |view, cx| view.list_results(cx))
}

/// The box around the picked pills of `picks`: row 2.
fn picks_row(cx: &mut VisualTestContext, picks: &[Facet]) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    let all: Vec<_> = picks.iter().map(|f| bounds(cx, pill_id(f))).collect();
    all.iter().skip(1).fold(all[0], |row, b| row.union(b))
}

fn drawn_stops(
    view: &gpui_kit::Entity<super::facet::FacetView>,
    cx: &mut VisualTestContext,
) -> Vec<SharedString> {
    cx.update(|window, cx| window.render_frame(cx));
    view.read_with(cx, |view, _| view.drawn_stop_ids())
}

const MENUS_NOTE: &str = "# Head\n\n## Sub\n\n#a #b #c [[x]] [[y]]\n";

#[gpui_kit::test]
fn row_one_holds_a_chip_per_kind_with_unpicked_counts(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, MENUS_NOTE, vec![tag("a"), heading("sub")]);
    settle(cx);
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 2")
    );
    assert_eq!(
        label(cx, menu_id(ListKind::Links)).as_deref(),
        Some("Links 2")
    );
    assert_eq!(
        label(cx, menu_id(ListKind::Sections)).as_deref(),
        Some("Sections 1")
    );
    assert_eq!(
        label(cx, menu_id(ListKind::All)).as_deref(),
        Some("Find (/)")
    );
    let line = bounds(cx, LINE_ID);
    assert!(f32::from(line.size.height) <= 26.0, "one row: {line:?}");
    let chips: Vec<_> = [
        ListKind::Tags,
        ListKind::Links,
        ListKind::Sections,
        ListKind::All,
    ]
    .into_iter()
    .map(|kind| bounds(cx, menu_id(kind)))
    .collect();
    for pair in chips.windows(2) {
        assert!(pair[0].right() <= pair[1].left(), "in order: {pair:?}");
    }
    for chip in &chips {
        assert_eq!(chip.top(), chips[0].top(), "on the one row");
        assert!(chip.top() >= line.top() && chip.bottom() <= line.bottom());
        assert!(f32::from(chip.size.height) >= 24.0, "a 24 px target");
    }
    // A spacer pushes the search chip to the row's right end.
    assert!(chips[3].left() - chips[2].right() > px(100.));
    assert!((f32::from(line.right() - chips[3].right())).abs() < 1.0);
    // No unpicked facet is drawn in the bar.
    assert!(!shows(cx, pill_id(&tag("b"))));
    assert!(!shows(cx, pill_id(&link("x"))));
    assert!(!shows(cx, pill_id(&heading("head"))));
}

#[gpui_kit::test]
fn a_kind_with_nothing_unpicked_has_no_chip(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "#a [[x]]\n", vec![tag("a")]);
    assert!(!shows(cx, menu_id(ListKind::Tags)), "every tag picked");
    assert_eq!(
        label(cx, menu_id(ListKind::Links)).as_deref(),
        Some("Links 1")
    );
    assert!(!shows(cx, menu_id(ListKind::Sections)), "no heading");
    assert!(shows(cx, menu_id(ListKind::All)));
    // Every facet picked: no chip, no search chip, no row 1.
    view.update(cx, |view, cx| view.toggle(link("x"), cx));
    for kind in [
        ListKind::Tags,
        ListKind::Links,
        ListKind::Sections,
        ListKind::All,
    ] {
        assert!(!shows(cx, menu_id(kind)), "{kind:?}");
    }
    assert!(!shows(cx, LINE_ID));
    assert!(shows(cx, pill_id(&tag("a"))), "the picks stay");
    assert!(shows(cx, CLEAR_ID));
}

#[gpui_kit::test]
fn the_bar_has_no_search_chip_without_facets(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "just text\n\nmore text\n", vec![]);
    assert!(!shows(cx, menu_id(ListKind::All)));
    assert!(!shows(cx, LINE_ID));
    assert!(!shows(cx, FACET_BAR_ID), "nothing to draw: no bar");
    view.update(cx, |view, cx| view.set_text("#a\n".into(), cx));
    assert!(
        shows(cx, menu_id(ListKind::All)),
        "a facet: the search chip"
    );
}

#[gpui_kit::test]
fn picks_draw_in_their_own_row_with_a_remove_mark(cx: &mut TestAppContext) {
    let picks_made = vec![tag("tag59"), link("link19"), heading("head")];
    let (view, cx, events, _sub) = open(cx, &crowded(), picks_made.clone());
    settle(cx);
    let line = bounds(cx, LINE_ID);
    let row = picks_row(cx, &picks_made);
    assert!(
        row.top() >= line.bottom(),
        "under the chips: {row:?} {line:?}"
    );
    let drawn: Vec<_> = picks_made.iter().map(|f| bounds(cx, pill_id(f))).collect();
    for pair in drawn.windows(2) {
        assert!(pair[0].right() <= pair[1].left(), "in pick order");
    }
    let summary_top = bounds(cx, SUMMARY_ID).top();
    for b in &drawn {
        assert!(
            b.top() >= line.bottom() && b.bottom() <= summary_top,
            "between the chips and the summary: {b:?}"
        );
    }
    assert_eq!(
        label(cx, pill_id(&tag("tag59"))).as_deref(),
        Some("Remove #tag59"),
        "a picked pill is named for what a press does"
    );
    for facet in &picks_made {
        assert!(
            shows(cx, remove_mark_id(facet)),
            "{facet:?} has a remove mark"
        );
    }
    // The summary line, with `Clear`, sits under the picks.
    assert!(bounds(cx, SUMMARY_ID).top() >= row.bottom());
    assert!(bounds(cx, CLEAR_ID).top() >= row.bottom());
    let summary = label(cx, SUMMARY_ID).expect("the summary line");
    assert!(
        summary.contains("#tag59") && summary.contains("link19"),
        "{summary}"
    );
    // A click on a remove mark unpicks its facet.
    click(cx, remove_mark_id(&link("link19")));
    assert_eq!(picks(&view, cx), [tag("tag59"), heading("head")]);
    assert_eq!(picks_changed(&events), 1);
    assert!(!shows(cx, pill_id(&link("link19"))));
}

#[gpui_kit::test]
fn picks_wrap_in_a_narrow_pane_and_every_one_can_be_removed(cx: &mut TestAppContext) {
    let long: Vec<Facet> = (0..10)
        .map(|i| tag(&format!("longtagname{i:02}")))
        .collect();
    let text: String = (0..10)
        .map(|i| format!("#longtagname{i:02} piece {i}\n\n"))
        .collect();
    let (view, cx, _events, _sub) = open(cx, &text, long.clone());
    cx.simulate_resize(gpui_kit::size(px(320.), px(600.)));
    settle(cx);
    let whole = bounds(cx, FACET_VIEW_ID);
    let row = picks_row(cx, &long);
    assert!(f32::from(row.size.height) > 30.0, "it wraps: {row:?}");
    for facet in &long {
        assert!(shows(cx, pill_id(facet)), "{facet:?} drawn");
        assert!(shows(cx, remove_mark_id(facet)), "{facet:?} keeps its mark");
        let b = bounds(cx, pill_id(facet));
        assert!(b.right() <= whole.right(), "{facet:?} whole: {b:?}");
        assert!(
            b.top() >= bounds(cx, FACET_BAR_ID).top() && b.bottom() <= bounds(cx, SUMMARY_ID).top(),
            "{facet:?} between the bar top and the summary: {b:?}"
        );
    }
    for (i, facet) in long.iter().enumerate() {
        click(cx, remove_mark_id(facet));
        assert_eq!(picks(&view, cx), long[i + 1..], "{facet:?} unpicked");
    }
    assert!(!shows(cx, CLEAR_ID), "no picks: no summary line either");
}

#[gpui_kit::test]
fn a_pick_longer_than_the_pane_keeps_its_remove_mark_in_view(cx: &mut TestAppContext) {
    let name = "Weekly planning and review of the garden project notes";
    let pick = heading(name);
    let (view, cx, _events, _sub) = open(cx, &format!("# {name}\n\ntext\n"), vec![pick.clone()]);
    cx.simulate_resize(gpui_kit::size(px(320.), px(600.)));
    settle(cx);
    let bar = bounds(cx, FACET_BAR_ID);
    let b = bounds(cx, pill_id(&pick));
    let slack = px(0.5);
    assert!(
        b.left() >= bar.left() - slack && b.right() <= bar.right() + slack,
        "the pill lies inside the bar: {b:?} {bar:?}"
    );
    assert!(shows(cx, remove_mark_id(&pick)), "the remove mark is shown");
    click(cx, remove_mark_id(&pick));
    assert_eq!(picks(&view, cx), Vec::<Facet>::new());
}

/// Whether `inner` lies inside `outer`, allowing half a pixel.
fn inside(
    inner: gpui_kit::Bounds<gpui_kit::Pixels>,
    outer: gpui_kit::Bounds<gpui_kit::Pixels>,
) -> bool {
    let slack = px(0.5);
    inner.top() >= outer.top() - slack && inner.bottom() <= outer.bottom() + slack
}

/// A short window and many long picks: the bar reaches its 40 percent cap.
/// A pick that takes the keyboard by Tab, or after the pick it held is
/// removed, scrolls into view.
#[gpui_kit::test]
fn a_focused_pick_scrolls_into_view_when_the_bar_is_capped(cx: &mut TestAppContext) {
    let long: Vec<Facet> = (0..20)
        .map(|i| tag(&format!("longtagname{i:02}")))
        .collect();
    let mut text: String = (0..20)
        .map(|i| format!("#longtagname{i:02} piece {i}\n\n"))
        .collect();
    text.push_str("#other\n");
    let (view, cx, _events, _sub) = open(cx, &text, long.clone());
    cx.simulate_resize(gpui_kit::size(px(320.), px(260.)));
    settle(cx);
    focus_view(&view, cx);
    let last = pill_id(&long[19]);
    tab_to(cx, last.clone(), 40);
    settle(cx);
    let bar = bounds(cx, FACET_BAR_ID);
    assert!(
        f32::from(bar.size.height) <= 260.0 * 0.4 + 0.5,
        "capped: {bar:?}"
    );
    let b = bounds(cx, last.clone());
    assert!(inside(b, bar), "the last pick in view: {b:?} {bar:?}");
    // Shift+Tab back to the first chip: the bar scrolls up again.
    let first = menu_id(ListKind::Tags);
    for _ in 0..30 {
        if has_focus(cx, first.clone()) {
            break;
        }
        key(cx, "shift-tab");
    }
    assert!(has_focus(cx, first.clone()));
    let chip = bounds(cx, first);
    assert!(
        inside(chip, bounds(cx, FACET_BAR_ID)),
        "the chip in view: {chip:?}"
    );
    // The last pick takes the keyboard, the bar is scrolled back to its
    // top, and the pick is unpicked: the pick before it scrolls into view.
    tab_to(cx, last.clone(), 40);
    cx.update(|window, cx| {
        let handle = view.read(cx).bar_scroll.clone();
        handle.set_offset(gpui_kit::point(px(0.), px(0.)));
        window.refresh();
        window.render_frame(cx);
    });
    let hidden = bounds(cx, last);
    assert!(
        !inside(hidden, bounds(cx, FACET_BAR_ID)),
        "the last pick is out of view before the unpick: {hidden:?}"
    );
    key(cx, "enter");
    settle(cx);
    let next = pill_id(&long[18]);
    assert!(has_focus(cx, next.clone()));
    let b = bounds(cx, next);
    let bar = bounds(cx, FACET_BAR_ID);
    assert!(
        inside(b, bar),
        "the new focused pick in view: {b:?} {bar:?}"
    );
}

#[gpui_kit::test]
fn a_menu_chip_opens_only_its_kinds_list(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, &crowded(), vec![]);
    settle(cx);
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 60")
    );
    click(cx, menu_id(ListKind::Tags));
    assert!(list_open(&view, cx));
    assert!(field_focused(&view, cx));
    assert_eq!(results(&view, cx), crowded_tags(), "the tags, nothing else");
    assert!(!exists(cx, list_group_id(FacetKind::Tag)), "no group label");
    let chip = bounds(cx, menu_id(ListKind::Tags));
    let list = bounds(cx, MORE_LIST_ID);
    assert!(
        list.top() >= chip.bottom(),
        "under its chip: {list:?} {chip:?}"
    );
    assert_eq!(
        label(cx, more_item_id(&tag("tag00"))).as_deref(),
        Some("#tag00"),
        "a list pill reads as a tag"
    );
    key(cx, "escape");
    assert!(!list_open(&view, cx));
    assert!(has_focus(cx, menu_id(ListKind::Tags)), "back on the chip");

    click(cx, menu_id(ListKind::Links));
    assert_eq!(results(&view, cx), crowded_links());
    assert_eq!(
        label(cx, more_item_id(&link("link00"))).as_deref(),
        Some("[[link00]]")
    );
    // Another chip's list replaces the open one.
    click(cx, menu_id(ListKind::Sections));
    assert_eq!(results(&view, cx), [heading("head")]);
    click(cx, more_item_id(&heading("head")));
    assert_eq!(picks(&view, cx), [heading("head")]);
    assert_eq!(picks_changed(&events), 1);
    assert!(shows(cx, pill_id(&heading("head"))), "now in the picks row");
}

#[gpui_kit::test]
fn a_picked_heading_shows_as_a_pill_with_the_outline_icon(cx: &mut TestAppContext) {
    let text = "# Head\n\nsome #a\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![heading("head"), tag("a")]);
    let pill = heading("head");
    assert_eq!(label(cx, pill_id(&pill)).as_deref(), Some("Remove Head"));
    assert!(shows(cx, outline_mark_id(&pill)), "the outline icon");
    let icon = bounds(cx, outline_mark_id(&pill));
    let b = bounds(cx, pill_id(&pill));
    assert!(icon.left() >= b.left() && icon.right() <= b.right());
    assert!(
        icon.right() <= b.center().x,
        "in front of the text: {icon:?} {b:?}"
    );
    assert!(shows(cx, remove_mark_id(&pill)), "and a remove mark");
    assert!(f32::from(b.size.height) <= 24.5, "still 24 px: {b:?}");
    // A picked tag has its remove mark but no outline icon.
    assert!(shows(cx, remove_mark_id(&tag("a"))));
    assert!(!shows(cx, outline_mark_id(&tag("a"))), "only on headings");
}

#[gpui_kit::test]
fn slash_opens_the_all_kinds_list_with_three_groups(cx: &mut TestAppContext) {
    let text = "# Head\n\n#b #a #a [[x]]\n";
    let (view, cx, events, _sub) = open(cx, text, vec![]);
    focus_view(&view, cx);
    key(cx, "/");
    assert!(list_open(&view, cx));
    assert!(field_focused(&view, cx));
    assert_eq!(query(&view, cx), "", "the slash is not typed into it");
    let groups = [
        (FacetKind::Tag, "Tags"),
        (FacetKind::NoteLink, "Links"),
        (FacetKind::Heading, "Sections"),
    ];
    for (kind, name) in groups {
        assert_eq!(label(cx, list_group_id(kind)).as_deref(), Some(name));
    }
    let field = bounds(cx, MORE_QUERY_ID);
    let tags = bounds(cx, list_group_id(FacetKind::Tag));
    let links = bounds(cx, list_group_id(FacetKind::NoteLink));
    let sections = bounds(cx, list_group_id(FacetKind::Heading));
    assert!(field.bottom() <= tags.top(), "the filter field first");
    assert!(tags.bottom() <= links.top() && links.bottom() <= sections.top());
    assert_eq!(
        results(&view, cx),
        [tag("a"), tag("b"), link("x"), heading("head")],
        "one flat order in group order"
    );
    let chip = bounds(cx, menu_id(ListKind::All));
    assert!(bounds(cx, MORE_LIST_ID).top() >= chip.bottom(), "under ⌕");
    // A filter that matches no tag leaves the Tags group out.
    let input = view.read_with(cx, |view, _| view.more_input().unwrap());
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_value("x", window, cx)));
    cx.run_until_parked();
    assert!(!shows(cx, list_group_id(FacetKind::Tag)));
    assert_eq!(results(&view, cx), [link("x")]);
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [link("x")]);
    key(cx, "escape");
    assert!(!list_open(&view, cx));
    assert!(has_focus(cx, menu_id(ListKind::All)), "back on ⌕");
    assert_eq!(picks_changed(&events), 1);
}

/// Every tag and link picked: `/` still opens the all-kinds list, with the
/// headings to pick and the picked tag ticked above them.
#[gpui_kit::test]
fn slash_lists_the_headings_when_every_tag_and_link_is_picked(cx: &mut TestAppContext) {
    let text = "# A\n\ntext #a\n\n# B\n\nmore\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("a")]);
    focus_view(&view, cx);
    key(cx, "/");
    assert!(list_open(&view, cx));
    assert_eq!(results(&view, cx), [tag("a"), heading("a"), heading("b")]);
    assert!(
        shows(cx, row_tick_id(&tag("a"))),
        "the picked tag is ticked"
    );
    assert!(shows(cx, list_group_id(FacetKind::Tag)));
    assert!(shows(cx, list_group_id(FacetKind::Heading)));
}

#[gpui_kit::test]
fn slash_opens_nothing_when_everything_is_picked(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "# A\n\ntext #a\n", vec![tag("a"), heading("a")]);
    focus_view(&view, cx);
    key(cx, "/");
    assert!(!list_open(&view, cx));
    assert!(view_focused(&view, cx), "the keyboard stays on the view");
}

/// Picking the last unpicked facet of the open list's scope keeps the list
/// open with every row ticked, so the pick shows. Escape closes it. Its chip
/// has nothing left to open, so it goes on the next frame and the keyboard
/// goes to the view.
#[gpui_kit::test]
fn the_list_stays_open_when_its_last_facet_is_picked(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "# A\n\ntext #a #b\n", vec![tag("a")]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    assert_eq!(results(&view, cx), [tag("a"), tag("b")], "a ticked, b left");
    key(cx, "down");
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("a"), tag("b")]);
    assert!(list_open(&view, cx), "it stays open to show the pick");
    assert!(field_focused(&view, cx), "the field keeps the keyboard");
    assert_eq!(results(&view, cx), [tag("a"), tag("b")]);
    assert_eq!(checked(cx, more_item_id(&tag("a"))), Some(true));
    assert_eq!(checked(cx, more_item_id(&tag("b"))), Some(true));
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 0"),
        "the chip stays while its list is open"
    );
    key(cx, "escape");
    assert!(!list_open(&view, cx));
    assert!(!shows(cx, MORE_LIST_ID));
    assert!(!shows(cx, menu_id(ListKind::Tags)), "nothing left to open");
    assert!(
        view_focused(&view, cx),
        "the chip went: the view has the keyboard"
    );
    // The all-kinds list stays open too once every facet is picked.
    key(cx, "/");
    assert_eq!(results(&view, cx), [tag("a"), tag("b"), heading("a")]);
    key(cx, "down");
    key(cx, "down");
    key(cx, "enter");
    assert!(list_open(&view, cx), "every row ticked, still open");
    assert_eq!(checked(cx, more_item_id(&heading("a"))), Some(true));
    assert!(shows(cx, menu_id(ListKind::All)), "the search chip stays");
    key(cx, "escape");
    assert!(!list_open(&view, cx));
    assert!(!shows(cx, menu_id(ListKind::All)));
    assert!(view_focused(&view, cx));
    key(cx, "/");
    assert!(
        !list_open(&view, cx),
        "nothing left to pick: `/` opens nothing"
    );
}

#[gpui_kit::test]
fn unpicking_a_row_when_every_row_is_ticked_brings_it_back(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "# A\n\ntext #a #b\n", vec![tag("a")]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    key(cx, "down");
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("a"), tag("b")]);
    assert!(lit(cx, more_item_id(&tag("b"))));
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("a")]);
    assert!(list_open(&view, cx), "the list stays open");
    assert_eq!(checked(cx, more_item_id(&tag("b"))), Some(false));
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 1"),
        "the chip counts it again"
    );
    key(cx, "escape");
    assert!(has_focus(cx, menu_id(ListKind::Tags)), "the chip is back");
}

#[gpui_kit::test]
fn a_chip_with_no_results_is_drawn_dimmed_and_still_opens(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "[[x]] one\n\n#b two\n", vec![]);
    focus_view(&view, cx);
    cx.update(|window, cx| window.render_frame(cx));
    let dimmed = view.read_with(cx, |view, _| view.chips_drawn_dimmed());
    assert!(dimmed.is_empty(), "every chip has results: {dimmed:?}");
    view.update(cx, |view, cx| view.toggle(link("x"), cx));
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 0")
    );
    let dimmed = view.read_with(cx, |view, _| view.chips_drawn_dimmed());
    assert_eq!(dimmed, [ListKind::Tags]);
    let stops = drawn_stops(&view, cx);
    assert!(stops.contains(&menu_id(ListKind::Tags)), "still a tab stop");
    click(cx, menu_id(ListKind::Tags));
    assert!(list_open(&view, cx), "still opens");
    assert_eq!(results(&view, cx), [tag("b")]);
}

/// A note with 20 tags, each in one place.
fn twenty_tags() -> String {
    (0..20).map(|i| format!("#t{i:02} ")).collect::<String>() + "\n"
}

/// Whether the row of `facet` is drawn and lies inside the open list.
fn row_in_view(cx: &mut VisualTestContext, facet: &Facet) -> bool {
    let id = more_item_id(facet);
    if !shows(cx, id.clone()) {
        return false;
    }
    let row = bounds(cx, id);
    let list = bounds(cx, MORE_LIST_ID);
    let slack = px(0.5);
    row.top() >= list.top() - slack && row.bottom() <= list.bottom() + slack
}

#[gpui_kit::test]
fn the_highlight_stays_in_view_while_down_walks_a_long_list(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &twenty_tags(), vec![]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    for _ in 0..15 {
        key(cx, "down");
    }
    let lit_row = results(&view, cx)[15].clone();
    assert!(lit(cx, more_item_id(&lit_row)));
    assert!(row_in_view(cx, &lit_row), "the lit row scrolled into view");
    for _ in 0..15 {
        key(cx, "up");
    }
    let first = results(&view, cx)[0].clone();
    assert!(row_in_view(cx, &first), "and back up to the first");
}

/// Whether `id` is drawn and lies inside the open list.
fn id_in_view(cx: &mut VisualTestContext, id: SharedString) -> bool {
    if !shows(cx, id.clone()) {
        return false;
    }
    let el = bounds(cx, id);
    let list = bounds(cx, MORE_LIST_ID);
    let slack = px(0.5);
    el.top() >= list.top() - slack && el.bottom() <= list.bottom() + slack
}

/// Up onto the first row of a group in the search list scrolls to the
/// group's label too, so the label is not left just above the rows.
#[gpui_kit::test]
fn up_onto_a_group_s_first_row_shows_its_label(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &crowded(), vec![]);
    focus_view(&view, cx);
    key(cx, "/");
    let rows = results(&view, cx);
    let first_link = rows
        .iter()
        .position(|f| f.kind == FacetKind::NoteLink)
        .unwrap();
    for _ in 0..rows.len() {
        key(cx, "down");
    }
    while !lit(cx, more_item_id(&rows[first_link + 1])) {
        key(cx, "up");
    }
    let label = list_group_id(FacetKind::NoteLink);
    assert!(
        !id_in_view(cx, label.clone()),
        "the label is above the rows"
    );
    key(cx, "up");
    assert!(lit(cx, more_item_id(&rows[first_link])));
    assert!(row_in_view(cx, &rows[first_link]));
    assert!(id_in_view(cx, label), "the label scrolled into view");
}

/// A note with 50 tags, each in one place.
fn fifty_tags() -> String {
    (0..50).map(|i| format!("#t{i:02} ")).collect::<String>() + "\n"
}

/// The rows keep their order while the list is open: Enter ticks the lit
/// row where it is, the highlight stays on it and the list does not
/// scroll, so the next Down reaches the next row. An unpick leaves it
/// there too.
#[gpui_kit::test]
fn a_pick_keeps_the_row_and_the_highlight_in_place(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &fifty_tags(), vec![]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    let before = results(&view, cx);
    for _ in 0..10 {
        key(cx, "down");
    }
    let tenth = before[10].clone();
    assert!(lit(cx, more_item_id(&tenth)));
    let at = bounds(cx, more_item_id(&tenth));
    key(cx, "enter");
    assert_eq!(picks(&view, cx), std::slice::from_ref(&tenth));
    assert_eq!(results(&view, cx), before, "no row moved");
    assert!(
        lit(cx, more_item_id(&tenth)),
        "the highlight stays on row 10"
    );
    assert_eq!(checked(cx, more_item_id(&tenth)), Some(true));
    assert_eq!(
        bounds(cx, more_item_id(&tenth)),
        at,
        "the list did not scroll"
    );
    key(cx, "down");
    assert!(lit(cx, more_item_id(&before[11])), "Down reaches row 11");
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tenth.clone(), before[11].clone()]);
    assert_eq!(results(&view, cx), before);
    key(cx, "up");
    key(cx, "enter");
    assert_eq!(picks(&view, cx), std::slice::from_ref(&before[11]));
    assert_eq!(results(&view, cx), before, "an unpick moves nothing");
    assert!(lit(cx, more_item_id(&tenth)));
}

/// A click on a row in the middle leaves every row where it was: nothing
/// moves under the pointer.
#[gpui_kit::test]
fn a_click_on_a_middle_row_moves_no_other_row(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &twenty_tags(), vec![]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    let before = results(&view, cx);
    let places: Vec<_> = before[..8]
        .iter()
        .map(|f| bounds(cx, more_item_id(f)))
        .collect();
    click(cx, more_item_id(&before[4]));
    assert_eq!(picks(&view, cx), std::slice::from_ref(&before[4]));
    assert_eq!(results(&view, cx), before, "no row moved");
    let after: Vec<_> = before[..8]
        .iter()
        .map(|f| bounds(cx, more_item_id(f)))
        .collect();
    assert_eq!(after, places, "every row stays where it was drawn");
    assert!(lit(cx, more_item_id(&before[4])), "the clicked row is lit");
}

/// The order is set when a list opens: open again and the picks come
/// first.
#[gpui_kit::test]
fn opening_a_list_again_sorts_the_picks_first(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &twenty_tags(), vec![]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    let before = results(&view, cx);
    click(cx, more_item_id(&before[7]));
    assert_eq!(results(&view, cx)[7], before[7], "it stays while open");
    key(cx, "escape");
    assert!(!list_open(&view, cx));
    click(cx, menu_id(ListKind::Tags));
    assert_eq!(results(&view, cx)[0], before[7], "now the pick is first");
}

/// The order is set again when the filter changes: the picks come first.
#[gpui_kit::test]
fn a_new_query_sorts_the_picks_first(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &twenty_tags(), vec![]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    let before = results(&view, cx);
    click(cx, more_item_id(&before[7]));
    assert_eq!(results(&view, cx)[7], before[7], "it stays while open");
    // Typed, not set: only typing tells the view the query changed.
    key(cx, "t");
    assert_eq!(results(&view, cx)[0], before[7], "now the pick is first");
}

#[gpui_kit::test]
fn a_new_query_scrolls_the_list_to_the_top(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &twenty_tags(), vec![]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    for _ in 0..15 {
        key(cx, "down");
    }
    let first = results(&view, cx)[0].clone();
    assert!(!row_in_view(cx, &first), "scrolled down");
    // Typed, not set: only typing tells the view the query changed.
    key(cx, "t");
    assert_eq!(query(&view, cx), "t");
    let first = results(&view, cx)[0].clone();
    assert!(
        lit(cx, more_item_id(&first)),
        "a new query lights the first"
    );
    assert!(row_in_view(cx, &first), "and shows it");
}

#[gpui_kit::test]
fn a_list_draws_at_most_the_cap_and_says_how_many_more(cx: &mut TestAppContext) {
    use super::facet::LIST_CAP;
    let tags: String = (0..500).map(|i| format!("#t{i:03} ")).collect();
    let text = format!("#pick here\n\n{tags}\n");
    let (view, cx, _events, _sub) = open(cx, &text, vec![tag("pick")]);
    click(cx, menu_id(ListKind::Tags));
    let shown = results(&view, cx);
    assert_eq!(shown.len(), LIST_CAP);
    // The picked row comes first and counts toward the cap.
    assert_eq!(shown[0], tag("pick"));
    assert_eq!(shown[LIST_CAP - 1], tag("t198"));
    assert!(exists(cx, more_item_id(&tag("t198"))));
    assert!(!exists(cx, more_item_id(&tag("t199"))));
    assert_eq!(
        label(cx, LIST_REST_ID).as_deref(),
        Some("301 more. Type to filter.")
    );
    let rest = bounds(cx, LIST_REST_ID);
    let items = bounds(cx, more_item_id(&tag("t000")));
    assert!(rest.top() >= items.bottom(), "the last line");
    // A filter that matches fewer than the cap draws them all, no rest line.
    let input = view.read_with(cx, |view, _| view.more_input().unwrap());
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_value("t49", window, cx)));
    cx.run_until_parked();
    assert!(results(&view, cx).len() < LIST_CAP);
    assert!(!exists(cx, LIST_REST_ID));
}

#[gpui_kit::test]
fn tab_order_is_chips_search_picks_clear(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, MENUS_NOTE, vec![tag("a"), link("x")]);
    focus_view(&view, cx);
    let want: Vec<SharedString> = vec![
        menu_id(ListKind::Tags),
        menu_id(ListKind::Links),
        menu_id(ListKind::Sections),
        menu_id(ListKind::All),
        pill_id(&tag("a")),
        pill_id(&link("x")),
        CLEAR_ID.into(),
    ];
    let drawn = drawn_stops(&view, cx);
    assert_eq!(drawn[..want.len()], want[..], "the bar's stops lead");
    for id in &want {
        key(cx, "tab");
        let ringed = view.read_with(cx, |view, _| view.ringed());
        assert!(has_focus(cx, id.clone()), "Tab reached {id}: {ringed:?}");
        assert_eq!(ringed, Some(id.clone()));
    }
    // Enter on a chip opens its list; Escape brings the keyboard back.
    tab_to(cx, menu_id(ListKind::Links), 20);
    key(cx, "enter");
    assert!(list_open(&view, cx));
    assert_eq!(results(&view, cx), [link("x"), link("y")]);
    key(cx, "escape");
    assert!(has_focus(cx, menu_id(ListKind::Links)));
}

#[gpui_kit::test]
fn enter_on_a_pick_unpicks_it_and_the_keyboard_moves_on_without_a_dead_ring(
    cx: &mut TestAppContext,
) {
    let (view, cx, _events, _sub) = open(cx, "#a #b #c\n", vec![tag("a"), tag("b"), tag("c")]);
    focus_view(&view, cx);
    tab_to(cx, pill_id(&tag("a")), 10);
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("b"), tag("c")]);
    let ringed = view.read_with(cx, |view, _| view.ringed());
    assert!(
        has_focus(cx, pill_id(&tag("b"))),
        "the next pick: {ringed:?}"
    );
    assert_eq!(ringed, Some(pill_id(&tag("b"))));
    // The last pick in the row hands the keyboard to the one before it.
    tab_to(cx, pill_id(&tag("c")), 10);
    key(cx, "space");
    assert_eq!(picks(&view, cx), [tag("b")]);
    assert!(has_focus(cx, pill_id(&tag("b"))));
    assert_eq!(
        view.read_with(cx, |view, _| view.ringed()),
        Some(pill_id(&tag("b")))
    );
    // The only pick: the view takes the keyboard, no ring is left.
    key(cx, "enter");
    assert!(picks(&view, cx).is_empty());
    assert!(view_focused(&view, cx));
    cx.update(|window, cx| window.render_frame(cx));
    assert_eq!(view.read_with(cx, |view, _| view.ringed()), None);
}

#[gpui_kit::test]
fn escape_closes_an_open_list_first(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, "#a #b\n", vec![tag("a")]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    assert!(list_open(&view, cx));
    key(cx, "escape");
    assert!(!list_open(&view, cx), "the list closes first");
    assert_eq!(picks(&view, cx), [tag("a")], "the picks stay");
    assert!(has_focus(cx, menu_id(ListKind::Tags)));
    assert_eq!(picks_changed(&events), 0);
    key(cx, "escape");
    assert!(picks(&view, cx).is_empty(), "then the picks clear");
}

#[gpui_kit::test]
fn drawing_5000_facets_asks_for_no_correcting_frame_for_the_bar(cx: &mut TestAppContext) {
    let tags: String = (0..5000).map(|i| format!("#t{i:04} ")).collect();
    let text = format!("#pick here\n\n{tags}\n");
    let (view, cx, _events, _sub) = open(cx, &text, vec![tag("pick")]);
    settle(cx);
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 5000")
    );
    cx.update(|window, cx| window.render_frame(cx));
    assert_eq!(next_frame(cx), 0, "a drawn bar asks for nothing");
    click(cx, menu_id(ListKind::Tags));
    assert_eq!(results(&view, cx).len(), super::facet::LIST_CAP);
    cx.update(|window, cx| window.render_frame(cx));
    assert_no_correction(&view, cx, "an open list");
}

#[gpui_kit::test]
fn no_old_bar_element_remains(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, &crowded(), vec![heading("head")]);
    settle(cx);
    for id in [
        "facet-search",
        "facet-more",
        "facet-sections",
        "facet-line-pills",
        "facet-line-end",
        "facet-leader:tag",
        "facet-more:tag",
    ] {
        assert!(!shows(cx, id), "{id} is gone");
    }
}

#[gpui_kit::test]
fn the_bar_does_not_change_the_piece_column_width(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &crowded(), vec![]);
    settle(cx);
    let whole = bounds(cx, FACET_VIEW_ID);
    let column = bounds(cx, COLUMN_ID);
    let piece = bounds(cx, chunk_id(0));
    assert_eq!(column.size.width, whole.size.width);
    view.update(cx, |view, cx| view.set_text("#a\n\nplain\n".into(), cx));
    settle(cx);
    assert_eq!(bounds(cx, COLUMN_ID).size.width, column.size.width);
    assert_eq!(bounds(cx, chunk_id(0)).size.width, piece.size.width);
    // A narrow pane: the column is the pane's width, never wider.
    cx.simulate_resize(gpui_kit::size(px(320.), px(600.)));
    view.update(cx, |view, cx| view.set_text(crowded(), cx));
    settle(cx);
    let whole = bounds(cx, FACET_VIEW_ID);
    assert_eq!(bounds(cx, COLUMN_ID).size.width, whole.size.width);
}

/// A lone group floats its corner buttons over the top-right of the view:
/// no chip or pick sits under them, and a click on a menu chip opens its
/// list, not the note menu.
#[gpui_kit::test]
fn the_corner_padding_still_clears_the_lone_pane_buttons(cx: &mut TestAppContext) {
    use super::facet_tests::in_a_group::{
        active, bounds_of, click as click_in, facet, open_facet, started,
    };
    use super::group_header::{CORNER_FACET_ID, CORNER_MENU_ID};
    let rt = super::start_tests::runtime();
    let _guard = rt.enter();
    let text = crowded();
    let (_v, s) = started(cx, &rt, &[("a.md", text.as_str())], "a.md");
    let group = active(&s, cx);
    open_facet(&s, &rt, group, "a.md", vec![tag("tag00")], cx);
    let view = facet(&s, group, cx).unwrap();
    let corner = bounds_of(&s, CORNER_FACET_ID, cx).union(&bounds_of(&s, CORNER_MENU_ID, cx));
    let controls: Vec<SharedString> = vec![
        menu_id(ListKind::Tags),
        menu_id(ListKind::Links),
        menu_id(ListKind::Sections),
        menu_id(ListKind::All),
        pill_id(&tag("tag00")),
        CLEAR_ID.into(),
    ];
    for id in &controls {
        let b = bounds_of(&s, id.clone(), cx);
        assert!(
            !b.intersects(&corner),
            "{id} {b:?} under the corner {corner:?}"
        );
    }
    click_in(&s, menu_id(ListKind::Tags), cx);
    assert!(
        cx.read(|cx| view.read(cx).more_input().is_some()),
        "the list opened"
    );
    assert!(
        !cx.read(|cx| s.workspace.read(cx).note_menu.is_some()),
        "the note menu did not"
    );
}

const RECOUNT_NOTE: &str = "#a [[x]]\n\n#b [[y]] [[z]]\n";

#[gpui_kit::test]
fn a_pick_recounts_the_links_chip(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, RECOUNT_NOTE, vec![]);
    let links = || menu_id(ListKind::Links);
    assert_eq!(label(cx, links()).as_deref(), Some("Links 3"));
    view.update(cx, |view, cx| view.toggle(tag("a"), cx));
    assert_eq!(label(cx, links()).as_deref(), Some("Links 1"));
    view.update(cx, |view, cx| view.toggle(tag("a"), cx));
    assert_eq!(label(cx, links()).as_deref(), Some("Links 3"));
}

#[gpui_kit::test]
fn a_text_change_recounts_the_links_chip(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, RECOUNT_NOTE, vec![tag("a")]);
    let links = || menu_id(ListKind::Links);
    assert_eq!(label(cx, links()).as_deref(), Some("Links 1"));
    view.update(cx, |view, cx| {
        view.set_text("#a [[x]] [[y]]\n\n#b [[z]]\n".into(), cx)
    });
    assert_eq!(label(cx, links()).as_deref(), Some("Links 2"));
}

// ── the lists as rows (slice 6) ──────────────────────────────────────────

/// The snapshot of the element `id` in a fresh frame, if drawn.
fn snapshot(
    cx: &mut VisualTestContext,
    id: impl Into<SharedString>,
) -> Option<gpui_kit::test::ElementSnapshot> {
    let id: SharedString = id.into();
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.try_find(id)
    })
}

/// Whether the element `id` reports itself checked (a ticked row).
fn checked(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> Option<bool> {
    snapshot(cx, id).and_then(|e| e.checked())
}

/// Whether the element `id` is marked selected (the list's highlight).
fn lit(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> bool {
    snapshot(cx, id).and_then(|e| e.selected()).unwrap_or(false)
}

/// The expanded state the element `id` reports.
fn expanded(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> Option<bool> {
    snapshot(cx, id).and_then(|e| e.expanded())
}

/// The accessibility description of the element `id`.
fn description(cx: &mut VisualTestContext, id: impl Into<SharedString>) -> Option<String> {
    snapshot(cx, id).and_then(|e| e.description().map(str::to_string))
}

/// The count drawn on `facet`'s row.
fn count(cx: &mut VisualTestContext, facet: &Facet) -> Option<String> {
    label(cx, row_count_id(facet))
}

fn set_filter(
    view: &gpui_kit::Entity<super::facet::FacetView>,
    cx: &mut VisualTestContext,
    text: &str,
) {
    let input = view.read_with(cx, |view, _| view.more_input().expect("the list is open"));
    let text = text.to_string();
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_value(text, window, cx)));
    cx.run_until_parked();
}

const ROWS_NOTE: &str = "#a one\n\n#a two\n\n#b three\n";

/// A picked pill shows the same number as its row: the pieces it gives in
/// view, not its occurrences in the whole note.
#[gpui_kit::test]
fn a_picked_pill_shows_the_count_its_row_shows(cx: &mut TestAppContext) {
    let text = "#c #c one\n\n#c two\n\n#d\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![tag("c")]);
    settle(cx);
    assert_eq!(label(cx, pill_count_id(&tag("c"))).as_deref(), Some("2"));
    click(cx, menu_id(ListKind::Tags));
    assert_eq!(count(cx, &tag("c")).as_deref(), Some("2"));
}

#[gpui_kit::test]
fn a_list_row_shows_label_count_and_a_tick_when_picked(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, ROWS_NOTE, vec![tag("a")]);
    settle(cx);
    click(cx, menu_id(ListKind::Tags));
    assert_eq!(
        results(&view, cx),
        [tag("a"), tag("b")],
        "the pick stays, first"
    );
    let (a, b) = (more_item_id(&tag("a")), more_item_id(&tag("b")));
    assert_eq!(
        label(cx, a.clone()).as_deref(),
        Some("#a"),
        "its own syntax"
    );
    assert_eq!(label(cx, b.clone()).as_deref(), Some("#b"));
    assert_eq!(checked(cx, a.clone()), Some(true), "the pick is ticked");
    assert_eq!(checked(cx, b.clone()), Some(false));
    assert!(shows(cx, row_tick_id(&tag("a"))));
    assert!(
        !exists(cx, row_tick_id(&tag("b"))),
        "no tick when not picked"
    );
    // The count is the pieces each gives in view: `a` sits in two blocks.
    assert_eq!(count(cx, &tag("a")).as_deref(), Some("2"));
    assert_eq!(count(cx, &tag("b")).as_deref(), Some("1"));
    assert_eq!(
        description(cx, a.clone()).as_deref(),
        Some("2 pieces in view")
    );
    assert_eq!(
        description(cx, b.clone()).as_deref(),
        Some("1 piece in view")
    );
    // Rows: stacked, each as wide as the list, the tick first, the count
    // at the right end.
    let (ra, rb) = (bounds(cx, a), bounds(cx, b));
    let list = bounds(cx, MORE_LIST_ID);
    let slack = px(0.5);
    assert!(
        rb.top() >= ra.bottom() - slack,
        "one row each: {ra:?} {rb:?}"
    );
    assert_eq!(ra.left(), rb.left());
    assert_eq!(ra.size.width, rb.size.width);
    assert!(
        ra.size.width > list.size.width * 0.8,
        "a row spans the list: {ra:?} {list:?}"
    );
    assert!(f32::from(ra.size.height) >= 24.0, "a 24 px target: {ra:?}");
    let tick = bounds(cx, row_tick_id(&tag("a")));
    assert!(tick.left() >= ra.left() && tick.right() < ra.center().x);
    let (ca, cb) = (
        bounds(cx, row_count_id(&tag("a"))),
        bounds(cx, row_count_id(&tag("b"))),
    );
    assert!(ca.left() > ra.center().x, "right-aligned: {ca:?} {ra:?}");
    assert!(
        ra.right() - ca.right() < px(16.),
        "at the end: {ca:?} {ra:?}"
    );
    assert!((f32::from(ca.right() - cb.right())).abs() < 0.5, "aligned");
}

#[gpui_kit::test]
fn a_row_with_no_result_in_view_is_dimmed_and_last(cx: &mut TestAppContext) {
    let text = "#a [[zed]]\n\n[[alpha]] one\n\n#b [[beta]]\n";
    let (view, cx, events, _sub) = open(cx, text, vec![tag("a")]);
    click(cx, menu_id(ListKind::Links));
    assert_eq!(
        results(&view, cx),
        [link("zed"), link("alpha"), link("beta")],
        "a link with results first, the rest last"
    );
    let rows = view.read_with(cx, |view, cx| view.list_row_pills(cx));
    let dim: Vec<bool> = rows.iter().map(super::facet_pill::dimmed).collect();
    assert_eq!(dim, [false, true, true]);
    assert_eq!(count(cx, &link("alpha")).as_deref(), Some("0"));
    assert_eq!(
        description(cx, more_item_id(&link("alpha"))).as_deref(),
        Some("No pieces in view")
    );
    assert!(
        shows(cx, more_item_id(&link("alpha"))),
        "dimmed, not hidden"
    );
    // A dimmed row still picks: the view shows the empty message.
    click(cx, more_item_id(&link("alpha")));
    assert_eq!(picks(&view, cx), [tag("a"), link("alpha")]);
    assert_eq!(picks_changed(&events), 1);
    assert!(shows(cx, EMPTY_ID), "nothing matches both picks");
    assert!(list_open(&view, cx));
    let rows = view.read_with(cx, |view, cx| view.list_row_pills(cx));
    assert_eq!(rows[1].facet, link("alpha"), "it keeps its place");
    assert!(
        !super::facet_pill::dimmed(&rows[1]),
        "a pick is never dimmed"
    );
}

#[gpui_kit::test]
fn picking_in_a_list_ticks_the_row_and_keeps_the_list_open(cx: &mut TestAppContext) {
    let text = "#a one\n\n#b two\n\n#c three\n";
    let (view, cx, events, _sub) = open(cx, text, vec![]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    click(cx, more_item_id(&tag("b")));
    assert_eq!(picks(&view, cx), [tag("b")]);
    assert!(list_open(&view, cx), "it stays open");
    assert!(field_focused(&view, cx), "the field keeps the keyboard");
    assert_eq!(checked(cx, more_item_id(&tag("b"))), Some(true));
    assert!(shows(cx, row_tick_id(&tag("b"))));
    assert_eq!(
        results(&view, cx),
        [tag("a"), tag("b"), tag("c")],
        "the row stays in place while the list is open"
    );
    click(cx, more_item_id(&tag("a")));
    assert_eq!(picks(&view, cx), [tag("b"), tag("a")]);
    assert_eq!(results(&view, cx), [tag("a"), tag("b"), tag("c")]);
    assert!(list_open(&view, cx));
    assert_eq!(picks_changed(&events), 2);
}

#[gpui_kit::test]
fn enter_on_a_ticked_row_unpicks_it(cx: &mut TestAppContext) {
    let text = "#a one\n\n#b two\n\n#c three\n";
    let (view, cx, events, _sub) = open(cx, text, vec![tag("b")]);
    focus_view(&view, cx);
    click(cx, menu_id(ListKind::Tags));
    assert_eq!(results(&view, cx), [tag("b"), tag("a"), tag("c")]);
    assert!(lit(cx, more_item_id(&tag("b"))), "the first row is lit");
    key(cx, "enter");
    assert!(picks(&view, cx).is_empty(), "Enter unpicked it");
    assert_eq!(picks_changed(&events), 1);
    assert!(list_open(&view, cx), "the list stays open");
    assert_eq!(checked(cx, more_item_id(&tag("b"))), Some(false));
    assert_eq!(
        results(&view, cx),
        [tag("b"), tag("a"), tag("c")],
        "it keeps its place while the list is open"
    );
    assert!(
        lit(cx, more_item_id(&tag("b"))),
        "the highlight stays on it"
    );
}

#[gpui_kit::test]
fn up_and_down_move_one_row_in_drawn_order(cx: &mut TestAppContext) {
    let text = "# Head\n\n#a #b [[x]]\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("b")]);
    focus_view(&view, cx);
    key(cx, "/");
    let order = [tag("b"), tag("a"), link("x"), heading("head")];
    assert_eq!(results(&view, cx), order, "the pick first, then by group");
    let mut last_top = None;
    for (i, facet) in order.iter().enumerate() {
        if i > 0 {
            key(cx, "down");
        }
        for (j, other) in order.iter().enumerate() {
            assert_eq!(lit(cx, more_item_id(other)), i == j, "{i}: {other:?}");
        }
        let top = bounds(cx, more_item_id(facet)).top();
        assert!(last_top.is_none_or(|t| top > t), "drawn in this order");
        last_top = Some(top);
    }
    key(cx, "down");
    assert!(
        lit(cx, more_item_id(&heading("head"))),
        "Down stops at the last"
    );
    for _ in 0..5 {
        key(cx, "up");
    }
    assert!(lit(cx, more_item_id(&tag("b"))), "Up stops at the first");
    assert_eq!(picks(&view, cx), [tag("b")], "moving picks nothing");
}

#[gpui_kit::test]
fn an_empty_filter_says_what_found_nothing(cx: &mut TestAppContext) {
    let text = "# Head\n\n#a [[x]]\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![]);
    focus_view(&view, cx);
    let cases = [
        (ListKind::Tags, "No tags match \"zzz\""),
        (ListKind::Links, "No links match \"zzz\""),
        (ListKind::Sections, "No sections match \"zzz\""),
        (ListKind::All, "No matches for \"zzz\""),
    ];
    for (kind, want) in cases {
        click(cx, menu_id(kind));
        assert!(list_open(&view, cx), "{kind:?}");
        assert!(!exists(cx, LIST_EMPTY_ID), "{kind:?}: rows match");
        set_filter(&view, cx, "zzz");
        assert_eq!(label(cx, LIST_EMPTY_ID).as_deref(), Some(want));
        assert!(results(&view, cx).is_empty());
        key(cx, "down");
        key(cx, "enter");
        assert!(picks(&view, cx).is_empty(), "{kind:?}: nothing to pick");
        assert!(list_open(&view, cx));
        set_filter(&view, cx, "");
        assert!(!exists(cx, LIST_EMPTY_ID), "{kind:?}: gone with the filter");
        key(cx, "escape");
    }
    let stops = drawn_stops(&view, cx);
    assert!(!stops.iter().any(|id| id.as_ref() == LIST_EMPTY_ID));
}

#[gpui_kit::test]
fn the_open_chip_is_drawn_selected_and_reports_expanded(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, MENUS_NOTE, vec![]);
    focus_view(&view, cx);
    let kinds = [
        ListKind::Tags,
        ListKind::Links,
        ListKind::Sections,
        ListKind::All,
    ];
    for kind in kinds {
        assert_eq!(expanded(cx, menu_id(kind)), Some(false), "{kind:?}");
    }
    click(cx, menu_id(ListKind::Tags));
    assert_eq!(expanded(cx, menu_id(ListKind::Tags)), Some(true));
    assert_eq!(expanded(cx, menu_id(ListKind::Links)), Some(false));
    click(cx, menu_id(ListKind::Links));
    assert_eq!(expanded(cx, menu_id(ListKind::Tags)), Some(false));
    assert_eq!(expanded(cx, menu_id(ListKind::Links)), Some(true));
    key(cx, "escape");
    assert_eq!(expanded(cx, menu_id(ListKind::Links)), Some(false));
    let open_chip = view.read_with(cx, |view, _| view.chip_drawn_open());
    assert_eq!(open_chip, None, "no chip drawn open once it closed");
    key(cx, "/");
    assert_eq!(expanded(cx, menu_id(ListKind::All)), Some(true));
    let open_chip = view.read_with(cx, |view, _| view.chip_drawn_open());
    assert_eq!(
        open_chip,
        Some(ListKind::All),
        "drawn in the selected style"
    );
}

#[gpui_kit::test]
fn chip_numbers_count_facets_with_results(cx: &mut TestAppContext) {
    let text = "#a [[x]]\n\n#b one\n\n#c [[x]]\n\n[[y]] two\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![]);
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 3")
    );
    view.update(cx, |view, cx| view.toggle(link("x"), cx));
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 2"),
        "a and c give results under [[x]], b does not"
    );
    view.update(cx, |view, cx| {
        view.toggle(link("x"), cx);
        view.toggle(link("y"), cx);
    });
    assert_eq!(
        label(cx, menu_id(ListKind::Tags)).as_deref(),
        Some("Tags 0"),
        "tags left, none with a result: the chip stays"
    );
}

#[gpui_kit::test]
fn picking_a_tag_changes_the_counts_in_the_links_list(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, RECOUNT_NOTE, vec![]);
    click(cx, menu_id(ListKind::Links));
    assert_eq!(results(&view, cx), [link("x"), link("y"), link("z")]);
    for l in ["x", "y", "z"] {
        assert_eq!(count(cx, &link(l)).as_deref(), Some("1"), "{l}");
    }
    view.update(cx, |view, cx| view.toggle(tag("b"), cx));
    assert!(list_open(&view, cx), "the Links list stays open");
    assert_eq!(count(cx, &link("x")).as_deref(), Some("0"));
    assert_eq!(count(cx, &link("y")).as_deref(), Some("1"));
    assert_eq!(count(cx, &link("z")).as_deref(), Some("1"));
    assert_eq!(
        results(&view, cx),
        [link("x"), link("y"), link("z")],
        "the counts change, the order stays while the list is open"
    );
    key(cx, "escape");
    click(cx, menu_id(ListKind::Links));
    assert_eq!(
        results(&view, cx),
        [link("y"), link("z"), link("x")],
        "opened again: the link with no result goes last"
    );
}
