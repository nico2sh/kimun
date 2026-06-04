use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::settings::themes::Theme;

/// What submitting the dialog will do with the current name field — drives
/// the live hint line so an overwrite is never silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveHint {
    /// The name matches the saved search the query came from (the breadcrumb
    /// provenance): submitting updates it in place.
    Update(String),
    /// The name matches a different existing saved search: submitting
    /// replaces that search's query. Rendered as a warning.
    Overwrite(String),
    /// A fresh name: submitting creates a new saved search.
    SaveNew,
    /// The name field is empty: submitting saves a new search named after the
    /// query itself (the query-as-name fallback).
    SaveNewAsQuery(String),
}

pub struct SaveSearchDialog {
    /// The query being saved (read-only context).
    pub query: String,
    /// User-supplied name for the saved search, pre-filled with the
    /// breadcrumb provenance when the query came from a saved search.
    name: SingleLineInput,
    /// The saved-search name the query came from (breadcrumb provenance).
    provenance: Option<String>,
    /// Existing saved-search names, loaded asynchronously after open (see
    /// [`AppEvent::SavedSearchNamesLoaded`]). Empty until then.
    existing: Vec<String>,
}

impl SaveSearchDialog {
    pub fn new(query: String, provenance: Option<String>) -> Self {
        let name = match &provenance {
            Some(p) => SingleLineInput::with_value(p.clone()),
            None => SingleLineInput::new(),
        };
        Self {
            query,
            name,
            provenance,
            existing: Vec::new(),
        }
    }

    /// Supply the vault's existing saved-search names (async load result).
    pub fn set_existing_names(&mut self, names: Vec<String>) {
        self.existing = names;
    }

    /// The name a submit would save under: the typed name, or the query
    /// itself when the field is empty (the query-as-name fallback).
    fn effective_name(&self) -> &str {
        let typed = self.name.value().trim();
        if typed.is_empty() { &self.query } else { typed }
    }

    /// What submitting right now would do. Name matching is ASCII
    /// case-insensitive — the same rule core's `save_search` applies.
    pub fn hint(&self) -> SaveHint {
        let effective = self.effective_name();
        if let Some(p) = &self.provenance
            && p.eq_ignore_ascii_case(effective)
        {
            return SaveHint::Update(p.clone());
        }
        if let Some(existing) = self
            .existing
            .iter()
            .find(|n| n.eq_ignore_ascii_case(effective))
        {
            return SaveHint::Overwrite(existing.clone());
        }
        if self.name.value().trim().is_empty() {
            SaveHint::SaveNewAsQuery(self.query.clone())
        } else {
            SaveHint::SaveNew
        }
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        let InputEvent::Key(key) = event else {
            return EventState::NotConsumed;
        };
        match self.name.handle_key(key) {
            InputOutcome::Submit => {
                tx.send(AppEvent::SaveSearchConfirmed {
                    name: self.effective_name().to_string(),
                    query: self.query.clone(),
                })
                .ok();
                tx.send(AppEvent::CloseOverlay).ok();
                EventState::Consumed
            }
            InputOutcome::Cancel => {
                tx.send(AppEvent::CloseOverlay).ok();
                EventState::Consumed
            }
            InputOutcome::Changed | InputOutcome::Consumed => EventState::Consumed,
            InputOutcome::NotConsumed => EventState::NotConsumed,
        }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let popup_area = super::fixed_centered_rect(62, 9, rect);

        f.render_widget(Clear, popup_area);

        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();
        let bg = theme.bg_panel.to_ratatui();

        let outer_block = Block::default()
            .title(" Save search ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(fg_muted))
            .style(theme.panel_style());
        let inner = outer_block.inner(popup_area);
        f.render_widget(outer_block, popup_area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 0: spacer
                Constraint::Length(1), // 1: query (read-only context)
                Constraint::Length(1), // 2: separator
                Constraint::Length(1), // 3: name input
                Constraint::Length(1), // 4: spacer
                Constraint::Length(1), // 5: hint
                Constraint::Min(0),    // 6: remainder
            ])
            .split(inner);

        // Row 1: read-only query context in muted style.
        f.render_widget(
            Paragraph::new(format!("  Query: {}", self.query))
                .style(Style::default().fg(fg_muted).bg(bg)),
            rows[1],
        );

        super::render_separator(f, rows[2], fg_muted, bg);

        // Row 3: name input with a "Name: " prefix.
        let prefix = "  Name: ";
        let prefix_len = prefix.len() as u16;
        f.render_widget(
            Paragraph::new(prefix).style(Style::default().fg(fg_muted).bg(bg)),
            rows[3],
        );
        self.name
            .render(f, rows[3], Style::default().fg(fg).bg(bg), prefix_len, true);

        // Row 5: live hint — what Enter will do with the current name.
        let (action, warn) = match self.hint() {
            SaveHint::Update(name) => (format!("Update '{name}'"), false),
            SaveHint::Overwrite(name) => (format!("Overwrite '{name}'"), true),
            SaveHint::SaveNew => ("Save new".to_string(), false),
            SaveHint::SaveNewAsQuery(query) => (format!("Save new: '{query}'"), false),
        };
        let hint_fg = if warn {
            ratatui::style::Color::Yellow
        } else {
            fg_muted
        };
        f.render_widget(
            Paragraph::new(format!("  [Enter] {action}   [Esc] Cancel"))
                .style(Style::default().fg(hint_fg).bg(bg)),
            rows[5],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::{AppEvent, InputEvent};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tokio::sync::mpsc::unbounded_channel;

    fn key(code: KeyCode) -> InputEvent {
        InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// Drain the channel and return the `SaveSearchConfirmed` payload, if any.
    fn confirmed(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    ) -> Option<(String, String)> {
        let mut found = None;
        while let Ok(e) = rx.try_recv() {
            if let AppEvent::SaveSearchConfirmed { name, query } = e {
                found = Some((name, query));
            }
        }
        found
    }

    #[test]
    fn submit_emits_save_event_with_typed_name() {
        let mut d = SaveSearchDialog::new("<{note}".to_string(), None);
        let (tx, mut rx) = unbounded_channel();
        for ch in ['l', 'i', 'n', 'k', 's'] {
            d.handle_input(&key(KeyCode::Char(ch)), &tx);
        }
        d.handle_input(&key(KeyCode::Enter), &tx);
        let (name, query) = confirmed(&mut rx).expect("SaveSearchConfirmed emitted");
        assert_eq!(name, "links");
        assert_eq!(query, "<{note}");
    }

    #[test]
    fn submit_empty_name_falls_back_to_query() {
        let mut d = SaveSearchDialog::new("#todo".to_string(), None);
        let (tx, mut rx) = unbounded_channel();
        d.handle_input(&key(KeyCode::Enter), &tx);
        let (name, query) = confirmed(&mut rx).expect("emitted");
        assert_eq!(name, "#todo"); // empty → query used as name
        assert_eq!(query, "#todo");
    }

    #[test]
    fn provenance_prefills_name_so_plain_enter_updates() {
        let mut d = SaveSearchDialog::new("#todo and #urgent".to_string(), Some("todo".into()));
        let (tx, mut rx) = unbounded_channel();
        d.handle_input(&key(KeyCode::Enter), &tx);
        let (name, query) = confirmed(&mut rx).expect("emitted");
        assert_eq!(name, "todo"); // provenance pre-filled, untouched
        assert_eq!(query, "#todo and #urgent");
    }

    #[test]
    fn hint_updates_when_name_matches_provenance() {
        let d = SaveSearchDialog::new("#todo".to_string(), Some("todo".into()));
        assert_eq!(d.hint(), SaveHint::Update("todo".into()));
    }

    #[test]
    fn hint_matches_provenance_case_insensitively() {
        let mut d = SaveSearchDialog::new("#todo".to_string(), None);
        d.set_existing_names(vec!["Todo".into()]);
        let (tx, _rx) = unbounded_channel();
        for ch in ['t', 'O', 'd', 'O'] {
            d.handle_input(&key(KeyCode::Char(ch)), &tx);
        }
        // Same rule core uses on save: ASCII case-insensitive name match.
        assert_eq!(d.hint(), SaveHint::Overwrite("Todo".into()));
    }

    #[test]
    fn hint_overwrites_when_name_matches_another_existing_search() {
        let mut d = SaveSearchDialog::new("#todo".to_string(), Some("todo".into()));
        d.set_existing_names(vec!["todo".into(), "other".into()]);
        let (tx, _rx) = unbounded_channel();
        // Clear the pre-filled "todo" and type "other".
        for _ in 0..4 {
            d.handle_input(&key(KeyCode::Backspace), &tx);
        }
        for ch in ['o', 't', 'h', 'e', 'r'] {
            d.handle_input(&key(KeyCode::Char(ch)), &tx);
        }
        assert_eq!(d.hint(), SaveHint::Overwrite("other".into()));
    }

    #[test]
    fn hint_saves_new_for_a_fresh_name() {
        let mut d = SaveSearchDialog::new("#todo".to_string(), None);
        d.set_existing_names(vec!["other".into()]);
        let (tx, _rx) = unbounded_channel();
        for ch in ['f', 'r', 'e', 's', 'h'] {
            d.handle_input(&key(KeyCode::Char(ch)), &tx);
        }
        assert_eq!(d.hint(), SaveHint::SaveNew);
    }

    #[test]
    fn hint_empty_name_shows_query_as_name_fallback() {
        let d = SaveSearchDialog::new("#todo".to_string(), None);
        assert_eq!(d.hint(), SaveHint::SaveNewAsQuery("#todo".into()));
    }

    #[test]
    fn hint_empty_name_with_colliding_query_warns_overwrite() {
        let mut d = SaveSearchDialog::new("#todo".to_string(), None);
        // A saved search literally named "#todo" exists; the query-as-name
        // fallback would overwrite it.
        d.set_existing_names(vec!["#todo".into()]);
        assert_eq!(d.hint(), SaveHint::Overwrite("#todo".into()));
    }
}
