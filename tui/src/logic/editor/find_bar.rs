//! The **find bar**: searching and replacing inside the open buffer,
//! toolkit-free.
//!
//! A module rather than a cluster of methods on the editor. The bar reaches
//! outside itself for exactly one thing — the **rope buffer** — so it takes one
//! as a parameter. [`crate::logic::editor::Editor`] owns the open bar and wires
//! it to its buffer; a front end only paints it (the two [`LineField`]s, the
//! [`SearchStatus`], the focus) and turns its own key events into [`KeyInput`].
//!
//! The bar owns its **current match** rather than writing the editor's
//! selection. A current match is not a selection: it cannot be extended,
//! copied, or typed over, and rendering it as one is why a mouse drag could
//! hand the bar a multi-row range it had no way to represent.

use crate::logic::editor::claim::char_col_to_byte;
use crate::logic::editor::find_replace;
use crate::logic::editor::line_field::{FieldOutcome, LineField};
use crate::logic::editor::overlay::{Overlay, OverlayKind};
use crate::logic::editor::rope_buffer::{CursorMove, RopeBuffer};
use crate::logic::input::{Key, KeyInput};

/// What the editor must do after handing the bar a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyOutcome {
    /// The bar is finished; the editor should drop it.
    pub close: bool,
}

/// Everything the bar wants painted this frame, in one value.
#[derive(Debug, Default)]
pub struct BarOverlay {
    /// The **replace preview**'s substituted lines, when one is showing.
    pub preview: Option<find_replace::Preview>,
    /// **Find pattern** matches, in logical buffer coordinates.
    pub matches: Vec<(usize, usize, usize)>,
}

impl BarOverlay {
    /// The preview's lines plus its spans as `Preview` / `PreviewCurrent`
    /// overlays, and the matches as `Match` overlays — the shape
    /// [`crate::logic::editor::session::FrameInput`] takes. Matches are
    /// dropped while a preview shows: `prepare_frame` ignores them then.
    pub fn into_frame_parts(self) -> (Option<Vec<String>>, Vec<Overlay>, Vec<Overlay>) {
        let (preview, preview_overlays) = match self.preview {
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
        let matches = if preview.is_some() {
            Vec::new()
        } else {
            self.matches
                .iter()
                .map(|&(row, start, end)| Overlay::new(row, start, end, OverlayKind::Match))
                .collect()
        };
        (preview, preview_overlays, matches)
    }
}

impl Default for FindBar {
    fn default() -> Self {
        Self::new()
    }
}

/// Which of the find bar's fields owns the keyboard. Only meaningful once a
/// **replace field** has been revealed — a find-only bar is always `Find`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarFocus {
    Find,
    Replace,
}

/// What the **find pattern** currently resolves to, for the bar's status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchStatus {
    /// The query is empty.
    Empty,
    /// The pattern matches (see [`FindBar::match_count`]).
    Match,
    /// The pattern compiles but matches nothing.
    NoMatch,
    /// The pattern does not compile; the regex error message.
    Invalid(String),
}

impl SearchStatus {
    fn from_found(found: bool) -> Self {
        if found { Self::Match } else { Self::NoMatch }
    }
}

// ---- Wording: what every front end prints ---------------------------------
//
// The bar's key map lives here, so the words that describe it do too: a front
// end picks a colour per [`StatusTone`] and lays the strings out, but never
// rewords them, so the TUI and the GUI read the same.

/// The find row's prompt.
pub const FIND_PROMPT: &str = "Find: ";
/// The replace row's prompt.
pub const REPLACE_PROMPT: &str = "Replace: ";
/// The find row's key hints, shown after a match count while only finding.
pub const FIND_HINTS: &str = "  [Enter] next  [Shift+Enter] prev  [Tab] replace  [Esc] close";
/// The replace row's key hints.
pub const REPLACE_HINTS: &str =
    "  [Enter] replace  [Shift+Enter] skip  [Ctrl+A] all  [Tab] field  [Esc] close";
/// The replace row's hint while a **replace all** with an empty replacement
/// waits for its confirming press.
pub const REPLACE_HINTS_ARMED: &str = "  delete every match? [Ctrl+A] confirm  [Esc] cancel";

/// Where the find bar stands among the **find pattern**'s matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchPosition {
    /// The **current match**'s place among them, 1-based; `None` when the
    /// bar has no current match (after a replace all, say).
    pub current: Option<usize>,
    /// Matches across the whole buffer.
    pub total: usize,
}

/// How a piece of bar text reads: quiet information, or a problem the user
/// has to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusTone {
    Muted,
    Error,
}

#[derive(Debug)]
pub struct FindBar {
    input: LineField,
    /// `Some` once the **replace field** is revealed. Its presence is what
    /// puts the bar in replace mode.
    replace: Option<LineField>,
    focus: BarFocus,
    status: SearchStatus,
    /// The compiled **find pattern**. `None` while the query is empty or does
    /// not compile — the one place the regex lives, so the highlighter, the
    /// counter and the replacer can never disagree about what matches.
    pattern: Option<find_replace::FindPattern>,
    /// Matches across the whole buffer, refreshed with the pattern. Shown
    /// before a **replace all** so the keystroke is an informed one.
    match_count: usize,
    /// The **current match** — the occurrence the cursor sits on, which the bar
    /// owns rather than writing into the editor's selection.
    current: Option<((usize, usize), (usize, usize))>,
    /// Set when a **replace all** with an empty replacement was requested but
    /// not yet confirmed. An empty field is indistinguishable from "still
    /// typing", so that one destructive case arms rather than commits.
    armed_empty: bool,
}

// ---- Queries: what a front end paints -------------------------------------
impl FindBar {
    pub fn new() -> Self {
        Self {
            input: LineField::new(),
            replace: None,
            focus: BarFocus::Find,
            status: SearchStatus::Empty,
            pattern: None,
            match_count: 0,
            current: None,
            armed_empty: false,
        }
    }

    /// The find field (the query and its cursor).
    pub fn find_field(&self) -> &LineField {
        &self.input
    }

    /// The query text.
    pub fn query(&self) -> &str {
        self.input.value()
    }

    /// The replace field, once revealed.
    pub fn replace_field(&self) -> Option<&LineField> {
        self.replace.as_ref()
    }

    /// Which field owns the keyboard.
    pub fn focus(&self) -> BarFocus {
        self.focus
    }

    /// What the pattern resolves to.
    pub fn status(&self) -> &SearchStatus {
        &self.status
    }

    /// Matches across the whole buffer.
    pub fn match_count(&self) -> usize {
        self.match_count
    }

    /// Whether a **replace all** with an empty replacement is waiting for its
    /// confirming second press.
    pub fn is_armed(&self) -> bool {
        self.armed_empty
    }

    /// The **current match**, for the editor to hand the view as its selection.
    pub fn current_match(&self) -> Option<((usize, usize), (usize, usize))> {
        self.current
    }

    /// Where the bar stands among its matches, for a status line (`3/12`).
    /// `None` while nothing matches (the empty query, an invalid pattern, no
    /// match). Costs one pass over the buffer, as the match highlight does.
    pub fn match_position(&self, buf: &RopeBuffer) -> Option<MatchPosition> {
        if self.status != SearchStatus::Match {
            return None;
        }
        let current = match (self.current, self.pattern.as_ref()) {
            (Some(((row, col), _)), Some(pattern)) => pattern
                .match_spans(buf.text().lines())
                .iter()
                .position(|&(r, start, _)| r == row && start == col)
                .map(|i| i + 1),
            _ => None,
        };
        Some(MatchPosition {
            current,
            total: self.match_count,
        })
    }

    /// The compiled **find pattern**, when the query compiles and is non-empty.
    pub fn pattern(&self) -> Option<&find_replace::FindPattern> {
        self.pattern.as_ref()
    }

    pub fn is_replacing(&self) -> bool {
        self.replace.is_some()
    }

    /// The replacement text, or `""` when the field is revealed but empty
    /// (which means deletion, not inaction).
    pub fn replacement(&self) -> &str {
        self.replace.as_ref().map(|r| r.value()).unwrap_or("")
    }

    /// What the pattern matches, in words: `3 matches  any case`,
    /// `no match  exact case`, `invalid regex: …`. `None` while the query is
    /// empty. Smartcase is never silent: the note says which way it resolved.
    pub fn status_text(&self) -> Option<(String, StatusTone)> {
        let case_note = match &self.pattern {
            Some(p) if p.case_sensitive() => "  exact case",
            Some(_) => "  any case",
            None => "",
        };
        match &self.status {
            SearchStatus::Empty => None,
            SearchStatus::Invalid(msg) => {
                Some((format!("invalid regex: {msg}"), StatusTone::Error))
            }
            SearchStatus::NoMatch => Some((format!("no match{case_note}"), StatusTone::Error)),
            SearchStatus::Match => {
                let n = self.match_count;
                let plural = if n == 1 { "match" } else { "matches" };
                Some((format!("{n} {plural}{case_note}"), StatusTone::Muted))
            }
        }
    }

    /// The replace row's hint: the key map, or the armed confirm.
    pub fn replace_hint(&self) -> (&'static str, StatusTone) {
        if self.armed_empty {
            (REPLACE_HINTS_ARMED, StatusTone::Error)
        } else {
            (REPLACE_HINTS, StatusTone::Muted)
        }
    }

    /// Everything the bar wants painted this frame.
    /// Costs one O(rows) pass per frame while the bar is open — the preview when
    /// replacing, the match scan otherwise, never both. That is inherent to
    /// the replace preview being a synthetic whole-buffer snapshot.
    ///
    /// Memoizing on (revision, pattern, replacement, current) was considered and
    /// left undone: the bar redraws on input, and find-bar input almost always
    /// changes the pattern, so it would miss on the common case and help only on
    /// redraws driven by something else (autosave, a background parse or layout
    /// install). A hit would still clone the preview's rows, so it reduces the
    /// constant rather than the order. Measure it with a bench that renders
    /// with the bar open before building it.
    pub fn overlay(&self, buf: &RopeBuffer) -> BarOverlay {
        let preview = self.preview(buf);
        let matches = if preview.is_none() {
            self.pattern
                .as_ref()
                .map(|p| p.match_spans(buf.text().lines()))
                .unwrap_or_default()
        } else {
            // Those columns already carry the preview colour, which is the
            // more important fact about them.
            Vec::new()
        };
        BarOverlay { preview, matches }
    }

    /// Build the **replace preview** for this frame: the note as it would read
    /// with every match replaced, plus where each replacement landed.
    ///
    /// Returns `None` whenever there is nothing to preview, in which case the
    /// caller renders the real buffer.
    pub fn preview(&self, buf: &RopeBuffer) -> Option<find_replace::Preview> {
        if !self.is_replacing() {
            return None;
        }
        let pattern = self.pattern.as_ref()?;
        let current = self.current.map(|((row, col), _)| (row, col));
        let preview =
            // `text().lines()` rather than a materialised vector: this runs on
            // every frame the preview is showing, and building a copy of the note
            // to read it doubled the cost of a feature that already rebuilds one.
            find_replace::build_preview(pattern, buf.text().lines(), self.replacement(), current);
        if preview.spans.is_empty() {
            return None;
        }
        Some(preview)
    }
}

// ---- Actions ---------------------------------------------------------------
impl FindBar {
    /// Reveal the **replace field**, putting the bar in replace mode. Focus
    /// lands in the find field while the pattern is still empty — you cannot
    /// usefully type a replacement for nothing.
    pub fn reveal_replace(&mut self) {
        if self.replace.is_none() {
            self.replace = Some(LineField::new());
        }
        self.focus = if self.input.is_empty() {
            BarFocus::Find
        } else {
            BarFocus::Replace
        };
    }

    /// Insert clipboard text into the focused field.
    ///
    /// The fields are single-line; a multi-line clipboard collapses to its
    /// first line rather than silently pasting nothing.
    pub fn paste(&mut self, text: &str, buf: &mut RopeBuffer) {
        let line = text.lines().next().unwrap_or_default().to_string();
        let focus = self.focus;
        self.focused_input_mut().insert_str(&line);
        if focus == BarFocus::Find {
            self.refresh_pattern(buf);
        }
    }

    /// Step to the next (or, `backward`, previous) match.
    pub fn advance(&mut self, buf: &mut RopeBuffer, backward: bool) {
        if self.input.is_empty() {
            return;
        }
        let found = if backward {
            buf.search_back(false)
        } else {
            buf.search_forward(false)
        };
        self.status = SearchStatus::from_found(found);
        self.highlight_current_match(buf, found);
    }

    /// Rewrite every match in the buffer — the `Ctrl+A` action. Returns the
    /// number replaced, or `None` when there was nothing to do.
    pub fn replace_all(&mut self, buf: &mut RopeBuffer) -> Option<usize> {
        let pattern = self.pattern.as_ref()?;
        let replacement = self.replacement().to_string();

        let before: Vec<String> = buf.rows();
        let (after, count) = find_replace::replace_all(pattern, &before, &replacement)?;

        // Restore the reading position afterwards. The naive path leaves the
        // cursor at the end of the inserted chunk — i.e. the bottom of the
        // note — which turns a bulk edit into a navigation.
        let (cur_row, cur_col) = buf.cursor();

        let joined = after.join("\n");
        // One **undo group** spanning the whole rewrite. The buffer also
        // derives `bulk` from it — a replace all rewrites rows the cursor does
        // not point at, which is exactly what `compute_damage_range`'s cursor
        // fast path assumes cannot happen.
        buf.edit(|buf| {
            buf.select_all();
            buf.insert_str(&joined);
            buf.cancel_selection();
        });
        if buf.text().to_string() != joined {
            return None;
        }

        // The row stays valid because a replace all never changes the line
        // count: the pattern cannot span a newline and the replace field is
        // single-line.
        let row = cur_row.min(after.len().saturating_sub(1));
        let col = cur_col.min(after[row].chars().count());
        buf.move_cursor(CursorMove::Jump(row, col));

        self.current = None;
        self.refresh_match_count(buf);
        Some(count)
    }

    /// Handle one key. The bar owns every key while it holds the **editor
    /// claim**, so this always consumes; the outcome says only whether the
    /// editor should drop the bar.
    ///
    /// The key map is the same on every engine and front end: `Tab` reveals
    /// the replace field then cycles focus; `Ctrl+A` replaces all (arming
    /// first when the replacement is empty); `Ctrl+Z` / `Ctrl+Y` /
    /// `Ctrl+Shift+Z` undo and redo; `Enter` / `Shift+Enter` step (or, while
    /// replacing, replace / skip); `Esc` disarms, then closes.
    pub fn handle_key(&mut self, key: KeyInput, buf: &mut RopeBuffer) -> KeyOutcome {
        let shift = key.mods.shift;
        let ctrl = key.mods.ctrl;
        let stay = KeyOutcome::default();

        // `Tab` reveals the replace field, then cycles focus between the two.
        // `LineField` never consumes it, so it is ours to take.
        if key.key == Key::Tab {
            if self.replace.is_none() {
                self.reveal_replace();
            } else {
                self.focus = match self.focus {
                    BarFocus::Find => BarFocus::Replace,
                    BarFocus::Replace => BarFocus::Find,
                };
            }
            return stay;
        }

        // Replace all. `LineField` deliberately bubbles Ctrl-modified chars
        // rather than typing them, so this is the documented seam. The chord is
        // shared with the editor's select-all and resolved by focus, as the
        // row-declared yank does for Ctrl+Y.
        if ctrl && matches!(key.key, Key::Char('a') | Key::Char('A')) {
            self.replace_all_key(buf);
            return stay;
        }

        // Undo / redo of the bar's OWN edits. The bar consumes every key, so
        // without this it swallows Ctrl+Z and strands the user on a note it
        // just rewrote — undo would only work after they thought to press Esc.
        if ctrl {
            match key.key {
                Key::Char('z') if !shift => {
                    if buf.undo() {
                        self.after_history_step(buf);
                    }
                    return stay;
                }
                Key::Char('y') | Key::Char('Z') => {
                    if buf.redo() {
                        self.after_history_step(buf);
                    }
                    return stay;
                }
                _ => {}
            }
        }

        let replacing = self.is_replacing();
        match self.focused_input_mut().handle_key(key) {
            FieldOutcome::Cancel => {
                // Esc disarms first, so a mis-aimed Ctrl+A never costs the bar.
                if self.armed_empty {
                    self.armed_empty = false;
                } else {
                    // The editor drops the bar and clears its selection; the
                    // pattern stays on the buffer so vim's `n`/`N` still work.
                    return KeyOutcome { close: true };
                }
            }
            FieldOutcome::Submit => {
                if replacing {
                    if shift {
                        // Skip: advance without writing.
                        self.advance(buf, false);
                    } else {
                        self.replace_current(buf);
                    }
                } else {
                    self.advance(buf, shift);
                }
            }
            FieldOutcome::Changed => {
                // Editing either field disarms a pending confirm and
                // invalidates the preview, which rebuilds from state anyway.
                if self.focus == BarFocus::Find {
                    self.refresh_pattern(buf);
                } else {
                    self.armed_empty = false;
                }
            }
            FieldOutcome::Consumed | FieldOutcome::NotConsumed => {}
        }
        stay
    }
}

// ---- Internals -------------------------------------------------------------
impl FindBar {
    /// The field the keyboard is currently driving.
    fn focused_input_mut(&mut self) -> &mut LineField {
        match self.focus {
            BarFocus::Replace if self.replace.is_some() => {
                self.replace.as_mut().expect("checked above")
            }
            _ => &mut self.input,
        }
    }

    /// Re-derive what an undo or redo invalidated. The **current match**
    /// pointed at text the history step just changed, so recompute it against
    /// the cursor rather than leaving a highlight over whatever now sits there.
    fn after_history_step(&mut self, buf: &RopeBuffer) {
        self.refresh_match_count(buf);
        self.current = buf.match_at_cursor();
    }

    /// Recompile the **find pattern** under smartcase, refresh the match count,
    /// push it to the buffer so its stepping uses the same regex the
    /// highlighter and the replacer do, and move to the first match at or
    /// after the cursor (live preview).
    fn refresh_pattern(&mut self, buf: &mut RopeBuffer) {
        self.armed_empty = false;
        if self.input.is_empty() {
            let _ = buf.set_search_pattern("");
            self.pattern = None;
            self.match_count = 0;
            self.status = SearchStatus::Empty;
            self.current = None;
            return;
        }
        let compiled = match find_replace::FindPattern::compile(self.input.value()) {
            Ok(p) => p,
            Err(e) => {
                let _ = buf.set_search_pattern("");
                self.pattern = None;
                self.match_count = 0;
                self.status = SearchStatus::Invalid(e.to_string());
                self.current = None;
                return;
            }
        };
        // Hand the buffer the *effective* pattern (smartcase already baked
        // in), not the raw query — otherwise its stepping and our highlighting
        // would disagree about case, which is exactly the split the bar's own
        // case model sets out to close.
        let _ = buf.set_search_pattern(compiled.as_regex().as_str());
        self.match_count = compiled.count_matches(buf.text().lines());
        self.pattern = Some(compiled);
        if self.match_count == 0 {
            self.status = SearchStatus::NoMatch;
            self.current = None;
            return;
        }
        let found = buf.search_forward(true);
        self.status = SearchStatus::from_found(found);
        self.highlight_current_match(buf, found);
    }

    /// Work out what an interactive replace would do, without touching the
    /// buffer: the match's row and char-column span, the expanded replacement,
    /// and the lines afterwards.
    ///
    /// Returns `None` when the cursor is not sitting exactly on a match.
    #[allow(clippy::type_complexity)]
    fn plan_replace_current(
        &self,
        buf: &RopeBuffer,
    ) -> Option<(usize, usize, usize, String, Vec<String>)> {
        let pattern = self.pattern.as_ref()?;
        let replacement = self.replacement();
        let (row, start_col) = buf.cursor();
        let line = buf.row(row)?;
        let start_byte = char_col_to_byte(&line, start_col);
        let caps = pattern.as_regex().captures_at(&line, start_byte)?;
        let m = caps.get(0)?;
        // `captures_at` finds the next match at OR AFTER the offset; only a
        // match starting exactly here is the current one.
        if m.start() != start_byte {
            return None;
        }
        let expanded = pattern.expand(&caps, replacement);
        let end_col = start_col + line[m.range()].chars().count();
        let mut after: Vec<String> = buf.rows();
        after[row].replace_range(m.range(), &expanded);
        Some((row, start_col, end_col, expanded, after))
    }

    /// Replace the **current match** and step to the next one — the `Enter`
    /// action while a **replace field** is revealed.
    fn replace_current(&mut self, buf: &mut RopeBuffer) {
        // Derive the span from the pattern at the cursor rather than reading
        // `self.current`, so the span is single-row by construction.
        let Some((row, start_col, end_col, expanded, after)) = self.plan_replace_current(buf)
        else {
            // Nothing usable under the cursor — step first, so the next Enter
            // has something to act on.
            self.advance(buf, false);
            return;
        };
        // Both ends have to be addressable before anything is edited. A match can
        // END inside a grapheme cluster — searching `e` over a decomposed `é`
        // does exactly that — and `Jump` refuses such a column by design
        // (a position the buffer cannot address is a no-op, never a
        // clamp). Discovering that half-way through the edit below leaves the
        // selection empty, so the replacement is *inserted* beside the match
        // instead of replacing it. Refuse rather than corrupt.
        let text = buf.text().clone();
        let addressable = |col: usize| {
            text.position(row, crate::ropetext::Column::new(col))
                .is_some()
        };
        if !addressable(start_col) || !addressable(end_col) {
            return;
        }
        // One `edit()` scope: select the match and overwrite it as a single
        // **undo group**, however many history entries that turns out to be.
        // Nothing here predicts the count, and nothing reads `insert_str`'s
        // bool — with an empty replacement it deletes and still returns false.
        buf.edit(|buf| {
            buf.move_cursor(CursorMove::Jump(row, start_col));
            buf.start_selection();
            buf.move_cursor(CursorMove::Jump(row, end_col));
            buf.insert_str(&expanded);
            buf.cancel_selection();
        });
        debug_assert_eq!(
            buf.text().to_string(),
            after.join("\n"),
            "replace_current wrote what it planned"
        );
        // The cursor sits just past the replacement, so the step below cannot
        // re-match inside text we just wrote.
        self.refresh_match_count(buf);
        self.advance(buf, false);
    }

    /// `Ctrl+A` inside the bar. Replace-all commits immediately — the match
    /// count is on screen beforehand and undo is one keystroke — except with
    /// an empty replacement, where the keystroke carries no evidence the user
    /// finished typing, so the first press arms and the second commits.
    fn replace_all_key(&mut self, buf: &mut RopeBuffer) {
        if !self.is_replacing() || self.pattern.is_none() {
            return;
        }
        if self.replacement().is_empty() && !self.armed_empty {
            self.armed_empty = true;
            return;
        }
        self.armed_empty = false;
        self.replace_all(buf);
    }

    /// Recount matches against the current buffer. Cheap — `find_iter` over
    /// lines the editor already holds.
    fn refresh_match_count(&mut self, buf: &RopeBuffer) {
        let Some(pattern) = self.pattern.as_ref() else {
            return;
        };
        self.match_count = pattern.count_matches(buf.text().lines());
    }

    /// After a search step, take the match at the buffer's cursor as the
    /// **current match**, which the front end paints in place of the selection.
    fn highlight_current_match(&mut self, buf: &RopeBuffer, found: bool) {
        self.current = if found { buf.match_at_cursor() } else { None };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::input::Mods;

    fn buf(text: &str) -> RopeBuffer {
        RopeBuffer::new(crate::ropetext::Text::from(text))
    }

    fn press(bar: &mut FindBar, buf: &mut RopeBuffer, key: Key, mods: Mods) -> KeyOutcome {
        bar.handle_key(KeyInput::new(key, mods), buf)
    }

    fn type_str(bar: &mut FindBar, buf: &mut RopeBuffer, s: &str) {
        for c in s.chars() {
            press(bar, buf, Key::Char(c), Mods::NONE);
        }
    }

    #[test]
    fn typing_compiles_the_pattern_and_lands_on_the_first_match() {
        let mut b = buf("foo\nbar baz bar");
        let mut bar = FindBar::new();
        type_str(&mut bar, &mut b, "bar");
        assert_eq!(bar.query(), "bar");
        assert_eq!(bar.status(), &SearchStatus::Match);
        assert_eq!(bar.match_count(), 2);
        assert_eq!(bar.current_match(), Some(((1, 0), (1, 3))));
        assert_eq!(b.cursor(), (1, 0));
    }

    #[test]
    fn enter_steps_and_shift_enter_steps_back() {
        let mut b = buf("x bar bar");
        let mut bar = FindBar::new();
        type_str(&mut bar, &mut b, "bar");
        assert_eq!(b.cursor(), (0, 2));
        press(&mut bar, &mut b, Key::Enter, Mods::NONE);
        assert_eq!(b.cursor(), (0, 6));
        press(&mut bar, &mut b, Key::Enter, Mods::SHIFT);
        assert_eq!(b.cursor(), (0, 2));
    }

    #[test]
    fn the_match_position_follows_the_current_match() {
        let mut b = buf("bar\nx bar bar");
        let mut bar = FindBar::new();
        assert_eq!(bar.match_position(&b), None, "empty query");
        type_str(&mut bar, &mut b, "bar");
        let at =
            |bar: &FindBar, b: &RopeBuffer| bar.match_position(b).map(|p| (p.current, p.total));
        assert_eq!(at(&bar, &b), Some((Some(1), 3)));
        press(&mut bar, &mut b, Key::Enter, Mods::NONE);
        assert_eq!(at(&bar, &b), Some((Some(2), 3)));
        press(&mut bar, &mut b, Key::Enter, Mods::NONE);
        assert_eq!(at(&bar, &b), Some((Some(3), 3)));
        type_str(&mut bar, &mut b, "q");
        assert_eq!(bar.match_position(&b), None, "no match");
        press(&mut bar, &mut b, Key::Backspace, Mods::NONE);
        type_str(&mut bar, &mut b, "(");
        assert_eq!(bar.match_position(&b), None, "invalid pattern");
    }

    #[test]
    fn an_invalid_regex_reports_and_clears_the_match() {
        let mut b = buf("a(b");
        let mut bar = FindBar::new();
        type_str(&mut bar, &mut b, "(");
        assert!(matches!(bar.status(), SearchStatus::Invalid(_)));
        assert_eq!(bar.current_match(), None);
        assert!(bar.pattern().is_none());
    }

    #[test]
    fn tab_reveals_replace_then_cycles_focus() {
        let mut b = buf("foo");
        let mut bar = FindBar::new();
        type_str(&mut bar, &mut b, "foo");
        press(&mut bar, &mut b, Key::Tab, Mods::NONE);
        assert!(bar.is_replacing());
        assert_eq!(bar.focus(), BarFocus::Replace);
        press(&mut bar, &mut b, Key::Tab, Mods::NONE);
        assert_eq!(bar.focus(), BarFocus::Find);
    }

    #[test]
    fn enter_while_replacing_replaces_the_current_match() {
        let mut b = buf("a foo foo");
        let mut bar = FindBar::new();
        type_str(&mut bar, &mut b, "foo");
        press(&mut bar, &mut b, Key::Tab, Mods::NONE);
        type_str(&mut bar, &mut b, "x");
        assert_eq!(bar.replacement(), "x");
        press(&mut bar, &mut b, Key::Enter, Mods::NONE);
        assert_eq!(b.text().to_string(), "a x foo");
        assert_eq!(bar.match_count(), 1);
    }

    #[test]
    fn ctrl_a_with_an_empty_replacement_arms_then_commits_and_esc_disarms() {
        let mut b = buf("foo foo");
        let mut bar = FindBar::new();
        type_str(&mut bar, &mut b, "foo");
        press(&mut bar, &mut b, Key::Tab, Mods::NONE);
        press(&mut bar, &mut b, Key::Char('a'), Mods::CTRL);
        assert!(bar.is_armed());
        assert_eq!(b.text().to_string(), "foo foo");
        // Esc disarms rather than closing.
        let out = press(&mut bar, &mut b, Key::Esc, Mods::NONE);
        assert!(!out.close);
        assert!(!bar.is_armed());
        press(&mut bar, &mut b, Key::Char('a'), Mods::CTRL);
        press(&mut bar, &mut b, Key::Char('a'), Mods::CTRL);
        assert_eq!(b.text().to_string(), " ");
        // Ctrl+Z takes the whole replace all back, Ctrl+Shift+Z redoes it.
        press(&mut bar, &mut b, Key::Char('z'), Mods::CTRL);
        assert_eq!(b.text().to_string(), "foo foo");
        assert_eq!(bar.match_count(), 2);
        press(
            &mut bar,
            &mut b,
            Key::Char('Z'),
            Mods {
                ctrl: true,
                shift: true,
                ..Mods::NONE
            },
        );
        assert_eq!(b.text().to_string(), " ");
    }

    #[test]
    fn status_text_counts_notes_the_case_and_flags_problems() {
        let mut b = buf("Foo foo\nbar");
        let mut bar = FindBar::new();
        assert_eq!(bar.status_text(), None);
        type_str(&mut bar, &mut b, "foo");
        assert_eq!(
            bar.status_text(),
            Some(("2 matches  any case".into(), StatusTone::Muted))
        );
        press(&mut bar, &mut b, Key::Backspace, Mods::NONE);
        press(&mut bar, &mut b, Key::Backspace, Mods::NONE);
        press(&mut bar, &mut b, Key::Backspace, Mods::NONE);
        type_str(&mut bar, &mut b, "Foo");
        assert_eq!(
            bar.status_text(),
            Some(("1 match  exact case".into(), StatusTone::Muted))
        );
        type_str(&mut bar, &mut b, "x");
        assert_eq!(
            bar.status_text(),
            Some(("no match  exact case".into(), StatusTone::Error))
        );
        type_str(&mut bar, &mut b, "(");
        let (text, tone) = bar.status_text().expect("invalid");
        assert!(text.starts_with("invalid regex: "), "{text}");
        assert_eq!(tone, StatusTone::Error);
    }

    #[test]
    fn the_replace_hint_turns_into_the_confirm_while_armed() {
        let mut b = buf("foo");
        let mut bar = FindBar::new();
        type_str(&mut bar, &mut b, "foo");
        press(&mut bar, &mut b, Key::Tab, Mods::NONE);
        assert_eq!(bar.replace_hint(), (REPLACE_HINTS, StatusTone::Muted));
        press(&mut bar, &mut b, Key::Char('a'), Mods::CTRL);
        assert_eq!(bar.replace_hint(), (REPLACE_HINTS_ARMED, StatusTone::Error));
    }

    #[test]
    fn esc_closes_an_unarmed_bar() {
        let mut b = buf("foo");
        let mut bar = FindBar::new();
        assert!(press(&mut bar, &mut b, Key::Esc, Mods::NONE).close);
    }

    #[test]
    fn paste_takes_the_first_line_into_the_focused_field() {
        let mut b = buf("first second");
        let mut bar = FindBar::new();
        bar.paste("second\nthird", &mut b);
        assert_eq!(bar.query(), "second");
        assert_eq!(bar.current_match(), Some(((0, 6), (0, 12))));
    }

    #[test]
    fn the_overlay_is_matches_while_finding_and_a_preview_while_replacing() {
        let mut b = buf("foo bar foo");
        let mut bar = FindBar::new();
        type_str(&mut bar, &mut b, "foo");
        let (preview, previews, matches) = bar.overlay(&b).into_frame_parts();
        assert!(preview.is_none() && previews.is_empty());
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().all(|o| o.kind == OverlayKind::Match));
        press(&mut bar, &mut b, Key::Tab, Mods::NONE);
        type_str(&mut bar, &mut b, "q");
        let (preview, previews, matches) = bar.overlay(&b).into_frame_parts();
        assert_eq!(preview, Some(vec!["q bar q".to_string()]));
        assert_eq!(previews.len(), 2);
        assert!(matches.is_empty());
    }
}
