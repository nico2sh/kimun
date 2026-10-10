//! The **theme picker** (leader `v c`, under "+vault → config"): a small
//! modal listing every theme. Typing fuzzy-filters the list, like the command
//! palette; moving the selection previews the theme live, Enter persists, Esc
//! reverts to the theme that was active when the picker opened.

use std::sync::Arc;

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{ListItem, Paragraph};

use super::ModalShell;
use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx};
use crate::components::hint_row::HintRow;
use crate::components::rich_row::RichRow;
use crate::components::search_list::{
    Filter, KeyReaction, SearchList, SearchMouse, SearchRow, StaticRowSource,
};
use crate::settings::AppSettings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// One picker row: a theme's name and its index into the resolved theme list.
#[derive(Clone)]
struct ThemeRow {
    name: String,
    index: usize,
    /// The theme active when the picker opened.
    current: bool,
}

impl SearchRow for ThemeRow {
    fn to_list_item(&self, theme: &Theme, _icons: &Icons, _selected: bool) -> ListItem<'static> {
        let glyph = if self.current { "●" } else { " " };
        RichRow::new(glyph, self.name.clone())
            .glyph_style(Style::default().fg(theme.gray.to_ratatui()))
            .into_list_item(theme)
    }

    fn match_text(&self) -> Option<&str> {
        Some(&self.name)
    }

    fn visual_height(&self) -> u16 {
        1
    }
}

pub struct ThemePickerDialog {
    /// Themes in presentation order, fully resolved once on open — applying
    /// a selection never goes back to disk.
    themes: Vec<Theme>,
    list: SearchList<ThemeRow>,
    /// Index of the theme to restore when the picker is cancelled.
    original: usize,
    /// Index of the theme currently applied as a preview.
    previewed: usize,
    /// Dismiss-on-outside-press and hint-chip clicks (see [`ModalShell`]).
    shell: ModalShell,
    hints: HintRow,
}

impl ThemePickerDialog {
    pub fn new(settings: &AppSettings) -> Self {
        let themes = settings.theme_list();
        let current = settings.effective_theme_name();
        let original = themes.iter().position(|t| t.name == current).unwrap_or(0);
        let rows: Vec<ThemeRow> = themes
            .iter()
            .enumerate()
            .map(|(index, t)| ThemeRow {
                name: t.name.clone(),
                index,
                current: index == original,
            })
            .collect();
        // Static, in-memory rows: built synchronously (the redraw callback is
        // never fired on this path).
        let mut list = SearchList::builder(StaticRowSource, Arc::new(|| {}))
            .filter(Filter::Fuzzy)
            .build_with_rows(rows);
        list.select(original);
        Self {
            themes,
            list,
            original,
            previewed: original,
            shell: ModalShell::default(),
            hints: HintRow::new(&[
                (KeyCode::Null, "type", "Filter"),
                (KeyCode::Null, "↑↓", "Move"),
                (KeyCode::Enter, "⏎", "Apply"),
                (KeyCode::Esc, "Esc", "Cancel"),
            ])
            .with_indent(0),
        }
    }

    /// Click a theme to preview it, click the previewed one to keep it; the
    /// wheel steps through themes; a press outside reverts and closes, like
    /// Esc. Modal: every mouse event is consumed.
    pub fn handle_mouse(&mut self, m: &MouseEvent, tx: &AppTx) -> EventState {
        if let Some(key) = self.shell.pointer_key(m, &[&self.hints]) {
            return self.handle_key(key, tx);
        }
        // The wheel steps the selection (previewing each theme) rather than
        // scrolling the viewport, which would be a no-op on a short list.
        match m.kind {
            MouseEventKind::ScrollUp => return self.handle_key(KeyEvent::from(KeyCode::Up), tx),
            MouseEventKind::ScrollDown => {
                return self.handle_key(KeyEvent::from(KeyCode::Down), tx);
            }
            _ => {}
        }
        match self.list.handle_mouse(m) {
            SearchMouse::Activated(_) | SearchMouse::DoubleClicked { repeat: false, .. } => {
                return self.handle_key(KeyEvent::from(KeyCode::Enter), tx);
            }
            _ => self.sync_preview(tx),
        }
        EventState::Consumed
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

    /// Preview whichever theme the selection landed on, if it changed. With
    /// nothing selected (the filter matches no theme) fall back to the theme
    /// the picker opened on, so the screen never shows a theme that isn't
    /// highlighted.
    fn sync_preview(&mut self, tx: &AppTx) {
        let target = self.list.selected_row().map_or(self.original, |r| r.index);
        if target != self.previewed {
            self.previewed = target;
            self.apply(target, false, tx);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match self.list.handle_key(&key) {
            KeyReaction::Submit => {
                // Nothing matches the filter: stay open rather than commit a
                // theme the user can't see selected.
                if let Some(index) = self.list.selected_row().map(|r| r.index) {
                    self.apply(index, true, tx);
                    tx.send(AppEvent::CloseOverlay).ok();
                }
            }
            KeyReaction::Cancel => {
                if self.previewed != self.original {
                    self.apply(self.original, false, tx);
                }
                tx.send(AppEvent::CloseOverlay).ok();
            }
            _ => self.sync_preview(tx),
        }
        EventState::Consumed
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let width = 52u16.min(rect.width);
        // Border (2) + query row + hint row, plus one row per theme.
        let height = (self.themes.len() as u16 + 4)
            .min(rect.height.saturating_sub(4))
            .max(7);
        let area = super::fixed_centered_rect(width, height, rect);
        self.shell.set(area);
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

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(inner);

        f.render_widget(
            Paragraph::new("› ").style(Style::default().fg(theme.yellow.to_ratatui())),
            rows[0],
        );
        let input_rect = Rect {
            x: rows[0].x + 2,
            width: rows[0].width.saturating_sub(2),
            ..rows[0]
        };
        self.list.render_query(f, input_rect, theme, true);

        self.list.render(f, rows[1], theme, true);
        self.list.set_panel_rect(area);

        self.hints.render(
            f,
            rows[2],
            Style::default().fg(theme.gray.to_ratatui()),
            theme,
        );
    }
}

impl_dialog!(ThemePickerDialog);

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

    fn type_str(picker: &mut ThemePickerDialog, text: &str, tx: &AppTx) {
        for c in text.chars() {
            picker.handle_key(KeyEvent::from(KeyCode::Char(c)), tx);
        }
    }

    #[test]
    fn typing_filters_and_previews_the_match() {
        let settings = AppSettings::default();
        let mut picker = ThemePickerDialog::new(&settings);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        type_str(&mut picker, "flexoki lig", &tx);
        let visible: Vec<_> = picker
            .list
            .visible_rows()
            .iter()
            .map(|r| r.name.clone())
            .collect();
        assert!(
            visible.contains(&"Flexoki Light".to_string()),
            "{visible:?}"
        );
        assert!(visible.len() < picker.themes.len(), "list must be filtered");

        picker.handle_key(KeyEvent::from(KeyCode::Enter), &tx);
        let mut last = None;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::ApplyTheme { theme, persist } = ev {
                last = Some((theme.name, persist));
            }
        }
        assert_eq!(last, Some(("Flexoki Light".to_string(), true)));
    }

    #[test]
    fn no_match_reverts_preview_to_original() {
        let settings = AppSettings::default();
        let mut picker = ThemePickerDialog::new(&settings);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        type_str(&mut picker, "flex", &tx);
        type_str(&mut picker, "zzzz", &tx);
        let mut last = None;
        while let Ok(ev) = rx.try_recv() {
            if let AppEvent::ApplyTheme { theme, persist } = ev {
                last = Some((theme.name, persist));
            }
        }
        assert_eq!(
            last,
            Some((settings.effective_theme_name().to_string(), false))
        );
    }

    #[test]
    fn wheel_steps_the_selection() {
        use ratatui::crossterm::event::KeyModifiers;
        let settings = AppSettings::default();
        let mut picker = ThemePickerDialog::new(&settings);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        // Short filtered list: the viewport can't scroll, the selection must.
        type_str(&mut picker, "flexoki", &tx);
        while rx.try_recv().is_ok() {}
        let wheel = |kind| MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let before = picker.list.selected_row().map(|r| r.index);
        // One direction is blocked by the list end; the other must move.
        picker.handle_mouse(&wheel(MouseEventKind::ScrollDown), &tx);
        picker.handle_mouse(&wheel(MouseEventKind::ScrollUp), &tx);
        picker.handle_mouse(&wheel(MouseEventKind::ScrollUp), &tx);
        assert_ne!(picker.list.selected_row().map(|r| r.index), before);
        assert!(matches!(
            rx.try_recv(),
            Ok(AppEvent::ApplyTheme { persist: false, .. })
        ));
    }

    #[test]
    fn enter_with_no_match_stays_open() {
        let settings = AppSettings::default();
        let mut picker = ThemePickerDialog::new(&settings);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        type_str(&mut picker, "zzzzqqqq", &tx);
        picker.handle_key(KeyEvent::from(KeyCode::Enter), &tx);
        while let Ok(ev) = rx.try_recv() {
            assert!(!matches!(ev, AppEvent::CloseOverlay), "must not close");
            if let AppEvent::ApplyTheme { persist, .. } = ev {
                assert!(!persist, "must not persist anything");
            }
        }
    }
}
