//! The **Drawer** — the single panel between the activity rail and the
//! editor. It renders whichever rail view is active: the file browser
//! (FILES), the Query panel (FIND), or a placeholder for the views that land
//! in later phases (TAGS, LINKS, OUTLINE, CFG).

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::Component;
use crate::components::ask_sources::SourcesPanel;
use crate::components::config_panel::ConfigPanel;
use crate::components::drawer_views::{LinksPanel, OutlinePanel, TagsPanel};
use crate::components::event_state::EventState;
use crate::components::events::{AppTx, InputEvent};
use crate::components::query_panel::QueryPanel;
use crate::components::semantic_search::SemanticPanel;
use crate::components::sidebar::SidebarComponent;
use crate::settings::themes::Theme;
use kimun_core::NoteVault;

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

pub use crate::components::config_panel::ConfigInfo;

/// Hosts the drawer views. FILES and FIND are the ported existing panels
/// (file browser and Query panel); TAGS, LINKS, and OUTLINE are the
/// phase-03 panels; CFG is a placeholder until the settings drawer lands.
pub struct DrawerHost {
    active: DrawerView,
    sidebar: SidebarComponent,
    query: QueryPanel,
    semantic: SemanticPanel,
    /// The Ask workspace's Sources drawer view. Its editor-area companion
    /// (`ThreadPanel`) lives in `PanelSet`; this face lists the selected
    /// turn's sources and flips to the source reader (adr/0030).
    ask_sources: SourcesPanel,
    tags: TagsPanel,
    links: LinksPanel,
    outline: OutlinePanel,
    config: ConfigPanel,
}

impl DrawerHost {
    pub fn new(
        vault: std::sync::Arc<NoteVault>,
        sidebar: SidebarComponent,
        query: QueryPanel,
        semantic: SemanticPanel,
        tags: TagsPanel,
        links: LinksPanel,
        outline: OutlinePanel,
    ) -> Self {
        Self {
            active: DrawerView::Files,
            sidebar,
            query,
            semantic,
            // The Sources view owns the vault handle for its reader load.
            ask_sources: SourcesPanel::new(vault),
            tags,
            links,
            outline,
            config: ConfigPanel::default(),
        }
    }

    /// Refresh what the CFG view shows (called when the view opens).
    pub fn set_config_info(&mut self, info: ConfigInfo) {
        self.config.set_info(info);
    }

    pub fn active_view(&self) -> DrawerView {
        self.active
    }

    /// Whether the active view is a text-input context (drives the status
    /// bar's ⌨/≣ indicator). The surface owns this knowledge: FIND hosts a
    /// query input; the list views are filter-as-you-type lists, which read
    /// as lists (spec mockup shows them with ≣).
    pub fn is_text_input(&self) -> bool {
        // ASK's drawer face is the Sources *list* (the question composer lives
        // in the editor area), so it reads as a list, not a text input.
        matches!(self.active, DrawerView::Find | DrawerView::Semantic)
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
    pub fn semantic_mut(&mut self) -> &mut SemanticPanel {
        &mut self.semantic
    }
    pub fn ask_sources_mut(&mut self) -> &mut SourcesPanel {
        &mut self.ask_sources
    }
    pub fn tags_mut(&mut self) -> &mut TagsPanel {
        &mut self.tags
    }
    pub fn links_mut(&mut self) -> &mut LinksPanel {
        &mut self.links
    }
    pub fn outline_mut(&mut self) -> &mut OutlinePanel {
        &mut self.outline
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        match self.active {
            DrawerView::Files => self.sidebar.hint_shortcuts(),
            DrawerView::Find => self.query.hint_shortcuts(),
            DrawerView::Semantic => self.semantic.hint_shortcuts(),
            DrawerView::Ask => self.ask_sources.hint_shortcuts(),
            DrawerView::Tags => self.tags.hint_shortcuts(),
            DrawerView::Links => self.links.hint_shortcuts(),
            DrawerView::Outline => self.outline.hint_shortcuts(),
            DrawerView::Config => self.config.hint_shortcuts(),
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
            DrawerView::Semantic => self.semantic.handle_input(event, tx),
            DrawerView::Ask => self.ask_sources.handle_input(event, tx),
            DrawerView::Tags => self.tags.handle_input(event, tx),
            DrawerView::Links => self.links.handle_input(event, tx),
            DrawerView::Outline => self.outline.handle_input(event, tx),
            DrawerView::Config => self.config.handle_input(event, tx),
        }
    }

    pub fn handle_mouse(&mut self, event: &InputEvent, tx: &AppTx) {
        let InputEvent::Mouse(mouse) = event else {
            return;
        };
        match self.active {
            // The Query panel has a dedicated mouse entry point; every other
            // view takes mouse events through its regular input path.
            DrawerView::Find => {
                self.query.handle_mouse(mouse, tx);
            }
            _ => {
                self.handle_input(event, tx);
            }
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        match self.active {
            DrawerView::Files => self.sidebar.render(f, rect, theme, focused),
            DrawerView::Find => self.query.render(f, rect, theme, focused),
            DrawerView::Semantic => self.semantic.render(f, rect, theme, focused),
            DrawerView::Ask => self.ask_sources.render(f, rect, theme, focused),
            DrawerView::Tags => self.tags.render(f, rect, theme, focused),
            DrawerView::Links => self.links.render(f, rect, theme, focused),
            DrawerView::Outline => self.outline.render(f, rect, theme, focused),
            DrawerView::Config => self.config.render(f, rect, theme, focused),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::AppEvent;
    use crate::settings::AppSettings;
    use crate::test_support::temp_vault;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    async fn make_host() -> DrawerHost {
        let vault = temp_vault("drawerhost").await;
        vault.validate_and_init().await.unwrap();
        let settings = AppSettings::default();
        let sidebar = crate::components::sidebar::SidebarComponent::new(
            settings.key_bindings.clone(),
            vault.clone(),
            settings.icons(),
            &settings,
        );
        let query = crate::components::query_panel::QueryPanel::new(
            vault.clone(),
            settings.key_bindings.clone(),
            settings.icons(),
        );
        let semantic = crate::components::semantic_search::SemanticPanel::new(
            vault.clone(),
            std::sync::Arc::new(std::sync::RwLock::new(settings.clone())),
            settings.icons(),
        );
        let tags = crate::components::drawer_views::TagsPanel::new(vault.clone(), settings.icons());
        let links =
            crate::components::drawer_views::LinksPanel::new(vault.clone(), settings.icons());
        let outline =
            crate::components::drawer_views::OutlinePanel::new(vault.clone(), settings.icons());
        DrawerHost::new(vault, sidebar, query, semantic, tags, links, outline)
    }

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// FIND's key-only rule: the Query panel speaks `handle_key`, so
    /// non-key events must pass through unconsumed rather than reach it.
    #[tokio::test]
    async fn find_view_delivers_keys_only() {
        let mut host = make_host().await;
        let (tx, _rx) = unbounded_channel();
        host.set_view(DrawerView::Find);
        assert_eq!(
            host.handle_input(&InputEvent::Paste("hi".into()), &tx),
            EventState::NotConsumed,
            "a paste is not a key; FIND must not consume it"
        );
    }

    /// The CFG arm delegates to the ConfigPanel component (adr/0023: no
    /// inline view logic in the host) — its launcher keys work through the
    /// host's dispatch.
    #[tokio::test]
    async fn config_view_routes_to_the_config_panel() {
        let mut host = make_host().await;
        let (tx, mut rx) = unbounded_channel();
        host.set_view(DrawerView::Config);
        assert_eq!(
            host.handle_input(&key(KeyCode::Char('t')), &tx),
            EventState::Consumed
        );
        assert!(matches!(
            rx.try_recv(),
            Ok(AppEvent::ExecuteLeaderAction(
                crate::keys::leader::LeaderAction::VaultTheme
            ))
        ));
        assert_eq!(
            host.handle_input(&key(KeyCode::Char('x')), &tx),
            EventState::NotConsumed
        );
    }

    /// Hints follow the active view — the routing seam every view shares.
    #[tokio::test]
    async fn hints_follow_the_active_view() {
        let mut host = make_host().await;
        host.set_view(DrawerView::Config);
        let cfg_hints = host.hint_shortcuts();
        assert!(cfg_hints.iter().any(|(_, h)| h == "Theme picker"));
        host.set_view(DrawerView::Files);
        assert_ne!(host.hint_shortcuts(), cfg_hints);
    }

    /// The status bar's ⌨/≣ indicator: only the query-input views read as
    /// text input.
    #[tokio::test]
    async fn text_input_views_are_find_and_semantic() {
        let mut host = make_host().await;
        for (view, expect) in [
            (DrawerView::Files, false),
            (DrawerView::Find, true),
            (DrawerView::Semantic, true),
            (DrawerView::Ask, false),
            (DrawerView::Tags, false),
            (DrawerView::Links, false),
            (DrawerView::Outline, false),
            (DrawerView::Config, false),
        ] {
            host.set_view(view);
            assert_eq!(host.is_text_input(), expect, "{view:?}");
        }
    }
}
