//! The **Drawer** — the single panel between the activity rail and the
//! editor. It renders whichever rail view is active: the file browser
//! (FILES), the Query panel (FIND), or a placeholder for the views that land
//! in later phases (TAGS, LINKS, OUTLINE, CFG).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;

use crate::components::Component;
use crate::components::backlinks_panel::QueryPanel;
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::components::panel::panel_block;
use crate::components::sidebar::SidebarComponent;
use crate::settings::themes::Theme;

/// The views the activity rail can put in the drawer. Closed set, mirrors
/// the rail items top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawerView {
    Files,
    Find,
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
            DrawerView::Tags => "TAGS",
            DrawerView::Links => "LINKS",
            DrawerView::Outline => "OUTLINE",
            DrawerView::Config => "CFG",
        }
    }
}

/// Hosts the drawer views. FILES and FIND are the ported existing panels
/// (file browser and Query panel); the rest are placeholders until their
/// phases land.
pub struct DrawerHost {
    active: DrawerView,
    sidebar: SidebarComponent,
    query: QueryPanel,
}

impl DrawerHost {
    pub fn new(sidebar: SidebarComponent, query: QueryPanel) -> Self {
        Self {
            active: DrawerView::Files,
            sidebar,
            query,
        }
    }

    pub fn active_view(&self) -> DrawerView {
        self.active
    }

    pub fn set_view(&mut self, view: DrawerView) {
        self.active = view;
    }

    // ── Typed accessors for view-specific calls from the host screen ───────

    pub fn sidebar(&self) -> &SidebarComponent {
        &self.sidebar
    }
    pub fn sidebar_mut(&mut self) -> &mut SidebarComponent {
        &mut self.sidebar
    }
    pub fn query(&self) -> &QueryPanel {
        &self.query
    }
    pub fn query_mut(&mut self) -> &mut QueryPanel {
        &mut self.query
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        match self.active {
            DrawerView::Files => self.sidebar.hint_shortcuts(),
            DrawerView::Find => self.query.hint_shortcuts(),
            _ => Vec::new(),
        }
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match self.active {
            DrawerView::Files => self.sidebar.handle_input(event, tx),
            DrawerView::Find => {
                // The Query panel speaks `handle_key`; non-key events are not
                // delivered to it.
                if let InputEvent::Key(key) = event {
                    self.query.handle_key(key, tx)
                } else {
                    EventState::NotConsumed
                }
            }
            _ => EventState::NotConsumed,
        }
    }

    pub fn handle_mouse(&mut self, event: &InputEvent, tx: &AppTx) {
        let InputEvent::Mouse(mouse) = event else {
            return;
        };
        match self.active {
            DrawerView::Files => {
                self.sidebar.handle_input(event, tx);
            }
            DrawerView::Find => {
                self.query.handle_mouse(mouse, tx);
            }
            _ => {}
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        match self.active {
            DrawerView::Files => self.sidebar.render(f, rect, theme, focused),
            DrawerView::Find => self.query.render(f, rect, theme, focused),
            view => {
                // Placeholder until the view's phase lands.
                let block = panel_block(view.label(), theme, focused);
                let inner = block.inner(rect);
                f.render_widget(block, rect);
                f.render_widget(
                    Paragraph::new(format!("{} — coming soon", view.label())).style(
                        Style::default()
                            .fg(theme.gray.to_ratatui())
                            .add_modifier(Modifier::ITALIC),
                    ),
                    inner,
                );
            }
        }
    }
}
