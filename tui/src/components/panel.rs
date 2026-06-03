//! The `Panel` seam — the persistent editor-screen surfaces (sidebar, editor,
//! Query panel) behind one interface, the persistent-surface counterpart to
//! the `Overlay` trait. See CONTEXT.md ("TUI surfaces").

/// Identifies a persistent panel. Closed set, mirrors `OverlayKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelKind {
    Sidebar,
    Editor,
    Query,
}

impl PanelKind {
    /// Footer label shown when this panel is focused.
    pub fn label(&self) -> &'static str {
        match self {
            PanelKind::Sidebar => "SIDEBAR",
            PanelKind::Editor => "EDITOR",
            PanelKind::Query => "BACKLINKS",
        }
    }
}
