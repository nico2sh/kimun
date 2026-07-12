//! The RAG answer overlay (P4 2b): a modal that asks the RAG server a question
//! and shows the LLM answer plus its cited source notes (each openable). Opened
//! by a keybinding; only useful when a server is configured + reachable.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, InputEvent};
use crate::components::overlay::{Overlay, OverlayKind, OverlayMsg};
use crate::components::semantic_search::rag_client;
use crate::rag::{RagAnswer, RagSource};
use crate::settings::SharedSettings;
use crate::settings::themes::Theme;

enum State {
    /// Typing the question.
    Prompt,
    /// Waiting on the server.
    Loading,
    /// Answer received.
    Answered(RagAnswer),
    /// The ask failed.
    Error(String),
}

/// Process-global ask counter, so request ids are unique across overlay
/// instances (a stale answer from a closed overlay never matches a new one).
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub struct RagAnswerOverlay {
    vault: Arc<NoteVault>,
    settings: SharedSettings,
    prompt: String,
    state: State,
    /// Selected source row (in the Answered state).
    selected: usize,
    /// Id of the in-flight ask; results with a different id are ignored. `0`
    /// (never issued) before the first ask.
    request_id: u64,
}

impl RagAnswerOverlay {
    pub fn new(vault: Arc<NoteVault>, settings: SharedSettings) -> Self {
        Self {
            vault,
            settings,
            prompt: String::new(),
            state: State::Prompt,
            selected: 0,
            request_id: 0,
        }
    }

    /// Spawns the ask job; the result comes back via `AppEvent::RagAnswerReady`.
    fn submit(&mut self, tx: &AppTx) {
        let query = self.prompt.trim().to_string();
        if query.is_empty() {
            return;
        }
        self.state = State::Loading;
        let request_id = NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        self.request_id = request_id;
        let vault = self.vault.clone();
        let settings = self.settings.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let result = match rag_client(&settings, &vault).await {
                Some(client) => client
                    .ask(&query, None)
                    .await
                    .map(|answer| RagAnswer {
                        answer: answer.answer,
                        sources: answer
                            .sources
                            .into_iter()
                            .map(|s| RagSource {
                                path: VaultPath::new(&s.path),
                                title: s.title,
                            })
                            .collect(),
                    })
                    .map_err(|e| e.to_string()),
                None => Err("No RAG server configured".to_string()),
            };
            let _ = tx.send(AppEvent::RagAnswerReady { request_id, result });
        });
    }

    fn sources(&self) -> &[RagSource] {
        match &self.state {
            State::Answered(a) => &a.sources,
            _ => &[],
        }
    }

    fn handle_key(&mut self, key: &KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Esc => {
                tx.send(AppEvent::CloseOverlay).ok();
                EventState::Consumed
            }
            _ => match self.state {
                State::Prompt => self.handle_prompt_key(key, tx),
                State::Answered(_) => self.handle_answered_key(key, tx),
                // Loading / Error: only Esc (above) does anything.
                _ => EventState::Consumed,
            },
        }
    }

    fn handle_prompt_key(&mut self, key: &KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Enter => self.submit(tx),
            KeyCode::Backspace => {
                self.prompt.pop();
            }
            KeyCode::Char(c) => self.prompt.push(c),
            _ => {}
        }
        EventState::Consumed
    }

    fn handle_answered_key(&mut self, key: &KeyEvent, tx: &AppTx) -> EventState {
        let count = self.sources().len();
        match key.code {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                if count > 0 {
                    self.selected = (self.selected + 1).min(count - 1);
                }
            }
            KeyCode::Enter => {
                if let Some(src) = self.sources().get(self.selected) {
                    tx.send(AppEvent::open(src.path.clone())).ok();
                    tx.send(AppEvent::CloseOverlay).ok();
                }
            }
            _ => {}
        }
        EventState::Consumed
    }
}

impl Overlay for RagAnswerOverlay {
    fn kind(&self) -> OverlayKind {
        OverlayKind::RagAnswer
    }

    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        match event {
            InputEvent::Key(key) => self.handle_key(key, tx),
            _ => EventState::Consumed, // modal: swallow mouse/other so it stays put
        }
    }

    fn handle_app_message(
        &mut self,
        msg: &AppEvent,
        _vault: &Arc<NoteVault>,
        _tx: &AppTx,
    ) -> OverlayMsg {
        if let AppEvent::RagAnswerReady { request_id, result } = msg {
            // Ignore a late answer from a superseded / closed ask.
            if *request_id != self.request_id {
                return OverlayMsg::NotConsumed;
            }
            self.selected = 0;
            self.state = match result {
                Ok(answer) => State::Answered(answer.clone()),
                Err(e) => State::Error(e.clone()),
            };
            OverlayMsg::Consumed
        } else {
            OverlayMsg::NotConsumed
        }
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        match self.state {
            State::Prompt => vec![
                ("Enter".into(), "Ask".into()),
                ("Esc".into(), "Close".into()),
            ],
            State::Answered(_) => vec![
                ("↑↓".into(), "Sources".into()),
                ("Enter".into(), "Open".into()),
                ("Esc".into(), "Close".into()),
            ],
            _ => vec![("Esc".into(), "Close".into())],
        }
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let modal = crate::components::centered_rect(80, 70, area);
        f.render_widget(Clear, modal);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Ask (RAG) ")
            .border_style(Style::default().fg(theme.accent.to_ratatui()));
        let inner = block.inner(modal);
        f.render_widget(block, modal);

        let fg = Style::default().fg(theme.fg.to_ratatui());
        let muted = Style::default().fg(theme.gray.to_ratatui());

        // Prompt row on top, body below.
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(inner);

        let prompt_line = Line::from(vec![
            Span::styled("? ", Style::default().fg(theme.accent.to_ratatui())),
            Span::styled(self.prompt.clone(), fg),
            Span::styled(
                if matches!(self.state, State::Prompt) {
                    "▏"
                } else {
                    ""
                },
                Style::default().fg(theme.accent.to_ratatui()),
            ),
        ]);
        f.render_widget(Paragraph::new(prompt_line), rows[0]);

        match &self.state {
            State::Prompt => {
                f.render_widget(
                    Paragraph::new("Type a question, then Enter.").style(muted),
                    rows[1],
                );
            }
            State::Loading => {
                f.render_widget(Paragraph::new("Thinking…").style(muted), rows[1]);
            }
            State::Error(e) => {
                f.render_widget(
                    Paragraph::new(format!("Error: {e}"))
                        .style(Style::default().fg(theme.red.to_ratatui()))
                        .wrap(Wrap { trim: true }),
                    rows[1],
                );
            }
            State::Answered(answer) => {
                let body = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(3), Constraint::Length(self.source_rows())])
                    .split(rows[1]);
                f.render_widget(
                    Paragraph::new(answer.answer.clone())
                        .style(fg)
                        .wrap(Wrap { trim: true }),
                    body[0],
                );
                let items: Vec<ListItem> = answer
                    .sources
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let selected = i == self.selected;
                        let style = if selected {
                            Style::default()
                                .fg(theme.accent.to_ratatui())
                                .add_modifier(Modifier::BOLD)
                        } else {
                            muted
                        };
                        let marker = if selected { "› " } else { "  " };
                        ListItem::new(Line::from(vec![
                            Span::styled(marker, style),
                            Span::styled(format!("{} — {}", s.title, s.path), style),
                        ]))
                    })
                    .collect();
                f.render_widget(List::new(items), body[1]);
            }
        }
    }
}

impl RagAnswerOverlay {
    /// Height budget for the sources list (capped so the answer keeps room).
    fn source_rows(&self) -> u16 {
        (self.sources().len() as u16).min(8)
    }
}
