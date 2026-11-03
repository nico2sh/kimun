//! The shell's toolkit-free half (ADR-0050): which views the rail offers,
//! what they are called, the sizing floors in rems, and the round trip
//! between the logic [`PanelLayout`] and the `[gui]` settings table. The
//! workspace view renders and routes; everything here is plain data and
//! tested without a window.

use kimun_notes::logic::layout::{Panel, PanelLayout, Side, WidthLimits};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::settings::gui::GuiSettings;

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

/// The layout the `[gui]` table describes: the left drawer shows the first
/// listed view the rail offers (the file browser when none is), at the
/// persisted width, shown or hidden; the editor has the focus. Unknown ids
/// are skipped, not fatal. The shell has one drawer slot so far (M5b-3
/// splits), so only the top view is placed.
pub fn restore(gui: &GuiSettings) -> PanelLayout<DrawerView> {
    let view = gui
        .drawer_views(Side::Left)
        .iter()
        .filter_map(|id| DrawerView::from_id(id))
        .find(|v| is_on_rail(*v))
        .unwrap_or(DrawerView::Files);
    let mut layout = PanelLayout::new(view, gui.drawer_width(Side::Left));
    if !gui.drawer_visible(Side::Left) {
        layout.hide(Panel::Drawer(Side::Left));
    }
    layout
}

/// Write the left drawer's width, visibility and view into `gui`. The view
/// replaces the first listed one; the rest of the list (slots a later
/// version placed) is kept. The right drawer is not the shell's yet and is
/// left as it was.
pub fn record(layout: &PanelLayout<DrawerView>, gui: &mut GuiSettings) {
    gui.set_drawer_width(Side::Left, layout.drawer_width(Side::Left));
    gui.set_drawer_visible(Side::Left, layout.is_visible(Panel::Drawer(Side::Left)));
    let mut views: Vec<String> = layout
        .slot_views(Side::Left)
        .iter()
        .map(|v| v.id().to_string())
        .collect();
    for id in gui.drawer_views(Side::Left).into_iter().skip(1) {
        if !views.contains(&id) {
            views.push(id);
        }
    }
    gui.set_drawer_views(Side::Left, &views);
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
        layout.place_view(DrawerView::Find);
        layout.hide(Panel::Drawer(Side::Left));
        layout.set_drawer_width(Side::Left, 333.0, limits(2000.0, 16.0));
        record(&layout, &mut settings);
        assert_eq!(settings.drawer_views(Side::Left), ["find", "backlinks"]);
        let back = restore(&settings);
        assert_eq!(back.slot_views(Side::Left), [DrawerView::Find]);
        assert_eq!(back.drawer_width(Side::Left), 333.0);
        assert!(!back.is_visible(Panel::Drawer(Side::Left)));
        // Keys the shell does not own survive a write.
        let written = toml::to_string(&settings).unwrap();
        assert!(written.contains("kept = 1"), "{written}");
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
