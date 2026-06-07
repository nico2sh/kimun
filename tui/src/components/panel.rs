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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style(focused))
        .style(theme.base_style());
    if title.is_empty() {
        // No title: keep the top border unbroken (a titled block would punch
        // a `─  ` gap into it).
        block
    } else {
        block.title(format!("─ {title} "))
    }
}

/// The popup background: regular panel bg, or the harder shade spec §6 gives
/// the telescope-style modals.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ModalBg {
    #[default]
    Panel,
    Hard,
    /// The screen's base background (`theme.base_style()`) — panels that
    /// read as part of the canvas rather than a raised popup.
    Base,
}

/// What a popup shell looks like. Pair with [`modal_chrome`].
#[derive(Default)]
pub struct ModalSpec<'a> {
    /// Top-border title, rendered as-is (callers keep their ` Title ` padding).
    pub title: Option<&'a str>,
    /// Border style; `None` = the focused-border style (`theme.border_style(true)`).
    pub border: Option<ratatui::style::Style>,
    pub bg: ModalBg,
}

/// The one way every popup draws its shell (spec §6): clear the area behind
/// it, draw the titled/bordered block, return the inner rect to fill.
/// Centering stays with the caller — popups center by percent, by fixed size,
/// or dock (which-key), but the shell is identical.
pub fn modal_chrome(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    theme: &Theme,
    spec: ModalSpec,
) -> ratatui::layout::Rect {
    f.render_widget(ratatui::widgets::Clear, area);
    let style = match spec.bg {
        ModalBg::Panel => theme.panel_style(),
        ModalBg::Hard => ratatui::style::Style::default()
            .fg(theme.fg.to_ratatui())
            .bg(theme.bg_hard.to_ratatui()),
        ModalBg::Base => theme.base_style(),
    };
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(spec.border.unwrap_or_else(|| theme.border_style(true)))
        .style(style);
    if let Some(title) = spec.title {
        block = block.title(title.to_string());
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    inner
}
