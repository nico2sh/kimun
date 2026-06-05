//! The **command palette** (spec §6: `›`-prefixed telescope scope): a fuzzy
//! list of every leader-tree command. Selecting one executes its
//! [`LeaderAction`] — the palette is a labelled door onto the same actions
//! the leader sequences fire, never a second implementation.

use async_trait::async_trait;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, ListItem, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, redraw_callback};
use crate::components::overlay::{Overlay, OverlayKind};
use crate::components::rich_row::RichRow;
use crate::components::search_list::{
    Emit, Filter, KeyReaction, RowSource, SearchList, SearchMouse, SearchRow,
};
use crate::keys::leader::{LeaderAction, LeaderNode};
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// One palette row: a leader leaf with its full key sequence.
#[derive(Clone)]
pub struct CommandEntry {
    /// `group label · leaf label`, e.g. `+find · files`.
    pub label: String,
    /// The key sequence, e.g. `Ctrl+G f f`.
    pub keys: String,
    /// `label + keys`, so the fuzzy filter matches either.
    haystack: String,
    pub action: LeaderAction,
}

impl SearchRow for CommandEntry {
    fn to_list_item(&self, theme: &Theme, _icons: &Icons, _selected: bool) -> ListItem<'static> {
        RichRow::new("›", self.label.clone())
            .glyph_style(Style::default().fg(theme.gray.to_ratatui()))
            .meta(self.keys.clone())
            .into_list_item(theme)
    }

    fn match_text(&self) -> Option<&str> {
        Some(&self.haystack)
    }

    fn visual_height(&self) -> u16 {
        1
    }
}

/// Flatten a leader tree into palette entries — the single keymap source.
pub fn command_entries(tree: &LeaderNode, gateway: &str) -> Vec<CommandEntry> {
    fn walk(node: &LeaderNode, group: &str, keys: &str, out: &mut Vec<CommandEntry>) {
        for (key, child) in node.children() {
            let child_keys = format!("{keys} {key}");
            match child {
                // The palette never lists itself — selecting it would just
                // close and reopen the palette.
                LeaderNode::Leaf { action, .. } if *action == LeaderAction::Palette => {}
                LeaderNode::Leaf { label, action } => {
                    let label = if group.is_empty() {
                        (*label).to_string()
                    } else {
                        format!("{group} · {label}")
                    };
                    out.push(CommandEntry {
                        haystack: format!("{label} {child_keys}"),
                        label,
                        keys: child_keys,
                        action: *action,
                    });
                }
                LeaderNode::Group { label, .. } => walk(child, label, &child_keys, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, "", gateway, &mut out);
    out
}

struct CommandSource {
    entries: Vec<CommandEntry>,
}

#[async_trait]
impl RowSource<CommandEntry> for CommandSource {
    async fn load(&self, _query: &str, emit: Emit<CommandEntry>) {
        emit.replace(self.entries.clone());
    }

    fn reload_on_query(&self) -> bool {
        false // load once; the fuzzy filter narrows
    }
}

/// The palette modal — same engine as the note browser, command rows.
pub struct CommandPaletteModal {
    list: SearchList<CommandEntry>,
}

impl CommandPaletteModal {
    pub fn new(tree: &LeaderNode, gateway: &str, icons: Icons, tx: AppTx) -> Self {
        let source = CommandSource {
            entries: command_entries(tree, gateway),
        };
        let list = SearchList::builder(source, redraw_callback(tx))
            .filter(Filter::Fuzzy)
            .icons(icons)
            .build();
        Self { list }
    }

    fn execute_selected(&self, tx: &AppTx) {
        if let Some(entry) = self.list.selected_row() {
            let action = entry.action;
            // Close first so the action runs with no overlay open — several
            // actions (dialogs, pickers) no-op while one is.
            tx.send(AppEvent::CloseOverlay).ok();
            tx.send(AppEvent::ExecuteLeaderAction(action)).ok();
        }
    }
}

impl Overlay for CommandPaletteModal {
    fn kind(&self) -> OverlayKind {
        OverlayKind::CommandPalette
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Key(key) => match self.list.handle_key(key) {
                KeyReaction::Submit => {
                    self.execute_selected(tx);
                    EventState::Consumed
                }
                KeyReaction::Cancel => {
                    tx.send(AppEvent::CloseOverlay).ok();
                    EventState::Consumed
                }
                _ => EventState::Consumed,
            },
            InputEvent::Mouse(mouse) => {
                if let SearchMouse::Activated(_) = self.list.handle_mouse(mouse) {
                    self.execute_selected(tx);
                }
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let popup = crate::components::centered_rect(60, 60, area);
        f.render_widget(Clear, popup);
        let modal_style = Style::default()
            .fg(theme.fg.to_ratatui())
            .bg(theme.bg_hard.to_ratatui());
        let block = Block::default()
            .title(" Commands ")
            .borders(Borders::ALL)
            .border_style(theme.border_style(true))
            .style(modal_style);
        let inner = block.inner(popup);
        f.render_widget(block, popup);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(inner);

        // `›` prefix + plain input (commands aren't query grammar).
        let prefix = "› ";
        f.render_widget(
            Paragraph::new(prefix).style(Style::default().fg(theme.yellow.to_ratatui())),
            rows[0],
        );
        let input_rect = Rect {
            x: rows[0].x + 2,
            width: rows[0].width.saturating_sub(2),
            ..rows[0]
        };
        self.list.render_query(f, input_rect, theme, true);

        self.list.render(f, rows[1], theme, true);
        self.list.set_list_rect(rows[1]);
        self.list.set_panel_rect(popup);

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "↑↓ move · ⏎ run · Esc close",
                Style::default().fg(theme.gray.to_ratatui()),
            ))),
            rows[2],
        );

        self.list.render_autocomplete(f, popup, theme);
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        vec![("↑↓".into(), "move".into()), ("Enter".into(), "run".into())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_cover_every_leader_leaf() {
        let entries = command_entries(&crate::keys::leader::leader_tree(), "Ctrl+G");
        // Spot-check shape and coverage.
        assert!(entries.len() > 20);
        assert!(
            entries
                .iter()
                .any(|e| e.keys == "Ctrl+G o f" && e.label.contains("files"))
        );
        assert!(
            entries
                .iter()
                .any(|e| e.action == LeaderAction::Help && e.keys == "Ctrl+G ?")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enter_closes_then_executes() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut palette = CommandPaletteModal::new(
            &crate::keys::leader::leader_tree(),
            "Ctrl+G",
            Icons::new(false),
            tx.clone(),
        );
        // Let the load land.
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            palette.list.poll();
        }
        assert!(palette.list.selected_row().is_some());

        palette.handle_input(
            &InputEvent::Key(ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Enter,
                ratatui::crossterm::event::KeyModifiers::NONE,
            )),
            &tx,
        );

        let mut order = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            match ev {
                AppEvent::CloseOverlay => order.push("close"),
                AppEvent::ExecuteLeaderAction(_) => order.push("execute"),
                _ => {}
            }
        }
        assert_eq!(order, vec!["close", "execute"]);
    }
}
