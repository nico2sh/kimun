//! The textarea editor, terminal-free: the rope buffer and its input
//! interpreter, the per-frame [`EditorSession`], and the state only the
//! textarea keeps between events (the painted selection, the drag origin, the
//! vim Insert-session bracket, and the asks queued for the host).
//!
//! A front end owns one `Editor` per open editor pane and draws it through
//! `session.display`. See ADR-0049.

use crate::logic::editor::claim::{
    ClipboardImage, FollowTarget, WHEEL_SCROLL_ROWS, follow_target_in_line, linkable_url,
    try_build_markdown_link,
};
use crate::logic::editor::find_bar::{FindBar, MatchPosition};
use crate::logic::editor::handled::{Handled, KeyPath};
use crate::logic::editor::request::EditorRequest;
use crate::logic::editor::rope_buffer::{CursorMove, RopeBuffer};
use crate::logic::editor::session::{EditorSession, FrameInput};
use crate::logic::editor::snapshot::EditorSnapshot;
use crate::logic::editor::textarea::TextareaBackend;
use crate::logic::editor::{markdown_edits, plain_keys, typing_run};
use crate::logic::input::{Button, Key, KeyInput, Mods, PointerInput, PointerKind};
use crate::logic::message::{AppMessage, AppTx};
use crate::logic::text_action::TextAction;
use std::num::NonZeroU64;

/// The textarea editor: buffer, interpreter, session and textarea-only state.
pub struct Editor {
    /// The rope buffer, its typing run and the input interpreter.
    pub(crate) buf: TextareaBackend,
    /// The open note, revision clock, needles, display and background jobs.
    session: EditorSession,
    /// Where the current mouse drag started, so each drag event rebuilds the
    /// selection from it rather than from a selection the last event reshaped.
    drag_origin: Option<(usize, usize)>,
    /// Current selection range in logical (row, byte-col) coordinates — what
    /// the renderer paints as the selection.
    selection: Option<((usize, usize), (usize, usize))>,
    /// Whether the last key was handled while vim was in Insert. A change either
    /// way ends the open **undo group**: leaving Insert closes vim's session,
    /// entering it starts a fresh one.
    last_insert_session: bool,
    /// Asks for the host to act on (a right-click with no selection wants the
    /// note's context menu); drained by [`Editor::take_requests`].
    requests: Vec<EditorRequest>,
    /// The open **find bar**, if any. While it is open it owns every key
    /// ([`Editor::handle_find_key`]) and its **current match** is what gets
    /// painted as the selection ([`Editor::painted_selection`]).
    find: Option<FindBar>,
}

impl Editor {
    /// An empty editor: the built-in vim emulation when `vim`, direct typing
    /// otherwise.
    pub fn new(vim: bool) -> Self {
        Self::from_session(crate::ropetext::Text::new(), vim, EditorSession::new())
    }

    /// A textarea editor over `text` that takes over `session` — the note,
    /// revision clock, needles, display, jobs and notifier of whatever engine
    /// held it before. Used by the nvim death recovery.
    pub fn from_session(text: crate::ropetext::Text, vim: bool, session: EditorSession) -> Self {
        let buf = if vim {
            TextareaBackend::vim(text)
        } else {
            TextareaBackend::direct(text)
        };
        Self {
            buf,
            session,
            drag_origin: None,
            selection: None,
            last_insert_session: false,
            requests: Vec::new(),
            find: None,
        }
    }

    /// The editor's session: the open note, revision clock, needles, display,
    /// background jobs and notifier.
    pub fn session(&self) -> &EditorSession {
        &self.session
    }

    /// Mutable access to the session, for the host that prepares frames and
    /// installs the notifier.
    pub fn session_mut(&mut self) -> &mut EditorSession {
        &mut self.session
    }

    /// The rope buffer the editor edits.
    pub fn buffer(&self) -> &RopeBuffer {
        &self.buf.ta
    }

    /// The rope buffer, mutably. A caller that edits through it (the
    /// autocomplete accept) must settle the edit afterwards
    /// ([`Editor::after_edit`]) so the revision clock and the parse damage
    /// follow. The find bar is the editor's own and is settled internally.
    pub fn buffer_mut(&mut self) -> &mut RopeBuffer {
        &mut self.buf.ta
    }

    // ---- Queries -----------------------------------------------------------

    /// The painted selection, in logical (row, byte-col) coordinates.
    pub fn selection(&self) -> Option<((usize, usize), (usize, usize))> {
        self.selection
    }

    /// Paint `selection` as the selection — for a host layer that moved the
    /// cursor onto something it wants shown (a search match), or cleared it.
    pub fn set_selection(&mut self, selection: Option<((usize, usize), (usize, usize))>) {
        self.selection = selection;
    }

    /// Where the current mouse drag started, for assertions.
    #[cfg(test)]
    pub(crate) fn drag_origin(&self) -> Option<(usize, usize)> {
        self.drag_origin
    }

    /// Whether the last key was handled in vim Insert, for assertions.
    #[cfg(test)]
    pub(crate) fn last_insert_session(&self) -> bool {
        self.last_insert_session
    }

    /// The open note's text.
    pub fn text(&self) -> crate::ropetext::Text {
        self.buf.ta.text().clone()
    }

    /// Single producer for the editor's atomic `(lines, cursor,
    /// content_revision)` view. Downstream consumers (`MarkdownEditorView`,
    /// `click_to_logical_u16`, the autocomplete host) take a
    /// `&EditorSnapshot` and stop guarding against drift between cursor
    /// and lines on every leaf access — the snapshot owns that
    /// invariant at construction time.
    ///
    /// The snapshot borrows live lines (no clone) and the cursor is already
    /// in-bounds.
    pub fn view_snapshot(&self) -> EditorSnapshot {
        let cursor = self.buf.ta.cursor();
        EditorSnapshot::of_buffer(
            self.buf.ta.text().clone(),
            cursor,
            self.session.revs.current(),
        )
    }

    /// Prepare this frame for a host that has nothing to add to it: the
    /// editor's own snapshot and selection, no replace preview, no find
    /// matches. Hosts that draw those build a [`FrameInput`] and call
    /// [`EditorSession::prepare_frame`] directly.
    pub fn prepare_frame(&mut self, width: u16, height: u16) {
        let (preview, preview_overlays, matches) = self.find_frame_parts();
        let input = FrameInput {
            snapshot: self.view_snapshot(),
            width,
            height,
            selection: self.painted_selection(),
            preview,
            preview_overlays,
            matches,
        };
        self.session.prepare_frame(input);
    }

    /// What is painted as the selection this frame: the find bar's **current
    /// match** while a bar is open (the bar owns it rather than writing the
    /// editor's selection), the editor's own selection otherwise.
    pub fn painted_selection(&self) -> Option<((usize, usize), (usize, usize))> {
        match &self.find {
            Some(bar) => bar.current_match(),
            None => self.selection,
        }
    }

    /// The open find bar's contribution to this frame, in
    /// [`FrameInput`]'s shape: the **replace preview**'s lines, its
    /// `Preview`/`PreviewCurrent` overlays, and the `Match` overlays. All
    /// empty when no bar is open. The buffer is never touched — only this
    /// frame's view of it is substituted.
    #[allow(clippy::type_complexity)]
    pub fn find_frame_parts(
        &self,
    ) -> (
        Option<Vec<String>>,
        Vec<crate::logic::editor::overlay::Overlay>,
        Vec<crate::logic::editor::overlay::Overlay>,
    ) {
        match &self.find {
            Some(bar) => bar.overlay(&self.buf.ta).into_frame_parts(),
            None => (None, Vec::new(), Vec::new()),
        }
    }

    /// Whether the vim interpreter is driving this editor.
    pub fn is_vim(&self) -> bool {
        self.buf.is_vim()
    }

    /// The footer modal-mode label (`NORMAL`, `INSERT`, ...). `None` for the
    /// plain direct-edit editor.
    pub fn mode_label(&self) -> Option<String> {
        self.buf.mode_label()
    }

    /// The in-progress vim command (pending count / operator / find) for the
    /// footer. `None` when nothing is pending.
    pub fn pending_input_hint(&self) -> Option<String> {
        self.buf.pending_input_hint()
    }

    /// Cursor-shape classifier: `None` = non-modal, `Some(true)` = Insert
    /// (bar), `Some(false)` = another modal mode (block).
    pub fn modal_is_insert(&self) -> Option<bool> {
        self.buf.modal_is_insert()
    }

    /// The cursor's (row, col) without materialising a snapshot.
    pub fn cursor(&self) -> (usize, usize) {
        self.buf.ta.cursor()
    }

    /// The whole buffer as one string.
    pub fn get_text(&self) -> String {
        self.buf.ta.text().to_string()
    }

    /// Current content revision. Bumped on every text-mutating handler;
    /// stable across cursor moves and idle frames. Used by the autosave
    /// path to record "this snapshot was saved" without rebuilding the
    /// buffer text on completion. `NonZeroU64` makes 0 unrepresentable
    /// so callers can express "no revision" as `Option<NonZeroU64>::None`
    /// without a magic-value sentinel.
    pub fn content_revision(&self) -> NonZeroU64 {
        self.session.revs.current()
    }

    /// The content revision, as the autocomplete layer and the find bar read it.
    pub fn revision(&self) -> NonZeroU64 {
        self.content_revision()
    }

    pub fn is_dirty(&self) -> bool {
        self.session.revs.is_dirty()
    }

    /// Whether a bare Space should start the leader (vim Normal mode only).
    /// Returns `false` for the direct textarea, vim Insert/Visual modes, and
    /// any pending state.
    ///
    /// A pure vim-mode fact. It used to also return false while the find bar
    /// was open — the bar's claim smuggled through the nearest differently
    /// named field, because the snapshot had nowhere to put it. That is now
    /// the host's `claim`.
    pub fn space_leads(&self) -> bool {
        self.buf.space_leads()
    }

    /// Whether a press in this editor moves the cursor to the cell under it.
    /// Always true for the textarea; the nvim engine, where the terminal and
    /// nvim own the mouse, answers false in the host.
    pub fn mouse_drives_cursor(&self) -> bool {
        true
    }

    /// Returns the link or label target under the cursor, or `None` if the
    /// cursor is not inside a wikilink, markdown link, or hashtag span.
    pub fn follow_target_at_cursor(&self) -> Option<FollowTarget> {
        let (row, col) = self.buf.ta.cursor();
        let line = self.buf.ta.row(row)?.into_owned();
        follow_target_in_line(&line, col)
    }

    // ---- Lifecycle ---------------------------------------------------------

    /// Set the search needles to emphasize in the rendered buffer (the note
    /// was opened from a query result). Cleared automatically on the first
    /// edit.
    pub fn set_search_needles(&mut self, needles: Vec<String>) {
        self.session.needles = needles
            .into_iter()
            .map(|n| n.to_lowercase())
            .filter(|n| !n.is_empty())
            .collect();
        self.session.revs.arm_needles();
    }

    /// Show `note` with `text`.
    ///
    /// The note already on screen is reloaded ([`Self::set_text`]): unchanged
    /// text keeps everything — history, selection, scroll, `gv`. Any other
    /// note is loaded from scratch even when its text is identical, so nothing
    /// of the previous note carries over.
    ///
    /// Returns whether the buffer was replaced, so the host can drop the state
    /// it keeps about the old text (an open popup, the find bar).
    pub fn open_note(&mut self, note: kimun_core::nfs::VaultPath, text: String) -> bool {
        if self
            .session
            .note
            .as_ref()
            .is_some_and(|open| open.is_like(&note))
        {
            return self.set_text(text);
        }
        self.session.note = Some(note);
        self.load(text);
        true
    }

    /// The note on screen was renamed to `note`: still the same note, so
    /// nothing is reset, and a later [`Self::open_note`] under the new name is
    /// a reload.
    pub fn renamed_to(&mut self, note: kimun_core::nfs::VaultPath) {
        self.session.note = Some(note);
    }

    /// Reload the note on screen with `text`.
    ///
    /// A no-op when the buffer would be identical — preserves view scroll,
    /// selection, edit generation cache, and an open autocomplete popup.
    /// Saves the expensive lines clone too. Still normalises the saved
    /// marker: if the buffer was flagged dirty by a previous divergent
    /// save, reloading the same content from disk should clear that
    /// flag rather than persist a phantom `[+]` in the title bar.
    ///
    /// That shortcut is only right for the *same* note; switching notes goes
    /// through [`Self::open_note`]. Returns whether the buffer was replaced.
    pub fn set_text(&mut self, text: String) -> bool {
        if text == self.get_text() {
            self.session.revs.mark_saved_current();
            return false;
        }
        self.load(text);
        true
    }

    /// Replace the buffer with `text` and drop everything that described the
    /// old one: history, selection, input state.
    fn load(&mut self, text: String) {
        // A drag under way belonged to the old text.
        self.drag_origin = None;
        self.buf
            .ta
            .replace(crate::ropetext::Text::from(text.as_str()));
        self.buf.reset_input_state();
        self.bump_content();
        let reconstructed = self.get_text();
        self.mark_saved(reconstructed);
        // `self.selection` would otherwise still describe the old text.
        self.selection = None;
        // The find bar is worse: `armed_empty` surviving a note swap means one
        // Ctrl+A deletes every match in a note the user never armed, skipping
        // the confirmation the flag exists to force.
        self.find = None;
        // The whole buffer was replaced. The cursor resets to the top, so if
        // the line count happens to match and row 0 differs, the damage fast
        // path would report `0..1` and leave the rest of the note parsed as the
        // previous one.
        self.session.display.note_bulk_edit();
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
        if !self.session.revs.mark_saved_at(rev) {
            return;
        }
        // Below the guard on purpose: a stale completion marks nothing, and an
        // action that did nothing must not close the group. One undo after a
        // real save lands on exactly what is on disk (CONTEXT.md).
        self.interrupt_typing();
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
            self.session.revs.mark_saved_current();
        } else {
            // Divergent save → stay dirty: forget the saved snapshot.
            self.session.revs.mark_diverged();
        }
    }

    // ---- Edit bookkeeping --------------------------------------------------

    /// Advances the revision clock. Use at every site that mutates the
    /// buffer (insert, delete, paste, undo/redo, autocomplete accept).
    /// `handle_input` uses the revision delta to detect a real text change
    /// without materialising the buffer.
    #[inline]
    pub fn bump_content(&mut self) {
        self.session.revs.bump();
    }

    /// Something happened that is not a continuation of typing.
    ///
    /// One entry point for every path that is not a keystroke — a click, a
    /// find, an autocomplete accept, a save, a vim motion — because the state it
    /// closes is kept on the editor while those paths reach the buffer by
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
    pub fn interrupt_typing(&mut self) {
        self.session.display.clear_visual_goal();
        if self.buf.modal_is_insert().unwrap_or(false) {
            return;
        }
        self.end_typing();
    }

    /// Apply an autocomplete accept: the trigger range becomes the suggestion
    /// and the cursor lands where the action says. It interrupts the typing
    /// run first ([`Self::interrupt_typing`]): outside a vim Insert session the
    /// accept is **one undo group** of its own, so `u` takes the suggestion
    /// back to what was typed. Inside one the session *is* the group, so the
    /// accept joins it and one `u` takes back the whole insert.
    pub fn apply_autocomplete(&mut self, action: &crate::logic::autocomplete::AcceptAction) {
        self.interrupt_typing();
        self.buf
            .ta
            .edit(|ta| crate::logic::autocomplete::apply_accept(ta, action));
        self.after_edit();
    }

    /// End the open typing run: the next keystroke starts a new **undo group**.
    pub(crate) fn end_typing(&mut self) {
        self.buf.typing.end();
    }

    /// Notice that vim entered or left Insert, and close the group if it did.
    ///
    /// This marks a session's *start*: the first key of a session arrives here
    /// with the flag still reading the old mode. The session's *end* is closed by
    /// [`Self::interrupt_typing`] on the engine path, because `Esc` is consumed
    /// there and never reaches this function at all.
    pub fn sync_insert_session(&mut self) {
        let in_insert = self.buf.modal_is_insert().unwrap_or(false);
        if self.last_insert_session == in_insert {
            return;
        }
        self.last_insert_session = in_insert;
        self.buf.typing.end();
    }

    /// What every edit made through the buffer owes the editor: the typing
    /// run ends (any non-typing action closes the open **undo group**), the
    /// selection mirror the renderer reads is refreshed, and the outcome is
    /// drained — bumping the revision and telling the view what to re-parse.
    /// Reports whether the text changed.
    pub fn after_edit(&mut self) -> bool {
        self.interrupt_typing();
        self.sync_highlight();
        self.apply_edit_outcome()
    }

    /// Drain the **rope buffer**'s measured outcome and apply it.
    ///
    /// The one place a text change turns into a revision bump and a
    /// parse-damage signal. Both facts are derived by the buffer from the
    /// content either side of the edit, so neither can be predicted wrongly
    /// (an `insert_str` that returns `false` after deleting) or simply
    /// forgotten at one of 22 sites.
    fn apply_edit_outcome(&mut self) -> bool {
        let outcome = self.buf.ta.take_outcome();
        if outcome.changed {
            self.bump_content();
        }
        if outcome.bulk {
            self.session.display.note_bulk_edit();
        }
        if let Some(rows) = outcome.damage {
            self.session.display.note_damage(rows, outcome.line_delta);
        }
        outcome.changed
    }

    /// Undo one *user action*, settled like any edit: the revision is bumped
    /// (so dirty and autosave see it) and the view is told what to re-parse.
    /// Reports whether anything was undone.
    pub fn undo(&mut self) -> bool {
        if self.undo_grouped() {
            self.apply_edit_outcome();
            true
        } else {
            false
        }
    }

    /// Redo one *user action*, settled. Mirror of [`Self::undo`].
    pub fn redo(&mut self) -> bool {
        if self.redo_grouped() {
            self.apply_edit_outcome();
            true
        } else {
            false
        }
    }

    /// Undo one *user action*, unsettled. The **edit buffer** replays history
    /// to the state the action started from, so nothing here counts entries.
    /// Hosts call [`Self::undo`], which also drains the edit outcome.
    pub(crate) fn undo_grouped(&mut self) -> bool {
        let moved = self.buf.ta.undo();
        if moved {
            self.sync_highlight();
        }
        moved
    }

    /// Redo one *user action*, unsettled. Mirror of [`Self::undo_grouped`].
    pub(crate) fn redo_grouped(&mut self) -> bool {
        let moved = self.buf.ta.redo();
        if moved {
            self.sync_highlight();
        }
        moved
    }

    /// Settle the editor after a pointer event has been handled. A left press
    /// ends vim's Visual mode (a drag selects through `select_dragged`; scroll,
    /// release and right-click leave the mode alone), and the painted selection
    /// is refreshed either way. Pass `left_down` as true for a left press.
    pub fn settle_pointer(&mut self, left_down: bool) {
        if left_down {
            self.buf.sync_mouse_selection(false);
        }
        self.sync_highlight();
    }

    /// Paint the selection as the user sees it: under vim's Visual modes the
    /// range Visual covers (`VimEngine::visual_range` decides what that is),
    /// otherwise the buffer's own selection.
    ///
    /// One painting choice on top: a `V` selection's last row is painted to
    /// the pane's edge rather than to its last character, so linewise reads as
    /// whole lines. Which rows are covered is still the engine's range.
    pub fn sync_highlight(&mut self) {
        let mut selection = self.buf.selection_as_shown();
        if self.buf.is_visual_line()
            && let Some((start, (er, _))) = selection
        {
            selection = Some((start, (er, usize::MAX)));
        }
        self.selection = selection;
    }

    /// Under vim's Visual modes the live textarea selection is not what the
    /// highlight shows: charwise it excludes the char under the cursor (vim's
    /// selection is inclusive), linewise it spans anchor-to-cursor columns
    /// rather than whole rows. Re-select the range Visual covers
    /// ([`crate::logic::editor::vim::VimEngine::visual_range`]) so out-of-engine consumers
    /// (paste-over-selection, bold/italic/strikethrough wrap) act on all of
    /// it. No-op outside Visual (Direct/Insert), where the buffer's
    /// selection is already what callers want.
    pub(crate) fn extend_visual_selection_inclusive(&mut self) {
        if let Some((start, end)) = self.buf.visual_range() {
            self.buf.ta.set_selection(start, end);
        }
    }

    // ---- Clipboard ---------------------------------------------------------

    /// Copy selected text to the OS clipboard, flashing the outcome.
    ///
    /// Routed through the shared [`crate::logic::clipboard::yank`] seam so a clipboard
    /// failure is reported rather than swallowed, and so "nothing was selected"
    /// is distinguishable from "the copy failed".
    pub fn copy_selection_to_clipboard(&mut self, tx: &AppTx) {
        // A vim Visual selection copies exactly as the Ctrl-C chord does —
        // one path for both gestures.
        if let Some(text) = self.buf.copy_visual() {
            crate::logic::clipboard::yank(text, "copied", tx);
            return;
        }
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
                .buf
                .selection_as_shown()
                .and_then(|range| self.buf.ta.text_between(range.0, range.1));
            match selected {
                Some(t) if !t.is_empty() => t,
                _ => {
                    tx.send(AppMessage::FlashMessage("nothing to copy".into()))
                        .ok();
                    return;
                }
            }
        };
        crate::logic::clipboard::yank(text, "copied", tx);
    }

    /// The textarea-only clipboard paste: no find-bar routing and no
    /// autocomplete epilogue (the host layers add those). Used by vim's
    /// ClipboardPaste and by GUI hosts.
    ///
    /// Paste text from the OS clipboard at the cursor, replacing any active
    /// selection. Every failure is reported — silence here is what made the
    /// vim-mode paste bug so hard to place.
    ///
    /// Returns whether clipboard text arrived and was inserted, so a host that
    /// follows the text (an autocomplete popup) can re-sync after it.
    pub fn paste_from_clipboard(&mut self, tx: &AppTx) -> bool {
        let Some(text) = crate::logic::clipboard::read_text(tx) else {
            return false;
        };
        self.paste_text(&text);
        // Report the paste like every other clipboard action. Without this the
        // footer keeps the raw chord echo, so Ctrl+V was the one clipboard key
        // that never said what it did.
        tx.send(AppMessage::FlashMessage("pasted".into())).ok();
        true
    }

    /// Inserts `text` at the cursor, replacing any active selection. When `text`
    /// is a URL (http/https/ftp/ftps/mailto) and a selection is active, the
    /// selection is wrapped as a markdown link `[selection](url)` instead of
    /// being replaced by the raw URL.
    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // The paste consumes a Visual selection: record it for `gv` before the
        // edit moves the text, and leave Visual after (as the image paste does).
        self.buf.conclude_visual();
        self.extend_visual_selection_inclusive();
        let selection = linkable_url(text).and_then(|_| self.buf.ta.selection_text());
        let wrapped = try_build_markdown_link(text, selection.as_deref());
        let insert = wrapped.as_deref().unwrap_or(text).to_string();
        // Replacing a selection is a cut plus an insert — one paste,
        // one undo.
        self.buf.ta.edit(|ta| {
            if ta.selection_range().is_some() {
                ta.cut();
            }
            ta.insert_str(insert);
        });
        self.buf.sync_mouse_selection(false);
        self.after_edit();
    }

    /// Text committed by a platform text-input service (an IME, a composed
    /// character, a press-and-hold accent): replace `range` — chars of one
    /// `row` — with `text`, or the selection (else nothing) at the cursor when
    /// `range` is `None`. An empty range inserts at its start; empty `text`
    /// deletes. Text input acts at the cursor, so a wheel-scrolled view follows
    /// it back, as a key does. `false`, and nothing touched, when the range is
    /// not addressable, or when vim is in a mode other than Insert.
    pub fn commit_text(
        &mut self,
        range: Option<(usize, std::ops::Range<usize>)>,
        text: &str,
    ) -> bool {
        // Committed text is typing, and vim only types in Insert. In Normal or
        // Visual a keystroke is a command that must go through `handle_key`;
        // committing here would bypass the engine, and in Visual the cut below
        // would also re-select what Visual covers (one char wider than `range`).
        if self.modal_is_insert() == Some(false) {
            return false;
        }
        if let Some((row, cols)) = range {
            let ta = &mut self.buf.ta;
            let placed = if cols.start == cols.end {
                // Validate before touching anything: a failed jump must leave
                // the selection (and its painted mirror) as it was.
                ta.text()
                    .position(row, crate::ropetext::Column::new(cols.start))
                    .is_some()
                    && {
                        ta.cancel_selection();
                        ta.jump_to(row, cols.start)
                    }
            } else {
                ta.set_selection((row, cols.start), (row, cols.end))
            };
            if !placed {
                return false;
            }
        }
        self.session.display.follow_cursor();
        if text.is_empty() {
            self.take_selection_for_external_paste();
        } else {
            // Committed text is typing. In a vim Insert session the session is
            // one **undo group** (CONTEXT.md), so the text joins it as a typed
            // char would (`i ab<IME é>cd Esc u` takes all of it back).
            self.sync_insert_session();
            if self.last_insert_session
                && let Some(first) = text.chars().next()
                && self
                    .buf
                    .typing
                    .continues_session(typing_run::Stroke::Insert(first), std::time::Instant::now())
            {
                self.buf.ta.continue_group();
            }
            self.insert_at_cursor(text);
        }
        true
    }

    /// Inserts `text` at the cursor, replacing any active selection.
    pub fn insert_at_cursor(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // Replacing the selection happens HERE, atomically with the insert, and
        // not earlier when the paste was merely started: the image encode and
        // the attachment save can both fail (disk full, read-only vault, a sync
        // conflict), and a cut done up front would leave the user's selected
        // text destroyed with nothing in its place and only a save error to
        // explain it.
        self.take_selection_for_external_paste();
        self.buf.ta.insert_str(text);
        self.after_edit();
    }

    /// Snapshot of the system clipboard image, if any. Returns owned RGBA bytes
    /// plus the image dimensions. The screen layer is responsible for encoding
    /// (e.g. PNG) and persisting via the vault.
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
        // Before the cut, so `gv` marks move with it rather than being set
        // from coordinates the cut has just invalidated.
        self.buf.conclude_visual();
        self.extend_visual_selection_inclusive();
        let ta = &mut self.buf.ta;
        let cut = ta.selection_range().is_some() && ta.cut();
        self.selection = ta.selection_range();
        if cut {
            self.apply_edit_outcome();
        }
        // `false` = no live selection, so a modal engine returns to Normal.
        // A no-op for Insert and for the non-modal backends.
        self.buf.sync_mouse_selection(false);
    }

    // ---- Text actions ------------------------------------------------------

    /// Auto-surround: wrap the selection in `open`…`close`. A vim charwise
    /// Visual selection is inclusive, so it is widened by one char first
    /// (otherwise `ve` then Bold yields `**hell**o`). `false` — and nothing
    /// touched — without a selection.
    pub fn wrap_selection(&mut self, open: &str, close: &str) -> bool {
        self.extend_visual_selection_inclusive();
        if !markdown_edits::wrap_selection(&mut self.buf.ta, open, close) {
            return false;
        }
        // The inner text stays selected so wraps chain; under vim that is a
        // Visual selection, or the mode and the highlight disagree.
        self.buf.adopt_host_selection();
        // Only here, past every `return false` above: this function is consulted
        // for each bare `( [ { < " ' ` * _ ~` keystroke, and the declining ones
        // fall through to ordinary typing, which must keep its run.
        self.after_edit();
        true
    }

    /// Bold / Italic / Strikethrough: wrap the selection in the marker, or with
    /// nothing selected insert a pair and leave the cursor between. No-op for
    /// other actions.
    pub fn apply_text_action(&mut self, action: TextAction) {
        let Some(marker) = markdown_edits::emphasis_marker(action) else {
            return;
        };
        // A Visual selection with nothing in it (an empty row): nothing to
        // wrap, and inserting a marker pair would type into a selection.
        if self.buf.visual_is_empty() {
            return;
        }
        if self.wrap_selection(marker, marker) {
            return;
        }
        markdown_edits::insert_pair(&mut self.buf.ta, marker, marker);
        self.after_edit();
    }

    /// Smart Enter — [`markdown_edits::smart_enter`] on the live buffer. `true`
    /// when handled, so the caller does not insert a plain newline.
    pub fn smart_enter(&mut self) -> bool {
        if !markdown_edits::smart_enter(&mut self.buf.ta) {
            return false;
        }
        self.after_edit();
        true
    }

    /// The OUTLINE drawer's jump — [`markdown_edits::jump_to_row`] on the
    /// live buffer.
    pub fn jump_to_row(&mut self, row: usize) {
        if markdown_edits::jump_to_row(&mut self.buf.ta, row) {
            self.sync_highlight();
        }
    }

    /// Indent or dedent whole rows by one **indent step** — the cursor's row,
    /// or every row a selection touches. A selection ending at column 0 of a
    /// later row does not visually include that row, so it is left alone.
    pub fn indent_lines(&mut self, dedent: bool) {
        let ta = &mut self.buf.ta;
        let rows = match ta.selection_range() {
            Some(((sr, _), (er, ec))) => {
                let last = if ec == 0 && er > sr { er - 1 } else { er };
                sr..=last
            }
            None => {
                let (row, _) = ta.cursor();
                row..=row
            }
        };
        if ta.indent_rows(rows, dedent) {
            self.after_edit();
        }
    }

    // ---- Find bar ----------------------------------------------------------

    /// The open **find bar**, for a front end to paint: its fields, focus,
    /// status and match count. `None` when closed.
    pub fn find_bar(&self) -> Option<&FindBar> {
        self.find.as_ref()
    }

    /// Where the open find bar stands among its matches (`3/12`), for a
    /// status line. `None` when the bar is closed or nothing matches.
    pub fn find_match_position(&self) -> Option<MatchPosition> {
        self.find.as_ref()?.match_position(&self.buf.ta)
    }

    /// Whether a find bar is open (and so holds the **editor claim**).
    pub fn find_open(&self) -> bool {
        self.find.is_some()
    }

    /// Open an empty find bar or, when one is already open, step to the next
    /// match (vim `/` pressed again, Ctrl+F on an open bar). Returns whether a
    /// bar was newly opened, so the host can close anything that would compete
    /// for its keys (an autocomplete popup).
    pub fn open_find(&mut self) -> bool {
        // Acts at the cursor: a wheel-scrolled view re-attaches to it.
        self.session.display.follow_cursor();
        if let Some(bar) = self.find.as_mut() {
            bar.advance(&mut self.buf.ta, false);
            self.apply_edit_outcome();
            return false;
        }
        self.find = Some(FindBar::new());
        true
    }

    /// Open the find bar with the **replace field** revealed, or reveal it on
    /// an already-open bar. Returns whether a bar was newly opened.
    pub fn open_replace(&mut self) -> bool {
        let opened = self.find.is_none();
        self.find.get_or_insert_with(FindBar::new).reveal_replace();
        opened
    }

    /// Hand a key to the open find bar, which consumes every key while open.
    /// Returns `false` (and does nothing) when no bar is open.
    ///
    /// Settles whatever the bar did to the buffer (revision, parse damage),
    /// drops the bar when it asks to close — clearing the selection with it —
    /// and ends the typing run, so an edit after the bar is its own **undo
    /// group**.
    pub fn handle_find_key(&mut self, key: KeyInput) -> bool {
        let Some(bar) = self.find.as_mut() else {
            return false;
        };
        // The bar acts at the cursor: a wheel-scrolled view re-attaches to it.
        self.session.display.follow_cursor();
        let outcome = bar.handle_key(key, &mut self.buf.ta);
        if outcome.close {
            self.close_find();
        }
        self.apply_edit_outcome();
        self.interrupt_typing();
        true
    }

    /// A key the front end could not translate into a [`KeyInput`] while the
    /// bar is open: the bar still owns it, so it is consumed and does nothing
    /// but end the typing run, as any bar key does. Returns `false` when no bar
    /// is open.
    pub fn ignore_find_key(&mut self) -> bool {
        if self.find.is_none() {
            return false;
        }
        self.session.display.follow_cursor();
        self.apply_edit_outcome();
        self.interrupt_typing();
        true
    }

    /// Paste `text` into the open bar's focused field (first line only).
    /// Returns `false` when no bar is open — the paste is then the buffer's.
    pub fn paste_into_find(&mut self, text: &str) -> bool {
        let Some(bar) = self.find.as_mut() else {
            return false;
        };
        bar.paste(text, &mut self.buf.ta);
        self.apply_edit_outcome();
        true
    }

    /// Close the find bar. The pattern stays on the buffer, so vim's `n`/`N`
    /// ([`Self::search_repeat`]) still work.
    ///
    /// Clears the selection anchor as well as the painted range: the bar's
    /// cursor jumps move the cursor without touching the anchor, so a
    /// selection that existed before the search would be left live but
    /// unpainted — and the next keystroke would silently delete it.
    pub fn close_find(&mut self) {
        if self.find.take().is_some() {
            self.buf.ta.cancel_selection();
            self.selection = None;
        }
    }

    /// Repeat the last search (vim `n`/`N`) with the buffer's persisted
    /// pattern, even when the bar is closed, and paint the match it landed on
    /// as the selection.
    pub fn search_repeat(&mut self, backward: bool) {
        let selection = if self.buf.ta.search_repeat(backward) {
            self.buf.ta.match_at_cursor()
        } else {
            None
        };
        self.selection = selection;
    }

    /// Drain what the editor has asked its host to do since the last call.
    pub fn take_requests(&mut self) -> Vec<EditorRequest> {
        std::mem::take(&mut self.requests)
    }

    /// Drain the requests the host acts on *in the same input call* that
    /// produced them (find bar, palette, viewport): everything except
    /// [`EditorRequest::ContextMenu`], which stays queued for the host's
    /// [`Self::take_requests`] poll after its own panel mouse handling.
    pub fn take_host_requests(&mut self) -> Vec<EditorRequest> {
        let (kept, taken) = std::mem::take(&mut self.requests)
            .into_iter()
            .partition(|r| *r == EditorRequest::ContextMenu);
        self.requests = kept;
        taken
    }
}

// ---- Input -----------------------------------------------------------------

impl Editor {
    /// One mapped key: the vim engine first (Normal/Visual consume the key
    /// there; Insert passes it through), then — in plain mode and for vim's
    /// `PassThrough` — the direct-edit path. A key with no [`KeyInput`] goes
    /// to [`Self::ignore_unmapped_key`] instead.
    ///
    /// The [`KeyPath`] says which way the key went, so the host can do its
    /// own follow-up (the TUI's popup diff runs on the direct path only).
    /// What the editor does not own is queued: vim `/`, `?`, `n`, `N` as
    /// [`EditorRequest`]s the host drains with [`Self::take_host_requests`]
    /// before the input call returns.
    pub fn handle_key(&mut self, key: KeyInput, tx: &AppTx) -> (Handled, KeyPath) {
        // macOS clipboard chords: Cmd+C/V/X do what Ctrl+C/V/X do. The GUI
        // passes Cmd through as `sup` (never as Ctrl), so without this alias
        // the native paste key reaches no engine at all.
        let key = match key {
            KeyInput {
                key: Key::Char(c @ ('c' | 'v' | 'x')),
                mods: Mods::SUPER,
            } => KeyInput::new(Key::Char(c), Mods::CTRL),
            other => other,
        };
        // Every key acts at the cursor, so a wheel-scrolled view comes back to
        // it — even when the key leaves the cursor where it was (find-next onto
        // the same match, a motion at the buffer edge). Idempotent, so a host
        // that also calls it earlier does not double-scroll.
        self.session.display.follow_cursor();
        // Vim interpreter: Normal/Visual consume the key here; Insert
        // mode returns PassThrough and falls into the direct path below
        // so typing, autocomplete, auto-surround and smart-Enter all
        // keep working.
        if let Some(outcome) = self.buf.vim_handle_key(&key) {
            use crate::logic::editor::vim::VimKeyOutcome;
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
                    return (Handled::Consumed, KeyPath::VimOther);
                }
                VimKeyOutcome::CursorOnly => {
                    // Mirror the textarea's selection into self.selection so
                    // Visual mode renders through the existing selection pipeline.
                    // For non-visual CursorOnly (plain motion), selection_range()
                    // returns None → self.selection = None (no regression).
                    self.sync_highlight();
                    return (Handled::Consumed, KeyPath::VimCursorOnly);
                }
                VimKeyOutcome::NoOp => return (Handled::Consumed, KeyPath::VimOther),
                VimKeyOutcome::PassThrough => { /* fall through to direct path */ }
                VimKeyOutcome::Host(action) => {
                    use crate::logic::editor::vim::VimHostAction;
                    let mut path = KeyPath::VimOther;
                    match action {
                        VimHostAction::OpenPalette => {
                            self.requests.push(EditorRequest::OpenPalette);
                        }
                        VimHostAction::OpenSearch { forward: _ } => {
                            // `/` and `?` open the host's find bar.
                            // (`?` backward-first is a later refinement;
                            // n/N still navigate both directions.)
                            self.requests.push(EditorRequest::OpenSearch);
                        }
                        VimHostAction::SearchNext => self.requests.push(EditorRequest::SearchNext),
                        VimHostAction::SearchPrev => self.requests.push(EditorRequest::SearchPrev),
                        // Copy and Cut: the engine already did the
                        // editing and the mode transition; all that is
                        // left is the I/O and reporting it.
                        VimHostAction::ClipboardCopy(text) => {
                            self.selection = None;
                            crate::logic::clipboard::yank(text, "copied", tx);
                        }
                        VimHostAction::ClipboardCut(text) => {
                            self.selection = None;
                            crate::logic::clipboard::yank(text, "cut", tx);
                        }
                        // Paste is different: the engine deliberately
                        // left the range SELECTED rather than cutting
                        // it, so the replacement is atomic. Keep the
                        // selection live — `paste_text` consumes it, and
                        // a failed/empty read must leave it untouched.
                        VimHostAction::ClipboardPaste => {
                            self.selection = self.buf.ta.selection_range();
                            if self.paste_from_clipboard(tx) {
                                path = KeyPath::VimPasted;
                            }
                        }
                    }
                    return (Handled::Consumed, path);
                }
            }
        }
        self.handle_direct_key(key, tx)
    }

    /// The direct-edit path alone, skipping the vim stage: what plain mode and
    /// vim's `PassThrough` keys get. Production enters through
    /// [`Self::handle_key`]; tests that pin the direct path call this.
    pub(crate) fn handle_direct_key(&mut self, key: KeyInput, tx: &AppTx) -> (Handled, KeyPath) {
        let rev_before = self.revision();
        let cursor_before = self.cursor();
        let (handled, pasted) = self.direct_key(key, tx);
        let path = KeyPath::Direct {
            text_changed: self.revision() != rev_before,
            cursor_moved: self.cursor() != cursor_before,
            pasted,
        };
        (handled, path)
    }

    /// The direct path's body. The `bool` is whether `Ctrl+V` inserted
    /// clipboard text.
    fn direct_key(&mut self, key: KeyInput, tx: &AppTx) -> (Handled, bool) {
        // No find-bar check here: the host routes keys to its find bar before
        // the editor ever sees them.

        // Whether this key types. Decided here rather than in the plain-key
        // section below, because a key claimed earlier — Ctrl+Z, a clipboard
        // chord, Tab — returns before ever reaching it, and a run left open
        // across an undo would try to extend a group that was just taken back.
        let stroke = plain_keys::operation(key).and_then(|op| match op {
            plain_keys::Operation::Insert(c) => Some(typing_run::Stroke::Insert(c)),
            plain_keys::Operation::InsertNewline => Some(typing_run::Stroke::Insert('\n')),
            plain_keys::Operation::DeleteBack | plain_keys::Operation::DeleteForward => {
                Some(typing_run::Stroke::Delete)
            }
            _ => None,
        });
        if stroke.is_none() {
            self.buf.typing.end();
        }

        // System clipboard shortcuts — intercept before passing to textarea.
        if key.mods == Mods::CTRL {
            match key.key {
                Key::Char('c') => {
                    self.copy_selection_to_clipboard(tx);
                    return (Handled::Consumed, false);
                }
                Key::Char('v') => {
                    let pasted = self.paste_from_clipboard(tx);
                    return (Handled::Consumed, pasted);
                }
                Key::Char('x') => {
                    self.copy_selection_to_clipboard(tx);
                    let ta = &mut self.buf.ta;
                    // `ta.cut()` returns `false` when the selection was
                    // empty / nothing to remove. Use its return value
                    // directly rather than pre-checking selection_range —
                    // one source of truth, no spurious view rebuild on
                    // no-op Ctrl+X.
                    let cut = ta.cut();
                    self.selection = ta.selection_range();
                    if cut {
                        self.apply_edit_outcome();
                    }
                    return (Handled::Consumed, false);
                }
                _ => {}
            }
        }

        // Undo / Redo (Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z). A replace is two
        // history entries and must cost one Ctrl+Z, not two: the **undo
        // group** bookkeeping is the editor's.
        if (Mods {
            shift: false,
            ..key.mods
        }) == Mods::CTRL
        {
            match key.key {
                Key::Char('z') if !key.mods.shift => {
                    self.undo();
                    return (Handled::Consumed, false);
                }
                Key::Char('y') | Key::Char('Z') => {
                    self.redo();
                    return (Handled::Consumed, false);
                }
                _ => {}
            }
        }

        // Standard text-editor shortcuts.
        // `input_without_shortcuts` only handles chars, backspace, delete, tab, newline —
        // all navigation and editing shortcuts must be mapped explicitly.
        // Outcome tracks whether the handled shortcut mutated the buffer, only
        // moved the cursor, or did literally nothing (e.g. Ctrl+Z on an empty
        // undo stack) — so the revision clock is not
        // bumped on true no-ops.
        // BackTab is what most terminals emit for Shift+Tab.
        match key.key {
            Key::Tab if !key.mods.ctrl && !key.mods.alt => {
                self.indent_lines(key.mods.shift);
                return (Handled::Consumed, false);
            }
            Key::BackTab => {
                self.indent_lines(true);
                return (Handled::Consumed, false);
            }
            _ => {}
        }
        if key.key == Key::Enter && key.mods == Mods::NONE && self.smart_enter() {
            return (Handled::Consumed, false);
        }

        // Auto-surround: an opening/symmetric pair char typed over a selection
        // wraps it instead of replacing it (see CONTEXT.md "Auto-surround").
        // Shift is allowed (most opening chars are shifted keys); Ctrl/Alt
        // chords fall through. The selection lands on the inner text so wraps
        // chain: `[` `[` builds a wikilink — and the host's post-key
        // sync legitimately opens the wikilink popup on the chained wrap.
        if let Key::Char(c) = key.key
            && (Mods {
                shift: false,
                ..key.mods
            }) == Mods::NONE
            && let Some((open, close)) = markdown_edits::surround_pair(c)
            && self.wrap_selection(open, close)
        {
            return (Handled::Consumed, false);
        }

        // A change of modal state ends whatever run was open: leaving Insert
        // closes vim's session, and entering it starts a fresh one. Also read in
        // `handle_key` for the keys the engine consumes, which never arrive
        // here — this call is what keeps the direct path (which the tests drive)
        // bracketed too.
        self.sync_insert_session();

        // Read before the buffer is borrowed below.
        let in_insert_session = self.last_insert_session;
        let Editor {
            buf,
            session,
            selection,
            ..
        } = self;
        let (ta, run) = (&mut buf.ta, &mut buf.typing);
        // Last: what the key means to the plain backend. It runs *after* the
        // editor's own claims — `Tab` indents rows, `Enter` may continue a
        // list, an opening bracket over a selection wraps it — because those are
        // the same keys and the editor's reading of them wins.
        if let Some(op) = plain_keys::operation(key) {
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
                session.display.clear_visual_goal();
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
                Some((down, extend)) if session.display.move_cursor_visually(ta, down, extend) => {
                    false
                }
                _ => plain_keys::apply(op, ta),
            };
            *selection = ta.selection_range();
            if changed {
                self.apply_edit_outcome();
            }
        }
        // A key the table declines — a function key, a modifier-only release, an
        // IME composition event — leaves the buffer alone, so a harmless keypress
        // cannot mark the note dirty and trigger an autosave.
        (Handled::Consumed, false)
    }

    /// A key with no [`KeyInput`] (`Null`, lock and media keys, any META/HYPER
    /// chord): nothing in the direct path can act on it, but it is still an
    /// action, so it ends the open typing run, settles vim's insert-session
    /// bracket, and is consumed.
    pub fn ignore_unmapped_key(&mut self) -> (Handled, KeyPath) {
        self.session.display.follow_cursor();
        self.buf.typing.end();
        self.sync_insert_session();
        (
            Handled::Consumed,
            KeyPath::Direct {
                text_changed: false,
                cursor_moved: false,
                pasted: false,
            },
        )
    }

    /// A pointer event inside the editor that has no [`PointerInput`]
    /// (horizontal scroll). It is still ours, so it counts
    /// as an action, but it does nothing: it is consumed.
    pub fn ignored_pointer(&mut self) -> Handled {
        self.interrupt_typing();
        self.selection = self.buf.ta.selection_range();
        Handled::Consumed
    }

    /// Handle a pointer event. The host has already checked that it landed
    /// inside the editor; `pointer` is relative to the text area.
    ///
    /// A right-press with no selection queues [`EditorRequest::ContextMenu`];
    /// a wheel scroll that moved the viewport queues
    /// [`EditorRequest::ViewportScrolled`].
    pub fn handle_pointer(&mut self, pointer: &PointerInput, tx: &AppTx) -> Handled {
        // Past the bounds check the event is ours, so it is an action: a click
        // moves the cursor, and even a scroll means attention moved. Placed above
        // the context-menu return below so a right-click counts too.
        self.interrupt_typing();
        // Right-click: with a selection it copies (unchanged behavior);
        // without one it asks the host to open the note's context menu
        // (file & note ops).
        if matches!(pointer.kind, PointerKind::Down(Button::Right))
            && self.selection.is_none_or(|(start, end)| start == end)
        {
            if !self.requests.contains(&EditorRequest::ContextMenu) {
                self.requests.push(EditorRequest::ContextMenu);
            }
            return Handled::Consumed;
        }
        // Handle right-click clipboard copy in its own scope to avoid borrow conflicts.
        if matches!(pointer.kind, PointerKind::Down(Button::Right)) {
            self.copy_selection_to_clipboard(tx);
            self.sync_highlight();
            return Handled::Consumed;
        }
        // A drag under vim selects what it covers, rebuilt from where it began.
        // With no press in this editor behind it (one that began elsewhere, or
        // before another note was opened) there is nothing it could select.
        if matches!(pointer.kind, PointerKind::Drag(_)) {
            let (lrow, lcol) = self
                .session
                .display
                .click_at_screen(pointer.row as usize, pointer.col as usize);
            match self.drag_origin {
                Some(origin)
                    if self
                        .buf
                        .select_dragged(origin, (lrow as usize, lcol as usize)) =>
                {
                    self.sync_highlight();
                    return Handled::Consumed;
                }
                None if self.buf.is_vim() => return Handled::Consumed,
                _ => {}
            }
        }
        // The drag, if any, is over.
        if matches!(pointer.kind, PointerKind::Up(_)) {
            self.drag_origin = None;
        }
        let Editor {
            buf,
            session,
            drag_origin,
            selection,
            requests,
            ..
        } = self;
        let ta = &mut buf.ta;
        match pointer.kind {
            PointerKind::Down(_) => {
                session.display.follow_cursor();
                ta.cancel_selection();
                let (lrow, lcol) = session
                    .display
                    .click_at_screen(pointer.row as usize, pointer.col as usize);
                ta.jump_to(lrow as usize, lcol as usize);
                ta.start_selection();
                *drag_origin = Some(ta.cursor());
            }
            PointerKind::Drag(_) => {
                let (lrow, lcol) = session
                    .display
                    .click_at_screen(pointer.row as usize, pointer.col as usize);
                ta.jump_to(lrow as usize, lcol as usize);
            }
            // A press that never became a drag leaves an empty selection
            // anchored at the caret; left armed, the next cursor motion
            // (vim's `k`) would grow it. Only a real drag keeps its range.
            PointerKind::Up(_) => {
                if ta.selection_range().is_none_or(|(a, b)| a == b) {
                    ta.cancel_selection();
                }
            }
            // The wheel moves kimün's own viewport, never the cursor — the
            // find bar relies on that to let the user read elsewhere mid-search.
            PointerKind::ScrollUp | PointerKind::ScrollDown => {
                let delta = if pointer.kind == PointerKind::ScrollUp {
                    -WHEEL_SCROLL_ROWS
                } else {
                    WHEEL_SCROLL_ROWS
                };
                // Anything anchored to the cursor (a popup) may now be off
                // screen: left open, it would take keys the user cannot see.
                if session.display.scroll_by(delta) {
                    requests.push(EditorRequest::ViewportScrolled);
                }
            }
            // Everything else is somebody else's. The incumbent forwarded these
            // to the widget, which scrolled a viewport kimün never renders from.
            _ => {}
        }
        *selection = ta.selection_range();
        // Mouse handling moves the cursor / selection but does not insert
        // text — click, drag and scroll are all it produces.
        Handled::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::editor::rope_buffer::CursorMove;
    use crate::logic::editor::snapshot::EditorMode;

    fn long_note_editor(vim: bool) -> Editor {
        let mut ed = Editor::new(vim);
        ed.buffer_mut().insert_str(
            (0..10)
                .map(|i| format!("line{i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        ed.buffer_mut().move_cursor(CursorMove::Top);
        ed.buffer_mut().move_cursor(CursorMove::Head);
        ed
    }

    // ---- Find bar, driven through the logic `KeyInput` ----

    fn find_keys(ed: &mut Editor, s: &str) {
        for c in s.chars() {
            assert!(ed.handle_find_key(KeyInput::plain(Key::Char(c))));
        }
    }

    #[test]
    fn find_keys_are_refused_without_a_bar() {
        let mut ed = Editor::new(false);
        ed.set_text("abc".into());
        assert!(!ed.handle_find_key(KeyInput::plain(Key::Char('a'))));
        assert!(!ed.paste_into_find("a"));
        assert!(!ed.ignore_find_key());
        assert_eq!(ed.get_text(), "abc");
    }

    #[test]
    fn open_find_types_into_the_bar_and_paints_the_current_match() {
        let mut ed = Editor::new(false);
        ed.set_text("foo bar\nbar".into());
        assert!(ed.open_find(), "first open is a fresh bar");
        find_keys(&mut ed, "bar");
        assert_eq!(ed.get_text(), "foo bar\nbar", "typing went into the bar");
        assert_eq!(ed.find_bar().unwrap().query(), "bar");
        assert_eq!(ed.painted_selection(), Some(((0, 4), (0, 7))));
        let (preview, _, matches) = ed.find_frame_parts();
        assert!(preview.is_none());
        assert_eq!(matches.len(), 2);
        // Opening again advances rather than resetting.
        assert!(!ed.open_find());
        assert_eq!(ed.painted_selection(), Some(((1, 0), (1, 3))));
    }

    #[test]
    fn esc_closes_the_bar_clears_the_selection_and_keeps_n_working() {
        let mut ed = Editor::new(false);
        ed.set_text("bar x bar".into());
        ed.open_find();
        find_keys(&mut ed, "bar");
        assert!(ed.handle_find_key(KeyInput::plain(Key::Esc)));
        assert!(!ed.find_open());
        assert_eq!(ed.selection(), None);
        assert_eq!(ed.buffer().selection_range(), None);
        // The pattern stays on the buffer for vim `n`.
        ed.search_repeat(false);
        assert_eq!(ed.selection(), Some(((0, 6), (0, 9))));
    }

    #[test]
    fn replace_through_the_editor_bumps_the_revision_and_undoes_as_one() {
        let mut ed = Editor::new(false);
        ed.set_text("a a a".into());
        assert!(ed.open_replace(), "Ctrl+H on a closed bar opens one");
        find_keys(&mut ed, "a");
        assert!(ed.handle_find_key(KeyInput::plain(Key::Tab)));
        find_keys(&mut ed, "b");
        let rev = ed.revision();
        assert!(ed.handle_find_key(KeyInput::ctrl('a')));
        assert_eq!(ed.get_text(), "b b b");
        assert!(ed.revision() > rev, "the external edit was settled");
        assert!(ed.is_dirty());
        assert!(ed.handle_find_key(KeyInput::ctrl('z')));
        assert_eq!(ed.get_text(), "a a a");
        assert!(ed.find_open(), "undo must not close the bar");
    }

    #[test]
    fn closing_a_bar_after_a_replace_preview_repaints_the_real_text() {
        // The preview swaps in other rows for a frame; closing the bar must
        // not leave the layout describing them.
        let mut ed = Editor::new(false);
        ed.set_text("**aa** ab\nsecond a line\nthird".into());
        ed.open_replace();
        find_keys(&mut ed, "a");
        assert!(ed.handle_find_key(KeyInput::plain(Key::Tab)));
        find_keys(&mut ed, "zzzzzzzzzzzzzzzzzzzz");
        ed.prepare_frame(20, 10);
        let _ = ed.session.display.frame(20, true);
        assert!(ed.handle_find_key(KeyInput::plain(Key::Esc)));
        assert!(!ed.find_open());
        ed.prepare_frame(20, 10);
        let _ = ed.session.display.frame(20, true);
    }

    /// Every row the last frame drew, as text.
    fn drawn_rows(ed: &Editor, width: u16) -> Vec<String> {
        ed.session
            .display
            .frame(width, true)
            .rows
            .iter()
            .map(|r| r.runs.iter().map(|run| run.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn a_preview_with_matches_far_from_the_cursor_swaps_in_and_out_cleanly() {
        // Regression (GUI manual gate): closing a bar whose preview had grown
        // a row far from the cursor (outside the cursor hint's ±4 window)
        // panicked in `frame` — the display took the swap for an edit at the
        // cursor row and left that row's layout describing the preview.
        let mut text = vec!["a here".to_string()];
        // Blank rows between paragraphs: reset boundaries, so the parse
        // splice stays narrow instead of widening over the whole note.
        text.extend((0..8).map(|i| {
            if i % 2 == 0 {
                String::new()
            } else {
                format!("filler {i}")
            }
        }));
        text.push("and a there".into());
        let mut ed = Editor::new(false);
        ed.set_text(text.join("\n"));
        ed.prepare_frame(40, 20);
        ed.open_replace();
        find_keys(&mut ed, "a ");
        assert_eq!(ed.cursor(), (0, 0));
        assert!(ed.handle_find_key(KeyInput::plain(Key::Tab)));
        for c in "LONGER ".chars() {
            find_keys(&mut ed, &c.to_string());
            ed.prepare_frame(40, 20);
            let rows = drawn_rows(&ed, 40);
            // The far row shows the whole preview, not a stale cut of it.
            let typed = &"LONGER "[..="LONGER ".find(c).unwrap()];
            let want = format!("and {typed}there");
            assert_eq!(rows[9], want, "while typing {c:?}");
        }
        assert!(ed.handle_find_key(KeyInput::plain(Key::Esc)));
        ed.prepare_frame(40, 20);
        let rows = drawn_rows(&ed, 40);
        assert_eq!(rows[0], "a here");
        assert_eq!(rows[9], "and a there");
    }

    /// `(col, cells)` stretches of row `row` of the last frame carrying `kind`.
    fn marked_cells(
        ed: &Editor,
        row: usize,
        kind: crate::logic::editor::overlay::OverlayKind,
    ) -> Vec<(usize, usize)> {
        let frame = ed.session.display.frame(40, true);
        let mut col = 0;
        let mut out: Vec<(usize, usize)> = Vec::new();
        for run in &frame.rows[row].runs {
            let w = unicode_width::UnicodeWidthStr::width(run.text.as_str());
            if run.overlays.contains(kind) && w > 0 {
                match out.last_mut() {
                    Some((c, n)) if *c + *n == col => *n += w,
                    _ => out.push((col, w)),
                }
            }
            col += w;
        }
        out
    }

    #[test]
    fn an_empty_replacement_previews_the_removal_point_not_the_old_match_columns() {
        // Regression (GUI manual gate): `ee` -> "" on "Three" showed "Thr",
        // but painted the old match's columns (the bar's current match, in
        // REAL text columns) as the selection over the preview row, so the
        // cells after "Thr" lit up. On a preview row only the preview's own
        // spans (preview columns) may paint.
        use crate::logic::editor::overlay::OverlayKind;
        let mut ed = Editor::new(false);
        ed.set_text("One Three apples\nsee eel ee".into());
        ed.open_replace();
        find_keys(&mut ed, "ee");
        assert!(ed.handle_find_key(KeyInput::plain(Key::Tab)));
        assert_eq!(
            ed.find_bar().unwrap().current_match(),
            Some(((0, 7), (0, 9)))
        );
        ed.prepare_frame(40, 10);
        assert_eq!(drawn_rows(&ed, 40), vec!["One Thr apples", "s l "]);
        // The removal point: one cell, right where "ee" was (after "Thr").
        assert_eq!(
            marked_cells(&ed, 0, OverlayKind::PreviewCurrent),
            vec![(7, 1)]
        );
        assert_eq!(marked_cells(&ed, 0, OverlayKind::Selection), vec![]);
        assert_eq!(marked_cells(&ed, 0, OverlayKind::CurrentMatch), vec![]);
        // Several matches on one row: the spans sit at the preview's columns.
        let (_, spans, _) = ed.find_frame_parts();
        let at: Vec<_> = spans.iter().map(|o| (o.row, o.start, o.end)).collect();
        assert_eq!(at, vec![(0, 7, 7), (1, 1, 1), (1, 2, 2), (1, 4, 4)]);
        // A non-empty replacement: the current span covers exactly it.
        find_keys(&mut ed, "X");
        ed.prepare_frame(40, 10);
        assert_eq!(drawn_rows(&ed, 40)[0], "One ThrX apples");
        assert_eq!(
            marked_cells(&ed, 0, OverlayKind::PreviewCurrent),
            vec![(7, 1)]
        );
        assert_eq!(marked_cells(&ed, 0, OverlayKind::Selection), vec![]);
        // Closing the bar: no preview, no stale highlight.
        assert!(ed.handle_find_key(KeyInput::plain(Key::Esc)));
        ed.prepare_frame(40, 10);
        assert_eq!(drawn_rows(&ed, 40)[0], "One Three apples");
        assert_eq!(marked_cells(&ed, 0, OverlayKind::Selection), vec![]);
    }

    #[test]
    fn paste_lands_in_the_bar_and_a_reload_drops_it() {
        let mut ed = Editor::new(false);
        ed.set_text("one two".into());
        ed.open_find();
        assert!(ed.paste_into_find("two\nthree"));
        assert_eq!(ed.find_bar().unwrap().query(), "two");
        assert_eq!(ed.get_text(), "one two");
        ed.set_text("other".into());
        assert!(!ed.find_open(), "the bar belonged to the old text");
    }

    #[test]
    fn commit_text_replaces_a_row_range_inserts_at_an_empty_one_and_deletes() {
        let mut ed = Editor::new(false);
        ed.set_text("first\nxe".into());
        // Replace "e" (row 1, chars 1..2).
        assert!(ed.commit_text(Some((1, 1..2)), "é"));
        assert_eq!(ed.get_text(), "first\nxé");
        // An empty range inserts at its start, not at the cursor.
        assert!(ed.commit_text(Some((0, 0..0)), ">"));
        assert_eq!(ed.get_text(), ">first\nxé");
        assert_eq!(ed.cursor(), (0, 1));
        // Empty text deletes the range.
        assert!(ed.commit_text(Some((0, 0..1)), ""));
        assert_eq!(ed.get_text(), "first\nxé");
        // No range: at the cursor.
        assert!(ed.commit_text(None, "!"));
        assert_eq!(ed.get_text(), "!first\nxé");
        // An unaddressable range changes nothing.
        assert!(!ed.commit_text(Some((9, 0..1)), "z"));
        assert_eq!(ed.get_text(), "!first\nxé");
    }

    #[test]
    fn a_failed_empty_range_commit_leaves_the_selection_alone() {
        let mut ed = Editor::new(false);
        ed.set_text("abc\nde".into());
        assert!(ed.buffer_mut().set_selection((0, 0), (0, 2)));
        ed.sync_highlight();
        let rev = ed.revision();
        assert!(!ed.commit_text(Some((9, 0..0)), "z"));
        assert_eq!(ed.get_text(), "abc\nde");
        assert_eq!(ed.revision(), rev);
        assert_eq!(ed.buffer().selection_range(), Some(((0, 0), (0, 2))));
        assert_eq!(ed.selection(), Some(((0, 0), (0, 2))));
    }

    #[test]
    fn commit_text_is_refused_outside_vim_insert() {
        let tx = tokio::sync::mpsc::unbounded_channel().0;
        let key = |c| KeyInput::plain(Key::Char(c));
        let mut ed = Editor::new(true);
        ed.set_text("abcd".into());
        // Normal: nothing is typed.
        let rev = ed.revision();
        assert!(!ed.commit_text(None, "x"));
        assert!(!ed.commit_text(Some((0, 1..2)), "x"));
        assert_eq!((ed.get_text().as_str(), ed.revision()), ("abcd", rev));
        // Visual over "ab": neither the text nor the selection moves (before
        // the guard, a ranged commit here cut one char more than its range).
        ed.handle_key(key('v'), &tx);
        ed.handle_key(key('l'), &tx);
        let shown = ed.selection();
        assert!(!ed.commit_text(Some((0, 0..1)), "Z"));
        assert!(!ed.commit_text(None, ""));
        assert_eq!(ed.get_text(), "abcd");
        assert_eq!(ed.selection(), shown);
        assert_eq!(ed.mode_label().as_deref(), Some("VISUAL"));
        // Insert: text input types as usual.
        ed.handle_key(KeyInput::plain(Key::Esc), &tx);
        ed.handle_key(key('0'), &tx);
        ed.handle_key(key('i'), &tx);
        assert!(ed.commit_text(None, "é"));
        assert_eq!(ed.get_text(), "éabcd");
        assert!(ed.commit_text(Some((0, 1..2)), "X"));
        assert_eq!(ed.get_text(), "éXbcd");
    }

    #[test]
    fn committed_text_joins_the_vim_insert_session_undo_group() {
        let tx = tokio::sync::mpsc::unbounded_channel().0;
        let key = |c| KeyInput::plain(Key::Char(c));
        let mut ed = Editor::new(true);
        ed.set_text("xy".into());
        ed.handle_key(key('i'), &tx);
        ed.handle_key(key('a'), &tx);
        ed.handle_key(key('b'), &tx);
        assert!(ed.commit_text(None, "é"));
        ed.handle_key(key('c'), &tx);
        ed.handle_key(key('d'), &tx);
        ed.handle_key(KeyInput::plain(Key::Esc), &tx);
        assert_eq!(ed.get_text(), "abécdxy");
        // One `u` takes back the whole Insert session, IME text included.
        ed.handle_key(key('u'), &tx);
        assert_eq!(ed.get_text(), "xy");
        // Committed text opening the session, then typing: still one group,
        // and the session before it stays its own.
        ed.handle_key(key('i'), &tx);
        assert!(ed.commit_text(None, "日"));
        ed.handle_key(key('z'), &tx);
        ed.handle_key(KeyInput::plain(Key::Esc), &tx);
        assert_eq!(ed.get_text(), "日zxy");
        ed.handle_key(key('u'), &tx);
        assert_eq!(ed.get_text(), "xy");
    }

    #[test]
    fn a_no_op_undo_leaves_the_revision_unchanged() {
        let mut ed = Editor::new(false);
        ed.set_text("abc".into());
        let rev = ed.revision();
        assert!(!ed.undo(), "nothing to undo after a load");
        assert_eq!(ed.revision(), rev);
        assert!(!ed.redo());
        assert_eq!(ed.revision(), rev);
    }

    #[test]
    fn commit_text_brings_a_scrolled_view_back_to_the_cursor() {
        let mut ed = long_note_editor(false);
        ed.prepare_frame(20, 3);
        assert!(ed.session_mut().display.scroll_by(6));
        ed.prepare_frame(20, 3);
        ed.commit_text(None, "x");
        ed.prepare_frame(20, 3);
        assert_eq!(ed.session().display.click_at_screen(0, 0).0, 0);
    }

    #[test]
    fn editor_prepare_frame_builds_the_frame_input_itself() {
        let mut ed = long_note_editor(false);
        ed.prepare_frame(20, 3);
        // The display took the editor's own cursor row: screen row 0 is row 0.
        assert_eq!(ed.session().display.click_at_screen(0, 0).0, 0);
    }

    #[test]
    fn handle_key_alone_reattaches_a_wheel_scrolled_view_to_the_cursor() {
        // A host with no wrapper (the GUI) gets cursor-following from the key.
        let tx = tx();
        let mut ed = long_note_editor(false);
        ed.prepare_frame(20, 3);
        assert!(ed.session_mut().display.scroll_by(6));
        ed.prepare_frame(20, 3);
        assert_eq!(ed.session().display.click_at_screen(0, 0).0, 6, "detached");
        // Left at the buffer start leaves the cursor exactly where it was.
        ed.handle_key(KeyInput::plain(Key::Left), &tx);
        ed.prepare_frame(20, 3);
        assert_eq!(ed.session().display.click_at_screen(0, 0).0, 0);
    }

    #[test]
    fn ignore_unmapped_key_reattaches_the_view_too() {
        let mut ed = long_note_editor(false);
        ed.prepare_frame(20, 3);
        ed.session_mut().display.scroll_by(6);
        ed.prepare_frame(20, 3);
        ed.ignore_unmapped_key();
        ed.prepare_frame(20, 3);
        assert_eq!(ed.session().display.click_at_screen(0, 0).0, 0);
    }

    #[test]
    fn mode_accessors_follow_the_input_interpreter() {
        let plain = Editor::new(false);
        assert!(!plain.is_vim());
        assert_eq!(plain.mode_label(), None);
        assert_eq!(plain.modal_is_insert(), None);
        assert_eq!(plain.pending_input_hint(), None);

        let mut vim = Editor::new(true);
        assert!(vim.is_vim());
        assert_eq!(vim.mode_label().as_deref(), Some("NORMAL"));
        assert_eq!(vim.modal_is_insert(), Some(false));
        vim.handle_key(KeyInput::plain(Key::Char('2')), &tx());
        assert_eq!(vim.pending_input_hint().as_deref(), Some("2"));
        vim.handle_key(KeyInput::plain(Key::Esc), &tx());
        vim.handle_key(KeyInput::plain(Key::Char('i')), &tx());
        assert_eq!(vim.modal_is_insert(), Some(true));
    }

    #[test]
    fn editor_exposes_buffer_cursor_and_revision_without_a_component() {
        let mut ed = Editor::new(false);
        assert!(!ed.buf.is_vim());
        ed.buffer_mut().insert_str("hello");
        assert_eq!(ed.buffer().text().to_string(), "hello");
        assert_eq!(ed.buffer().cursor(), (0, 5));
        let rev = ed.session().revs.current();
        ed.buffer_mut().move_cursor(CursorMove::Head);
        assert_eq!(ed.buffer().cursor(), (0, 0));
        assert_eq!(
            ed.session().revs.current(),
            rev,
            "a cursor move is not a revision"
        );
        assert!(ed.take_requests().is_empty());
        assert!(Editor::new(true).buf.is_vim());
    }

    #[test]
    fn editor_paste_text_replaces_the_selection() {
        let mut ed = Editor::new(false);
        ed.buffer_mut().insert_str("hello world");
        ed.buffer_mut().set_selection((0, 0), (0, 5));
        let rev = ed.session().revs.current();
        ed.paste_text("bye");
        assert_eq!(ed.get_text(), "bye world");
        assert_ne!(ed.session().revs.current(), rev, "the paste is a revision");
        assert!(ed.undo());
        assert_eq!(ed.get_text(), "hello world", "one undo restores the text");
    }

    #[test]
    fn undo_and_redo_bump_the_revision() {
        let mut ed = Editor::new(false);
        ed.buffer_mut().insert_str("hello");
        ed.after_edit();
        let r0 = ed.session().revs.current();
        assert!(ed.undo());
        let r1 = ed.session().revs.current();
        assert_ne!(r1, r0, "undo is a revision");
        assert!(ed.redo());
        assert_ne!(ed.session().revs.current(), r1, "redo is a revision");
        assert_eq!(ed.get_text(), "hello");
        assert!(!ed.redo(), "nothing left to redo");
    }

    #[test]
    fn settle_pointer_left_down_leaves_vim_visual() {
        let mut ed = Editor::new(true);
        ed.buffer_mut().insert_str("hello world");
        ed.buf.sync_mouse_selection(true);
        assert_eq!(ed.mode_label().as_deref(), Some("VISUAL"));
        ed.settle_pointer(true);
        assert_eq!(ed.mode_label().as_deref(), Some("NORMAL"));
    }

    #[test]
    fn settle_pointer_without_left_down_keeps_visual() {
        let mut ed = Editor::new(true);
        ed.buffer_mut().insert_str("hello world");
        ed.buf.sync_mouse_selection(true);
        ed.settle_pointer(false);
        assert_eq!(ed.mode_label().as_deref(), Some("VISUAL"));
    }

    #[test]
    fn editor_paste_text_with_empty_text_leaves_the_selection_alone() {
        let mut ed = Editor::new(false);
        ed.buffer_mut().insert_str("hello world");
        ed.buffer_mut().set_selection((0, 0), (0, 5));
        ed.paste_text("");
        assert_eq!(ed.get_text(), "hello world");
        assert!(ed.buffer().selection_range().is_some());
    }

    #[test]
    fn editor_insert_at_cursor_with_empty_text_is_a_no_op() {
        let mut ed = Editor::new(false);
        ed.buffer_mut().insert_str("hello world");
        ed.buffer_mut().set_selection((0, 0), (0, 5));
        ed.insert_at_cursor("");
        assert_eq!(ed.get_text(), "hello world");
        assert!(ed.buffer().selection_range().is_some());
    }

    #[test]
    fn editor_apply_text_action_bold_wraps_the_selection() {
        let mut ed = Editor::new(false);
        ed.buffer_mut().insert_str("hello world");
        ed.buffer_mut().set_selection((0, 0), (0, 5));
        ed.apply_text_action(TextAction::Bold);
        assert_eq!(ed.get_text(), "**hello** world");
    }

    // ---- Input -------------------------------------------------------------

    fn tx() -> AppTx {
        tokio::sync::mpsc::unbounded_channel().0
    }

    fn typed(c: char) -> KeyInput {
        KeyInput::plain(Key::Char(c))
    }

    #[test]
    fn cmd_v_pastes_like_ctrl_v() {
        for vim in [false, true] {
            let _clip = crate::logic::clipboard::set_test_clipboard_text(Ok("pasted".into()));
            let mut ed = Editor::new(vim);
            if vim {
                ed.handle_key(typed('i'), &tx());
            }
            ed.handle_key(KeyInput::new(Key::Char('v'), Mods::SUPER), &tx());
            assert_eq!(ed.get_text(), "pasted", "vim = {vim}");
        }
    }

    fn pointer(kind: PointerKind, col: u16, row: u16) -> PointerInput {
        PointerInput {
            kind,
            col,
            row,
            mods: Mods::NONE,
        }
    }

    /// Lay `ed` out at `width` x `height`, as a frame would.
    fn lay_out(ed: &mut Editor, width: u16, height: u16) {
        let snap =
            EditorSnapshot::of_buffer(ed.buffer().text().clone(), ed.cursor(), ed.revision());
        ed.session_mut().display.update_sized(&snap, width, height);
    }

    #[test]
    fn editor_handle_key_types_and_undoes_without_a_component() {
        let mut ed = Editor::new(false);
        let tx = tx();
        for c in ['a', 'b'] {
            let (handled, path) = ed.handle_key(typed(c), &tx);
            assert_eq!(handled, Handled::Consumed);
            assert_eq!(
                path,
                KeyPath::Direct {
                    text_changed: true,
                    cursor_moved: true,
                    pasted: false
                }
            );
        }
        assert_eq!(ed.get_text(), "ab");
        assert!(ed.is_dirty());
        ed.handle_key(KeyInput::ctrl('z'), &tx);
        assert_eq!(ed.get_text(), "", "one undo takes the typing run back");
    }

    #[test]
    fn editor_vim_slash_queues_open_search() {
        let mut ed = Editor::new(true);
        let (handled, path) = ed.handle_key(typed('/'), &tx());
        assert_eq!(handled, Handled::Consumed);
        assert_eq!(path, KeyPath::VimOther);
        assert_eq!(ed.take_host_requests(), vec![EditorRequest::OpenSearch]);
        assert!(ed.take_requests().is_empty(), "drained once");
    }

    #[test]
    fn editor_vim_colon_queues_open_palette_and_sends_no_message() {
        let mut ed = Editor::new(true);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (handled, path) = ed.handle_key(typed(':'), &tx);
        assert_eq!(handled, Handled::Consumed);
        assert_eq!(path, KeyPath::VimOther);
        assert_eq!(ed.take_host_requests(), vec![EditorRequest::OpenPalette]);
        assert!(
            rx.try_recv().is_err(),
            "the editor no longer messages the app"
        );
    }

    #[test]
    fn editor_wheel_scroll_queues_viewport_scrolled() {
        let mut ed = Editor::new(false);
        let text: String = (0..100)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        ed.set_text(text);
        lay_out(&mut ed, 40, 10);
        let cursor = ed.cursor();
        let handled = ed.handle_pointer(&pointer(PointerKind::ScrollDown, 5, 5), &tx());
        assert_eq!(handled, Handled::Consumed);
        assert_eq!(
            ed.take_host_requests(),
            vec![EditorRequest::ViewportScrolled]
        );
        assert_eq!(ed.cursor(), cursor, "the wheel never moves the cursor");
    }

    #[test]
    fn editor_unmapped_key_ends_the_run_and_is_consumed() {
        let mut ed = Editor::new(false);
        let tx = tx();
        ed.handle_key(typed('a'), &tx);
        assert_eq!(
            ed.ignore_unmapped_key(),
            (
                Handled::Consumed,
                KeyPath::Direct {
                    text_changed: false,
                    cursor_moved: false,
                    pasted: false
                }
            )
        );
        ed.handle_key(typed('b'), &tx);
        assert_eq!(ed.get_text(), "ab");
        ed.handle_key(KeyInput::ctrl('z'), &tx);
        assert_eq!(
            ed.get_text(),
            "a",
            "the unmapped key split the run: one undo takes back only `b`"
        );
    }

    #[test]
    fn editor_pointer_click_places_the_cursor() {
        let mut ed = Editor::new(false);
        ed.set_text("hello world\nsecond line".to_string());
        lay_out(&mut ed, 40, 10);
        let handled = ed.handle_pointer(&pointer(PointerKind::Down(Button::Left), 3, 1), &tx());
        assert_eq!(handled, Handled::Consumed);
        assert_eq!(ed.cursor(), (1, 3));
        assert!(
            ed.selection().is_none_or(|(start, end)| start == end),
            "a press selects nothing"
        );
        assert!(ed.take_requests().is_empty());
    }

    #[test]
    fn a_click_then_vim_motion_selects_nothing() {
        for vim in [true, false] {
            let mut ed = Editor::new(vim);
            ed.set_text("one\ntwo\nthree\nfour".to_string());
            lay_out(&mut ed, 40, 10);
            let tx = tx();
            ed.handle_pointer(&pointer(PointerKind::Down(Button::Left), 1, 2), &tx);
            ed.settle_pointer(true);
            ed.handle_pointer(&pointer(PointerKind::Up(Button::Left), 1, 2), &tx);
            ed.settle_pointer(false);
            if vim {
                assert_eq!(ed.mode_label().as_deref(), Some("NORMAL"));
            }
            ed.handle_key(
                if vim {
                    typed('k')
                } else {
                    KeyInput::plain(Key::Up)
                },
                &tx,
            );
            assert_eq!(ed.cursor(), (1, 1));
            assert!(
                ed.selection().is_none_or(|(a, b)| a == b),
                "motion after a click must not select: {:?}",
                ed.selection()
            );
            assert!(ed.painted_selection().is_none_or(|(a, b)| a == b));
        }
    }

    #[test]
    fn editor_context_menu_request_survives_take_host_requests() {
        let mut ed = Editor::new(false);
        ed.set_text("hello".to_string());
        lay_out(&mut ed, 40, 10);
        let handled = ed.handle_pointer(&pointer(PointerKind::Down(Button::Right), 1, 0), &tx());
        assert_eq!(handled, Handled::Consumed);
        assert!(ed.take_host_requests().is_empty());
        assert_eq!(ed.take_requests(), vec![EditorRequest::ContextMenu]);
    }

    // ---- Moved from the component: pure editor behaviour ------------------

    fn make_editor() -> Editor {
        Editor::new(false)
    }

    fn make_vim_editor() -> Editor {
        Editor::new(true)
    }

    fn dummy_tx() -> AppTx {
        tokio::sync::mpsc::unbounded_channel().0
    }

    fn note(name: &str) -> kimun_core::nfs::VaultPath {
        kimun_core::nfs::VaultPath::note_path_from(name)
    }

    /// Selects the char-coordinate range `start..end`.
    fn select_range(editor: &mut Editor, start: (usize, usize), end: (usize, usize)) {
        let ta = editor.buffer_mut();
        ta.cancel_selection();
        ta.move_cursor(CursorMove::Jump(start.0, start.1));
        ta.start_selection();
        ta.move_cursor(CursorMove::Jump(end.0, end.1));
        assert!(ta.selection_range().is_some());
    }

    fn send_char(editor: &mut Editor, c: char) {
        let _ = editor.handle_key(typed(c), &dummy_tx());
    }

    fn press(editor: &mut Editor, keys: &str) {
        let tx = dummy_tx();
        for c in keys.chars() {
            editor.handle_key(typed(c), &tx);
        }
    }

    /// Helper: place cursor at a specific column on the first row.
    fn place_cursor_at_col(editor: &mut Editor, col: usize) {
        let ta = editor.buffer_mut();
        ta.move_cursor(CursorMove::Head);
        for _ in 0..col {
            ta.move_cursor(CursorMove::Forward);
        }
    }

    fn vim_mode(editor: &Editor) -> EditorMode {
        match &editor.buf.input {
            crate::logic::editor::textarea::InputInterpreter::Vim(e) => e.mode().clone(),
            _ => panic!("expected Vim input interpreter"),
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
        assert_eq!(editor.selection(), Some(((0, 1), (0, 6))));
    }

    #[test]
    fn chained_brackets_build_a_wikilink() {
        let mut editor = make_editor();
        editor.set_text("my note".to_string());
        select_range(&mut editor, (0, 0), (0, 7));
        send_char(&mut editor, '[');
        send_char(&mut editor, '[');
        assert_eq!(editor.get_text(), "[[my note]]");
        assert_eq!(editor.selection(), Some(((0, 2), (0, 9))));
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
        assert_eq!(editor.selection(), Some(((0, 2), (0, 6))));
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
    fn text_action_keeps_selection_on_inner_text() {
        // Bold/Italic/Strikethrough route through the same wrap mechanism as
        // auto-surround: the inner text stays selected so wraps chain.
        let mut editor = make_editor();
        editor.set_text("bold word".to_string());
        select_range(&mut editor, (0, 0), (0, 4));
        editor.apply_text_action(TextAction::Bold);
        assert_eq!(editor.get_text(), "**bold** word");
        assert_eq!(editor.selection(), Some(((0, 2), (0, 6))));
    }

    #[test]
    fn unsupported_text_action_is_noop() {
        let mut editor = make_editor();
        editor.set_text("hello".to_string());
        editor.apply_text_action(TextAction::Underline);
        assert_eq!(editor.get_text(), "hello");
    }

    #[test]
    fn jump_to_row_moves_the_view_cursor() {
        // The OUTLINE drawer reads the position back through the view snapshot.
        let mut ed = make_editor();
        ed.set_text("intro\n# Top\nbody\n## Sub One\nmore\n".to_string());
        ed.jump_to_row(3);
        assert_eq!(ed.view_snapshot().cursor.0, 3);
        // A click's zero-width selection must not turn the jump into a span
        // the next keystroke replaces.
        select_range(&mut ed, (2, 1), (2, 1));
        ed.jump_to_row(1);
        assert_eq!(ed.selection(), None);
        assert_eq!(ed.view_snapshot().cursor.0, 1);
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
            matches!(result, Some(FollowTarget::Link { .. })),
            "expected Link variant, got {result:?}"
        );
    }

    /// The follow target carries the link's kind: it decides whether the
    /// target names a note (wikilink) or is a path (markdown).
    #[test]
    fn follow_target_at_cursor_carries_the_link_kind() {
        use kimun_core::note::scan::LinkSpanKind;
        let mut editor = make_editor();
        editor.set_text("[[a/dup]] [x](dup.md) ![i](p.png)".to_string());
        for (col, target, kind) in [
            (3, "a/dup", LinkSpanKind::WikiLink),
            (12, "dup.md", LinkSpanKind::Markdown),
            (24, "p.png", LinkSpanKind::Image),
        ] {
            place_cursor_at_col(&mut editor, col);
            assert_eq!(
                editor.follow_target_at_cursor(),
                Some(FollowTarget::Link {
                    target: target.to_string(),
                    kind
                }),
                "col {col}"
            );
        }
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
            matches!(result, Some(FollowTarget::Link { .. })),
            "expected Link variant for markdown link fragment, got {result:?}"
        );
    }

    /// Another note is another note, whatever its text: the one on screen
    /// before must leave no undo history behind. With the text-equality
    /// shortcut, `u` in note B restored note A's text — and autosave would
    /// have written it to B.
    #[test]
    fn a_note_with_the_same_text_does_not_inherit_undo_history() {
        let mut editor = make_vim_editor();
        editor.open_note(note("alpha"), "one".to_string());
        press(&mut editor, "x");
        assert_eq!(editor.get_text(), "ne");
        editor.open_note(note("beta"), "ne".to_string());
        press(&mut editor, "u");
        assert_eq!(editor.get_text(), "ne", "beta has nothing to undo");
    }

    /// Opening the note already on screen is a reload, not a switch: its
    /// undo history (and `gv`) stay.
    #[test]
    fn reopening_the_same_note_keeps_its_history() {
        let mut editor = make_vim_editor();
        editor.open_note(note("alpha"), "one".to_string());
        press(&mut editor, "x");
        editor.open_note(note("alpha"), "ne".to_string());
        press(&mut editor, "u");
        assert_eq!(editor.get_text(), "one");
    }

    /// A rename is the same note under a new name: history survives it, and
    /// opening it under the new name afterwards is still the same note.
    #[test]
    fn renaming_the_open_note_keeps_its_history() {
        let mut editor = make_vim_editor();
        editor.open_note(note("alpha"), "one".to_string());
        press(&mut editor, "x");
        editor.renamed_to(note("gamma"));
        editor.set_text("ne".to_string());
        editor.open_note(note("gamma"), "ne".to_string());
        press(&mut editor, "u");
        assert_eq!(editor.get_text(), "one");
    }

    /// Emphasis over a charwise selection keeps the inner text selected in
    /// Visual, so a second emphasis wraps exactly it — not its closing marker.
    #[test]
    fn two_emphases_over_a_visual_selection_nest_cleanly() {
        let mut editor = make_vim_editor();
        editor.set_text("hello world".to_string());
        press(&mut editor, "ve");
        editor.apply_text_action(TextAction::Bold);
        assert_eq!(vim_mode(&editor), EditorMode::Visual);
        editor.apply_text_action(TextAction::Strikethrough);
        assert_eq!(editor.get_text(), "**~~hello~~** world");
    }

    /// `[` `[` over a Visual selection builds a wikilink: the first wrap
    /// leaves the inner text selected *in Visual*, where the second `[` wraps
    /// again.
    #[test]
    fn surround_chains_into_a_wikilink_from_visual() {
        let mut editor = make_vim_editor();
        editor.set_text("hello world".to_string());
        press(&mut editor, "ve[");
        assert_eq!(vim_mode(&editor), EditorMode::Visual);
        press(&mut editor, "[");
        assert!(
            editor.get_text().starts_with("[[hello]]"),
            "{:?}",
            editor.get_text()
        );
    }

    /// After the wrap, the mode and the highlight agree: a motion extends a
    /// Visual selection, not an invisible one under a Normal footer.
    #[test]
    fn a_motion_after_a_surround_extends_the_visual_selection() {
        let mut editor = make_vim_editor();
        editor.set_text("hello world".to_string());
        press(&mut editor, "ve*");
        assert_eq!(editor.get_text(), "*hello* world");
        assert_eq!(vim_mode(&editor), EditorMode::Visual);
        assert_eq!(editor.selection(), Some(((0, 1), (0, 6))), "the inner text");
    }

    /// `V` then a pair char on an empty row has nothing to wrap: nothing
    /// happens — no literal char typed while in Normal.
    #[test]
    fn a_pair_char_over_an_empty_visual_row_does_nothing() {
        for keys in ["jV*", "jv*"] {
            let mut editor = make_vim_editor();
            editor.set_text("a\n\nb".to_string());
            press(&mut editor, keys);
            assert_eq!(editor.get_text(), "a\n\nb", "{keys}");
            assert_ne!(
                vim_mode(&editor),
                EditorMode::Normal,
                "{keys}: still selecting"
            );
        }
    }

    #[test]
    fn emphasis_over_an_empty_visual_row_does_nothing() {
        let mut editor = make_vim_editor();
        editor.set_text("a\n\nb".to_string());
        press(&mut editor, "jv");
        editor.apply_text_action(TextAction::Bold);
        assert_eq!(editor.get_text(), "a\n\nb");
        assert_eq!(vim_mode(&editor), EditorMode::Visual);
    }

    /// A wrap whose text ends in a line break cannot stay selected in Visual
    /// (a Visual range cannot end on a newline); it lands in Normal rather
    /// than chaining into an unbalanced second wrap.
    #[test]
    fn a_wrap_ending_in_a_line_break_does_not_chain() {
        let mut editor = make_vim_editor();
        editor.set_text("a\n\nz".to_string());
        press(&mut editor, "vj*");
        assert_eq!(editor.get_text(), "*a\n*\nz");
        assert_eq!(vim_mode(&editor), EditorMode::Normal);
        press(&mut editor, "*");
        assert_eq!(editor.get_text(), "*a\n*\nz");
    }

    /// Emphasis from `V` wraps the row and keeps its text selected in Visual.
    #[test]
    fn emphasis_from_visual_line_keeps_the_row_text_selected() {
        let mut editor = make_vim_editor();
        editor.set_text("hello\nx".to_string());
        press(&mut editor, "V");
        editor.apply_text_action(TextAction::Bold);
        assert_eq!(editor.get_text(), "**hello**\nx");
        assert_eq!(vim_mode(&editor), EditorMode::Visual);
        assert_eq!(editor.selection(), Some(((0, 2), (0, 7))));
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
}
