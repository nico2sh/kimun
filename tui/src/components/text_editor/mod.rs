pub mod autocomplete_glue;
pub mod backend;
pub mod find_bar;
pub mod markdown;
mod markdown_style;
pub mod nvim_decode;
pub mod nvim_host;
pub mod nvim_rpc;
use crate::keys::crossterm_input::pointer_input;
use crate::logic::editor::revisions::Revisions;
#[cfg(test)]
use crate::logic::editor::rope_buffer::CursorMove;
#[cfg(test)]
use crate::logic::input::KeyInput;
use crate::logic::input::{Button, PointerInput, PointerKind};
pub mod view;

use ratatui::Frame;
use ratatui::crossterm::event::{MouseButton, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use std::num::NonZeroU64;

/// Build an `EditorSnapshot` from the editor's backend + content
/// revision. Free function (not a method on `TextEditorComponent`) so
/// production callers that need to mutate other fields of
/// `TextEditorComponent` afterwards can pass `&self.backend` and
/// the session's `revs.current()` directly — the borrow checker can split
/// borrows across distinct fields but not across method calls.
fn snapshot_from_backend(backend: &BackendState, content_revision: NonZeroU64) -> EditorSnapshot {
    match backend {
        BackendState::Textarea(tb) => {
            let cursor = tb.cursor();
            EditorSnapshot::of_buffer(tb.buffer().text().clone(), cursor, content_revision)
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
use self::backend::BackendState;
#[cfg(test)]
use self::find_bar::{BarFocus, SearchStatus};
use self::nvim_host::NvimHost;
use self::view::PainterState;
use crate::logic::editor::Editor;
use crate::logic::editor::claim::{
    ClipboardImage, EditorClaim, FollowTarget, has_trigger_before_cursor,
};
use crate::logic::editor::handled::{Handled, KeyPath};
use crate::logic::editor::overlay::{Overlay, OverlayKind};
use crate::logic::editor::request::EditorRequest;
use crate::logic::editor::rope_buffer::RopeBuffer;
use crate::logic::editor::session::{EditorSession, FrameInput};
use crate::logic::editor::snapshot::EditorSnapshot;

use std::sync::Arc;

use kimun_core::NoteVault;

use crate::components::Component;
use crate::components::autocomplete::{
    self, AutocompleteController, AutocompleteHost, AutocompleteMode, HandleKeyOutcome,
};
use crate::components::event_state::EventState;
use crate::components::events::InputEvent;
use crate::components::text_editor::autocomplete_glue::apply_accept_to_textarea;
use crate::keys::KeyBindings;
#[cfg(test)]
use crate::logic::message::AppMessage;
use crate::logic::message::AppTx;
use crate::logic::message::Notifier;
use crate::logic::text_action::TextAction;
use crate::settings::AppSettings;
use crate::settings::themes::Theme;

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
        // `painter.last_cursor_screen` is available, then re-anchors and
        // draws with the correct position.
        Some(self.cursor_screen.unwrap_or((0, 0)))
    }
}

/// Free-function builder for `EditorHostSnapshot`. Production
/// callers pass `&self.backend`, the session's `revs.current()`,
/// `self.painter.last_cursor_screen` directly so the borrow checker
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
    /// The editing engine. The textarea arm is a [`Editor`], which owns its
    /// buffer, its [`EditorSession`] and its textarea-only state (selection,
    /// drag origin, Insert-session bracket, queued requests).
    backend: BackendState,
    /// Tracks the rendered rect to map mouse click coordinates.
    rect: Rect,
    key_bindings: KeyBindings,
    /// The nvim arm's per-frame machinery: the open note, the revision clock,
    /// needles, the display, the background jobs and the notifier. Unused
    /// while the textarea is active (its [`Editor`] owns its own); handed over
    /// to the textarea if nvim dies. Reach either through `session()`.
    nvim_session: EditorSession,
    /// The nvim arm's queued asks (a right-click wants the note's context
    /// menu); drained by `take_requests` together with the editor's.
    nvim_requests: Vec<EditorRequest>,
    /// What the painter remembers between frames (cursor anchor and shape).
    painter: PainterState,
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
            nvim_session: EditorSession::new(),
            nvim_requests: Vec::new(),
            painter: PainterState::default(),
            nvim_host: NvimHost::new(),
            search: None,
            autocomplete: None,
            autocomplete_vault: None,
        }
    }

    /// The active engine's session: the textarea [`Editor`]'s own, or the
    /// nvim arm's.
    fn session(&self) -> &EditorSession {
        match &self.backend {
            BackendState::Textarea(ed) => ed.session(),
            BackendState::Nvim(_) => &self.nvim_session,
        }
    }

    fn session_mut(&mut self) -> &mut EditorSession {
        match &mut self.backend {
            BackendState::Textarea(ed) => ed.session_mut(),
            BackendState::Nvim(_) => &mut self.nvim_session,
        }
    }

    /// The painted selection. Only the textarea tracks one; always `None` on
    /// nvim.
    fn selection(&self) -> Option<((usize, usize), (usize, usize))> {
        self.backend.as_editor().and_then(|ed| ed.selection())
    }

    /// Set the painted selection. A no-op on nvim, which never has one.
    #[cfg(test)]
    fn set_selection(&mut self, selection: Option<((usize, usize), (usize, usize))>) {
        if let Some(ed) = self.backend.as_editor_mut() {
            ed.set_selection(selection);
        }
    }

    /// The textarea's buffer. Panics on nvim: tests drive the textarea.
    #[cfg(test)]
    fn ta(&mut self) -> &mut RopeBuffer {
        self.backend
            .as_textarea_mut()
            .expect("expected Textarea backend")
    }

    /// The painted selection, for assertions.
    #[cfg(test)]
    fn selection_for_test(&self) -> Option<((usize, usize), (usize, usize))> {
        self.selection()
    }

    /// The active session's revision clock, for assertions.
    #[cfg(test)]
    fn revs_for_test(&self) -> &Revisions {
        &self.session().revs
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
            self.install_notifier_on_autocomplete();
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
        // Fresh controller — it needs the stored notifier installed.
        self.install_notifier_on_autocomplete();
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
        Some(ta.cursor())
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
            self.session().revs.current(),
            self.painter.last_cursor_screen,
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
            let (row, col) = ta.cursor();
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
            self.session().revs.current(),
            self.painter.last_cursor_screen,
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

    /// The open note's text. Empty for the nvim backend, which owns its own.
    pub fn text(&self) -> crate::ropetext::Text {
        match &self.backend {
            BackendState::Textarea(ed) => ed.text(),
            BackendState::Nvim(_) => crate::ropetext::Text::new(),
        }
    }

    /// The editor's atomic `(lines, cursor, content_revision)` view
    /// ([`Editor::view_snapshot`]). On the Nvim backend the lines are cloned
    /// out from behind the `Mutex` and the cursor row is clamped to
    /// `lines.len() - 1` before the snapshot is returned.
    ///
    /// Production hot paths that also need `&mut self.painter` (notably
    /// `render`) must instead inline the snapshot via
    /// `snapshot_from_backend(&self.backend, self.session().revs.current())`
    /// so the borrow checker can split the borrows across distinct
    /// fields.
    pub fn view_snapshot(&self) -> EditorSnapshot {
        match &self.backend {
            BackendState::Textarea(ed) => ed.view_snapshot(),
            BackendState::Nvim(_) => {
                snapshot_from_backend(&self.backend, self.session().revs.current())
            }
        }
    }

    /// The cursor's (row, col) without materialising a snapshot — the Nvim
    /// path of `view_snapshot` clones every buffer line, far too heavy for
    /// per-frame consumers that only want the position (status-bar ln/col).
    pub fn cursor_pos(&self) -> (usize, usize) {
        match &self.backend {
            BackendState::Textarea(ed) => ed.cursor(),
            BackendState::Nvim(_) => self.backend.cursor(),
        }
    }

    /// Set the search needles to emphasize in the rendered buffer (the note
    /// was opened from a query result). Cleared automatically on the first
    /// edit.
    pub fn set_search_needles(&mut self, needles: Vec<String>) {
        match &mut self.backend {
            BackendState::Textarea(ed) => ed.set_search_needles(needles),
            BackendState::Nvim(_) => {
                self.nvim_session.needles = needles
                    .into_iter()
                    .map(|n| n.to_lowercase())
                    .filter(|n| !n.is_empty())
                    .collect();
                self.nvim_session.revs.arm_needles();
            }
        }
    }

    /// Show `note` with `text` ([`Editor::open_note`]).
    pub fn open_note(&mut self, note: kimun_core::nfs::VaultPath, text: String) {
        match &mut self.backend {
            BackendState::Textarea(ed) => {
                if ed.open_note(note, text) {
                    self.after_load();
                }
            }
            BackendState::Nvim(_) => {
                if self
                    .nvim_session
                    .note
                    .as_ref()
                    .is_some_and(|open| open.is_like(&note))
                {
                    self.set_text(text);
                    return;
                }
                self.nvim_session.note = Some(note);
                self.load_nvim(text);
            }
        }
    }

    /// The note on screen was renamed to `note` ([`Editor::renamed_to`]).
    pub fn renamed_to(&mut self, note: kimun_core::nfs::VaultPath) {
        match &mut self.backend {
            BackendState::Textarea(ed) => ed.renamed_to(note),
            BackendState::Nvim(_) => self.nvim_session.note = Some(note),
        }
    }

    /// Reload the note on screen with `text` ([`Editor::set_text`]).
    pub fn set_text(&mut self, text: String) {
        match &mut self.backend {
            BackendState::Textarea(ed) => {
                if ed.set_text(text) {
                    self.after_load();
                }
            }
            BackendState::Nvim(_) => {
                if text == self.backend.text() {
                    self.nvim_session.revs.mark_saved_current();
                    if let Some(nvim) = self.backend.as_nvim() {
                        nvim.mark_clean();
                    }
                    return;
                }
                self.load_nvim(text);
            }
        }
    }

    /// What the TUI layers drop when the buffer was replaced: an open popup
    /// would linger over the new note (e.g. after Ctrl+G follow-link), and the
    /// find bar is worse — `armed_empty` surviving a note swap means one Ctrl+A
    /// deletes every match in a note the user never armed, skipping the
    /// confirmation the flag exists to force.
    fn after_load(&mut self) {
        self.close_autocomplete();
        self.search = None;
    }

    /// Replace the nvim buffer with `text` and drop everything that described
    /// the old one.
    fn load_nvim(&mut self, text: String) {
        if let BackendState::Nvim(nvim) = &mut self.backend {
            nvim.set_text(&text);
        }
        self.bump_content();
        let reconstructed = self.get_text();
        self.mark_saved(reconstructed);
        self.after_load();
        self.nvim_session.display.note_bulk_edit();
    }

    pub fn get_text(&self) -> String {
        match &self.backend {
            BackendState::Textarea(ed) => ed.get_text(),
            BackendState::Nvim(_) => self.backend.text(),
        }
    }

    /// Current content revision ([`Editor::content_revision`]).
    pub fn content_revision(&self) -> NonZeroU64 {
        self.session().revs.current()
    }

    /// Mark the buffer clean iff `rev` is still current
    /// ([`Editor::mark_saved_at_revision`]).
    pub fn mark_saved_at_revision(&mut self, rev: NonZeroU64) {
        match &mut self.backend {
            BackendState::Textarea(ed) => ed.mark_saved_at_revision(rev),
            BackendState::Nvim(_) => {
                if !self.nvim_session.revs.mark_saved_at(rev) {
                    return;
                }
                self.interrupt_typing();
                if let Some(nvim) = self.backend.as_nvim() {
                    nvim.mark_clean();
                }
            }
        }
    }

    /// Synchronous mark-saved ([`Editor::mark_saved`]).
    pub fn mark_saved(&mut self, text: String) {
        match &mut self.backend {
            BackendState::Textarea(ed) => ed.mark_saved(text),
            BackendState::Nvim(_) => {
                self.interrupt_typing();
                let matches = text == self.get_text();
                if matches {
                    if let Some(nvim) = self.backend.as_nvim() {
                        nvim.mark_clean();
                    }
                    self.nvim_session.revs.mark_saved_current();
                } else {
                    self.nvim_session.revs.mark_diverged();
                }
            }
        }
    }

    /// Something happened that is not a continuation of typing
    /// ([`Editor::interrupt_typing`]). Nvim has no typing run: only the
    /// vertical-motion goal cell is forgotten.
    fn interrupt_typing(&mut self) {
        match &mut self.backend {
            BackendState::Textarea(ed) => ed.interrupt_typing(),
            BackendState::Nvim(_) => self.nvim_session.display.clear_visual_goal(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        match &self.backend {
            BackendState::Textarea(ed) => ed.is_dirty(),
            BackendState::Nvim(nvim) => nvim.snapshot().dirty,
        }
    }

    /// Whether a bare Space should start the leader ([`Editor::space_leads`]).
    /// Always false on nvim. The find bar's claim is [`Self::claim`]'s job.
    pub fn space_leads(&self) -> bool {
        match &self.backend {
            BackendState::Textarea(ed) => ed.space_leads(),
            BackendState::Nvim(_) => false,
        }
    }

    /// Whether a press in this editor moves the cursor to the cell under it.
    ///
    /// False on the **nvim** backend, where the terminal and nvim own the
    /// mouse: `Editor::handle_pointer` (via the component) returns `NotConsumed` before any
    /// `jump_to`. Anything that treats a click as pointing *at* something —
    /// following a link, most obviously — has to ask this first, or it reads a
    /// cursor the click never moved.
    pub fn mouse_drives_cursor(&self) -> bool {
        match &self.backend {
            BackendState::Textarea(ed) => ed.mouse_drives_cursor(),
            BackendState::Nvim(_) => false,
        }
    }

    /// Whether this cell is one this editor would place the cursor on.
    ///
    /// Narrower than the editor *column*, which the panel set hit-tests: the
    /// column includes the frame drawn around this component, and `self.rect`
    /// is the interior it was handed at render — minus the find-bar row, which
    /// `render` already excludes. `handle_input` bounds-checks against exactly
    /// this and answers `NotConsumed` outside it, so a press anywhere else
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
        match &self.backend {
            BackendState::Textarea(ed) => ed.follow_target_at_cursor(),
            BackendState::Nvim(nvim) => {
                let snap = nvim.snapshot();
                let (row, col) = snap.cursor;
                let line = snap.lines.get(row)?.to_string();
                crate::logic::editor::claim::follow_target_in_line(&line, col)
            }
        }
    }

    /// Copy selected text to the OS clipboard, flashing the outcome.
    ///
    /// Routed through the shared [`crate::logic::clipboard::yank`] seam so a clipboard
    /// failure is reported rather than swallowed, and so "nothing was selected"
    /// is distinguishable from "the copy failed".
    #[cfg(test)]
    fn copy_selection_to_clipboard(&mut self, tx: &AppTx) {
        match &mut self.backend {
            BackendState::Textarea(ed) => ed.copy_selection_to_clipboard(tx),
            // Nvim has no vim-Visual or textarea selection to read from here.
            BackendState::Nvim(_) => {
                tx.send(AppMessage::FlashMessage("nothing to copy".into()))
                    .ok();
            }
        }
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
                bar.paste(text, tb.buffer_mut());
            }
            self.apply_edit_outcome();
            return;
        }
        match &mut self.backend {
            BackendState::Textarea(ed) => ed.paste_text(text),
            BackendState::Nvim(nvim) => {
                nvim.paste(text, tx.clone());
                self.bump_content();
            }
        }
        // The buffer just changed under the popup's feet; reconcile
        // the trigger context so a stale replace_range cannot survive
        // into the next Accept.
        self.ensure_notifier(tx);
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
        if let BackendState::Textarea(ed) = &mut self.backend {
            ed.insert_at_cursor(text);
        }
        // See `paste_text` — out-of-band buffer mutation must
        // re-reconcile the popup state.
        self.ensure_notifier(tx);
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
        crate::logic::clipboard::read_image()
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
        if let BackendState::Textarea(ed) = &mut self.backend {
            ed.take_selection_for_external_paste();
        }
    }

    /// Bold / Italic / Strikethrough ([`Editor::apply_text_action`]). No-op on
    /// the Nvim backend.
    pub fn apply_text_action(&mut self, action: TextAction) {
        if let BackendState::Textarea(ed) = &mut self.backend {
            ed.apply_text_action(action);
        }
    }

    /// Smart Enter ([`Editor::smart_enter`]). Always `false` on the Nvim
    /// backend.
    pub fn smart_enter(&mut self) -> bool {
        match &mut self.backend {
            BackendState::Textarea(ed) => ed.smart_enter(),
            BackendState::Nvim(_) => false,
        }
    }

    /// The OUTLINE drawer's jump ([`Editor::jump_to_row`]). No-op on the Nvim
    /// backend.
    pub fn jump_to_row(&mut self, row: usize) {
        if let BackendState::Textarea(ed) = &mut self.backend {
            ed.jump_to_row(row);
        }
    }

    /// Indent or dedent whole rows ([`Editor::indent_lines`]). No-op on the
    /// Nvim backend.
    pub fn indent_lines(&mut self, dedent: bool) {
        if let BackendState::Textarea(ed) = &mut self.backend {
            ed.indent_lines(dedent);
        }
    }
}

impl TextEditorComponent {
    /// Advances the revision clock ([`Editor::bump_content`]).
    ///
    /// Not called by the Nvim path — the reverse-refresh task in
    /// `backend.rs` bumps `snap.content_gen` on real diffs, the frame
    /// snapshot derives its revision from that, and `render` adopts the
    /// snapshot's value (see the `revs` field doc).
    #[inline]
    fn bump_content(&mut self) {
        self.session_mut().revs.bump();
    }

    /// If the Nvim process has died, fall back to a Textarea with the last known content.
    fn maybe_recover_from_dead_nvim(&mut self) {
        if self.backend.recover_from_dead_nvim(&mut self.nvim_session) {
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
        let selection = if tb.buffer_mut().search_repeat(backward) {
            tb.buffer().match_at_cursor()
        } else {
            None
        };
        tb.set_selection(selection);
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
        let outcome = f(bar, tb.buffer_mut());
        if outcome.close {
            self.search = None;
            // Clear the anchor as well as the mirrored range. The bar's cursor
            // jumps (`search_forward`) move the cursor without touching
            // `selection_start`, so a selection that existed before the search
            // is left live but unpainted — and the next keystroke silently
            // deletes it.
            tb.buffer_mut().cancel_selection();
            tb.set_selection(None);
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
    fn replace_preview(&self) -> Option<crate::logic::editor::find_replace::Preview> {
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

    /// Hand the host's wake-up to the component: the background full-parse
    /// and full-layout tasks call it when they finish, and the autocomplete
    /// controller calls it when its query task resolves. Set it up front
    /// (e.g. on note open) so the FIRST render of a large buffer can wake
    /// the event-driven render loop.
    pub fn set_notifier(&mut self, notifier: Notifier) {
        self.session_mut().notifier = Some(notifier);
        self.install_notifier_on_autocomplete();
    }

    /// Install the stored notifier on the autocomplete controller, if both
    /// exist. Called whenever either is (re)created.
    fn install_notifier_on_autocomplete(&mut self) {
        if let (Some(n), Some(c)) = (self.session().notifier.clone(), self.autocomplete.as_mut()) {
            c.set_redraw_callback(n);
        }
    }

    /// TUI-only convenience preserving the lazy binding: callers that still
    /// receive an `&AppTx` fill in an unset notifier from it. It is a TUI-side
    /// convenience and stays.
    fn ensure_notifier(&mut self, tx: &AppTx) {
        if self.session().notifier.is_none() {
            self.set_notifier(crate::logic::message::redraw_callback(tx.clone()));
        }
    }

    /// What every edit made through the buffer owes the component
    /// ([`Editor::after_edit`]). Nvim has no edit buffer: the typing state is
    /// settled and nothing changed.
    fn after_edit(&mut self) -> bool {
        match &mut self.backend {
            BackendState::Textarea(ed) => ed.after_edit(),
            BackendState::Nvim(_) => {
                self.interrupt_typing();
                false
            }
        }
    }

    /// Settle an edit a TUI layer (find bar, autocomplete accept) made on the
    /// buffer ([`Editor::finish_external_edit`]). Nvim has no edit buffer.
    fn apply_edit_outcome(&mut self) -> bool {
        match &mut self.backend {
            BackendState::Textarea(ed) => ed.finish_external_edit(),
            BackendState::Nvim(_) => false,
        }
    }

    /// The direct-edit path alone, as the tests that pin it drive it
    /// ([`Editor::handle_direct_key`]): no popup, find-bar or vim stage, and
    /// no autocomplete follow-up.
    #[cfg(test)]
    fn handle_textarea_key(&mut self, key: KeyInput, tx: &AppTx) -> EventState {
        let Some(ed) = self.backend.as_editor_mut() else {
            unreachable!("handle_textarea_key called with non-Textarea backend")
        };
        event_state(ed.handle_direct_key(key, tx).0)
    }

    /// Act on what the autocomplete popup did with a key or a click — one
    /// path, so an accept edits the buffer the same way from either. `None`:
    /// the popup passed, the event is the editor's.
    fn apply_popup_outcome(&mut self, outcome: HandleKeyOutcome) -> Option<EventState> {
        match outcome {
            HandleKeyOutcome::Accepted(action) => {
                self.interrupt_typing();
                if let Some(ta) = self.backend.as_textarea_mut() {
                    ta.edit(|ta| apply_accept_to_textarea(ta, &action));
                }
                self.after_edit();
                Some(EventState::Consumed)
            }
            HandleKeyOutcome::Dismissed | HandleKeyOutcome::Consumed => Some(EventState::Consumed),
            HandleKeyOutcome::NotHandled => None,
        }
    }

    /// A pointer event inside the editor that has no [`PointerInput`]
    /// (horizontal scroll), on the nvim engine: still an action, but the
    /// terminal owns it.
    fn ignored_nvim_pointer(&mut self) -> EventState {
        self.interrupt_typing();
        EventState::NotConsumed
    }

    /// Drain what the editor has asked its host to do since the last call.
    pub fn take_requests(&mut self) -> Vec<EditorRequest> {
        let mut requests = std::mem::take(&mut self.nvim_requests);
        if let Some(ed) = self.backend.as_editor_mut() {
            requests.extend(ed.take_requests());
        }
        requests
    }

    /// A pointer event on the nvim engine. The caller has already checked
    /// that it landed inside the editor. Only the right-press context-menu
    /// ask is backend-independent; the terminal and nvim own the rest.
    fn handle_nvim_pointer(&mut self, pointer: &PointerInput) -> EventState {
        // Past the bounds check the event is ours, so it is an action.
        self.interrupt_typing();
        // Right-click without a selection (nvim never has one here) asks the
        // host to open the note's context menu (file & note ops).
        if matches!(pointer.kind, PointerKind::Down(Button::Right))
            && self.selection().is_none_or(|(start, end)| start == end)
        {
            self.nvim_requests.push(EditorRequest::ContextMenu);
            return EventState::Consumed;
        }
        EventState::NotConsumed
    }

    /// Act on what the textarea [`Editor`] queued during this input call
    /// ([`Editor::take_host_requests`]): the find bar and the popup are this
    /// component's; `OpenPalette` is forwarded to the app as
    /// `AppMessage::ExecuteLeaderAction(LeaderAction::Palette)`. `ContextMenu`
    /// stays queued for the screen's [`Self::take_requests`] poll.
    fn drain_host_requests(&mut self, tx: &AppTx) {
        let Some(ed) = self.backend.as_editor_mut() else {
            return;
        };
        for request in ed.take_host_requests() {
            match request {
                EditorRequest::OpenSearch => self.open_or_advance_search(),
                EditorRequest::SearchNext => self.search_repeat(false),
                EditorRequest::SearchPrev => self.search_repeat(true),
                // The palette is the app's: reuse its leader-action gateway.
                EditorRequest::OpenPalette => {
                    tx.send(crate::logic::message::AppMessage::ExecuteLeaderAction(
                        crate::keys::leader::LeaderAction::Palette,
                    ))
                    .ok();
                }
                // The popup is anchored to the cursor, which may now be off
                // screen: left open, it would take keys the user cannot see.
                EditorRequest::ViewportScrolled => self.close_autocomplete(),
                // Never handed out by `take_host_requests`.
                EditorRequest::ContextMenu => {}
            }
        }
    }
}

/// The one place the editor's neutral answer becomes the TUI's.
fn event_state(handled: Handled) -> EventState {
    match handled {
        Handled::Consumed => EventState::Consumed,
        Handled::NotConsumed => EventState::NotConsumed,
    }
}

/// Viewport post-pass: emphasize search-needle matches
/// (`color_search_match`, bold) and style task checkboxes — `[ ]` accent,
/// `[x]` rows dimmed + struck. Operates on the rendered buffer
/// rows, so cost is bounded by the visible area regardless of note size.
impl Component for TextEditorComponent {
    fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        self.maybe_recover_from_dead_nvim();
        self.ensure_notifier(tx);

        match event {
            InputEvent::Key(key) => {
                // The one crossterm -> neutral conversion for the direct path
                // and the vim engine. The popup, find bar and nvim stages keep
                // the raw event: they speak crossterm.
                let input = crate::keys::crossterm_input::key_input(key);
                // Cheap popup-open probe first. The snapshot is now a
                // Cow-borrowed view of the textarea's lines (zero
                // allocation on the Textarea path — perf #8), so
                // idle keystrokes pay nothing here even when popup
                // checks fire. The free-function form lets `&self.backend`
                // and `&mut self.autocomplete` coexist via field-disjoint
                // borrows.
                let popup_open = self.autocomplete.as_ref().is_some_and(|c| c.is_open());
                // `Editor::handle_key` re-attaches the view to the cursor for
                // the keys that reach it. These never do, and acted at the
                // cursor all the same.
                if popup_open || self.search.is_some() || self.backend.as_editor().is_none() {
                    self.session_mut().display.follow_cursor();
                }
                if popup_open
                    && let Some(host) = build_editor_host_snapshot(
                        &self.backend,
                        self.session().revs.current(),
                        self.painter.last_cursor_screen,
                    )
                    && let Some(controller) = self.autocomplete.as_mut()
                {
                    let outcome = controller.handle_key(*key, &host);
                    if let Some(state) = self.apply_popup_outcome(outcome) {
                        return state;
                    }
                }
                // Find bar intercepts all keys while active. Must run before the
                // vim engine, which would otherwise consume keys in Normal mode
                // (the bar is routed only from here; handle_textarea_key has no
                // find-bar check of its own).
                if self.dispatch_to_find_bar(key) {
                    // Here rather than inside `dispatch_bar`: that runs for every
                    // key whether or not a bar is open, so interrupting there
                    // would make each keystroke its own undo group.
                    self.interrupt_typing();
                    return EventState::Consumed;
                }
                // The nvim engine takes the key whole, after the popup and the
                // bar (the vim stage never fires for it).
                if let Some(state) = self.handle_nvim_key(key, tx) {
                    return state;
                }
                let Some(ed): Option<&mut Editor> = self.backend.as_editor_mut() else {
                    unreachable!("handle_nvim_key declined a non-Textarea backend")
                };
                // The vim engine, then the direct path ([`Editor::handle_key`]).
                // A key with no `KeyInput` (e.g. `Null`, media keys) is one vim
                // can't use: it takes the direct path's "unmapped" branch.
                let (handled, path) = match input {
                    Some(i) => ed.handle_key(i, tx),
                    None => ed.ignore_unmapped_key(),
                };
                // Vim's `/`, `?`, `n`, `N` act in this same call.
                self.drain_host_requests(tx);
                match path {
                    // The direct path compares the buffer either side of it
                    // using cheap counters instead of cloning the whole
                    // buffer. Three outcomes:
                    //   - text changed → sync (may open a fresh popup)
                    //   - text unchanged, cursor moved → refresh (close
                    //     popup if cursor left the trigger range; never
                    //     open new popup just because the cursor passed
                    //     over an existing wikilink/hashtag)
                    //   - both unchanged → no autocomplete work needed
                    // A clipboard paste re-syncs first, as every paste does.
                    KeyPath::Direct {
                        text_changed,
                        cursor_moved,
                        pasted,
                    } => {
                        // Mirrors the old paste-then-post-key-diff double
                        // sync; idempotent when the text did not change.
                        if pasted {
                            self.sync_autocomplete();
                        }
                        if text_changed {
                            self.sync_autocomplete();
                        } else if cursor_moved {
                            self.refresh_autocomplete_if_open();
                        }
                    }
                    KeyPath::VimCursorOnly => self.refresh_autocomplete_if_open(),
                    // The buffer just changed under the popup's feet; reconcile
                    // the trigger context so a stale replace_range cannot
                    // survive into the next Accept.
                    KeyPath::VimPasted => self.sync_autocomplete(),
                    KeyPath::VimOther => {}
                }
                event_state(handled)
            }
            InputEvent::Mouse(mouse) => {
                // An open popup gets first crack: a click on a suggestion
                // accepts it — the same edit Tab/Enter make — and the wheel
                // over it moves the highlight instead of the buffer.
                if self.autocomplete.as_ref().is_some_and(|c| c.is_open())
                    && let Some(host) = build_editor_host_snapshot(
                        &self.backend,
                        self.session().revs.current(),
                        self.painter.last_cursor_screen,
                    )
                    && let Some(controller) = self.autocomplete.as_mut()
                {
                    let outcome = controller.handle_mouse(mouse, &host);
                    if let Some(state) = self.apply_popup_outcome(outcome) {
                        return state;
                    }
                }
                let text_rev_before = self.session().revs.current();
                let cursor_before = self.textarea_cursor();
                let result = if self.covers(mouse.column, mouse.row) {
                    let pointer = pointer_input(mouse, (self.rect.x, self.rect.y));
                    match self.backend.as_editor_mut() {
                        Some(ed) => {
                            let handled = match pointer {
                                Some(pointer) => ed.handle_pointer(&pointer, tx),
                                None => ed.ignored_pointer(),
                            };
                            // A wheel scroll that moved the view closes the
                            // popup, before the diff below.
                            self.drain_host_requests(tx);
                            event_state(handled)
                        }
                        None => match pointer {
                            Some(pointer) => self.handle_nvim_pointer(&pointer),
                            None => self.ignored_nvim_pointer(),
                        },
                    }
                } else {
                    EventState::NotConsumed
                };
                let cursor_after = self.textarea_cursor();
                // Mouse clicks typically only move the cursor — refresh
                // (which may close the popup) but do not auto-open.
                if self.session().revs.current() != text_rev_before {
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

                // Reconcile the vim engine's mode by gesture, not by the shape
                // the selection happens to have: a left press ends Visual, and a
                // drag selects (`select_dragged`, in `Editor::handle_pointer`); scroll,
                // release and right-click leave the mode alone. Insert and
                // Replace, and the other backends, are untouched either way.
                let left_down = matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left));
                if let Some(ed) = self.backend.as_editor_mut() {
                    ed.settle_pointer(left_down);
                }
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
            BackendState::Textarea(ed) => match self.search.as_ref() {
                Some(bar) => bar.current_match(),
                None => ed.selection(),
            },
            BackendState::Nvim(nvim) => {
                self.nvim_host
                    .frame_sync(nvim, editor_rect.width, editor_rect.height)
            }
        };
        // Everything between here and `prepare_frame` is the find bar's: the
        // **replace preview** is computed before the snapshot so it owns its
        // lines outright. The buffer is never touched — only this frame's view
        // of it is substituted, which is what makes the preview structurally
        // incapable of committing. One call gets everything the bar wants
        // painted: preview lines and spans, match spans, and the current match.
        let overlay = match (self.search.as_ref(), self.backend.as_textarea()) {
            (Some(bar), Some(buf)) => bar.overlay(buf),
            _ => find_bar::BarOverlay::default(),
        };
        // Single producer for the atomic snapshot: the free function lets the
        // borrow checker split `&self.backend` from `&mut self.session`.
        let snap = snapshot_from_backend(&self.backend, self.session().revs.current());
        let empty_note = snap.text.len_bytes() == 0;
        let (preview, preview_overlays) = match overlay.preview {
            None => (None, Vec::new()),
            Some(p) => (
                Some(p.lines),
                p.spans
                    .iter()
                    .map(|p| {
                        Overlay::new(
                            p.row,
                            p.start,
                            p.end,
                            if p.is_current {
                                OverlayKind::PreviewCurrent
                            } else {
                                OverlayKind::Preview
                            },
                        )
                    })
                    .collect(),
            ),
        };
        // `prepare_frame` ignores find matches while a replace preview shows,
        // so don't build them then.
        let matches = if preview.is_some() {
            Vec::new()
        } else {
            overlay
                .matches
                .iter()
                .map(|&(row, start, end)| Overlay::new(row, start, end, OverlayKind::Match))
                .collect()
        };
        // Drain background results, adopt the revision, update the display,
        // set overlays, spawn pending jobs — in that order. See
        // [`EditorSession::prepare_frame`].
        self.session_mut().prepare_frame(FrameInput {
            snapshot: snap,
            width: editor_rect.width,
            height: editor_rect.height,
            selection,
            preview,
            preview_overlays,
            matches,
        });
        // When the find bar is active, draw it AFTER the editor so its caret
        // (set via set_cursor_position) wins over the editor's caret call.
        let bar_focused = self.search.is_some() && focused;
        let editor_focused = focused && !bar_focused;
        use crate::logic::editor::display::CursorShape;
        let cursor_shape = match self.backend.modal_is_insert() {
            None => None, // Direct textarea — leave terminal default
            Some(true) => Some(CursorShape::Bar),
            Some(false) => Some(CursorShape::Block),
        };
        if editor_rect.height > 0 {
            let frame = self
                .session_mut()
                .display
                .frame(editor_rect.width, editor_focused);
            view::paint_frame(
                f,
                editor_rect,
                theme,
                frame,
                cursor_shape,
                &mut self.painter,
            );
        }

        // Search-match emphasis: paint needle matches and task
        // checkboxes over the rendered viewport. Buffer-level post-pass —
        // viewport-only, so large notes pay nothing beyond the visible rows.

        // Empty-note tip: dim ghost text in a fresh/empty buffer,
        // gone the instant the first character lands (the buffer stops being
        // empty). Drawn after the view so it sits over the blank canvas.
        if empty_note && editor_rect.height > 0 {
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
        let live_anchor = self.painter.last_cursor_screen;
        if let Some(state) = self.autocomplete.as_mut().and_then(|c| c.state_mut()) {
            match live_anchor {
                Some(anchor) => {
                    state.anchor = anchor;
                    autocomplete::render(f, state, editor_rect, theme);
                }
                // Not drawn this frame, so nothing of it may be clickable.
                None => state.rows_rect = Rect::default(),
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
        // most relevant action.
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
    use super::*;
    use crate::keys::KeyBindings;
    use crate::logic::editor::snapshot::EditorMode;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    fn make_editor() -> TextEditorComponent {
        TextEditorComponent::new(
            KeyBindings::empty(),
            &crate::settings::AppSettings::default(),
        )
    }

    fn dummy_tx() -> AppTx {
        tokio::sync::mpsc::unbounded_channel().0
    }

    /// A click on a suggestion in the editor's popup inserts it — the same
    /// edit Tab makes.
    #[tokio::test]
    async fn clicking_an_autocomplete_suggestion_inserts_it() {
        use crate::components::search_list::{SuggestionItem, SuggestionSource};
        use ratatui::{Terminal, backend::TestBackend};
        struct Tags;
        #[async_trait::async_trait]
        impl SuggestionSource for Tags {
            async fn notes_by_prefix(&self, _p: &str, _n: usize) -> Vec<SuggestionItem> {
                vec![]
            }
            async fn tags_by_prefix(&self, p: &str, _n: usize) -> Vec<SuggestionItem> {
                if "projects".starts_with(p) {
                    vec![SuggestionItem::plain("projects")]
                } else {
                    vec![]
                }
            }
        }
        let mut editor = make_editor();
        editor.autocomplete = Some(
            AutocompleteController::new(std::sync::Arc::new(Tags), AutocompleteMode::Both)
                .with_debounce(std::time::Duration::ZERO),
        );
        let tx = dummy_tx();
        let theme = Theme::default();
        let mut t = Terminal::new(TestBackend::new(60, 20)).unwrap();
        let draw = |editor: &mut TextEditorComponent, t: &mut Terminal<TestBackend>| {
            t.draw(|f| editor.render(f, f.area(), &theme, true))
                .unwrap();
        };
        draw(&mut editor, &mut t);
        for c in ['#', 'p', 'r', 'o'] {
            editor.handle_input(
                &InputEvent::Key(ratatui::crossterm::event::KeyEvent::from(
                    ratatui::crossterm::event::KeyCode::Char(c),
                )),
                &tx,
            );
            draw(&mut editor, &mut t);
        }
        for _ in 0..50 {
            tokio::task::yield_now().await;
            draw(&mut editor, &mut t);
        }
        let rows = editor
            .autocomplete
            .as_ref()
            .and_then(|c| c.state())
            .map(|st| st.rows_rect)
            .filter(|r| !r.is_empty())
            .expect("popup drawn");
        editor.handle_input(&crate::test_support::mouse_down_at(rows.x + 1, rows.y), &tx);
        assert_eq!(editor.get_text(), "#projects");
    }

    /// A large note parses in the background; when that parse finishes the
    /// component must wake the host through its notifier.
    #[tokio::test]
    async fn a_finished_background_parse_wakes_the_host() {
        use ratatui::{Terminal, backend::TestBackend};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let woken = std::sync::Arc::new(AtomicUsize::new(0));
        let mut editor = make_editor();
        let counter = woken.clone();
        editor.set_notifier(std::sync::Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        let text: String = (0..1200)
            .map(|i| format!("paragraph {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        editor.set_text(text);
        let theme = Theme::default();
        let mut t = Terminal::new(TestBackend::new(60, 20)).unwrap();
        t.draw(|f| editor.render(f, f.area(), &theme, true))
            .unwrap();
        for _ in 0..200 {
            if woken.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(woken.load(Ordering::SeqCst) >= 1, "host was never woken");
    }

    /// Tests never call `set_notifier`; the first `handle_input` fills it in
    /// from the `AppTx` it receives (TUI-only lazy binding).
    #[test]
    fn an_unset_notifier_is_filled_in_from_the_first_handle_input_tx() {
        let mut editor = make_editor();
        assert!(editor.session().notifier.is_none());
        editor.handle_input(
            &InputEvent::Key(ratatui::crossterm::event::KeyEvent::from(
                ratatui::crossterm::event::KeyCode::Char('a'),
            )),
            &dummy_tx(),
        );
        assert!(editor.session().notifier.is_some());
    }

    /// Regression: copy is read-only over a vim charwise Visual selection.
    /// Every clipboard action reports its outcome, paste included. Before this,
    /// Ctrl+V was the only one that said nothing, so the footer was left showing
    /// the raw chord echo — indistinguishable from an unbound key.
    ///
    /// Drives the production `Editor::paste_from_clipboard` through the
    /// hermetic clipboard override, so the OS clipboard is never touched.
    #[test]
    fn paste_reports_its_outcome() {
        let mut editor = make_editor();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        editor.set_text("x".to_string());
        let _clip = crate::logic::clipboard::set_test_clipboard_text(Ok("hello".into()));
        let ed = editor.backend.as_editor_mut().unwrap();
        assert!(
            ed.paste_from_clipboard(&tx),
            "text arrived and was inserted"
        );
        let reported = std::iter::from_fn(|| rx.try_recv().ok())
            .any(|e| matches!(e, AppMessage::FlashMessage(m) if m == "pasted"));
        assert!(reported, "a paste attempt must always report something");
    }

    fn get_ta(editor: &mut TextEditorComponent) -> &mut RopeBuffer {
        editor.ta()
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
    fn mouse_down_clears_selection() {
        let mut editor = make_editor();
        editor.set_text("hello world".to_string());
        let ta = get_ta(&mut editor);
        ta.start_selection();
        ta.move_cursor(CursorMove::WordForward);
        assert!(ta.selection_range().is_some());
        ta.cancel_selection();
        let selection = editor
            .backend
            .as_textarea()
            .and_then(|ta| ta.selection_range());
        editor.set_selection(selection);
        assert!(editor.selection_for_test().is_none());
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
    fn open_char_without_selection_inserts_normally() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        let ta = get_ta(&mut editor);
        ta.move_cursor(CursorMove::End);
        send_char(&mut editor, '(');
        assert_eq!(editor.get_text(), "hello(");
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
        assert_eq!(ed.session().needles, vec!["alpha"]);
        assert!(!ed.revs_for_test().needles_stale());

        // An edit bumps the revision; the render-side guard would clear.
        ed.set_text("alpha beta gamma".to_string());
        assert!(ed.revs_for_test().needles_stale());
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
        assert_eq!(editor.selection_for_test(), None);
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
        assert!(editor.selection_for_test().is_none());
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
    fn a_selection_ending_at_column_zero_leaves_that_row_alone() {
        let mut editor = make_editor();
        editor.set_text("foo\nbar\nbaz".to_string());
        select_range(&mut editor, (0, 0), (2, 0));
        editor.indent_lines(false);
        assert_eq!(get_ta(&mut editor).rows(), &["    foo", "    bar", "baz"]);
        assert_eq!(editor.selection_for_test(), Some(((0, 4), (2, 0))));
    }

    #[test]
    fn bold_action_with_no_selection_inserts_a_pair_through_the_component() {
        // The wrap declines without a selection, and the component falls
        // through to the pair insert — the one branch the module tests cannot see.
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::End);
        }
        editor.apply_text_action(TextAction::Bold);
        assert_eq!(editor.get_text(), "hello****");
        assert_eq!(get_ta(&mut editor).cursor(), (0, 7));
        assert!(editor.is_dirty(), "the insert is an edit");
    }

    #[test]
    fn tab_indents_the_cursor_row_and_backtab_dedents_it() {
        let mut editor = make_editor();
        editor.set_text("foo".to_string());
        let tx = dummy_tx();
        let tab = key(KeyCode::Tab, KeyModifiers::NONE);
        let _ = editor.handle_input(&InputEvent::Key(tab), &tx);
        assert_eq!(editor.get_text(), "    foo");
        assert!(editor.is_dirty(), "an indent is an edit");
        let back = key(KeyCode::BackTab, KeyModifiers::SHIFT);
        let _ = editor.handle_input(&InputEvent::Key(back), &tx);
        assert_eq!(editor.get_text(), "foo");
    }

    #[test]
    fn enter_at_the_end_of_a_list_item_continues_it() {
        let mut editor = make_editor();
        editor.set_text("- foo".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::End);
        }
        let tx = dummy_tx();
        let enter = key(KeyCode::Enter, KeyModifiers::NONE);
        let _ = editor.handle_input(&InputEvent::Key(enter), &tx);
        assert_eq!(editor.get_text(), "- foo\n- ");
        assert!(editor.is_dirty());
    }

    /// Regression: Enter in vim **Normal** mode (cursor merely navigated onto
    /// the row, never typed into it this session) must continue a list the
    /// same way Insert mode's PassThrough does, and — like `o`/`O` — drop
    /// into Insert so the next item can be typed right away. Normal mode had
    /// no arm for `KeyCode::Enter` at all, so the key fell to the catch-all
    /// `NoOp` and was swallowed before ever reaching `smart_enter` —
    /// reproduces the "pressing Enter doesn't continue the list" report,
    /// which only shows up once the cursor sits in Normal mode (e.g. after a
    /// mouse click, or Esc back from Insert) rather than mid-typing.
    #[test]
    fn vim_normal_mode_enter_at_the_end_of_a_list_item_continues_it() {
        let mut editor = make_vim_editor();
        editor.set_text("- foo".to_string());
        assert_eq!(
            vim_mode(&editor),
            EditorMode::Normal,
            "vim starts in Normal"
        );
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::End);
        }
        let tx = dummy_tx();
        let enter = key(KeyCode::Enter, KeyModifiers::NONE);
        let _ = editor.handle_input(&InputEvent::Key(enter), &tx);
        assert_eq!(editor.get_text(), "- foo\n- ");
        assert!(editor.is_dirty());
        assert_eq!(
            vim_mode(&editor),
            EditorMode::Insert,
            "continuing a list is also where o/O leave you: ready to type the next item"
        );
    }

    /// Enter on an ordinary row — where `smart_enter` has nothing to
    /// continue — falls back to a plain split at the cursor, same as Insert
    /// mode's own Enter. It must not stay a no-op: a note-taking app's Enter
    /// always makes a new line, continuing a list only when there is one.
    #[test]
    fn vim_normal_mode_enter_on_a_plain_row_splits_it_like_insert_mode() {
        let mut editor = make_vim_editor();
        editor.set_text("foobar".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.jump_to(0, 3);
        }
        let tx = dummy_tx();
        let enter = key(KeyCode::Enter, KeyModifiers::NONE);
        let _ = editor.handle_input(&InputEvent::Key(enter), &tx);
        assert_eq!(editor.get_text(), "foo\nbar");
        assert_eq!(vim_mode(&editor), EditorMode::Insert);
    }

    /// Guardrail: a pending operator keeps its own meaning for Enter rather
    /// than SmartEnter guessing one (`d<CR>` is unmapped here, not a
    /// surprise edit). Enter must stay inert and the operator must stay
    /// pending, exactly like today's behaviour for any other unmapped key.
    #[test]
    fn vim_normal_mode_enter_with_a_pending_operator_stays_unmapped() {
        let mut editor = make_vim_editor();
        editor.set_text("- foo".to_string());
        let tx = dummy_tx();
        let _ = editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('d'), KeyModifiers::NONE)),
            &tx,
        );
        let _ = editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "- foo", "d<CR> must not edit");
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
        // The pending `d` is still live: `d` again completes `dd`.
        let _ = editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('d'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(
            editor.get_text(),
            "",
            "the pending operator must have survived Enter"
        );
    }

    /// Guardrail: a pending count keeps its own meaning too (`3<CR>` is
    /// unmapped here, not "continue the list 3 times").
    #[test]
    fn vim_normal_mode_enter_with_a_pending_count_stays_unmapped() {
        let mut editor = make_vim_editor();
        editor.set_text("- foo".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::End);
        }
        let tx = dummy_tx();
        let _ = editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('3'), KeyModifiers::NONE)),
            &tx,
        );
        let _ = editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "- foo", "3<CR> must not edit");
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
    }

    /// `.` must reproduce the Enter action — not skip it — the same way it
    /// reproduces `o`. The natural workflow: continue a list, type the item,
    /// Esc, `.` to add another — with NO repositioning in between. Esc's
    /// usual one-column step back (vim: cursor settles on the last typed
    /// char, not past it) must not make the replay's `smart_enter` call
    /// decline for being "mid-row" and silently split the word instead.
    #[test]
    fn vim_dot_repeats_the_enter_action() {
        let mut editor = make_vim_editor();
        editor.set_text("- foo".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::End);
        }
        let tx = dummy_tx();
        let _ = editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "- foo\n- ");
        for c in "bar".chars() {
            send_char(&mut editor, c);
        }
        assert_eq!(editor.get_text(), "- foo\n- bar");
        let _ = editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
        assert_eq!(
            get_ta(&mut editor).cursor(),
            (1, 4),
            "Esc lands one column short of the row's true end (len 5) — the exact state that broke dot-repeat"
        );

        let _ = editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('.'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "- foo\n- bar\n- bar");
    }

    /// Dot-repeat where nothing was typed before Esc: replaying at the
    /// current row re-evaluates `smart_enter` fresh there, same as a second,
    /// literal Enter press would. The row IS the just-opened, still-empty
    /// bullet, so that re-evaluation clears it rather than opening a third —
    /// correct (it matches a real second keypress at that spot), not a
    /// repeat of the previous continuation.
    #[test]
    fn vim_dot_repeats_an_empty_enter_by_clearing_the_still_empty_bullet() {
        let mut editor = make_vim_editor();
        editor.set_text("- foo".to_string());
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::End);
        }
        let tx = dummy_tx();
        let _ = editor.handle_input(
            &InputEvent::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
            &tx,
        );
        let _ = editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        let _ = editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('.'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(editor.get_text(), "- foo\n");
    }

    #[test]
    fn a_letter_typed_after_smart_enter_starts_its_own_undo_group() {
        let mut editor = make_editor();
        editor.set_text(String::new());
        for c in "- foo".chars() {
            send_char(&mut editor, c);
        }
        let tx = dummy_tx();
        let enter = key(KeyCode::Enter, KeyModifiers::NONE);
        let _ = editor.handle_input(&InputEvent::Key(enter), &tx);
        send_char(&mut editor, 'b');
        assert_eq!(editor.get_text(), "- foo\n- b");
        assert!(get_ta(&mut editor).undo());
        assert_eq!(
            editor.get_text(),
            "- foo\n- ",
            "the letter after a list continuation is its own undo group"
        );
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
    /// the editor's selection — `Editor::handle_pointer` has no find-bar guard. Reading the
    /// span from there dropped the end row and handed `replace_range` an
    /// inverted byte range, panicking the whole TUI.
    #[test]
    fn a_multi_row_selection_cannot_derail_an_interactive_replace() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("alpha beta\nxy".to_string());
        open_replace_bar(&mut editor, &tx, "beta", "Z");
        // Exactly what a drag from row 0 col 6 to row 1 col 1 leaves behind.
        editor.set_selection(Some(((0, 6), (1, 1))));
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

    /// Indenting N lines is 2N history entries, so before the **rope buffer**
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
    /// suppresses the right-click context menu, which reads the editor's selection.
    #[test]
    fn closing_the_bar_clears_a_stale_selection() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("alpha beta".to_string());
        editor.set_selection(Some(((0, 0), (0, 5))));
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
            editor.selection_for_test(),
            None,
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
            editor.selection_for_test(),
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

    /// vim: `>` over a charwise selection shifts every row it touches, with
    /// the ends mid-row.
    #[test]
    fn vim_charwise_visual_indent_shifts_the_touched_rows() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("one\ntwo\nthree".to_string());
        for c in ['l', 'v', 'j'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        // Terminals report `>` as the shifted key it is.
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('>'), KeyModifiers::SHIFT)),
            &tx,
        );
        assert_eq!(editor.get_text(), "    one\n    two\nthree");
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
        assert_eq!(editor.selection_for_test(), None);
    }

    /// Opening another note forgets the last Visual selection: `gv` there
    /// must not select rows the user never selected in that note.
    #[test]
    fn vim_gv_does_not_cross_notes() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("a\nb\nc".to_string());
        for c in ['V', 'j'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        editor.set_text("x\ny\nz".to_string());
        for c in ['g', 'v'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
        assert_eq!(editor.selection_for_test(), None);
    }

    fn press(editor: &mut TextEditorComponent, keys: &str) {
        let tx = dummy_tx();
        for c in keys.chars() {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
    }

    fn note(name: &str) -> kimun_core::nfs::VaultPath {
        kimun_core::nfs::VaultPath::note_path_from(name)
    }

    /// The common case of identical text: two empty notes. Redo must not
    /// reach into the other one either.
    #[test]
    fn empty_notes_do_not_share_history() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.open_note(note("alpha"), String::new());
        press(&mut editor, "ix");
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        press(&mut editor, "u");
        assert_eq!(editor.get_text(), "");
        editor.open_note(note("beta"), String::new());
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            &tx,
        );
        assert_eq!(editor.get_text(), "", "beta has nothing to redo");
    }

    /// Nor the selection, its highlight, or `gv`.
    #[test]
    fn a_note_with_the_same_text_does_not_inherit_the_selection() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.open_note(note("alpha"), "a\nb\nc".to_string());
        press(&mut editor, "Vj");
        editor.handle_input(&InputEvent::Key(key(KeyCode::Esc, KeyModifiers::NONE)), &tx);
        press(&mut editor, "Vj");
        editor.open_note(note("beta"), "a\nb\nc".to_string());
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
        assert_eq!(editor.selection_for_test(), None);
        press(&mut editor, "gv");
        assert_eq!(
            vim_mode(&editor),
            EditorMode::Normal,
            "no selection to reselect"
        );
    }

    /// The top screen row after rendering, trimmed.
    fn top_row(term: &ratatui::Terminal<ratatui::backend::TestBackend>, width: u16) -> String {
        (0..width)
            .filter_map(|x| term.backend().buffer().cell((x, 0)).map(|c| c.symbol()))
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// Another note opens scrolled to its top — also when its text is the
    /// same as the note that was scrolled down.
    #[test]
    fn another_note_opens_scrolled_to_the_top() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut editor = make_vim_editor();
        let theme = Theme::default();
        let mut term = Terminal::new(TestBackend::new(20, 8)).unwrap();
        let area = Rect::new(0, 0, 20, 8);
        let body: String = (0..400).map(|i| format!("line {i}\n")).collect();
        editor.open_note(note("alpha"), body.clone());
        press(&mut editor, "300G");
        term.draw(|f| editor.render(f, area, &theme, true)).unwrap();
        assert_ne!(top_row(&term, 20), "line 0", "alpha is scrolled down");

        editor.open_note(note("beta"), body);
        term.draw(|f| editor.render(f, area, &theme, true)).unwrap();
        assert_eq!(top_row(&term, 20), "line 0");
    }

    /// A paste from outside the key path (clipboard image) replaces the
    /// selection; `gv` afterwards lands where it was, not on stale numbers.
    #[test]
    fn vim_gv_after_external_paste_lands_where_the_selection_was() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("a\nb\nc".to_string());
        for c in ['j', 'V', 'j'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        editor.take_selection_for_external_paste();
        for c in ['g', 'v'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        assert_eq!(vim_mode(&editor), EditorMode::VisualLine);
        let ta = get_ta(&mut editor);
        assert_eq!(ta.cursor().0, 1);
    }

    /// `gv` from Normal must repaint the highlight, not only put the engine
    /// back in Visual.
    #[test]
    fn vim_gv_repaints_the_last_selection() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("a\nb\nc".to_string());
        for c in ['V', 'j', '>', 'g', 'v'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        assert_eq!(vim_mode(&editor), EditorMode::VisualLine);
        assert!(
            editor.selection_for_test().is_some(),
            "gv must paint the highlight"
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

    // ── Direct-path modifier matrix (pins behaviour across the KeyInput move) ─

    /// A mapped key as the direct path receives it.
    fn ki(code: KeyCode, mods: KeyModifiers) -> KeyInput {
        crate::keys::crossterm_input::key_input(&ratatui::crossterm::event::KeyEvent::new(
            code, mods,
        ))
        .expect("mapped key")
    }

    /// One key through the public path.
    fn send(
        editor: &mut TextEditorComponent,
        tx: &AppTx,
        code: KeyCode,
        mods: KeyModifiers,
    ) -> EventState {
        editor.handle_input(&InputEvent::Key(key(code, mods)), tx)
    }

    #[test]
    fn a_key_with_no_key_input_ends_the_typing_run_and_is_consumed() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text(String::new());
        send(&mut editor, &tx, KeyCode::Char('a'), KeyModifiers::NONE);
        send(&mut editor, &tx, KeyCode::Char('b'), KeyModifiers::NONE);
        let state = send(&mut editor, &tx, KeyCode::Null, KeyModifiers::NONE);
        assert!(matches!(state, EventState::Consumed));
        send(&mut editor, &tx, KeyCode::Char('c'), KeyModifiers::NONE);
        assert_eq!(editor.get_text(), "abc");
        send(&mut editor, &tx, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(editor.get_text(), "ab", "the Null key ended the run");
    }

    #[test]
    fn meta_chord_on_the_direct_path_does_nothing() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("foo".to_string());
        let state = send(&mut editor, &tx, KeyCode::Char('x'), KeyModifiers::META);
        assert!(matches!(state, EventState::Consumed));
        assert_eq!(editor.get_text(), "foo");
        let state = send(&mut editor, &tx, KeyCode::Enter, KeyModifiers::META);
        assert!(matches!(state, EventState::Consumed));
        assert_eq!(editor.get_text(), "foo");
        select_range(&mut editor, (0, 0), (0, 3));
        let state = send(&mut editor, &tx, KeyCode::Char('('), KeyModifiers::META);
        assert!(matches!(state, EventState::Consumed));
        assert_eq!(editor.get_text(), "foo", "Meta+( must not wrap");
    }

    /// Deliberate deviation: Meta+Tab indented before the `KeyInput` split
    /// (the old guard only excluded Ctrl and Alt); a META chord has no
    /// `KeyInput` now, so the direct path ignores it like every other one.
    #[test]
    fn meta_tab_is_now_ignored() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("foo".to_string());
        let state = send(&mut editor, &tx, KeyCode::Tab, KeyModifiers::META);
        assert!(matches!(state, EventState::Consumed));
        assert_eq!(editor.get_text(), "foo");
    }

    #[test]
    fn meta_backtab_is_now_ignored() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("- foo".to_string());
        send(&mut editor, &tx, KeyCode::Tab, KeyModifiers::NONE);
        let indented = editor.get_text();
        assert_ne!(indented, "- foo", "Tab indents the list line");
        for (code, mods) in [
            (KeyCode::BackTab, KeyModifiers::META),
            (KeyCode::BackTab, KeyModifiers::SHIFT | KeyModifiers::META),
            (KeyCode::Tab, KeyModifiers::SHIFT | KeyModifiers::META),
        ] {
            let state = send(&mut editor, &tx, code, mods);
            assert!(matches!(state, EventState::Consumed));
            assert_eq!(
                editor.get_text(),
                indented,
                "{code:?}+{mods:?} must not dedent"
            );
        }
    }

    #[test]
    fn clipboard_chords_need_exactly_ctrl() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut editor = make_editor();
        editor.set_text("foo".to_string());
        // No selection: a copy that fires says so, one that does not is silent.
        for mods in [
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            send(&mut editor, &tx, KeyCode::Char('c'), mods);
            assert!(rx.try_recv().is_err(), "{mods:?}+c must not copy");
        }
        send(&mut editor, &tx, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(
            matches!(rx.try_recv(), Ok(AppMessage::FlashMessage(_))),
            "Ctrl+c copies"
        );
        assert_eq!(editor.get_text(), "foo");
    }

    #[test]
    fn undo_redo_modifier_matrix() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text(String::new());
        send(&mut editor, &tx, KeyCode::Char('a'), KeyModifiers::NONE);
        send(&mut editor, &tx, KeyCode::Char('b'), KeyModifiers::NONE);
        assert_eq!(editor.get_text(), "ab");

        send(&mut editor, &tx, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(editor.get_text(), "", "Ctrl+z undoes");
        send(
            &mut editor,
            &tx,
            KeyCode::Char('Z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(editor.get_text(), "ab", "Ctrl+Shift+Z redoes");
        send(&mut editor, &tx, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(editor.get_text(), "");
        send(&mut editor, &tx, KeyCode::Char('y'), KeyModifiers::CONTROL);
        assert_eq!(editor.get_text(), "ab", "Ctrl+y redoes");

        // A shifted lowercase z is neither undo nor redo.
        send(
            &mut editor,
            &tx,
            KeyCode::Char('z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(editor.get_text(), "ab");
        // Alt+z is not undo.
        send(&mut editor, &tx, KeyCode::Char('z'), KeyModifiers::ALT);
        assert_eq!(editor.get_text(), "ab");
        // Plain z types.
        send(&mut editor, &tx, KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(editor.get_text(), "abz");
    }

    #[test]
    fn tab_matrix() {
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text("foo".to_string());
        send(&mut editor, &tx, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(editor.get_text(), "    foo", "Tab indents");
        send(&mut editor, &tx, KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(editor.get_text(), "foo", "BackTab dedents");
        send(&mut editor, &tx, KeyCode::Tab, KeyModifiers::NONE);
        send(&mut editor, &tx, KeyCode::Tab, KeyModifiers::SHIFT);
        assert_eq!(editor.get_text(), "foo", "Tab+Shift dedents");

        for mods in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            let state = send(&mut editor, &tx, KeyCode::Tab, mods);
            assert!(matches!(state, EventState::Consumed));
            assert_eq!(editor.get_text(), "foo", "{mods:?}+Tab must not indent");
        }
    }

    #[test]
    fn enter_needs_no_modifiers_for_smart_enter() {
        let mk = || {
            let mut editor = make_editor();
            editor.set_text("- foo".to_string());
            get_ta(&mut editor).move_cursor(CursorMove::End);
            editor
        };
        let tx = dummy_tx();
        let mut editor = mk();
        send(&mut editor, &tx, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(editor.get_text(), "- foo\n- ", "Enter continues the list");
        let mut editor = mk();
        send(&mut editor, &tx, KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(
            editor.get_text(),
            "- foo\n",
            "Shift+Enter is a plain newline"
        );
    }

    #[test]
    fn auto_surround_ignores_shift_but_not_ctrl_alt() {
        let tx = dummy_tx();
        let selected = || {
            let mut editor = make_editor();
            editor.set_text("foo".to_string());
            select_range(&mut editor, (0, 0), (0, 3));
            editor
        };
        let mut editor = selected();
        send(&mut editor, &tx, KeyCode::Char('('), KeyModifiers::SHIFT);
        assert_eq!(editor.get_text(), "(foo)", "Shift+( wraps");
        let mut editor = selected();
        send(&mut editor, &tx, KeyCode::Char('('), KeyModifiers::NONE);
        assert_eq!(editor.get_text(), "(foo)", "( wraps");
        for mods in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
            let mut editor = selected();
            send(&mut editor, &tx, KeyCode::Char('('), mods);
            assert_ne!(editor.get_text(), "(foo)", "{mods:?}+( must not wrap");
        }
    }

    // ── Undo grouping ────────────────────────────────────────────────────────

    fn type_out(editor: &mut TextEditorComponent, tx: &AppTx, text: &str) {
        for c in text.chars() {
            let code = if c == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(c)
            };
            editor.handle_textarea_key(ki(code, KeyModifiers::NONE), tx);
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
        let mut editor = make_editor();
        let tx = dummy_tx();
        editor.set_text(String::new());
        type_out(&mut editor, &tx, "helllo");
        editor.handle_textarea_key(ki(KeyCode::Backspace, KeyModifiers::NONE), &tx);
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
        editor.handle_textarea_key(ki(code, KeyModifiers::NONE), tx);
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
        // The App loop (`app::run_app`) drains queued input without redrawing between events, so an
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
        editor.handle_textarea_key(ki(KeyCode::Down, KeyModifiers::SHIFT), &tx);
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
            BackendState::Textarea(ed) => match &ed.buf.input {
                crate::logic::editor::textarea::InputInterpreter::Vim(e) => e.mode().clone(),
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

    /// Linewise Visual covers whole rows, so a paste from outside the key
    /// path replaces all of them — not the anchor-to-cursor columns the
    /// buffer's selection happens to span.
    #[test]
    fn external_paste_over_visual_line_replaces_the_whole_rows() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("a\nbb\ncc\nd".to_string());
        for c in ['j', 'l', 'V', 'j'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        editor.take_selection_for_external_paste();
        assert_eq!(editor.get_text(), "a\n\nd");
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
    }

    /// A terminal paste over `V` rows replaces them, as Ctrl+V does.
    #[test]
    fn bracketed_paste_over_visual_line_replaces_the_whole_rows() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("a\nbb\ncc\nd".to_string());
        for c in ['j', 'l', 'V', 'j'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        editor.paste_text("X", &tx);
        assert_eq!(editor.get_text(), "a\nX\nd");
    }

    /// Charwise: the char under the cursor is inside the selection (vim), so
    /// it is inside the wrap.
    #[test]
    fn surround_over_charwise_visual_includes_the_cursor_char() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("hello world".to_string());
        for c in ['v', 'e', '*'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        assert_eq!(editor.get_text(), "*hello* world");
    }

    /// `v` on a multi-codepoint emoji then `*` wraps the whole emoji — not
    /// half of it, and not a literal `*` typed in Normal.
    #[test]
    fn surround_over_an_emoji_wraps_all_of_it() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("a❤️b".to_string());
        for c in ['l', 'v', '*'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        assert_eq!(editor.get_text(), "a*❤️*b");
    }

    /// The charwise highlight covers the whole character under the cursor.
    #[test]
    fn visual_highlight_covers_the_whole_emoji() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("a👍🏽b".to_string());
        for c in ['l', 'v'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        assert_eq!(editor.selection_for_test(), Some(((0, 1), (0, 3))));
    }

    /// A terminal paste over a Visual selection ends Visual: afterwards `x`
    /// is a Normal-mode `x`, not an operator on an empty selection that takes
    /// the whole line.
    #[test]
    fn bracketed_paste_over_visual_returns_to_normal() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("keep\nabc".to_string());
        press(&mut editor, "jV");
        editor.paste_text("xyz", &tx);
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
        assert_eq!(editor.get_text(), "keep\nxyz");
        press(&mut editor, "0x");
        assert_eq!(editor.get_text(), "keep\nyz");
    }

    /// A wrap typed in Insert (over a mouse selection) stays in Insert —
    /// only a Visual hand-off becomes a Visual selection.
    #[test]
    fn a_surround_typed_in_insert_stays_in_insert() {
        let mut editor = make_vim_editor();
        editor.set_text("hello world".to_string());
        press(&mut editor, "i");
        {
            let ta = get_ta(&mut editor);
            ta.move_cursor(CursorMove::Jump(0, 0));
            ta.start_selection();
            ta.move_cursor(CursorMove::Jump(0, 5));
        }
        press(&mut editor, "(");
        assert_eq!(editor.get_text(), "(hello) world");
        assert_eq!(vim_mode(&editor), EditorMode::Insert);
    }

    fn mouse(kind: ratatui::crossterm::event::MouseEventKind, column: u16) -> InputEvent {
        InputEvent::Mouse(ratatui::crossterm::event::MouseEvent {
            kind,
            column,
            row: 0,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn rendered_vim_editor(text: &str) -> TextEditorComponent {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut editor = make_vim_editor();
        editor.set_text(text.to_string());
        let mut term = Terminal::new(TestBackend::new(30, 4)).unwrap();
        term.draw(|f| editor.render(f, Rect::new(0, 0, 30, 4), &Theme::default(), true))
            .unwrap();
        editor
    }

    /// A selectionless right-click asks the host for the context menu, once;
    /// with a selection it copies instead and asks for nothing.
    #[test]
    fn right_click_without_a_selection_requests_the_context_menu_once() {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        let tx = dummy_tx();
        let mut editor = rendered_vim_editor("hello world");
        editor.handle_input(&mouse(MouseEventKind::Down(MouseButton::Right), 3), &tx);
        assert_eq!(editor.take_requests(), [EditorRequest::ContextMenu]);
        assert_eq!(editor.take_requests(), []);

        let mut editor = rendered_vim_editor("hello world");
        press(&mut editor, "ve");
        editor.handle_input(&mouse(MouseEventKind::Down(MouseButton::Right), 1), &tx);
        assert!(!editor.take_requests().contains(&EditorRequest::ContextMenu));
    }

    /// Right-click in Visual is Ctrl-C: same copy, and the same return to
    /// Normal — for `v` and for `V`, an empty line included.
    #[test]
    fn right_click_in_visual_copies_like_ctrl_c() {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        let tx = dummy_tx();
        for (text, keys) in [("hello world", "ve"), ("hello", "V"), ("\nx", "V")] {
            let mut editor = rendered_vim_editor(text);
            press(&mut editor, keys);
            editor.handle_input(&mouse(MouseEventKind::Down(MouseButton::Right), 1), &tx);
            assert_eq!(
                vim_mode(&editor),
                EditorMode::Normal,
                "after `{keys}` on {text:?}"
            );
            assert_eq!(editor.selection_for_test(), None);
        }
    }

    /// The mouse never looked at META/HYPER: those bits must not change what
    /// a click does.
    #[test]
    fn meta_modified_mouse_events_act_like_unmodified_ones() {
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let tx = dummy_tx();
        for modifiers in [KeyModifiers::META, KeyModifiers::HYPER] {
            let ev = |kind, column| {
                InputEvent::Mouse(MouseEvent {
                    kind,
                    column,
                    row: 0,
                    modifiers,
                })
            };
            let mut editor = rendered_vim_editor("hello world");
            editor.handle_input(&ev(MouseEventKind::Down(MouseButton::Right), 3), &tx);
            assert_eq!(
                editor.take_requests(),
                [EditorRequest::ContextMenu],
                "right-click {modifiers:?}"
            );

            let mut editor = rendered_vim_editor("hello world");
            editor.handle_input(&ev(MouseEventKind::Down(MouseButton::Left), 4), &tx);
            assert_eq!(editor.textarea_cursor(), Some((0, 4)), "left {modifiers:?}");
        }
    }

    /// Pointer events with no `PointerInput` (horizontal scroll) inside the editor are still the editor's: they are consumed as
    /// an action that changes nothing, and never open the context menu.
    #[test]
    fn an_ignored_pointer_event_inside_is_consumed_and_changes_nothing() {
        use ratatui::crossterm::event::{MouseEvent, MouseEventKind};
        let tx = dummy_tx();
        let events = [
            (MouseEventKind::ScrollLeft, KeyModifiers::NONE),
            (MouseEventKind::ScrollRight, KeyModifiers::NONE),
        ];
        for (kind, modifiers) in events {
            let mut editor = rendered_vim_editor("hello world");
            let cursor = editor.textarea_cursor();
            let ev = InputEvent::Mouse(MouseEvent {
                kind,
                column: 3,
                row: 0,
                modifiers,
            });
            assert_eq!(
                editor.handle_input(&ev, &tx),
                EventState::Consumed,
                "{kind:?} {modifiers:?}"
            );
            assert_eq!(editor.textarea_cursor(), cursor, "{kind:?} {modifiers:?}");
            assert_eq!(editor.selection_for_test(), None);
            assert!(editor.take_requests().is_empty(), "{kind:?} {modifiers:?}");
        }
    }

    /// A drag selects what was dragged over: the highlight, and what `d`
    /// takes, end where the mouse stopped — forwards and backwards.
    #[test]
    fn a_drag_selects_exactly_what_was_dragged() {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        let tx = dummy_tx();
        for (from, to) in [(0, 5), (5, 0)] {
            let mut editor = rendered_vim_editor("hello world");
            editor.handle_input(&mouse(MouseEventKind::Down(MouseButton::Left), from), &tx);
            editor.handle_input(&mouse(MouseEventKind::Drag(MouseButton::Left), to), &tx);
            assert_eq!(vim_mode(&editor), EditorMode::Visual);
            assert_eq!(
                editor.selection_for_test(),
                Some(((0, 0), (0, 5))),
                "drag {from}→{to}"
            );
            press(&mut editor, "d");
            assert_eq!(editor.get_text(), " world", "drag {from}→{to}");
        }
    }

    /// A left click ends Visual, as it always did: the click's empty
    /// selection is not a one-character (or one-row) Visual selection.
    #[test]
    fn a_click_leaves_visual() {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        let tx = dummy_tx();
        for keys in ["vll", "V"] {
            let mut editor = rendered_vim_editor("hello world");
            press(&mut editor, keys);
            editor.handle_input(&mouse(MouseEventKind::Down(MouseButton::Left), 8), &tx);
            assert_eq!(vim_mode(&editor), EditorMode::Normal, "after `{keys}`");
            press(&mut editor, "x");
            assert_eq!(
                editor.get_text(),
                "hello wold",
                "after `{keys}`: a plain `x`"
            );
        }
    }

    /// The click also leaves `gv` the selection it ended.
    #[test]
    fn gv_after_a_click_reselects_what_the_click_ended() {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        let tx = dummy_tx();
        let mut editor = rendered_vim_editor("hello world");
        press(&mut editor, "vll");
        editor.handle_input(&mouse(MouseEventKind::Down(MouseButton::Left), 8), &tx);
        press(&mut editor, "gv");
        assert_eq!(vim_mode(&editor), EditorMode::Visual);
        assert_eq!(editor.selection_for_test(), Some(((0, 0), (0, 3))));
    }

    /// Scrolling is not a selection gesture: Visual survives it, even over
    /// an empty row, where the Visual range itself is empty.
    #[test]
    fn scrolling_keeps_visual() {
        use ratatui::crossterm::event::MouseEventKind;
        let tx = dummy_tx();
        for keys in ["jV", "jv", "ve"] {
            let mut editor = rendered_vim_editor("a\n\nb");
            press(&mut editor, keys);
            let mode = vim_mode(&editor);
            editor.handle_input(&mouse(MouseEventKind::ScrollDown, 1), &tx);
            assert_eq!(vim_mode(&editor), mode, "after `{keys}`");
        }
    }

    /// A drag with no press in this editor behind it — one that began
    /// elsewhere, or before another note was opened — selects nothing.
    #[test]
    fn a_drag_without_its_own_press_selects_nothing() {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        let tx = dummy_tx();
        let mut editor = rendered_vim_editor("hello world");
        editor.open_note(note("alpha"), "hello world".to_string());
        editor.handle_input(&mouse(MouseEventKind::Down(MouseButton::Left), 2), &tx);
        editor.open_note(note("beta"), "other text".to_string());
        editor.handle_input(&mouse(MouseEventKind::Drag(MouseButton::Left), 8), &tx);
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
        assert!(
            get_ta(&mut editor)
                .selection_range()
                .is_none_or(|(s, e)| s == e)
        );

        let mut editor = rendered_vim_editor("hello world");
        editor.handle_input(&mouse(MouseEventKind::Down(MouseButton::Left), 2), &tx);
        editor.handle_input(&mouse(MouseEventKind::Up(MouseButton::Left), 2), &tx);
        get_ta(&mut editor).cancel_selection();
        editor.handle_input(&mouse(MouseEventKind::Drag(MouseButton::Left), 8), &tx);
        assert_eq!(
            vim_mode(&editor),
            EditorMode::Normal,
            "a drag after the release"
        );
    }

    /// A drag over one character selects that character, and stays Visual.
    #[test]
    fn a_one_character_drag_selects_it() {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        let tx = dummy_tx();
        let mut editor = rendered_vim_editor("hello");
        editor.handle_input(&mouse(MouseEventKind::Down(MouseButton::Left), 1), &tx);
        editor.handle_input(&mouse(MouseEventKind::Drag(MouseButton::Left), 2), &tx);
        assert_eq!(vim_mode(&editor), EditorMode::Visual);
        assert_eq!(editor.selection_for_test(), Some(((0, 1), (0, 2))));
        press(&mut editor, "d");
        assert_eq!(editor.get_text(), "hllo");
    }

    /// Dragging back and forth never loses a character: each event selects
    /// from where the drag began.
    #[test]
    fn a_drag_that_turns_back_selects_from_its_origin() {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        let tx = dummy_tx();
        let mut editor = rendered_vim_editor("hello world");
        editor.handle_input(&mouse(MouseEventKind::Down(MouseButton::Left), 6), &tx);
        for col in [3, 1, 0, 9, 11] {
            editor.handle_input(&mouse(MouseEventKind::Drag(MouseButton::Left), col), &tx);
        }
        assert_eq!(editor.selection_for_test(), Some(((0, 6), (0, 11))));
        press(&mut editor, "d");
        assert_eq!(editor.get_text(), "hello ");
    }

    /// Auto-surround over a `V` row wraps the row, matching the highlight.
    #[test]
    fn surround_over_visual_line_wraps_the_whole_row() {
        let mut editor = make_vim_editor();
        let tx = dummy_tx();
        editor.set_text("a\nbold\nc".to_string());
        for c in ['j', 'l', 'V', '*'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        assert_eq!(editor.get_text(), "a\n*bold*\nc");
    }

    /// Copying a Visual selection from the host (right-click) is the Ctrl-C
    /// chord: it covers the inclusive range the highlight shows ("hello", see
    /// `copy_visual_takes_the_inclusive_range_and_leaves_visual`) and leaves
    /// Visual. That also settles the old drift, where each repeated copy grew
    /// the live selection one char: after the first copy there is none left
    /// to grow.
    #[test]
    fn vim_visual_copy_leaves_visual_and_nothing_to_grow() {
        let mut editor = make_vim_editor();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        editor.set_text("hello world".to_string());
        press(&mut editor, "ve");
        assert_eq!(
            editor.backend.selection_as_shown(),
            Some(((0, 0), (0, 5))),
            "the copy reads the inclusive range including the cursor char"
        );
        editor.copy_selection_to_clipboard(&tx);
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
        assert_eq!(get_ta(&mut editor).selection_range(), None);
        editor.copy_selection_to_clipboard(&tx);
        assert_eq!(get_ta(&mut editor).selection_range(), None);
        assert_eq!(
            get_ta(&mut editor).cursor(),
            (0, 0),
            "at the copied range's start"
        );
    }

    /// Regression: `V` (linewise Visual) must keep the WHOLE line highlighted
    /// no matter where the cursor moves within it afterwards — column position
    /// is irrelevant to a linewise selection in vim. the painted selection used to
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
            editor.selection_for_test(),
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
            editor.selection_for_test(),
            Some(((0, 0), (0, usize::MAX))),
            "moving the cursor back must not shrink the linewise highlight"
        );
    }

    /// Regression: a Visual `<` with nothing to remove (the lines are already
    /// flush left) still cancels the buffer selection and drops to Normal, so
    /// the painted highlight must go with it. The engine reported the
    /// no-change case as `NoOp`, and the host's `NoOp` arm returns without
    /// touching the painted selection — leaving the rows painted as selected after
    /// Visual mode had already ended.
    #[test]
    fn vim_visual_outdent_with_nothing_to_remove_clears_the_highlight() {
        let mut editor = make_vim_editor();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        editor.set_text("alpha\nbeta".to_string());
        for c in ['V', 'j'] {
            editor.handle_input(
                &InputEvent::Key(key(KeyCode::Char(c), KeyModifiers::NONE)),
                &tx,
            );
        }
        assert_eq!(vim_mode(&editor), EditorMode::VisualLine);
        assert!(
            editor.selection_for_test().is_some(),
            "V over two rows must paint a highlight"
        );

        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('<'), KeyModifiers::NONE)),
            &tx,
        );
        assert_eq!(
            editor.get_text(),
            "alpha\nbeta",
            "there is no indentation to remove"
        );
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
        assert_eq!(
            editor.selection_for_test(),
            None,
            "leaving Visual mode must clear the highlight even when the outdent changed nothing"
        );
    }

    /// Regression: a bare left click (Down with no Drag) must NOT flip
    /// vim Normal → Visual. The Down arm calls `start_selection()`, which
    /// leaves a collapsed (start==end) selection; the mode is reconciled by
    /// gesture (`settle_pointer(true)` for a left press), not by that shape.
    ///
    /// Driven through `Editor::handle_pointer` + `Editor::settle_pointer`, the
    /// exact pair the component's mouse arm runs, on a frame the editor
    /// prepared itself so screen cells resolve to logical positions.
    #[test]
    fn vim_sync_collapsed_sel_stays_normal() {
        use crate::logic::input::{Button, Mods, PointerInput, PointerKind};
        let mut editor = make_vim_editor();
        editor.set_text("hello world".to_string());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Sanity: starts in Normal.
        assert_eq!(vim_mode(&editor), EditorMode::Normal);

        // A bare click: Down with no Drag leaves a collapsed selection.
        let ed = editor.backend.as_editor_mut().expect("textarea backend");
        ed.prepare_frame(40, 5);
        let down = PointerInput {
            kind: PointerKind::Down(Button::Left),
            col: 3,
            row: 0,
            mods: Mods::NONE,
        };
        ed.handle_pointer(&down, &tx);
        ed.settle_pointer(true);
        assert_eq!(
            vim_mode(&editor),
            EditorMode::Normal,
            "collapsed (bare click) selection must not enter Visual mode"
        );
    }

    /// A drag that creates a real (non-empty) selection DOES enter Visual mode.
    #[test]
    fn vim_sync_real_sel_enters_visual() {
        use crate::logic::input::{Button, Mods, PointerInput, PointerKind};
        let mut editor = make_vim_editor();
        editor.set_text("hello world".to_string());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        // Sanity: starts in Normal.
        assert_eq!(vim_mode(&editor), EditorMode::Normal);

        let ed = editor.backend.as_editor_mut().expect("textarea backend");
        ed.prepare_frame(40, 5);
        let at = |kind, col| PointerInput {
            kind,
            col,
            row: 0,
            mods: Mods::NONE,
        };
        ed.handle_pointer(&at(PointerKind::Down(Button::Left), 0), &tx);
        ed.settle_pointer(true);
        ed.handle_pointer(&at(PointerKind::Drag(Button::Left), 5), &tx);
        ed.settle_pointer(false);
        assert_eq!(
            vim_mode(&editor),
            EditorMode::Visual,
            "real drag selection must enter Visual mode"
        );
    }

    /// Vim `:` asks for the palette (`EditorRequest::OpenPalette`); the
    /// component forwards it to the app as the palette leader action.
    #[test]
    fn vim_colon_forwards_the_palette_request_to_the_app() {
        use crate::keys::leader::LeaderAction;
        use crate::logic::message::AppMessage;
        let mut editor = make_vim_editor();
        editor.set_text("hello".to_string());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char(':'), KeyModifiers::NONE)),
            &tx,
        );
        let mut got_palette = false;
        while let Ok(message) = rx.try_recv() {
            got_palette |= matches!(
                message,
                AppMessage::ExecuteLeaderAction(LeaderAction::Palette)
            );
        }
        assert!(got_palette, "`:` must reach the app as the palette action");
        assert_eq!(editor.get_text(), "hello");
    }

    /// A key the find bar takes never reaches `Editor::handle_key`, which is
    /// what re-attaches a wheel-scrolled view; the component's own guard must
    /// do it, or the view stays detached while the user types a query.
    #[test]
    fn a_find_bar_key_reattaches_a_wheel_scrolled_view() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut editor = make_editor();
        let long = (0..60)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        editor.set_text(long);
        let tx = dummy_tx();
        let theme = Theme::default();
        let mut t = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let mut draw = |editor: &mut TextEditorComponent| {
            t.draw(|f| editor.render(f, f.area(), &theme, true))
                .unwrap();
        };
        draw(&mut editor);
        assert!(editor.session_mut().display.scroll_by(20));
        draw(&mut editor);
        assert_ne!(
            editor.session().display.click_at_screen(0, 0).0,
            0,
            "detached"
        );
        editor.open_or_advance_search();
        draw(&mut editor);
        // 'q' matches nothing, so the cursor stays on row 0.
        editor.handle_input(
            &InputEvent::Key(key(KeyCode::Char('q'), KeyModifiers::NONE)),
            &tx,
        );
        draw(&mut editor);
        assert_eq!(editor.get_text().lines().count(), 60);
        assert_eq!(editor.session().display.click_at_screen(0, 0).0, 0);
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

    // ── Input stage pins (the key/mouse flow moving into `Editor`) ──────────

    /// A popup controller over a fake tag source that knows `projects`, with
    /// no debounce, so a test can open the popup by typing `#pro`.
    fn tag_popup_controller() -> AutocompleteController {
        use crate::components::search_list::{SuggestionItem, SuggestionSource};
        struct Tags;
        #[async_trait::async_trait]
        impl SuggestionSource for Tags {
            async fn notes_by_prefix(&self, _p: &str, _n: usize) -> Vec<SuggestionItem> {
                vec![]
            }
            async fn tags_by_prefix(&self, p: &str, _n: usize) -> Vec<SuggestionItem> {
                if "projects".starts_with(p) {
                    vec![SuggestionItem::plain("projects")]
                } else {
                    vec![]
                }
            }
        }
        AutocompleteController::new(std::sync::Arc::new(Tags), AutocompleteMode::Both)
            .with_debounce(std::time::Duration::ZERO)
    }

    /// Render `editor` into a 60x20 terminal until its popup has results (or
    /// give up after a bounded number of yields).
    async fn draw_until_popup_open(
        editor: &mut TextEditorComponent,
        t: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
    ) {
        let theme = Theme::default();
        for _ in 0..100 {
            t.draw(|f| editor.render(f, f.area(), &theme, true))
                .unwrap();
            if editor.autocomplete.as_ref().is_some_and(|c| c.is_open()) {
                return;
            }
            tokio::task::yield_now().await;
        }
    }

    /// Vim `/` opens the find bar within the very `handle_input` call that
    /// carried it, and `n` / `N` move the painted match in theirs — the
    /// requests the editor queues are drained before the call returns.
    #[test]
    fn vim_search_keys_open_and_repeat_the_find_bar_in_the_same_call() {
        let mut editor = make_vim_editor();
        editor.set_text("lo xx lo yy lo".to_string());
        let tx = dummy_tx();
        send(&mut editor, &tx, KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(editor.search.is_some(), "`/` opens the bar in its own call");
        send(&mut editor, &tx, KeyCode::Char('l'), KeyModifiers::NONE);
        send(&mut editor, &tx, KeyCode::Char('o'), KeyModifiers::NONE);
        assert_eq!(
            editor.search.as_ref().unwrap().current_match(),
            Some(((0, 0), (0, 2)))
        );
        send(&mut editor, &tx, KeyCode::Esc, KeyModifiers::NONE);
        assert!(editor.search.is_none());

        send(&mut editor, &tx, KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(editor.cursor_pos(), (0, 6), "`n` steps forward");
        assert_eq!(editor.selection_for_test(), Some(((0, 6), (0, 8))));
        send(&mut editor, &tx, KeyCode::Char('n'), KeyModifiers::NONE);
        assert_eq!(editor.cursor_pos(), (0, 12));
        assert_eq!(editor.selection_for_test(), Some(((0, 12), (0, 14))));
        send(&mut editor, &tx, KeyCode::Char('N'), KeyModifiers::SHIFT);
        assert_eq!(editor.cursor_pos(), (0, 6), "`N` steps back");
        assert_eq!(editor.selection_for_test(), Some(((0, 6), (0, 8))));
        assert_eq!(editor.get_text(), "lo xx lo yy lo");
    }

    /// The wheel moves the viewport away from the cursor the popup is anchored
    /// to, so a scroll that moves the view closes the popup.
    #[tokio::test]
    async fn wheel_scroll_closes_the_autocomplete_popup() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut editor = make_editor();
        editor.autocomplete = Some(tag_popup_controller());
        let text: String = std::iter::once(String::new())
            .chain((0..60).map(|i| format!("line {i}")))
            .collect::<Vec<_>>()
            .join("\n");
        editor.set_text(text);
        let tx = dummy_tx();
        let mut t = Terminal::new(TestBackend::new(60, 20)).unwrap();
        t.draw(|f| editor.render(f, f.area(), &Theme::default(), true))
            .unwrap();
        for c in ['#', 'p', 'r', 'o'] {
            send(&mut editor, &tx, KeyCode::Char(c), KeyModifiers::NONE);
        }
        draw_until_popup_open(&mut editor, &mut t).await;
        assert!(
            editor.autocomplete.as_ref().unwrap().is_open(),
            "precondition: the popup is open"
        );
        let wheel = InputEvent::Mouse(ratatui::crossterm::event::MouseEvent {
            kind: ratatui::crossterm::event::MouseEventKind::ScrollDown,
            column: 50,
            row: 15,
            modifiers: KeyModifiers::NONE,
        });
        let state = editor.handle_input(&wheel, &tx);
        assert!(matches!(state, EventState::Consumed));
        assert!(
            !editor.autocomplete.as_ref().unwrap().is_open(),
            "a scroll that moved the view closes the popup"
        );
        assert_eq!(
            editor.cursor_pos(),
            (0, 4),
            "the wheel never moves the cursor"
        );
    }

    /// A cursor-only vim key re-reads an open popup's trigger: a motion inside
    /// the tag narrows the query, a motion out of it closes the popup.
    #[tokio::test]
    async fn vim_normal_motion_refreshes_an_open_popup() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut editor = make_vim_editor();
        editor.autocomplete = Some(tag_popup_controller());
        editor.set_text("#pro".to_string());
        let mut t = Terminal::new(TestBackend::new(60, 20)).unwrap();
        t.draw(|f| editor.render(f, f.area(), &Theme::default(), true))
            .unwrap();
        // Normal mode on the `o`; nothing a vim key does in Normal opens a
        // popup, so open it the way a text change would.
        press(&mut editor, "lll");
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
        assert_eq!(editor.cursor_pos(), (0, 3));
        editor.sync_autocomplete();
        draw_until_popup_open(&mut editor, &mut t).await;
        assert!(
            editor.autocomplete.as_ref().unwrap().is_open(),
            "precondition: the popup is open"
        );
        assert_eq!(
            editor.autocomplete.as_ref().unwrap().state().unwrap().query,
            "pr"
        );

        press(&mut editor, "h");
        assert_eq!(editor.cursor_pos(), (0, 2));
        assert_eq!(
            editor
                .autocomplete
                .as_ref()
                .unwrap()
                .state()
                .map(|s| s.query.clone()),
            Some("p".to_string()),
            "a motion inside the tag refreshes the query"
        );
        press(&mut editor, "0");
        assert!(
            editor.autocomplete.as_ref().unwrap().state().is_none(),
            "a motion out of the tag closes the popup"
        );
    }

    /// Vim Visual `Ctrl+V` replaces the selection with the clipboard in one
    /// edit (one undo brings it back), and never destroys it when the
    /// clipboard read fails or is empty. The clipboard is injected, so the
    /// test is hermetic.
    #[test]
    fn ctrl_v_in_vim_visual_replaces_the_selection_atomically() {
        let mut editor = make_vim_editor();
        editor.set_text("hello world".to_string());
        let tx = dummy_tx();
        press(&mut editor, "ve");
        assert_eq!(vim_mode(&editor), EditorMode::Visual);
        {
            // A failed read destroys nothing (a fresh editor: the failed
            // paste leaves Visual).
            let mut failing = make_vim_editor();
            failing.set_text("hello world".to_string());
            press(&mut failing, "ve");
            let _clip =
                crate::logic::clipboard::set_test_clipboard_text(Err("no clipboard".into()));
            send(&mut failing, &tx, KeyCode::Char('v'), KeyModifiers::CONTROL);
            assert_eq!(
                failing.get_text(),
                "hello world",
                "a failed read destroys nothing"
            );
        }
        let _clip = crate::logic::clipboard::set_test_clipboard_text(Ok("pasted".into()));
        send(&mut editor, &tx, KeyCode::Char('v'), KeyModifiers::CONTROL);
        assert_eq!(
            editor.get_text(),
            "pasted world",
            "the whole selected word is replaced"
        );
        assert_ne!(vim_mode(&editor), EditorMode::Visual);
        press(&mut editor, "u");
        assert_eq!(editor.get_text(), "hello world", "one undo restores it");
    }

    /// A vim Visual paste that lands a link trigger re-syncs the popup
    /// (`KeyPath::VimPasted`): `#pro` pasted over a selection opens it.
    #[tokio::test]
    async fn vim_visual_paste_of_a_link_trigger_resyncs_the_popup() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut editor = make_vim_editor();
        editor.autocomplete = Some(tag_popup_controller());
        editor.set_text("hello world".to_string());
        let tx = dummy_tx();
        let mut t = Terminal::new(TestBackend::new(60, 20)).unwrap();
        t.draw(|f| editor.render(f, f.area(), &Theme::default(), true))
            .unwrap();
        press(&mut editor, "ve");
        assert_eq!(vim_mode(&editor), EditorMode::Visual);
        assert!(!editor.autocomplete.as_ref().unwrap().is_open());
        let _clip = crate::logic::clipboard::set_test_clipboard_text(Ok("#pro".into()));
        send(&mut editor, &tx, KeyCode::Char('v'), KeyModifiers::CONTROL);
        assert_eq!(editor.get_text(), "#pro world");
        draw_until_popup_open(&mut editor, &mut t).await;
        let ctl = editor.autocomplete.as_ref().unwrap();
        assert!(ctl.is_open(), "the paste re-synced the popup");
        assert_eq!(ctl.state().unwrap().query, "pro");
    }

    /// Pins that a direct paste re-syncs the popup.
    #[tokio::test]
    async fn ctrl_v_on_the_direct_path_resyncs_the_popup() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut editor = make_editor();
        editor.autocomplete = Some(tag_popup_controller());
        editor.set_text(String::new());
        let tx = dummy_tx();
        let mut t = Terminal::new(TestBackend::new(60, 20)).unwrap();
        t.draw(|f| editor.render(f, f.area(), &Theme::default(), true))
            .unwrap();
        assert!(!editor.autocomplete.as_ref().unwrap().is_open());
        let _clip = crate::logic::clipboard::set_test_clipboard_text(Ok("#pro".into()));
        send(&mut editor, &tx, KeyCode::Char('v'), KeyModifiers::CONTROL);
        assert_eq!(editor.get_text(), "#pro");
        draw_until_popup_open(&mut editor, &mut t).await;
        let ctl = editor.autocomplete.as_ref().unwrap();
        assert!(ctl.is_open(), "the paste re-synced the popup");
        assert_eq!(ctl.state().unwrap().query, "pro");
    }
}
