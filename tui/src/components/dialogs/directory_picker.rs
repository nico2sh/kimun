//! A vault directory chooser for dialogs: a filter line over a fuzzy-ranked
//! list of every directory (the root included). The host dialog owns where
//! it sits and when it takes keys; the picker owns the loaded directories,
//! the filter text, the highlighted row and its hit-testing, and says what a
//! key or click meant ([`PickerEvent`]).
//!
//! The directories load off the UI thread (they come from walking the disk,
//! see [`file_ops::list_directories`]); the host starts that load and hands
//! the result to [`DirectoryPicker::set_directories`].

use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

use crate::components::clickable::list_index_at;
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::logic::file_ops;
use crate::settings::themes::Theme;

/// What a key or click asked of the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerEvent {
    /// Nothing for the host: the picker handled it (or it was not for it).
    None,
    /// A directory was chosen (Enter on a row, or a click on one).
    Chosen(VaultPath),
    /// Escape: leave the picker without choosing.
    Back,
}

pub struct DirectoryPicker {
    /// Every vault directory, once loaded.
    directories: Vec<VaultPath>,
    loaded: bool,
    /// Why the directories could not be listed.
    load_error: Option<String>,
    /// What the user typed to narrow the list.
    query: SingleLineInput,
    /// The rows shown: `directories` ranked against the query.
    shown: Vec<VaultPath>,
    list_state: ListState,
    /// The list's inner rect from the last render, where rows are hit.
    list_rect: Rect,
    /// Draw the filter and the list each in a bordered box, not bare.
    bordered: bool,
}

impl Default for DirectoryPicker {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectoryPicker {
    pub fn new() -> Self {
        Self {
            directories: Vec::new(),
            loaded: false,
            load_error: None,
            query: SingleLineInput::new(),
            shown: Vec::new(),
            list_state: ListState::default(),
            list_rect: Rect::default(),
            bordered: false,
        }
    }

    /// Draw the filter and the list each in a bordered box.
    pub fn bordered(mut self) -> Self {
        self.bordered = true;
        self
    }

    /// Highlight `dir` when it is shown.
    pub fn select(&mut self, dir: &VaultPath) {
        if let Some(i) = self.shown.iter().position(|d| d.is_like(dir)) {
            self.list_state.select(Some(i));
        }
    }

    /// The directories arrived (or could not be listed). The row already
    /// highlighted stays highlighted when the user got to choosing before the
    /// load finished; otherwise `current` is, when it is among them.
    pub fn set_directories(
        &mut self,
        directories: Result<Vec<VaultPath>, String>,
        current: &VaultPath,
    ) {
        self.loaded = true;
        match directories {
            Ok(directories) => {
                self.directories = directories;
                self.load_error = None;
            }
            Err(reason) => self.load_error = Some(reason),
        }
        let keep = self
            .highlighted()
            .cloned()
            .unwrap_or_else(|| current.clone());
        self.refilter(Some(&keep));
    }

    /// Start a choice afresh: no filter, `current` highlighted.
    pub fn reset(&mut self, current: &VaultPath) {
        self.query = SingleLineInput::new();
        self.refilter(Some(current));
    }

    /// The highlighted directory.
    pub fn highlighted(&self) -> Option<&VaultPath> {
        self.list_state.selected().and_then(|i| self.shown.get(i))
    }

    /// Rank `directories` against the filter and highlight `keep` when it
    /// is still shown (else the best match).
    fn refilter(&mut self, keep: Option<&VaultPath>) {
        self.shown = file_ops::filter_directories(&self.directories, self.query.value());
        let at = keep
            .and_then(|k| self.shown.iter().position(|d| d.is_like(k)))
            .or(if self.shown.is_empty() { None } else { Some(0) });
        self.list_state.select(at);
        if keep.is_none() {
            *self.list_state.offset_mut() = 0;
        }
    }

    fn move_highlight(&mut self, delta: isize) {
        if self.shown.is_empty() {
            return;
        }
        let last = self.shown.len() - 1;
        let from = self.list_state.selected().unwrap_or(0) as isize;
        let to = (from + delta).clamp(0, last as isize) as usize;
        self.list_state.select(Some(to));
    }

    /// A key while the picker has the keyboard.
    pub fn handle_key(&mut self, key: KeyEvent) -> PickerEvent {
        match key.code {
            KeyCode::Up => self.move_highlight(-1),
            KeyCode::Down => self.move_highlight(1),
            KeyCode::Enter => {
                return match self.highlighted() {
                    Some(dir) => PickerEvent::Chosen(dir.clone()),
                    None => PickerEvent::None,
                };
            }
            KeyCode::Esc => return PickerEvent::Back,
            KeyCode::Char(_) if !(key.modifiers - KeyModifiers::SHIFT).is_empty() => {}
            _ => {
                if self.query.handle_key(&key) == InputOutcome::Changed {
                    self.refilter(None);
                }
            }
        }
        PickerEvent::None
    }

    /// A click on a row chooses it; the wheel over the list moves the
    /// highlight.
    pub fn handle_mouse(&mut self, m: &MouseEvent) -> PickerEvent {
        let over_list = self.list_rect.contains(Position::new(m.column, m.row));
        match m.kind {
            MouseEventKind::ScrollUp if over_list => self.move_highlight(-1),
            MouseEventKind::ScrollDown if over_list => self.move_highlight(1),
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {}
            _ => {
                if let Some(i) = list_index_at(
                    m,
                    self.list_rect,
                    self.list_state.offset(),
                    self.shown.len(),
                ) {
                    return PickerEvent::Chosen(self.shown[i].clone());
                }
            }
        }
        PickerEvent::None
    }

    /// Whether `m` lands on the picker's list.
    pub fn list_contains(&self, m: &MouseEvent) -> bool {
        self.list_rect.contains(Position::new(m.column, m.row))
    }

    /// Draw the filter line and the list in `rect` (at least 2 rows, 6 when
    /// bordered).
    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme) {
        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let gray = theme.gray.to_ratatui();
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(if self.bordered { 3 } else { 1 }),
                Constraint::Min(1),
            ])
            .split(rect);
        let boxed = || {
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(gray))
                .style(Style::default().bg(bg))
        };

        let input_rect = if self.bordered {
            let block = boxed();
            let inner = block.inner(rows[0]);
            f.render_widget(block, rows[0]);
            inner
        } else {
            f.render_widget(
                Paragraph::new("  >").style(Style::default().fg(gray).bg(bg)),
                rows[0],
            );
            Rect {
                x: rows[0].x + 4,
                width: rows[0].width.saturating_sub(4),
                ..rows[0]
            }
        };
        self.query
            .render(f, input_rect, Style::default().fg(fg).bg(bg), 0, true);

        let list_block = self.bordered.then(boxed);
        self.list_rect = list_block
            .as_ref()
            .map_or(rows[1], |block| block.inner(rows[1]));
        let items: Vec<ListItem> = if self.shown.is_empty() {
            let text = match (&self.load_error, self.loaded) {
                (Some(reason), _) => format!("  (could not list directories: {reason})"),
                (None, true) => "  (no matches)".to_string(),
                (None, false) => "  (loading...)".to_string(),
            };
            vec![ListItem::new(text).style(Style::default().fg(gray).bg(bg))]
        } else {
            self.shown
                .iter()
                .map(|d| ListItem::new(directory_label(d)).style(Style::default().fg(fg).bg(bg)))
                .collect()
        };
        let mut list = List::new(items);
        if let Some(block) = list_block {
            list = list.block(block);
        }
        let list = list
            .highlight_style(
                Style::default()
                    .bg(theme.selection_bg.to_ratatui())
                    .fg(theme.selection_fg.to_ratatui())
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");
        f.render_stateful_widget(list, rows[1], &mut self.list_state);
    }
}

/// How a directory reads in the list.
pub fn directory_label(dir: &VaultPath) -> String {
    format!("  {}", file_ops::directory_name(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> VaultPath {
        VaultPath::new(s)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn loaded() -> DirectoryPicker {
        let mut picker = DirectoryPicker::new();
        picker.set_directories(
            Ok(vec![
                VaultPath::root(),
                p("/archive"),
                p("/projects"),
                p("/projects/rust"),
            ]),
            &p("/projects"),
        );
        picker
    }

    #[test]
    fn the_current_directory_starts_highlighted() {
        assert_eq!(loaded().highlighted(), Some(&p("/projects")));
    }

    #[test]
    fn a_directory_not_in_the_list_leaves_the_first_highlighted() {
        let mut picker = DirectoryPicker::new();
        picker.set_directories(Ok(vec![VaultPath::root(), p("/a")]), &p("/gone"));
        assert_eq!(picker.highlighted(), Some(&VaultPath::root()));
    }

    #[test]
    fn arrows_move_the_highlight_and_stop_at_the_ends() {
        let mut picker = loaded();
        picker.handle_key(key(KeyCode::Down));
        assert_eq!(picker.highlighted(), Some(&p("/projects/rust")));
        picker.handle_key(key(KeyCode::Down));
        assert_eq!(picker.highlighted(), Some(&p("/projects/rust")));
        for _ in 0..5 {
            picker.handle_key(key(KeyCode::Up));
        }
        assert_eq!(picker.highlighted(), Some(&VaultPath::root()));
    }

    #[test]
    fn typing_narrows_the_list_and_enter_chooses_the_best_match() {
        let mut picker = loaded();
        for c in "rust".chars() {
            assert_eq!(picker.handle_key(key(KeyCode::Char(c))), PickerEvent::None);
        }
        assert_eq!(picker.shown, vec![p("/projects/rust")]);
        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            PickerEvent::Chosen(p("/projects/rust"))
        );
    }

    #[test]
    fn escape_asks_to_go_back() {
        assert_eq!(loaded().handle_key(key(KeyCode::Esc)), PickerEvent::Back);
    }

    #[test]
    fn a_modified_char_is_not_typed() {
        let mut picker = loaded();
        let ctrl_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL);
        picker.handle_key(ctrl_k);
        assert_eq!(picker.shown.len(), 4, "no filter applied");
    }

    #[test]
    fn enter_with_nothing_shown_chooses_nothing() {
        let mut picker = loaded();
        for c in "zzzzqq".chars() {
            picker.handle_key(key(KeyCode::Char(c)));
        }
        assert!(picker.shown.is_empty());
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), PickerEvent::None);
    }

    #[test]
    fn reset_clears_the_filter_and_highlights_the_current_directory() {
        let mut picker = loaded();
        picker.handle_key(key(KeyCode::Char('a')));
        picker.reset(&p("/archive"));
        assert_eq!(picker.shown.len(), 4);
        assert_eq!(picker.highlighted(), Some(&p("/archive")));
    }

    #[test]
    fn a_click_on_a_row_chooses_it() {
        use ratatui::crossterm::event::MouseButton;
        let mut picker = loaded();
        picker.list_rect = Rect::new(0, 10, 30, 4);
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 12,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            picker.handle_mouse(&click),
            PickerEvent::Chosen(p("/projects"))
        );
    }

    /// The load finishing while the user is already choosing must not move
    /// the highlight they have arrowed to.
    #[test]
    fn a_late_load_keeps_the_highlight_the_user_moved_to() {
        let mut picker = DirectoryPicker::new();
        picker.set_directories(Ok(vec![VaultPath::root(), p("/a"), p("/b")]), &p("/a"));
        picker.handle_key(key(KeyCode::Down));
        assert_eq!(picker.highlighted(), Some(&p("/b")));
        // The same list arrives again (a late or repeated load).
        picker.set_directories(Ok(vec![VaultPath::root(), p("/a"), p("/b")]), &p("/a"));
        assert_eq!(picker.highlighted(), Some(&p("/b")));
    }

    #[test]
    fn a_failed_load_is_reported_and_chooses_nothing() {
        let mut picker = DirectoryPicker::new();
        picker.set_directories(Err("denied".to_string()), &VaultPath::root());
        assert!(picker.highlighted().is_none());
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), PickerEvent::None);
        assert_eq!(picker.load_error.as_deref(), Some("denied"));
    }

    #[test]
    fn labels_name_the_root() {
        assert_eq!(directory_label(&VaultPath::root()), "  / (vault root)");
        assert_eq!(directory_label(&p("/a/b")), "  /a/b");
    }
}
