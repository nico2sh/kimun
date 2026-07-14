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
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

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

    /// When `true`, the filter is drawn as a bordered search box (like the note
    /// finder) with a right-aligned result/loading status, and the results area
    /// shows a dimmed "Searching…/No results" message. Suits server-backed views
    /// (semantic search) where the query round-trips over the network and the
    /// input deserves visual separation from the results. Default `false`: a
    /// bare one-row filter for the compact local drawer views (TAGS/LINKS/
    /// OUTLINE), whose behavior is unchanged.
    const BORDERED_INPUT: bool = false;

    /// Whether the typed query ALSO fuzzy-filters the loaded rows locally
    /// (`true`, default). Set `false` for server-backed sources whose `load`
    /// already applies the query (semantic search): the server returns ranked,
    /// conceptually-relevant notes that rarely contain the query words verbatim,
    /// so a local fuzzy pass over their titles would wrongly discard nearly all
    /// of them. `false` keeps the input (it drives the server reload) but shows
    /// every returned row in server rank order.
    const LOCAL_FILTER: bool = true;

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
            // A server-backed source (LOCAL_FILTER = false) already applied the
            // query in `load`; keep its ranked rows as-is (SourceOrder) instead of
            // fuzzy-filtering them again by the literal query text.
            builder = builder.filter(if S::LOCAL_FILTER {
                Filter::Fuzzy
            } else {
                Filter::SourceOrder
            });
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
        if S::HAS_FILTER && S::BORDERED_INPUT {
            // Bordered search box (Length 3) clearly separated from the results,
            // reusing the note finder's layout. A right-aligned status doubles as
            // the progress indicator: "Searching…" while a query is in flight,
            // otherwise the result count.
            // Drain any completed load NOW. `SearchList::render` is what normally
            // polls, but the placeholder path below renders a message instead of
            // the list — without this the loader never drains, so `is_loading`
            // sticks true forever ("Searching…" that never resolves) and typed
            // results never land.
            list.poll();

            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(0)])
                .split(body);

            let loading = list.is_loading();
            let count = list.match_count();
            let status = if loading {
                "Searching…".to_string()
            } else {
                format!("{count} results")
            };
            let dim = Style::default().fg(theme.gray.to_ratatui());
            let search_block = Block::default()
                .title(" Search ")
                .title(Line::from(Span::styled(format!(" {status} "), dim)).right_aligned())
                .borders(Borders::ALL)
                .border_style(theme.border_style(focused));
            let search_inner = search_block.inner(rows[0]);
            f.render_widget(search_block, rows[0]);
            list.render_query(f, search_inner, theme, focused);

            // Results: show a dimmed placeholder while a query is in flight with
            // nothing yet on screen, or when a completed query found nothing;
            // otherwise the (possibly stale) results stay visible.
            let query_empty = list.query().trim().is_empty();
            if count == 0 && (loading || !query_empty) {
                let msg = if loading {
                    "Searching…"
                } else {
                    "No results"
                };
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(msg, dim))).alignment(Alignment::Center),
                    rows[1],
                );
            } else {
                list.render(f, rows[1], theme, focused);
            }
            list.set_list_rect(rows[1]);
        } else if S::HAS_FILTER {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::search_list::{Emit, SearchRow};
    use crate::settings::themes::Theme;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tokio::sync::mpsc::unbounded_channel;

    #[derive(Clone)]
    struct Row(String);
    impl SearchRow for Row {
        fn to_list_item(
            &self,
            _t: &Theme,
            _i: &Icons,
            _s: bool,
        ) -> ratatui::widgets::ListItem<'static> {
            ratatui::widgets::ListItem::new(self.0.clone())
        }
        fn visual_height(&self) -> u16 {
            1
        }
        fn match_text(&self) -> Option<&str> {
            Some(&self.0)
        }
    }

    /// A source that completes immediately with no rows — the "query found
    /// nothing" case.
    struct EmptySource;
    #[async_trait::async_trait]
    impl RowSource<Row> for EmptySource {
        async fn load(&self, _q: &str, emit: Emit<Row>) {
            emit.replace(Vec::new());
        }
    }

    /// A source whose load never resolves — stays `is_loading` forever, the
    /// "query in flight" case.
    struct PendingSource;
    #[async_trait::async_trait]
    impl RowSource<Row> for PendingSource {
        async fn load(&self, _q: &str, _emit: Emit<Row>) {
            std::future::pending::<()>().await;
        }
    }

    struct BorderedSpec;
    impl ListPanelSpec for BorderedSpec {
        type Row = Row;
        const TITLE: &'static str = "Semantic";
        const BORDERED_INPUT: bool = true;
        fn submit(_row: &Row, _tx: &AppTx) {}
        fn hints() -> Vec<(String, String)> {
            Vec::new()
        }
    }

    /// A server-backed source: returns the same three rows for any query (the
    /// server did the ranking; the query is not a local substring filter).
    struct ThreeSource;
    #[async_trait::async_trait]
    impl RowSource<Row> for ThreeSource {
        async fn load(&self, _q: &str, emit: Emit<Row>) {
            emit.replace(vec![
                Row("alpha".into()),
                Row("beta".into()),
                Row("gamma".into()),
            ]);
        }
    }

    /// Like `BorderedSpec` but opts out of local filtering (server-backed).
    struct NoFilterSpec;
    impl ListPanelSpec for NoFilterSpec {
        type Row = Row;
        const TITLE: &'static str = "Semantic";
        const BORDERED_INPUT: bool = true;
        const LOCAL_FILTER: bool = false;
        fn submit(_row: &Row, _tx: &AppTx) {}
        fn hints() -> Vec<(String, String)> {
            Vec::new()
        }
    }

    fn buffer_text<S: ListPanelSpec>(panel: &mut QueryListPanel<S>) -> String {
        let theme = Theme::default();
        let mut term = Terminal::new(TestBackend::new(40, 12)).unwrap();
        term.draw(|f| panel.render(f, Rect::new(0, 0, 40, 12), &theme, true))
            .unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Regression: a server-backed view (`LOCAL_FILTER = false`) must NOT drop
    /// the server's ranked rows just because their titles don't contain the
    /// typed query. This was the "semantic search shows one result" bug — the
    /// local fuzzy filter discarded every conceptually-relevant note whose title
    /// lacked the query words.
    #[tokio::test]
    async fn no_local_filter_keeps_server_rows_that_dont_match_query() {
        let (tx, _rx) = unbounded_channel();
        let mut panel = QueryListPanel::<NoFilterSpec>::new(Icons::new(false));
        panel.set_source(ThreeSource, &tx);
        {
            let list = panel.list_mut().unwrap();
            list.poll_until_idle().await;
            // A query that matches none of the row titles verbatim.
            list.set_query("zzz-not-in-any-title");
            list.poll_until_idle().await;
        }
        assert_eq!(
            panel.list().unwrap().match_count(),
            3,
            "server rows must survive a non-matching query (no local filter)"
        );
        let text = buffer_text(&mut panel);
        assert!(
            text.contains("alpha") && text.contains("beta") && text.contains("gamma"),
            "all server rows shown:\n{text}"
        );
    }

    /// Contrast: a local-filter view (`LOCAL_FILTER = true`, the drawer default)
    /// DOES narrow rows by the typed query — the behavior semantic search must
    /// avoid but TAGS/LINKS/OUTLINE rely on.
    #[tokio::test]
    async fn local_filter_narrows_rows_by_query() {
        let (tx, _rx) = unbounded_channel();
        let mut panel = QueryListPanel::<BorderedSpec>::new(Icons::new(false));
        panel.set_source(ThreeSource, &tx);
        {
            let list = panel.list_mut().unwrap();
            list.poll_until_idle().await;
            list.set_query("alpha");
            list.poll_until_idle().await;
        }
        assert_eq!(
            panel.list().unwrap().match_count(),
            1,
            "local fuzzy filter keeps only the matching row"
        );
    }

    #[tokio::test]
    async fn bordered_input_shows_searching_indicator_while_in_flight() {
        let (tx, _rx) = unbounded_channel();
        let mut panel = QueryListPanel::<BorderedSpec>::new(Icons::new(false));
        panel.set_source(PendingSource, &tx);
        // The initial load is pending → is_loading stays true.
        let text = buffer_text(&mut panel);
        assert!(text.contains("Search"), "bordered search box:\n{text}");
        assert!(text.contains("Searching"), "in-flight indicator:\n{text}");
    }

    /// Regression: the placeholder path must still drain the loader. Render is
    /// the only thing that polls; if it renders a message *instead of* the list
    /// without polling, `is_loading` sticks true forever and typed results never
    /// land. Here the load completes (empty) off-thread; a single `render` — with
    /// NO manual `poll_until_idle` — must clear loading and show "No results".
    #[tokio::test]
    async fn render_drains_loader_in_placeholder_path() {
        let (tx, _rx) = unbounded_channel();
        let mut panel = QueryListPanel::<BorderedSpec>::new(Icons::new(false));
        panel.set_source(EmptySource, &tx);
        panel.list_mut().unwrap().set_query("x"); // starts a load (reload_on_query)
        // Let the spawned load run and land on the channel — but do NOT poll it
        // in ourselves; render must be what drains it.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        let text = buffer_text(&mut panel); // render → must poll
        assert!(
            !panel.list().unwrap().is_loading(),
            "render must drain the loader; is_loading stuck:\n{text}"
        );
        assert!(text.contains("No results"), "resolved to empty:\n{text}");
        assert!(
            !text.contains("Searching"),
            "must not be stuck searching:\n{text}"
        );
    }

    #[tokio::test]
    async fn bordered_input_shows_no_results_for_empty_completed_query() {
        let (tx, _rx) = unbounded_channel();
        let mut panel = QueryListPanel::<BorderedSpec>::new(Icons::new(false));
        panel.set_source(EmptySource, &tx);
        {
            let list = panel.list_mut().unwrap();
            list.poll_until_idle().await; // drain the initial empty-query load
            list.set_query("nothing-matches");
            list.poll_until_idle().await;
        }
        let text = buffer_text(&mut panel);
        assert!(text.contains("Search"), "bordered search box:\n{text}");
        assert!(text.contains("No results"), "empty-result message:\n{text}");
    }
}
