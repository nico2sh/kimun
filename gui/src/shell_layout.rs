//! The shell's toolkit-free half (ADR-0050): which views the rail offers,
//! what they are called, the sizing floors in rems, and the round trip
//! between the logic [`PanelLayout`] and the `[gui]` settings table. The
//! workspace view renders and routes; everything here is plain data and
//! tested without a window.

use kimun_notes::logic::layout::{Panel, PanelLayout, Side, WidthLimits};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::gui::{GuiSettings, ViewEntry};

/// The rail's width, in rems: one icon cell plus its inset.
pub const RAIL_REMS: f32 = 3.0;
/// The narrowest a drawer may be dragged, in rems: room for a short label
/// and its icon.
pub const DRAWER_MIN_REMS: f32 = 10.0;
/// The width the editor always keeps, in rems: a readable line of text.
pub const EDITOR_MIN_REMS: f32 = 20.0;

/// The views the rail offers, top to bottom; the last (settings) is pinned
/// to the bottom, as in the TUI. SEM and ASK need a kimün server the GUI
/// does not talk to yet, so it leaves them out, as the TUI rail does
/// without one.
pub const RAIL_VIEWS: [DrawerView; 6] = [
    DrawerView::Files,
    DrawerView::Find,
    DrawerView::Tags,
    DrawerView::Links,
    DrawerView::Outline,
    DrawerView::Config,
];

/// The side of the window the rail sits on. Moves from the rail are
/// spatial ([`rail_move_target`]): they mirror if the rail ever moves.
pub const RAIL_SIDE: Side = Side::Left;

/// The drawer a `MoveViewTo…Drawer` chord pressed on the rail moves the
/// cursor's view to: the one toward `to`, which is the drawer beside the
/// rail (the rail is at the window's edge, so the nearest drawer that
/// way). `None` toward the window's edge: nothing is there.
pub fn rail_move_target(to: Side) -> Option<Side> {
    (to != RAIL_SIDE).then_some(RAIL_SIDE)
}

/// The hint an empty drawer or an empty slot shows: how a view gets there.
/// `right` and `left` are the move chords as the keymap binds them (`None`
/// when unbound, and then left out).
pub fn empty_slot_hint(right: Option<&str>, left: Option<&str>) -> String {
    let chords: Vec<&str> = [right, left].into_iter().flatten().collect();
    if chords.is_empty() {
        "No view here. Pick one from the rail.".to_string()
    } else {
        format!(
            "No view here. Move one with {}, or pick one from the rail.",
            chords.join(" / ")
        )
    }
}

/// Whether the rail offers `view`.
pub fn is_on_rail(view: DrawerView) -> bool {
    RAIL_VIEWS.contains(&view)
}

/// What the interface calls `view`: the drawer's title and the rail cell's
/// accessible name and tooltip.
pub fn view_name(view: DrawerView) -> &'static str {
    match view {
        DrawerView::Files => "Files",
        DrawerView::Find => "Find",
        DrawerView::Semantic => "Semantic search",
        DrawerView::Ask => "Ask",
        DrawerView::Tags => "Tags",
        DrawerView::Links => "Links",
        DrawerView::Outline => "Outline",
        DrawerView::Config => "Settings",
    }
}

/// Both drawers, for the per-side round trip.
const SIDES: [Side; 2] = [Side::Left, Side::Right];

/// A width limit that clamps nothing: a restored width is the persisted one,
/// and the window clamps it only when it is drawn ([`shown_width`]).
const UNCLAMPED: WidthLimits = WidthLimits {
    drawer_min: 0.0,
    editor_min: 0.0,
    available: f64::INFINITY,
};

/// One drawer's placements as the legacy lists describe them, over the ids
/// in `list` (top position first): each id with its slot and whether it is
/// shown. Without `drawer_<side>_bottom` every id but the first sits at the
/// bottom; without `drawer_<side>_shown` the first id at each slot is shown
/// — so a list written before placements reads as it always did (its first
/// two views shown, top and bottom).
fn placements(gui: &GuiSettings, side: Side, list: &[String]) -> Vec<(usize, bool)> {
    let bottom = gui.drawer_bottom(side);
    let shown = gui.drawer_shown(side);
    let slots: Vec<usize> = list
        .iter()
        .enumerate()
        .map(|(i, id)| match &bottom {
            Some(bottom) => usize::from(bottom.contains(id)),
            None => usize::from(i > 0),
        })
        .collect();
    list.iter()
        .enumerate()
        .map(|(i, id)| {
            let shown = match &shown {
                Some(shown) => shown.contains(id),
                None => !slots[..i].contains(&slots[i]),
            };
            (slots[i], shown)
        })
        .collect()
}

/// A rail view's id, as `[gui]` names it; `None` for unknown ids and views
/// the rail does not offer.
fn ours(id: &str) -> Option<DrawerView> {
    DrawerView::from_id(id.trim()).filter(|v| is_on_rail(*v))
}

/// The layout the `[gui]` table describes: each view placed at its side and
/// slot, shown or hidden, each drawer at its persisted width and split
/// ratio, shown or hidden; the editor has the focus. With `[gui.views]` a
/// drawer saved visible stays visible even when it shows nothing (it was
/// toggled open empty, [`PanelLayout::open_drawer`]); empty slots are
/// session-only and never come back.
///
/// Lenient, never fatal: unknown ids and views the rail does not offer are
/// skipped, a second view shown at one slot stays hidden, and a table that
/// places nothing at all gives the default (FILES on the left). Ratios are
/// clamped by the model. Within a slot the shown view comes first, then the
/// others in id order.
///
/// A file without `[gui.views]` is read from the legacy keys
/// ([`restore_legacy`]).
pub fn restore(gui: &GuiSettings) -> PanelLayout<DrawerView> {
    let mut layout = PanelLayout::empty(gui.drawer_width(Side::Left));
    let entries = gui.view_entries().is_some();
    match gui.view_entries() {
        Some(entries) => {
            let mut placed: Vec<(DrawerView, ViewEntry)> = entries
                .iter()
                .filter_map(|(id, entry)| ours(id).map(|view| (view, *entry)))
                .collect();
            placed.sort_by_key(|(_, e)| (e.slot, !e.shown));
            for side in SIDES {
                for (view, entry) in placed.iter().filter(|(_, e)| e.side == side) {
                    if !layout.place(side, *view, entry.slot, entry.shown) {
                        layout.place(side, *view, entry.slot, false);
                    }
                }
                layout.set_split_ratio(side, gui.split_ratio(side));
            }
        }
        None => restore_legacy(gui, &mut layout),
    }
    if SIDES
        .iter()
        .all(|side| layout.placed_views(*side).is_empty())
    {
        layout = PanelLayout::new(DrawerView::Files, gui.drawer_width(Side::Left));
    }
    for side in SIDES {
        layout.set_drawer_width(side, gui.drawer_width(side), UNCLAMPED);
        if gui.drawer_visible(side) {
            if entries {
                layout.open_drawer(side);
            } else {
                layout.show(Panel::Drawer(side));
            }
        }
    }
    layout.focus(Panel::Editor);
    layout
}

/// Place the views of a file from before `[gui.views]` into `layout`, as
/// those builds read it.
///
/// The `drawer_<side>` lists with their `_bottom` and `_shown` subsets
/// ([`placements`]); a view listed on both sides stays on the left. Older
/// still: the lists mean the shown views, a `drawer_<side>_hidden` entry
/// places its view hidden at its old slot (when the drawer still holds its
/// `with` view alone), and views in `drawer_right_home` are placed hidden on
/// top of the right drawer.
fn restore_legacy(gui: &GuiSettings, layout: &mut PanelLayout<DrawerView>) {
    // Each side's listed views; a view listed on both sides stays left.
    let mut lists: Vec<(Side, Vec<DrawerView>)> = Vec::new();
    for side in SIDES {
        let mut views: Vec<DrawerView> = Vec::new();
        for view in gui.drawer_views(side).iter().filter_map(|id| ours(id)) {
            if !lists.iter().any(|(_, l)| l.contains(&view)) && !views.contains(&view) {
                views.push(view);
            }
        }
        lists.push((side, views));
    }
    let listed = |view: &DrawerView| lists.iter().any(|(_, l)| l.contains(view));
    for (side, views) in &lists {
        let side = *side;
        // An earlier build's hidden-slot memory: the drawer lists its `with`
        // view alone and the hidden view is listed nowhere.
        let fresh = gui.drawer_bottom(side).is_none() && gui.drawer_shown(side).is_none();
        if let Some(hidden) = gui.legacy_hidden_slot(side)
            && let (true, Some(view), Some(with)) = (fresh, ours(&hidden.view), ours(&hidden.with))
            && views[..] == [with]
            && !listed(&view)
        {
            layout.place(side, with, 1 - hidden.slot, true);
            layout.place(side, view, hidden.slot, false);
            layout.set_split_ratio(side, hidden.split);
            continue;
        }
        let ids: Vec<String> = views.iter().map(|v| v.id().to_string()).collect();
        for (view, (slot, shown)) in views.iter().copied().zip(placements(gui, side, &ids)) {
            if !layout.place(side, view, slot, shown) {
                layout.place(side, view, slot, false);
            }
        }
        layout.set_split_ratio(side, gui.split_ratio(side));
    }
    for view in gui
        .legacy_right_home_views()
        .iter()
        .filter_map(|id| ours(id))
    {
        if layout.placement(&view).is_none() {
            layout.place(Side::Right, view, 0, false);
        }
    }
}

/// Write both drawers into `gui`: one `[gui.views]` entry per placed view
/// (side, slot, shown), and each drawer's width, visibility and split
/// ratio. Entries this shell does not own (unknown ids, a newer version's,
/// views the rail does not offer) are kept as written, so a newer layout
/// survives a round trip here; a rail view placed nowhere has no entry.
///
/// A file still in the legacy shape is migrated: its unknown ids become
/// entries at the side, slot and shown state the lists gave them, and the
/// legacy keys are dropped (their content was turned into placements when
/// the table was read).
pub fn record(layout: &PanelLayout<DrawerView>, gui: &mut GuiSettings) {
    let mut entries: Vec<(String, ViewEntry)> = Vec::new();
    for side in SIDES {
        for view in layout.placed_views(side) {
            if let Some((_, slot)) = layout.placement(&view) {
                let shown = layout.view_location(&view).is_some();
                entries.push((view.id().to_string(), ViewEntry { side, slot, shown }));
            }
        }
    }
    let foreign = |id: &str| ours(id).is_none();
    if gui.view_entries().is_none() {
        for side in SIDES {
            let raw = gui.drawer_views(side);
            for (id, (slot, shown)) in raw.iter().zip(placements(gui, side, &raw)) {
                if foreign(id) && !entries.iter().any(|(e, _)| e == id) {
                    entries.push((id.clone(), ViewEntry { side, slot, shown }));
                }
            }
        }
    }
    gui.set_view_entries(&entries, foreign);
    for side in SIDES {
        gui.set_drawer_width(side, layout.drawer_width(side));
        gui.set_drawer_visible(side, layout.is_visible(Panel::Drawer(side)));
        if let Some(ratio) = layout.split_ratio(side) {
            gui.set_split_ratio(side, ratio);
        }
    }
    gui.remove_legacy_keys();
}

/// The model's sizing floors for a window `window_width` px wide at `rem`
/// px per rem.
pub fn limits(window_width: f64, rem: f64) -> WidthLimits {
    WidthLimits {
        drawer_min: f64::from(DRAWER_MIN_REMS) * rem,
        editor_min: f64::from(EDITOR_MIN_REMS) * rem,
        available: (window_width - f64::from(RAIL_REMS) * rem).max(0.0),
    }
}

/// The width `side`'s drawer is drawn at: its preferred (persisted, dragged)
/// width, clamped to the current window by the model's own rule, without
/// changing the preference — a window made narrow and wide again gets the
/// drawer back.
pub fn shown_width(layout: &PanelLayout<DrawerView>, side: Side, limits: WidthLimits) -> f64 {
    layout.clamped_drawer_width(side, limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_notes::settings::AppSettings;
    use kimun_notes::settings::gui::DEFAULT_DRAWER_WIDTH;

    fn gui(toml: &str) -> GuiSettings {
        toml::from_str::<AppSettings>(toml).unwrap().gui
    }

    /// The `[gui.views]` entries as (id, side, slot, shown), in id order.
    fn entries(gui: &GuiSettings) -> Vec<(String, Side, usize, bool)> {
        gui.view_entries()
            .expect("[gui.views] is written")
            .into_iter()
            .map(|(id, e)| (id, e.side, e.slot, e.shown))
            .collect()
    }

    fn e(id: &str, side: Side, slot: usize, shown: bool) -> (String, Side, usize, bool) {
        (id.to_string(), side, slot, shown)
    }

    /// No legacy key is left in `gui`.
    fn assert_no_legacy_keys(gui: &GuiSettings) {
        let written = toml::to_string(gui).unwrap();
        for key in [
            "drawer_left =",
            "drawer_right =",
            "_bottom",
            "_shown",
            "_hidden",
            "drawer_right_home",
        ] {
            assert!(!written.contains(key), "{key} in {written}");
        }
    }

    #[test]
    fn a_missing_table_is_the_default_shell() {
        let layout = restore(&GuiSettings::default());
        assert_eq!(layout.slot_views(Side::Left), [DrawerView::Files]);
        assert!(layout.is_visible(Panel::Drawer(Side::Left)));
        assert!(!layout.is_visible(Panel::Drawer(Side::Right)));
        assert_eq!(layout.drawer_width(Side::Left), DEFAULT_DRAWER_WIDTH);
        assert_eq!(layout.focused(), Panel::Editor);
    }

    #[test]
    fn restore_reads_view_width_and_visibility() {
        let layout = restore(&gui(
            "[gui]\ndrawer_left = [\"tags\"]\ndrawer_left_width = 300\ndrawer_left_visible = false\n",
        ));
        assert_eq!(layout.slot_views(Side::Left), [DrawerView::Tags]);
        assert_eq!(layout.drawer_width(Side::Left), 300.0);
        assert!(!layout.is_visible(Panel::Drawer(Side::Left)));
        assert_eq!(layout.focused(), Panel::Editor);
    }

    #[test]
    fn unknown_and_off_rail_ids_fall_through() {
        let layout = restore(&gui(
            "[gui]\ndrawer_left = [\"backlinks\", \"ask\", \"outline\"]\n",
        ));
        assert_eq!(layout.slot_views(Side::Left), [DrawerView::Outline]);
        let layout = restore(&gui("[gui]\ndrawer_left = [\"nope\"]\n"));
        assert_eq!(layout.slot_views(Side::Left), [DrawerView::Files]);
    }

    #[test]
    fn record_round_trips_and_keeps_later_slots() {
        let mut settings = gui("[gui]\ndrawer_left = [\"files\", \"backlinks\"]\nkept = 1\n");
        let mut layout = restore(&settings);
        // Find takes Files' top slot; Files stays placed there, hidden.
        layout.place_view(DrawerView::Find);
        layout.hide(Panel::Drawer(Side::Left));
        layout.set_drawer_width(Side::Left, 333.0, limits(2000.0, 16.0));
        record(&layout, &mut settings);
        // The unknown id keeps the place the lists gave it: bottom, shown.
        assert_eq!(
            entries(&settings),
            [
                e("backlinks", Side::Left, 1, true),
                e("files", Side::Left, 0, false),
                e("find", Side::Left, 0, true),
            ]
        );
        assert_no_legacy_keys(&settings);
        let back = restore(&settings);
        assert_eq!(back.slot_views(Side::Left), [DrawerView::Find]);
        assert_eq!(back.drawer_width(Side::Left), 333.0);
        assert!(!back.is_visible(Panel::Drawer(Side::Left)));
        // Keys the shell does not own survive a write.
        let written = toml::to_string(&settings).unwrap();
        assert!(written.contains("kept = 1"), "{written}");
    }

    #[test]
    fn restore_fills_both_drawers_and_their_slots() {
        let layout = restore(&gui(
            "[gui]\ndrawer_left = [\"files\", \"find\"]\ndrawer_right = [\"outline\", \"tags\"]\n\
             drawer_left_split = 0.3\ndrawer_right_width = 300\ndrawer_right_visible = true\n",
        ));
        assert_eq!(
            layout.slot_views(Side::Left),
            [DrawerView::Files, DrawerView::Find]
        );
        assert_eq!(
            layout.slot_views(Side::Right),
            [DrawerView::Outline, DrawerView::Tags]
        );
        assert_eq!(layout.split_ratio(Side::Left), Some(0.3));
        assert_eq!(layout.split_ratio(Side::Right), Some(0.5));
        assert_eq!(layout.drawer_width(Side::Left), DEFAULT_DRAWER_WIDTH);
        assert_eq!(layout.drawer_width(Side::Right), 300.0);
        assert!(layout.is_visible(Panel::Drawer(Side::Left)));
        assert!(layout.is_visible(Panel::Drawer(Side::Right)));
        assert_eq!(layout.focused(), Panel::Editor);
        // Placed at the bottom of the right drawer.
        assert_eq!(layout.placement(&DrawerView::Tags), Some((Side::Right, 1)));
    }

    #[test]
    fn restore_is_lenient_about_bad_placements() {
        // A third view is over the cap; a view on both sides stays left; a
        // stale ratio is clamped by the model; a hidden right drawer stays
        // hidden.
        let layout = restore(&gui(
            "[gui]\ndrawer_left = [\"files\", \"tags\", \"find\"]\n\
             drawer_right = [\"tags\", \"links\"]\ndrawer_left_split = 0.01\n",
        ));
        assert_eq!(
            layout.slot_views(Side::Left),
            [DrawerView::Files, DrawerView::Tags]
        );
        assert_eq!(layout.slot_views(Side::Right), [DrawerView::Links]);
        assert_eq!(layout.split_ratio(Side::Left), Some(0.1));
        assert!(!layout.is_visible(Panel::Drawer(Side::Right)));
        // Everything moved right: the left drawer is empty and hidden.
        let layout = restore(&gui(
            "[gui]\ndrawer_left = []\ndrawer_right = [\"files\"]\ndrawer_right_visible = true\n",
        ));
        assert!(layout.slot_views(Side::Left).is_empty());
        assert!(!layout.is_visible(Panel::Drawer(Side::Left)));
        assert_eq!(layout.slot_views(Side::Right), [DrawerView::Files]);
        assert!(layout.is_visible(Panel::Drawer(Side::Right)));
        assert_eq!(layout.focused(), Panel::Editor);
        // Nothing placeable anywhere: the default shell.
        let layout = restore(&gui("[gui]\ndrawer_left = []\ndrawer_right = [\"ask\"]\n"));
        assert_eq!(layout.slot_views(Side::Left), [DrawerView::Files]);
        assert!(layout.slot_views(Side::Right).is_empty());
    }

    #[test]
    fn record_writes_both_drawers_and_the_split() {
        let mut settings = gui("[gui]\ndrawer_right = [\"backlinks\"]\n");
        let mut layout = restore(&settings);
        layout.split_slot(Side::Left, DrawerView::Find).unwrap();
        layout.set_split_ratio(Side::Left, 0.25);
        layout.move_view(DrawerView::Outline, Side::Right);
        layout.set_drawer_width(Side::Right, 222.0, limits(2000.0, 16.0));
        record(&layout, &mut settings);
        // The unknown id a newer version placed is kept.
        assert_eq!(
            entries(&settings),
            [
                e("backlinks", Side::Right, 0, true),
                e("files", Side::Left, 0, true),
                e("find", Side::Left, 1, true),
                e("outline", Side::Right, 0, true),
            ]
        );
        assert_eq!(settings.split_ratio(Side::Left), 0.25);
        assert_eq!(settings.drawer_width(Side::Right), 222.0);
        assert!(settings.drawer_visible(Side::Right));
        let back = restore(&settings);
        assert_eq!(back.slot_views(Side::Left), layout.slot_views(Side::Left));
        assert_eq!(back.slot_views(Side::Right), [DrawerView::Outline]);
        assert_eq!(back.split_ratio(Side::Left), Some(0.25));
        assert_eq!(back.drawer_width(Side::Right), 222.0);

        // Moving a view back drops it from the side it left.
        layout.move_view(DrawerView::Outline, Side::Left);
        record(&layout, &mut settings);
        assert_eq!(
            entries(&settings),
            [
                e("backlinks", Side::Right, 0, true),
                e("files", Side::Left, 0, false),
                e("find", Side::Left, 1, true),
                e("outline", Side::Left, 0, true),
            ]
        );
        assert!(!settings.drawer_visible(Side::Right));
    }

    #[test]
    fn placements_round_trip_with_hidden_and_shared_positions() {
        let mut settings = gui("[gui]\ndrawer_left = [\"files\", \"find\"]\n\
             drawer_right = [\"outline\", \"tags\"]\ndrawer_right_visible = true\n");
        let mut layout = restore(&settings);
        record(&layout, &mut settings);
        assert_eq!(
            entries(&settings),
            [
                e("files", Side::Left, 0, true),
                e("find", Side::Left, 1, true),
                e("outline", Side::Right, 0, true),
                e("tags", Side::Right, 1, true),
            ]
        );

        layout.set_split_ratio(Side::Left, 0.3);
        layout.hide_view(&DrawerView::Files);
        layout.hide_view(&DrawerView::Tags);
        // Links shares Outline's top position; showing it hides Outline.
        layout.focus_region(kimun_notes::logic::layout::Region::Slot {
            side: Side::Right,
            index: 0,
        });
        layout.move_view(DrawerView::Links, Side::Right);
        assert_eq!(layout.slot_views(Side::Right), [DrawerView::Links]);
        record(&layout, &mut settings);
        assert_eq!(
            entries(&settings),
            [
                e("files", Side::Left, 0, false),
                e("find", Side::Left, 1, true),
                e("links", Side::Right, 0, true),
                e("outline", Side::Right, 0, false),
                e("tags", Side::Right, 1, false),
            ]
        );
        assert_eq!(settings.split_ratio(Side::Left), 0.3);

        let mut back = restore(&settings);
        for side in SIDES {
            assert_eq!(back.slot_views(side), layout.slot_views(side));
            for view in layout.placed_views(side) {
                assert_eq!(back.placement(&view), layout.placement(&view));
            }
        }
        // A second save writes the same entries.
        let mut again = settings.clone();
        record(&back, &mut again);
        assert_eq!(again, settings);
        // The hidden top view comes back above the bottom one, at the ratio.
        assert_eq!(back.open_view(DrawerView::Files), (Side::Left, 0));
        assert_eq!(
            back.slot_views(Side::Left),
            [DrawerView::Files, DrawerView::Find]
        );
        assert_eq!(back.split_ratio(Side::Left), Some(0.3));
        // Outline and Links toggle each other on top.
        assert_eq!(back.open_view(DrawerView::Outline), (Side::Right, 0));
        assert_eq!(back.slot_views(Side::Right), [DrawerView::Outline]);
        assert_eq!(back.open_view(DrawerView::Tags), (Side::Right, 1));
        assert_eq!(
            back.slot_views(Side::Right),
            [DrawerView::Outline, DrawerView::Tags]
        );
    }

    #[test]
    fn a_lone_bottom_view_and_bad_keys_restore_safely() {
        // Only the bottom position shows a view; a hidden top view returns
        // above it.
        let mut layout = restore(&gui("[gui]\ndrawer_left = [\"files\", \"find\"]\n\
             drawer_left_shown = [\"find\"]\n"));
        assert_eq!(layout.slot_views(Side::Left), [DrawerView::Find]);
        assert_eq!(layout.placement(&DrawerView::Find), Some((Side::Left, 1)));
        layout.open_view(DrawerView::Files);
        assert_eq!(
            layout.slot_views(Side::Left),
            [DrawerView::Files, DrawerView::Find]
        );
        // Two shown at one position: the first wins. Unknown ids in the
        // subsets are ignored; all hidden keeps the drawer hidden.
        let layout = restore(&gui(
            "[gui]\ndrawer_left = [\"files\", \"tags\", \"find\"]\n\
             drawer_left_bottom = [\"nope\"]\ndrawer_left_shown = [\"tags\", \"files\", \"nope\"]\n",
        ));
        assert_eq!(layout.slot_views(Side::Left), [DrawerView::Files]);
        assert_eq!(layout.placed_views(Side::Left).len(), 3);
        let layout = restore(&gui(
            "[gui]\ndrawer_left = [\"files\"]\ndrawer_left_shown = []\n",
        ));
        assert!(layout.slot_views(Side::Left).is_empty());
        assert!(!layout.is_visible(Panel::Drawer(Side::Left)));
        assert_eq!(layout.placement(&DrawerView::Files), Some((Side::Left, 0)));
    }

    #[test]
    fn old_format_files_load_into_placements() {
        // An earlier build's hidden-slot memory and right homes.
        let mut settings = gui("[gui]\ndrawer_left = [\"find\"]\n\
             drawer_left_hidden = { view = \"files\", with = \"find\", slot = 0, split = 0.3 }\n\
             drawer_right = [\"outline\"]\ndrawer_right_visible = true\n\
             drawer_right_hidden = { view = \"tags\", with = \"outline\", slot = 1 }\n\
             drawer_right_home = [\"links\", \"backlinks\", \"files\"]\n");
        let mut layout = restore(&settings);
        assert_eq!(layout.slot_views(Side::Left), [DrawerView::Find]);
        assert_eq!(layout.placement(&DrawerView::Files), Some((Side::Left, 0)));
        assert_eq!(layout.placement(&DrawerView::Find), Some((Side::Left, 1)));
        assert_eq!(layout.split_ratio(Side::Left), Some(0.3));
        assert_eq!(layout.placement(&DrawerView::Tags), Some((Side::Right, 1)));
        assert_eq!(layout.placement(&DrawerView::Links), Some((Side::Right, 0)));
        assert_eq!(layout.slot_views(Side::Right), [DrawerView::Outline]);
        assert_eq!(layout.open_view(DrawerView::Files), (Side::Left, 0));
        assert_eq!(
            layout.slot_views(Side::Left),
            [DrawerView::Files, DrawerView::Find]
        );
        assert_eq!(layout.open_view(DrawerView::Tags), (Side::Right, 1));
        // The next save writes view entries and drops the old keys; the
        // unknown home id is not kept (it was never placed).
        record(&layout, &mut settings);
        assert_no_legacy_keys(&settings);
        assert_eq!(
            entries(&settings),
            [
                e("files", Side::Left, 0, true),
                e("find", Side::Left, 1, true),
                e("links", Side::Right, 0, false),
                e("outline", Side::Right, 0, true),
                e("tags", Side::Right, 1, true),
            ]
        );
        let back = restore(&settings);
        for view in layout.placed_views(Side::Right) {
            assert_eq!(back.placement(&view), layout.placement(&view));
        }

        // Stale entries change nothing: the drawer no longer holds `with`
        // alone, the hidden view is placed, unknown or off-rail ids.
        for (drawers, view, with) in [
            (r#"drawer_left = ["files", "find"]"#, "tags", "files"),
            (r#"drawer_left = ["find"]"#, "tags", "files"),
            (r#"drawer_right = ["tags"]"#, "tags", "files"),
            ("", "nope", "files"),
            ("", "ask", "files"),
            ("", "tags", "nope"),
        ] {
            let table = format!(
                "[gui]\n{drawers}\ndrawer_left_hidden = {{ view = {view:?}, with = {with:?}, slot = 0 }}\n"
            );
            let layout = restore(&gui(&table));
            assert_ne!(
                layout.placement(&DrawerView::Tags),
                Some((Side::Left, 0)),
                "{table}"
            );
        }
        // A fitting entry in a hidden drawer: the view shows it split.
        let mut layout = restore(&gui(
            "[gui]\ndrawer_left_visible = false\ndrawer_left_hidden = { view = \"tags\", with = \"files\", slot = 0 }\n",
        ));
        assert_eq!(layout.open_view(DrawerView::Tags), (Side::Left, 0));
        assert_eq!(
            layout.slot_views(Side::Left),
            [DrawerView::Tags, DrawerView::Files]
        );
        // A home for a placed, unknown or off-rail view changes nothing.
        let layout = restore(&gui(
            "[gui]\ndrawer_left = [\"files\"]\ndrawer_right_home = [\"files\", \"nope\", \"ask\"]\n",
        ));
        assert_eq!(layout.placement(&DrawerView::Files), Some((Side::Left, 0)));
        assert!(layout.placed_views(Side::Right).is_empty());
    }

    #[test]
    fn an_unsplit_drawer_round_trips_merged() {
        let mut settings = gui("[gui]\ndrawer_left = [\"files\", \"find\"]\n");
        let mut layout = restore(&settings);
        layout.set_split_ratio(Side::Left, 0.3);
        // Unsplit the bottom slot: Find merges into Files' top slot.
        layout.close_slot(Side::Left, 1).unwrap();
        record(&layout, &mut settings);
        assert_eq!(
            entries(&settings),
            [
                e("files", Side::Left, 0, true),
                e("find", Side::Left, 0, false),
            ]
        );
        let mut back = restore(&settings);
        assert_eq!(back.slot_views(Side::Left), [DrawerView::Files]);
        assert_eq!(back.placement(&DrawerView::Find), Some((Side::Left, 0)));
        // Showing Find replaces Files: still one slot.
        assert_eq!(back.open_view(DrawerView::Find), (Side::Left, 0));
        assert_eq!(back.slot_views(Side::Left), [DrawerView::Find]);
        assert_eq!(back.split_ratio(Side::Left), Some(0.3));
    }

    #[test]
    fn view_entries_restore_and_win_over_legacy_keys() {
        let layout = restore(&gui(r#"
[gui]
drawer_left = ["tags"]
drawer_right_home = ["links"]
drawer_right_visible = true
[gui.views]
find = { side = "right", slot = 1, shown = true }
files = { side = "right", slot = 0, shown = false }
outline = { side = "right", slot = 0, shown = true }
"#));
        // The lists and the home are legacy input: ignored next to entries.
        assert_eq!(layout.placement(&DrawerView::Tags), None);
        assert_eq!(layout.placement(&DrawerView::Links), None);
        assert!(layout.placed_views(Side::Left).is_empty());
        assert_eq!(
            layout.slot_views(Side::Right),
            [DrawerView::Outline, DrawerView::Find]
        );
        // The shown view comes first in its slot.
        assert_eq!(
            layout.placed_views(Side::Right),
            [DrawerView::Outline, DrawerView::Files, DrawerView::Find]
        );
    }

    #[test]
    fn invalid_view_entries_are_skipped_or_clamped() {
        let layout = restore(&gui(r#"
[gui.views]
files = { side = "left", slot = 0, shown = true }
find = { side = "left", slot = 0, shown = true }
tags = { side = "left", slot = 7, shown = true }
links = { side = "middle", slot = 0, shown = true }
outline = { side = "left", slot = -1, shown = true }
ask = { side = "right", slot = 0, shown = true }
nope = { side = "right", slot = 0, shown = true }
"#));
        // Two shown at one slot: the first (in id order) wins.
        assert_eq!(
            layout.slot_views(Side::Left),
            [DrawerView::Files, DrawerView::Tags]
        );
        assert_eq!(layout.placement(&DrawerView::Find), Some((Side::Left, 0)));
        // Past the last slot: the last.
        assert_eq!(layout.placement(&DrawerView::Tags), Some((Side::Left, 1)));
        // A bad side, a negative slot, off-rail and unknown ids: skipped.
        assert_eq!(layout.placement(&DrawerView::Links), None);
        assert_eq!(layout.placement(&DrawerView::Outline), None);
        assert!(layout.placed_views(Side::Right).is_empty());
        // Nothing placeable: the default shell.
        let layout = restore(&gui("[gui.views]\nnope = { side = \"left\", slot = 0 }\n"));
        assert_eq!(layout.slot_views(Side::Left), [DrawerView::Files]);
    }

    #[test]
    fn foreign_view_entries_survive_a_save() {
        let mut settings = gui(r#"
[gui.views]
files = { side = "left", slot = 0, shown = true }
tags = { side = "left", slot = 0, shown = false }
ask = { side = "right", slot = 0, shown = true }
newer = { side = "right", slot = 3, shown = true, pinned = true }
broken = "left"
"#);
        let mut layout = restore(&settings);
        layout.move_view(DrawerView::Tags, Side::Right);
        record(&layout, &mut settings);
        let written = toml::to_string(&settings).unwrap();
        let table: toml::Table = toml::from_str(&written).unwrap();
        let views = table["views"].as_table().unwrap();
        // Kept exactly as written, invalid or not.
        assert_eq!(views["newer"]["pinned"].as_bool(), Some(true));
        assert_eq!(views["newer"]["slot"].as_integer(), Some(3));
        assert_eq!(views["broken"].as_str(), Some("left"));
        assert_eq!(views["ask"]["side"].as_str(), Some("right"));
        assert_eq!(views["tags"]["side"].as_str(), Some("right"));
        assert_eq!(views.len(), 5);
    }

    #[test]
    fn legacy_list_files_migrate_to_view_entries() {
        // Before placements: the lists alone.
        let mut settings = gui("[gui]\ndrawer_left = [\"files\", \"find\", \"tags\"]\n\
             drawer_right = [\"outline\", \"backlinks\"]\ndrawer_left_split = 0.3\n");
        record(&restore(&settings), &mut settings);
        assert_no_legacy_keys(&settings);
        assert_eq!(
            entries(&settings),
            [
                e("backlinks", Side::Right, 1, true),
                e("files", Side::Left, 0, true),
                e("find", Side::Left, 1, true),
                e("outline", Side::Right, 0, true),
                e("tags", Side::Left, 1, false),
            ]
        );
        assert_eq!(settings.split_ratio(Side::Left), 0.3);

        // Placements with their subsets.
        let mut settings = gui("[gui]\ndrawer_left = [\"files\", \"find\", \"links\"]\n\
             drawer_left_bottom = [\"find\"]\ndrawer_left_shown = [\"links\", \"find\"]\n");
        let layout = restore(&settings);
        assert_eq!(
            layout.slot_views(Side::Left),
            [DrawerView::Links, DrawerView::Find]
        );
        record(&layout, &mut settings);
        assert_no_legacy_keys(&settings);
        assert_eq!(
            entries(&settings),
            [
                e("files", Side::Left, 0, false),
                e("find", Side::Left, 1, true),
                e("links", Side::Left, 0, true),
            ]
        );
        // The migrated file restores the same layout.
        assert_eq!(
            restore(&settings).slot_views(Side::Left),
            layout.slot_views(Side::Left)
        );
    }

    #[test]
    fn an_empty_drawer_toggled_open_round_trips_visible() {
        let mut settings = gui("");
        let mut layout = restore(&settings);
        assert!(!layout.open_drawer(Side::Right));
        record(&layout, &mut settings);
        assert!(settings.drawer_visible(Side::Right));
        let back = restore(&settings);
        assert!(back.is_visible(Panel::Drawer(Side::Right)));
        assert!(back.slots(Side::Right).is_empty());
        assert_eq!(back.focused(), Panel::Editor);
    }

    #[test]
    fn the_empty_slot_is_never_saved() {
        let mut settings = gui("");
        let mut layout = restore(&settings);
        record(&layout, &mut settings);
        let before = settings.clone();
        layout.split_empty(Side::Left).unwrap();
        let mut with_marker = before.clone();
        record(&layout, &mut with_marker);
        // Only the split ratio the empty slot exposes may be written.
        assert_eq!(entries(&with_marker), entries(&before));
        let back = restore(&with_marker);
        assert_eq!(back.empty_slot(Side::Left), None);
        assert_eq!(back.slots(Side::Left), [Some(DrawerView::Files)]);
    }

    #[test]
    fn rail_moves_are_spatial() {
        assert_eq!(RAIL_SIDE, Side::Left);
        assert_eq!(rail_move_target(Side::Right), Some(Side::Left));
        assert_eq!(rail_move_target(Side::Left), None);
    }

    #[test]
    fn the_empty_slot_hint_names_the_bound_chords() {
        assert_eq!(
            empty_slot_hint(Some("Alt+Shift+Right"), Some("Alt+Shift+Left")),
            "No view here. Move one with Alt+Shift+Right / Alt+Shift+Left, \
             or pick one from the rail."
        );
        assert_eq!(
            empty_slot_hint(None, Some("Alt+Shift+Left")),
            "No view here. Move one with Alt+Shift+Left, or pick one from the rail."
        );
        assert_eq!(
            empty_slot_hint(None, None),
            "No view here. Pick one from the rail."
        );
    }

    #[test]
    fn limits_are_rem_based() {
        let l = limits(1000.0, 16.0);
        assert_eq!(l.drawer_min, 160.0);
        assert_eq!(l.editor_min, 320.0);
        assert_eq!(l.available, 952.0);
        // Zoomed: the floors grow with the rem.
        assert_eq!(limits(1000.0, 20.0).drawer_min, 200.0);
        assert_eq!(limits(10.0, 16.0).available, 0.0);
    }

    #[test]
    fn the_shown_width_clamps_without_losing_the_preference() {
        let layout = restore(&gui("[gui]\ndrawer_left_width = 600\n"));
        // Wide window: as preferred.
        assert_eq!(
            shown_width(&layout, Side::Left, limits(1600.0, 16.0)),
            600.0
        );
        // 1000 - 48 rail - 320 editor = 632 left for the drawer.
        assert_eq!(
            shown_width(&layout, Side::Left, limits(1000.0, 16.0)),
            600.0
        );
        assert_eq!(shown_width(&layout, Side::Left, limits(800.0, 16.0)), 432.0);
        // Too narrow for both floors: the drawer keeps its minimum.
        assert_eq!(shown_width(&layout, Side::Left, limits(300.0, 16.0)), 160.0);
        assert_eq!(layout.drawer_width(Side::Left), 600.0);
    }
}
