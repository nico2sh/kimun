//! **QueryListPanel** — the one body shared by every list-shaped drawer view
//! (TAGS, LINKS, OUTLINE): an optional filter input over a [`SearchList`],
//! with submit / right-click behavior injected through [`ListPanelSpec`].
//!
//! The views vary only in their row type, what Enter does, and whether rows
//! are real notes (right-click context menu); everything about key routing,
//! mouse hit-testing, and layout is identical — so it lives exactly once
//! here, and a new drawer view is a spec + a source, not a copied panel.

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent, redraw_callback};
use crate::components::panel::panel_block;
use crate::components::search_list::{
    Filter, KeyReaction, RowSource, SearchList, SearchMouse, SearchRow,
};
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// What varies between list-shaped drawer views. The panel is the depth;
/// each view is a thin adapter of this seam.
pub trait ListPanelSpec {
    type Row: SearchRow + Clone + Send + Sync + 'static;

    /// Panel-block title.
    const TITLE: &'static str;
    /// Whether the top row is a typed filter input (`true`: every key goes
    /// to the list engine; `false`: only navigation keys reach the list —
    /// plain letters stay free for the host, e.g. LINKS' `b/o/u`).
    const HAS_FILTER: bool = true;

    /// What Enter / click-activate does with the selected row.
    fn submit(row: &Self::Row, tx: &AppTx);

    /// The event a right-click on a row fires (rows that are real notes
    /// open the file-ops menu). `None` = right-click selects only.
    fn context_event(_row: &Self::Row) -> Option<AppEvent> {
        None
    }

    fn hints() -> Vec<(String, String)>;
}

/// The shared panel body. Hosts that need extra chrome (LINKS' tab bar) draw
/// it themselves and hand the remaining body rect to [`Self::render_in`].
pub struct QueryListPanel<S: ListPanelSpec> {
    icons: Icons,
    list: Option<SearchList<S::Row>>,
}

impl<S: ListPanelSpec> QueryListPanel<S> {
    pub fn new(icons: Icons) -> Self {
        Self { icons, list: None }
    }

    /// (Re)build the list over a fresh source — the engine-per-context
    /// pattern every drawer view uses.
    pub fn set_source(&mut self, source: impl RowSource<S::Row> + 'static, tx: &AppTx) {
        let mut builder = SearchList::builder(source, redraw_callback(tx.clone()));
        if S::HAS_FILTER {
            builder = builder.filter(Filter::Fuzzy);
        }
        self.list = Some(builder.icons(self.icons.clone()).build());
    }

    pub fn is_loaded(&self) -> bool {
        self.list.is_some()
    }

    pub fn selected_row(&self) -> Option<&S::Row> {
        self.list.as_ref().and_then(|l| l.selected_row())
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        S::hints()
    }

    fn submit_selected(&self, tx: &AppTx) {
        if let Some(row) = self.selected_row() {
            S::submit(row, tx);
        }
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Key(key) => {
                let Some(list) = &mut self.list else {
                    return EventState::NotConsumed;
                };
                if S::HAS_FILTER {
                    match list.handle_key(key) {
                        KeyReaction::Submit => {
                            self.submit_selected(tx);
                            EventState::Consumed
                        }
                        KeyReaction::Consumed | KeyReaction::Cancel => EventState::Consumed,
                        KeyReaction::Intercepted(_) | KeyReaction::Unhandled => {
                            EventState::NotConsumed
                        }
                    }
                } else {
                    // No filter input: only navigation keys reach the list,
                    // so plain letters stay available to the host.
                    match key.code {
                        KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::PageUp
                        | KeyCode::PageDown
                        | KeyCode::Home
                        | KeyCode::End => {
                            list.handle_key(key);
                            EventState::Consumed
                        }
                        KeyCode::Enter => {
                            self.submit_selected(tx);
                            EventState::Consumed
                        }
                        _ => EventState::NotConsumed,
                    }
                }
            }
            InputEvent::Mouse(mouse) => {
                let Some(list) = &mut self.list else {
                    return EventState::NotConsumed;
                };
                match list.handle_mouse(mouse) {
                    SearchMouse::Activated(_) => self.submit_selected(tx),
                    SearchMouse::Context(_) => {
                        if let Some(event) = list.selected_row().and_then(S::context_event) {
                            tx.send(event).ok();
                        }
                    }
                    _ => {}
                }
                EventState::Consumed
            }
            _ => EventState::NotConsumed,
        }
    }

    /// Standard rendering: panel block + (filter input row) + list.
    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let block = panel_block(S::TITLE, theme, focused);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        self.render_in(f, inner, rect, theme, focused);
    }

    /// Render the body into `body` (a host that drew extra chrome — LINKS'
    /// tab bar — passes what remains). `panel` is the full panel rect, for
    /// wheel hit-testing.
    pub fn render_in(
        &mut self,
        f: &mut Frame,
        body: Rect,
        panel: Rect,
        theme: &Theme,
        focused: bool,
    ) {
        let Some(list) = &mut self.list else {
            return;
        };
        if S::HAS_FILTER {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(body);
            list.render_query(f, rows[0], theme, focused);
            list.render(f, rows[1], theme, focused);
            list.set_list_rect(rows[1]);
        } else {
            list.render(f, body, theme, focused);
            list.set_list_rect(body);
        }
        list.set_panel_rect(panel);
    }

    /// Test access to the underlying list.
    #[cfg(test)]
    pub(crate) fn list_mut(&mut self) -> Option<&mut SearchList<S::Row>> {
        self.list.as_mut()
    }

    #[cfg(test)]
    pub(crate) fn list(&self) -> Option<&SearchList<S::Row>> {
        self.list.as_ref()
    }
}
