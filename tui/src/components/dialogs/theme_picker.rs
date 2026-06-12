//! The **theme picker** (leader `v c`, spec §8c "+vault → config"): a small
//! modal listing every theme; moving the selection previews it live, Enter
//! persists, Esc reverts to the theme that was active when the picker opened.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

pub struct ThemePickerDialog {
    /// Themes in presentation order, fully resolved once on open — applying
    /// a selection never goes back to disk.
    themes: Vec<Theme>,
    selected: usize,
    /// Index of the theme to restore when the picker is cancelled.
    original: usize,
    /// Scroll offset for long lists.
    offset: usize,
}

impl ThemePickerDialog {
    pub fn new(settings: &AppSettings) -> Self {
        let themes = settings.theme_list();
        let current = settings.effective_theme_name();
        let selected = themes.iter().position(|t| t.name == current).unwrap_or(0);
        Self {
            themes,
            selected,
            original: selected,
            offset: 0,
        }
    }

    fn apply(&self, index: usize, persist: bool, tx: &AppTx) {
        if let Some(theme) = self.themes.get(index) {
            tx.send(AppEvent::ApplyTheme {
                theme: Box::new(theme.clone()),
                persist,
            })
            .ok();
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                let prev = self.selected;
                self.selected = self.selected.saturating_sub(1);
                if self.selected != prev {
                    self.apply(self.selected, false, tx);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let prev = self.selected;
                self.selected = (self.selected + 1).min(self.themes.len().saturating_sub(1));
                if self.selected != prev {
                    self.apply(self.selected, false, tx);
                }
            }
            KeyCode::Enter => {
                self.apply(self.selected, true, tx);
                tx.send(AppEvent::CloseOverlay).ok();
            }
            KeyCode::Esc => {
                if self.selected != self.original {
                    self.apply(self.original, false, tx);
                }
                tx.send(AppEvent::CloseOverlay).ok();
            }
            _ => {}
        }
        EventState::Consumed
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let width = 40u16.min(rect.width);
        let height = (self.themes.len() as u16 + 2)
            .min(rect.height.saturating_sub(4))
            .max(5);
        let area = super::fixed_centered_rect(width, height, rect);
        let inner = crate::components::panel::modal_chrome(
            f,
            area,
            theme,
            crate::components::panel::ModalSpec {
                title: Some("─ Theme "),
                border: Some(theme.border_style(focused)),
                bg: crate::components::panel::ModalBg::Base,
            },
        );

        // Keep the selection in the visible window.
        let visible = inner.height as usize;
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if visible > 0 && self.selected >= self.offset + visible {
            self.offset = self.selected + 1 - visible;
        }

        for (row, (i, entry)) in self
            .themes
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(visible)
            .enumerate()
        {
            let name = &entry.name;
            let style = if i == self.selected {
                Style::default()
                    .fg(theme.selection_fg.to_ratatui())
                    .bg(theme.selection_bg.to_ratatui())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg.to_ratatui())
            };
            let marker = if i == self.selected { "› " } else { "  " };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(format!("{marker}{name}"), style))),
                Rect::new(inner.x, inner.y + row as u16, inner.width, 1),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_applies_persisted_and_closes() {
        let settings = AppSettings::default();
        let mut picker = ThemePickerDialog::new(&settings);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        picker.handle_key(
            KeyEvent::new(KeyCode::Down, ratatui::crossterm::event::KeyModifiers::NONE),
            &tx,
        );
        picker.handle_key(
            KeyEvent::new(
                KeyCode::Enter,
                ratatui::crossterm::event::KeyModifiers::NONE,
            ),
            &tx,
        );

        let mut applied = Vec::new();
        let mut closed = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                AppEvent::ApplyTheme { theme, persist } => applied.push((theme.name, persist)),
                AppEvent::CloseOverlay => closed = true,
                _ => {}
            }
        }
        // Down previews (persist=false), Enter commits (persist=true).
        assert_eq!(applied.len(), 2);
        assert!(!applied[0].1);
        assert!(applied[1].1);
        // Both carry the SAME resolved theme (the moved-to selection).
        assert_eq!(applied[0].0, applied[1].0);
        assert!(closed);
    }

    #[test]
    fn esc_reverts_to_original() {
        let mut settings = AppSettings::default();
        settings.theme = "Gruvbox Dark".to_string();
        let mut picker = ThemePickerDialog::new(&settings);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        picker.handle_key(
            KeyEvent::new(KeyCode::Down, ratatui::crossterm::event::KeyModifiers::NONE),
            &tx,
        );
        picker.handle_key(
            KeyEvent::new(KeyCode::Esc, ratatui::crossterm::event::KeyModifiers::NONE),
            &tx,
        );

        let mut last_applied = None;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::ApplyTheme { theme, persist } = ev {
                last_applied = Some((theme.name, persist));
            }
        }
        assert_eq!(
            last_applied,
            Some(("Gruvbox Dark".to_string(), false)),
            "Esc must re-apply the original theme"
        );
    }
}
