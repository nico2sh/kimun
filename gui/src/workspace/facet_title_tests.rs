//! UI tests for the view title row of the **Facet view**. Tabs do not
//! always show, so the view says what it shows: a collection names its
//! query after a muted `Collection`, a single-note view names its note. The
//! title sits outside the scrolling column, one step above the datelines.
#![cfg(test)]

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Role, TestAppContext, VisualTestContext};
use kimun_core::nfs::VaultPath;

use super::facet::{
    COLLECTION_LINE_ID, COLUMN_ID, DATELINE_LEVEL, FacetView, MemberInit, VIEW_TITLE_ID,
    VIEW_TITLE_KIND_ID, VIEW_TITLE_LEVEL, VIEW_TITLE_TEXT_ID, dateline_title_id,
    dateline_title_size, view_title_size,
};
use super::facet_tests::{bounds, label, open, open_with, settle, shows};

/// A collection of one note, `Alpha`, for the query `query`.
fn collection<'a>(cx: &'a mut TestAppContext, query: &str) -> &'a mut VisualTestContext {
    let query = query.to_string();
    let members = vec![MemberInit {
        path: VaultPath::new("/notes/a.md"),
        title: "Alpha".into(),
        text: "a\n".into(),
    }];
    let (_view, cx, _events, _sub) = open_with(cx, move |cx| {
        FacetView::new_collection(query, members, 1, vec![], cx)
    });
    cx
}

fn role(cx: &mut VisualTestContext, id: &'static str) -> Option<Role> {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window.find(id).role()
    })
}

#[gpui_kit::test]
fn a_collection_title_has_the_word_collection_above_the_query(cx: &mut TestAppContext) {
    let cx = collection(cx, "#q");
    assert_eq!(label(cx, VIEW_TITLE_ID).as_deref(), Some("Collection: #q"));
    assert_eq!(role(cx, VIEW_TITLE_ID), Some(Role::Heading));
    let (kind, text) = (
        bounds(cx, VIEW_TITLE_KIND_ID),
        bounds(cx, VIEW_TITLE_TEXT_ID),
    );
    assert!(kind.bottom() <= text.top(), "the word is above the query");
    assert_eq!(kind.left(), text.left(), "both start at the left edge");
    assert!(shows(cx, VIEW_TITLE_KIND_ID));
    let title = bounds(cx, VIEW_TITLE_ID);
    assert!(title.bottom() <= bounds(cx, COLLECTION_LINE_ID).top());
    assert!(title.bottom() <= bounds(cx, COLUMN_ID).top());
}

/// A query too long for the view stays on one line, cut with an ellipsis.
/// The full query is the title's name.
#[gpui_kit::test]
fn a_long_query_stays_on_one_line(cx: &mut TestAppContext) {
    let long = "#tag ".repeat(200);
    let cx = collection(cx, long.trim_end());
    let (kind, text) = (
        bounds(cx, VIEW_TITLE_KIND_ID),
        bounds(cx, VIEW_TITLE_TEXT_ID),
    );
    assert!(text.size.height < kind.size.height * 2.5, "one line");
    assert!(text.right() <= bounds(cx, VIEW_TITLE_ID).right() + gpui_kit::px(0.5));
    let want = format!("Collection: {}", long.trim_end());
    assert_eq!(label(cx, VIEW_TITLE_ID), Some(want));
}

#[gpui_kit::test]
fn a_single_note_view_titles_itself_with_the_note_title(cx: &mut TestAppContext) {
    let (_view, cx, _e, _s) = open(cx, "# Groceries\n\nmilk\n", vec![]);
    assert!(shows(cx, VIEW_TITLE_ID));
    assert_eq!(label(cx, VIEW_TITLE_ID).as_deref(), Some("Note: Groceries"));
    assert!(shows(cx, VIEW_TITLE_KIND_ID));
    assert!(bounds(cx, VIEW_TITLE_KIND_ID).bottom() <= bounds(cx, VIEW_TITLE_TEXT_ID).top());
    assert_eq!(role(cx, VIEW_TITLE_ID), Some(Role::Heading));
}

/// The title follows new text and a rename. A note with no title line
/// falls back to its name, as a dateline does.
#[gpui_kit::test]
fn a_single_note_title_follows_new_text_and_a_rename(cx: &mut TestAppContext) {
    let (view, cx, _e, _s) = open(cx, "# Groceries\n\nmilk\n", vec![]);
    view.update(cx, |v, cx| v.set_text("# Shopping\n\nmilk\n".into(), cx));
    settle(cx);
    assert_eq!(label(cx, VIEW_TITLE_ID).as_deref(), Some("Note: Shopping"));
    view.update(cx, |v, cx| {
        v.set_source(VaultPath::new("/notes/errands.md"));
        v.set_text(String::new(), cx);
    });
    settle(cx);
    assert_eq!(label(cx, VIEW_TITLE_ID).as_deref(), Some("Note: errands"));
}

/// The title is one type step (1.25) above the datelines, from one shared
/// size, so it stays above them at any editor size.
#[gpui_kit::test]
fn the_title_is_one_step_above_the_datelines(cx: &mut TestAppContext) {
    let cx = collection(cx, "#q");
    for size in [None, Some(gpui_kit::px(18.))] {
        cx.update(|_, cx| {
            if size.is_some() {
                cx.set_global(crate::editor_font::EditorFontSetting { family: None, size });
            }
        });
        let (title, dateline) = cx.update(|_, cx| (view_title_size(cx), dateline_title_size(cx)));
        assert_eq!(title, dateline * 1.25, "{size:?}");
        cx.update(|window, _| window.refresh());
        settle(cx);
        let drawn = bounds(cx, VIEW_TITLE_TEXT_ID).size.height;
        let below = bounds(cx, dateline_title_id(0)).size.height;
        assert!(drawn > below, "{size:?}: {drawn:?} over {below:?}");
    }
}

/// The title is the outline's first level, each dateline the second.
#[test]
fn the_title_is_heading_level_one_and_datelines_level_two() {
    assert_eq!((VIEW_TITLE_LEVEL, DATELINE_LEVEL), (1, 2));
}
