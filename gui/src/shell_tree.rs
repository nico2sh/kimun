//! The round trip between the GUI's [`LayoutTree`] and the live layout in
//! `[gui.layout]` (ADR-0052 §5): the tree as the spec TOML the layout model
//! reads and writes ([`spec::build`], [`spec::to_spec`]). Toolkit-free and
//! tested without a window.
//!
//! Every view is written where it is placed, ASK included (ADR-0053: Ask
//! is one regular view; its rail entry, not its placement, waits for the
//! server).

use kimun_notes::logic::layout_tree::LayoutTree;
use kimun_notes::logic::layout_tree::spec::{self, ChildSpec};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::rag_caps::Caps;
use kimun_notes::settings::gui::GuiSettings;

/// What the window restores against: every view the layout may hold
/// whatever the server says now (a SEM or ASK view placed by a session
/// with the server on is kept).
pub const RESTORE_CAPS: Caps = Caps {
    semantic: true,
    ask: true,
};

/// The layout `[gui.layout]` describes, built against `caps` (views it
/// does not offer are skipped, with a warning in the log). Absent, not a
/// layout, or invalid: the default layout, the problem logged.
pub fn restore(gui: &GuiSettings, caps: Caps) -> LayoutTree<DrawerView> {
    let fallback = || LayoutTree::default_layout(DrawerView::Files);
    let Some(value) = gui.layout() else {
        return fallback();
    };
    let spec: ChildSpec = match value.try_into() {
        Ok(spec) => spec,
        Err(e) => {
            tracing::warn!("[gui.layout] is not a layout, using the default: {e}");
            return fallback();
        }
    };
    match spec::build(&spec, caps) {
        Ok((tree, warnings)) => {
            for warning in warnings {
                tracing::warn!("[gui.layout]: {warning}");
            }
            tree
        }
        Err(errors) => {
            for error in errors {
                tracing::warn!("[gui.layout], using the default: {error}");
            }
            fallback()
        }
    }
}

/// Write `layout` into `gui` as `[gui.layout]`. Every other `[gui]` key
/// is left alone.
pub fn record(layout: &LayoutTree<DrawerView>, gui: &mut GuiSettings) {
    let spec = spec::to_spec(layout);
    match toml::Value::try_from(&spec) {
        Ok(value) => gui.set_layout(value),
        Err(e) => tracing::warn!("could not write [gui.layout]: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_notes::logic::layout_tree::{Axis, Focus, Leaf};
    use kimun_notes::settings::AppSettings;

    fn gui(toml: &str) -> GuiSettings {
        toml::from_str::<AppSettings>(toml).unwrap().gui
    }

    /// A TOML document, one line per entry.
    fn lines(lines: &[&str]) -> String {
        lines.join("\n")
    }

    fn default() -> LayoutTree<DrawerView> {
        LayoutTree::default_layout(DrawerView::Files)
    }

    #[test]
    fn a_missing_or_broken_layout_is_the_default() {
        assert_eq!(restore(&GuiSettings::default(), RESTORE_CAPS), default());
        // Not a layout at all.
        assert_eq!(
            restore(&gui("[gui.layout]\nfoo = 1\n"), RESTORE_CAPS),
            default()
        );
        // A layout that does not validate: an unknown view.
        let bad = lines(&[
            "[gui.layout]",
            "split = \"horizontal\"",
            "children = [",
            "{ views = [\"nope\"] },",
            "{ editor = true },",
            "]",
        ]);
        assert_eq!(restore(&gui(&bad), RESTORE_CAPS), default());
    }

    #[test]
    fn record_round_trips_placements_and_weights() {
        let mut tree = default();
        let left = tree.slots()[0];
        tree.place(DrawerView::Tags, left, false);
        let low = tree.split_slot(left, Axis::Vertical).unwrap();
        tree.move_view(DrawerView::Find, low);
        let right = *tree.slots().last().unwrap();
        tree.move_view(DrawerView::Outline, right);
        let mut settings = gui("[gui]\nkept = 1\n");
        record(&tree, &mut settings);
        let back = restore(&settings, RESTORE_CAPS);
        // Slot ids are renumbered in reading order; the arrangement,
        // weights and placements are the same.
        assert_eq!(spec::to_spec(&back), spec::to_spec(&tree));
        let (old, new) = (tree.slots(), back.slots());
        assert_eq!(old.len(), new.len());
        for (o, n) in old.into_iter().zip(new) {
            assert_eq!(back.slot_views(n), tree.slot_views(o));
            assert_eq!(back.shown(n), tree.shown(o));
        }
        // Focus is not part of the layout: back on the editor.
        assert_eq!(
            back.focused(),
            Focus::Leaf(Leaf::Group(back.active_group()))
        );
        // Keys the layout does not own survive a write.
        let written = toml::to_string(&settings).unwrap();
        assert!(written.contains("kept = 1"), "{written}");
        // A second save writes the same table.
        let mut again = settings.clone();
        record(&back, &mut again);
        assert_eq!(again, settings);
    }

    /// ASK is a regular view (ADR-0053): recorded where it is placed and
    /// shown, and restored there whatever the server says at startup (its
    /// rail entry waits for the server, its placement does not).
    #[test]
    fn a_saved_layout_keeps_ask() {
        let mut tree = default();
        let right = *tree.slots().last().unwrap();
        tree.move_view(DrawerView::Ask, right);
        tree.show_view(&DrawerView::Ask, true);
        assert_eq!(tree.shown(right), Some(DrawerView::Ask));
        let mut settings = GuiSettings::default();
        record(&tree, &mut settings);
        let written = toml::to_string(&settings).unwrap();
        assert!(written.contains("\"ask\""), "{written}");
        let back = restore(&settings, RESTORE_CAPS);
        assert_eq!(back.view_slot(&DrawerView::Ask), Some(right));
        assert_eq!(back.shown(right), Some(DrawerView::Ask));
        assert_eq!(spec::to_spec(&back), spec::to_spec(&tree));
    }

    #[test]
    fn a_placed_semantic_view_survives_with_the_server_off() {
        let mut tree = default();
        let right = *tree.slots().last().unwrap();
        tree.move_view(DrawerView::Semantic, right);
        let mut settings = GuiSettings::default();
        record(&tree, &mut settings);
        let back = restore(&settings, RESTORE_CAPS);
        assert_eq!(back.shown(right), Some(DrawerView::Semantic));
    }

    #[test]
    fn invalid_saved_layout_falls_back_to_default() {
        // No editor group: a layout must hold one.
        let no_editor = lines(&[
            "[gui.layout]",
            "split = \"horizontal\"",
            "children = [",
            "{ views = [\"files\"], shown = \"files\" },",
            "{ views = [\"tags\"], shown = \"tags\" },",
            "]",
        ]);
        assert_eq!(restore(&gui(&no_editor), RESTORE_CAPS), default());
        // A shown view that is not placed in its slot.
        let stray = lines(&[
            "[gui.layout]",
            "split = \"horizontal\"",
            "children = [",
            "{ views = [\"files\"], shown = \"tags\" },",
            "{ editor = true },",
            "]",
        ]);
        assert_eq!(restore(&gui(&stray), RESTORE_CAPS), default());
        // Not a table.
        assert_eq!(
            restore(&gui("[gui]\nlayout = 3\n"), RESTORE_CAPS),
            default()
        );
    }

    #[test]
    fn record_does_not_write_zoom_or_collapse_state() {
        let mut tree = default();
        let left = tree.slots()[0];
        tree.zoom(Some(Leaf::Slot(left)));
        assert_eq!(tree.zoomed(), Some(Leaf::Slot(left)));
        let mut settings = GuiSettings::default();
        record(&tree, &mut settings);
        let written = toml::to_string(&settings).unwrap();
        assert!(!written.contains("zoom"), "{written}");
        assert!(!written.contains("collapse"), "{written}");
        // The unzoomed tree writes the same table.
        let mut plain = GuiSettings::default();
        record(&default(), &mut plain);
        assert_eq!(plain, settings);
        assert_eq!(restore(&settings, RESTORE_CAPS).zoomed(), None);
    }
}
