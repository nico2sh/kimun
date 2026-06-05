//! The `Panel` seam — the persistent editor-screen surfaces (activity rail,
//! drawer, editor) behind one interface, the persistent-surface counterpart to
//! the `Overlay` trait. See CONTEXT.md ("TUI surfaces").

use ratatui::widgets::{Block, Borders};

use crate::settings::themes::Theme;

/// Identifies a persistent panel. Closed set, mirrors `OverlayKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelKind {
    /// The fixed-width activity rail on the far left.
    Rail,
    /// The single drawer panel; renders whichever rail view is active.
    Drawer,
    /// The editor; always visible, fills the remaining width.
    Editor,
}

impl PanelKind {
    /// Footer label shown when this panel is focused. The drawer's label
    /// depends on its active view — `PanelSet::focused_label()` resolves it.
    pub fn label(&self) -> &'static str {
        match self {
            PanelKind::Rail => "RAIL",
            PanelKind::Drawer => "DRAWER",
            PanelKind::Editor => "EDITOR",
        }
    }
}

/// Shared panel chrome: a single-line box with the title embedded in the top
/// border (`┌─ Title ────┐`) and the border colored by focus state — the one
/// way every panel draws its frame.
pub fn panel_block(title: &str, theme: &Theme, focused: bool) -> Block<'static> {
    Block::default()
        .title(format!("─ {title} "))
        .borders(Borders::ALL)
        .border_style(theme.border_style(focused))
        .style(theme.base_style())
}
