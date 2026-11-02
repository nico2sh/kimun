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
