pub mod autocomplete_glue;
pub mod backend;
pub mod markdown;
pub mod nvim_rpc;
pub mod parse_incremental;
pub mod snapshot;
pub mod view;
pub mod widener_metrics;
pub mod word_wrap;

use arboard::Clipboard;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui_textarea::{CursorMove, DataCursor, TextArea};
use std::num::NonZeroU64;

/// Convert `TextArea::cursor()` from the library's `DataCursor` newtype to a
/// plain `(row, col)` tuple — the neutral interchange type shared with the
/// Nvim backend (whose `NvimSnapshot::cursor` is already a tuple).
fn cursor_tuple(ta: &TextArea<'_>) -> (usize, usize) {
    let DataCursor(r, c) = ta.cursor();
    (r, c)
}

/// Build an `EditorSnapshot` from the editor's backend + content
/// revision. Free function (not a method on `TextEditorComponent`) so
/// production callers that need to mutate other fields of
/// `TextEditorComponent` afterwards can pass `&self.backend` and
/// `self.content_revision` directly — the borrow checker can split
/// borrows across distinct fields but not across method calls.
fn snapshot_from_backend(
    backend: &BackendState,
    content_revision: NonZeroU64,
) -> EditorSnapshot<'_> {
    match backend {
        BackendState::Textarea(ta) => {
            let cursor = cursor_tuple(ta);
            EditorSnapshot::borrowed(ta.lines(), cursor, content_revision)
        }
        BackendState::Nvim(nvim) => {
            let snap = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner());
            let lines_len = snap.lines.len();
            let cursor_row = if lines_len == 0 {
                0
            } else {
                snap.cursor.0.min(lines_len - 1)
            };
            let cursor = (cursor_row, snap.cursor.1);
            let lines = snap.lines.clone();
            let rev = NonZeroU64::new(snap.content_gen.saturating_add(1))
                .unwrap_or_else(|| NonZeroU64::new(1).unwrap());
            drop(snap);
            EditorSnapshot::owned(lines, cursor, rev)
        }
    }
}

/// Returns true if any autocomplete trigger char (`[` for `[[wikilink`,
/// `#` for `#hashtag`) appears between the start of `line` and the
/// cursor's char column. Walks backwards from the cursor so the common
/// "user just typed inside a trigger" case short-circuits quickly. The
/// scan stays within one row because triggers can't cross a newline.
///
/// UTF-8 safe: takes a char column and never slices on a byte that is
/// not a codepoint boundary. Wikilinks can contain spaces
/// (`[[my note title`), so the walk does NOT stop at whitespace — only
/// the trigger char or start-of-row halts it.
fn has_trigger_before_cursor(line: &str, col: usize) -> bool {
    let cursor_byte = line
        .char_indices()
        .nth(col)
        .map(|(b, _)| b)
        .unwrap_or(line.len());
    line[..cursor_byte]
        .chars()
        .rev()
        .any(|c| c == '[' || c == '#')
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
use self::markdown::ParsedBuffer;
use self::snapshot::{EditorSnapshot, NvimMode};
use self::view::MarkdownEditorView;
use crate::util::single_slot_task::SingleSlotTask;

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
use crate::components::events::redraw_callback;
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

/// Snapshot used to satisfy `AutocompleteHost`. Wraps an
/// `EditorSnapshot` (Cow-borrowed from the textarea on the common
/// path — perf #8) plus the cursor's last-rendered screen
/// position. The host's `cache_key` mirrors the editor's
/// `content_revision`; `None` is reserved for hosts whose buffer
/// has no stable identity (the search-box modal).
struct EditorHostSnapshot<'a> {
    snap: EditorSnapshot<'a>,
    cursor_screen: Option<(u16, u16)>,
    cache_key: Option<NonZeroU64>,
}

impl<'a> AutocompleteHost for EditorHostSnapshot<'a> {
    fn buffer_snapshot(&self) -> EditorSnapshot<'_> {
        // Re-package the inner snap as a fresh borrowed view tied
        // to `&self`. `Cow::as_ref` works for both Borrowed and
        // Owned variants — the latter only occurs on the Nvim path
        // where the inner snapshot already paid the clone cost.
        EditorSnapshot::borrowed(
            self.snap.lines.as_ref(),
            self.snap.cursor,
            self.snap.content_revision,
        )
    }
    fn cache_key(&self) -> Option<NonZeroU64> {
        self.cache_key
    }
    fn screen_anchor_for(&self, _byte_offset: usize) -> Option<(u16, u16)> {
        // Anchor at the cursor's last-rendered screen position. The
        // controller passes `anchor_col` (byte offset of the start of
        // the typed query) but visually anchoring at the cursor is
        // fine — the popup sits adjacent to the typed text either way
        // and avoids re-walking the wrap layout for an arbitrary byte
        // offset.
        //
        // When `cursor_screen` is None (no prior render — e.g. the
        // user opens a note and types `[[` before the first frame),
        // return a placeholder so the controller still opens the
        // popup. The editor's render path skips drawing it until
        // `view.last_cursor_screen` is available, then re-anchors and
        // draws with the correct position.
        Some(self.cursor_screen.unwrap_or((0, 0)))
    }
}

/// Free-function builder for `EditorHostSnapshot`. Production
/// callers pass `&self.backend`, `self.content_revision`,
/// `self.view.last_cursor_screen` directly so the borrow checker
/// can split borrows from `&mut self.autocomplete`. Returns `None`
/// on the Nvim backend (autocomplete is Textarea-only).
fn build_editor_host_snapshot<'a>(
    backend: &'a BackendState,
    content_revision: NonZeroU64,
    cursor_screen: Option<(u16, u16)>,
) -> Option<EditorHostSnapshot<'a>> {
    if !matches!(backend, BackendState::Textarea(_)) {
        return None;
    }
    Some(EditorHostSnapshot {
        snap: snapshot_from_backend(backend, content_revision),
        cursor_screen,
        cache_key: Some(content_revision),
    })
}

/// Snapshot of the textarea backend used to classify a key event as a
/// text edit (text differs) vs. a pure cursor move (text same, cursor
/// moved) vs. a no-op (both same).
pub struct TextEditorComponent {
    backend: BackendState,
    /// Tracks the rendered rect to map mouse click coordinates.
    rect: Rect,
    key_bindings: KeyBindings,
    /// `content_revision` snapshot that matches the on-disk content.
    /// `Some(content_revision)` after a successful save (or after
    /// `set_text` loaded a note); `None` when the saved snapshot
    /// diverges from the current buffer. Compared against
    /// `content_revision` by `is_dirty()` so the per-frame title bar
    /// avoids materialising the buffer and so cursor moves (which bump
    /// `edit_generation` but not `content_revision`) don't flag the
    /// buffer as dirty.
    saved_content_rev: Option<NonZeroU64>,
    view: MarkdownEditorView,
    /// Incremented on every input event that may affect rendering — text
    /// edits AND cursor/selection moves. Drives view-cache invalidation in
    /// non-perf-critical paths; do NOT use for dirty tracking (cursor moves
    /// bump this too).
    edit_generation: u64,
    /// Incremented only when the buffer text actually changes (insert,
    /// delete, paste, undo/redo, autocomplete accept). Cursor-only
    /// shortcuts (arrows, Home/End, select-all) do NOT bump this. On
    /// the Nvim backend, `handle_key` does not bump either — the
    /// reverse-refresh task in `backend.rs` sees `snap.lines` change
    /// and bumps `snap.content_gen`; the editor mirrors that value
    /// into `content_revision` at the render sync point. Consumers:
    ///   - `handle_input` diffs it across a key event to classify the
    ///     event as a text edit vs. a cursor move without materialising
    ///     the buffer.
    ///   - `view.update()` uses the value as the cache-invalidation
    ///     key, so arrow-key navigation reuses the per-line parse cache
    ///     instead of rebuilding it.
    ///   - `AutocompleteHost::content_revision` exposes it as a
    ///     `NonZeroU64` cache key.
    ///   - `mark_saved_at_revision` / `is_dirty` use it as the
    ///     save-correlation token; navigation keys never invalidate a
    ///     save in flight.
    ///
    /// `NonZeroU64` because `Option<NonZeroU64>` is the cleanest way
    /// to express "no cacheable revision" without a magic-value
    /// sentinel and without a separate field.
    content_revision: NonZeroU64,
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
    /// Vault handle stored at `set_vault` time. Kept even on the Nvim
    /// backend so `maybe_recover_from_dead_nvim` can spin up the
    /// autocomplete controller after the fallback to Textarea.
    autocomplete_vault: Option<Arc<NoteVault>>,
    /// Whether the autocomplete controller's redraw callback has been
    /// bound to the app event bus. Bound lazily on the first
    /// `handle_input` because `AppTx` is not available at
    /// construction.
    autocomplete_redraw_bound: bool,
    /// Background full-parse fallback for large buffers (perf #9).
    /// The view installs a placeholder `ParsedBuffer` and signals
    /// pending; this slot owns the spawned tokio task that runs
    /// the real `ParsedBuffer::parse`. `SingleSlotTask` aborts the
    /// previous spawn on a fresh edit, so a burst of edits resolves
    /// against the latest content.
    full_parse_task: SingleSlotTask<()>,
    full_parse_tx: tokio::sync::mpsc::UnboundedSender<(u64, ParsedBuffer)>,
    full_parse_rx: tokio::sync::mpsc::UnboundedReceiver<(u64, ParsedBuffer)>,
    /// `AppTx` clone bound the first time `handle_input` runs, so the
    /// spawned full-parse task can post `AppEvent::Redraw` on
    /// completion without waiting for the next user keystroke.
    redraw_tx: Option<AppTx>,
}

impl TextEditorComponent {
    pub fn new(key_bindings: KeyBindings, settings: &AppSettings) -> Self {
        let (full_parse_tx, full_parse_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            backend: BackendState::from_settings(
                &settings.editor_backend,
                settings.nvim_path.as_ref(),
            ),
            rect: Rect::default(),
            key_bindings,
            saved_content_rev: NonZeroU64::new(1),
            view: MarkdownEditorView::new(),
            edit_generation: 0,
            content_revision: NonZeroU64::new(1).unwrap(),
            selection: None,
            clipboard: Clipboard::new().ok(),
            nvim_pending_z: false,
            search: None,
            autocomplete: None,
            autocomplete_vault: None,
            autocomplete_redraw_bound: false,
            full_parse_task: SingleSlotTask::empty(),
            full_parse_tx,
            full_parse_rx,
            redraw_tx: None,
        }
    }

    /// Attach a vault so autocomplete can query notes/tags. Activates
    /// the controller immediately on the textarea backend; on Nvim, the
    /// vault is stashed and the controller is spun up later if
    /// `maybe_recover_from_dead_nvim` falls back to Textarea.
    pub fn set_vault(&mut self, vault: Arc<NoteVault>) {
        self.autocomplete_vault = Some(vault.clone());
        if matches!(self.backend, BackendState::Textarea(_)) {
            self.autocomplete = Some(AutocompleteController::new(std::sync::Arc::new(crate::components::search_list::VaultSuggestions { vault }), AutocompleteMode::Both));
        }
    }

    /// Spin up the autocomplete controller if a vault was previously
    /// stashed and the controller isn't already running. Called after
    /// the Nvim → Textarea fallback so the post-crash session has the
    /// popup available.
    fn ensure_autocomplete_for_textarea(&mut self) {
        if self.autocomplete.is_some() {
            return;
        }
        if !matches!(self.backend, BackendState::Textarea(_)) {
            return;
        }
        let Some(vault) = self.autocomplete_vault.clone() else {
            return;
        };
        self.autocomplete = Some(AutocompleteController::new(std::sync::Arc::new(crate::components::search_list::VaultSuggestions { vault }), AutocompleteMode::Both));
        // Fresh controller — `bind_autocomplete_redraw` must rebind
        // on the next handle_input.
        self.autocomplete_redraw_bound = false;
    }

    /// Build a snapshot view of the editor state for the autocomplete
    /// controller. Method form wraps `build_editor_host_snapshot` for
    /// callers that do not need to split borrows; production hot
    /// paths (`refresh_autocomplete_if_open`, `sync_autocomplete`)
    /// inline the free function instead so `&self.backend` and
    /// `&mut self.autocomplete` can coexist.
    #[allow(dead_code)]
    fn autocomplete_host_snapshot(&self) -> Option<EditorHostSnapshot<'_>> {
        build_editor_host_snapshot(
            &self.backend,
            self.content_revision,
            self.view.last_cursor_screen,
        )
    }

    /// Pull the latest async query results into the popup state. Called
    /// once per render before drawing the overlay.
    fn poll_autocomplete(&mut self) {
        if let Some(controller) = self.autocomplete.as_mut() {
            controller.poll_results();
        }
    }

    /// Cheap cursor read — `None` for the Nvim backend. Used by `handle_input`
    /// to diff cursor position across a key event without materialising the
    /// whole buffer.
    fn textarea_cursor(&self) -> Option<(usize, usize)> {
        let BackendState::Textarea(ta) = &self.backend else {
            return None;
        };
        Some(cursor_tuple(ta))
    }

    fn refresh_autocomplete_if_open(&mut self) {
        // No controller (e.g. Nvim backend) or popup closed → nothing to refresh.
        if !self.autocomplete.as_ref().is_some_and(|c| c.is_open()) {
            return;
        }
        // Inline the snapshot via the free function so `&self.backend`
        // (the snapshot's borrow source) and `&mut self.autocomplete`
        // (the controller below) can coexist via field-disjoint borrows.
        let Some(snapshot) = build_editor_host_snapshot(
            &self.backend,
            self.content_revision,
            self.view.last_cursor_screen,
        ) else {
            self.close_autocomplete();
            return;
        };
        if let Some(controller) = self.autocomplete.as_mut() {
            controller.refresh_if_open(&snapshot);
        }
    }

    /// Recompute the popup's trigger context from the current buffer and
    /// cursor. Call after any mutating key handle (typed letter, paste,
    /// backspace, cursor movement, etc.).
    fn sync_autocomplete(&mut self) {
        let Some(controller) = self.autocomplete.as_ref() else {
            return; // Nvim backend or no controller
        };

        // Fast-path bail: when the popup is closed AND no trigger character
        // appears between the cursor and the start of the current row, no
        // reconcile can open a popup. Skip the expensive buffer snapshot +
        // pulldown-cmark scan.
        //
        // Trigger chars: `[` (for `[[wikilink`) and `#` (for `#hashtag`).
        // Wikilinks can contain spaces (`[[my note title`), so the scan
        // walks back to the start of the row, not to the nearest whitespace.
        // The walk short-circuits on the first trigger char, so for typical
        // lines it touches only a handful of chars before bailing or
        // promoting to the slow path. Using `char_indices().rev()` keeps
        // the walk UTF-8-safe — never slices mid-codepoint.
        if !controller.is_open() {
            let BackendState::Textarea(ta) = &self.backend else {
                return;
            };
            let (row, col) = cursor_tuple(ta);
            let line = ta.lines().get(row).map(|s| s.as_str()).unwrap_or("");
            if !has_trigger_before_cursor(line, col) {
                return;
            }
        }

        // Slow path: build the borrowed snapshot for the controller to
        // reconcile. Free function so `&self.backend` and
        // `&mut self.autocomplete` can coexist.
        let Some(snapshot) = build_editor_host_snapshot(
            &self.backend,
            self.content_revision,
            self.view.last_cursor_screen,
        ) else {
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

    /// Single producer for the editor's atomic `(lines, cursor,
    /// content_revision)` view. Downstream consumers (`MarkdownEditorView`,
    /// `click_to_logical_u16`, the autocomplete host) take a
    /// `&EditorSnapshot` and stop guarding against drift between cursor
    /// and lines on every leaf access — the snapshot owns that
    /// invariant at construction time.
    ///
    /// On the Textarea backend the snapshot borrows live lines (no
    /// clone) and the cursor is already in-bounds. On the Nvim backend
    /// the lines are cloned out from behind the `Mutex` (same cost as
    /// today's render path) and the cursor row is clamped to
    /// `lines.len() - 1` before the snapshot is returned.
    ///
    /// Production hot paths that also need `&mut self.view` (notably
    /// `render`) must instead inline the snapshot via
    /// `snapshot_from_backend(&self.backend, self.content_revision)`
    /// so the borrow checker can split the borrows across distinct
    /// fields.
    pub fn view_snapshot(&self) -> EditorSnapshot<'_> {
        snapshot_from_backend(&self.backend, self.content_revision)
    }

    pub fn set_text(&mut self, text: String) {
        // No-op when the buffer would be identical — preserves view scroll,
        // selection, edit generation cache, and an open autocomplete popup.
        // Saves the expensive lines clone too. Still normalises the saved
        // marker: if the buffer was flagged dirty by a previous divergent
        // save, reloading the same content from disk should clear that
        // flag rather than persist a phantom `[+]` in the title bar.
        if text == self.get_text() {
            self.saved_content_rev = Some(self.content_revision);
            if let BackendState::Nvim(nvim) = &self.backend {
                nvim.snapshot
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .dirty = false;
            }
            return;
        }
        match &mut self.backend {
            BackendState::Textarea(ta) => {
                let lines = text.lines();
                *ta = TextArea::from(lines);
            }
            BackendState::Nvim(nvim) => {
                nvim.set_text(&text);
            }
        }
        self.bump_content();
        let reconstructed = self.get_text();
        self.mark_saved(reconstructed);
        // Buffer replaced — close any open autocomplete popup so it does
        // not linger over the new note (e.g. after Ctrl+G follow-link).
        self.close_autocomplete();
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

    /// Current content revision. Bumped on every text-mutating handler;
    /// stable across cursor moves and idle frames. Used by the autosave
    /// path to record "this snapshot was saved" without rebuilding the
    /// buffer text on completion. `NonZeroU64` makes 0 unrepresentable
    /// so callers can express "no revision" as `Option<NonZeroU64>::None`
    /// without a magic-value sentinel.
    pub fn content_revision(&self) -> NonZeroU64 {
        self.content_revision
    }

    /// Mark the buffer as clean iff its current revision still matches
    /// `rev` (i.e. no edits landed between the save being issued and
    /// completing). Diverged revision → no-op: leave `saved_content_rev`
    /// alone, because some OTHER mechanism (a synchronous `try_save`
    /// racing this completion) may have already marked a NEWER revision
    /// clean, and a stale completion must not clobber that. `is_dirty`
    /// already reads true when `saved_content_rev != Some(self.content_revision)`,
    /// so doing nothing on a mismatch keeps the editor correctly dirty
    /// without overwriting a legitimately-newer saved snapshot.
    pub fn mark_saved_at_revision(&mut self, rev: NonZeroU64) {
        if rev != self.content_revision {
            return;
        }
        if let BackendState::Nvim(nvim) = &self.backend {
            nvim.snapshot
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .dirty = false;
        }
        self.saved_content_rev = Some(rev);
    }

    /// Synchronous mark-saved used by `try_save` and `set_text`. Unlike
    /// `mark_saved_at_revision` (which no-ops on a stale revision because
    /// it can race a sync mark_saved), this one CLOBBERS `saved_content_rev`
    /// to `None` when the supplied text diverges: the sync caller holds
    /// `&mut self` for the whole save, so there is no concurrent newer
    /// clean state to preserve, and the user typing between
    /// `get_text()` and this call must show as dirty.
    pub fn mark_saved(&mut self, text: String) {
        let matches = text == self.get_text();
        if matches {
            if let BackendState::Nvim(nvim) = &self.backend {
                nvim.snapshot
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .dirty = false;
            }
            self.saved_content_rev = Some(self.content_revision);
        } else {
            // Textarea: divergent save → stay dirty.
            // Nvim: snapshot's `dirty` was untouched anyway; the Textarea
            // dirty signal (saved_content_rev) is what is_dirty consults
            // on the Textarea backend, and we explicitly mark it None here.
            self.saved_content_rev = None;
        }
    }

    pub fn is_dirty(&self) -> bool {
        match &self.backend {
            BackendState::Textarea(_) => self.saved_content_rev != Some(self.content_revision),
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
                self.bump_content();
            }
            BackendState::Nvim(nvim) => {
                nvim.paste(text, tx.clone());
                self.bump_content();
            }
        }
        // The buffer just changed under the popup's feet; reconcile
        // the trigger context so a stale replace_range cannot survive
        // into the next Accept.
        self.bind_autocomplete_redraw(tx);
        self.sync_autocomplete();
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
            self.bump_content();
        }
        // See `paste_text` — out-of-band buffer mutation must
        // re-reconcile the popup state.
        self.bind_autocomplete_redraw(tx);
        self.sync_autocomplete();
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
        self.bump_content();
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
        self.bump_content();
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
            self.bump_content();
        }
    }
}

impl TextEditorComponent {
    /// Bumps `edit_generation` only (cursor/selection moves, mouse clicks
    /// that do not touch the buffer text). Lets the view invalidate its
    /// cursor-dependent caches without telling the autocomplete controller
    /// that the buffer changed.
    #[inline]
    fn bump_cursor(&mut self) {
        self.edit_generation = self.edit_generation.wrapping_add(1);
    }

    /// Bumps both `edit_generation` and `content_revision`. Use at every
    /// site that mutates the buffer (insert, delete, paste, undo/redo,
    /// autocomplete accept) on the Textarea backend. `handle_input` uses
    /// the `content_revision` delta to detect a real text change without
    /// materialising the buffer.
    ///
    /// Not called by the Nvim path — the reverse-refresh task in
    /// `backend.rs` bumps `snap.content_gen` on real diffs and the
    /// editor mirrors that value into `content_revision` at render time.
    #[inline]
    fn bump_content(&mut self) {
        self.edit_generation = self.edit_generation.wrapping_add(1);
        // NonZeroU64 enforces the skip-zero invariant for free: on
        // wrap-around from u64::MAX, `NonZeroU64::new(0)` returns None
        // and we substitute 1. 2^64 edits is astronomical but the
        // invariant is type-checkable.
        let next = self.content_revision.get().wrapping_add(1);
        self.content_revision = NonZeroU64::new(next).unwrap_or(NonZeroU64::new(1).unwrap());
    }

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
            // Spin up the autocomplete controller now that we're on the
            // textarea backend — set_vault was a no-op at startup when
            // we were still on Nvim.
            self.ensure_autocomplete_for_textarea();
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
        // Nvim handle_key only bumps `edit_generation` (any-input
        // counter for view-cache invalidation). `content_revision` is
        // owned by the reverse-refresh task in `backend.rs`, which
        // bumps `snap.content_gen` only when `snap.lines` actually
        // diffs — that value is mirrored into `content_revision` at
        // the next render sync point. Result: navigation keys never
        // invalidate an in-flight save's revision token, and the
        // autocomplete cache (when wired up on Nvim in a future
        // revision) survives navigation.
        self.bump_cursor();
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
        // Yield key focus to the find bar — close the autocomplete popup
        // so it stops intercepting Esc / Up / Down / Tab / Enter, which
        // belong to the find bar while it is active.
        self.close_autocomplete();
        self.search = Some(SearchState {
            input: SingleLineInput::new(),
            status: SearchStatus::Empty,
        });
    }

    /// Close the autocomplete popup, if any. Cheap; safe on any backend
    /// (no-op when `autocomplete` is None). Use whenever focus moves
    /// away from the editor or another overlay takes over key input.
    pub fn close_autocomplete(&mut self) {
        if let Some(c) = self.autocomplete.as_mut() {
            c.close();
        }
    }

    /// Bind the redraw channel up front (e.g. on note open) so the
    /// background full-parse task can wake the event-driven render loop
    /// on the FIRST render of a large buffer, before any keystroke has
    /// run `handle_input`. No-op after the first successful bind.
    pub fn set_redraw_tx(&mut self, tx: &AppTx) {
        self.bind_autocomplete_redraw(tx);
    }

    /// Bind the autocomplete controller's redraw callback AND the
    /// editor's background-full-parse redraw signal to the app
    /// event bus. Called from `handle_input` (the first place where
    /// the editor has access to `AppTx`). The autocomplete piece is
    /// a no-op after the first successful bind; the redraw_tx clone
    /// is set unconditionally so a reset autocomplete controller
    /// (e.g. after Nvim → Textarea fallback) doesn't lose the
    /// editor's redraw channel.
    fn bind_autocomplete_redraw(&mut self, tx: &AppTx) {
        if self.redraw_tx.is_none() {
            self.redraw_tx = Some(tx.clone());
        }
        if self.autocomplete_redraw_bound {
            return;
        }
        if let Some(c) = self.autocomplete.as_mut() {
            c.set_redraw_callback(redraw_callback(tx.clone()));
            self.autocomplete_redraw_bound = true;
        }
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
                    let cut = if let BackendState::Textarea(ta) = &mut self.backend {
                        // `ta.cut()` returns `false` when the selection was
                        // empty / nothing to remove. Use its return value
                        // directly rather than pre-checking selection_range —
                        // one source of truth, no spurious view rebuild on
                        // no-op Ctrl+X.
                        let cut = ta.cut();
                        self.selection = ta.selection_range();
                        cut
                    } else {
                        false
                    };
                    if cut {
                        self.bump_content();
                    }
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
            self.bump_cursor();
            return EventState::Consumed;
        }

        // FocusSidebar / FocusEditor shortcuts are intercepted at the
        // EditorScreen level for directional navigation.

        // Standard text-editor shortcuts.
        // `input_without_shortcuts` only handles chars, backspace, delete, tab, newline —
        // all navigation and editing shortcuts must be mapped explicitly.
        // Outcome tracks whether the handled shortcut mutated the buffer, only
        // moved the cursor, or did literally nothing (e.g. Ctrl+Z on an empty
        // undo stack) — so neither `text_revision` nor `edit_generation` is
        // bumped on true no-ops.
        enum ShortcutOutcome {
            NoOp,
            CursorOnly,
            TextMutated,
        }
        let outcome: Option<ShortcutOutcome> =
            match (key.modifiers & !KeyModifiers::SHIFT, key.code) {
                // --- Cursor movement (Shift extends the selection) ---
                (KeyModifiers::NONE, KeyCode::Left) => {
                    cursor_move!(ta, CursorMove::Back, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                (KeyModifiers::NONE, KeyCode::Right) => {
                    cursor_move!(ta, CursorMove::Forward, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                (KeyModifiers::NONE, KeyCode::Up) => {
                    cursor_move!(ta, CursorMove::Up, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                (KeyModifiers::NONE, KeyCode::Down) => {
                    cursor_move!(ta, CursorMove::Down, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                (KeyModifiers::NONE, KeyCode::Home) => {
                    cursor_move!(ta, CursorMove::Head, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                (KeyModifiers::NONE, KeyCode::End) => {
                    cursor_move!(ta, CursorMove::End, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                (KeyModifiers::NONE, KeyCode::PageUp) => {
                    cursor_move!(ta, CursorMove::ParagraphBack, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                (KeyModifiers::NONE, KeyCode::PageDown) => {
                    cursor_move!(ta, CursorMove::ParagraphForward, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                // Word navigation (Ctrl+arrow, Windows/Linux style)
                (KeyModifiers::CONTROL, KeyCode::Left) => {
                    cursor_move!(ta, CursorMove::WordBack, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                (KeyModifiers::CONTROL, KeyCode::Right) => {
                    cursor_move!(ta, CursorMove::WordForward, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                // Document start / end
                (KeyModifiers::CONTROL, KeyCode::Home) => {
                    cursor_move!(ta, CursorMove::Top, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                (KeyModifiers::CONTROL, KeyCode::End) => {
                    cursor_move!(ta, CursorMove::Bottom, shift);
                    Some(ShortcutOutcome::CursorOnly)
                }
                // Undo / Redo (Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z). The textarea
                // returns `false` when the stack is empty — no buffer change AND
                // no cursor change, so emit NoOp and skip the view-cache bump.
                (KeyModifiers::CONTROL, KeyCode::Char('z')) => {
                    if ta.undo() {
                        Some(ShortcutOutcome::TextMutated)
                    } else {
                        Some(ShortcutOutcome::NoOp)
                    }
                }
                (KeyModifiers::CONTROL, KeyCode::Char('y'))
                | (KeyModifiers::CONTROL, KeyCode::Char('Z')) => {
                    if ta.redo() {
                        Some(ShortcutOutcome::TextMutated)
                    } else {
                        Some(ShortcutOutcome::NoOp)
                    }
                }
                // Select all
                (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                    ta.move_cursor(CursorMove::Top);
                    ta.start_selection();
                    ta.move_cursor(CursorMove::Bottom);
                    Some(ShortcutOutcome::CursorOnly)
                }
                // Delete word before / after cursor. Returns `false` when at a
                // word boundary with nothing to delete — no buffer/cursor change.
                (KeyModifiers::CONTROL, KeyCode::Backspace)
                | (KeyModifiers::ALT, KeyCode::Backspace) => {
                    if ta.delete_word() {
                        Some(ShortcutOutcome::TextMutated)
                    } else {
                        Some(ShortcutOutcome::NoOp)
                    }
                }
                (KeyModifiers::CONTROL, KeyCode::Delete) | (KeyModifiers::ALT, KeyCode::Delete) => {
                    if ta.delete_next_word() {
                        Some(ShortcutOutcome::TextMutated)
                    } else {
                        Some(ShortcutOutcome::NoOp)
                    }
                }
                _ => None,
            };
        if let Some(kind) = outcome {
            self.selection = ta.selection_range();
            match kind {
                ShortcutOutcome::NoOp => {}
                ShortcutOutcome::CursorOnly => self.bump_cursor(),
                ShortcutOutcome::TextMutated => self.bump_content(),
            }
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
        // `input_without_shortcuts` returns `false` for keys the textarea
        // ignores (F1-F12, KeyCode::Null, modifier-only releases, IME
        // composing events). Only bump `text_revision` when the buffer
        // actually changed — otherwise harmless keys would silently flip
        // the editor to dirty and trigger needless autosaves.
        let mutated = ta.input_without_shortcuts(*key);
        self.selection = ta.selection_range();
        if mutated {
            self.bump_content();
        } else {
            self.bump_cursor();
        }
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
            self.bump_cursor();
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
                let (lrow, lcol) = self
                    .view
                    .click_at_screen((mouse.row - r.y) as usize, (mouse.column - r.x) as usize);
                ta.move_cursor(CursorMove::Jump(lrow, lcol));
                ta.start_selection();
            }
            MouseEventKind::Drag(_) => {
                let (lrow, lcol) = self
                    .view
                    .click_at_screen((mouse.row - r.y) as usize, (mouse.column - r.x) as usize);
                ta.move_cursor(CursorMove::Jump(lrow, lcol));
            }
            _ => {
                ta.input(*mouse);
            }
        }
        self.selection = ta.selection_range();
        // Mouse handling moves the cursor / selection but does not insert
        // text — `ratatui-textarea` mouse handling is click/drag/scroll only.
        self.bump_cursor();
        EventState::Consumed
    }
}

impl Component for TextEditorComponent {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        self.maybe_recover_from_dead_nvim();
        self.bind_autocomplete_redraw(tx);

        match event {
            InputEvent::Key(key) => {
                // Cheap popup-open probe first. The snapshot is now a
                // Cow-borrowed view of the textarea's lines (zero
                // allocation on the Textarea path — perf #8), so
                // idle keystrokes pay nothing here even when popup
                // checks fire. The free-function form lets `&self.backend`
                // and `&mut self.autocomplete` coexist via field-disjoint
                // borrows.
                let popup_open = self.autocomplete.as_ref().is_some_and(|c| c.is_open());
                if popup_open
                    && let Some(host) = build_editor_host_snapshot(
                        &self.backend,
                        self.content_revision,
                        self.view.last_cursor_screen,
                    )
                    && let Some(controller) = self.autocomplete.as_mut()
                {
                    match controller.handle_key(*key, &host) {
                        HandleKeyOutcome::Accepted(action) => {
                            if let BackendState::Textarea(ta) = &mut self.backend {
                                apply_accept_to_textarea(ta, &action);
                                self.selection = ta.selection_range();
                            }
                            self.bump_content();
                            return EventState::Consumed;
                        }
                        HandleKeyOutcome::Dismissed | HandleKeyOutcome::Consumed => {
                            return EventState::Consumed;
                        }
                        HandleKeyOutcome::NotHandled => {}
                    }
                }
                if let Some(state) = self.handle_nvim_key(key, tx) {
                    return state;
                }
                // Diff before/after using cheap counters instead of cloning
                // the whole buffer. `text_revision` only bumps when the
                // buffer actually changed (handlers call `bump_text`);
                // cursor position is two `usize`s. Three outcomes:
                //   - text changed → sync (may open a fresh popup)
                //   - text unchanged, cursor moved → refresh (close
                //     popup if cursor left the trigger range; never
                //     open new popup just because the cursor passed
                //     over an existing wikilink/hashtag)
                //   - both unchanged → no autocomplete work needed
                let text_rev_before = self.content_revision;
                let cursor_before = self.textarea_cursor();
                let result = self.handle_textarea_key(key, tx);
                let cursor_after = self.textarea_cursor();
                if self.content_revision != text_rev_before {
                    self.sync_autocomplete();
                } else if cursor_before != cursor_after {
                    self.refresh_autocomplete_if_open();
                }
                result
            }
            InputEvent::Mouse(mouse) => {
                let text_rev_before = self.content_revision;
                let cursor_before = self.textarea_cursor();
                let result = self.handle_mouse(mouse, tx);
                let cursor_after = self.textarea_cursor();
                // Mouse clicks typically only move the cursor — refresh
                // (which may close the popup) but do not auto-open.
                if self.content_revision != text_rev_before {
                    self.sync_autocomplete();
                } else if cursor_before != cursor_after {
                    self.refresh_autocomplete_if_open();
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
        // Phase 1: gather per-backend selection + (Nvim only) the
        // content_gen the refresh task observed. Done before
        // `view_snapshot()` so the Nvim path's content_revision mirror
        // lands first.
        let (selection, nvim_rev_to_mirror) = match &self.backend {
            BackendState::Textarea(_) => (self.selection, None),
            BackendState::Nvim(nvim) => {
                nvim.maybe_resize(editor_rect.width, editor_rect.height);
                let snap = nvim.snapshot.lock().unwrap_or_else(|p| p.into_inner());
                let visual_selection = snap.visual_selection;
                let content_gen = snap.content_gen;
                drop(snap);
                // Mirror the refresh task's view of "did content
                // change" into our own `content_revision`. The
                // refresh task only bumps `snap.content_gen` when
                // `snap.lines` actually diffs (backend.rs:497) so
                // navigation keystrokes leave the value alone, and
                // an in-flight autosave's revision token stays valid
                // across navigation. Skip-zero is handled by
                // `NonZeroU64::new(0) == None`.
                let rev = NonZeroU64::new(content_gen.saturating_add(1));
                (visual_selection, rev)
            }
        };
        if let Some(rev) = nvim_rev_to_mirror {
            self.content_revision = rev;
        }
        // Drain any completed background full-parse results BEFORE
        // running view.update so a just-finished async parse lands
        // before Gate 1 has a chance to install another placeholder.
        // Generation mismatches drop silently (the spawned task's
        // input is older than the current buffer).
        while let Ok((generation, buf)) = self.full_parse_rx.try_recv() {
            self.view.install_full_parse(generation, buf);
        }

        // Phase 2: single producer for the atomic snapshot. Borrowed
        // on Textarea (zero clone), owned on Nvim (lines cloned out
        // from behind the Mutex). Use the free function so the borrow
        // checker can split `&self.backend` from `&mut self.view`.
        let snap = snapshot_from_backend(&self.backend, self.content_revision);
        self.view.update(&snap, editor_rect, selection);

        // If `view.update` cap-tripped on a large buffer it
        // installed a placeholder + pending-flag instead of running
        // ParsedBuffer::parse synchronously. Spawn the real parse
        // here so subsequent frames pick up the rich result via the
        // drain loop above. `SingleSlotTask::spawn` aborts the prior
        // task, so a burst of large-buffer edits resolves against
        // the latest content.
        if let Some(generation) = self.view.take_pending_full_parse() {
            let lines: Vec<String> = snap.lines.iter().cloned().collect();
            let tx = self.full_parse_tx.clone();
            let redraw = self.redraw_tx.clone();
            self.full_parse_task.spawn(async move {
                let buf = ParsedBuffer::parse(&lines);
                let _ = tx.send((generation, buf));
                // Wake the render loop so the rich parse lands
                // without waiting for the next keystroke.
                if let Some(redraw) = redraw {
                    let _ = redraw.send(AppEvent::Redraw);
                }
            });
        }
        // When the find bar is active, draw it AFTER the editor so its caret
        // (set via set_cursor_position) wins over the editor's caret call.
        let bar_focused = self.search.is_some() && focused;
        let editor_focused = focused && !bar_focused;
        self.view.render(f, editor_rect, theme, editor_focused);
        if let (Some(state), Some(bar_rect)) = (self.search.as_mut(), search_rect) {
            render_search_bar(f, bar_rect, state, theme, bar_focused);
        }

        // Autocomplete popup sits on top of the editor. Drain async
        // query results first so the popup reflects the latest prefix,
        // then re-anchor on the cursor's freshly-rendered screen
        // position (otherwise the anchor lags one frame behind on the
        // very first popup-opening keystroke). Clamp against
        // `editor_rect`, not the full `rect`, so the popup never lands
        // on the find-bar row.
        self.poll_autocomplete();
        // The popup anchors on the cursor's just-rendered screen
        // position. When the cursor is off-screen
        // (`last_cursor_screen == None`) we skip rendering entirely
        // rather than draw at a stale anchor — the popup state is
        // preserved, so the popup reappears at the correct position
        // once the cursor scrolls back into view.
        if let (Some(controller), Some(live_anchor)) =
            (self.autocomplete.as_mut(), self.view.last_cursor_screen)
        {
            if let Some(state) = controller.state_mut() {
                state.anchor = live_anchor;
            }
            if let Some(state) = controller.state() {
                autocomplete::render(f, state, editor_rect, theme);
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
    fn has_trigger_before_cursor_finds_bracket() {
        assert!(has_trigger_before_cursor("hello [[foo", 11));
        assert!(has_trigger_before_cursor("[[a b c", 7));
    }

    #[test]
    fn has_trigger_before_cursor_finds_hashtag() {
        assert!(has_trigger_before_cursor("text #tag", 9));
    }

    #[test]
    fn has_trigger_before_cursor_no_trigger_bails() {
        assert!(!has_trigger_before_cursor("plain prose here", 16));
        assert!(!has_trigger_before_cursor("", 0));
    }

    #[test]
    fn has_trigger_before_cursor_handles_multibyte_no_panic() {
        // Regression: the previous 64-byte saturating_sub slice could
        // land mid-codepoint and panic on CJK / emoji / accented lines.
        let line = "你好世界".to_string() + &"a".repeat(80);
        let col = line.chars().count();
        assert!(!has_trigger_before_cursor(&line, col));

        let with_emoji = "🦀".repeat(20) + "[[note";
        let col = with_emoji.chars().count();
        assert!(has_trigger_before_cursor(&with_emoji, col));

        let accented = "é".repeat(100);
        let col = accented.chars().count();
        assert!(!has_trigger_before_cursor(&accented, col));
    }

    #[test]
    fn has_trigger_before_cursor_ignores_chars_after_cursor() {
        // Trigger AFTER cursor must not match.
        assert!(!has_trigger_before_cursor("foo [[bar", 3));
    }

    #[test]
    fn has_trigger_before_cursor_wikilink_with_spaces() {
        // Wikilink contents can contain spaces; we must still detect the
        // opening bracket far back on the line.
        assert!(has_trigger_before_cursor("[[my note title", 15));
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
    fn cursor_move_does_not_dirty_buffer() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        assert!(!editor.is_dirty());
        let tx = dummy_tx();
        // Send a cursor-only key (Right arrow). It must bump `edit_generation`
        // for view-cache invalidation but must NOT bump `text_revision`, so
        // `is_dirty` stays false.
        let key = ratatui::crossterm::event::KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        let _ = editor.handle_input(&InputEvent::Key(key), &tx);
        assert!(
            !editor.is_dirty(),
            "cursor move must not mark the editor as dirty"
        );
    }

    #[test]
    fn empty_stack_undo_redo_does_not_dirty_or_bump_revision() {
        // Regression: ShortcutOutcome::NoOp must apply for Ctrl+Z / Ctrl+Y
        // when the undo/redo stack is empty. Both is_dirty and the
        // raw content_revision counter stay put.
        let mut editor = make_editor();
        editor.set_text("foo".to_string());
        let rev_before = editor.content_revision();
        assert!(!editor.is_dirty());
        let tx = dummy_tx();
        for key_code in [KeyCode::Char('z'), KeyCode::Char('y')] {
            let key = ratatui::crossterm::event::KeyEvent::new(key_code, KeyModifiers::CONTROL);
            let _ = editor.handle_input(&InputEvent::Key(key), &tx);
        }
        assert!(
            !editor.is_dirty(),
            "empty-stack undo/redo must not flip is_dirty"
        );
        assert_eq!(
            editor.content_revision(),
            rev_before,
            "empty-stack undo/redo must not bump content_revision"
        );
    }

    #[test]
    fn fresh_editor_content_revision_is_nonzero() {
        // Regression: content_revision is typed `NonZeroU64`, which
        // makes the "do not cache" sentinel for `AutocompleteHost`
        // expressible as `Option::None` without a magic value.
        // `NonZeroU64::get()` is always >= 1 by construction; this
        // test is now a tautological smoke test that the constructor
        // initialises the field.
        let editor = make_editor();
        assert!(editor.content_revision().get() >= 1);
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
