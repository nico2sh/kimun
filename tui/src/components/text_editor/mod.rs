pub mod autocomplete_glue;
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
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui_textarea::{CursorMove, DataCursor, TextArea};

/// Convert `TextArea::cursor()` from the library's `DataCursor` newtype to a
/// plain `(row, col)` tuple — the neutral interchange type shared with the
/// Nvim backend (whose `NvimSnapshot::cursor` is already a tuple).
fn cursor_tuple(ta: &TextArea<'_>) -> (usize, usize) {
    let DataCursor(r, c) = ta.cursor();
    (r, c)
}

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

/// Convert a 0-based character column into a byte offset within `line`.
/// Out-of-range columns return `line.len()`.
fn char_col_to_byte(line: &str, char_col: usize) -> usize {
    line.char_indices()
        .nth(char_col)
        .map(|(b, _)| b)
        .unwrap_or(line.len())
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
    Some(if sr == er {
        let line = &lines[sr];
        let sb = char_col_to_byte(line, sc);
        let eb = char_col_to_byte(line, ec);
        line[sb..eb].to_string()
    } else {
        let first = &lines[sr];
        let sb = char_col_to_byte(first, sc);
        let mut parts = vec![first[sb..].to_string()];
        for line in &lines[(sr + 1)..er] {
            parts.push(line.clone());
        }
        let last = &lines[er];
        let eb = char_col_to_byte(last, ec);
        parts.push(last[..eb].to_string());
        parts.join("\n")
    })
}

/// Owned RGBA image data lifted from the system clipboard. Returned by
/// [`TextEditorComponent::take_clipboard_image`] so the screen layer can
/// encode + persist without holding the editor's clipboard borrow.
#[derive(Debug, Clone)]
pub struct ClipboardImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Schemes the paste-over-selection flow recognises as "linkable" — broader
/// than `core::is_remote_url` (http/https only) because users routinely paste
/// `mailto:` and FTP links and expect them wrapped as markdown links too.
const LINKABLE_PASTE_SCHEMES: &[&str] = &["http", "https", "ftp", "ftps", "mailto"];

fn linkable_url(s: &str) -> Option<&str> {
    kimun_core::note::url_with_allowed_scheme(s, LINKABLE_PASTE_SCHEMES)
}

/// If `clip` is a linkable URL and `selection` is non-empty, returns
/// `Some("[escaped_selection](url)")`. Otherwise returns `None`, signalling the
/// caller to insert `clip` verbatim.
fn try_build_markdown_link(clip: &str, selection: Option<&str>) -> Option<String> {
    let url = linkable_url(clip)?;
    let sel = selection.filter(|s| !s.is_empty())?;
    let escaped = sel.replace('\\', r"\\").replace(']', r"\]");
    Some(format!("[{escaped}]({url})"))
}

use std::sync::Arc;

use kimun_core::NoteVault;

use crate::components::Component;
use crate::components::autocomplete::{
    self, AutocompleteController, AutocompleteHost, AutocompleteMode, HandleKeyOutcome,
};
use crate::components::event_state::EventState;
use crate::components::events::AppEvent;
use crate::components::events::AppTx;
use crate::components::events::InputEvent;
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::components::text_editor::autocomplete_glue::apply_accept_to_textarea;
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::TextAction;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

/// The resolved target of a cursor follow-link action.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkTarget {
    /// A note reference (wiki-link or markdown link) with the raw target string.
    Note(String),
    /// A hashtag label with the name **without** the leading `#`.
    Label(String),
}

struct SearchState {
    input: SingleLineInput,
    status: SearchStatus,
}

enum SearchStatus {
    Empty,
    Match,
    NoMatch,
    Invalid(String),
}

impl SearchStatus {
    fn from_found(found: bool) -> Self {
        if found { Self::Match } else { Self::NoMatch }
    }
}

const FIND_PROMPT: &str = "Find: ";
const FIND_HINTS: &str = "  [Enter] next  [Shift+Enter] prev  [Esc] close";

fn render_search_bar(
    f: &mut Frame,
    rect: Rect,
    state: &mut SearchState,
    theme: &Theme,
    focused: bool,
) {
    let base = theme.base_style();
    let muted = Style::default()
        .fg(theme.fg_muted.to_ratatui())
        .bg(theme.bg.to_ratatui());
    let err = Style::default()
        .fg(ratatui::style::Color::Red)
        .bg(theme.bg.to_ratatui());
    let prompt_cols = unicode_width::UnicodeWidthStr::width(FIND_PROMPT) as u16;
    // Tail sits after the full value (in display columns, accounting for
    // wide/CJK chars), not after the caret — otherwise it would overlap the
    // trailing characters when the user moves the cursor mid-string.
    let value_total_cols = state.input.display_width() as u16;
    let tail: Option<(String, Style)> = match &state.status {
        SearchStatus::Empty => None,
        SearchStatus::Match => Some((FIND_HINTS.to_string(), muted)),
        SearchStatus::NoMatch => Some(("  no match".to_string(), err)),
        SearchStatus::Invalid(msg) => Some((format!("  invalid regex: {msg}"), err)),
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            FIND_PROMPT,
            base.add_modifier(Modifier::BOLD),
        )))
        .style(base),
        Rect {
            width: prompt_cols.min(rect.width),
            ..rect
        },
    );
    state.input.render(f, rect, base, prompt_cols, focused);
    if let Some((text, style)) = tail {
        let consumed = prompt_cols.saturating_add(value_total_cols);
        let tail_rect = Rect {
            x: rect.x.saturating_add(consumed),
            width: rect.width.saturating_sub(consumed),
            ..rect
        };
        f.render_widget(Paragraph::new(text).style(style), tail_rect);
    }
}

/// Snapshot used to satisfy `AutocompleteHost`. The snapshot is owned so
/// the controller's borrow does not overlap with the textarea's `&mut`
/// borrow during key handling and replacement.
struct EditorHostSnapshot {
    lines: Vec<String>,
    cursor_byte: usize,
    cursor_screen: Option<(u16, u16)>,
}

impl AutocompleteHost for EditorHostSnapshot {
    fn buffer_text(&self) -> String {
        self.lines.join("\n")
    }
    fn cursor_byte_offset(&self) -> usize {
        self.cursor_byte
    }
    fn screen_anchor_for(&self, _byte_offset: usize) -> Option<(u16, u16)> {
        // We anchor at the cursor's last-rendered screen position. The
        // controller passes `anchor_col` (byte offset of the start of the
        // typed query) but visually anchoring at the cursor is fine — the
        // popup sits adjacent to the typed text either way and avoids
        // re-walking the wrap layout for an arbitrary byte offset.
        self.cursor_screen
    }
}

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
    /// Active Ctrl+F find bar; `None` when not searching.
    search: Option<SearchState>,
    /// Wikilink/hashtag autocomplete. Only populated for the textarea
    /// backend after `set_vault` is called; remains `None` for the Nvim
    /// backend (nvim users have their own completion ecosystem).
    autocomplete: Option<AutocompleteController>,
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
            search: None,
            autocomplete: None,
        }
    }

    /// Attach a vault so autocomplete can query notes/tags. No-op for the
    /// Nvim backend — the embedded neovim instance owns its own input
    /// pipeline.
    pub fn set_vault(&mut self, vault: Arc<NoteVault>) {
        if matches!(self.backend, BackendState::Nvim(_)) {
            return;
        }
        self.autocomplete = Some(AutocompleteController::new(
            vault,
            AutocompleteMode::Both,
        ));
    }

    /// Build a snapshot view of the editor state for the autocomplete
    /// controller. Lives separately so the controller can borrow it
    /// without colliding with the textarea's `&mut self` borrow.
    fn autocomplete_host_snapshot(&self) -> Option<EditorHostSnapshot> {
        let BackendState::Textarea(ta) = &self.backend else {
            return None;
        };
        let lines: Vec<String> = ta.lines().iter().map(|l| l.to_string()).collect();
        let (row, col) = cursor_tuple(ta);
        let cursor_byte =
            autocomplete_glue::row_char_col_to_byte(&lines, row, col);
        Some(EditorHostSnapshot {
            lines,
            cursor_byte,
            cursor_screen: self.view.last_cursor_screen,
        })
    }

    /// Pull the latest async query results into the popup state. Called
    /// once per render before drawing the overlay.
    fn poll_autocomplete(&mut self) {
        if let Some(controller) = self.autocomplete.as_mut() {
            controller.poll_results();
        }
    }

    /// Recompute the popup's trigger context from the current buffer and
    /// cursor. Call after any mutating key handle (typed letter, paste,
    /// backspace, cursor movement, etc.).
    fn sync_autocomplete(&mut self) {
        let Some(snapshot) = self.autocomplete_host_snapshot() else {
            if let Some(c) = self.autocomplete.as_mut() {
                c.close();
            }
            return;
        };
        if let Some(controller) = self.autocomplete.as_mut() {
            controller.sync(&snapshot);
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
        // Buffer replaced — close any open autocomplete popup so it does
        // not linger over the new note (e.g. after Ctrl+G follow-link).
        if let Some(c) = self.autocomplete.as_mut() {
            c.close();
        }
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

    /// Returns the link or label target under the cursor, or `None` if the
    /// cursor is not inside a wikilink, markdown link, or hashtag span.
    pub fn link_at_cursor(&self) -> Option<LinkTarget> {
        let (_row, col, line) = match &self.backend {
            BackendState::Textarea(ta) => {
                let (row, col) = cursor_tuple(ta);
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

        // F5: Check wiki-link / markdown-link spans first; Link wins over Label
        // even if a future edit accidentally lets a Label slip through a Link range.
        if let Some(span) = kimun_core::note::link_char_spans(&line)
            .into_iter()
            .find(|s| s.start <= col && col < s.end)
        {
            return Some(LinkTarget::Note(span.target));
        }

        // Fallback: check for a hashtag label (via the markdown parser).
        let parsed = self::markdown::ParsedLine::parse(&line);
        parsed
            .elements
            .iter()
            .find(|e| {
                e.kind == self::markdown::ElementKind::Label
                    && col >= e.start_char
                    && col < e.end_char
            })
            .map(|e| {
                let span: String = line
                    .chars()
                    .skip(e.start_char)
                    .take(e.end_char - e.start_char)
                    .collect();
                let name = span.trim_start_matches('#').to_string();
                LinkTarget::Label(name)
            })
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
    fn paste_from_clipboard(&mut self, tx: &AppTx) {
        let text = match &mut self.clipboard {
            Some(cb) => match cb.get_text() {
                Ok(t) if !t.is_empty() => t,
                _ => return,
            },
            None => return,
        };
        self.paste_text(&text, tx);
    }

    /// Inserts `text` at the cursor, replacing any active selection. When `text`
    /// is a URL (http/https/ftp/ftps/mailto) and a selection is active, the
    /// selection is wrapped as a markdown link `[selection](url)` instead of
    /// being replaced by the raw URL.
    ///
    /// On the Nvim backend the URL-wrap shortcut is skipped (would require
    /// reading the visual selection from nvim) — `text` is forwarded via
    /// `nvim_paste`, which honours the current mode (insert/normal/visual).
    pub fn paste_text(&mut self, text: &str, tx: &AppTx) {
        if text.is_empty() {
            return;
        }
        match &mut self.backend {
            BackendState::Textarea(ta) => {
                let selection = linkable_url(text).and_then(|_| selection_text(ta));
                let wrapped = try_build_markdown_link(text, selection.as_deref());
                if ta.selection_range().is_some() {
                    ta.cut();
                }
                ta.insert_str(wrapped.as_deref().unwrap_or(text));
                self.selection = ta.selection_range();
                self.edit_generation = self.edit_generation.wrapping_add(1);
            }
            BackendState::Nvim(nvim) => {
                nvim.paste(text, tx.clone());
                self.edit_generation = self.edit_generation.wrapping_add(1);
            }
        }
    }

    /// Inserts `text` at the cursor, replacing any active selection. Routes
    /// through `nvim_paste` on the Nvim backend (delegates to [`paste_text`]
    /// for that case — URL-wrap is a no-op when nothing in the supplied text
    /// matches `linkable_url`, so the two paths are equivalent on Nvim).
    pub fn insert_at_cursor(&mut self, text: &str, tx: &AppTx) {
        if matches!(self.backend, BackendState::Nvim(_)) {
            self.paste_text(text, tx);
            return;
        }
        if let BackendState::Textarea(ta) = &mut self.backend {
            if ta.selection_range().is_some() {
                ta.cut();
            }
            ta.insert_str(text);
            self.selection = ta.selection_range();
            self.edit_generation = self.edit_generation.wrapping_add(1);
        }
    }

    /// Snapshot of the system clipboard image, if any. Returns owned RGBA bytes
    /// plus the image dimensions. The screen layer is responsible for encoding
    /// (e.g. PNG) and persisting via the vault.
    pub fn take_clipboard_image(&mut self) -> Option<ClipboardImage> {
        let cb = self.clipboard.as_mut()?;
        let img = cb.get_image().ok()?;
        Some(ClipboardImage {
            width: img.width,
            height: img.height,
            rgba: img.bytes.into_owned(),
        })
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
            let (row, col) = cursor_tuple(ta);
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
            Some(cursor_tuple(ta))
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

    /// Open the find bar; if already open, advance to the next match. No-op
    /// on the Nvim backend (which has its own `/` search). Public so
    /// `EditorScreen` can route the configurable `FindInBuffer` shortcut here.
    pub fn open_or_advance_search(&mut self) {
        if !matches!(self.backend, BackendState::Textarea(_)) {
            return;
        }
        if self.search.is_some() {
            self.search_advance(false);
            return;
        }
        self.search = Some(SearchState {
            input: SingleLineInput::new(),
            status: SearchStatus::Empty,
        });
    }

    fn close_search(&mut self) {
        if let BackendState::Textarea(ta) = &mut self.backend {
            let _ = ta.set_search_pattern("");
        }
        self.search = None;
        self.selection = None;
    }

    /// Push pattern to the textarea. When `jump` is true and the query compiles,
    /// also jumps to the first match at or after the cursor (live preview).
    fn refresh_search_pattern(&mut self, jump: bool) {
        let Some(state) = self.search.as_mut() else {
            return;
        };
        let BackendState::Textarea(ta) = &mut self.backend else {
            return;
        };
        if state.input.is_empty() {
            let _ = ta.set_search_pattern("");
            state.status = SearchStatus::Empty;
            self.selection = None;
            return;
        }
        if let Err(e) = ta.set_search_pattern(state.input.value()) {
            state.status = SearchStatus::Invalid(e.to_string());
            self.selection = None;
            return;
        }
        if !jump {
            state.status = SearchStatus::Match;
            return;
        }
        let found = ta.search_forward(true);
        state.status = SearchStatus::from_found(found);
        self.highlight_current_match(found);
    }

    fn search_advance(&mut self, backward: bool) {
        let Some(state) = self.search.as_mut() else {
            return;
        };
        if state.input.is_empty() {
            return;
        }
        let BackendState::Textarea(ta) = &mut self.backend else {
            return;
        };
        let found = if backward {
            ta.search_back(false)
        } else {
            ta.search_forward(false)
        };
        state.status = SearchStatus::from_found(found);
        self.highlight_current_match(found);
    }

    /// After a search step, paint the match at the textarea's cursor as the
    /// editor selection so the user can see where the match is — our custom
    /// `MarkdownEditorView` does not render the textarea library's built-in
    /// search highlights.
    fn highlight_current_match(&mut self, found: bool) {
        self.selection = if found {
            self.compute_match_selection()
        } else {
            None
        };
    }

    /// Locate the regex match starting at the textarea cursor and return its
    /// span as a `(row, char_col)` pair. Returns `None` when no pattern is set,
    /// the cursor is out of range, or the cursor is not on a match — guards
    /// against stale cursor/pattern state if callers ever invoke without a
    /// fresh search step.
    fn compute_match_selection(&self) -> Option<((usize, usize), (usize, usize))> {
        let BackendState::Textarea(ta) = &self.backend else {
            return None;
        };
        let re = ta.search_pattern()?;
        let DataCursor(row, col_chars) = ta.cursor();
        let line = ta.lines().get(row)?;
        let byte_off = char_col_to_byte(line, col_chars);
        let m = re.find_at(line, byte_off)?;
        if m.start() != byte_off {
            return None;
        }
        let match_chars = line[m.range()].chars().count();
        Some(((row, col_chars), (row, col_chars + match_chars)))
    }

    /// Returns `true` when the key was consumed by the find bar.
    fn handle_search_key(&mut self, key: &ratatui::crossterm::event::KeyEvent) -> bool {
        let Some(state) = self.search.as_mut() else {
            return false;
        };
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match state.input.handle_key(key) {
            InputOutcome::Cancel => self.close_search(),
            InputOutcome::Submit => self.search_advance(shift),
            InputOutcome::Changed => self.refresh_search_pattern(true),
            InputOutcome::Consumed | InputOutcome::NotConsumed => {}
        }
        true
    }

    /// Handle a key event when using the Textarea backend.
    fn handle_textarea_key(
        &mut self,
        key: &ratatui::crossterm::event::KeyEvent,
        tx: &AppTx,
    ) -> EventState {
        // Find bar — intercept ALL keys while active.
        if self.handle_search_key(key) {
            return EventState::Consumed;
        }

        // System clipboard shortcuts — intercept before passing to textarea.
        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Char('c') => {
                    self.copy_selection_to_clipboard();
                    return EventState::Consumed;
                }
                KeyCode::Char('v') => {
                    self.paste_from_clipboard(tx);
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
                // Autocomplete popup, when open, gets first crack at the
                // key. Accept / Dismiss are fully handled here; navigation
                // is consumed but does nothing to the buffer; NotHandled
                // falls through to the normal textarea flow (and then we
                // resync the popup so the typed letter refines the query
                // or breaks the trigger context).
                if let Some(host) = self.autocomplete_host_snapshot() {
                    if let Some(controller) = self.autocomplete.as_mut() {
                        if controller.is_open() {
                            match controller.handle_key(*key, &host) {
                                HandleKeyOutcome::Accepted(action) => {
                                    if let BackendState::Textarea(ta) = &mut self.backend {
                                        apply_accept_to_textarea(ta, &action);
                                        self.edit_generation =
                                            self.edit_generation.wrapping_add(1);
                                        self.selection = ta.selection_range();
                                    }
                                    return EventState::Consumed;
                                }
                                HandleKeyOutcome::Dismissed
                                | HandleKeyOutcome::Consumed => {
                                    return EventState::Consumed;
                                }
                                HandleKeyOutcome::NotHandled => {}
                            }
                        }
                    }
                }
                if let Some(state) = self.handle_nvim_key(key, tx) {
                    return state;
                }
                let gen_before = self.edit_generation;
                let result = self.handle_textarea_key(key, tx);
                // Only trigger the popup on actual edits — pure cursor
                // movement (arrows, Home/End, Ctrl+G-to-follow, mouse
                // click) must not pop the autocomplete open. If the popup
                // is already open and the cursor moves out of the trigger
                // range, close it; otherwise leave it alone.
                if self.edit_generation != gen_before {
                    self.sync_autocomplete();
                } else if let Some(c) = self.autocomplete.as_mut() {
                    c.close();
                }
                result
            }
            InputEvent::Mouse(mouse) => {
                let result = self.handle_mouse(mouse, tx);
                if let Some(c) = self.autocomplete.as_mut() {
                    c.close();
                }
                result
            }
            // Bracketed paste is intercepted by EditorScreen so it can run the
            // image-paste flow first. It never reaches us here.
            InputEvent::Paste(_) => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        // Reserve the bottom row for the find bar when active.
        let (editor_rect, search_rect) = if self.search.is_some() && rect.height > 1 {
            (
                Rect {
                    height: rect.height - 1,
                    ..rect
                },
                Some(Rect {
                    y: rect.y + rect.height - 1,
                    height: 1,
                    ..rect
                }),
            )
        } else {
            (rect, None)
        };
        // Store the editor area (not the full rect) so mouse hit-testing ignores
        // clicks on the find-bar row.
        self.rect = editor_rect;
        match &self.backend {
            BackendState::Textarea(ta) => {
                let cursor = cursor_tuple(ta);
                let lines = ta.lines();
                self.view.update(
                    lines,
                    cursor,
                    editor_rect,
                    self.edit_generation,
                    self.selection,
                );
            }
            BackendState::Nvim(nvim) => {
                nvim.maybe_resize(editor_rect.width, editor_rect.height);
                let snap = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                let cursor = snap.cursor;
                let lines = snap.lines.clone();
                let content_gen = snap.content_gen;
                let visual_selection = snap.visual_selection;
                drop(snap);
                self.view
                    .update(&lines, cursor, editor_rect, content_gen, visual_selection);
            }
        }
        // When the find bar is active, draw it AFTER the editor so its caret
        // (set via set_cursor_position) wins over the editor's caret call.
        let bar_focused = self.search.is_some() && focused;
        let editor_focused = focused && !bar_focused;
        self.view.render(f, editor_rect, theme, editor_focused);
        if let (Some(state), Some(bar_rect)) = (self.search.as_mut(), search_rect) {
            render_search_bar(f, bar_rect, state, theme, bar_focused);
        }

        // Autocomplete popup sits on top of the editor (and the find bar
        // when present). Drain any pending async query results first so
        // the popup reflects the latest typed prefix.
        self.poll_autocomplete();
        if let Some(controller) = self.autocomplete.as_ref() {
            if let Some(state) = controller.state() {
                autocomplete::render(f, state, rect, theme);
            }
        }
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
            (ActionShortcuts::FindInBuffer, "find"),
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

    fn dummy_tx() -> AppTx {
        tokio::sync::mpsc::unbounded_channel().0
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
    fn linkable_url_accepts_supported_schemes() {
        assert_eq!(
            linkable_url("https://example.com"),
            Some("https://example.com")
        );
        assert_eq!(
            linkable_url("http://example.com/path?q=1#frag"),
            Some("http://example.com/path?q=1#frag"),
        );
        assert_eq!(
            linkable_url("  https://example.com  "),
            Some("https://example.com")
        );
        assert_eq!(
            linkable_url("ftp://files.example.com/x"),
            Some("ftp://files.example.com/x"),
        );
        assert_eq!(
            linkable_url("ftps://files.example.com/x"),
            Some("ftps://files.example.com/x"),
        );
        assert_eq!(
            linkable_url("mailto:user@example.com"),
            Some("mailto:user@example.com"),
        );
        assert_eq!(
            linkable_url("mailto:user@example.com?subject=hi"),
            Some("mailto:user@example.com?subject=hi"),
        );
    }

    #[test]
    fn linkable_url_rejects_other_schemes_and_plain_text() {
        assert_eq!(linkable_url("file:///etc/passwd"), None);
        assert_eq!(linkable_url("ssh://host"), None);
        assert_eq!(linkable_url("javascript:alert(1)"), None);
        assert_eq!(linkable_url("example.com"), None);
        assert_eq!(linkable_url("not a url"), None);
        assert_eq!(linkable_url(""), None);
        assert_eq!(linkable_url("https://example.com\nmore"), None);
    }

    #[test]
    fn try_build_markdown_link_wraps_selection_when_clip_is_url() {
        assert_eq!(
            try_build_markdown_link("https://example.com", Some("click here")).as_deref(),
            Some("[click here](https://example.com)"),
        );
    }

    #[test]
    fn try_build_markdown_link_trims_url_whitespace() {
        assert_eq!(
            try_build_markdown_link("  https://example.com\n", Some("link")).as_deref(),
            Some("[link](https://example.com)"),
        );
    }

    #[test]
    fn try_build_markdown_link_returns_none_when_no_selection() {
        assert_eq!(try_build_markdown_link("https://example.com", None), None);
    }

    #[test]
    fn try_build_markdown_link_returns_none_when_not_url() {
        assert_eq!(try_build_markdown_link("plain text", Some("sel")), None);
    }

    #[test]
    fn try_build_markdown_link_returns_none_when_selection_empty() {
        assert_eq!(
            try_build_markdown_link("https://example.com", Some("")),
            None
        );
    }

    #[test]
    fn try_build_markdown_link_escapes_close_bracket_in_selection() {
        assert_eq!(
            try_build_markdown_link("https://example.com", Some("a]b")).as_deref(),
            Some(r"[a\]b](https://example.com)"),
        );
    }

    #[test]
    fn try_build_markdown_link_wraps_ftp_url() {
        assert_eq!(
            try_build_markdown_link("ftp://files.example.com/x", Some("download")).as_deref(),
            Some("[download](ftp://files.example.com/x)"),
        );
    }

    fn key(code: KeyCode, mods: KeyModifiers) -> ratatui::crossterm::event::KeyEvent {
        ratatui::crossterm::event::KeyEvent::new(code, mods)
    }

    #[test]
    fn open_or_advance_search_opens_find_bar_with_empty_query() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        editor.open_or_advance_search();
        let state = editor.search.as_ref().expect("find bar opened");
        assert!(state.input.is_empty());
        assert!(matches!(state.status, SearchStatus::Empty));
    }

    #[test]
    fn open_or_advance_search_advances_when_already_open() {
        let mut editor = make_editor();
        editor.set_text("ab ab ab".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        editor.handle_textarea_key(&key(KeyCode::Char('a'), KeyModifiers::NONE), &tx);
        editor.handle_textarea_key(&key(KeyCode::Char('b'), KeyModifiers::NONE), &tx);
        // Cursor now at first match (col 0). Re-invoking advances to second.
        editor.open_or_advance_search();
        let DataCursor(_, col) = get_ta(&mut editor).cursor();
        assert_eq!(col, 3, "second invocation advances to next match");
    }

    #[test]
    fn typing_in_find_bar_jumps_cursor_to_first_match() {
        let mut editor = make_editor();
        editor.set_text("foo bar baz".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        for ch in ['b', 'a', 'r'] {
            editor.handle_textarea_key(&key(KeyCode::Char(ch), KeyModifiers::NONE), &tx);
        }
        let state = editor.search.as_ref().unwrap();
        assert_eq!(state.input.value(), "bar");
        assert!(matches!(state.status, SearchStatus::Match));
        let DataCursor(_, col) = get_ta(&mut editor).cursor();
        assert_eq!(col, 4, "cursor jumped to start of 'bar'");
    }

    #[test]
    fn enter_in_find_bar_advances_to_next_match() {
        let mut editor = make_editor();
        editor.set_text("ab ab ab".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        editor.handle_textarea_key(&key(KeyCode::Char('a'), KeyModifiers::NONE), &tx);
        editor.handle_textarea_key(&key(KeyCode::Char('b'), KeyModifiers::NONE), &tx);
        // first match is at col 0 (match_cursor=true on type)
        editor.handle_textarea_key(&key(KeyCode::Enter, KeyModifiers::NONE), &tx);
        let DataCursor(_, col) = get_ta(&mut editor).cursor();
        assert_eq!(col, 3, "Enter advances to second match");
    }

    #[test]
    fn match_is_highlighted_as_selection_after_search() {
        let mut editor = make_editor();
        editor.set_text("foo bar baz".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        for ch in ['b', 'a', 'r'] {
            editor.handle_textarea_key(&key(KeyCode::Char(ch), KeyModifiers::NONE), &tx);
        }
        // "bar" lives at cols 4..7 on row 0.
        assert_eq!(editor.selection, Some(((0, 4), (0, 7))));
    }

    #[test]
    fn no_match_clears_selection() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        editor.handle_textarea_key(&key(KeyCode::Char('z'), KeyModifiers::NONE), &tx);
        assert_eq!(editor.selection, None);
    }

    #[test]
    fn esc_in_find_bar_clears_selection_highlight() {
        let mut editor = make_editor();
        editor.set_text("foo bar".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        editor.handle_textarea_key(&key(KeyCode::Char('b'), KeyModifiers::NONE), &tx);
        editor.handle_textarea_key(&key(KeyCode::Char('a'), KeyModifiers::NONE), &tx);
        editor.handle_textarea_key(&key(KeyCode::Char('r'), KeyModifiers::NONE), &tx);
        assert!(editor.selection.is_some());
        editor.handle_textarea_key(&key(KeyCode::Esc, KeyModifiers::NONE), &tx);
        assert!(editor.selection.is_none());
    }

    #[test]
    fn esc_in_find_bar_closes_it() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        assert!(editor.search.is_some());
        editor.handle_textarea_key(&key(KeyCode::Esc, KeyModifiers::NONE), &tx);
        assert!(editor.search.is_none());
    }

    #[test]
    fn find_bar_consumes_typing_so_editor_text_is_unchanged() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        editor.handle_textarea_key(&key(KeyCode::Char('x'), KeyModifiers::NONE), &tx);
        assert_eq!(editor.get_text(), "hello");
    }

    #[test]
    fn no_match_status_when_query_absent() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        editor.handle_textarea_key(&key(KeyCode::Char('z'), KeyModifiers::NONE), &tx);
        let state = editor.search.as_ref().unwrap();
        assert!(matches!(state.status, SearchStatus::NoMatch));
    }

    #[test]
    fn try_build_markdown_link_wraps_mailto_url() {
        assert_eq!(
            try_build_markdown_link("mailto:user@example.com", Some("email me")).as_deref(),
            Some("[email me](mailto:user@example.com)"),
        );
    }

    #[test]
    fn insert_at_cursor_appends_text() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::End);
        }
        editor.insert_at_cursor(" world", &dummy_tx());
        assert_eq!(editor.get_text(), "hello world");
    }

    #[test]
    fn insert_at_cursor_replaces_selection() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(ratatui_textarea::CursorMove::Head);
            ta.start_selection();
            ta.move_cursor(ratatui_textarea::CursorMove::WordForward);
        }
        editor.insert_at_cursor("HEY ", &dummy_tx());
        assert_eq!(editor.get_text(), "HEY world");
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

    // ── link_at_cursor: label detection ──────────────────────────────────────

    /// Helper: place cursor at a specific column on the first row.
    fn place_cursor_at_col(editor: &mut TextEditorComponent, col: usize) {
        let ta = get_ta(editor);
        ta.move_cursor(ratatui_textarea::CursorMove::Head);
        for _ in 0..col {
            ta.move_cursor(ratatui_textarea::CursorMove::Forward);
        }
    }

    #[test]
    fn link_at_cursor_returns_label_when_cursor_on_hashtag() {
        let mut editor = make_editor();
        editor.set_text("see #rust now".to_string());
        // "#rust" starts at col 4, ends at col 9 (5 chars). Place cursor at col 5 (inside).
        place_cursor_at_col(&mut editor, 5);
        assert_eq!(
            editor.link_at_cursor(),
            Some(LinkTarget::Label("rust".into())),
        );
    }

    #[test]
    fn link_at_cursor_returns_label_at_hash_char() {
        let mut editor = make_editor();
        editor.set_text("see #rust now".to_string());
        // Cursor exactly on '#' (col 4).
        place_cursor_at_col(&mut editor, 4);
        assert_eq!(
            editor.link_at_cursor(),
            Some(LinkTarget::Label("rust".into())),
        );
    }

    #[test]
    fn link_at_cursor_returns_none_outside_hashtag() {
        let mut editor = make_editor();
        editor.set_text("see #rust now".to_string());
        // Cursor at col 0 ("s") — not on a hashtag.
        place_cursor_at_col(&mut editor, 0);
        assert_eq!(editor.link_at_cursor(), None);
    }

    #[test]
    fn link_at_cursor_returns_note_for_wikilink() {
        let mut editor = make_editor();
        editor.set_text("open [[my note]] please".to_string());
        // "my note" is inside [[…]]; cursor at col 7 (inside link text).
        place_cursor_at_col(&mut editor, 7);
        let result = editor.link_at_cursor();
        assert!(
            matches!(result, Some(LinkTarget::Note(_))),
            "expected Note variant, got {result:?}"
        );
    }

    // ── F5: link_at_cursor prioritises Link over Label ────────────────────────

    #[test]
    fn link_at_cursor_returns_note_for_markdown_link_with_fragment() {
        // "[see docs](#section)" — cursor on `#section` should return Note, not Label.
        // After F3, the Label inside a link is never emitted, so the bug is
        // structurally prevented. This test guards F5: even if a future edit
        // accidentally adds a Label, Link wins because link_char_spans is checked first.
        let line = "[see docs](#section)";
        let mut editor = make_editor();
        editor.set_text(line.to_string());
        // "#section" starts at byte/char offset 11 (after "[see docs](").
        let cursor = "[see docs](#sec".chars().count(); // col 15, inside #section
        place_cursor_at_col(&mut editor, cursor);
        let result = editor.link_at_cursor();
        assert!(
            matches!(result, Some(LinkTarget::Note(_))),
            "expected Note variant for markdown link fragment, got {result:?}"
        );
    }
}
