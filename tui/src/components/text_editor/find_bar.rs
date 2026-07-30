//! The **find bar**: searching and replacing inside the open buffer
//! (adr/0033–0036).
//!
//! A module rather than a cluster of methods on the editor. The bar reaches
//! outside itself for exactly one thing — the **edit buffer** — so it takes one
//! as a parameter and the editor is left owning policy (which backend may open
//! a bar), layout, and wiring the bar's overlay into the view.
//!
//! The bar owns its **current match** rather than writing the editor's
//! selection. A current match is not a selection: it cannot be extended,
//! copied, or typed over, and rendering it as one is why a mouse drag could
//! hand the bar a multi-row range it had no way to represent.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::char_col_to_byte;
use super::find_replace;
use super::rope_buffer::{CursorMove, RopeBuffer};
use crate::components::single_line_input::{InputOutcome, SingleLineInput};
use crate::settings::themes::Theme;

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

impl Default for FindBar {
    fn default() -> Self {
        Self::new()
    }
}

/// Which of the find bar's inputs owns the keyboard. Only meaningful once a
/// **replace field** has been revealed — a find-only bar is always `Find`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BarFocus {
    Find,
    Replace,
}

pub struct FindBar {
    // `pub(super)` on the state fields is a concession, not a design: the
    // editor's own tests still assert against them. Migrating those tests into
    // this module — where the bar can be driven directly against an
    // `RopeBuffer` — is the follow-up that lets these go private again.
    pub(super) input: SingleLineInput,
    /// `Some` once the **replace field** is revealed. Its presence is what
    /// puts the bar in replace mode (adr/0033).
    pub(super) replace: Option<SingleLineInput>,
    pub(super) focus: BarFocus,
    pub(super) status: SearchStatus,
    /// The compiled **find pattern**. `None` while the query is empty or does
    /// not compile — the one place the regex lives, so the highlighter, the
    /// counter and the replacer can never disagree about what matches.
    pub(super) pattern: Option<find_replace::FindPattern>,
    /// Matches across the whole buffer, refreshed with the pattern. Shown
    /// before a **replace all** so the keystroke is an informed one.
    pub(super) match_count: usize,
    /// The **current match** — the occurrence the cursor sits on, which the bar
    /// owns rather than writing into the editor's selection.
    pub(super) current: Option<((usize, usize), (usize, usize))>,
    /// Set when a **replace all** with an empty replacement was requested but
    /// not yet confirmed. An empty field is indistinguishable from "still
    /// typing", so that one destructive case arms rather than commits.
    pub(super) armed_empty: bool,
}

impl FindBar {
    pub fn new() -> Self {
        Self {
            input: SingleLineInput::new(),
            replace: None,
            focus: BarFocus::Find,
            status: SearchStatus::Empty,
            pattern: None,
            match_count: 0,
            current: None,
            armed_empty: false,
        }
    }

    /// Reveal the **replace field**, putting the bar in replace mode. Focus
    /// lands in the find field while the pattern is still empty — you cannot
    /// usefully type a replacement for nothing.
    pub fn reveal_replace(&mut self) {
        if self.replace.is_none() {
            self.replace = Some(SingleLineInput::new());
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
        let input = self.focused_input_mut();
        let at = input.cursor_byte();
        input.replace_range_bytes(at..at, &line, at + line.len());
        if focus == BarFocus::Find {
            self.refresh_pattern(buf);
        }
    }

    /// Re-derive what an undo or redo invalidated. The **current match**
    /// pointed at text the history step just changed, so recompute it against
    /// the cursor rather than leaving a highlight over whatever now sits there.
    fn after_history_step(&mut self, buf: &RopeBuffer) {
        self.refresh_match_count(buf);
        self.current = buf.match_at_cursor();
    }

    /// Everything the bar wants painted this frame.
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

    /// The **current match**, for the editor to hand the view as its selection.
    pub fn current_match(&self) -> Option<((usize, usize), (usize, usize))> {
        self.current
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
    pub(super) fn replacement(&self) -> &str {
        self.replace.as_ref().map(|r| r.value()).unwrap_or("")
    }

    /// The input the keyboard is currently driving.
    fn focused_input_mut(&mut self) -> &mut SingleLineInput {
        match self.focus {
            BarFocus::Replace if self.replace.is_some() => {
                self.replace.as_mut().expect("checked above")
            }
            _ => &mut self.input,
        }
    }
}

pub(super) enum SearchStatus {
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
const REPLACE_PROMPT: &str = "Replace: ";
const FIND_HINTS: &str = "  [Enter] next  [Shift+Enter] prev  [Tab] replace  [Esc] close";
const REPLACE_HINTS: &str =
    "  [Enter] replace  [Shift+Enter] skip  [Ctrl+A] all  [Tab] field  [Esc] close";
const REPLACE_HINTS_ARMED: &str = "  delete every match? [Ctrl+A] confirm  [Esc] cancel";

/// Render one prompt-plus-input row, returning the columns the prompt and
/// value consumed so the caller can place a tail after them.
fn render_bar_row(
    f: &mut Frame,
    rect: Rect,
    prompt: &str,
    input: &mut SingleLineInput,
    theme: &Theme,
    focused: bool,
) -> u16 {
    let base = theme.base_style();
    let prompt_cols = unicode_width::UnicodeWidthStr::width(prompt) as u16;
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            prompt,
            base.add_modifier(Modifier::BOLD),
        )))
        .style(base),
        Rect {
            width: prompt_cols.min(rect.width),
            ..rect
        },
    );
    input.render(f, rect, base, prompt_cols, focused);
    // Tail sits after the full value (in display columns, accounting for
    // wide/CJK chars), not after the caret — otherwise it would overlap the
    // trailing characters when the user moves the cursor mid-string.
    prompt_cols.saturating_add(input.display_width() as u16)
}

fn render_tail(f: &mut Frame, rect: Rect, consumed: u16, text: &str, style: Style) {
    let tail_rect = Rect {
        x: rect.x.saturating_add(consumed),
        width: rect.width.saturating_sub(consumed),
        ..rect
    };
    f.render_widget(Paragraph::new(text.to_string()).style(style), tail_rect);
}

/// Draw the find bar. `rect` is one row while finding, two once a **replace
/// field** is revealed: row one is the pattern and what it matches, row two is
/// the replacement and what will happen to it.
impl FindBar {
    /// Rows this bar occupies: one while finding, two once a **replace field**
    /// is revealed.
    pub fn rows(&self) -> u16 {
        if self.is_replacing() { 2 } else { 1 }
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        let muted = Style::default()
            .fg(theme.gray.to_ratatui())
            .bg(theme.bg.to_ratatui());
        let err = Style::default()
            .fg(theme.red.to_ratatui())
            .bg(theme.bg.to_ratatui());

        let replacing = self.is_replacing();
        let find_focused = focused && self.focus == BarFocus::Find;
        let find_row = Rect { height: 1, ..rect };

        // Row 1 tail: what the pattern matches. Smartcase is never silent — the
        // indicator says which way it resolved.
        let case_note = match &self.pattern {
            Some(p) if p.case_sensitive() => "  exact case",
            Some(_) => "  any case",
            None => "",
        };
        let tail: Option<(String, Style)> = match &self.status {
            SearchStatus::Empty => None,
            SearchStatus::Invalid(msg) => Some((format!("  invalid regex: {msg}"), err)),
            SearchStatus::NoMatch => Some((format!("  no match{case_note}"), err)),
            SearchStatus::Match => {
                let n = self.match_count;
                let plural = if n == 1 { "match" } else { "matches" };
                let hints = if replacing { "" } else { FIND_HINTS };
                Some((format!("  {n} {plural}{case_note}{hints}"), muted))
            }
        };

        let consumed = render_bar_row(
            f,
            find_row,
            FIND_PROMPT,
            &mut self.input,
            theme,
            find_focused,
        );
        if let Some((text, style)) = tail {
            render_tail(f, find_row, consumed, &text, style);
        }

        if !replacing || rect.height < 2 {
            return;
        }
        let replace_row = Rect {
            y: rect.y + 1,
            height: 1,
            ..rect
        };
        let armed = self.armed_empty;
        let replace_focused = focused && self.focus == BarFocus::Replace;
        let consumed = {
            let input = self.replace.as_mut().expect("replacing");
            render_bar_row(
                f,
                replace_row,
                REPLACE_PROMPT,
                input,
                theme,
                replace_focused,
            )
        };
        let (hints, style) = if armed {
            (REPLACE_HINTS_ARMED, err)
        } else {
            (REPLACE_HINTS, muted)
        };
        render_tail(f, replace_row, consumed, hints, style);
    }
}
impl FindBar {
    /// Recompile the **find pattern** under smartcase, refresh the match count,
    /// and push it to the textarea so its stepping uses the same regex the
    /// highlighter and the replacer do. When `jump` is true, also move to the
    /// first match at or after the cursor (live preview).
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
        // Hand the textarea the *effective* pattern (smartcase already baked
        // in), not the raw query — otherwise its stepping and our highlighting
        // would disagree about case, which is exactly the split adr/0033 set
        // out to close.
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
    /// Work out what an interactive replace would do, without touching the
    /// buffer: the match's row and char-column span, the expanded replacement,
    /// and the lines before and after.
    ///
    /// Returns `None` when the cursor is not sitting exactly on a match.
    #[allow(clippy::type_complexity)]
    fn plan_replace_current(
        &self,
        buf: &RopeBuffer,
    ) -> Option<(usize, usize, usize, String, Vec<String>, Vec<String>)> {
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
        let before: Vec<String> = buf.lines().to_vec();
        let mut after = before.clone();
        after[row].replace_range(m.range(), &expanded);
        Some((row, start_col, end_col, expanded, before, after))
    }
    /// Replace the **current match** and step to the next one — the `Enter`
    /// action while a **replace field** is revealed.
    fn replace_current(&mut self, buf: &mut RopeBuffer) {
        // Derive the span from the pattern at the cursor rather than reading
        // `self.current`. That field is shared with the visual-mode and mouse
        // selection, and `handle_mouse` has no find-bar guard, so a drag can
        // leave a MULTI-ROW range in it while the bar is open — which this used
        // to collapse to one row by discarding the end row, then hand
        // `replace_range` an inverted byte range. A span derived from the match
        // is single-row by construction.
        let Some((row, start_col, end_col, expanded, before, after)) =
            self.plan_replace_current(buf)
        else {
            // Nothing usable under the cursor — step first, so the next Enter
            // has something to act on.
            self.advance(buf, false);
            return;
        };
        // `CursorMove::Jump` takes u16 and clamps silently, so a position past
        // 65535 would select the wrong range and splice text into the middle of
        // a line. Refuse rather than corrupt.
        let Some((row_u16, start_u16, end_u16)) = Some((row, start_col, end_col)) else {
            return;
        };
        // One `edit()` scope: select the match and overwrite it as a single
        // **undo group**, however many history entries that turns out to be.
        // Nothing here predicts the count, and nothing reads `insert_str`'s
        // bool — with an empty replacement it deletes and still returns false.
        buf.edit(|buf| {
            buf.move_cursor(CursorMove::Jump(row_u16, start_u16));
            buf.start_selection();
            buf.move_cursor(CursorMove::Jump(row_u16, end_u16));
            buf.insert_str(&expanded);
            buf.cancel_selection();
        });
        debug_assert_eq!(
            buf.lines(),
            after.as_slice(),
            "replace_current wrote what it planned"
        );
        let _ = before;
        // Land the cursor just past the replacement so the step below cannot
        // re-match inside text we just wrote.
        self.refresh_match_count(buf);
        self.advance(buf, false);
    }
    /// Rewrite every match in the buffer — the `Ctrl+A` action. Returns the
    /// number replaced, or `None` when there was nothing to do.
    pub fn replace_all(&mut self, buf: &mut RopeBuffer) -> Option<usize> {
        let pattern = self.pattern.as_ref()?;
        let replacement = self.replacement().to_string();

        let before: Vec<String> = buf.lines().to_vec();
        let (after, count) = find_replace::replace_all(pattern, &before, &replacement)?;

        // Restore the reading position afterwards. The naive path leaves the
        // cursor at the end of the inserted chunk — i.e. the bottom of the
        // note — which turns a bulk edit into a navigation. The row is always
        // still valid: the pattern cannot span a newline and the replacement
        // is single-line, so a replace all never changes the line count.
        let (cur_row, cur_col) = buf.cursor();

        let joined = after.join("\n");
        // One **undo group** spanning the whole rewrite. The buffer also
        // derives `bulk` from it — a replace all rewrites rows the cursor does
        // not point at, which is exactly what `compute_damage_range`'s cursor
        // fast path assumes cannot happen (adr/0035).
        buf.edit(|buf| {
            buf.select_all();
            buf.insert_str(&joined);
            buf.cancel_selection();
        });
        if buf.lines() != after.as_slice() {
            return None;
        }
        let _ = &before;

        // Restore the reading position. The row stays valid because a replace all
        // never changes the line count: the pattern cannot span a newline and the
        // replace field is single-line.
        let row = cur_row.min(after.len().saturating_sub(1));
        let col = cur_col.min(after[row].chars().count());
        buf.move_cursor(CursorMove::Jump(row, col));

        self.current = None;
        self.refresh_match_count(buf);
        Some(count)
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
    /// Build the **replace preview** for this frame: the note as it would read
    /// with every match replaced, plus where each replacement landed.
    ///
    /// Returns `None` whenever there is nothing to preview, in which case the
    /// caller renders the real buffer.
    pub(super) fn preview(&self, buf: &RopeBuffer) -> Option<find_replace::Preview> {
        if !self.is_replacing() {
            return None;
        }
        let pattern = self.pattern.as_ref()?;
        let current = self.current.map(|((row, col), _)| (row, col));
        let preview =
            find_replace::build_preview(pattern, buf.lines(), self.replacement(), current);
        if preview.spans.is_empty() {
            return None;
        }
        Some(preview)
    }
    /// After a search step, paint the match at the textarea's cursor as the
    /// editor selection so the user can see where the match is — our custom
    /// `MarkdownEditorView` does not render the textarea library's built-in
    /// search highlights.
    fn highlight_current_match(&mut self, buf: &RopeBuffer, found: bool) {
        self.current = if found { buf.match_at_cursor() } else { None };
    }
    /// Handle one key. The bar owns every key while it holds the **editor
    /// claim**, so this always consumes; the outcome says only whether the
    /// editor should drop the bar.
    ///
    /// The key map is the same on both backends — the vim emulation's old
    /// "Enter confirms and closes" special case is gone, because two Enters
    /// over one widget is what made the bar ambiguous (adr/0033).
    pub fn handle_key(&mut self, key: &KeyEvent, buf: &mut RopeBuffer) -> KeyOutcome {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let stay = KeyOutcome::default();

        // `Tab` reveals the replace field, then cycles focus between the two.
        // `SingleLineInput` never consumes it, so it is ours to take.
        if key.code == KeyCode::Tab {
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

        // Replace all. `SingleLineInput` deliberately bubbles Ctrl-modified
        // chars rather than typing them, so this is the documented seam. The
        // chord is shared with the editor's select-all and resolved by focus,
        // as adr/0032 does for Ctrl+Y.
        if ctrl && matches!(key.code, KeyCode::Char('a') | KeyCode::Char('A')) {
            self.replace_all_key(buf);
            return stay;
        }

        // Undo / redo of the bar's OWN edits. The bar consumes every key, so
        // without this it swallows Ctrl+Z and strands the user on a note it
        // just rewrote — undo would only work after they thought to press Esc.
        if ctrl {
            match key.code {
                KeyCode::Char('z') if !shift => {
                    if buf.undo() {
                        self.after_history_step(buf);
                    }
                    return stay;
                }
                KeyCode::Char('y') | KeyCode::Char('Z') => {
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
            InputOutcome::Cancel => {
                // Esc disarms first, so a mis-aimed Ctrl+A never costs the bar.
                if self.armed_empty {
                    self.armed_empty = false;
                } else {
                    // The editor drops the bar and clears its selection; the
                    // pattern stays on the buffer so vim's `n`/`N` still work.
                    return KeyOutcome { close: true };
                }
            }
            InputOutcome::Submit => {
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
            InputOutcome::Changed => {
                // Editing either field disarms a pending confirm and
                // invalidates the preview, which rebuilds from state anyway.
                if self.focus == BarFocus::Find {
                    self.refresh_pattern(buf);
                } else {
                    self.armed_empty = false;
                }
            }
            InputOutcome::Consumed | InputOutcome::NotConsumed => {}
        }
        stay
    }
}
