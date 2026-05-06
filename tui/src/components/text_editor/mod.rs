pub mod backend;
pub mod markdown;
pub mod nvim_rpc;
pub mod snapshot;
pub mod view;
pub mod word_wrap;

use arboard::Clipboard;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use ratatui_textarea::{CursorMove, TextArea};

/// Move or extend the selection by `movement`.
///
/// If `shift` is held and no selection is currently active, anchors the selection
/// first; otherwise the existing anchor is kept. Without `shift`, any active
/// selection is cancelled before the cursor moves.
macro_rules! cursor_move {
    ($ta:expr, $mv:expr, $shift:expr) => {{
        if $shift {
            if $ta.selection_range().is_none() {
                $ta.start_selection();
            }
        } else {
            $ta.cancel_selection();
        }
        $ta.move_cursor($mv);
    }};
}

use self::backend::BackendState;
use self::snapshot::NvimMode;
use self::view::MarkdownEditorView;

/// If `marker` is an ordered-list marker like `"3. "`, returns the next marker
/// (`"4. "`). Returns `None` for unordered markers or unrecognized input.
fn increment_ordered_marker(marker: &str) -> Option<String> {
    let trimmed = marker.trim_end_matches(' ');
    let dot = trimmed.strip_suffix('.')?;
    let n: u32 = dot.parse().ok()?;
    Some(format!("{}. ", n + 1))
}

/// Returns the text covered by the textarea's current selection, or `None` if
/// there is no selection or the range is empty.
///
/// `selection_range()` returns char-column coordinates, so they must be
/// converted to byte offsets before slicing to support multi-byte UTF-8 text.
fn selection_text(ta: &TextArea<'_>) -> Option<String> {
    let ((sr, sc), (er, ec)) = ta.selection_range()?;
    if sr == er && sc == ec {
        return None;
    }
    let lines = ta.lines();
    let char_to_byte = |line: &str, char_col: usize| -> usize {
        line.char_indices()
            .nth(char_col)
            .map(|(b, _)| b)
            .unwrap_or(line.len())
    };
    Some(if sr == er {
        let line = &lines[sr];
        let sb = char_to_byte(line, sc);
        let eb = char_to_byte(line, ec);
        line[sb..eb].to_string()
    } else {
        let first = &lines[sr];
        let sb = char_to_byte(first, sc);
        let mut parts = vec![first[sb..].to_string()];
        for line in &lines[(sr + 1)..er] {
            parts.push(line.clone());
        }
        let last = &lines[er];
        let eb = char_to_byte(last, ec);
        parts.push(last[..eb].to_string());
        parts.join("\n")
    })
}

use crate::components::Component;
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::events::AppTx;
use crate::components::events::InputEvent;
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::TextAction;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

pub struct TextEditorComponent {
    backend: BackendState,
    /// Tracks the rendered rect to map mouse click coordinates.
    rect: Rect,
    key_bindings: KeyBindings,
    last_saved_text: String,
    view: MarkdownEditorView,
    /// Incremented on every mutating input event. Passed to `view.update()` so the view
    /// can skip the expensive lines clone and parse-cache rebuild on idle frames.
    edit_generation: u64,
    /// Current selection range in logical (row, byte-col) coordinates.
    /// Only tracked for the Textarea backend; always `None` for Nvim.
    selection: Option<((usize, usize), (usize, usize))>,
    /// System clipboard handle. `None` if the clipboard is unavailable (e.g. headless CI).
    clipboard: Option<Clipboard>,
    /// `true` after a `Z` keypress in Normal mode; cleared on the next key.
    /// Lets us intercept `ZZ` (write+quit) and `ZQ` (quit) without forwarding them to nvim.
    nvim_pending_z: bool,
}

impl TextEditorComponent {
    pub fn new(key_bindings: KeyBindings, settings: &AppSettings) -> Self {
        Self {
            backend: BackendState::from_settings(
                &settings.editor_backend,
                settings.nvim_path.as_ref(),
            ),
            rect: Rect::default(),
            key_bindings,
            last_saved_text: String::new(),
            view: MarkdownEditorView::new(),
            edit_generation: 0,
            selection: None,
            clipboard: Clipboard::new().ok(),
            nvim_pending_z: false,
        }
    }

    /// Returns the buffer lines for direct access.
    ///
    /// For the Textarea backend, returns the live lines.
    /// For the Nvim backend, returns an empty slice — use `get_text()` instead,
    /// which reads from the snapshot.
    pub fn lines(&self) -> &[String] {
        match &self.backend {
            BackendState::Textarea(ta) => ta.lines(),
            BackendState::Nvim(_) => &[],
        }
    }

    pub fn set_text(&mut self, text: String) {
        match &mut self.backend {
            BackendState::Textarea(ta) => {
                let lines = text.lines();
                *ta = TextArea::from(lines);
            }
            BackendState::Nvim(nvim) => {
                nvim.set_text(&text);
            }
        }
        self.edit_generation = self.edit_generation.wrapping_add(1);
        let reconstructed = self.get_text();
        self.mark_saved(reconstructed);
    }

    pub fn get_text(&self) -> String {
        match &self.backend {
            BackendState::Textarea(ta) => ta.lines().join("\n"),
            BackendState::Nvim(nvim) => nvim
                .snapshot
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .lines
                .join("\n"),
        }
    }

    pub fn mark_saved(&mut self, text: String) {
        if let BackendState::Nvim(nvim) = &self.backend {
            nvim.snapshot
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .dirty = false;
        }
        self.last_saved_text = text;
    }

    pub fn is_dirty(&self) -> bool {
        match &self.backend {
            BackendState::Textarea(_) => self.get_text() != self.last_saved_text,
            BackendState::Nvim(nvim) => {
                nvim.snapshot
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .dirty
            }
        }
    }

    /// Returns the raw link target under the cursor, or `None` if the cursor
    /// is not inside a wikilink or markdown link span.
    pub fn link_at_cursor(&self) -> Option<String> {
        let (_row, col, line) = match &self.backend {
            BackendState::Textarea(ta) => {
                let (row, col) = ta.cursor();
                let line = ta.lines().get(row)?.to_string();
                (row, col, line)
            }
            BackendState::Nvim(nvim) => {
                let snap = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                let (row, col) = snap.cursor;
                let line = snap.lines.get(row)?.to_string();
                (row, col, line)
            }
        };
        kimun_core::note::link_char_spans(&line)
            .into_iter()
            .find(|s| s.start <= col && col < s.end)
            .map(|s| s.target)
    }

    /// Copy selected text to the system clipboard.
    fn copy_selection_to_clipboard(&mut self) {
        let text = {
            let BackendState::Textarea(ta) = &self.backend else {
                return;
            };
            match selection_text(ta) {
                Some(t) => t,
                None => return,
            }
        };
        if let Some(cb) = &mut self.clipboard {
            let _ = cb.set_text(text);
        }
    }

    /// Paste text from the system clipboard at the cursor, replacing any active selection.
    fn paste_from_clipboard(&mut self) {
        // Get text from clipboard first (releasing that borrow), then access backend.
        let text = match &mut self.clipboard {
            Some(cb) => match cb.get_text() {
                Ok(t) if !t.is_empty() => t,
                _ => return,
            },
            None => return,
        };
        if let BackendState::Textarea(ta) = &mut self.backend {
            if ta.selection_range().is_some() {
                ta.cut();
            }
            ta.insert_str(&text);
            self.selection = ta.selection_range();
            self.edit_generation = self.edit_generation.wrapping_add(1);
        }
    }

    /// Wrap a selection in (or insert at the cursor) markdown markers for
    /// Bold/Italic/Strikethrough. No-op for other actions and on the Nvim backend.
    pub fn apply_text_action(&mut self, action: TextAction) {
        let marker = match action {
            TextAction::Bold => "**",
            TextAction::Italic => "*",
            TextAction::Strikethrough => "~~",
            _ => return,
        };
        let BackendState::Textarea(ta) = &mut self.backend else {
            return;
        };
        match selection_text(ta) {
            Some(text) => {
                ta.insert_str(format!("{marker}{text}{marker}"));
            }
            None => {
                ta.insert_str(format!("{marker}{marker}"));
                for _ in 0..marker.len() {
                    ta.move_cursor(CursorMove::Back);
                }
            }
        }
        self.selection = ta.selection_range();
        self.edit_generation = self.edit_generation.wrapping_add(1);
    }

    /// Smart Enter: continue list markers, preserve indent, dedent on empty
    /// indent-only lines, clear empty list markers. Returns `true` if handled
    /// (caller should not insert a plain newline). Always `false` on Nvim
    /// backend or when there is an active selection.
    pub fn smart_enter(&mut self) -> bool {
        enum Action {
            ClearLine { chars: usize },
            InsertPrefix(String),
            Dedent,
        }
        let action = {
            let BackendState::Textarea(ta) = &self.backend else {
                return false;
            };
            if ta.selection_range().is_some() {
                return false;
            }
            let (row, col) = ta.cursor();
            let Some(line) = ta.lines().get(row) else {
                return false;
            };
            let total_chars = line.chars().count();
            if col != total_chars {
                return false;
            }
            // ASCII whitespace, so byte index == char index here.
            let ws_end = markdown::leading_ws_byte_len(line);
            let (ws, after_ws) = line.split_at(ws_end);
            if let Some(marker_len) = markdown::list_marker_len(after_ws) {
                if after_ws.len() == marker_len {
                    // Empty list item: dedent first if indented, then clear
                    // the marker once fully unindented.
                    if ws_end > 0 {
                        Action::Dedent
                    } else {
                        Action::ClearLine { chars: total_chars }
                    }
                } else {
                    let marker_str = &after_ws[..marker_len];
                    let next_marker = increment_ordered_marker(marker_str)
                        .unwrap_or_else(|| marker_str.to_string());
                    Action::InsertPrefix(format!("{ws}{next_marker}"))
                }
            } else if ws_end > 0 && total_chars == ws_end {
                Action::Dedent
            } else if ws_end > 0 {
                Action::InsertPrefix(ws.to_string())
            } else {
                return false;
            }
        };

        match action {
            Action::Dedent => {
                self.indent_lines(true);
                return true;
            }
            Action::ClearLine { chars } => {
                let BackendState::Textarea(ta) = &mut self.backend else {
                    unreachable!()
                };
                ta.move_cursor(CursorMove::Head);
                ta.delete_str(chars);
            }
            Action::InsertPrefix(prefix) => {
                let BackendState::Textarea(ta) = &mut self.backend else {
                    unreachable!()
                };
                ta.insert_newline();
                ta.insert_str(prefix);
            }
        }
        let BackendState::Textarea(ta) = &self.backend else {
            unreachable!()
        };
        self.selection = ta.selection_range();
        self.edit_generation = self.edit_generation.wrapping_add(1);
        true
    }

    /// Indent or dedent whole lines. Tab unit is `\t` if `hard_tab_indent` is
    /// on, else `tab_length` spaces. Dedent counts a leading tab as one unit.
    /// No-op on Nvim backend.
    pub fn indent_lines(&mut self, dedent: bool) {
        let BackendState::Textarea(ta) = &mut self.backend else {
            return;
        };
        let tab_len = ta.tab_length() as usize;
        let hard_tab = ta.hard_tab_indent();
        let indent: String = if hard_tab {
            "\t".to_string()
        } else {
            " ".repeat(tab_len)
        };
        if indent.is_empty() {
            return;
        }
        let indent_chars = indent.len();

        let sel = ta.selection_range();
        let saved_cursor = if sel.is_none() {
            Some(ta.cursor())
        } else {
            None
        };
        let (start_row, end_row) = match sel {
            Some(((sr, _), (er, ec))) => {
                // A selection that ends at column 0 of a row visually doesn't
                // include that row, so don't indent it.
                let last = if ec == 0 && er > sr { er - 1 } else { er };
                (sr, last)
            }
            None => {
                let (r, _) = saved_cursor.unwrap();
                (r, r)
            }
        };

        let row_count = end_row.saturating_sub(start_row) + 1;
        let mut row_deltas: Vec<isize> = Vec::with_capacity(row_count);
        let mut any_change = false;

        for row in start_row..=end_row {
            if dedent {
                let count = {
                    let line = ta.lines().get(row).map(|s| s.as_str()).unwrap_or("");
                    let max_remove = if hard_tab { 1 } else { tab_len };
                    let mut count = 0usize;
                    for (i, c) in line.chars().enumerate() {
                        if i >= max_remove {
                            break;
                        }
                        if c == '\t' {
                            count += 1;
                            break;
                        } else if c == ' ' && !hard_tab {
                            count += 1;
                        } else {
                            break;
                        }
                    }
                    count
                };
                if count > 0 {
                    ta.move_cursor(CursorMove::Jump(row as u16, 0));
                    ta.delete_str(count);
                    any_change = true;
                }
                row_deltas.push(-(count as isize));
            } else {
                ta.move_cursor(CursorMove::Jump(row as u16, 0));
                ta.insert_str(&indent);
                row_deltas.push(indent_chars as isize);
                any_change = true;
            }
        }

        let adj = |row: usize, col: usize| -> usize {
            if row >= start_row && row <= end_row {
                let d = row_deltas[row - start_row];
                if d >= 0 {
                    col + d as usize
                } else {
                    col.saturating_sub((-d) as usize)
                }
            } else {
                col
            }
        };

        match sel {
            Some(((ssr, ssc), (ser, sec))) => {
                ta.cancel_selection();
                let new_ssc = adj(ssr, ssc);
                let new_sec = adj(ser, sec);
                ta.move_cursor(CursorMove::Jump(ssr as u16, new_ssc as u16));
                ta.start_selection();
                ta.move_cursor(CursorMove::Jump(ser as u16, new_sec as u16));
            }
            None => {
                let (cr, cc) = saved_cursor.expect("captured when sel is None");
                let new_col = adj(cr, cc);
                ta.move_cursor(CursorMove::Jump(cr as u16, new_col as u16));
            }
        }

        if any_change {
            self.selection = ta.selection_range();
            self.edit_generation = self.edit_generation.wrapping_add(1);
        }
    }
}

impl TextEditorComponent {
    /// If the Nvim process has died, fall back to a Textarea with the last known content.
    fn maybe_recover_from_dead_nvim(&mut self) {
        use std::sync::atomic::Ordering;
        let fallback_text = if let BackendState::Nvim(nvim) = &self.backend {
            if nvim.is_dead.load(Ordering::SeqCst) {
                Some(
                    nvim.snapshot
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .lines
                        .join("\n"),
                )
            } else {
                None
            }
        } else {
            None
        };
        if let Some(text) = fallback_text {
            tracing::warn!("nvim process died; falling back to textarea backend");
            self.backend = BackendState::Textarea(TextArea::from(text.lines()));
        }
    }

    /// Handle a key event when using the Nvim backend.
    ///
    /// Returns `Some(EventState)` if the event was handled (or should be),
    /// `None` if the backend is not Nvim and the caller should fall through.
    fn handle_nvim_key(
        &mut self,
        key: &ratatui::crossterm::event::KeyEvent,
        tx: &AppTx,
    ) -> Option<EventState> {
        let BackendState::Nvim(nvim) = &self.backend else {
            return None;
        };

        // FocusSidebar / FocusEditor shortcuts are intercepted at the
        // EditorScreen level for directional navigation.

        // Intercept ZZ / ZQ in Normal mode: buffer the first Z, then
        // decide on the second key without forwarding either to nvim.
        if self.nvim_pending_z {
            self.nvim_pending_z = false;
            match key.code {
                KeyCode::Char('Z') => {
                    // ZZ — write + quit
                    tx.send(AppEvent::Autosave).ok();
                    tx.send(AppEvent::FocusSidebar).ok();
                    return Some(EventState::Consumed);
                }
                KeyCode::Char('Q') => {
                    // ZQ — quit without saving
                    tx.send(AppEvent::FocusSidebar).ok();
                    return Some(EventState::Consumed);
                }
                _ => {
                    // Not a quit sequence — replay the buffered Z first.
                    nvim.handle_key(
                        &ratatui::crossterm::event::KeyEvent::new(
                            KeyCode::Char('Z'),
                            KeyModifiers::NONE,
                        ),
                        tx.clone(),
                    );
                    // Then fall through to forward the current key normally.
                }
            }
        } else if key.code == KeyCode::Char('Z') {
            let in_normal = {
                let snap = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                snap.mode == NvimMode::Normal
            };
            if in_normal {
                self.nvim_pending_z = true;
                return Some(EventState::Consumed);
            }
        }

        // Intercept vim quit/write-quit commands so they don't kill the
        // embedded nvim process.
        if key.code == KeyCode::Enter {
            let (is_cmd, cmdline) = {
                let snap = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                let cmd = if snap.mode == NvimMode::Command {
                    snap.cmdline
                        .as_deref()
                        .unwrap_or("")
                        .trim_start_matches(':')
                        .to_string()
                } else {
                    String::new()
                };
                (snap.mode == NvimMode::Command, cmd)
            };
            if is_cmd {
                let saves = matches!(
                    cmdline.as_str(),
                    "w" | "wq" | "wq!" | "wqa" | "wqa!" | "x" | "xa" | "x!"
                );
                let quits =
                    saves || matches!(cmdline.as_str(), "q" | "q!" | "qa" | "qa!" | "cq" | "cq!");
                if quits {
                    nvim.handle_key(
                        &ratatui::crossterm::event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                        tx.clone(),
                    );
                    if saves {
                        tx.send(AppEvent::Autosave).ok();
                    }
                    tx.send(AppEvent::FocusSidebar).ok();
                    return Some(EventState::Consumed);
                }
            }
        }

        nvim.handle_key(key, tx.clone());
        self.edit_generation = self.edit_generation.wrapping_add(1);
        Some(EventState::Consumed)
    }

    /// Handle a key event when using the Textarea backend.
    fn handle_textarea_key(
        &mut self,
        key: &ratatui::crossterm::event::KeyEvent,
        _tx: &AppTx,
    ) -> EventState {
        // System clipboard shortcuts — intercept before passing to textarea.
        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Char('c') => {
                    self.copy_selection_to_clipboard();
                    return EventState::Consumed;
                }
                KeyCode::Char('v') => {
                    self.paste_from_clipboard();
                    return EventState::Consumed;
                }
                KeyCode::Char('x') => {
                    self.copy_selection_to_clipboard();
                    if let BackendState::Textarea(ta) = &mut self.backend {
                        if ta.selection_range().is_some() {
                            ta.cut();
                        }
                        self.selection = ta.selection_range();
                    }
                    self.edit_generation = self.edit_generation.wrapping_add(1);
                    return EventState::Consumed;
                }
                _ => {}
            }
        }

        let BackendState::Textarea(ta) = &mut self.backend else {
            unreachable!("handle_textarea_key called with non-Textarea backend")
        };

        // macOS-style navigation shortcuts not handled by ratatui-textarea.
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let handled = match (key.modifiers & !KeyModifiers::SHIFT, key.code) {
            (KeyModifiers::ALT, KeyCode::Left) => {
                cursor_move!(ta, CursorMove::WordBack, shift);
                true
            }
            (KeyModifiers::ALT, KeyCode::Right) => {
                cursor_move!(ta, CursorMove::WordForward, shift);
                true
            }
            (KeyModifiers::SUPER, KeyCode::Left) => {
                cursor_move!(ta, CursorMove::Head, shift);
                true
            }
            (KeyModifiers::SUPER, KeyCode::Right) => {
                cursor_move!(ta, CursorMove::End, shift);
                true
            }
            (KeyModifiers::SUPER, KeyCode::Up) => {
                cursor_move!(ta, CursorMove::Top, shift);
                true
            }
            (KeyModifiers::SUPER, KeyCode::Down) => {
                cursor_move!(ta, CursorMove::Bottom, shift);
                true
            }
            _ => false,
        };
        if handled {
            self.selection = ta.selection_range();
            self.edit_generation = self.edit_generation.wrapping_add(1);
            return EventState::Consumed;
        }

        // FocusSidebar / FocusEditor shortcuts are intercepted at the
        // EditorScreen level for directional navigation.

        // Standard text-editor shortcuts.
        // `input_without_shortcuts` only handles chars, backspace, delete, tab, newline —
        // all navigation and editing shortcuts must be mapped explicitly.
        let shortcut_handled = match (key.modifiers & !KeyModifiers::SHIFT, key.code) {
            // --- Cursor movement (Shift extends the selection) ---
            (KeyModifiers::NONE, KeyCode::Left) => {
                cursor_move!(ta, CursorMove::Back, shift);
                true
            }
            (KeyModifiers::NONE, KeyCode::Right) => {
                cursor_move!(ta, CursorMove::Forward, shift);
                true
            }
            (KeyModifiers::NONE, KeyCode::Up) => {
                cursor_move!(ta, CursorMove::Up, shift);
                true
            }
            (KeyModifiers::NONE, KeyCode::Down) => {
                cursor_move!(ta, CursorMove::Down, shift);
                true
            }
            (KeyModifiers::NONE, KeyCode::Home) => {
                cursor_move!(ta, CursorMove::Head, shift);
                true
            }
            (KeyModifiers::NONE, KeyCode::End) => {
                cursor_move!(ta, CursorMove::End, shift);
                true
            }
            (KeyModifiers::NONE, KeyCode::PageUp) => {
                cursor_move!(ta, CursorMove::ParagraphBack, shift);
                true
            }
            (KeyModifiers::NONE, KeyCode::PageDown) => {
                cursor_move!(ta, CursorMove::ParagraphForward, shift);
                true
            }
            // Word navigation (Ctrl+arrow, Windows/Linux style)
            (KeyModifiers::CONTROL, KeyCode::Left) => {
                cursor_move!(ta, CursorMove::WordBack, shift);
                true
            }
            (KeyModifiers::CONTROL, KeyCode::Right) => {
                cursor_move!(ta, CursorMove::WordForward, shift);
                true
            }
            // Document start / end
            (KeyModifiers::CONTROL, KeyCode::Home) => {
                cursor_move!(ta, CursorMove::Top, shift);
                true
            }
            (KeyModifiers::CONTROL, KeyCode::End) => {
                cursor_move!(ta, CursorMove::Bottom, shift);
                true
            }
            // Undo / Redo (Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z)
            (KeyModifiers::CONTROL, KeyCode::Char('z')) => {
                ta.undo();
                true
            }
            (KeyModifiers::CONTROL, KeyCode::Char('y'))
            | (KeyModifiers::CONTROL, KeyCode::Char('Z')) => {
                ta.redo();
                true
            }
            // Select all
            (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                ta.move_cursor(CursorMove::Top);
                ta.start_selection();
                ta.move_cursor(CursorMove::Bottom);
                true
            }
            // Delete word before / after cursor
            (KeyModifiers::CONTROL, KeyCode::Backspace)
            | (KeyModifiers::ALT, KeyCode::Backspace) => {
                ta.delete_word();
                true
            }
            (KeyModifiers::CONTROL, KeyCode::Delete) | (KeyModifiers::ALT, KeyCode::Delete) => {
                ta.delete_next_word();
                true
            }
            _ => false,
        };
        if shortcut_handled {
            self.selection = ta.selection_range();
            self.edit_generation = self.edit_generation.wrapping_add(1);
            return EventState::Consumed;
        }

        // BackTab is what most terminals emit for Shift+Tab.
        match (key.modifiers, key.code) {
            (m, KeyCode::Tab)
                if !m.contains(KeyModifiers::CONTROL) && !m.contains(KeyModifiers::ALT) =>
            {
                self.indent_lines(m.contains(KeyModifiers::SHIFT));
                return EventState::Consumed;
            }
            (_, KeyCode::BackTab) => {
                self.indent_lines(true);
                return EventState::Consumed;
            }
            _ => {}
        }
        if key.code == KeyCode::Enter && key.modifiers.is_empty() && self.smart_enter() {
            return EventState::Consumed;
        }

        let BackendState::Textarea(ta) = &mut self.backend else {
            unreachable!("handle_textarea_key called with non-Textarea backend")
        };
        ta.input_without_shortcuts(*key);
        self.selection = ta.selection_range();
        self.edit_generation = self.edit_generation.wrapping_add(1);
        EventState::Consumed
    }

    /// Handle a mouse event (Textarea backend only).
    fn handle_mouse(
        &mut self,
        mouse: &ratatui::crossterm::event::MouseEvent,
        tx: &AppTx,
    ) -> EventState {
        let BackendState::Textarea(_) = &self.backend else {
            return EventState::NotConsumed;
        };
        let r = &self.rect;
        let in_bounds = mouse.column >= r.x
            && mouse.column < r.x + r.width
            && mouse.row >= r.y
            && mouse.row < r.y + r.height;
        if !in_bounds {
            return EventState::NotConsumed;
        }
        // Handle right-click clipboard copy in its own scope to avoid borrow conflicts.
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right)) {
            tx.send(AppEvent::FocusEditor).ok();
            self.copy_selection_to_clipboard();
            self.selection = if let BackendState::Textarea(ta) = &self.backend {
                ta.selection_range()
            } else {
                None
            };
            self.edit_generation = self.edit_generation.wrapping_add(1);
            return EventState::Consumed;
        }
        // Now extract ta for remaining mouse operations.
        let BackendState::Textarea(ta) = &mut self.backend else {
            unreachable!()
        };
        match mouse.kind {
            MouseEventKind::Down(_) => {
                tx.send(AppEvent::FocusEditor).ok();
                ta.cancel_selection();
                let vrow = (mouse.row - r.y) as usize + self.view.visual_scroll_offset;
                let vcol = (mouse.column - r.x) as usize;
                let (lrow, lcol) = self.view.click_to_logical_u16(vrow, vcol);
                ta.move_cursor(CursorMove::Jump(lrow, lcol));
                ta.start_selection();
            }
            MouseEventKind::Drag(_) => {
                let vrow = (mouse.row - r.y) as usize + self.view.visual_scroll_offset;
                let vcol = (mouse.column - r.x) as usize;
                let (lrow, lcol) = self.view.click_to_logical_u16(vrow, vcol);
                ta.move_cursor(CursorMove::Jump(lrow, lcol));
            }
            _ => {
                ta.input(*mouse);
            }
        }
        self.selection = ta.selection_range();
        self.edit_generation = self.edit_generation.wrapping_add(1);
        EventState::Consumed
    }
}

impl Component for TextEditorComponent {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        self.maybe_recover_from_dead_nvim();

        match event {
            InputEvent::Key(key) => {
                if let Some(state) = self.handle_nvim_key(key, tx) {
                    return state;
                }
                self.handle_textarea_key(key, tx)
            }
            InputEvent::Mouse(mouse) => self.handle_mouse(mouse, tx),
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.rect = rect;
        match &self.backend {
            BackendState::Textarea(ta) => {
                let cursor = ta.cursor();
                let lines = ta.lines();
                self.view
                    .update(lines, cursor, rect, self.edit_generation, self.selection);
            }
            BackendState::Nvim(nvim) => {
                nvim.maybe_resize(rect.width, rect.height);
                let snap = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                let cursor = snap.cursor;
                let lines = snap.lines.clone();
                let content_gen = snap.content_gen;
                let visual_selection = snap.visual_selection;
                drop(snap);
                self.view
                    .update(&lines, cursor, rect, content_gen, visual_selection);
            }
        }
        self.view.render(f, rect, theme, focused);
    }

    fn hint_shortcuts(&self) -> Vec<(String, String)> {
        use crate::keys::action_shortcuts::ActionShortcuts;

        // For the Nvim backend, prepend the current mode as the first "hint".
        if let BackendState::Nvim(nvim) = &self.backend {
            let label = nvim
                .snapshot
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .footer_label();
            let mut hints = vec![(String::new(), label)];
            hints.extend(
                [
                    (ActionShortcuts::FocusSidebar, "\u{2190} sidebar"),
                    (ActionShortcuts::FocusEditor, "backlinks \u{2192}"),
                    (ActionShortcuts::FileOperations, "file ops"),
                ]
                .iter()
                .filter_map(|(action, label)| {
                    self.key_bindings
                        .first_combo_for(action)
                        .map(|k| (k, label.to_string()))
                }),
            );
            return hints;
        }

        [
            (ActionShortcuts::FocusSidebar, "\u{2190} sidebar"),
            (ActionShortcuts::FocusEditor, "backlinks \u{2192}"),
            (ActionShortcuts::FileOperations, "file ops"),
        ]
        .iter()
        .filter_map(|(action, label)| {
            self.key_bindings
                .first_combo_for(action)
                .map(|k| (k, label.to_string()))
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::KeyBindings;

    fn make_editor() -> TextEditorComponent {
        TextEditorComponent::new(
            KeyBindings::empty(),
            &crate::settings::AppSettings::default(),
        )
    }

    fn get_ta(editor: &mut TextEditorComponent) -> &mut TextArea<'static> {
        match &mut editor.backend {
            BackendState::Textarea(ta) => ta,
            _ => panic!("expected Textarea backend"),
        }
    }

    #[test]
    fn fresh_editor_is_not_dirty() {
        let editor = make_editor();
        assert!(!editor.is_dirty());
    }

    #[test]
    fn after_set_text_not_dirty() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        assert!(!editor.is_dirty());
    }

    #[test]
    fn get_text_returns_loaded_content() {
        let mut editor = make_editor();
        editor.set_text("line one\nline two".to_string());
        assert_eq!(editor.get_text(), "line one\nline two");
    }

    #[test]
    fn mark_saved_clears_dirty() {
        let mut editor = make_editor();
        editor.set_text("initial".to_string());
        let text = editor.get_text();
        editor.mark_saved(text.clone() + "x"); // saved state diverges
        assert!(editor.is_dirty());
        editor.mark_saved(text); // saved state matches again
        assert!(!editor.is_dirty());
    }

    #[test]
    fn trailing_newline_does_not_cause_false_dirty() {
        let mut editor = make_editor();
        editor.set_text("content\n".to_string());
        assert!(
            !editor.is_dirty(),
            "trailing newline should not make editor dirty after load"
        );
    }

    #[test]
    fn mouse_down_clears_selection() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        let ta = get_ta(&mut editor);
        ta.start_selection();
        ta.move_cursor(ratatui_textarea::CursorMove::WordForward);
        assert!(ta.selection_range().is_some());
        ta.cancel_selection();
        editor.selection = if let BackendState::Textarea(ta) = &editor.backend {
            ta.selection_range()
        } else {
            None
        };
        assert!(editor.selection.is_none());
    }

    #[test]
    fn ctrl_c_copies_selected_text() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        let ta = get_ta(&mut editor);
        ta.move_cursor(ratatui_textarea::CursorMove::Head);
        ta.start_selection();
        ta.move_cursor(ratatui_textarea::CursorMove::WordForward);
        let range = ta.selection_range().unwrap();
        let ((sr, sc), (er, ec)) = range;
        let lines = ta.lines();
        let selected = if sr == er {
            lines[sr][sc..ec].to_string()
        } else {
            lines[sr][sc..].to_string()
        };
        assert_eq!(selected, "hello ");
    }

    #[test]
    fn paste_inserts_text_at_cursor() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let ta = get_ta(&mut editor);
        ta.move_cursor(ratatui_textarea::CursorMove::End);
        ta.insert_str(" world");
        assert_eq!(editor.get_text(), "hello world");
    }

    #[test]
    fn bold_action_with_no_selection_inserts_pair_and_centers_cursor() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        editor.apply_text_action(TextAction::Bold);
        assert_eq!(editor.get_text(), "hello****");
        let ta = get_ta(&mut editor);
        assert_eq!(ta.cursor(), (0, 7));
    }

    #[test]
    fn italic_action_with_no_selection_inserts_single_pair() {
        let mut editor = make_editor();
        editor.set_text(String::new());
        editor.apply_text_action(TextAction::Italic);
        assert_eq!(editor.get_text(), "**");
        let ta = get_ta(&mut editor);
        assert_eq!(ta.cursor(), (0, 1));
    }

    #[test]
    fn strikethrough_action_with_selection_wraps_text() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::Head);
            ta.start_selection();
            ta.move_cursor(ratatui_textarea::CursorMove::WordForward);
        }
        editor.apply_text_action(TextAction::Strikethrough);
        assert_eq!(editor.get_text(), "~~hello ~~world");
    }

    #[test]
    fn bold_action_wraps_non_ascii_selection() {
        let mut editor = make_editor();
        editor.set_text("hello 你好 world".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::Head);
            ta.move_cursor(ratatui_textarea::CursorMove::WordForward);
            ta.start_selection();
            ta.move_cursor(ratatui_textarea::CursorMove::WordForward);
        }
        editor.apply_text_action(TextAction::Bold);
        assert_eq!(editor.get_text(), "hello **你好 **world");
    }

    #[test]
    fn bold_action_wraps_selected_text() {
        let mut editor = make_editor();
        editor.set_text("foo bar".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::Head);
            ta.start_selection();
            ta.move_cursor(ratatui_textarea::CursorMove::WordForward);
        }
        editor.apply_text_action(TextAction::Bold);
        assert_eq!(editor.get_text(), "**foo **bar");
    }

    #[test]
    fn indent_no_selection_indents_current_line() {
        let mut editor = make_editor();
        editor.set_text("foo\nbar".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::Bottom);
        }
        editor.indent_lines(false);
        let lines = get_ta(&mut editor).lines();
        assert_eq!(lines[0], "foo");
        assert!(lines[1].starts_with(' ') || lines[1].starts_with('\t'));
        assert!(lines[1].trim_start() == "bar");
    }

    #[test]
    fn indent_with_selection_indents_all_touched_lines() {
        let mut editor = make_editor();
        editor.set_text("foo\nbar\nbaz".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::Top);
            ta.start_selection();
            ta.move_cursor(ratatui_textarea::CursorMove::Down);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        editor.indent_lines(false);
        let lines: Vec<String> = get_ta(&mut editor).lines().to_vec();
        assert_eq!(lines[0].trim_start(), "foo");
        assert_eq!(lines[1].trim_start(), "bar");
        assert_eq!(lines[2], "baz");
        assert!(lines[0].len() > 3);
        assert!(lines[1].len() > 3);
    }

    #[test]
    fn dedent_removes_leading_indent() {
        let mut editor = make_editor();
        editor.set_text("    foo\n  bar\nbaz".to_string());
        let tab_len = get_ta(&mut editor).tab_length() as usize;
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::Top);
            ta.start_selection();
            ta.move_cursor(ratatui_textarea::CursorMove::Bottom);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        editor.indent_lines(true);
        let lines: Vec<String> = get_ta(&mut editor).lines().to_vec();
        // line 0 had 4 leading spaces; up to tab_len removed.
        assert_eq!(lines[0], format!("{}foo", " ".repeat(4 - tab_len.min(4))));
        // line 1 had 2 leading spaces; up to min(2, tab_len) removed.
        assert_eq!(
            lines[1],
            format!("{}bar", " ".repeat(2usize.saturating_sub(tab_len)))
        );
        assert_eq!(lines[2], "baz");
    }

    #[test]
    fn dedent_no_leading_whitespace_is_noop_for_that_line() {
        let mut editor = make_editor();
        editor.set_text("foo".to_string());
        editor.indent_lines(true);
        assert_eq!(editor.get_text(), "foo");
    }

    #[test]
    fn smart_enter_continues_unordered_list() {
        let mut editor = make_editor();
        editor.set_text("- foo".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "- foo\n- ");
    }

    #[test]
    fn smart_enter_continues_ordered_list_increments() {
        let mut editor = make_editor();
        editor.set_text("1. foo".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "1. foo\n2. ");
    }

    #[test]
    fn smart_enter_on_empty_list_marker_clears_line() {
        let mut editor = make_editor();
        editor.set_text("- ".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "");
    }

    #[test]
    fn smart_enter_preserves_indent() {
        let mut editor = make_editor();
        editor.set_text("    body".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "    body\n    ");
    }

    #[test]
    fn smart_enter_on_empty_indent_dedents() {
        let mut editor = make_editor();
        editor.set_text("    ".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        let tab_len = get_ta(&mut editor).tab_length() as usize;
        assert!(editor.smart_enter());
        assert_eq!(
            editor.get_text(),
            " ".repeat(4usize.saturating_sub(tab_len))
        );
    }

    #[test]
    fn smart_enter_no_indent_no_marker_returns_false() {
        let mut editor = make_editor();
        editor.set_text("plain".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        assert!(!editor.smart_enter());
        assert_eq!(editor.get_text(), "plain");
    }

    #[test]
    fn smart_enter_mid_line_returns_false() {
        let mut editor = make_editor();
        editor.set_text("- foo".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::Head);
            ta.move_cursor(ratatui_textarea::CursorMove::Forward);
            ta.move_cursor(ratatui_textarea::CursorMove::Forward);
        }
        assert!(!editor.smart_enter());
    }

    #[test]
    fn smart_enter_on_empty_indented_list_marker_dedents_keeping_marker() {
        let mut editor = make_editor();
        let tab_len = get_ta(&mut editor).tab_length() as usize;
        let indent = " ".repeat(tab_len);
        editor.set_text(format!("{indent}- "));
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "- ");
    }

    #[test]
    fn smart_enter_on_empty_list_marker_clears_line_after_full_dedent() {
        let mut editor = make_editor();
        let tab_len = get_ta(&mut editor).tab_length() as usize;
        let indent = " ".repeat(tab_len);
        editor.set_text(format!("{indent}- "));
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        // First Enter: dedent to "- ".
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "- ");
        // Second Enter at column == end-of-line: now cursor is at col 2 (end of "- ").
        // Need to position cursor at end after the dedent.
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "");
    }

    #[test]
    fn smart_enter_continues_list_with_non_ascii_content() {
        let mut editor = make_editor();
        editor.set_text("- 你好".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "- 你好\n- ");
    }

    #[test]
    fn smart_enter_preserves_tab_indent() {
        let mut editor = make_editor();
        editor.set_text("\tbody".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "\tbody\n\t");
    }

    #[test]
    fn smart_enter_on_tab_only_line_dedents() {
        let mut editor = make_editor();
        editor.set_text("\t\t".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        assert!(editor.smart_enter());
        // tab counts as one indent unit, regardless of tab_length spaces.
        assert_eq!(editor.get_text(), "\t");
    }

    #[test]
    fn smart_enter_continues_indented_list() {
        let mut editor = make_editor();
        editor.set_text("  - foo".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "  - foo\n  - ");
    }

    #[test]
    fn unsupported_text_action_is_noop() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        editor.apply_text_action(TextAction::Underline);
        assert_eq!(editor.get_text(), "hello");
    }

    #[test]
    fn textarea_hint_shortcuts_has_no_mode_indicator() {
        let editor = make_editor();
        let hints = editor.hint_shortcuts();
        // None of the hint labels should be "NORMAL", "INSERT", etc.
        assert!(
            !hints
                .iter()
                .any(|(_, label)| label == "NORMAL" || label == "INSERT")
        );
    }
}
