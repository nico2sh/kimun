//! The small menu behind FILES' `[+]` chip: what can be made here, a note
//! or a directory, each with its own chord when bound. Picking one
//! hands over to the new-entry dialog on the same directory (the menu gives
//! way to it, as the file operations menu does).

use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Paragraph;

use super::ModalShell;
use crate::components::Component;
use crate::components::clickable::key_at;
use crate::components::event_state::EventState;
use crate::components::hint_row::HintRow;
use crate::components::panel::{ModalSpec, modal_chrome};
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::ActionShortcuts;
use crate::logic::message::{AppMessage, AppTx, FileOp};
use crate::settings::themes::Theme;

pub struct NewMenuDialog {
    /// The directory the new entry would go into.
    base: VaultPath,
    /// `"  in {base}"` for rendering.
    base_display: String,
    /// The chords that make each directly, as bound now (`None`: unbound).
    note_chord: Option<String>,
    directory_chord: Option<String>,
    shell: ModalShell,
    /// Each action's cell from the last render, with its key.
    action_rects: Vec<(Rect, KeyCode)>,
    hints: HintRow,
}

impl NewMenuDialog {
    pub fn new(base: VaultPath, key_bindings: &KeyBindings) -> Self {
        Self {
            base_display: format!("  in {base}"),
            base,
            note_chord: key_bindings.first_combo_for(&ActionShortcuts::NewNote),
            directory_chord: key_bindings.first_combo_for(&ActionShortcuts::NewDirectory),
            shell: ModalShell::default(),
            action_rects: Vec::new(),
            hints: HintRow::new(&[(KeyCode::Esc, "Esc", "Cancel")]),
        }
    }

    /// Clicking an action runs its key; a press outside the menu cancels it.
    /// Modal: every mouse event is consumed.
    pub fn handle_mouse(&mut self, m: &MouseEvent, tx: &AppTx) -> EventState {
        if let Some(key) = self.shell.pointer_key(m, &[&self.hints]) {
            return self.handle_key(key, tx);
        }
        if let Some(key) = key_at(&self.action_rects, m) {
            return self.handle_key(key, tx);
        }
        EventState::Consumed
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        // A letter with Ctrl or Alt held is a chord (the very ones this menu
        // shows), not the menu's key: `Alt+N` is not `N`.
        if !(key.modifiers - KeyModifiers::SHIFT).is_empty() {
            return EventState::Consumed;
        }
        match key.code {
            KeyCode::Char('n') | KeyCode::Char('N') => {
                tx.send(AppMessage::FileOp(FileOp::ShowNewNote {
                    base: self.base.clone(),
                    initial: None,
                }))
                .ok();
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                tx.send(AppMessage::FileOp(FileOp::ShowNewDirectory {
                    base: self.base.clone(),
                }))
                .ok();
            }
            KeyCode::Esc => {
                tx.send(AppMessage::CloseOverlay).ok();
            }
            // Swallow the rest while the menu is open.
            _ => {}
        }
        EventState::Consumed
    }
}

fn width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

impl Component for NewMenuDialog {
    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        let popup_area = super::fixed_centered_rect(56, 9, rect);
        self.shell.set(popup_area);
        let inner = modal_chrome(
            f,
            popup_area,
            theme,
            ModalSpec {
                title: Some(" New "),
                border: Some(Style::default().fg(theme.fg.to_ratatui())),
                ..Default::default()
            },
        );

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // 0: spacer
                Constraint::Length(1), // 1: where
                Constraint::Length(1), // 2: separator
                Constraint::Length(1), // 3: actions
                Constraint::Length(1), // 4: spacer
                Constraint::Length(1), // 5: hint
                Constraint::Min(0),
            ])
            .split(inner);

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let gray = theme.gray.to_ratatui();
        super::render_path_row(f, rows[1], &self.base_display, fg, bg);
        super::render_separator(f, rows[2], gray, bg);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
            .split(rows[3]);
        let key_style = Style::default()
            .bg(bg)
            .patch(theme.action())
            .add_modifier(Modifier::BOLD);
        let label_style = Style::default().fg(fg).bg(bg);
        let hint_style = Style::default().fg(gray).bg(bg);

        self.action_rects.clear();
        let actions = [
            (
                "[N]",
                " Note",
                self.note_chord.as_deref(),
                KeyCode::Char('n'),
            ),
            (
                "[D]",
                " Directory",
                self.directory_chord.as_deref(),
                KeyCode::Char('d'),
            ),
        ];
        for (col, (key, label, chord, code)) in cols.iter().zip(actions) {
            // The drawn text, chord included, is the target: what is drawn
            // is what clicks.
            let chord_w = chord.map_or(0, |c| width(c) + 2);
            let text_w = (width(key) + width(label) + chord_w) as u16;
            let r = Rect::new(col.x + 1, col.y, text_w.min(col.width.saturating_sub(1)), 1);
            crate::components::clickable::register(r);
            self.action_rects.push((r, code));
            let line = ratatui::text::Line::from(vec![
                ratatui::text::Span::raw(" "),
                ratatui::text::Span::styled(key, key_style),
                ratatui::text::Span::styled(label, label_style),
                ratatui::text::Span::styled(
                    chord.map(|c| format!("  {c}")).unwrap_or_default(),
                    hint_style,
                ),
            ]);
            f.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), *col);
        }

        self.hints
            .render(f, rows[5], Style::default().fg(gray).bg(bg), theme);
    }
}

impl_dialog!(NewMenuDialog);

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;
    use tokio::sync::mpsc;

    fn menu() -> (NewMenuDialog, AppTx, mpsc::UnboundedReceiver<AppMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let bindings = crate::settings::AppSettings::default().key_bindings.clone();
        (
            NewMenuDialog::new(VaultPath::new("/notes"), &bindings),
            tx,
            rx,
        )
    }

    fn press(d: &mut NewMenuDialog, c: KeyCode, tx: &AppTx) -> EventState {
        d.handle_key(KeyEvent::new(c, KeyModifiers::NONE), tx)
    }

    #[test]
    fn n_asks_for_a_new_note_on_the_menus_directory() {
        let (mut d, tx, mut rx) = menu();
        assert_eq!(press(&mut d, KeyCode::Char('n'), &tx), EventState::Consumed);
        assert!(matches!(
            rx.try_recv(),
            Ok(AppMessage::FileOp(FileOp::ShowNewNote { base, initial: None }))
                if base == VaultPath::new("/notes")
        ));
    }

    #[test]
    fn d_asks_for_a_new_directory_on_the_menus_directory() {
        let (mut d, tx, mut rx) = menu();
        press(&mut d, KeyCode::Char('D'), &tx);
        assert!(matches!(
            rx.try_recv(),
            Ok(AppMessage::FileOp(FileOp::ShowNewDirectory { base }))
                if base == VaultPath::new("/notes")
        ));
    }

    #[test]
    fn escape_closes_and_other_keys_are_swallowed() {
        let (mut d, tx, mut rx) = menu();
        assert_eq!(press(&mut d, KeyCode::Char('x'), &tx), EventState::Consumed);
        assert!(rx.try_recv().is_err());
        press(&mut d, KeyCode::Esc, &tx);
        assert!(matches!(rx.try_recv(), Ok(AppMessage::CloseOverlay)));
    }

    /// The chords the menu shows are not its keys: Alt+N must not open the
    /// note dialog, Ctrl+D must not open the directory one.
    #[test]
    fn a_chord_is_not_the_menus_letter() {
        let (mut d, tx, mut rx) = menu();
        for (code, mods) in [
            (KeyCode::Char('n'), KeyModifiers::ALT),
            (KeyCode::Char('n'), KeyModifiers::CONTROL),
            (KeyCode::Char('d'), KeyModifiers::CONTROL),
        ] {
            let state = d.handle_key(KeyEvent::new(code, mods), &tx);
            assert_eq!(state, EventState::Consumed);
        }
        assert!(rx.try_recv().is_err(), "nothing opened");
        // A shifted letter still counts.
        d.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT), &tx);
        assert!(matches!(
            rx.try_recv(),
            Ok(AppMessage::FileOp(FileOp::ShowNewNote { .. }))
        ));
    }

    #[test]
    fn the_directory_chord_is_unbound_by_default() {
        let (d, _tx, _rx) = menu();
        assert_eq!(d.note_chord, None);
        assert_eq!(d.directory_chord, None);
    }
}
