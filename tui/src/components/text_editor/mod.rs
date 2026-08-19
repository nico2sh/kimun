pub mod autocomplete_glue;
pub mod backend;
pub mod find_bar;
pub mod find_replace;
pub mod markdown;
pub mod nvim_decode;
pub mod nvim_host;
pub mod nvim_rpc;
pub mod parse_incremental;
pub mod plain_keys;
mod revisions;
pub mod rope_buffer;
pub mod typing_run;
use revisions::Revisions;
pub mod snapshot;
pub mod text_coords;
pub mod view;
mod vim;
mod vim_objects;
pub mod widener_metrics;

use self::rope_buffer::CursorMove;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use std::num::NonZeroU64;

/// Convert `TextArea::cursor()` from the library's `DataCursor` newtype to a
/// plain `(row, col)` tuple — the neutral interchange type shared with the
/// Nvim backend (whose `NvimSnapshot::cursor` is already a tuple).
pub(crate) fn cursor_tuple(ta: &rope_buffer::RopeBuffer) -> (usize, usize) {
    ta.cursor()
}

/// Build an `EditorSnapshot` from the editor's backend + content
/// revision. Free function (not a method on `TextEditorComponent`) so
/// production callers that need to mutate other fields of
/// `TextEditorComponent` afterwards can pass `&self.backend` and
/// `self.revs.current()` directly — the borrow checker can split
/// borrows across distinct fields but not across method calls.
fn snapshot_from_backend(backend: &BackendState, content_revision: NonZeroU64) -> EditorSnapshot {
    match backend {
        BackendState::Textarea(tb) => {
            let cursor = cursor_tuple(&tb.ta);
            EditorSnapshot::of_buffer(tb.ta.text().clone(), cursor, content_revision)
        }
        BackendState::Nvim(nvim) => {
            let snap = nvim.snapshot();
            let lines_len = snap.lines.len();
            let cursor_row = if lines_len == 0 {
                0
            } else {
                snap.cursor.0.min(lines_len - 1)
            };
            let cursor = (cursor_row, snap.cursor.1);
            let lines = snap.lines.clone();
            let rev = Revisions::rev_from_gen(snap.content_gen);
            drop(snap);
            EditorSnapshot::owned(lines, cursor, rev)
        }
    }
}

/// Identity for a **replace preview**'s snapshot: the real content revision
/// folded together with the previewed text.
///
/// The view gates parse-cache rebuilds on `content_revision`, so a preview must
/// not reuse the buffer's — it would show a parse of text that is not on
/// screen. Deriving it from the previewed lines means an unchanged preview
/// keeps its cache entry across frames, and any change to the pattern, the
/// replacement, or the buffer produces a new one.
fn preview_revision(base: NonZeroU64, lines: &[String]) -> NonZeroU64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    base.get().hash(&mut h);
    lines.hash(&mut h);
    NonZeroU64::new(h.finish()).unwrap_or(NonZeroU64::MIN)
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

use self::backend::BackendState;
#[cfg(test)]
use self::find_bar::{BarFocus, SearchStatus};
use self::markdown::ParsedBuffer;
use self::nvim_host::NvimHost;
use self::rope_buffer::RopeBuffer;
use self::snapshot::EditorSnapshot;
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
pub(super) fn char_col_to_byte(line: &str, char_col: usize) -> usize {
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
fn selection_text(ta: &rope_buffer::RopeBuffer) -> Option<String> {
    selection_text_in(ta, ta.selection_range()?)
}

/// Like [`selection_text`] but over an explicit char-column `range` rather than
/// the textarea's live selection — lets read-only callers apply the vim
/// charwise-Visual inclusive `+1` without mutating the live selection/cursor.
fn selection_text_in(
    ta: &rope_buffer::RopeBuffer,
    range: ((usize, usize), (usize, usize)),
) -> Option<String> {
    let ((sr, sc), (er, ec)) = range;
    if sr == er && sc == ec {
        return None;
    }
    // The engine answers this directly, and checks the span against the text it
    // came from — where the row-walk it replaces assumed every index was in range.
    ta.span_between((sr, sc), (er, ec))
        .and_then(|span| ta.text().slice(span))
        .map(|text| text.into_owned())
}

/// Auto-surround pair for `c`: typing an opening pair character or a
/// symmetric one while a selection is active wraps the selection instead of
/// replacing it. Closing characters return `None` — they replace, like any
/// other key. See CONTEXT.md "Auto-surround".
fn surround_pair(c: char) -> Option<(&'static str, &'static str)> {
    match c {
        '(' => Some(("(", ")")),
        '[' => Some(("[", "]")),
        '{' => Some(("{", "}")),
        '<' => Some(("<", ">")),
        '"' => Some(("\"", "\"")),
        '\'' => Some(("'", "'")),
        '`' => Some(("`", "`")),
        '*' => Some(("*", "*")),
        '_' => Some(("_", "_")),
        '~' => Some(("~", "~")),
        _ => None,
    }
}

/// Re-establishes the textarea selection over `start..end` (char-based data
/// coordinates, as returned by `selection_range`).
///
/// Refuses rather than approximating: this used to saturate both endpoints at
/// `u16::MAX`, which on a pathologically large buffer silently selected a
/// *different* range — and callers then cut or overwrote it.
fn set_selection(ta: &mut RopeBuffer, start: (usize, usize), end: (usize, usize)) -> bool {
    let max = u16::MAX as usize;
    if start.0 > max || start.1 > max || end.0 > max || end.1 > max {
        return false;
    }
    ta.cancel_selection();
    ta.jump_to(start.0, start.1);
    ta.start_selection();
    ta.jump_to(end.0, end.1);
    true
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
/// than `core::note::scan::is_remote_url` (http/https only) because users routinely paste
/// `mailto:` and FTP links and expect them wrapped as markdown links too.
const LINKABLE_PASTE_SCHEMES: &[&str] = &["http", "https", "ftp", "ftps", "mailto"];

fn linkable_url(s: &str) -> Option<&str> {
    kimun_core::note::scan::url_with_allowed_scheme(s, LINKABLE_PASTE_SCHEMES)
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
use crate::components::text_editor::autocomplete_glue::apply_accept_to_textarea;
use crate::keys::KeyBindings;
use crate::keys::action_shortcuts::TextAction;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

/// What following resolves to — the **follow target** under the cursor.
///
/// Named for the action rather than the destination, because the destination is
/// not known here: a `Link` is still the raw string as written in the note, and
/// only `EditorScreen::follow_link` decides whether it names a vault note, an
/// external URL, or an attachment. Wider than a **note link**, which is
/// note→note only.
#[derive(Debug, Clone, PartialEq)]
pub enum FollowTarget {
    /// A wiki-link or markdown link, with the raw target string.
    Link(String),
    /// A hashtag label with the name **without** the leading `#`.
    Label(String),
}

/// Which editor-internal surface currently holds input — the **editor claim**.
///
/// Read into the `Intent` classifier's snapshot so ownership is decided once,
/// there, instead of being re-asserted per event kind further down. The holder
/// is named rather than merely counted because what a claim blocks differs by
/// holder: the find bar blocks a paste, a click and a bare Space; the popup
/// wants all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorClaim {
    #[default]
    None,
    FindBar,
    Autocomplete,
}

/// Snapshot used to satisfy `AutocompleteHost`. Wraps an
/// `EditorSnapshot` (Cow-borrowed from the textarea on the common
/// path — perf #8) plus the cursor's last-rendered screen
/// position. The host's `cache_key` mirrors the editor's
/// `content_revision`; `None` is reserved for hosts whose buffer
/// has no stable identity (the search-box modal).
struct EditorHostSnapshot {
    snap: EditorSnapshot,
    cursor_screen: Option<(u16, u16)>,
    cache_key: Option<NonZeroU64>,
}

impl AutocompleteHost for EditorHostSnapshot {
    fn buffer_snapshot(&self) -> EditorSnapshot {
        // Re-package the inner snap as a fresh view tied
        // to `&self`. `Cow::as_ref` works for both Borrowed and
        // Owned variants — the latter only occurs on the Nvim path
        // where the inner snapshot already paid the clone cost.
        EditorSnapshot::of_buffer(
            self.snap.text.clone(),
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
/// callers pass `&self.backend`, `self.revs.current()`,
/// `self.view.last_cursor_screen` directly so the borrow checker
/// can split borrows from `&mut self.autocomplete`. Returns `None`
/// on the Nvim backend (autocomplete is Textarea-only).
fn build_editor_host_snapshot(
    backend: &BackendState,
    content_revision: NonZeroU64,
    cursor_screen: Option<(u16, u16)>,
) -> Option<EditorHostSnapshot> {
    if !backend.is_textarea() {
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
    view: MarkdownEditorView,
    /// The one revision clock plus its comparison snapshots (saved,
    /// needles) — see [`Revisions`]. `revs.current()` advances iff the
    /// buffer text changes: `bump_content` on the textarea backend, the
    /// per-frame `adopt` of the snapshot's revision on the nvim backend
    /// (the snapshot derives it from the backend's `content_gen` under a
    /// single lock — the only `content_gen → NonZeroU64` site). Cursor
    /// moves never touch it, so an in-flight autosave's revision token
    /// survives navigation, and `view.update` reuses its parse cache.
    revs: Revisions,
    /// Current selection range in logical (row, byte-col) coordinates.
    /// Only tracked for the Textarea backend; always `None` for Nvim.
    selection: Option<((usize, usize), (usize, usize))>,
    /// Host-side state and policy for the Nvim backend (pending-Z intercept,
    /// frame sync). See [`nvim_host`].
    nvim_host: NvimHost,
    /// Active Ctrl+F find bar; `None` when not searching.
    search: Option<find_bar::FindBar>,
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
    /// Background full-wrap fallback for large buffers, the layout-side
    /// twin of `full_parse_task`. The view installs a `Layout::unwrapped`
    /// stub and signals pending; this slot owns the spawned tokio task
    /// that runs the real `Layout::compute`. `SingleSlotTask` aborts the
    /// previous spawn on a fresh edit, so a burst of edits resolves
    /// against the latest content.
    layout_task: SingleSlotTask<()>,
    /// Whether the last key was handled while vim was in Insert. A change either
    /// way ends the open **undo group**: leaving Insert closes vim's session,
    /// entering it starts a fresh one.
    last_insert_session: bool,
    /// Set by a right-click with no selection: the host (which owns the note
    /// path) opens the note's context menu and clears the flag.
    pub wants_context_menu: bool,
    /// Lowercased needles to emphasize in the rendered buffer — set when the
    /// note was opened from a query result (spec §5.1 "search match"), and
    /// dropped on the first edit (`revs.needles_stale()`).
    search_needles: Vec<String>,
    full_parse_tx: tokio::sync::mpsc::UnboundedSender<(u64, ParsedBuffer)>,
    full_parse_rx: tokio::sync::mpsc::UnboundedReceiver<(u64, ParsedBuffer)>,
    layout_tx: tokio::sync::mpsc::UnboundedSender<(u64, crate::ropetext::Layout)>,
    layout_rx: tokio::sync::mpsc::UnboundedReceiver<(u64, crate::ropetext::Layout)>,
    /// `AppTx` clone bound the first time `handle_input` runs, so the
    /// spawned full-parse/full-wrap tasks can post `AppEvent::Redraw` on
    /// completion without waiting for the next user keystroke.
    redraw_tx: Option<AppTx>,
}

impl TextEditorComponent {
    pub fn new(key_bindings: KeyBindings, settings: &AppSettings) -> Self {
        let (full_parse_tx, full_parse_rx) = tokio::sync::mpsc::unbounded_channel();
        let (layout_tx, layout_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            backend: BackendState::from_settings(
                &settings.editor_backend,
                settings.nvim_path.as_ref(),
            ),
            rect: Rect::default(),
            key_bindings,
            view: MarkdownEditorView::new(),
            revs: Revisions::new(),
            selection: None,
            nvim_host: NvimHost::new(),
            search: None,
            autocomplete: None,
            autocomplete_vault: None,
            autocomplete_redraw_bound: false,
            full_parse_task: SingleSlotTask::empty(),
            layout_task: SingleSlotTask::empty(),
            last_insert_session: false,
            wants_context_menu: false,
            search_needles: Vec::new(),
            full_parse_tx,
            full_parse_rx,
            layout_tx,
            layout_rx,
            redraw_tx: None,
        }
    }

    /// Attach a vault so autocomplete can query notes/tags. Activates
    /// the controller immediately on the textarea backend; on Nvim, the
    /// vault is stashed and the controller is spun up later if
    /// `maybe_recover_from_dead_nvim` falls back to Textarea.
    pub fn set_vault(&mut self, vault: Arc<NoteVault>) {
        self.autocomplete_vault = Some(vault.clone());
        if self.backend.is_textarea() {
            self.autocomplete = Some(AutocompleteController::new(
                std::sync::Arc::new(crate::components::search_list::VaultSuggestions { vault }),
                AutocompleteMode::Both,
            ));
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
        if !self.backend.is_textarea() {
            return;
        }
        let Some(vault) = self.autocomplete_vault.clone() else {
            return;
        };
        self.autocomplete = Some(AutocompleteController::new(
            std::sync::Arc::new(crate::components::search_list::VaultSuggestions { vault }),
            AutocompleteMode::Both,
        ));
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
    fn autocomplete_host_snapshot(&self) -> Option<EditorHostSnapshot> {
        build_editor_host_snapshot(
            &self.backend,
            self.revs.current(),
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
        let ta = self.backend.as_textarea()?;
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
            self.revs.current(),
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
            let Some(ta) = self.backend.as_textarea() else {
                return;
            };
            let (row, col) = cursor_tuple(ta);
            let line = ta.row(row).unwrap_or_default();
            if !has_trigger_before_cursor(&line, col) {
                return;
            }
        }

        // Slow path: build the borrowed snapshot for the controller to
        // reconcile. Free function so `&self.backend` and
        // `&mut self.autocomplete` can coexist.
        let Some(snapshot) = build_editor_host_snapshot(
            &self.backend,
            self.revs.current(),
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
    /// The open note's text. Empty for the nvim backend, which owns its own.
    pub fn text(&self) -> crate::ropetext::Text {
        match &self.backend {
            BackendState::Textarea(tb) => tb.ta.text().clone(),
            BackendState::Nvim(_) => crate::ropetext::Text::new(),
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
    /// `snapshot_from_backend(&self.backend, self.revs.current())`
    /// so the borrow checker can split the borrows across distinct
    /// fields.
    pub fn view_snapshot(&self) -> EditorSnapshot {
        snapshot_from_backend(&self.backend, self.revs.current())
    }

    /// The cursor's (row, col) without materialising a snapshot — the Nvim
    /// path of `view_snapshot` clones every buffer line, far too heavy for
    /// per-frame consumers that only want the position (status-bar ln/col).
    pub fn cursor_pos(&self) -> (usize, usize) {
        self.backend.cursor()
    }

    /// Set the search needles to emphasize in the rendered buffer (the note
    /// was opened from a query result). Cleared automatically on the first
    /// edit.
    pub fn set_search_needles(&mut self, needles: Vec<String>) {
        self.search_needles = needles
            .into_iter()
            .map(|n| n.to_lowercase())
            .filter(|n| !n.is_empty())
            .collect();
        self.revs.arm_needles();
    }

    pub fn set_text(&mut self, text: String) {
        // No-op when the buffer would be identical — preserves view scroll,
        // selection, edit generation cache, and an open autocomplete popup.
        // Saves the expensive lines clone too. Still normalises the saved
        // marker: if the buffer was flagged dirty by a previous divergent
        // save, reloading the same content from disk should clear that
        // flag rather than persist a phantom `[+]` in the title bar.
        if text == self.get_text() {
            self.revs.mark_saved_current();
            if let Some(nvim) = self.backend.as_nvim() {
                nvim.mark_clean();
            }
            return;
        }
        match &mut self.backend {
            BackendState::Textarea(tb) => {
                tb.ta.replace(crate::ropetext::Text::from(text.as_str()));
            }
            BackendState::Nvim(nvim) => {
                nvim.set_text(&text);
            }
        }
        self.backend.reset_input_state();
        self.bump_content();
        let reconstructed = self.get_text();
        self.mark_saved(reconstructed);
        // Buffer replaced — close any open autocomplete popup so it does
        // not linger over the new note (e.g. after Ctrl+G follow-link).
        self.close_autocomplete();
        // Everything below described the OLD buffer. A textarea swap installs
        // a fresh, empty history, so recorded **undo groups** now point at
        // states this history cannot reach — and a group whose `after` is an
        // empty buffer would hash-match any empty note, popping an extra entry
        // against unrelated history. The find bar is worse: `armed_empty`
        // surviving a note swap means one Ctrl+A deletes every match in a note
        // the user never armed, skipping the confirmation the flag exists to
        // force. `self.selection` would likewise still describe the old text.
        self.search = None;
        self.selection = None;
        // The whole buffer was replaced. The cursor resets to the top, so if
        // the line count happens to match and row 0 differs, the damage fast
        // path would report `0..1` and leave the rest of the note parsed as the
        // previous one.
        self.view.note_bulk_edit();
    }

    pub fn get_text(&self) -> String {
        self.backend.text()
    }

    /// Current content revision. Bumped on every text-mutating handler;
    /// stable across cursor moves and idle frames. Used by the autosave
    /// path to record "this snapshot was saved" without rebuilding the
    /// buffer text on completion. `NonZeroU64` makes 0 unrepresentable
    /// so callers can express "no revision" as `Option<NonZeroU64>::None`
    /// without a magic-value sentinel.
    pub fn content_revision(&self) -> NonZeroU64 {
        self.revs.current()
    }

    /// Mark the buffer as clean iff its current revision still matches
    /// `rev` (i.e. no edits landed between the save being issued and
    /// completing). Diverged revision → no-op: leave the saved snapshot
    /// alone, because some OTHER mechanism (a synchronous `try_save`
    /// racing this completion) may have already marked a NEWER revision
    /// clean, and a stale completion must not clobber that. `is_dirty`
    /// already reads true when the saved snapshot mismatches the current
    /// revision, so doing nothing on a mismatch keeps the editor correctly
    /// dirty without overwriting a legitimately-newer saved snapshot.
    pub fn mark_saved_at_revision(&mut self, rev: NonZeroU64) {
        if !self.revs.mark_saved_at(rev) {
            return;
        }
        // Below the guard on purpose: a stale completion marks nothing, and an
        // action that did nothing must not close the group. One undo after a
        // real save lands on exactly what is on disk (CONTEXT.md).
        self.interrupt_typing();
        if let Some(nvim) = self.backend.as_nvim() {
            nvim.mark_clean();
        }
    }

    /// Synchronous mark-saved used by `try_save` and `set_text`. Unlike
    /// `mark_saved_at_revision` (which no-ops on a stale revision because
    /// it can race a sync mark_saved), this one CLOBBERS the saved snapshot
    /// to `None` when the supplied text diverges: the sync caller holds
    /// `&mut self` for the whole save, so there is no concurrent newer
    /// clean state to preserve, and the user typing between
    /// `get_text()` and this call must show as dirty.
    pub fn mark_saved(&mut self, text: String) {
        self.interrupt_typing();
        let matches = text == self.get_text();
        if matches {
            if let Some(nvim) = self.backend.as_nvim() {
                nvim.mark_clean();
            }
            self.revs.mark_saved_current();
        } else {
            // Textarea: divergent save → stay dirty.
            // Nvim: snapshot's `dirty` was untouched anyway; the saved
            // snapshot in `revs` is what is_dirty consults on the
            // Textarea backend, and we explicitly forget it here.
            self.revs.mark_diverged();
        }
    }

    /// Something happened that is not a continuation of typing.
    ///
    /// One entry point for every path that is not a keystroke — a click, a
    /// find, an autocomplete accept, a save, a vim motion — because the state it
    /// closes is kept on the component while those paths reach the buffer by
    /// four different routes, and a rule applied on one of them is a rule the
    /// other three forget.
    ///
    /// The two halves are deliberately not gated alike:
    ///
    /// - The **goal cell** belongs to a run of `↑`/`↓` and to nothing else, so
    ///   any other action forgets it, in every mode.
    /// - The **undo group** is closed only outside a vim Insert session. There
    ///   the session *is* the group (CONTEXT.md), so an autosave landing
    ///   mid-word, or an auto-surround typed inside Insert, must not split what
    ///   one `u` is supposed to take back.
    fn interrupt_typing(&mut self) {
        self.view.clear_visual_goal();
        if self.backend.modal_is_insert().unwrap_or(false) {
            return;
        }
        if let Some((_, run)) = self.backend.as_textarea_parts_mut() {
            run.end();
        }
    }

    /// Notice that vim entered or left Insert, and close the group if it did.
    ///
    /// This marks a session's *start*: the first key of a session arrives here
    /// with the flag still reading the old mode. The session's *end* is closed by
    /// [`Self::interrupt_typing`] on the engine path, because `Esc` is consumed
    /// there and never reaches this function at all.
    fn sync_insert_session(&mut self) {
        let in_insert = self.backend.modal_is_insert().unwrap_or(false);
        if self.last_insert_session == in_insert {
            return;
        }
        self.last_insert_session = in_insert;
        if let Some((_, run)) = self.backend.as_textarea_parts_mut() {
            run.end();
        }
    }

    pub fn is_dirty(&self) -> bool {
        match &self.backend {
            BackendState::Textarea(_) => self.revs.is_dirty(),
            BackendState::Nvim(nvim) => nvim.snapshot().dirty,
        }
    }

    /// Whether a bare Space should start the leader (vim Normal mode only).
    /// Returns `false` for the direct textarea backend, the nvim backend,
    /// vim Insert/Visual modes, and any pending state.
    ///
    /// A pure vim-mode fact. It used to also return false while the find bar
    /// was open — the bar's claim smuggled through the nearest differently
    /// named field, because the snapshot had nowhere to put it. That is now
    /// [`Self::claim`]'s job.
    pub fn space_leads(&self) -> bool {
        self.backend.space_leads()
    }

    /// Whether a press in this editor moves the cursor to the cell under it.
    ///
    /// False on the **nvim** backend, where the terminal and nvim own the
    /// mouse: this component's `handle_mouse` returns `NotConsumed` before any
    /// `jump_to`. Anything that treats a click as pointing *at* something —
    /// following a link, most obviously — has to ask this first, or it reads a
    /// cursor the click never moved.
    pub fn mouse_drives_cursor(&self) -> bool {
        self.backend.is_textarea()
    }

    /// Whether this cell is one this editor would place the cursor on.
    ///
    /// Narrower than the editor *column*, which the panel set hit-tests: the
    /// column includes the frame drawn around this component, and `self.rect`
    /// is the interior it was handed at render — minus the find-bar row, which
    /// `render` already excludes. `handle_mouse` bounds-checks against exactly
    /// this and returns `NotConsumed` outside it, so a press anywhere else
    /// leaves the cursor where it was. Callers that read the cursor *after* a
    /// press have to ask, or they read a position the press never set.
    pub fn covers(&self, column: u16, row: u16) -> bool {
        self.rect
            .contains(ratatui::layout::Position::new(column, row))
    }

    /// Which editor-internal surface currently holds input.
    ///
    /// The find bar outranks the popup because opening the bar closes it
    /// (`open_or_advance_search`), so the two cannot genuinely coexist.
    pub fn claim(&self) -> EditorClaim {
        if self.search.is_some() {
            EditorClaim::FindBar
        } else if self.autocomplete.as_ref().is_some_and(|c| c.is_open()) {
            EditorClaim::Autocomplete
        } else {
            EditorClaim::None
        }
    }

    /// Returns the link or label target under the cursor, or `None` if the
    /// cursor is not inside a wikilink, markdown link, or hashtag span.
    pub fn follow_target_at_cursor(&self) -> Option<FollowTarget> {
        let (_row, col, line) = match &self.backend {
            BackendState::Textarea(tb) => {
                let (row, col) = cursor_tuple(&tb.ta);
                let line = tb.ta.row(row)?.into_owned();
                (row, col, line)
            }
            BackendState::Nvim(nvim) => {
                let snap = nvim.snapshot();
                let (row, col) = snap.cursor;
                let line = snap.lines.get(row)?.to_string();
                (row, col, line)
            }
        };

        // F5: Check wiki-link / markdown-link spans first; Link wins over Label
        // even if a future edit accidentally lets a Label slip through a Link range.
        if let Some(span) = kimun_core::note::scan::link_char_spans(&line)
            .into_iter()
            .find(|s| s.start <= col && col < s.end)
        {
            return Some(FollowTarget::Link(span.target));
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
                FollowTarget::Label(name)
            })
    }

    /// Copy selected text to the OS clipboard, flashing the outcome.
    ///
    /// Routed through the shared [`crate::components::yank`] seam so a clipboard
    /// failure is reported rather than swallowed, and so "nothing was selected"
    /// is distinguishable from "the copy failed".
    fn copy_selection_to_clipboard(&mut self, tx: &AppTx) {
        let text = {
            // Match the highlighted range in vim charwise Visual mode: the
            // textarea selection is half-open, but the cursor's char is part of
            // the visual selection, so copy it too (right-click copy reaches
            // here after a mouse drag that flipped the engine into Visual).
            // Read-only — must NOT move the cursor or grow the live selection,
            // since copy leaves the selection active (repeated copy would drift
            // wider). `extend_visual_selection_inclusive` is for one-shot
            // consumers (paste/wrap) that collapse the selection afterwards.
            let selected = self
                .inclusive_visual_range()
                .zip(self.backend.as_textarea())
                .and_then(|(range, ta)| selection_text_in(ta, range));
            match selected {
                Some(t) if !t.is_empty() => t,
                _ => {
                    tx.send(AppEvent::FlashMessage("nothing to copy".into()))
                        .ok();
                    return;
                }
            }
        };
        crate::components::yank(text, "copied", tx);
    }

    /// The live selection range, with the end extended by one char when in vim
    /// charwise Visual mode (vim treats the selection as inclusive of the char
    /// under the cursor; ratatui's range is half-open). Read-only: computes the
    /// range without touching the cursor or live selection. `None` when there
    /// is no selection or no textarea backend.
    fn inclusive_visual_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let charwise = self.backend.selection_includes_cursor();
        let ta = self.backend.as_textarea()?;
        let (start, (er, ec)) = ta.selection_range()?;
        let end = if charwise {
            let len = ta.row(er).map(|l| l.chars().count()).unwrap_or(ec);
            (er, (ec + 1).min(len))
        } else {
            (er, ec)
        };
        Some((start, end))
    }

    /// Paste text from the OS clipboard at the cursor, replacing any active
    /// selection. Every failure is reported — silence here is what made the
    /// vim-mode paste bug so hard to place.
    fn paste_from_clipboard(&mut self, tx: &AppTx) {
        let text = match crate::components::with_clipboard(|c| c.get_text()) {
            Ok(t) if !t.is_empty() => t,
            Ok(_) => {
                tx.send(AppEvent::FlashMessage("clipboard is empty".into()))
                    .ok();
                return;
            }
            Err(e) => {
                tx.send(AppEvent::FlashMessage(format!("clipboard: {e}")))
                    .ok();
                return;
            }
        };
        self.paste_text(&text, tx);
        // Report the paste like every other clipboard action. Without this the
        // footer keeps the raw chord echo, so Ctrl+V was the one clipboard key
        // that never said what it did.
        tx.send(AppEvent::FlashMessage("pasted".into())).ok();
    }

    /// Inserts `text` at the cursor, replacing any active selection. When `text`
    /// is a URL (http/https/ftp/ftps/mailto) and a selection is active, the
    /// selection is wrapped as a markdown link `[selection](url)` instead of
    /// being replaced by the raw URL.
    ///
    /// On the Nvim backend the URL-wrap shortcut is skipped (would require
    /// reading the visual selection from nvim) — `text` is forwarded via
    /// `nvim_paste`, which honours the current mode (insert/normal/visual).
    /// In vim charwise Visual mode the live textarea selection is half-open and
    /// excludes the char under the cursor, but vim treats the selection as
    /// inclusive. Extend the selection end by one so out-of-engine consumers
    /// (paste-over-selection, bold/italic/strikethrough wrap) act on the WHOLE
    /// visual range — mirrors the highlight path (see `selection_includes_cursor`)
    /// and the vim engine's own `select_range(.., inclusive=true)`. No-op
    /// outside charwise Visual (Direct/Insert/VisualLine/Nvim), where the
    /// half-open range is already what callers want.
    fn extend_visual_selection_inclusive(&mut self) {
        if !self.backend.selection_includes_cursor() {
            return;
        }
        if let Some((start, end)) = self.inclusive_visual_range()
            && let Some(ta) = self.backend.as_textarea_mut()
        {
            set_selection(ta, start, end);
        }
    }

    pub fn paste_text(&mut self, text: &str, tx: &AppTx) {
        if text.is_empty() {
            return;
        }
        // While the **find bar** is open it owns input — but that was only
        // implemented for key events, so a bracketed paste used to land in the
        // buffer behind the bar, leaving the match count and the highlighted
        // current match describing text that no longer exists. Route it into
        // the focused field instead, which is what the user meant: pasting a
        // term to search for or to replace with.
        if self.search.is_some() {
            if let (Some(bar), BackendState::Textarea(tb)) =
                (self.search.as_mut(), &mut self.backend)
            {
                bar.paste(text, &mut tb.ta);
            }
            self.apply_edit_outcome();
            return;
        }
        self.extend_visual_selection_inclusive();
        match &mut self.backend {
            BackendState::Textarea(tb) => {
                let selection = linkable_url(text).and_then(|_| selection_text(&tb.ta));
                let wrapped = try_build_markdown_link(text, selection.as_deref());
                let insert = wrapped.as_deref().unwrap_or(text).to_string();
                // Replacing a selection is a cut plus an insert — one paste,
                // one undo.
                tb.ta.edit(|ta| {
                    if ta.selection_range().is_some() {
                        ta.cut();
                    }
                    ta.insert_str(insert);
                });
                self.selection = tb.ta.selection_range();
                self.apply_edit_outcome();
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
    /// through `nvim_paste` on the Nvim backend (delegates to [`Self::paste_text`]
    /// for that case — URL-wrap is a no-op when nothing in the supplied text
    /// matches `linkable_url`, so the two paths are equivalent on Nvim).
    pub fn insert_at_cursor(&mut self, text: &str, tx: &AppTx) {
        if matches!(self.backend, BackendState::Nvim(_)) {
            self.paste_text(text, tx);
            return;
        }
        // Replacing the selection happens HERE, atomically with the insert, and
        // not earlier when the paste was merely started: the image encode and
        // the attachment save can both fail (disk full, read-only vault, a sync
        // conflict), and a cut done up front would leave the user's selected
        // text destroyed with nothing in its place and only a save error to
        // explain it.
        self.take_selection_for_external_paste();
        if let Some(ta) = self.backend.as_textarea_mut() {
            ta.insert_str(text);
            self.selection = ta.selection_range();
            self.apply_edit_outcome();
        }
        // See `paste_text` — out-of-band buffer mutation must
        // re-reconcile the popup state.
        self.bind_autocomplete_redraw(tx);
        self.sync_autocomplete();
    }

    /// Snapshot of the system clipboard image, if any. Returns owned RGBA bytes
    /// plus the image dimensions. The screen layer is responsible for encoding
    /// (e.g. PNG) and persisting via the vault.
    ///
    /// Reads go through the same shared handle as writes — not for
    /// ownership (only writes need that) but so there is one connection and one
    /// reconnect policy. No flash here: this is a *probe* run ahead of every
    /// Ctrl+V, and "no image on the clipboard" is the ordinary case, not a
    /// failure to report.
    pub fn take_clipboard_image(&mut self) -> Option<ClipboardImage> {
        let img = crate::components::with_clipboard(|c| c.get_image()).ok()?;
        Some(ClipboardImage {
            width: img.width,
            height: img.height,
            rgba: img.bytes.into_owned(),
        })
    }

    /// Prepare the buffer for content arriving from *outside* the editor's own
    /// key path — today only the clipboard-image paste, which the screen layer
    /// owns because it alone can reach the vault.
    ///
    /// Removes the active selection (the incoming content replaces it, as with
    /// every other paste) and reconciles the vim engine out of Visual, through
    /// the same door the mouse path uses. Without this the engine keeps a mode
    /// that the buffer no longer supports: still Visual, selection gone.
    pub fn take_selection_for_external_paste(&mut self) {
        self.extend_visual_selection_inclusive();
        let cut = if let Some(ta) = self.backend.as_textarea_mut() {
            let cut = ta.selection_range().is_some() && ta.cut();
            self.selection = ta.selection_range();
            cut
        } else {
            false
        };
        if cut {
            self.apply_edit_outcome();
        }
        // `false` = no live selection, so a modal engine returns to Normal.
        // A no-op for Insert and for the non-modal backends.
        self.backend.sync_mouse_selection(false);
    }

    /// Wraps the active selection in `open`/`close` and re-selects the inner
    /// text so wraps chain (see CONTEXT.md "Auto-surround"). Returns `false`
    /// without touching the buffer when there is no (non-empty) selection or
    /// on the Nvim backend. Callers on the key path don't reconcile the
    /// autocomplete popup — `handle_input` re-syncs on any content bump.
    fn wrap_selection(&mut self, open: &str, close: &str) -> bool {
        // Vim charwise Visual selections are inclusive; extend the half-open
        // range so the char under the cursor is wrapped too (otherwise `ve`
        // then Bold yields `**hell**o`). No-op outside charwise Visual.
        self.extend_visual_selection_inclusive();
        let Some(ta) = self.backend.as_textarea_mut() else {
            return false;
        };
        let Some(((sr, sc), (er, ec))) = ta.selection_range() else {
            return false;
        };
        let Some(text) = selection_text(ta) else {
            return false;
        };
        ta.insert_str(format!("{open}{text}{close}"));
        // Reselect the inner text. The open marker shifts cols on the first
        // selected line only; coordinates are char-based, matching
        // `selection_range`.
        let shift = open.chars().count();
        let inner_end_col = if sr == er { ec + shift } else { ec };
        set_selection(ta, (sr, sc + shift), (er, inner_end_col));
        self.selection = ta.selection_range();
        // Only here, past every `return false` above: this function is consulted
        // for each bare `( [ { < " ' ` * _ ~` keystroke, and the declining ones
        // fall through to ordinary typing, which must keep its run.
        self.interrupt_typing();
        self.apply_edit_outcome();
        true
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
        if self.wrap_selection(marker, marker) {
            return;
        }
        self.interrupt_typing();
        let Some(ta) = self.backend.as_textarea_mut() else {
            return;
        };
        ta.insert_str(format!("{marker}{marker}"));
        for _ in 0..marker.len() {
            ta.move_cursor(CursorMove::Back);
        }
        self.selection = ta.selection_range();
        self.apply_edit_outcome();
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
            let Some(ta) = self.backend.as_textarea() else {
                return false;
            };
            // A mouse click leaves a zero-width selection active (handle_mouse
            // calls start_selection on Down), so only bail on a non-empty one.
            if ta
                .selection_range()
                .is_some_and(|(start, end)| start != end)
            {
                return false;
            }
            let (row, col) = cursor_tuple(ta);
            let Some(line) = ta.row(row) else {
                return false;
            };
            let total_chars = line.chars().count();
            if col != total_chars {
                return false;
            }
            // ASCII whitespace, so byte index == char index here.
            let ws_end = markdown::leading_ws_byte_len(&line);
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
                let Some(ta) = self.backend.as_textarea_mut() else {
                    unreachable!()
                };
                ta.move_cursor(CursorMove::Head);
                ta.delete_str(chars);
            }
            Action::InsertPrefix(prefix) => {
                let Some(ta) = self.backend.as_textarea_mut() else {
                    unreachable!()
                };
                // Newline plus prefix is two history entries; one `edit()`
                // scope makes continuing a list one undo.
                ta.edit(|ta| {
                    ta.insert_newline();
                    ta.insert_str(prefix);
                });
            }
        }
        let Some(ta) = self.backend.as_textarea() else {
            unreachable!()
        };
        self.selection = ta.selection_range();
        self.apply_edit_outcome();
        true
    }

    /// Move the cursor to the first markdown heading line whose text equals
    /// `heading` (any level), e.g. for the OUTLINE drawer's jump. No-op when
    /// the heading is not found, and on the Nvim backend (same policy as
    /// [`Self::indent_lines`]).
    pub fn jump_to_heading(&mut self, heading: &str) {
        let Some(ta) = self.backend.as_textarea_mut() else {
            return;
        };
        // The OUTLINE entries carry the extractor-rendered heading text
        // (inline markup resolved, closing ATX `#` dropped), so normalise
        // both sides before comparing: strip the ATX markers and the
        // common inline-emphasis characters.
        fn normalise(text: &str) -> String {
            text.trim()
                .trim_end_matches('#')
                .trim()
                .replace(['*', '_', '`'], "")
        }
        let wanted = normalise(heading);
        let row = (0..ta.row_count()).find(|&row| {
            let Some(line) = ta.row(row) else {
                return false;
            };
            let t = line.trim_start();
            let stripped = t.trim_start_matches('#');
            stripped.len() != t.len() && normalise(stripped) == wanted
        });
        if let Some(row) = row {
            ta.jump_to(row, 0);
        }
    }

    /// Indent or dedent whole lines. One step is `\t` if `hard_tab_indent` is
    /// on, else `indent_width` spaces. Dedent counts a leading tab as one step.
    /// No-op on Nvim backend.
    pub fn indent_lines(&mut self, dedent: bool) {
        let Some(ta) = self.backend.as_textarea_mut() else {
            return;
        };
        let tab_len = ta.indent_width() as usize;
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

        // Drop the live selection before mutating: with the anchor still set,
        // `move_cursor(Jump(row, 0))` re-anchors the selection from the start
        // column back to col 0, so `insert_str`/`delete_str` would replace the
        // text before the selection. The selection is restored at the end.
        ta.cancel_selection();

        // Indenting N lines is 2N history entries; one `edit()` scope makes
        // the whole block one undo instead of N.
        ta.edit(|ta| {
            for row in start_row..=end_row {
                if dedent {
                    let count = {
                        let line = ta.row(row).unwrap_or_default();
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
                        ta.jump_to(row, 0);
                        ta.delete_str(count);
                        any_change = true;
                    }
                    row_deltas.push(-(count as isize));
                } else {
                    ta.jump_to(row, 0);
                    ta.insert_str(&indent);
                    row_deltas.push(indent_chars as isize);
                    any_change = true;
                }
            }
        });

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
                set_selection(ta, (ssr, adj(ssr, ssc)), (ser, adj(ser, sec)));
            }
            None => {
                let (cr, cc) = saved_cursor.expect("captured when sel is None");
                let new_col = adj(cr, cc);
                ta.jump_to(cr, new_col);
            }
        }

        if any_change {
            self.selection = ta.selection_range();
            self.apply_edit_outcome();
        }
    }
}

impl TextEditorComponent {
    /// Advances the revision clock. Use at every site that mutates the
    /// buffer (insert, delete, paste, undo/redo, autocomplete accept) on
    /// the Textarea backend. `handle_input` uses the revision delta to
    /// detect a real text change without materialising the buffer.
    ///
    /// Not called by the Nvim path — the reverse-refresh task in
    /// `backend.rs` bumps `snap.content_gen` on real diffs, the frame
    /// snapshot derives its revision from that, and `render` adopts the
    /// snapshot's value (see the `revs` field doc).
    #[inline]
    fn bump_content(&mut self) {
        self.revs.bump();
    }

    /// If the Nvim process has died, fall back to a Textarea with the last known content.
    fn maybe_recover_from_dead_nvim(&mut self) {
        if self.backend.recover_from_dead_nvim() {
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
        // FocusSidebar / FocusEditor shortcuts are intercepted at the
        // EditorScreen level for directional navigation. The pending-Z
        // intercept and quit-command policy live in `nvim_host`.
        let nvim = self.backend.as_nvim()?;
        // No revision bump here: navigation keys don't change the buffer,
        // and content changes surface through the reverse-refresh task's
        // `content_gen`, adopted from the frame snapshot in `render` — so
        // an in-flight save's revision token survives navigation.
        self.nvim_host.handle_key(nvim, key, tx);
        Some(EventState::Consumed)
    }

    /// Open the find bar; if already open, advance to the next match. No-op on
    /// the Nvim backend, which has its own `/` search. Policy lives here
    /// because only the editor knows which backend is active.
    pub fn open_or_advance_search(&mut self) {
        if !self.backend.is_textarea() {
            return;
        }
        if self.search.is_some() {
            self.dispatch_bar(|bar, buf| {
                bar.advance(buf, false);
                find_bar::KeyOutcome::default()
            });
            return;
        }
        // Yield key focus to the bar — close the autocomplete popup so it stops
        // intercepting Esc / Up / Down / Tab / Enter, which belong to the bar.
        self.close_autocomplete();
        self.search = Some(find_bar::FindBar::new());
    }

    /// Open the find bar with the **replace field** already revealed, or reveal
    /// it on an already-open bar.
    pub fn open_replace(&mut self) {
        if !self.backend.is_textarea() {
            return;
        }
        if self.search.is_none() {
            self.close_autocomplete();
            self.search = Some(find_bar::FindBar::new());
        }
        if let Some(bar) = self.search.as_mut() {
            bar.reveal_replace();
        }
    }

    /// Repeat the last search (vim `n`/`N`) using the buffer's persisted
    /// pattern, even when the bar is closed.
    fn search_repeat(&mut self, backward: bool) {
        let BackendState::Textarea(tb) = &mut self.backend else {
            return;
        };
        // Paint the match `n`/`N` landed on. The bar is closed here, so the
        // buffer answers — which is why `match_at_cursor` lives on it.
        self.selection = if tb.ta.search_repeat(backward) {
            tb.ta.match_at_cursor()
        } else {
            None
        };
    }

    /// Run `f` against the open bar and its buffer, then apply what the buffer
    /// measured. Returns `false` when no bar is open.
    fn dispatch_bar(
        &mut self,
        f: impl FnOnce(&mut find_bar::FindBar, &mut RopeBuffer) -> find_bar::KeyOutcome,
    ) -> bool {
        let BackendState::Textarea(tb) = &mut self.backend else {
            return false;
        };
        let Some(bar) = self.search.as_mut() else {
            return false;
        };
        let outcome = f(bar, &mut tb.ta);
        if outcome.close {
            self.search = None;
            // Clear the anchor as well as the mirrored range. The bar's cursor
            // jumps (`search_forward`) move the cursor without touching
            // `selection_start`, so a selection that existed before the search
            // is left live but unpainted — and the next keystroke silently
            // deletes it.
            tb.ta.cancel_selection();
            self.selection = None;
        }
        self.apply_edit_outcome();
        true
    }

    /// Feed a key to the open bar. The bar consumes every key it sees.
    fn dispatch_to_find_bar(&mut self, key: &ratatui::crossterm::event::KeyEvent) -> bool {
        self.dispatch_bar(|bar, buf| bar.handle_key(key, buf))
    }

    /// The **replace preview** for this frame, when a bar is open. Test-facing:
    /// production reads it through `FindBar::overlay`.
    #[cfg(test)]
    fn replace_preview(&self) -> Option<find_replace::Preview> {
        let bar = self.search.as_ref()?;
        let buf = self.backend.as_textarea()?;
        bar.preview(buf)
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

    /// Drain the **edit buffer**'s measured outcome and apply it.
    ///
    /// The one place a text change turns into a revision bump and a
    /// parse-damage signal. Both facts are derived by the buffer from the
    /// content either side of the edit, so neither can be predicted wrongly
    /// (an `insert_str` that returns `false` after deleting) or simply
    /// forgotten at one of 22 sites.
    ///
    /// The revision clock stays on the component because it serves the nvim
    /// backend too, which has no edit buffer.
    fn apply_edit_outcome(&mut self) -> bool {
        let Some(outcome) = self.backend.as_textarea_mut().map(|ta| ta.take_outcome()) else {
            return false;
        };
        if outcome.changed {
            self.bump_content();
        }
        if outcome.bulk {
            self.view.note_bulk_edit();
        }
        if let Some(rows) = outcome.damage {
            self.view.note_damage(rows, outcome.line_delta);
        }
        outcome.changed
    }

    /// Undo one *user action*. The **edit buffer** replays history to the
    /// state the action started from, so nothing here counts entries.
    fn undo_grouped(&mut self) -> bool {
        let moved = self.backend.as_textarea_mut().is_some_and(|ta| ta.undo());
        if moved {
            self.selection = self
                .backend
                .as_textarea()
                .and_then(|ta| ta.selection_range());
        }
        moved
    }

    /// Redo one *user action*. Mirror of [`Self::undo_grouped`].
    fn redo_grouped(&mut self) -> bool {
        let moved = self.backend.as_textarea_mut().is_some_and(|ta| ta.redo());
        if moved {
            self.selection = self
                .backend
                .as_textarea()
                .and_then(|ta| ta.selection_range());
        }
        moved
    }

    /// Handle a key event when using the Textarea backend.
    fn handle_textarea_key(
        &mut self,
        key: &ratatui::crossterm::event::KeyEvent,
        tx: &AppTx,
    ) -> EventState {
        // No find-bar check here: `handle_input` — this function's one
        // production caller — already routes to the bar before the vim engine,
        // so a second check could never fire. It survived only because tests
        // call this function directly.

        // Whether this key types. Decided here rather than in the plain-key
        // section below, because a key claimed earlier — Ctrl+Z, a clipboard
        // chord, Tab — returns before ever reaching it, and a run left open
        // across an undo would try to extend a group that was just taken back.
        let stroke = plain_keys::operation(*key).and_then(|op| match op {
            plain_keys::Operation::Insert(c) => Some(typing_run::Stroke::Insert(c)),
            plain_keys::Operation::InsertNewline => Some(typing_run::Stroke::Insert('\n')),
            plain_keys::Operation::DeleteBack | plain_keys::Operation::DeleteForward => {
                Some(typing_run::Stroke::Delete)
            }
            _ => None,
        });
        if stroke.is_none()
            && let Some((_, run)) = self.backend.as_textarea_parts_mut()
        {
            run.end();
        }

        // System clipboard shortcuts — intercept before passing to textarea.
        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Char('c') => {
                    self.copy_selection_to_clipboard(tx);
                    return EventState::Consumed;
                }
                KeyCode::Char('v') => {
                    self.paste_from_clipboard(tx);
                    return EventState::Consumed;
                }
                KeyCode::Char('x') => {
                    self.copy_selection_to_clipboard(tx);
                    let cut = if let Some(ta) = self.backend.as_textarea_mut() {
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
                        self.apply_edit_outcome();
                    }
                    return EventState::Consumed;
                }
                _ => {}
            }
        }

        // Undo / Redo (Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z). Handled before the
        // textarea borrow below because the **undo group** bookkeeping lives on
        // the component, and `as_textarea_mut` borrows all of `self`. A replace
        // is two history entries and must cost one Ctrl+Z, not two.
        if key.modifiers & !KeyModifiers::SHIFT == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Char('z') if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                    if self.undo_grouped() {
                        self.apply_edit_outcome();
                    }
                    return EventState::Consumed;
                }
                KeyCode::Char('y') | KeyCode::Char('Z') => {
                    if self.redo_grouped() {
                        self.apply_edit_outcome();
                    }
                    return EventState::Consumed;
                }
                _ => {}
            }
        }

        // FocusSidebar / FocusEditor shortcuts are intercepted at the
        // EditorScreen level for directional navigation.

        // Standard text-editor shortcuts.
        // `input_without_shortcuts` only handles chars, backspace, delete, tab, newline —
        // all navigation and editing shortcuts must be mapped explicitly.
        // Outcome tracks whether the handled shortcut mutated the buffer, only
        // moved the cursor, or did literally nothing (e.g. Ctrl+Z on an empty
        // undo stack) — so the revision clock is not
        // bumped on true no-ops.
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

        // Auto-surround: an opening/symmetric pair char typed over a selection
        // wraps it instead of replacing it (see CONTEXT.md "Auto-surround").
        // Shift is allowed (most opening chars are shifted keys); Ctrl/Alt
        // chords fall through. The selection lands on the inner text so wraps
        // chain: `[` `[` builds a wikilink — and `handle_input`'s post-key
        // sync legitimately opens the wikilink popup on the chained wrap.
        if let KeyCode::Char(c) = key.code
            && (key.modifiers & !KeyModifiers::SHIFT).is_empty()
            && let Some((open, close)) = surround_pair(c)
            && self.wrap_selection(open, close)
        {
            return EventState::Consumed;
        }

        // A change of modal state ends whatever run was open: leaving Insert
        // closes vim's session, and entering it starts a fresh one. Also read in
        // `handle_input` for the keys the engine consumes, which never arrive
        // here — this call is what keeps the direct path (which the tests drive)
        // bracketed too.
        self.sync_insert_session();

        // Read before the buffer is borrowed below.
        let in_insert_session = self.last_insert_session;
        let Some((ta, run)) = self.backend.as_textarea_parts_mut() else {
            unreachable!("handle_textarea_key called with non-Textarea backend")
        };
        // Last: what the key means to the plain backend. It runs *after* the
        // component's own claims — `Tab` indents rows, `Enter` may continue a
        // list, an opening bracket over a selection wraps it — because those are
        // the same keys and the component's reading of them wins.
        if let Some(op) = plain_keys::operation(*key) {
            // ↑/↓ move by *drawn* line, so they need the layout — which lives on
            // the view, not the buffer. Everything else the buffer can answer
            // alone. A run of them keeps its goal cell; anything else ends the run.
            let vertical = match op {
                plain_keys::Operation::Move {
                    to: CursorMove::Up,
                    extend,
                } => Some((false, extend)),
                plain_keys::Operation::Move {
                    to: CursorMove::Down,
                    extend,
                } => Some((true, extend)),
                _ => None,
            };
            if vertical.is_none() {
                self.view.clear_visual_goal();
            }
            // Does this keystroke continue the last one's **undo group**? The
            // policy is the backend's; the engine only offers a group
            // that can span keystrokes. Everything that is not typing ends the
            // run, which is what makes the idle rule correct without a timer:
            // undo is itself one of those things.
            if let Some(stroke) = stroke {
                {
                    let now = std::time::Instant::now();
                    // In vim's Insert mode the session is the group: `u` takes
                    // back everything typed since `i`, so neither a word boundary
                    // nor a pause may break it. `sync_insert_session` above ended
                    // the run at the boundary, which marks a session's start.
                    let carries_on = if in_insert_session {
                        run.continues_session(stroke, now)
                    } else {
                        run.continues(stroke, now)
                    };
                    if carries_on {
                        ta.continue_group();
                    }
                }
            }

            let changed = match vertical {
                // A stale layout — an edit landed before the frame that re-lays
                // it out — falls back to the logical move rather than reading it.
                Some((down, extend)) if self.view.move_cursor_visually(ta, down, extend) => false,
                _ => plain_keys::apply(op, ta),
            };
            self.selection = ta.selection_range();
            if changed {
                self.apply_edit_outcome();
            }
        }
        // A key the table declines — a function key, a modifier-only release, an
        // IME composition event — leaves the buffer alone, so a harmless keypress
        // cannot mark the note dirty and trigger an autosave.
        EventState::Consumed
    }

    /// Handle a mouse event (Textarea backend only).
    fn handle_mouse(
        &mut self,
        mouse: &ratatui::crossterm::event::MouseEvent,
        tx: &AppTx,
    ) -> EventState {
        if !self.covers(mouse.column, mouse.row) {
            return EventState::NotConsumed;
        }
        // Kept for the screen→layout conversion below, which is only reachable
        // past the bounds check that `covers` just made.
        let r = self.rect;
        // Past the bounds check the event is ours, so it is an action: a click
        // moves the cursor, and even a scroll means attention moved. Placed above
        // the context-menu return below so a right-click counts too.
        self.interrupt_typing();
        // Right-click: with a selection it copies (unchanged behavior);
        // without one it asks the host to open the note's context menu
        // (spec §10 — file & note ops).
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right))
            && self.selection.is_none_or(|(start, end)| start == end)
        {
            self.wants_context_menu = true;
            return EventState::Consumed;
        }
        // Everything below drives the textarea backend directly; on Nvim the
        // terminal/nvim own the mouse (only the context-menu ask above is
        // backend-independent).
        if !self.backend.is_textarea() {
            return EventState::NotConsumed;
        }
        // Handle right-click clipboard copy in its own scope to avoid borrow conflicts.
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right)) {
            self.copy_selection_to_clipboard(tx);
            self.selection = if let Some(ta) = self.backend.as_textarea() {
                ta.selection_range()
            } else {
                None
            };
            return EventState::Consumed;
        }
        // Now extract ta for remaining mouse operations.
        let Some(ta) = self.backend.as_textarea_mut() else {
            unreachable!()
        };
        match mouse.kind {
            MouseEventKind::Down(_) => {
                ta.cancel_selection();
                let (lrow, lcol) = self
                    .view
                    .click_at_screen((mouse.row - r.y) as usize, (mouse.column - r.x) as usize);
                ta.jump_to(lrow as usize, lcol as usize);
                ta.start_selection();
            }
            MouseEventKind::Drag(_) => {
                let (lrow, lcol) = self
                    .view
                    .click_at_screen((mouse.row - r.y) as usize, (mouse.column - r.x) as usize);
                ta.jump_to(lrow as usize, lcol as usize);
            }
            // Everything else is somebody else's: a click and a drag are handled
            // above, and a scroll is classified as an **Intent** before it reaches
            // the buffer. The incumbent forwarded these to the widget, which
            // scrolled a viewport kimün never renders from.
            _ => {}
        }
        self.selection = ta.selection_range();
        // Mouse handling moves the cursor / selection but does not insert
        // text — click, drag and scroll are all it produces.
        EventState::Consumed
    }
}

/// Viewport post-pass: emphasize search-needle matches
/// (`color_search_match`, bold) and style task checkboxes — `[ ]` accent,
/// `[x]` rows dimmed + struck (spec §5.1). Operates on the rendered buffer
/// rows, so cost is bounded by the visible area regardless of note size.
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
                        self.revs.current(),
                        self.view.last_cursor_screen,
                    )
                    && let Some(controller) = self.autocomplete.as_mut()
                {
                    match controller.handle_key(*key, &host) {
                        HandleKeyOutcome::Accepted(action) => {
                            self.interrupt_typing();
                            if let Some(ta) = self.backend.as_textarea_mut() {
                                ta.edit(|ta| apply_accept_to_textarea(ta, &action));
                                self.selection = ta.selection_range();
                            }
                            self.apply_edit_outcome();
                            return EventState::Consumed;
                        }
                        HandleKeyOutcome::Dismissed | HandleKeyOutcome::Consumed => {
                            return EventState::Consumed;
                        }
                        HandleKeyOutcome::NotHandled => {}
                    }
                }
                // Find bar intercepts all keys while active. Must run before the
                // vim engine, which would otherwise consume keys in Normal mode
                // (the textarea backend also intercepts inside handle_textarea_key,
                // but the vim Normal-mode path never reaches that).
                if self.dispatch_to_find_bar(key) {
                    // Here rather than inside `dispatch_bar`: that runs for every
                    // key whether or not a bar is open, so interrupting there
                    // would make each keystroke its own undo group.
                    self.interrupt_typing();
                    return EventState::Consumed;
                }
                // Vim interpreter: Normal/Visual consume the key here; Insert
                // mode returns PassThrough and falls into the direct path below
                // so typing, autocomplete, auto-surround and smart-Enter all
                // keep working.
                if let Some(outcome) = self.backend.vim_handle_key(key) {
                    use self::vim::VimKeyOutcome;
                    // Anything the engine consumed is an action rather than a
                    // continuation of typing. PassThrough is the exception: that
                    // key falls through to the direct path below, where the plain
                    // handler decides — and where a run of ↑/↓ keeps its goal.
                    if !matches!(outcome, VimKeyOutcome::PassThrough) {
                        self.interrupt_typing();
                    }
                    // Whatever the engine did, the buffer measured it. One
                    // drain replaces the group handshake, the pre-dispatch
                    // clone and the hand-placed revision bump.
                    self.apply_edit_outcome();
                    match outcome {
                        VimKeyOutcome::TextMutated => {
                            // No bump here — the drain above already applied
                            // what the buffer measured.
                            self.selection = None;
                            return EventState::Consumed;
                        }
                        VimKeyOutcome::CursorOnly => {
                            // Mirror the textarea's selection into self.selection so
                            // Visual mode renders through the existing selection pipeline.
                            // For non-visual CursorOnly (plain motion), selection_range()
                            // returns None → self.selection = None (no regression).
                            self.selection = self
                                .backend
                                .as_textarea()
                                .and_then(|ta| ta.selection_range());
                            // Charwise Visual highlight: extend end col by 1 so the
                            // char under the cursor is visually included (vim inclusive).
                            if self.backend.selection_includes_cursor()
                                && let Some(((sr, sc), (er, ec))) = self.selection
                            {
                                let len = self
                                    .backend
                                    .as_textarea()
                                    .and_then(|ta| ta.row(er))
                                    .map(|l| l.chars().count())
                                    .unwrap_or(ec);
                                self.selection = Some(((sr, sc), (er, (ec + 1).min(len))));
                            }
                            // Linewise Visual (`V`): the textarea's live selection is
                            // still just charwise under the hood (Head..End at the
                            // moment `V` was pressed), so its column only happens to
                            // span the full line until the cursor moves. Vim's own
                            // linewise Visual ignores column entirely — normalize to
                            // full width so every selected row highlights whole,
                            // no matter where the cursor sits within it.
                            if self.backend.is_visual_line()
                                && let Some(((sr, _), (er, _))) = self.selection
                            {
                                self.selection = Some(((sr, 0), (er, usize::MAX)));
                            }
                            self.refresh_autocomplete_if_open();
                            return EventState::Consumed;
                        }
                        VimKeyOutcome::NoOp => return EventState::Consumed,
                        VimKeyOutcome::PassThrough => { /* fall through to direct path */ }
                        VimKeyOutcome::Host(action) => {
                            use self::vim::VimHostAction;
                            match action {
                                VimHostAction::OpenPalette => {
                                    // Reuse the existing palette gateway.
                                    tx.send(AppEvent::ExecuteLeaderAction(
                                        crate::keys::leader::LeaderAction::Palette,
                                    ))
                                    .ok();
                                }
                                VimHostAction::OpenSearch { forward: _ } => {
                                    // `/` and `?` open the existing find bar.
                                    // (`?` backward-first is a later refinement;
                                    // n/N still navigate both directions.)
                                    self.open_or_advance_search();
                                }
                                VimHostAction::SearchNext => self.search_repeat(false),
                                VimHostAction::SearchPrev => self.search_repeat(true),
                                // Copy and Cut: the engine already did the
                                // editing and the mode transition; all that is
                                // left is the I/O and reporting it.
                                VimHostAction::ClipboardCopy(text) => {
                                    self.selection = None;
                                    crate::components::yank(text, "copied", tx);
                                }
                                VimHostAction::ClipboardCut(text) => {
                                    self.selection = None;
                                    crate::components::yank(text, "cut", tx);
                                }
                                // Paste is different: the engine deliberately
                                // left the range SELECTED rather than cutting
                                // it, so the replacement is atomic. Keep the
                                // selection live — `paste_text` consumes it, and
                                // a failed/empty read must leave it untouched.
                                VimHostAction::ClipboardPaste => {
                                    self.selection = self
                                        .backend
                                        .as_textarea()
                                        .and_then(|ta| ta.selection_range());
                                    self.paste_from_clipboard(tx);
                                }
                            }
                            return EventState::Consumed;
                        }
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
                let text_rev_before = self.revs.current();
                let cursor_before = self.textarea_cursor();
                let result = self.handle_textarea_key(key, tx);
                let cursor_after = self.textarea_cursor();
                if self.revs.current() != text_rev_before {
                    self.sync_autocomplete();
                } else if cursor_before != cursor_after {
                    self.refresh_autocomplete_if_open();
                }
                result
            }
            InputEvent::Mouse(mouse) => {
                let text_rev_before = self.revs.current();
                let cursor_before = self.textarea_cursor();
                let result = self.handle_mouse(mouse, tx);
                let cursor_after = self.textarea_cursor();
                // Mouse clicks typically only move the cursor — refresh
                // (which may close the popup) but do not auto-open.
                if self.revs.current() != text_rev_before {
                    self.sync_autocomplete();
                } else if cursor_before != cursor_after {
                    self.refresh_autocomplete_if_open();
                }
                // A press only places the cursor. Following is a double-click
                // (see `app_screen::click_run`) and is not decided here: it classifies to
                // `EditorIntent::FollowLink` — the same intent Ctrl+N produces
                // — and the editor screen executes it against this cursor.
                // The press that placed the cursor is the *first* of the pair,
                // which is why one path can serve both.

                // Plan 3 Task 5: reconcile the vim engine mode from whether the
                // textarea selection is live after the mouse event. A drag that
                // creates a selection enters Visual; a click that clears one
                // returns to Normal. Insert mode is left untouched (the engine
                // match arm is a no-op for all modes other than Normal/Visual).
                // A bare click leaves a collapsed (zero-width) selection active
                // because handle_mouse's Down arm calls start_selection().
                // Only treat a NON-EMPTY selection as "real" to avoid flipping
                // vim Normal→Visual on a plain click.  Mirrors the same guard
                // at ~line 1014 which protects auto-indent from collapsed sel.
                let has_sel = self
                    .backend
                    .as_textarea()
                    .and_then(|ta| ta.selection_range())
                    .is_some_and(|(s, e)| s != e);
                self.backend.sync_mouse_selection(has_sel);
                result
            }
            // Bracketed paste is intercepted by EditorScreen so it can run the
            // image-paste flow first. It never reaches us here.
            InputEvent::Paste(_) => EventState::NotConsumed,
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        // Reserve the bottom row(s) for the find bar when active — one while
        // finding, two once a **replace field** is revealed (row one is the
        // pattern and what it matches, row two the replacement and what
        // happens to it).
        let bar_rows: u16 = self.search.as_ref().map_or(0, |bar| bar.rows());
        // Clamp rather than drop: with `rect.height == bar_rows` the old
        // `>` left the bar unrendered while it was still open and still
        // swallowing every key — an invisible modal. Better to show it and
        // give the editor whatever is left, even if that is nothing.
        let bar_rows = bar_rows.min(rect.height);
        let (editor_rect, search_rect) = if bar_rows > 0 {
            (
                Rect {
                    height: rect.height - bar_rows,
                    ..rect
                },
                Some(Rect {
                    y: rect.y + rect.height - bar_rows,
                    height: bar_rows,
                    ..rect
                }),
            )
        } else {
            (rect, None)
        };
        // Store the editor area (not the full rect) so mouse hit-testing ignores
        // clicks on the find-bar row.
        self.rect = editor_rect;
        // Phase 1: gather the per-backend selection (and, on Nvim, run the
        // frame housekeeping — resize). The revision is NOT read here: the
        // snapshot below is the single producer, and `revs` adopts its
        // value, so dirty tracking and the view always agree in a frame.
        let selection = match &self.backend {
            // While the bar is open the **current match** is what gets painted
            // as the selection — the bar owns it rather than writing this field.
            BackendState::Textarea(_) => match self.search.as_ref() {
                Some(bar) => bar.current_match(),
                None => self.selection,
            },
            BackendState::Nvim(nvim) => {
                self.nvim_host
                    .frame_sync(nvim, editor_rect.width, editor_rect.height)
            }
        };
        // Drain any completed background full-parse results BEFORE
        // running view.update so a just-finished async parse lands
        // before Gate 1 has a chance to install another placeholder.
        // Generation mismatches drop silently (the spawned task's
        // input is older than the current buffer).
        while let Ok((generation, buf)) = self.full_parse_rx.try_recv() {
            self.view.install_full_parse(generation, buf);
        }
        // Same drain, for a just-finished background full wrap. Order
        // relative to the parse drain above does not matter — the two
        // are staleness-gated independently on `generation`.
        while let Ok((generation, layout)) = self.layout_rx.try_recv() {
            self.view.install_full_layout(generation, layout);
        }

        // Phase 2: single producer for the atomic snapshot. Borrowed
        // on Textarea (zero clone), owned on Nvim (lines cloned out
        // from behind the Mutex). Use the free function so the borrow
        // checker can split `&self.backend` from `&mut self.view`.
        // The **replace preview** is computed before the snapshot borrow so it
        // owns its lines outright. The buffer is never touched — only this
        // frame's view of it is substituted, which is what makes the preview
        // structurally incapable of committing.
        // One call gets everything the bar wants painted: preview lines and
        // spans, match spans, and the current match (a candidate seam).
        let overlay = match (self.search.as_ref(), self.backend.as_textarea()) {
            (Some(bar), Some(buf)) => bar.overlay(buf),
            _ => find_bar::BarOverlay::default(),
        };
        let preview = overlay.preview;
        let snap = snapshot_from_backend(&self.backend, self.revs.current());
        // One revision domain: adopt the snapshot's value (the nvim arm
        // derived it from the backend's `content_gen` under one lock; the
        // textarea arm passed `revs.current()` through — a no-op adopt).
        // Adopt from the REAL snapshot, never the preview's synthetic
        // revision, or dirty tracking would follow the preview.
        self.revs.adopt(snap.content_revision);
        // The lines the view actually draws this frame: the preview's when one
        // is showing, the buffer's otherwise. Kept in scope because the
        // deferred full-parse below must parse *these*, not the buffer's.
        let (view_lines, preview_spans) = match preview {
            None => (None, Vec::new()),
            Some(p) => (Some(p.lines), p.spans),
        };
        match &view_lines {
            None => self.view.update(&snap, editor_rect),
            Some(lines) => {
                // The parse cache keys on `content_revision`, so the preview
                // carries an identity of its own — derived from the real
                // revision plus what is being previewed. Same preview, same
                // key: the cache still works instead of thrashing per frame.
                let rev = preview_revision(snap.content_revision, lines);
                let view_snap = EditorSnapshot::borrowed(lines, snap.cursor, rev);
                self.view.update(&view_snap, editor_rect);
            }
        }
        // Needles reach the view as **overlays** now; the cell-space post-pass
        // that used to paint them is gone, and with it the coordinate split
        // that let a find pattern match text it could never highlight.
        if self.revs.needles_stale() {
            self.search_needles.clear();
            self.revs.disarm_needles();
        }
        self.view.set_needles(self.search_needles.clone());

        // Assemble this frame's **overlays**. The view appends the two kinds it
        // derives from content (tasks, needles) for the visible rows.
        let mut overlays: Vec<view::Overlay> = Vec::new();
        if let Some(((sr, sc), (er, ec))) = selection {
            // A multi-row selection becomes one overlay per row; the middle
            // rows run to their full width, which `restyle_over_range` clamps.
            for row in sr..=er {
                let start = if row == sr { sc } else { 0 };
                let end = if row == er { ec } else { usize::MAX };
                overlays.push(view::Overlay::new(
                    row,
                    start,
                    end,
                    view::OverlayKind::Selection,
                ));
            }
        }
        overlays.extend(preview_spans.iter().map(|p| {
            view::Overlay::new(
                p.row,
                p.start,
                p.end,
                if p.is_current {
                    view::OverlayKind::PreviewCurrent
                } else {
                    view::OverlayKind::Preview
                },
            )
        }));
        // Find-bar matches. Skipped while previewing: those columns already
        // carry the preview colour, which is the more important fact.
        if view_lines.is_none() {
            overlays.extend(overlay.matches.iter().map(|&(row, start, end)| {
                view::Overlay::new(row, start, end, view::OverlayKind::Match)
            }));
        }
        self.view.set_overlays(overlays);

        // If `view.update` cap-tripped on a large buffer it
        // installed a placeholder + pending-flag instead of running
        // ParsedBuffer::parse synchronously. Spawn the real parse
        // here so subsequent frames pick up the rich result via the
        // drain loop above. `SingleSlotTask::spawn` aborts the prior
        // task, so a burst of large-buffer edits resolves against
        // the latest content.
        if let Some(generation) = self.view.take_pending_full_parse() {
            // Parse the lines the view is DRAWING, not the buffer's. Under a
            // **replace preview** those differ, and the placeholder this
            // generation came from was keyed on the preview's synthetic
            // revision — so handing over the buffer's lines would install a
            // parse of text that is not on screen and sail through
            // `install_full_parse`'s staleness check, styling a large note by
            // element boundaries computed against a different string.
            // The task gets the text itself. A clone shares its structure, so
            // handing a 5000-row note to a background parse costs a pointer
            // rather than a copy of the note — where this used to clone every
            // row.
            let text = match &view_lines {
                Some(lines) => crate::ropetext::Text::from(lines.join("\n").as_str()),
                None => snap.text.clone(),
            };
            let tx = self.full_parse_tx.clone();
            let redraw = self.redraw_tx.clone();
            self.full_parse_task.spawn(async move {
                let buf = ParsedBuffer::parse(&text);
                let _ = tx.send((generation, buf));
                // Wake the render loop so the rich parse lands
                // without waiting for the next keystroke.
                if let Some(redraw) = redraw {
                    let _ = redraw.send(AppEvent::Redraw);
                }
            });
        }
        // Same shape, for the layout side: `view.update` may have installed
        // a `Layout::unwrapped` stub instead of blocking on `Layout::compute`.
        // The job already carries its own `Text`/`rendered_cache`/
        // `gutter_insets` clones — nothing here needs `view_lines`/`snap`.
        if let Some(job) = self.view.take_pending_full_layout() {
            let tx = self.layout_tx.clone();
            let redraw = self.redraw_tx.clone();
            self.layout_task.spawn(async move {
                let hints = view::row_hints(&job.rendered_cache, &job.gutter_insets);
                let layout = crate::ropetext::Layout::compute(
                    &job.text,
                    job.width,
                    crate::ropetext::Metrics::default(),
                    &hints,
                );
                let _ = tx.send((job.generation, layout));
                if let Some(redraw) = redraw {
                    let _ = redraw.send(AppEvent::Redraw);
                }
            });
        }
        // When the find bar is active, draw it AFTER the editor so its caret
        // (set via set_cursor_position) wins over the editor's caret call.
        let bar_focused = self.search.is_some() && focused;
        let editor_focused = focused && !bar_focused;
        use self::view::CursorShape;
        let cursor_shape = match self.backend.modal_is_insert() {
            None => None, // Direct textarea — leave terminal default
            Some(true) => Some(CursorShape::Bar),
            Some(false) => Some(CursorShape::Block),
        };
        self.view
            .render(f, editor_rect, theme, editor_focused, cursor_shape);

        // Search-match emphasis (spec §5.1): paint needle matches and task
        // checkboxes over the rendered viewport. Buffer-level post-pass —
        // viewport-only, so large notes pay nothing beyond the visible rows.
        if self.revs.needles_stale() {
            self.search_needles.clear();
            self.revs.disarm_needles();
        }

        // Empty-note tip (spec §5.2): dim ghost text in a fresh/empty buffer,
        // gone the instant the first character lands (the buffer stops being
        // empty). Drawn after the view so it sits over the blank canvas.
        if snap.text.len_bytes() == 0 && editor_rect.height > 0 {
            let leader = self
                .key_bindings
                .first_combo_for(&crate::keys::action_shortcuts::ActionShortcuts::Leader)
                .unwrap_or_else(|| "leader".to_string());
            f.render_widget(
                ratatui::widgets::Paragraph::new(format!(
                    "Type to start · [[ to link · # to tag · {leader} for commands"
                ))
                .style(
                    Style::default()
                        .fg(theme.gray.to_ratatui())
                        .add_modifier(Modifier::ITALIC),
                ),
                Rect {
                    x: editor_rect.x.saturating_add(2),
                    width: editor_rect.width.saturating_sub(2),
                    height: 1,
                    ..editor_rect
                },
            );
        }
        if let (Some(state), Some(bar_rect)) = (self.search.as_mut(), search_rect) {
            state.render(f, bar_rect, theme, bar_focused);
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

        // Prepend the modal-mode label (nvim or vim) as the first "hint".
        // When the vim interpreter has a pending command sequence (e.g. "2d",
        // "f", ">"), append it to the label so the user can see what they have
        // typed so far.
        if let Some(mut label) = self.backend.mode_label() {
            if let Some(p) = self.backend.pending_input_hint() {
                label = format!("{label}  {p}");
            }
            let mut hints = vec![(String::new(), label)];
            hints.extend(
                [
                    (ActionShortcuts::FocusSidebar, "\u{2190} focus left"),
                    (ActionShortcuts::FocusEditor, "focus right \u{2192}"),
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

        // Cursor-context hints come first: what the cursor is on decides the
        // most relevant action (spec §5.2).
        let mut hints: Vec<(String, String)> = Vec::new();
        match self.follow_target_at_cursor() {
            Some(FollowTarget::Link(_)) => {
                if let Some(k) = self
                    .key_bindings
                    .first_combo_for(&ActionShortcuts::FollowLink)
                {
                    hints.push((k, "follow link".to_string()));
                }
            }
            Some(FollowTarget::Label(_)) => {
                if let Some(k) = self
                    .key_bindings
                    .first_combo_for(&ActionShortcuts::FollowLink)
                {
                    hints.push((k, "browse tag".to_string()));
                }
            }
            None => {}
        }
        hints.extend(crate::components::hints::hints_for(
            &self.key_bindings,
            &[
                (ActionShortcuts::FocusSidebar, "\u{2190} focus left"),
                (ActionShortcuts::FocusEditor, "focus right \u{2192}"),
                (ActionShortcuts::FileOperations, "file ops"),
                (ActionShortcuts::FindInBuffer, "find"),
            ],
        ));
        hints
    }
}

#[cfg(test)]
mod tests {
    use super::snapshot::EditorMode;
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

    fn get_ta(editor: &mut TextEditorComponent) -> &mut RopeBuffer {
        match &mut editor.backend {
            BackendState::Textarea(tb) => &mut tb.ta,
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
        // Send a cursor-only key (Right arrow). It must NOT advance the
        // revision clock, so `is_dirty` stays false.
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
        ta.move_cursor(CursorMove::WordForward);
        assert!(ta.selection_range().is_some());
        ta.cancel_selection();
        editor.selection = if let BackendState::Textarea(tb) = &editor.backend {
            tb.ta.selection_range()
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
        ta.move_cursor(CursorMove::Head);
        ta.start_selection();
        ta.move_cursor(CursorMove::WordForward);
        let range = ta.selection_range().unwrap();
        let ((sr, sc), (er, ec)) = range;
        let lines = ta.rows();
        let selected = if sr == er {
            lines[sr][sc..ec].to_string()
        } else {
            lines[sr][sc..].to_string()
        };
        assert_eq!(selected, "hello ");
    }

    /// Selects the char-coordinate range `start..end` in the editor's textarea.
    fn select_range(editor: &mut TextEditorComponent, start: (usize, usize), end: (usize, usize)) {
        let ta = get_ta(editor);
        ta.cancel_selection();
        ta.move_cursor(CursorMove::Jump(start.0, start.1));
        ta.start_selection();
        ta.move_cursor(CursorMove::Jump(end.0, end.1));
        assert!(ta.selection_range().is_some());
    }

    fn send_char(editor: &mut TextEditorComponent, c: char) {
        let tx = dummy_tx();
        let key = ratatui::crossterm::event::KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        let _ = editor.handle_input(&InputEvent::Key(key), &tx);
    }

    #[test]
    fn surround_pair_maps_open_and_symmetric_chars() {
        assert_eq!(surround_pair('('), Some(("(", ")")));
        assert_eq!(surround_pair('['), Some(("[", "]")));
        assert_eq!(surround_pair('{'), Some(("{", "}")));
        assert_eq!(surround_pair('<'), Some(("<", ">")));
        assert_eq!(surround_pair('"'), Some(("\"", "\"")));
        assert_eq!(surround_pair('\''), Some(("'", "'")));
        assert_eq!(surround_pair('`'), Some(("`", "`")));
        assert_eq!(surround_pair('*'), Some(("*", "*")));
        assert_eq!(surround_pair('_'), Some(("_", "_")));
        assert_eq!(surround_pair('~'), Some(("~", "~")));
        // Closing chars and plain chars never wrap.
        assert_eq!(surround_pair(')'), None);
        assert_eq!(surround_pair(']'), None);
        assert_eq!(surround_pair('}'), None);
        assert_eq!(surround_pair('>'), None);
        assert_eq!(surround_pair('a'), None);
    }

    #[test]
    fn typing_open_paren_with_selection_wraps_it() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        select_range(&mut editor, (0, 0), (0, 5)); // "hello"
        send_char(&mut editor, '(');
        assert_eq!(editor.get_text(), "(hello) world");
        assert!(editor.is_dirty(), "wrap must mark the buffer dirty");
    }

    #[test]
    fn wrap_keeps_selection_on_inner_text() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        select_range(&mut editor, (0, 0), (0, 5));
        send_char(&mut editor, '(');
        // Selection must cover "hello" inside the parens so wraps chain.
        assert_eq!(editor.selection, Some(((0, 1), (0, 6))));
    }

    #[test]
    fn chained_brackets_build_a_wikilink() {
        let mut editor = make_editor();
        editor.set_text("my note".to_string());
        select_range(&mut editor, (0, 0), (0, 7));
        send_char(&mut editor, '[');
        send_char(&mut editor, '[');
        assert_eq!(editor.get_text(), "[[my note]]");
        assert_eq!(editor.selection, Some(((0, 2), (0, 9))));
    }

    #[test]
    fn symmetric_chars_wrap_and_chain() {
        let mut editor = make_editor();
        editor.set_text("bold".to_string());
        select_range(&mut editor, (0, 0), (0, 4));
        send_char(&mut editor, '*');
        assert_eq!(editor.get_text(), "*bold*");
        send_char(&mut editor, '*');
        assert_eq!(editor.get_text(), "**bold**");
        assert_eq!(editor.selection, Some(((0, 2), (0, 6))));
    }

    #[test]
    fn closing_char_replaces_selection() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        select_range(&mut editor, (0, 0), (0, 5));
        send_char(&mut editor, ')');
        assert_eq!(editor.get_text(), ") world");
    }

    #[test]
    fn open_char_without_selection_inserts_normally() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let ta = get_ta(&mut editor);
        ta.move_cursor(CursorMove::End);
        send_char(&mut editor, '(');
        assert_eq!(editor.get_text(), "hello(");
    }

    #[test]
    fn wrap_spans_multiline_selection() {
        let mut editor = make_editor();
        editor.set_text("abc\ndef".to_string());
        select_range(&mut editor, (0, 0), (1, 3));
        send_char(&mut editor, '(');
        assert_eq!(editor.get_text(), "(abc\ndef)");
        // Inner selection: open char shifts only the first line.
        assert_eq!(editor.selection, Some(((0, 1), (1, 3))));
    }

    #[test]
    fn wrap_handles_multibyte_selection() {
        let mut editor = make_editor();
        editor.set_text("héllo🦀 x".to_string());
        select_range(&mut editor, (0, 0), (0, 6)); // "héllo🦀" = 6 chars
        send_char(&mut editor, '`');
        assert_eq!(editor.get_text(), "`héllo🦀` x");
        assert_eq!(editor.selection, Some(((0, 1), (0, 7))));
    }

    #[test]
    fn wrap_with_reversed_selection_direction() {
        // Selection made right-to-left must wrap the same way.
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        select_range(&mut editor, (0, 5), (0, 0));
        send_char(&mut editor, '(');
        assert_eq!(editor.get_text(), "(hello) world");
        assert_eq!(editor.selection, Some(((0, 1), (0, 6))));
    }

    #[test]
    fn text_action_keeps_selection_on_inner_text() {
        // Bold/Italic/Strikethrough route through the same wrap mechanism as
        // auto-surround: the inner text stays selected so wraps chain.
        let mut editor = make_editor();
        editor.set_text("bold word".to_string());
        select_range(&mut editor, (0, 0), (0, 4));
        editor.apply_text_action(TextAction::Bold);
        assert_eq!(editor.get_text(), "**bold** word");
        assert_eq!(editor.selection, Some(((0, 2), (0, 6))));
    }

    #[test]
    fn bold_undo_is_one_step_back_to_original() {
        // The sibling of the wrap: `apply_text_action` reaches the same
        // `wrap_selection`, so bolding a selection is one entry for the same
        // reason. Pinned separately because it is the path a toolbar action
        // takes, and nothing else would catch it regressing on its own.
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        select_range(&mut editor, (0, 0), (0, 5));
        editor.apply_text_action(TextAction::Bold);
        assert_eq!(editor.get_text(), "**hello** world");
        assert!(get_ta(&mut editor).undo(), "the bold is one entry");
        assert_eq!(editor.get_text(), "hello world");
        assert!(
            !get_ta(&mut editor).undo(),
            "and has no second half left to take back"
        );
    }

    #[test]
    fn wrap_undo_is_one_step_back_to_original() {
        // A wrap replaces the selection inside a single transaction, so the whole
        // gesture is one history entry. Under the incumbent it was delete+insert
        // and cost two, and this test asked for two undos — which proved nothing,
        // since a second undo against a one-entry history is a no-op and lands on
        // the same string. Asserting what each undo *returns* is what makes this a
        // claim about grouping rather than about the final text.
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        select_range(&mut editor, (0, 0), (0, 5));
        send_char(&mut editor, '(');
        assert_eq!(editor.get_text(), "(hello) world");
        assert!(get_ta(&mut editor).undo(), "the wrap is one entry");
        assert_eq!(editor.get_text(), "hello world");
        assert!(
            !get_ta(&mut editor).undo(),
            "and has no second half left to take back"
        );
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

    /// Arrive-from-query needles survive until the first edit.
    #[test]
    fn search_needles_clear_on_edit() {
        let settings = crate::settings::AppSettings::default();
        let mut ed = TextEditorComponent::new(settings.key_bindings.clone(), &settings);
        ed.set_text("alpha beta".to_string());
        ed.set_search_needles(vec!["Alpha".to_string()]);
        assert_eq!(ed.search_needles, vec!["alpha"]);
        assert!(!ed.revs.needles_stale());

        // An edit bumps the revision; the render-side guard would clear.
        ed.set_text("alpha beta gamma".to_string());
        assert!(ed.revs.needles_stale());
    }

    #[test]
    fn jump_to_heading_moves_cursor_to_heading_line() {
        let settings = crate::settings::AppSettings::default();
        let mut ed = TextEditorComponent::new(settings.key_bindings.clone(), &settings);
        ed.set_text("intro\n# Top\nbody\n## Sub One\nmore\n".to_string());

        ed.jump_to_heading("Sub One");
        assert_eq!(ed.view_snapshot().cursor.0, 3);

        ed.jump_to_heading("Top");
        assert_eq!(ed.view_snapshot().cursor.0, 1);

        // Unknown heading: cursor stays.
        ed.jump_to_heading("Nope");
        assert_eq!(ed.view_snapshot().cursor.0, 1);
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
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            &tx,
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('b'), KeyModifiers::NONE)),
            &tx,
        );
        // Cursor now at first match (col 0). Re-invoking advances to second.
        editor.open_or_advance_search();
        let (_, col) = get_ta(&mut editor).cursor();
        assert_eq!(col, 3, "second invocation advances to next match");
    }

    #[test]
    fn typing_in_find_bar_jumps_cursor_to_first_match() {
        let mut editor = make_editor();
        editor.set_text("foo bar baz".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        for ch in ['b', 'a', 'r'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(ch), KeyModifiers::NONE)),
                &tx,
            );
        }
        let state = editor.search.as_ref().unwrap();
        assert_eq!(state.input.value(), "bar");
        assert!(matches!(state.status, SearchStatus::Match));
        let (_, col) = get_ta(&mut editor).cursor();
        assert_eq!(col, 4, "cursor jumped to start of 'bar'");
    }

    #[test]
    fn enter_in_find_bar_advances_to_next_match() {
        let mut editor = make_editor();
        editor.set_text("ab ab ab".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            &tx,
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('b'), KeyModifiers::NONE)),
            &tx,
        );
        // first match is at col 0 (match_cursor=true on type)
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        let (_, col) = get_ta(&mut editor).cursor();
        assert_eq!(col, 3, "Enter advances to second match");
    }

    #[test]
    fn match_is_highlighted_as_selection_after_search() {
        let mut editor = make_editor();
        editor.set_text("foo bar baz".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        for ch in ['b', 'a', 'r'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(ch), KeyModifiers::NONE)),
                &tx,
            );
        }
        // "bar" lives at cols 4..7 on row 0. The **current match** belongs to
        // the bar now, not to the editor's selection.
        assert_eq!(
            editor.search.as_ref().unwrap().current_match(),
            Some(((0, 4), (0, 7)))
        );
    }

    #[test]
    fn no_match_clears_selection() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('z'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.selection, None);
    }

    #[test]
    fn esc_in_find_bar_clears_selection_highlight() {
        let mut editor = make_editor();
        editor.set_text("foo bar".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('b'), KeyModifiers::NONE)),
            &tx,
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            &tx,
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('r'), KeyModifiers::NONE)),
            &tx,
        );
        assert!(
            editor
                .search
                .as_ref()
                .is_some_and(|b| b.current_match().is_some())
        );
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        // Esc drops the bar, and the current match goes with it.
        assert!(editor.search.is_none());
        assert!(editor.selection.is_none());
    }

    #[test]
    fn esc_in_find_bar_closes_it() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        assert!(editor.search.is_some());
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        assert!(editor.search.is_none());
    }

    #[test]
    fn find_bar_consumes_typing_so_editor_text_is_unchanged() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "hello");
    }

    #[test]
    fn no_match_status_when_query_absent() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let tx = dummy_tx();
        editor.open_or_advance_search();
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('z'), KeyModifiers::NONE)),
            &tx,
        );
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
            ta.move_cursor(CursorMove::End);
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
            ta.move_cursor(CursorMove::Head);
            ta.start_selection();
            ta.move_cursor(CursorMove::WordForward);
        }
        editor.insert_at_cursor("HEY ", &dummy_tx());
        assert_eq!(editor.get_text(), "HEY world");
    }

    #[test]
    fn paste_inserts_text_at_cursor() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let ta = get_ta(&mut editor);
        ta.move_cursor(CursorMove::End);
        ta.insert_str(" world");
        assert_eq!(editor.get_text(), "hello world");
    }

    #[test]
    fn bold_action_with_no_selection_inserts_pair_and_centers_cursor() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::End);
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
            ta.move_cursor(CursorMove::Head);
            ta.start_selection();
            ta.move_cursor(CursorMove::WordForward);
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
            ta.move_cursor(CursorMove::Head);
            ta.move_cursor(CursorMove::WordForward);
            ta.start_selection();
            ta.move_cursor(CursorMove::WordForward);
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
            ta.move_cursor(CursorMove::Head);
            ta.start_selection();
            ta.move_cursor(CursorMove::WordForward);
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
            ta.move_cursor(CursorMove::Bottom);
        }
        editor.indent_lines(false);
        let lines = get_ta(&mut editor).rows();
        assert_eq!(lines[0], "foo");
        assert!(lines[1].starts_with(' ') || lines[1].starts_with('\t'));
        assert!(lines[1].trim_start() == "bar");
    }

    #[test]
    fn indent_midline_selection_keeps_text_before_and_selection() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::Jump(0, 6));
            ta.start_selection();
            ta.move_cursor(CursorMove::End);
        }
        editor.indent_lines(false);
        let ta = get_ta(&mut editor);
        // Text before the selection must survive; only a leading indent added.
        assert_eq!(ta.rows()[0].trim_start(), "hello world");
        // Selection preserved, shifted right by the inserted indent.
        let indent = ta.rows()[0].len() - "hello world".len();
        assert_eq!(
            ta.selection_range(),
            Some(((0, 6 + indent), (0, 11 + indent)))
        );
    }

    #[test]
    fn indent_with_selection_indents_all_touched_lines() {
        let mut editor = make_editor();
        editor.set_text("foo\nbar\nbaz".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::Top);
            ta.start_selection();
            ta.move_cursor(CursorMove::Down);
            ta.move_cursor(CursorMove::End);
        }
        editor.indent_lines(false);
        let lines: Vec<String> = get_ta(&mut editor).rows().to_vec();
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
        let tab_len = get_ta(&mut editor).indent_width() as usize;
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::Top);
            ta.start_selection();
            ta.move_cursor(CursorMove::Bottom);
            ta.move_cursor(CursorMove::End);
        }
        editor.indent_lines(true);
        let lines: Vec<String> = get_ta(&mut editor).rows().to_vec();
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
            ta.move_cursor(CursorMove::End);
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
            ta.move_cursor(CursorMove::End);
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
            ta.move_cursor(CursorMove::End);
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
            ta.move_cursor(CursorMove::End);
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
            ta.move_cursor(CursorMove::End);
        }
        let tab_len = get_ta(&mut editor).indent_width() as usize;
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
            ta.move_cursor(CursorMove::End);
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
            ta.move_cursor(CursorMove::Head);
            ta.move_cursor(CursorMove::Forward);
            ta.move_cursor(CursorMove::Forward);
        }
        assert!(!editor.smart_enter());
    }

    #[test]
    fn smart_enter_on_empty_indented_list_marker_dedents_keeping_marker() {
        let mut editor = make_editor();
        let tab_len = get_ta(&mut editor).indent_width() as usize;
        let indent = " ".repeat(tab_len);
        editor.set_text(format!("{indent}- "));
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::End);
        }
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "- ");
    }

    #[test]
    fn smart_enter_on_empty_list_marker_clears_line_after_full_dedent() {
        let mut editor = make_editor();
        let tab_len = get_ta(&mut editor).indent_width() as usize;
        let indent = " ".repeat(tab_len);
        editor.set_text(format!("{indent}- "));
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::End);
        }
        // First Enter: dedent to "- ".
        assert!(editor.smart_enter());
        assert_eq!(editor.get_text(), "- ");
        // Second Enter at column == end-of-line: now cursor is at col 2 (end of "- ").
        // Need to position cursor at end after the dedent.
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::End);
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
            ta.move_cursor(CursorMove::End);
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
            ta.move_cursor(CursorMove::End);
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
            ta.move_cursor(CursorMove::End);
        }
        assert!(editor.smart_enter());
        // tab counts as one indent unit, regardless of indent_width spaces.
        assert_eq!(editor.get_text(), "\t");
    }

    #[test]
    fn smart_enter_continues_indented_list() {
        let mut editor = make_editor();
        editor.set_text("  - foo".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::End);
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

    // ── follow_target_at_cursor: label detection ──────────────────────────────────────

    /// Helper: place cursor at a specific column on the first row.
    fn place_cursor_at_col(editor: &mut TextEditorComponent, col: usize) {
        let ta = get_ta(editor);
        ta.move_cursor(CursorMove::Head);
        for _ in 0..col {
            ta.move_cursor(CursorMove::Forward);
        }
    }

    #[test]
    fn follow_target_at_cursor_returns_label_when_cursor_on_hashtag() {
        let mut editor = make_editor();
        editor.set_text("see #rust now".to_string());
        // "#rust" starts at col 4, ends at col 9 (5 chars). Place cursor at col 5 (inside).
        place_cursor_at_col(&mut editor, 5);
        assert_eq!(
            editor.follow_target_at_cursor(),
            Some(FollowTarget::Label("rust".into())),
        );
    }

    #[test]
    fn follow_target_at_cursor_returns_label_at_hash_char() {
        let mut editor = make_editor();
        editor.set_text("see #rust now".to_string());
        // Cursor exactly on '#' (col 4).
        place_cursor_at_col(&mut editor, 4);
        assert_eq!(
            editor.follow_target_at_cursor(),
            Some(FollowTarget::Label("rust".into())),
        );
    }

    #[test]
    fn follow_target_at_cursor_returns_none_outside_hashtag() {
        let mut editor = make_editor();
        editor.set_text("see #rust now".to_string());
        // Cursor at col 0 ("s") — not on a hashtag.
        place_cursor_at_col(&mut editor, 0);
        assert_eq!(editor.follow_target_at_cursor(), None);
    }

    #[test]
    fn follow_target_at_cursor_returns_link_for_wikilink() {
        let mut editor = make_editor();
        editor.set_text("open [[my note]] please".to_string());
        // "my note" is inside [[…]]; cursor at col 7 (inside link text).
        place_cursor_at_col(&mut editor, 7);
        let result = editor.follow_target_at_cursor();
        assert!(
            matches!(result, Some(FollowTarget::Link(_))),
            "expected Link variant, got {result:?}"
        );
    }

    // ── F5: follow_target_at_cursor prioritises Link over Label ────────────────────────

    #[test]
    fn follow_target_at_cursor_returns_link_for_markdown_link_with_fragment() {
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
        let result = editor.follow_target_at_cursor();
        assert!(
            matches!(result, Some(FollowTarget::Link(_))),
            "expected Link variant for markdown link fragment, got {result:?}"
        );
    }

    #[test]
    fn vim_normal_i_then_typing_inserts_text() {
        let mut settings = crate::settings::AppSettings::default();
        settings.editor_backend = crate::settings::EditorBackendSetting::Vim;
        let mut editor = TextEditorComponent::new(KeyBindings::empty(), &settings);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        // In Normal mode, 'x' is unmapped → no text change.
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "");
        // 'i' enters Insert; then 'x' types a literal x via the direct path.
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('i'), KeyModifiers::NONE)),
            &tx,
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "x");
    }

    // ── Find and replace ─────────────────────

    /// Drive the find bar: open it, type `pattern`, reveal the replace field
    /// with Tab, type `replacement`. Leaves the bar open and focused.
    fn open_replace_bar(
        editor: &mut TextEditorComponent,
        tx: &AppTx,
        pattern: &str,
        replacement: &str,
    ) {
        editor.open_or_advance_search();
        for c in pattern.chars() {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                tx,
            );
        }
        editor.handle_input(&InputEvent::Key(key(KeyCode::Tab, KeyModifiers::NONE)), tx);
        for c in replacement.chars() {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                tx,
            );
        }
    }

    #[test]
    fn tab_reveals_the_replace_field_and_then_cycles_focus() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo".to_string());
        editor.open_or_advance_search();
        assert!(
            !editor.search.as_ref().unwrap().is_replacing(),
            "a find-only bar must not start with a replace field"
        );

        editor.handle_input(&InputEvent::Key(key(KeyCode::Tab, KeyModifiers::NONE)), &tx);
        let s = editor.search.as_ref().unwrap();
        assert!(s.is_replacing(), "Tab must reveal the replace field");
        // Pattern is empty, so focus stays in the find field — you cannot type
        // a replacement for nothing.
        assert_eq!(s.focus, BarFocus::Find);

        editor.handle_input(&InputEvent::Key(key(KeyCode::Tab, KeyModifiers::NONE)), &tx);
        assert_eq!(editor.search.as_ref().unwrap().focus, BarFocus::Replace);
        editor.handle_input(&InputEvent::Key(key(KeyCode::Tab, KeyModifiers::NONE)), &tx);
        assert_eq!(editor.search.as_ref().unwrap().focus, BarFocus::Find);
    }

    #[test]
    fn typing_in_the_replace_field_does_not_touch_the_buffer() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo and todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "done");
        assert_eq!(
            editor.get_text(),
            "todo and todo",
            "the preview is a view of the note, never a write to it"
        );
    }

    #[test]
    fn enter_replaces_the_current_match_and_advances() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo and todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "done");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "done and todo");
    }

    #[test]
    fn replacing_a_match_that_ends_inside_a_cluster_is_refused_not_corrupted() {
        // "e\u{301}f" is a decomposed é followed by f. Searching `e` matches a
        // scalar whose END sits inside the cluster, which is not an addressable
        // column — so the second jump does nothing, the selection stays empty,
        // and the replacement used to be INSERTED beside the match rather than
        // over it, leaving "xe\u{301}f". Refusing is the contract.
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("e\u{301}f".to_string());
        open_replace_bar(&mut editor, &tx, "e", "x");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(
            editor.get_text(),
            "e\u{301}f",
            "the note is left alone rather than half-rewritten"
        );
    }

    #[test]
    fn ctrl_a_replaces_every_match() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo and todo\nmore todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "done");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "done and done\nmore done");
    }

    #[test]
    fn replace_all_keeps_the_reading_position() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo\nxx\ntodo\nyy".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "done");
        // Park the cursor on row 3 AFTER the bar is set up — incremental
        // search legitimately moves it to the first match while typing, so
        // parking beforehand would prove nothing.
        if let Some(ta) = editor.backend.as_textarea_mut() {
            ta.move_cursor(CursorMove::Jump(3, 1));
        }
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "done\nxx\ndone\nyy");
        let (row, _) = editor.cursor_pos();
        assert_eq!(
            row, 3,
            "replace all must not throw the cursor to the end of the note"
        );
    }

    #[test]
    fn an_empty_replacement_arms_before_it_deletes() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo and todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo ", "");

        let ctrl_a = InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        editor.handle_input(&ctrl_a, &tx);
        assert_eq!(
            editor.get_text(),
            "todo and todo",
            "the first Ctrl+A on an empty replacement must arm, not delete"
        );
        assert!(editor.search.as_ref().unwrap().armed_empty);

        editor.handle_input(&ctrl_a, &tx);
        assert_eq!(editor.get_text(), "and todo");
    }

    #[test]
    fn esc_disarms_an_empty_replace_all_without_closing_the_bar() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        let s = editor
            .search
            .as_ref()
            .expect("Esc disarms before it closes");
        assert!(!s.armed_empty);
        assert_eq!(editor.get_text(), "todo");
    }

    #[test]
    fn one_ctrl_z_undoes_a_whole_replace_all() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo and todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "done");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "done and done");

        // Close the bar so Ctrl+Z reaches the editor rather than the bar.
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(
            editor.get_text(),
            "todo and todo",
            "a replace is two history entries and must cost ONE undo — \
             popping half leaves the note with a hole in it"
        );
    }

    #[test]
    fn one_ctrl_z_undoes_a_single_replace_step() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo and todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "done");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "done and todo");
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "todo and todo");
    }

    #[test]
    fn redo_regroups_the_replace() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "done");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "todo");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('y'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(
            editor.get_text(),
            "done",
            "one redo must restore the whole replace"
        );
    }

    #[test]
    fn smartcase_drives_both_the_count_and_the_replace() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo Todo TODO".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "x");
        assert_eq!(
            editor.search.as_ref().unwrap().match_count,
            3,
            "an all-lowercase pattern matches any case"
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "x x x");
    }

    #[test]
    fn an_uppercase_pattern_is_case_sensitive() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo Todo TODO".to_string());
        open_replace_bar(&mut editor, &tx, "Todo", "x");
        assert_eq!(editor.search.as_ref().unwrap().match_count, 1);
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "todo x TODO");
    }

    #[test]
    fn the_preview_substitutes_lines_without_writing_them() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo and todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "done");
        let preview = editor.replace_preview().expect("a preview must be built");
        assert_eq!(preview.lines, vec!["done and done".to_string()]);
        assert_eq!(preview.spans.len(), 2);
        assert!(
            preview.spans.iter().any(|s| s.is_current),
            "the match under the cursor must be flagged so Enter's target is visible"
        );
        assert_eq!(
            editor.get_text(),
            "todo and todo",
            "building a preview must never mutate the buffer"
        );
    }

    /// The find bar owns the terminal caret while it is open, so the editor
    /// draws none — the flagged current span is the only thing on screen
    /// saying where in the note you are. It must survive an empty
    /// replacement, where the previewed match has zero width.
    #[test]
    fn a_deletion_preview_still_marks_the_current_match() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo and todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "");
        let preview = editor.replace_preview().expect("a preview must be built");
        assert_eq!(preview.lines, vec![" and ".to_string()]);
        let current = preview
            .spans
            .iter()
            .find(|s| s.is_current)
            .expect("the current match must stay flagged when it previews as nothing");
        assert_eq!(
            current.start, current.end,
            "an empty replacement previews as a zero-width span — the renderer \
             widens it to a caret cell so the marker cannot vanish"
        );
    }

    /// A mouse drag while the bar is open leaves a multi-row range in
    /// `self.selection` — `handle_mouse` has no find-bar guard. Reading the
    /// span from there dropped the end row and handed `replace_range` an
    /// inverted byte range, panicking the whole TUI.
    #[test]
    fn a_multi_row_selection_cannot_derail_an_interactive_replace() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("alpha beta\nxy".to_string());
        open_replace_bar(&mut editor, &tx, "beta", "Z");
        // Exactly what a drag from row 0 col 6 to row 1 col 1 leaves behind.
        editor.selection = Some(((0, 6), (1, 1)));
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "alpha Z\nxy");
    }

    /// `insert_str("")` deletes the selection and still returns `false`
    /// (`insert_piece` bails on the empty string), so trusting its bool left
    /// the buffer modified while the note read clean — never saved, and still
    /// rendering the pre-deletion text.
    #[test]
    fn deleting_a_match_marks_the_note_dirty() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo and todo".to_string());
        editor.mark_saved("todo and todo".to_string());
        assert!(!editor.is_dirty());

        open_replace_bar(&mut editor, &tx, "todo ", "");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "and todo");
        assert!(
            editor.is_dirty(),
            "a deletion is an edit — if the revision does not move, autosave \
             never writes it and the change is silently lost"
        );
    }

    /// The same trap on the bulk path, where the result is an empty buffer.
    #[test]
    fn emptying_the_note_via_replace_all_marks_it_dirty_and_is_undoable() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo".to_string());
        editor.mark_saved("todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "");
        let ctrl_a = InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL));
        editor.handle_input(&ctrl_a, &tx); // arms
        editor.handle_input(&ctrl_a, &tx); // commits
        assert_eq!(editor.get_text(), "");
        assert!(editor.is_dirty());

        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "todo");
    }

    /// Ctrl+Z must work from inside the bar. The bar consumes every key, so
    /// without an explicit route the user is stranded on a note it just
    /// rewrote until they think to press Esc first.
    #[test]
    fn ctrl_z_works_without_closing_the_bar_first() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo and todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "done");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "done and done");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "todo and todo");
        assert!(editor.search.is_some(), "undo must not close the bar");
    }

    /// A zero-width match (`\b`, `x*`) makes the selection empty, so
    /// `delete_selection` pushes no history entry and the action is ONE entry,
    /// not two. Recording two made the next Ctrl+Z pop an unrelated edit.
    #[test]
    fn a_zero_width_match_does_not_over_claim_history_entries() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("ab".to_string());
        open_replace_bar(&mut editor, &tx, r"\b", "|");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "|ab");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(
            editor.get_text(),
            "ab",
            "one undo must land exactly on the pre-replace text, not past it"
        );
    }

    /// A note swap must not carry the previous note's find-bar state across.
    /// `armed_empty` surviving means one Ctrl+A deletes every match in a note
    /// the user never armed.
    #[test]
    fn a_note_swap_resets_the_find_bar_and_its_undo_groups() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert!(editor.search.as_ref().unwrap().armed_empty);

        editor.set_text("todo elsewhere".to_string());
        assert!(editor.search.is_none(), "the bar belonged to the old note");
        // The buffer's groups went with it: `set_text` replaces the textarea,
        // and `RopeBuffer::replace` drops states the new history cannot reach.
        assert!(
            !editor.backend.as_textarea_mut().unwrap().undo(),
            "the new note's history has nothing to undo"
        );
    }

    /// Find-match highlighting is built from logical coordinates, so it
    /// describes the same matches the count and the stepping do. The old
    /// post-pass matched against text reconstructed from drawn cells, where
    /// markdown sigils are already concealed — so a pattern targeting a sigil
    /// counted and stepped to matches it could never paint.
    #[test]
    fn concealed_markdown_still_highlights_what_it_counts() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("# Heading\n[[note]]".to_string());
        editor.open_or_advance_search();
        for c in r"\[\[".chars() {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        let state = editor.search.as_ref().unwrap();
        assert_eq!(state.match_count, 1, "the `[[` sigil is a real match");
        let spans = state
            .pattern
            .as_ref()
            .unwrap()
            .match_spans(editor.backend.as_textarea().unwrap().text().lines());
        assert_eq!(
            spans,
            vec![(1, 0, 2)],
            "and it must be reported as a paintable span, not silently dropped \
             because the rendered row conceals it"
        );
    }

    /// A bracketed paste used to land in the buffer behind the open bar,
    /// leaving the match count and the highlighted match describing text that
    /// no longer existed. It belongs in the focused field — that is the
    /// holder's own behaviour, which survives the claim refactor.
    #[test]
    fn paste_goes_into_the_focused_bar_field() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "");
        editor.paste_text("done", &tx);
        assert_eq!(editor.get_text(), "todo", "the buffer is untouched");
        assert_eq!(editor.search.as_ref().unwrap().replacement(), "done");
    }

    #[test]
    fn a_multiline_paste_collapses_to_its_first_line() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("x".to_string());
        editor.open_or_advance_search();
        editor.paste_text("first\nsecond", &tx);
        assert_eq!(editor.search.as_ref().unwrap().input.value(), "first");
    }

    /// With the pane exactly as tall as the bar, the old `>` comparison left
    /// the bar unrendered while it was still open and still consuming keys —
    /// an invisible modal.
    #[test]
    fn the_bar_is_never_an_invisible_modal() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut editor = make_editor();
        editor.set_text("todo".to_string());
        let theme = Theme::default();
        let mut term = Terminal::new(TestBackend::new(40, 1)).unwrap();
        let area = Rect::new(0, 0, 40, 1);
        editor.open_or_advance_search();
        term.draw(|f| editor.render(f, area, &theme, true)).unwrap();
        let row: String = (0..40)
            .filter_map(|x| {
                term.backend()
                    .buffer()
                    .cell(ratatui::layout::Position::new(x, 0))
                    .map(|c| c.symbol().to_string())
            })
            .collect();
        assert!(
            row.contains("Find:"),
            "an open bar must be drawn even when it costs the whole pane, got {row:?}"
        );
    }

    /// End-to-end: after a replace all, a rewritten row far from the cursor
    /// must render from a fresh parse, not the pre-replace one. The construct
    /// has to be one the renderer *conceals* (a wikilink), because a stale
    /// parse is only visible where parsing changes what is drawn.
    ///
    /// Honest caveat: this passes with `note_bulk_edit` removed, because the
    /// widener cap-trips to a full parse on a damage range this far from a
    /// reset boundary. It guards the user-visible outcome, not the mechanism —
    /// the mechanism is pinned by
    /// `the_cursor_hint_under_reports_a_two_place_edit` in
    /// `parse_incremental`, which does discriminate.
    #[test]
    fn a_row_far_from_the_cursor_reparses_after_replace_all() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut editor = make_editor();
        let tx = dummy_tx();
        let mut lines: Vec<String> = (0..400).map(|i| format!("filler {i}")).collect();
        lines[0] = "todo".to_string();
        lines[398] = "todo".to_string();
        editor.set_text(lines.join("\n"));
        let theme = Theme::default();
        let mut term = Terminal::new(TestBackend::new(20, 8)).unwrap();
        let area = Rect::new(0, 0, 20, 8);
        term.draw(|f| editor.render(f, area, &theme, true)).unwrap();

        open_replace_bar(&mut editor, &tx, "todo", "[[x]]");
        // Cursor on the LAST match, so the damage hint points 398 rows away
        // from the first one.
        if let Some(ta) = editor.backend.as_textarea_mut() {
            ta.move_cursor(CursorMove::Jump(398, 0));
        }
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        // Back to the top, cursor OFF row 0 — a cursor inside the link would
        // reveal it legitimately and prove nothing.
        if let Some(ta) = editor.backend.as_textarea_mut() {
            ta.move_cursor(CursorMove::Jump(1, 0));
        }
        term.draw(|f| editor.render(f, area, &theme, true)).unwrap();
        let row0: String = (0..20)
            .filter_map(|x| {
                term.backend()
                    .buffer()
                    .cell(ratatui::layout::Position::new(x, 0))
                    .map(|c| c.symbol().to_string())
            })
            .collect::<String>()
            .trim_end()
            .to_string();
        assert_eq!(
            row0, "x",
            "row 0 must render as a parsed wikilink; `[[x]]` would mean it \
             kept the parse of the text that was there before the replace"
        );
    }

    /// Indenting N lines is 2N history entries, so before the **edit buffer**
    /// grouped it, one Ctrl+Z un-indented only the last line and the user had
    /// to press it N times. Same class as `guu`, and fixed by the same move.
    #[test]
    fn indenting_a_block_undoes_in_one_step() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("a\nb\nc".to_string());
        get_ta(&mut editor).move_cursor(CursorMove::Jump(0, 0));
        get_ta(&mut editor).start_selection();
        get_ta(&mut editor).move_cursor(CursorMove::Jump(2, 1));
        editor.indent_lines(false);
        assert_eq!(editor.get_text(), "    a\n    b\n    c");

        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(
            editor.get_text(),
            "a\nb\nc",
            "one undo must revert the whole block, not just the last line"
        );
    }

    /// A paste over a selection is a cut plus an insert — one action.
    #[test]
    fn pasting_over_a_selection_undoes_in_one_step() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("hello world".to_string());
        get_ta(&mut editor).move_cursor(CursorMove::Jump(0, 0));
        get_ta(&mut editor).start_selection();
        get_ta(&mut editor).move_cursor(CursorMove::Jump(0, 5));
        editor.paste_text("goodbye", &tx);
        assert_eq!(editor.get_text(), "goodbye world");

        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "hello world");
    }

    /// Closing the bar must clear the editor's selection, as `close_search`
    /// did before the bar became a module. A stale mouse-drag range otherwise
    /// suppresses the right-click context menu, which reads `self.selection`.
    #[test]
    fn closing_the_bar_clears_a_stale_selection() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("alpha beta".to_string());
        editor.selection = Some(((0, 0), (0, 5)));
        editor.open_or_advance_search();
        for c in "beta".chars() {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        assert!(editor.search.is_none());
        assert_eq!(
            editor.selection, None,
            "a selection from before the search must not outlive the bar"
        );
    }

    /// An undo inside the bar changes the text the **current match** pointed
    /// at, so the highlight must be re-derived rather than left over it.
    #[test]
    fn undo_inside_the_bar_rederives_the_current_match() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("foo foo".to_string());
        open_replace_bar(&mut editor, &tx, "foo", "xy");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "xy foo");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "foo foo");
        let current = editor.search.as_ref().unwrap().current_match();
        if let Some(((row, start), (_, end))) = current {
            let line = &editor.get_text()[..];
            let text: String = line
                .lines()
                .nth(row)
                .unwrap()
                .chars()
                .skip(start)
                .take(end - start)
                .collect();
            assert_eq!(
                text, "foo",
                "the highlight must sit on a real match, got {text:?}"
            );
        }
    }

    /// vim `n` repeats the search with the bar closed, and must paint what it
    /// landed on — the highlight moved onto the bar when the module was
    /// extracted, and the closed-bar path lost it.
    #[test]
    fn vim_n_highlights_the_match_it_lands_on() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("lo xx lo".to_string());
        editor.open_or_advance_search();
        for c in "lo".chars() {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('n'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(
            editor.selection,
            Some(((0, 6), (0, 8))),
            "`n` must paint the match it jumped to"
        );
    }

    /// Closing the bar must clear the buffer's selection ANCHOR, not just the
    /// mirrored range. `search_forward` moves the cursor without touching
    /// `selection_start`, so a selection made before the search stays live but
    /// unpainted — and the next keystroke silently deletes it.
    #[test]
    fn closing_the_bar_cannot_leave_an_invisible_selection() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("foo bar baz".to_string());
        // Select the whole note, as Ctrl+A does.
        get_ta(&mut editor).select_all();
        editor.open_or_advance_search();
        for c in "bar".chars() {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
            &tx,
        );
        assert!(
            editor.get_text().contains("foo"),
            "typing after the bar closed must not eat unhighlighted text, got {:?}",
            editor.get_text()
        );
    }

    /// vim `>` over a selection pushes one history entry per row, so it took N
    /// undos. One vim command is one undo.
    #[test]
    fn vim_visual_indent_undoes_in_one_step() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("a\nb\nc".to_string());
        for c in ['V', 'j', 'j', '>'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        let indented = editor.get_text();
        assert_ne!(indented, "a\nb\nc", "`>` must indent the selection");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('u'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(
            editor.get_text(),
            "a\nb\nc",
            "one `u` must revert the whole indent, not one row"
        );
    }

    /// End-to-end for the anchor invariant: `n` moved the cursor while a
    /// selection anchor was live, turning it into an unpainted selection that
    /// the next keystroke deleted. `"foo bar foo"` became `"Xfoo"`.
    #[test]
    fn vim_n_cannot_leave_an_invisible_selection() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("foo bar foo".to_string());
        editor.open_or_advance_search();
        for c in "foo".chars() {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        // A selection made after the bar closed, then `n`.
        get_ta(&mut editor).move_cursor(CursorMove::Jump(0, 0));
        get_ta(&mut editor).start_selection();
        get_ta(&mut editor).move_cursor(CursorMove::Jump(0, 3));
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('n'), KeyModifiers::NONE)),
            &tx,
        );
        for c in ['i', 'X'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        assert!(
            editor.get_text().contains("bar"),
            "typing after `n` must not eat unhighlighted text, got {:?}",
            editor.get_text()
        );
    }

    /// End-to-end for the overlay move: task and needle decoration used to be
    /// painted from drawn cells and is now mapped from logical columns. The
    /// rendered result must be the same, which is the whole point — a list
    /// bullet is rendered, so the two coordinate spaces do not coincide.
    #[test]
    fn overlays_paint_where_the_post_pass_used_to() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::layout::Position;
        let mut editor = make_editor();
        editor.set_text("find the needle here\n- [x] done task\n- [ ] open task".to_string());
        editor.set_search_needles(vec!["needle".to_string()]);
        let theme = Theme::default();
        let mut term = Terminal::new(TestBackend::new(40, 6)).unwrap();
        let area = Rect::new(0, 0, 40, 6);
        term.draw(|f| editor.render(f, area, &theme, false))
            .unwrap();
        let buf = term.backend().buffer();

        let row: String = (0..40)
            .filter_map(|x| {
                buf.cell(Position::new(x, 0))
                    .map(|c| c.symbol().to_string())
            })
            .collect();
        let at = row.find("needle").expect("needle is on screen");
        let cell = buf.cell(Position::new(at as u16, 0)).unwrap();
        assert_eq!(
            cell.fg,
            theme.color_search_match.to_ratatui(),
            "the needle must still be emphasised"
        );

        // The done task's text is struck; the open one's is not.
        let struck = |y: u16| {
            (0..40).any(|x| {
                buf.cell(Position::new(x, y)).is_some_and(|c| {
                    c.style()
                        .add_modifier
                        .contains(ratatui::style::Modifier::CROSSED_OUT)
                })
            })
        };
        assert!(struck(1), "a done task strikes its text");
        assert!(!struck(2), "an open task does not");

        // And the checkbox itself carries the accent colour on both rows.
        for y in [1u16, 2] {
            assert!(
                (0..40).any(|x| buf
                    .cell(Position::new(x, y))
                    .is_some_and(|c| c.fg == theme.accent.to_ratatui())),
                "row {y} must have an accent-coloured checkbox"
            );
        }
    }

    #[test]
    fn no_preview_without_a_replace_field() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo".to_string());
        editor.open_or_advance_search();
        for c in "todo".chars() {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        assert!(
            editor.replace_preview().is_none(),
            "a find-only bar previews nothing"
        );
    }

    #[test]
    fn capture_expansion_is_gated_on_the_pattern_capturing() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        // No capture group: `$1` is literal text, not an empty expansion.
        editor.set_text("cost".to_string());
        open_replace_bar(&mut editor, &tx, "cost", "$1");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "$1");
    }

    #[test]
    fn the_bar_reserves_two_rows_only_while_replacing() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("todo".to_string());
        let theme = Theme::default();
        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let area = Rect::new(0, 0, 40, 10);

        editor.open_or_advance_search();
        term.draw(|f| editor.render(f, area, &theme, true)).unwrap();
        assert_eq!(editor.rect.height, 9, "a find-only bar takes one row");

        editor.handle_input(&InputEvent::Key(key(KeyCode::Tab, KeyModifiers::NONE)), &tx);
        term.draw(|f| editor.render(f, area, &theme, true)).unwrap();
        assert_eq!(
            editor.rect.height, 8,
            "the replace field takes a second row"
        );
    }

    /// `guu` is a cut plus an insert, so it always landed in history as two
    /// entries and took two `u` presses to revert — `guu_undoes_in_one_step`
    /// in vim.rs documents that with a comment rather than fixing it. Now that
    /// grouping exists, the case operators use it and the name is true.
    #[test]
    fn guu_really_does_undo_in_one_step() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("Mixed Case Line".to_string());
        for c in "guu".chars() {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        assert_eq!(editor.get_text(), "mixed case line");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('u'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "Mixed Case Line");
    }

    /// Vim's `u` must also take a whole **undo group**. The engine performs the
    /// undo inside its own command apply, so the host has to peek before
    /// dispatch and finish the group afterwards — a path the Ctrl+Z tests
    /// above do not touch.
    #[test]
    fn vim_u_undoes_a_whole_replace() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("todo and todo".to_string());
        open_replace_bar(&mut editor, &tx, "todo", "done");
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "done and done");

        // Esc closes the bar, returning keys to the vim engine in Normal mode.
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('u'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "todo and todo");
    }

    // ── Undo grouping ────────────────────────────────────────────────────────

    fn type_out(editor: &mut TextEditorComponent, tx: &AppTx, text: &str) {
        use ratatui::crossterm::event::KeyEvent;
        for c in text.chars() {
            let code = if c == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(c)
            };
            editor.handle_textarea_key(&KeyEvent::new(code, KeyModifiers::NONE), tx);
        }
    }

    #[test]
    fn undo_takes_back_a_word_not_a_letter() {
        // The incumbent recorded one history entry per character, so leaving a
        // sentence took as many presses as it had letters.
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text(String::new());
        type_out(&mut editor, &tx, "hello world");
        assert_eq!(editor.get_text(), "hello world");

        assert!(get_ta(&mut editor).undo());
        assert_eq!(editor.get_text(), "hello ", "the last word goes whole");
        assert!(get_ta(&mut editor).undo());
        assert_eq!(editor.get_text(), "", "and so does the first");
    }

    #[test]
    fn a_cursor_move_separates_two_runs() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text(String::new());
        type_out(&mut editor, &tx, "ab");
        arrow(&mut editor, &tx, KeyCode::Home);
        type_out(&mut editor, &tx, "cd");
        assert_eq!(editor.get_text(), "cdab");

        assert!(get_ta(&mut editor).undo());
        assert_eq!(
            editor.get_text(),
            "ab",
            "only what was typed after the move comes back off"
        );
    }

    #[test]
    fn backspacing_to_fix_a_typo_is_its_own_action() {
        use ratatui::crossterm::event::KeyEvent;
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text(String::new());
        type_out(&mut editor, &tx, "helllo");
        editor.handle_textarea_key(&KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), &tx);
        assert_eq!(editor.get_text(), "helll");

        assert!(get_ta(&mut editor).undo());
        assert_eq!(
            editor.get_text(),
            "helllo",
            "the delete undoes on its own, without taking the typing with it"
        );
    }

    #[test]
    fn an_undo_between_two_runs_separates_them() {
        // Ctrl+Z is claimed before the plain key table, so the run has to be ended
        // where every key passes rather than where typing is applied.
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text(String::new());
        type_out(&mut editor, &tx, "ab");
        assert!(get_ta(&mut editor).undo());
        assert_eq!(editor.get_text(), "");
        type_out(&mut editor, &tx, "cd");
        assert_eq!(editor.get_text(), "cd");
        assert!(get_ta(&mut editor).undo());
        assert_eq!(
            editor.get_text(),
            "",
            "the second run is its own group, not an extension of an undone one"
        );
    }

    #[test]
    fn a_save_closes_the_open_group() {
        // CONTEXT.md: "a group never spans a save, and one undo after saving
        // lands on exactly what is on disk". The save arrives by a path that is
        // not a keystroke, so nothing on the key path could have closed it.
        let mut editor = make_editor();
        let tx = dummy_tx();
        type_out(&mut editor, &tx, "abc");
        let saved = editor.get_text();
        editor.mark_saved(saved);
        // Immediately, well inside the idle window, and mid-"word" so the
        // boundary rule cannot close the run either.
        type_out(&mut editor, &tx, "def");

        assert!(get_ta(&mut editor).undo());
        assert_eq!(
            editor.get_text(),
            "abc",
            "one undo lands on what was saved, not before it"
        );
    }

    #[test]
    fn a_stale_save_completion_does_not_close_the_group() {
        // The other half of the same rule: `mark_saved_at_revision` is a
        // documented no-op when the revision moved on, and an action that did
        // nothing must not split the user's word.
        let mut editor = make_editor();
        let tx = dummy_tx();
        type_out(&mut editor, &tx, "abc");
        let stale = NonZeroU64::new(1).expect("nonzero");
        editor.mark_saved_at_revision(stale);
        type_out(&mut editor, &tx, "def");

        assert!(get_ta(&mut editor).undo());
        assert_eq!(
            editor.get_text(),
            "",
            "the run carried on across a completion that marked nothing"
        );
    }

    #[test]
    fn a_second_vim_insert_session_is_its_own_group() {
        // The session flag was refreshed only on the pass-through path, which
        // `Esc` never takes, so it latched true on the first `i` and every later
        // session folded into whatever entry preceded it.
        use ratatui::crossterm::event::KeyEvent;
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text(String::new());
        let press = |editor: &mut TextEditorComponent, code| {
            let _ = editor.handle_input(
                &InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
                &tx,
            );
        };
        press(&mut editor, KeyCode::Char('i'));
        for c in "one".chars() {
            press(&mut editor, KeyCode::Char(c));
        }
        press(&mut editor, KeyCode::Esc);
        press(&mut editor, KeyCode::Char('i'));
        for c in "two".chars() {
            press(&mut editor, KeyCode::Char(c));
        }
        press(&mut editor, KeyCode::Esc);
        // `Esc` steps the cursor left, so the second `i` inserts before the
        // final `e` — the position is incidental, the grouping is the point.
        assert_eq!(editor.get_text(), "ontwoe");

        assert!(get_ta(&mut editor).undo());
        assert_eq!(
            editor.get_text(),
            "one",
            "`u` takes back the second session only"
        );
    }

    #[test]
    fn a_vim_insert_session_undoes_whole() {
        use ratatui::crossterm::event::KeyEvent;
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text(String::new());
        // `i` enters Insert; the text then flows through the same plain key path.
        let press = |editor: &mut TextEditorComponent, code| {
            let _ = editor.handle_input(
                &InputEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
                &tx,
            );
        };
        press(&mut editor, KeyCode::Char('i'));
        for c in "hello world".chars() {
            press(&mut editor, KeyCode::Char(c));
        }
        press(&mut editor, KeyCode::Esc);
        assert_eq!(editor.get_text(), "hello world");

        assert!(get_ta(&mut editor).undo());
        assert_eq!(
            editor.get_text(),
            "",
            "vim's `u` takes back the whole session, word boundaries included"
        );
    }

    // ── Arrow keys move by drawn line ────────────────────────────────────────

    /// Render once so the view has a layout for the width under test.
    fn lay_out(editor: &mut TextEditorComponent, width: u16, height: u16) {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let theme = Theme::default();
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        let area = Rect::new(0, 0, width, height);
        term.draw(|f| editor.render(f, area, &theme, true)).unwrap();
    }

    fn arrow(editor: &mut TextEditorComponent, tx: &AppTx, code: KeyCode) {
        use ratatui::crossterm::event::KeyEvent;
        editor.handle_textarea_key(&KeyEvent::new(code, KeyModifiers::NONE), tx);
    }

    #[test]
    fn down_moves_one_drawn_line_not_one_row() {
        // The whole point of owning both the cursor and the layout. A paragraph
        // that wraps into four drawn lines takes four presses to leave, not one.
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text(
            "aaaa bbbb cccc dddd
second row"
                .to_string(),
        );
        lay_out(&mut editor, 6, 10);

        get_ta(&mut editor).jump_to(0, 0);
        arrow(&mut editor, &tx, KeyCode::Down);
        assert_eq!(
            get_ta(&mut editor).cursor(),
            (0, 5),
            "still inside the first row, on its second drawn line"
        );
        arrow(&mut editor, &tx, KeyCode::Down);
        assert_eq!(get_ta(&mut editor).cursor(), (0, 10));
        arrow(&mut editor, &tx, KeyCode::Down);
        assert_eq!(get_ta(&mut editor).cursor(), (0, 15));
        arrow(&mut editor, &tx, KeyCode::Down);
        assert_eq!(
            get_ta(&mut editor).cursor().0,
            1,
            "and only the fourth press reaches the next row"
        );
    }

    #[test]
    fn up_and_down_are_symmetric_across_a_wrap() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("aaaa bbbb cccc".to_string());
        lay_out(&mut editor, 6, 10);

        get_ta(&mut editor).jump_to(0, 0);
        arrow(&mut editor, &tx, KeyCode::Down);
        let middle = get_ta(&mut editor).cursor();
        arrow(&mut editor, &tx, KeyCode::Up);
        assert_eq!(get_ta(&mut editor).cursor(), (0, 0));
        assert_eq!(middle, (0, 5));
    }

    #[test]
    fn an_arrow_against_a_stale_layout_falls_back_instead_of_panicking() {
        // `main.rs` drains queued input without redrawing between events, so an
        // edit and an arrow can be processed in one batch. Shrinking a row does
        // not change the row COUNT, which is all the old guard compared — and the
        // layout's byte ranges then sliced past the end of the shortened row.
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("abcd\nefgh".to_string());
        lay_out(&mut editor, 20, 10);

        get_ta(&mut editor).jump_to(0, 4);
        for _ in 0..3 {
            get_ta(&mut editor).delete_char();
        }
        assert_eq!(get_ta(&mut editor).rows(), &["a", "efgh"]);

        // The move falls back to a logical one rather than reading the layout.
        arrow(&mut editor, &tx, KeyCode::Down);
        assert_eq!(get_ta(&mut editor).cursor().0, 1, "still moved down a row");
    }

    #[test]
    fn an_action_between_arrows_forgets_the_goal_cell() {
        // The other side of `a_run_of_arrows_keeps_its_goal_cell`: the column is
        // borrowed for a run of arrows and for nothing else, so anything that is
        // not one forgets it. Driven here through a save, because that is a path
        // with no keystroke on it at all — the same choke point serves the click,
        // the find and the vim motion.
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text(
            "aaaaaaaa
bb
cccccccc"
                .to_string(),
        );
        lay_out(&mut editor, 20, 10);

        get_ta(&mut editor).jump_to(0, 7);
        arrow(&mut editor, &tx, KeyCode::Down);
        assert_eq!(
            get_ta(&mut editor).cursor(),
            (1, 2),
            "clamped to the short row"
        );

        let saved = editor.get_text();
        editor.mark_saved(saved);

        arrow(&mut editor, &tx, KeyCode::Down);
        assert_eq!(
            get_ta(&mut editor).cursor(),
            (2, 2),
            "the goal was forgotten, so the third row keeps the clamped column"
        );
    }

    #[test]
    fn a_run_of_arrows_keeps_its_goal_cell() {
        // Passing through a shorter drawn line clamps, but does not forget: the
        // column is borrowed for one line rather than lost.
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text(
            "aaaaaaaa
bb
cccccccc"
                .to_string(),
        );
        lay_out(&mut editor, 20, 10);

        get_ta(&mut editor).jump_to(0, 7);
        arrow(&mut editor, &tx, KeyCode::Down);
        assert_eq!(
            get_ta(&mut editor).cursor(),
            (1, 2),
            "clamped to the short row"
        );
        arrow(&mut editor, &tx, KeyCode::Down);
        assert_eq!(
            get_ta(&mut editor).cursor(),
            (2, 7),
            "and back out to the cell the run still wants"
        );
    }

    #[test]
    fn another_key_ends_the_run() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text(
            "aaaaaaaa
bb
cccccccc"
                .to_string(),
        );
        lay_out(&mut editor, 20, 10);

        get_ta(&mut editor).jump_to(0, 7);
        arrow(&mut editor, &tx, KeyCode::Down);
        arrow(&mut editor, &tx, KeyCode::Home);
        arrow(&mut editor, &tx, KeyCode::Down);
        assert_eq!(
            get_ta(&mut editor).cursor(),
            (2, 0),
            "Home set a new goal; the old one is gone"
        );
    }

    #[test]
    fn shift_down_extends_by_a_drawn_line() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("aaaa bbbb cccc".to_string());
        lay_out(&mut editor, 6, 10);

        get_ta(&mut editor).jump_to(0, 0);
        editor.handle_textarea_key(
            &ratatui::crossterm::event::KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
            &tx,
        );
        assert_eq!(
            get_ta(&mut editor).selection_range(),
            Some(((0, 0), (0, 5)))
        );
    }

    /// Helper: construct a vim-backend editor.
    fn make_vim_editor() -> TextEditorComponent {
        let mut settings = crate::settings::AppSettings::default();
        settings.editor_backend = crate::settings::EditorBackendSetting::Vim;
        TextEditorComponent::new(KeyBindings::empty(), &settings)
    }

    /// Helper: extract the current vim EditorMode, panicking if the backend
    /// is not a vim textarea (so test failures are obvious).
    fn vim_mode(editor: &TextEditorComponent) -> EditorMode {
        match &editor.backend {
            BackendState::Textarea(tb) => match &tb.input {
                backend::InputInterpreter::Vim(e) => e.mode().clone(),
                _ => panic!("expected Vim input interpreter"),
            },
            _ => panic!("expected Textarea backend"),
        }
    }

    /// Regression: pasting a URL over a vim charwise Visual selection made with
    /// `ve` (cursor lands ON the last char) must wrap the WHOLE word as a
    /// markdown link. ratatui's `selection_range()` is half-open and stops
    /// before the char under the cursor, so without the inclusive extension in
    /// `paste_text` the last letter was left dangling (`[hell](url)o`).
    #[test]
    fn vim_visual_paste_url_wraps_whole_selected_word() {
        let mut editor = make_vim_editor();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        editor.set_text("hello world".to_string());
        // `v` enters charwise Visual at col 0, `e` extends to the end of the
        // word — cursor ends ON the 'o' of "hello".
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('v'), KeyModifiers::NONE)),
            &tx,
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('e'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(vim_mode(&editor), EditorMode::Visual);
        editor.paste_text("https://example.com", &tx);
        assert_eq!(
            editor.get_text(),
            "[hello](https://example.com) world",
            "the whole selected word (including the char under the cursor) must be wrapped"
        );
    }

    /// Regression: applying Bold over a vim charwise Visual selection made with
    /// `ve` must wrap the WHOLE word. The formatting action is dispatched at the
    /// app-screen keybinding layer (before the vim engine), so it reads the
    /// half-open textarea selection directly — without the inclusive extension
    /// the last letter was left outside the markers (`**hell**o`).
    #[test]
    fn vim_visual_bold_wraps_whole_selected_word() {
        let mut editor = make_vim_editor();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        editor.set_text("hello world".to_string());
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('v'), KeyModifiers::NONE)),
            &tx,
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('e'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(vim_mode(&editor), EditorMode::Visual);
        editor.apply_text_action(TextAction::Bold);
        assert_eq!(
            editor.get_text(),
            "**hello** world",
            "the whole selected word (including the char under the cursor) must be wrapped"
        );
    }

    /// Regression: copy is read-only over a vim charwise Visual selection.
    /// Every clipboard action reports its outcome, paste included. Before this,
    /// Ctrl+V was the only one that said nothing, so the footer was left showing
    /// the raw chord echo — indistinguishable from an unbound key.
    ///
    /// Headless CI has no clipboard, so accept either the success message or a
    /// clipboard error; what must never happen is silence.
    #[test]
    fn paste_reports_its_outcome() {
        let mut editor = make_editor();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        editor.set_text("x".to_string());
        editor.paste_from_clipboard(&tx);
        let reported = std::iter::from_fn(|| rx.try_recv().ok()).any(|e| {
            matches!(e, AppEvent::FlashMessage(m)
                if m == "pasted" || m == "clipboard is empty" || m.starts_with("clipboard: "))
        });
        assert!(reported, "a paste attempt must always report something");
    }

    /// The image-paste path bypasses the editor's key handling entirely (the
    /// screen layer owns it, because only it can reach the vault), so it has to
    /// reconcile the engine itself. Before this, an image pasted in Visual mode
    /// left the engine in Visual with a selection that no longer existed —
    /// every subsequent motion silently extended a ghost.
    #[test]
    fn external_paste_drops_the_selection_and_leaves_visual() {
        let mut editor = make_vim_editor();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        editor.set_text("hello world".to_string());
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('v'), KeyModifiers::NONE)),
            &tx,
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('e'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(vim_mode(&editor), EditorMode::Visual);

        editor.take_selection_for_external_paste();

        assert_eq!(
            vim_mode(&editor),
            EditorMode::Normal,
            "the engine must not keep believing it is in Visual"
        );
        assert_eq!(
            get_ta(&mut editor).selection_range(),
            None,
            "the selection the incoming content replaces must be gone"
        );
        assert_eq!(
            editor.get_text(),
            " world",
            "the inclusive visual range is what gets replaced"
        );
    }

    /// The same call outside Visual must not eat anything — Ctrl+V with an
    /// image on the clipboard is an ordinary insert-at-cursor in Normal mode.
    #[test]
    fn external_paste_without_a_selection_leaves_the_buffer_alone() {
        let mut editor = make_vim_editor();
        editor.set_text("hello world".to_string());
        editor.take_selection_for_external_paste();
        assert_eq!(editor.get_text(), "hello world");
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
    }

    /// It must include the char under the cursor (matching the highlight), but
    /// must NOT mutate the live selection — otherwise repeated right-click copy
    /// drifts the selection one char wider each time (`((0,0),(0,4))` →
    /// `(0,5)` → `(0,6)` …).
    #[test]
    fn vim_visual_copy_is_read_only_and_does_not_grow_selection() {
        let mut editor = make_vim_editor();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        editor.set_text("hello world".to_string());
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('v'), KeyModifiers::NONE)),
            &tx,
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('e'), KeyModifiers::NONE)),
            &tx,
        );
        let before = get_ta(&mut editor).selection_range();
        assert_eq!(before, Some(((0, 0), (0, 4))));
        // The text copied must cover the inclusive range "hello".
        assert_eq!(
            editor.inclusive_visual_range(),
            Some(((0, 0), (0, 5))),
            "copy must read the inclusive range including the cursor char"
        );
        // Repeated copy must leave the live selection untouched.
        editor.copy_selection_to_clipboard(&tx);
        editor.copy_selection_to_clipboard(&tx);
        assert_eq!(
            get_ta(&mut editor).selection_range(),
            before,
            "copy must not move the cursor or grow the live selection"
        );
    }

    /// Regression: `V` (linewise Visual) must keep the WHOLE line highlighted
    /// no matter where the cursor moves within it afterwards — column position
    /// is irrelevant to a linewise selection in vim. `self.selection` used to
    /// mirror the textarea's raw (charwise) selection range verbatim, which
    /// only looked like a full line right after `V` because it happens to run
    /// Head..End; moving the cursor back then shrank the highlight to
    /// "start of line .. cursor".
    #[test]
    fn vim_visual_line_selection_stays_full_width_after_cursor_moves_back() {
        let mut editor = make_vim_editor();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        editor.set_text("hello world".to_string());
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('V'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(vim_mode(&editor), EditorMode::VisualLine);
        assert_eq!(
            editor.selection,
            Some(((0, 0), (0, usize::MAX))),
            "V must highlight the full line right away"
        );
        for _ in 0..5 {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Left, KeyModifiers::NONE)),
                &tx,
            );
        }
        assert_eq!(
            editor.selection,
            Some(((0, 0), (0, usize::MAX))),
            "moving the cursor back must not shrink the linewise highlight"
        );
    }

    /// Regression: a bare left click (Down with no Drag) must NOT flip
    /// vim Normal → Visual.  The textarea's Down arm calls `start_selection()`
    /// which leaves a collapsed (start==end) selection; the fix at ~line 2124
    /// uses `.is_some_and(|(s, e)| s != e)` to require a non-empty selection
    /// before treating it as "real" (mirrors the same guard at ~line 1014).
    ///
    /// We test `sync_mouse_selection` directly (the exact code that was
    /// broken) rather than routing through `handle_input` → `handle_mouse`,
    /// which needs a fully rendered view to resolve screen→logical coordinates.
    #[test]
    fn vim_sync_collapsed_sel_stays_normal() {
        let mut editor = make_vim_editor();
        editor.set_text("hello world".to_string());

        // Sanity: starts in Normal.
        assert_eq!(vim_mode(&editor), EditorMode::Normal);

        // A bare click leaves has_sel == false (collapsed selection filtered
        // out by the is_some_and guard).  Sync with no selection must keep Normal.
        editor.backend.sync_mouse_selection(false);
        assert_eq!(
            vim_mode(&editor),
            EditorMode::Normal,
            "collapsed (bare click) selection must not enter Visual mode"
        );
    }

    /// A drag that creates a real (non-empty) selection DOES enter Visual mode.
    #[test]
    fn vim_sync_real_sel_enters_visual() {
        let mut editor = make_vim_editor();
        editor.set_text("hello world".to_string());

        // Sanity: starts in Normal.
        assert_eq!(vim_mode(&editor), EditorMode::Normal);

        // A drag with start != end yields has_sel == true.
        editor.backend.sync_mouse_selection(true);
        assert_eq!(
            vim_mode(&editor),
            EditorMode::Visual,
            "real drag selection must enter Visual mode"
        );
    }

    /// Regression: with the find bar open in vim Normal mode, typed keys must
    /// go into the find query, NOT be processed by the vim engine (which would
    /// treat 'l'/'o' as motions and move the cursor).
    #[test]
    fn vim_find_bar_captures_typing_not_cursor() {
        let mut editor = make_vim_editor();
        editor.set_text("hello world\nsecond line".to_string());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Open the find bar (same path as the '/' key: OpenSearch → open_or_advance_search).
        editor.open_or_advance_search();
        assert!(editor.search.is_some(), "find bar must be open");

        // Type "lo" — should go into the find query, not be processed as vim motions.
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('l'), KeyModifiers::NONE)),
            &tx,
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('o'), KeyModifiers::NONE)),
            &tx,
        );

        // Find query must capture "lo". This proves keys went to the find bar
        // and not the vim engine (which would treat 'l' as a rightward motion
        // and 'o' as Open-line-below, mutating the buffer).
        let q = editor
            .search
            .as_ref()
            .map(|s| s.input.value().to_string())
            .unwrap_or_default();
        assert_eq!(q, "lo", "find query must capture typed characters");

        // Buffer must be unchanged — 'o' in vim Normal mode inserts a new line,
        // so a mutated buffer means the key escaped to the vim engine.
        assert_eq!(
            editor.get_text(),
            "hello world\nsecond line",
            "buffer must not be modified while find bar is open"
        );

        // The cursor is allowed to move to the first search match (that is
        // correct search behaviour — refresh_search_pattern jumps to the hit).
        // What must NOT happen is a vim motion: 'l' in Normal mode would leave
        // the cursor at col 1 with no query update; here it must be at the
        // "lo" match col instead (3 — the second 'l' in "hello").
        assert_eq!(
            editor.cursor_pos().1,
            3,
            "cursor must jump to the search match (col 3), not to a vim motion position"
        );
    }

    /// Vim `/pattern`: Enter steps to the next match (same as the textarea
    /// backend — one key map on both), `Esc` closes the bar, and
    /// `n` / `N` keep working afterwards because closing no longer wipes the
    /// pattern.
    #[test]
    fn vim_search_enter_steps_and_esc_keeps_the_pattern_for_n() {
        let mut editor = make_vim_editor();
        // Three "lo" at cols 0, 6, 12 on a single line.
        editor.set_text("lo xx lo yy lo".to_string());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Open the find bar (same path as the '/' key: OpenSearch → open_or_advance_search).
        editor.open_or_advance_search();
        assert!(editor.search.is_some(), "find bar must open");

        // Type "lo" — keys go into the find query (incremental search).
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('l'), KeyModifiers::NONE)),
            &tx,
        );
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('o'), KeyModifiers::NONE)),
            &tx,
        );

        // Enter steps to the next match and the bar STAYS OPEN — incremental
        // search parked the cursor on the first "lo" (col 0), so this lands on
        // the second (col 6).
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        assert!(
            editor.search.is_some(),
            "find bar stays open on Enter — it steps, it does not confirm"
        );
        let (_, c1) = editor.cursor_pos();
        assert_eq!(c1, 6, "Enter must step to the 2nd 'lo' at col 6");

        // Esc closes the bar. It must NOT wipe the pattern: that was the only
        // difference between closing with Esc and closing with Enter, and it
        // silently killed n/N.
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        assert!(editor.search.is_none(), "Esc must close the find bar");

        // 'n' must navigate, not type into the (now-closed) bar.
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('n'), KeyModifiers::NONE)),
            &tx,
        );
        let (_, c2) = editor.cursor_pos();
        assert_eq!(c2, 12, "'n' must jump to the 3rd 'lo' at col 12");

        // The buffer must never have been modified.
        assert_eq!(editor.get_text(), "lo xx lo yy lo");
    }
}
