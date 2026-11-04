//! The **Overlay** vocabulary both frontends share: which overlays can open
//! ([`OverlayOpen`], a construction recipe) and what kind the open one is
//! ([`OverlayKind`], its presentation identity). Pure data — the TUI builds
//! its ratatui overlays from a recipe; the GUI opens its own surfaces for the
//! recipes it has.

/// Identifies which overlay is active — used for toggle, focus label, and hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    NoteBrowser,
    SavedSearches,
    CommandPalette,
    Dialog,
}

impl OverlayKind {
    /// Footer label for this overlay kind.
    pub fn label(&self) -> &'static str {
        match self {
            OverlayKind::NoteBrowser => "NOTE BROWSER",
            OverlayKind::SavedSearches => "SAVED SEARCHES",
            OverlayKind::CommandPalette => "COMMANDS",
            OverlayKind::Dialog => "DIALOG",
        }
    }
}

/// Every overlay the editor screen can open, as a construction recipe the
/// executor's one `build_overlay` match turns into the live overlay. Distinct
/// from `OverlayKind` (the *presentation* kind): two recipes share
/// `OverlayKind::NoteBrowser` (search browser vs file finder), so the kind
/// alone cannot pick the opener.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OverlayOpen {
    SearchBrowser,
    FileFinder,
    SavedSearches,
    CommandPalette,
    WorkspaceSwitcher,
    ThemePicker,
    /// The flat key-bindings help (F1).
    Help,
    /// The search query syntax reference (F1 over the Find panel).
    QueryHelp,
    /// The full leader-tree cheatsheet (leader `?`).
    Cheatsheet,
    SortQuery,
    SortSidebar,
    /// The sort dialog stacked over the open sortable note browser (Ctrl+K
    /// search browser or Ctrl+O file finder) — the one
    /// recipe that opens over an overlay instead of being refused by it.
    SortBrowser,
    /// The query syntax reference stacked over the open Ctrl+K browser —
    /// the other recipe that opens over an overlay.
    QueryHelpOverBrowser,
    QuickNote,
    /// The pinned-notes dialog (leader `f p`).
    PinnedNotes,
}

impl OverlayOpen {
    /// Whether this recipe opens *over* an open overlay (parking it) rather
    /// than being refused by one. Every other recipe needs nothing open.
    pub fn stacks(self) -> bool {
        matches!(
            self,
            OverlayOpen::SortBrowser | OverlayOpen::QueryHelpOverBrowser
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_browser_recipes_stack() {
        assert!(OverlayOpen::SortBrowser.stacks());
        assert!(OverlayOpen::QueryHelpOverBrowser.stacks());
        assert!(!OverlayOpen::CommandPalette.stacks());
        assert!(!OverlayOpen::FileFinder.stacks());
    }

    #[test]
    fn kind_labels() {
        assert_eq!(OverlayKind::CommandPalette.label(), "COMMANDS");
        assert_eq!(OverlayKind::Dialog.label(), "DIALOG");
    }
}
