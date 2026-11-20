//! UI tests for the **Facet view**'s filter line (Option B of the design
//! review): one ranked line of pills that never wraps, the `+N` chip that
//! opens the list of every tag and link, and the `Sections N` chip. Each
//! test draws a `FacetView` alone in a headless window and measures the
//! bounds it painted (see `facet_tests` for the helpers).
#![cfg(test)]

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Pixels, SharedString, TestAppContext, VisualTestContext, px,
};
use kimun_core::note::{Facet, FacetKind};

use super::facet::{
    CLEAR_ID, COLUMN_ID, FACET_VIEW_ID, LINE_ID, LINE_PILLS_ID, MORE_ID, MORE_LIST_ID,
    MORE_QUERY_ID, SECTIONS_ID, SUMMARY_ID, chunk_id, list_group_id, more_item_id, outline_mark_id,
    pill_id, remove_mark_id,
};
use super::facet_tests::{
    bounds, click, field_focused, focus_view, has_focus, heading, key, label, list_open, open,
    picks, picks_changed, query, settle, shows, tag, view_focused,
};

fn link(key: &str) -> Facet {
    Facet::new(FacetKind::NoteLink, key)
}

/// The 60 tags and 20 links of [`crowded`], in line order (every count is
/// one, so by label: the links first).
fn crowded_facets() -> Vec<Facet> {
    let links = (0..20).map(|i| link(&format!("link{i:02}")));
    let tags = (0..60).map(|i| tag(&format!("tag{i:02}")));
    links.chain(tags).collect()
}

/// A note with a heading, then 60 tags and 20 links in one paragraph: far
/// more than one line holds.
fn crowded() -> String {
    let tags: String = (0..60).map(|i| format!("#tag{i:02} ")).collect();
    let links: String = (0..20).map(|i| format!("[[link{i:02}]] ")).collect();
    format!("# Head\n\nintro\n\n{tags}{links}\n")
}

/// The pills drawn in the line, with their bounds, left to right.
fn drawn_pills(cx: &mut VisualTestContext, facets: &[Facet]) -> Vec<(Facet, Bounds<Pixels>)> {
    let mut drawn: Vec<(Facet, Bounds<Pixels>)> = Vec::new();
    for facet in facets {
        if shows(cx, pill_id(facet)) {
            drawn.push((facet.clone(), bounds(cx, pill_id(facet))));
        }
    }
    drawn.sort_by(|a, b| a.1.left().partial_cmp(&b.1.left()).unwrap());
    drawn
}

fn more_count(
    view: &gpui_kit::Entity<super::facet::FacetView>,
    cx: &mut VisualTestContext,
) -> usize {
    view.read_with(cx, |view, _| view.more_count())
}

#[gpui_kit::test]
fn the_bar_is_one_line_with_many_facets(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, &crowded(), vec![]);
    settle(cx);
    let line = bounds(cx, LINE_ID);
    assert!(
        f32::from(line.size.height) <= 26.0,
        "one pill row tall: {line:?}"
    );
    let drawn = drawn_pills(cx, &crowded_facets());
    assert!(drawn.len() > 2, "some pills are drawn: {}", drawn.len());
    for (facet, b) in &drawn {
        assert_eq!(b.top(), drawn[0].1.top(), "{facet:?} on the one row");
        assert!(b.top() >= line.top() && b.bottom() <= line.bottom());
    }
    assert!(shows(cx, MORE_ID));
    assert!(shows(cx, SECTIONS_ID));
}

#[gpui_kit::test]
fn picked_pills_come_first_and_always_show(cx: &mut TestAppContext) {
    let picks_made = vec![tag("tag59"), link("link19")];
    let (view, cx, _events, _sub) = open(cx, &crowded(), picks_made.clone());
    settle(cx);
    let drawn = drawn_pills(cx, &crowded_facets());
    let order: Vec<Facet> = drawn.iter().map(|(f, _)| f.clone()).collect();
    assert_eq!(order[..2], picks_made[..], "the picks lead, in pick order");
    assert!(shows(cx, remove_mark_id(&tag("tag59"))), "a remove mark");
    assert_eq!(
        label(cx, pill_id(&tag("tag59"))).as_deref(),
        Some("Remove #tag59"),
        "a picked pill is named for what a press does"
    );
    let unpicked = pill_id(&link("link00"));
    assert_eq!(label(cx, unpicked).as_deref(), Some("[[link00]]"));
    assert!(shows(cx, remove_mark_id(&link("link19"))));
    assert!(
        !shows(cx, remove_mark_id(&order[2])),
        "no remove mark on an unpicked pill"
    );
    // A click anywhere on a picked pill unpicks it (its remove mark too).
    click(cx, remove_mark_id(&link("link19")));
    assert_eq!(picks(&view, cx), [tag("tag59")]);
    view.update(cx, |view, cx| view.toggle(link("link19"), cx));
    // In a narrow pane the picks stay, and the summary names every one.
    cx.simulate_resize(gpui_kit::size(px(300.), px(600.)));
    settle(cx);
    assert!(shows(cx, pill_id(&tag("tag59"))));
    assert!(shows(cx, pill_id(&link("link19"))));
    let summary = label(cx, SUMMARY_ID).expect("the summary line");
    assert!(
        summary.contains("#tag59") && summary.contains("link19"),
        "{summary}"
    );
}

#[gpui_kit::test]
fn the_more_chip_counts_the_facets_not_shown(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &crowded(), vec![]);
    settle(cx);
    let facets = crowded_facets();
    let drawn = drawn_pills(cx, &facets);
    assert_eq!(more_count(&view, cx), facets.len() - drawn.len());
    assert!(more_count(&view, cx) > 0);
    let name = format!("{} more facets (/)", more_count(&view, cx));
    assert_eq!(label(cx, MORE_ID), Some(name), "the name carries the count");
    let chip = bounds(cx, MORE_ID);
    assert!(
        f32::from(chip.size.height) >= 24.0,
        "a 24 px target: {chip:?}"
    );
    let last = drawn.last().unwrap().1;
    assert!(last.right() <= chip.left(), "the chip follows the pills");
    assert!(chip.right() <= bounds(cx, SECTIONS_ID).left());
    // The pills shown inline are the first of the line, in order.
    let order: Vec<Facet> = drawn.into_iter().map(|(f, _)| f).collect();
    assert_eq!(order[..], facets[..order.len()]);
}

#[gpui_kit::test]
fn the_more_chip_is_absent_when_everything_fits(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "#a #b [[x]]\n", vec![]);
    settle(cx);
    assert!(!shows(cx, MORE_ID));
    assert_eq!(more_count(&view, cx), 0);
    for facet in [tag("a"), tag("b"), link("x")] {
        assert!(shows(cx, pill_id(&facet)), "{facet:?}");
    }
    assert!(!shows(cx, SECTIONS_ID), "no headings: no Sections chip");
}

#[gpui_kit::test]
fn link_pills_read_as_wikilinks_and_tag_pills_as_hashtags(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, "#Run and [[Plan]]\n", vec![]);
    assert_eq!(label(cx, pill_id(&tag("run"))).as_deref(), Some("#Run"));
    assert_eq!(
        label(cx, pill_id(&link("plan"))).as_deref(),
        Some("[[plan]]")
    );
}

#[gpui_kit::test]
fn a_picked_heading_shows_as_a_pill_with_the_outline_icon(cx: &mut TestAppContext) {
    let text = "# Head\n\nsome #a\n";
    let (_view, cx, _events, _sub) = open(cx, text, vec![heading("head")]);
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
    assert!(!shows(cx, outline_mark_id(&tag("a"))), "only on headings");
}

/// The width the view measures for a pill is the width it draws: the fit
/// is computed from the same text, padding and border.
#[gpui_kit::test]
fn measured_widths_match_the_drawn_pills(cx: &mut TestAppContext) {
    let text = "# Head\n\n#alpha #alpha #alpha and [[a note]] and #b\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![heading("head"), tag("b")]);
    settle(cx);
    for facet in [tag("alpha"), link("a note"), heading("head"), tag("b")] {
        let drawn = f32::from(bounds(cx, pill_id(&facet)).size.width);
        let measured = view
            .read_with(cx, |view, _| view.measured_width(&facet))
            .expect("measured");
        assert!(
            measured >= drawn && measured <= drawn + 1.5,
            "{facet:?}: measured {measured}, drawn {drawn}"
        );
    }
}

#[gpui_kit::test]
fn the_list_groups_tags_then_links_and_filters(cx: &mut TestAppContext) {
    let text = "# Head\n\n#b #a #a [[x]]\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![]);
    focus_view(&view, cx);
    key(cx, "/");
    assert!(list_open(&view, cx));
    assert_eq!(
        label(cx, list_group_id(FacetKind::Tag)).as_deref(),
        Some("Tags")
    );
    assert_eq!(
        label(cx, list_group_id(FacetKind::NoteLink)).as_deref(),
        Some("Links")
    );
    let field = bounds(cx, MORE_QUERY_ID);
    let tags = bounds(cx, list_group_id(FacetKind::Tag));
    let links = bounds(cx, list_group_id(FacetKind::NoteLink));
    assert!(field.bottom() <= tags.top(), "the filter field first");
    assert!(tags.bottom() <= links.top(), "Tags, then Links");
    assert!(bounds(cx, more_item_id(&tag("a"))).top() >= tags.bottom());
    assert!(bounds(cx, more_item_id(&link("x"))).top() >= links.bottom());
    assert_eq!(
        view.read_with(cx, |view, cx| view.list_results(cx)),
        [tag("a"), tag("b"), link("x")],
        "one flat order: tags, then links"
    );
    assert!(
        !shows(cx, more_item_id(&heading("head"))),
        "headings stay out"
    );
    assert_eq!(
        label(cx, more_item_id(&link("x"))).as_deref(),
        Some("[[x]]"),
        "the list reads as the line does"
    );
    // A filter that matches no tag leaves the Tags group out.
    let input = view.read_with(cx, |view, _| view.more_input().unwrap());
    cx.update(|window, cx| input.update(cx, |i, cx| i.set_value("x", window, cx)));
    cx.run_until_parked();
    assert!(!shows(cx, list_group_id(FacetKind::Tag)));
    assert!(shows(cx, list_group_id(FacetKind::NoteLink)));
    assert_eq!(
        view.read_with(cx, |view, cx| view.list_results(cx)),
        [link("x")]
    );
    // The highlight runs over the flat list: Enter toggles the lit facet.
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [link("x")]);
}

#[gpui_kit::test]
fn slash_opens_the_same_list_as_the_more_chip(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, &crowded(), vec![]);
    settle(cx);
    click(cx, MORE_ID);
    assert!(list_open(&view, cx));
    assert!(field_focused(&view, cx));
    let from_chip = view.read_with(cx, |view, cx| view.list_results(cx));
    assert_eq!(from_chip.len(), 80, "every tag and link");
    let chip = bounds(cx, MORE_ID);
    let list = bounds(cx, MORE_LIST_ID);
    assert!(
        list.top() >= chip.bottom(),
        "under the chip: {list:?} {chip:?}"
    );
    key(cx, "escape");
    assert!(!list_open(&view, cx));
    assert!(has_focus(cx, MORE_ID), "the keyboard is back on the chip");
    key(cx, "/");
    assert!(list_open(&view, cx));
    assert_eq!(
        view.read_with(cx, |view, cx| view.list_results(cx)),
        from_chip,
        "the same list"
    );
    assert!(bounds(cx, MORE_LIST_ID).top() >= chip.bottom());
    key(cx, "escape");
    assert!(has_focus(cx, MORE_ID));
    assert_eq!(picks_changed(&events), 0);
}

/// With everything inline there is no `+N` chip: `/` still opens the list,
/// under the end of the line, and Escape gives the keyboard to the view.
#[gpui_kit::test]
fn slash_opens_the_list_without_a_more_chip(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "#a #b [[x]]\n", vec![]);
    settle(cx);
    assert!(!shows(cx, MORE_ID));
    focus_view(&view, cx);
    key(cx, "/");
    assert!(list_open(&view, cx));
    assert!(field_focused(&view, cx));
    assert_eq!(query(&view, cx), "", "the slash is not typed into it");
    let line = bounds(cx, LINE_ID);
    let list = bounds(cx, MORE_LIST_ID);
    assert!(
        list.top() >= line.top() + px(20.),
        "under the line: {list:?} {line:?}"
    );
    key(cx, "escape");
    assert!(!list_open(&view, cx));
    assert!(view_focused(&view, cx));
}

/// A note with headings but no tag or link: `/` opens the Sections list,
/// and Escape gives the keyboard to the `Sections N` chip.
#[gpui_kit::test]
fn slash_opens_the_sections_list_without_tags_or_links(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "# A\n\ntext\n\n# B\n\nmore\n", vec![]);
    focus_view(&view, cx);
    key(cx, "/");
    assert!(list_open(&view, cx));
    assert!(field_focused(&view, cx));
    assert!(shows(cx, more_item_id(&heading("a"))));
    assert!(shows(cx, more_item_id(&heading("b"))));
    assert!(bounds(cx, MORE_LIST_ID).top() >= bounds(cx, SECTIONS_ID).bottom());
    key(cx, "escape");
    assert!(!list_open(&view, cx));
    assert!(has_focus(cx, SECTIONS_ID));
}

/// Every tag and link picked: the list of tags and links would hold
/// nothing, so `/` opens the Sections list, and Escape gives the keyboard
/// to the `Sections N` chip.
#[gpui_kit::test]
fn slash_opens_the_sections_list_when_every_tag_and_link_is_picked(cx: &mut TestAppContext) {
    let text = "# A\n\ntext #a\n\n# B\n\nmore\n";
    let (view, cx, _events, _sub) = open(cx, text, vec![tag("a")]);
    focus_view(&view, cx);
    key(cx, "/");
    assert!(list_open(&view, cx));
    assert!(shows(cx, more_item_id(&heading("a"))), "the headings list");
    assert!(shows(cx, more_item_id(&heading("b"))));
    key(cx, "escape");
    assert!(!list_open(&view, cx));
    assert!(has_focus(cx, SECTIONS_ID));
}

/// Every tag and link picked in a note with no heading: nothing to list,
/// `/` opens nothing.
#[gpui_kit::test]
fn slash_opens_nothing_when_every_tag_and_link_is_picked_and_no_heading(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "text #a\n", vec![tag("a")]);
    focus_view(&view, cx);
    key(cx, "/");
    assert!(!list_open(&view, cx));
}

/// Picking the last unpicked tag inside the open list leaves it nothing to
/// show: it closes, and the keyboard goes to the view.
#[gpui_kit::test]
fn the_list_closes_when_its_last_facet_is_picked(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, "# A\n\ntext #a #b\n", vec![tag("a")]);
    focus_view(&view, cx);
    key(cx, "/");
    assert!(list_open(&view, cx));
    assert_eq!(
        view.read_with(cx, |view, cx| view.list_results(cx)),
        [tag("b")]
    );
    key(cx, "enter");
    assert_eq!(picks(&view, cx), [tag("a"), tag("b")]);
    assert!(!list_open(&view, cx), "nothing left to list");
    assert!(view_focused(&view, cx));
    assert!(!shows(cx, MORE_LIST_ID));
}

/// The summary line (and `Clear`) sits under the filter line, as in the
/// approved mock.
#[gpui_kit::test]
fn the_summary_line_sits_under_the_filter_line(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, &crowded(), vec![tag("tag00")]);
    settle(cx);
    let line = bounds(cx, LINE_ID);
    assert!(bounds(cx, SUMMARY_ID).top() >= line.bottom());
    assert!(bounds(cx, CLEAR_ID).top() >= line.bottom());
}

#[gpui_kit::test]
fn the_line_clips_instead_of_wrapping_in_a_narrow_pane(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &crowded(), vec![]);
    cx.simulate_resize(gpui_kit::size(px(320.), px(600.)));
    settle(cx);
    let line = bounds(cx, LINE_ID);
    assert!(f32::from(line.size.height) <= 26.0, "one row: {line:?}");
    let chip = bounds(cx, MORE_ID);
    let drawn = drawn_pills(cx, &crowded_facets());
    for (facet, b) in &drawn {
        assert!(
            b.right() <= chip.left(),
            "{facet:?} whole, before +N: {b:?}"
        );
        assert!(b.right() <= line.right(), "{facet:?} inside the line");
        assert_eq!(b.top(), chip.top(), "{facet:?} on the one row");
    }
    assert_eq!(more_count(&view, cx), 80 - drawn.len());
    assert!(bounds(cx, SECTIONS_ID).right() <= line.right());
}

/// A line whose pills all but fill the room: they all fit (no `+N`), and
/// the last pill is whole, not cut at the clip's edge by a gap the line
/// keeps for a chip it does not draw.
#[gpui_kit::test]
fn the_last_pill_is_whole_when_the_pills_just_fit(cx: &mut TestAppContext) {
    let text = "#alpha #beta #gamma #delta [[epsilon]] #zeta\n";
    let facets = [
        tag("alpha"),
        tag("beta"),
        link("epsilon"),
        tag("delta"),
        tag("gamma"),
        tag("zeta"),
    ];
    let (_view, cx, _events, _sub) = open(cx, text, vec![]);
    settle(cx);
    let window = cx.update(|window, _| window.viewport_size().width);
    let line = bounds(cx, LINE_ID);
    let drawn = drawn_pills(cx, &facets);
    assert_eq!(drawn.len(), facets.len(), "a wide window fits them all");
    let used = drawn.last().unwrap().1.right() - drawn[0].1.left();
    // The window that leaves the line 1.5 px more room than the pills use.
    let width = window - line.size.width + used + px(1.5);
    cx.simulate_resize(gpui_kit::size(width, px(600.)));
    settle(cx);
    assert_eq!(bounds(cx, LINE_ID).size.width, used + px(1.5), "the room");
    assert!(!shows(cx, MORE_ID), "everything fits: no +N");
    let clip = bounds(cx, LINE_PILLS_ID);
    let drawn = drawn_pills(cx, &facets);
    assert_eq!(drawn.len(), facets.len());
    let (last, b) = drawn.last().unwrap();
    assert!(
        b.right() <= clip.right(),
        "{last:?} cut by the clip: {b:?} past {clip:?}"
    );
}

#[gpui_kit::test]
fn no_leaders_rows_or_search_button_remain(cx: &mut TestAppContext) {
    let (_view, cx, _events, _sub) = open(cx, &crowded(), vec![heading("head")]);
    settle(cx);
    for id in [
        "facet-search",
        "facet-leader:tag",
        "facet-leader:link",
        "facet-leader:heading",
        "facet-more:tag",
        "facet-more:link",
        "facet-more:heading",
    ] {
        assert!(!shows(cx, id), "{id} is gone");
    }
    // Picked heading, tags and links share the one row.
    let head = bounds(cx, pill_id(&heading("head")));
    let first = drawn_pills(cx, &crowded_facets())[0].1;
    assert_eq!(head.top(), first.top());
}

#[gpui_kit::test]
fn tab_order_follows_the_line(cx: &mut TestAppContext) {
    let (view, cx, _events, _sub) = open(cx, &crowded(), vec![tag("tag00")]);
    settle(cx);
    focus_view(&view, cx);
    let pills: Vec<SharedString> = drawn_pills(cx, &crowded_facets())
        .into_iter()
        .map(|(f, _)| pill_id(&f))
        .collect();
    assert_eq!(pills[0], pill_id(&tag("tag00")));
    // The filter line first, then the summary line under it (`Clear`).
    let mut want: Vec<SharedString> = pills;
    want.push(MORE_ID.into());
    want.push(SECTIONS_ID.into());
    want.push(CLEAR_ID.into());
    for id in &want {
        key(cx, "tab");
        let ringed = view.read_with(cx, |view, _| view.ringed());
        assert!(has_focus(cx, id.clone()), "Tab reached {id}: {ringed:?}");
        assert_eq!(ringed, Some(id.clone()));
    }
    // Enter on `+N` opens the list; Escape brings the keyboard back to it.
    key(cx, "shift-tab");
    key(cx, "shift-tab");
    assert!(has_focus(cx, MORE_ID));
    key(cx, "enter");
    assert!(list_open(&view, cx));
    key(cx, "escape");
    assert!(has_focus(cx, MORE_ID));
}

#[gpui_kit::test]
fn the_line_does_not_change_the_piece_column_width(cx: &mut TestAppContext) {
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

#[gpui_kit::test]
fn sections_chip_still_opens_the_heading_list(cx: &mut TestAppContext) {
    let (view, cx, events, _sub) = open(cx, &crowded(), vec![]);
    settle(cx);
    assert_eq!(label(cx, SECTIONS_ID).as_deref(), Some("Sections 1"));
    click(cx, SECTIONS_ID);
    assert!(list_open(&view, cx));
    assert!(shows(cx, more_item_id(&heading("head"))));
    assert!(!shows(cx, more_item_id(&tag("tag00"))), "headings only");
    assert!(!shows(cx, list_group_id(FacetKind::Tag)), "no group labels");
    let chip = bounds(cx, SECTIONS_ID);
    assert!(bounds(cx, MORE_LIST_ID).top() >= chip.bottom());
    click(cx, more_item_id(&heading("head")));
    assert_eq!(picks(&view, cx), [heading("head")]);
    assert_eq!(picks_changed(&events), 1);
    assert!(shows(cx, pill_id(&heading("head"))), "now in the line");
    key(cx, "escape");
    assert!(
        has_focus(cx, SECTIONS_ID),
        "the keyboard is back on the chip"
    );
}

/// A lone group floats its corner buttons over the top-right of the view:
/// no pill or chip of a crowded line sits under them, and a click on `+N`
/// opens the list, not the note menu.
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
    let mut controls: Vec<SharedString> = vec![MORE_ID.into(), SECTIONS_ID.into(), CLEAR_ID.into()];
    for facet in std::iter::once(tag("tag00")).chain(crowded_facets()) {
        let drawn = cx
            .update_window(s.handle, |_, window, cx| {
                window.render_frame(cx);
                window
                    .try_find(pill_id(&facet))
                    .is_some_and(|e| e.visible())
            })
            .unwrap();
        if drawn {
            controls.push(pill_id(&facet));
        }
    }
    assert!(controls.len() > 4, "some pills are drawn");
    for id in &controls {
        let b = bounds_of(&s, id.clone(), cx);
        assert!(
            !b.intersects(&corner),
            "{id} {b:?} under the corner {corner:?}"
        );
    }
    click_in(&s, MORE_ID, cx);
    assert!(
        cx.read(|cx| view.read(cx).more_input().is_some()),
        "the list opened"
    );
    assert!(
        !cx.read(|cx| s.workspace.read(cx).note_menu.is_some()),
        "the note menu did not"
    );
}
