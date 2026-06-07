//! The **which-key overlay** (spec §8b) — the popup docked above the status
//! bar that documents the pending leader sequence. It renders the node the
//! `LeaderEngine` currently sits on, so it can never drift from the tree:
//! same data, two surfaces.
//!
//! Reveal policy: hidden while a sequence is typed fluently; shown once the
//! user hesitates past `leader_timeout_ms`. Hidden the instant the sequence
//! fires or cancels (the engine simply stops being pending).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::components::panel::{ModalBg, ModalSpec, modal_chrome};
use crate::keys::leader::{LeaderEngine, LeaderNode};
use crate::settings::themes::Theme;

/// Minimum column width for a `key → target` cell.
const CELL_WIDTH: u16 = 24;

/// Rows the overlay needs for the current node (header + grid + borders).
/// The caller carves this out of the area directly above the status bar.
pub fn desired_height(engine: &LeaderEngine, width: u16) -> u16 {
    let n = engine.current_node().children().len() as u16;
    let cols = (width.saturating_sub(2) / CELL_WIDTH).max(1);
    let grid_rows = n.div_ceil(cols);
    grid_rows + 3 // top border + header + grid + bottom border
}

/// Render the overlay into `rect` (the caller positions it docked above the
/// status bar, full width).
pub fn render(
    f: &mut Frame,
    rect: Rect,
    theme: &Theme,
    engine: &LeaderEngine,
    gateway_label: &str,
) {
    let inner = modal_chrome(
        f,
        rect,
        theme,
        ModalSpec {
            border: Some(Style::default().fg(theme.focus_border.to_ratatui())),
            bg: ModalBg::Hard,
            ..Default::default()
        },
    );
    if inner.height == 0 {
        return;
    }

    let node = engine.current_node();
    let keycap = Style::default()
        .fg(theme.yellow.to_ratatui())
        .add_modifier(Modifier::BOLD);
    let muted = Style::default().fg(theme.gray.to_ratatui());
    let caption_style = Style::default().fg(theme.fg_secondary.to_ratatui());

    // ── Header: pressed keycaps · caption · right-aligned controls ─────────
    let mut pressed = format!(" {gateway_label}");
    for c in engine.path() {
        pressed.push(' ');
        pressed.push(*c);
    }
    let controls = "Esc cancel · BkSp up ";
    let controls_w = controls.len() as u16;
    let header_cols = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints([
            ratatui::layout::Constraint::Min(0),
            ratatui::layout::Constraint::Length(controls_w),
        ])
        .split(Rect::new(inner.x, inner.y, inner.width, 1));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(pressed, keycap),
            Span::styled(format!("  {}", node.label()), caption_style),
        ])),
        header_cols[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(controls, muted)))
            .alignment(ratatui::layout::Alignment::Right),
        header_cols[1],
    );

    // ── Body: multi-column key → target grid ───────────────────────────────
    let children = node.children();
    if children.is_empty() {
        return;
    }
    let cols = (inner.width / CELL_WIDTH).max(1) as usize;
    let rows = children.len().div_ceil(cols);
    let arrow = Span::styled(" → ", muted);
    for (i, (key, child)) in children.iter().enumerate() {
        // Column-major fill: read top-to-bottom within a column, like the
        // spec mockup.
        let col = i / rows;
        let row = i % rows;
        let y = inner.y + 1 + row as u16;
        if y >= inner.bottom() {
            continue;
        }
        let x = inner.x + (col as u16) * CELL_WIDTH;
        if x >= inner.right() {
            continue;
        }
        let target_style = match child {
            LeaderNode::Group { .. } => Style::default().fg(theme.aqua.to_ratatui()),
            LeaderNode::Leaf { .. } => Style::default().fg(theme.fg.to_ratatui()),
        };
        let cell = Rect::new(x, y, CELL_WIDTH.min(inner.right() - x), 1);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {key}"), keycap),
                arrow.clone(),
                Span::styled(child.label().to_string(), target_style),
            ])),
            cell,
        );
    }
}
