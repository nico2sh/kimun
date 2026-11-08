//! The static registry of built-in views: the lookup table the layout tree
//! uses for ids, titles, names, minimum sizes and availability. Toolkit-free.
//! Sizes are in units (1 unit = 1 rem in the GUI).

use crate::logic::nav::DrawerView;
use crate::logic::rag_caps::Caps;

/// What a view needs from the server before it is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Needs {
    Nothing,
    /// Semantic search (`Caps::semantic`).
    Semantic,
    /// The ASK LLM (`Caps::ask`).
    Ask,
}

/// One built-in view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSpec {
    pub id: &'static str,
    pub view: DrawerView,
    /// Slot header title.
    pub title: &'static str,
    /// Interface name (rail tooltip, accessible name).
    pub name: &'static str,
    pub needs: Needs,
    /// Minimum (width, height) in units.
    pub min: (f64, f64),
    /// Where an unplaced view first opens: true only for ASK (right side).
    pub home_right: bool,
}

const MIN: (f64, f64) = (10.0, 6.0);
const ASK_MIN: (f64, f64) = (18.0, 12.0);

// Titles and names copied from gui/src/shell_layout.rs `slot_title` and
// `view_name`; Task 7 deletes the gui copies.
const fn s(
    id: &'static str,
    view: DrawerView,
    title: &'static str,
    name: &'static str,
    needs: Needs,
) -> ViewSpec {
    ViewSpec { id, view, title, name, needs, min: MIN, home_right: false }
}

static TABLE: [ViewSpec; 8] = [
    s("files", DrawerView::Files, "Files", "Files", Needs::Nothing),
    s("find", DrawerView::Find, "Find", "Find", Needs::Nothing),
    s("semantic", DrawerView::Semantic, "Semantic search", "Semantic search", Needs::Semantic),
    ViewSpec {
        id: "ask",
        view: DrawerView::Ask,
        title: "Sources",
        name: "Ask",
        needs: Needs::Ask,
        min: ASK_MIN,
        home_right: true,
    },
    s("tags", DrawerView::Tags, "Tags", "Tags", Needs::Nothing),
    s("links", DrawerView::Links, "Links", "Links", Needs::Nothing),
    s("outline", DrawerView::Outline, "Outline", "Outline", Needs::Nothing),
    s("config", DrawerView::Config, "Settings", "Settings", Needs::Nothing),
];

/// Every built-in view, in rail order.
pub fn all() -> &'static [ViewSpec] {
    &TABLE
}

pub fn by_id(id: &str) -> Option<&'static ViewSpec> {
    TABLE.iter().find(|s| s.id == id)
}

pub fn spec(view: DrawerView) -> &'static ViewSpec {
    TABLE
        .iter()
        .find(|s| s.view == view)
        .expect("every DrawerView has a spec")
}

/// Whether the server currently offers `spec`'s view.
pub fn available(spec: &ViewSpec, caps: Caps) -> bool {
    match spec.needs {
        Needs::Nothing => true,
        Needs::Semantic => caps.semantic,
        Needs::Ask => caps.ask,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_drawer_view_has_exactly_one_spec() {
        for v in DrawerView::ALL {
            assert_eq!(all().iter().filter(|s| s.view == v).count(), 1, "{v:?}");
            assert_eq!(spec(v).view, v);
        }
        assert_eq!(all().len(), DrawerView::ALL.len());
    }

    #[test]
    fn ids_are_unique_and_round_trip_through_from_id() {
        for s in all() {
            assert_eq!(DrawerView::from_id(s.id), Some(s.view));
            assert_eq!(s.id, s.view.id());
            assert_eq!(by_id(s.id).map(|x| x.view), Some(s.view));
            assert_eq!(all().iter().filter(|o| o.id == s.id).count(), 1);
        }
        assert!(by_id("nope").is_none());
    }

    #[test]
    fn ask_needs_ask_and_files_needs_nothing() {
        let none = Caps::default();
        let sem = Caps { semantic: true, ask: false };
        let ask = Caps { semantic: false, ask: true };
        assert!(available(spec(DrawerView::Files), none));
        assert!(!available(spec(DrawerView::Ask), none));
        assert!(available(spec(DrawerView::Ask), ask));
        assert!(!available(spec(DrawerView::Semantic), none));
        assert!(available(spec(DrawerView::Semantic), sem));
        assert_eq!(spec(DrawerView::Ask).needs, Needs::Ask);
        assert_eq!(spec(DrawerView::Files).needs, Needs::Nothing);
        assert!(spec(DrawerView::Ask).home_right);
        assert_eq!(all().iter().filter(|s| s.home_right).count(), 1);
    }

    #[test]
    fn minimums_are_positive() {
        for s in all() {
            assert!(s.min.0 > 0.0 && s.min.1 > 0.0, "{}", s.id);
        }
        assert_eq!(spec(DrawerView::Ask).min, (18.0, 12.0));
        assert_eq!(spec(DrawerView::Files).min, (10.0, 6.0));
    }
}
