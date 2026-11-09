//! Navigation payload types shared by the drawer and its views.

/// The views the activity rail can put in the drawer. Closed set, mirrors
/// the rail items top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawerView {
    Files,
    Find,
    Semantic,
    Ask,
    Tags,
    Links,
    Outline,
    Config,
}

impl DrawerView {
    /// Status-bar label when the drawer shows this view.
    pub fn label(&self) -> &'static str {
        match self {
            DrawerView::Files => "FILES",
            DrawerView::Find => "FIND",
            DrawerView::Semantic => "SEMANTIC",
            DrawerView::Ask => "ASK",
            DrawerView::Tags => "TAGS",
            DrawerView::Links => "LINKS",
            DrawerView::Outline => "OUTLINE",
            DrawerView::Config => "CFG",
        }
    }

    /// Every view, in rail order (top to bottom; CFG last).
    pub const ALL: [DrawerView; 8] = [
        DrawerView::Files,
        DrawerView::Find,
        DrawerView::Semantic,
        DrawerView::Ask,
        DrawerView::Tags,
        DrawerView::Links,
        DrawerView::Outline,
        DrawerView::Config,
    ];

    /// The stable id the `[gui]` settings name this view by
    /// (`drawer_left = ["files"]`, ADR-0050 §5). Never change one: it is
    /// persisted.
    pub fn id(&self) -> &'static str {
        match self {
            DrawerView::Files => "files",
            DrawerView::Find => "find",
            DrawerView::Semantic => "semantic",
            DrawerView::Ask => "ask",
            DrawerView::Tags => "tags",
            DrawerView::Links => "links",
            DrawerView::Outline => "outline",
            DrawerView::Config => "config",
        }
    }

    /// The view a persisted id names, if any (exact, lower-case match).
    pub fn from_id(id: &str) -> Option<DrawerView> {
        Self::ALL.into_iter().find(|v| v.id() == id)
    }
}

#[cfg(test)]
mod tests {
    use super::DrawerView;

    #[test]
    fn view_ids_round_trip_and_are_unique() {
        for view in DrawerView::ALL {
            assert_eq!(DrawerView::from_id(view.id()), Some(view));
        }
        let mut ids: Vec<_> = DrawerView::ALL.iter().map(DrawerView::id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), DrawerView::ALL.len());
        assert_eq!(DrawerView::from_id("files"), Some(DrawerView::Files));
        assert_eq!(DrawerView::from_id("backlinks"), None);
        assert_eq!(DrawerView::from_id("Files"), None);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinksTab {
    Backlinks,
    Outgoing,
    Unlinked,
}

impl LinksTab {
    /// The sub-view order, single source for cycling and the tab bar.
    pub const ORDER: [LinksTab; 3] = [LinksTab::Backlinks, LinksTab::Outgoing, LinksTab::Unlinked];

    /// The tab `steps` away in [`Self::ORDER`], wrapping.
    pub fn cycled(self, steps: isize) -> LinksTab {
        let n = Self::ORDER.len() as isize;
        let i = Self::ORDER.iter().position(|t| *t == self).unwrap_or(0) as isize;
        Self::ORDER[((i + steps).rem_euclid(n)) as usize]
    }

    pub fn label(self) -> &'static str {
        match self {
            LinksTab::Backlinks => "backlinks",
            LinksTab::Outgoing => "outgoing",
            LinksTab::Unlinked => "unlinked",
        }
    }
}

/// Which heading an OUTLINE row stands for, robust to the buffer moving on
/// after the row was listed: the `occurrence`-th of the `count` headings
/// reading `text`, last seen on `line`. See `OutlinePanel::resolve`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HeadingTarget {
    pub text: String,
    pub occurrence: usize,
    pub count: usize,
    pub line: usize,
}
