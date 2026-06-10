//! Built-in vim emulation: a modal input interpreter over a `TextArea`.
//! Pure over `&mut TextArea` — no component state, no async (adr/0012).

use super::snapshot::EditorMode;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::{CursorMove, TextArea};

/// Screen-level actions the host performs on the engine's behalf (adr/0012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimHostAction {
    OpenPalette,                  // `:`
    OpenSearch { forward: bool }, // `/` (true) `?` (false)
    SearchNext,                   // `n`
    SearchPrev,                   // `N`
}

/// What a key did, so the host can bump the right revision counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimKeyOutcome {
    /// Buffer text changed — host calls `bump_content()`.
    TextMutated,
    /// Only the cursor/selection moved — host refreshes view, not content.
    CursorOnly,
    /// Nothing happened (unmapped key in Normal mode).
    NoOp,
    /// Insert mode: defer to the existing `handle_textarea_key` path.
    PassThrough,
    /// The host must perform a screen-level action.
    Host(VimHostAction),
}

// ── Plan 2 Task 1: reified command model ────────────────────────────────────

/// A cursor motion. Operators consume a motion to form a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordForward,
    WordBack,
    WordEnd,
    LineStart,
    FirstNonBlank,
    LineEnd,
    FileStart,
    FileEnd,
    ParagraphForward,
    ParagraphBack,
    MatchingPair,                              // % — Task 9
    FindChar { ch: char, till: bool, forward: bool }, // f/F/t/T — Task 7
}

/// An operator awaiting a motion or text object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
}

/// A text object (`iw`, `a"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObject {
    Word { around: bool },
    Pair { open: char, close: char, around: bool },
    Quote { ch: char, around: bool },
}

/// The fully-parsed unit of work, ready to apply (adr/0011).
// Some variants are recorded for dot-repeat/future macros but not yet read in replay (adr/0011).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Command {
    OperateMotion(Operator, Motion, usize), // e.g. 2dw
    OperateLine(Operator, usize),           // dd / cc / yy / >> with count
    OperateObject(Operator, TextObject),    // diw, ci"
    OperateToLineEnd(Operator),             // D / C / Y
    DeleteChar { forward: bool, count: usize }, // x / X
    ReplaceChar(char),                      // r<ch>
    SubstituteChar(usize),                  // s
    SubstituteLine,                         // S
    JoinLines(usize),                       // J
    ToggleCase(usize),                      // ~
    Paste { after: bool, count: usize },    // p / P
}

// ── Plan 2 Task 1: pending-state helper types ────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct PendingFind {
    operator: Option<Operator>,
    till: bool,
    forward: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterKind {
    Charwise,
    Linewise,
}

#[derive(Debug, Clone)]
struct Change {
    command: Command,
    inserted: Option<String>,
}

#[derive(Debug, Clone)]
struct InsertCapture {
    command: Command,
    start: (usize, usize),
}

// ── VimEngine ────────────────────────────────────────────────────────────────

/// Modal vim state layered over the textarea buffer.
#[derive(Debug)]
pub struct VimEngine {
    mode: EditorMode,
    // Plan 2 Task 1: pending-state + dot-repeat fields
    pending_count: Option<usize>,
    pending_operator: Option<Operator>,
    pending_g: bool,              // first key of `gg`
    pending_find: Option<PendingFind>,
    pending_replace: bool,        // awaiting the char after `r`
    pending_object_kind: Option<bool>, // Some(around): saw `i`/`a` after operator
    last_find: Option<(char, bool, bool)>, // (ch, till, forward) for ; and ,
    register: RegisterKind,
    /// The last mutating command + captured insert delta, for `.` (adr/0011).
    last_change: Option<Change>,
    /// While in Insert via a vim command, the text typed is accumulated here
    /// (resulting delta) so `.` can replay it.
    insert_capture: Option<InsertCapture>,
    /// Set around `replay` calls so `apply_operator_on_selection`'s Change arm
    /// doesn't re-enter Insert mode during dot-repeat (the replay branch
    /// inserts the text directly instead).
    replaying: bool,
}

impl Default for VimEngine {
    fn default() -> Self {
        Self {
            mode: EditorMode::Normal,
            pending_count: None,
            pending_operator: None,
            pending_g: false,
            pending_find: None,
            pending_replace: false,
            pending_object_kind: None,
            last_find: None,
            register: RegisterKind::Charwise,
            last_change: None,
            insert_capture: None,
            replaying: false,
        }
    }
}

impl VimEngine {
    pub fn mode(&self) -> &EditorMode {
        &self.mode
    }

    /// Footer label for the current mode (e.g. "NORMAL").
    pub fn mode_label(&self) -> String {
        self.mode.label().to_string()
    }

    /// The in-progress command sequence, for the footer hint (e.g. "2d", "f").
    /// Returns `None` when nothing is pending (no display needed).
    pub fn pending_hint(&self) -> Option<String> {
        // Fast path: nothing pending — skip all allocation (common idle-frame case).
        if self.pending_count.is_none()
            && self.pending_operator.is_none()
            && !self.pending_g
            && !self.pending_replace
            && self.pending_find.is_none()
        {
            return None;
        }
        let mut s = String::new();
        if let Some(n) = self.pending_count {
            s.push_str(&n.to_string());
        }
        if let Some(op) = self.pending_operator {
            s.push(match op {
                Operator::Delete => 'd',
                Operator::Change => 'c',
                Operator::Yank => 'y',
                Operator::Indent => '>',
                Operator::Outdent => '<',
            });
        }
        if self.pending_g {
            s.push('g');
        }
        if self.pending_replace {
            s.push('r');
        }
        if self.pending_find.is_some() {
            s.push('f');
        }
        if s.is_empty() { None } else { Some(s) }
    }

    pub fn reset_to_normal(&mut self) {
        self.mode = EditorMode::Normal;
        self.clear_pending();
    }

    /// Reconcile mode after a host-driven selection change (mouse). A live
    /// selection means Visual; losing the selection in Visual returns to Normal.
    pub fn sync_mouse_selection(&mut self, has_selection: bool) {
        match (has_selection, &self.mode) {
            (true, EditorMode::Normal) => self.mode = EditorMode::Visual,
            (false, EditorMode::Visual) | (false, EditorMode::VisualLine) => {
                self.mode = EditorMode::Normal
            }
            _ => {}
        }
    }

    /// True when a bare Space should start the leader: Normal mode, nothing
    /// pending (so `d<Space>`, `f<Space>`, counts etc. still take Space as an
    /// argument/motion, not the leader).
    pub fn space_leads(&self) -> bool {
        self.mode == EditorMode::Normal
            && self.pending_count.is_none()
            && self.pending_operator.is_none()
            && !self.pending_g
            && !self.pending_replace
            && self.pending_find.is_none()
            && self.pending_object_kind.is_none()
    }

    /// Interpret one key. In Insert mode everything except `Esc` is
    /// `PassThrough` (the host runs the existing direct textarea path).
    /// In Visual/VisualLine mode, motions extend the selection; operators
    /// act on the live selection. In Normal mode, motions move the cursor
    /// and the insert-entry keys switch to Insert mode.
    pub fn handle_key(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        match self.mode {
            EditorMode::Insert => self.handle_insert(key, ta),
            EditorMode::Visual | EditorMode::VisualLine => self.handle_visual(key, ta),
            _ => self.handle_normal(key, ta),
        }
    }

    // ── Plan 2 Task 10: Visual + Visual-line mode handler ────────────────────

    fn handle_visual(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        // Esc: cancel selection and return to Normal.
        if key.code == KeyCode::Esc {
            ta.cancel_selection();
            self.mode = EditorMode::Normal;
            return VimKeyOutcome::CursorOnly;
        }

        // Arrow keys: extend the selection.
        let plain =
            key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT;
        let KeyCode::Char(c) = key.code else {
            match key.code {
                KeyCode::Left => {
                    ta.move_cursor(CursorMove::Back);
                    return VimKeyOutcome::CursorOnly;
                }
                KeyCode::Right => {
                    ta.move_cursor(CursorMove::Forward);
                    return VimKeyOutcome::CursorOnly;
                }
                KeyCode::Up => {
                    ta.move_cursor(CursorMove::Up);
                    return VimKeyOutcome::CursorOnly;
                }
                KeyCode::Down => {
                    ta.move_cursor(CursorMove::Down);
                    return VimKeyOutcome::CursorOnly;
                }
                _ => return VimKeyOutcome::NoOp,
            }
        };
        if !plain {
            return VimKeyOutcome::NoOp;
        }

        // Count accumulation.
        if self.accumulate_count(c) {
            return VimKeyOutcome::NoOp;
        }

        // Operators act on the EXISTING live selection (already started by v/V).
        // In VisualLine mode: use linewise deletion (preserves newlines correctly).
        // In Visual mode: use charwise cut on the current selection.
        let op = match c {
            'd' | 'x' => Some(Operator::Delete),
            'c' | 's' => Some(Operator::Change),
            'y' => Some(Operator::Yank),
            _ => None,
        };
        if let Some(op) = op {
            let is_line = self.mode == EditorMode::VisualLine;
            if is_line {
                // VisualLine: operate on whole selected lines, preserving newlines.
                // Determine the line range from the current selection.
                let (start_row, end_row) = if let Some(((sr, _), (er, _))) = ta.selection_range() {
                    (sr, er)
                } else {
                    let (r, _) = super::cursor_tuple(ta);
                    (r, r)
                };
                // Cancel the current selection so apply_operator_linewise can
                // re-anchor from the correct start row.
                ta.cancel_selection();
                ta.move_cursor(CursorMove::Jump(start_row as u16, 0));
                let count = end_row - start_row + 1;
                self.apply_operator_linewise(op, count, ta);
            } else {
                // Charwise Visual: vim selection is inclusive of the char under
                // the cursor. Ratatui uses half-open [anchor, cursor), so extend
                // the high end by one char (clamped to end-of-line) before applying.
                if let Some(((sr, sc), (er, ec))) = ta.selection_range() {
                    let end_line_len = ta.lines().get(er).map(|l| l.chars().count()).unwrap_or(ec);
                    let ec_incl = (ec + 1).min(end_line_len);
                    ta.cancel_selection();
                    ta.move_cursor(CursorMove::Jump(sr as u16, sc as u16));
                    ta.start_selection();
                    ta.move_cursor(CursorMove::Jump(er as u16, ec_incl as u16));
                }
                self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
            }
            self.mode = if op == Operator::Change {
                EditorMode::Insert
            } else {
                EditorMode::Normal
            };
            self.clear_pending();
            return self.outcome_for(op);
        }

        // 'p'/'P': replace the current visual selection with the register.
        // CRITICAL: capture the register text+kind BEFORE cut() overwrites yank_text.
        if c == 'p' || c == 'P' {
            let text = ta.yank_text();
            let kind = self.register;
            if text.is_empty() {
                ta.cancel_selection();
                self.mode = EditorMode::Normal;
                return VimKeyOutcome::CursorOnly;
            }
            if self.mode == EditorMode::VisualLine {
                // VisualLine: delete the selected whole lines, then paste the register.
                let (start_row, end_row) = if let Some(((sr, _), (er, _))) = ta.selection_range() {
                    (sr, er)
                } else {
                    let (r, _) = super::cursor_tuple(ta);
                    (r, r)
                };
                ta.cancel_selection();
                ta.move_cursor(CursorMove::Jump(start_row as u16, 0));
                let count = end_row - start_row + 1;
                // Delete the lines (reuse linewise delete logic).
                self.apply_operator_linewise(Operator::Delete, count, ta);
                // Restore the original register (the delete just clobbered it).
                ta.set_yank_text(&text);
                self.register = kind;
                // Paste the register at the current position (before the line that
                // landed at start_row after the delete).
                self.paste(false, 1, ta);
            } else {
                // Charwise: make an inclusive selection, delete it, then insert.
                if let Some(((sr, sc), (er, ec))) = ta.selection_range() {
                    let len = ta.lines().get(er).map(|l| l.chars().count()).unwrap_or(ec);
                    let ec_incl = (ec + 1).min(len);
                    ta.cancel_selection();
                    ta.move_cursor(CursorMove::Jump(sr as u16, sc as u16));
                    ta.start_selection();
                    ta.move_cursor(CursorMove::Jump(er as u16, ec_incl as u16));
                }
                ta.cut(); // cursor lands at the deletion gap
                // Restore the original register (cut clobbered it) and insert.
                ta.set_yank_text(&text);
                self.register = kind;
                if kind == RegisterKind::Charwise {
                    // Record where the paste starts so we can leave the cursor there
                    // (vim visual-p leaves cursor at the start of the pasted text).
                    let paste_start = super::cursor_tuple(ta);
                    ta.insert_str(&text);
                    ta.move_cursor(CursorMove::Jump(paste_start.0 as u16, paste_start.1 as u16));
                } else {
                    // Linewise register over a charwise selection.
                    self.paste(false, 1, ta);
                }
            }
            self.mode = EditorMode::Normal;
            self.clear_pending();
            return VimKeyOutcome::TextMutated;
        }

        // 'o': swap selection end — documented v1 no-op (swap-end not implemented).
        if c == 'o' {
            return VimKeyOutcome::NoOp;
        }

        // Task 12: visual `>`/`<` — indent/outdent the selected line range.
        if c == '>' || c == '<' {
            let outdent = c == '<';
            let line_count = if let Some(((sr, _), (er, _))) = ta.selection_range() {
                er.saturating_sub(sr) + 1
            } else {
                1
            };
            // Cancel selection; jump to first selected row; then indent.
            let start_row = if let Some(((sr, _), _)) = ta.selection_range() {
                sr
            } else {
                super::cursor_tuple(ta).0
            };
            ta.cancel_selection();
            ta.move_cursor(CursorMove::Jump(start_row as u16, 0));
            self.indent_lines(outdent, line_count, ta);
            self.mode = EditorMode::Normal;
            self.clear_pending();
            return VimKeyOutcome::TextMutated;
        }

        // Pair chars: set Normal and return PassThrough so the host's existing
        // auto-surround path wraps the selection (Q11 decision; verified in Plan 3).
        if matches!(c, '(' | '[' | '{' | '<' | '"' | '\'' | '`' | '*' | '_' | '~') {
            self.mode = EditorMode::Normal;
            return VimKeyOutcome::PassThrough;
        }

        // Motions extend the selection.
        let count = self.pending_count.unwrap_or(1);
        if let Some(m) = Self::motion_for_char(c) {
            self.apply_motion(m, count, ta);
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }

        VimKeyOutcome::NoOp
    }

    fn handle_insert(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        if key.code == KeyCode::Esc {
            self.mode = EditorMode::Normal;
            // Compute the inserted text once at Esc, slicing from the start cursor
            // recorded when Insert began to the current cursor (multi-line aware).
            if let Some(cap) = self.insert_capture.take() {
                let end = super::cursor_tuple(ta);
                let inserted = Self::text_between(ta.lines(), cap.start, end);
                self.last_change = Some(Change {
                    command: cap.command,
                    inserted: Some(inserted),
                });
            }
            if super::cursor_tuple(ta).1 > 0 {
                ta.move_cursor(CursorMove::Back);
            }
            return VimKeyOutcome::CursorOnly;
        }
        VimKeyOutcome::PassThrough
    }

    fn handle_normal(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        // Task 6: consume the replacement char when pending_replace is set
        if self.pending_replace {
            self.pending_replace = false;
            if let KeyCode::Char(c) = key.code {
                return self.replace_char(c, ta);
            }
            return VimKeyOutcome::NoOp; // Esc etc cancels
        }

        // Task 7: consume the find target char when pending_find is set
        if let Some(pf) = self.pending_find.take() {
            if let KeyCode::Char(ch) = key.code {
                self.last_find = Some((ch, pf.till, pf.forward));
                let motion = Motion::FindChar { ch, till: pf.till, forward: pf.forward };
                let cnt = self.take_count();
                if let Some(op) = pf.operator {
                    ta.start_selection();
                    self.apply_motion(motion, cnt, ta);
                    // f is inclusive of the target: extend one more for delete
                    if !pf.till && pf.forward {
                        ta.move_cursor(CursorMove::Forward);
                    }
                    // Task 11: for Change with find, handle directly for proper capture.
                    if op == Operator::Change && !self.replaying {
                        ta.cut();
                        self.register = RegisterKind::Charwise;
                        self.enter_insert_capture(Command::OperateMotion(op, motion, cnt), ta);
                    } else {
                        self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
                        if op != Operator::Change {
                            self.record(Command::OperateMotion(op, motion, cnt));
                        }
                    }
                    self.clear_pending();
                    return self.outcome_for(op);
                }
                self.apply_motion(motion, cnt, ta);
                self.clear_pending();
                return VimKeyOutcome::CursorOnly;
            }
            return VimKeyOutcome::NoOp; // non-char (e.g. Esc) cancels
        }

        // Esc cancels any pending Normal-mode sequence (operator, count, g, i/a object).
        if key.code == KeyCode::Esc {
            self.clear_pending();
            return VimKeyOutcome::NoOp;
        }

        // Task 6: Ctrl-r → redo (before the plain filter so it isn't stripped)
        if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let cnt = self.take_count();
            for _ in 0..cnt {
                ta.redo();
            }
            self.clear_pending();
            return VimKeyOutcome::TextMutated;
        }

        let plain =
            key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT;
        match key.code {
            KeyCode::Char(c) if plain => self.normal_char(c, ta),
            KeyCode::Left => {
                self.apply_motion(Motion::Left, 1, ta);
                self.clear_pending();
                VimKeyOutcome::CursorOnly
            }
            KeyCode::Right => {
                self.apply_motion(Motion::Right, 1, ta);
                self.clear_pending();
                VimKeyOutcome::CursorOnly
            }
            KeyCode::Up => {
                self.apply_motion(Motion::Up, 1, ta);
                self.clear_pending();
                VimKeyOutcome::CursorOnly
            }
            KeyCode::Down => {
                self.apply_motion(Motion::Down, 1, ta);
                self.clear_pending();
                VimKeyOutcome::CursorOnly
            }
            _ => VimKeyOutcome::NoOp,
        }
    }

    /// Map a Normal/Visual motion key to its Motion. Shared by normal_char and handle_visual.
    fn motion_for_char(c: char) -> Option<Motion> {
        match c {
            'h' => Some(Motion::Left),
            'l' => Some(Motion::Right),
            'k' => Some(Motion::Up),
            'j' => Some(Motion::Down),
            'w' | 'W' => Some(Motion::WordForward),
            'b' | 'B' => Some(Motion::WordBack),
            'e' | 'E' => Some(Motion::WordEnd),
            '0' => Some(Motion::LineStart),
            '^' => Some(Motion::FirstNonBlank),
            '$' => Some(Motion::LineEnd),
            'G' => Some(Motion::FileEnd),
            '{' => Some(Motion::ParagraphBack),
            '}' => Some(Motion::ParagraphForward),
            '%' => Some(Motion::MatchingPair),
            _ => None,
        }
    }

    // ── Plan 2 Task 2: count accumulation helpers ────────────────────────────

    fn take_count(&mut self) -> usize {
        self.pending_count.take().unwrap_or(1)
    }

    fn clear_pending(&mut self) {
        self.pending_count = None;
        self.pending_operator = None;
        self.pending_g = false;
        self.pending_replace = false;
        self.pending_object_kind = None;
        // pending_find is cleared by its own resolution path
    }

    /// Returns true if `c` was consumed as a count digit.
    fn accumulate_count(&mut self, c: char) -> bool {
        if c.is_ascii_digit() {
            // bare '0' with no pending count is the LineStart motion, not a digit
            if c == '0' && self.pending_count.is_none() {
                return false;
            }
            let d = c as usize - '0' as usize;
            self.pending_count = Some(self.pending_count.unwrap_or(0) * 10 + d);
            return true;
        }
        false
    }

    // ── Plan 2 Task 3: motion resolution ────────────────────────────────────

    fn apply_motion(&self, motion: Motion, count: usize, ta: &mut TextArea<'static>) {
        for _ in 0..count.max(1) {
            match motion {
                Motion::Left => ta.move_cursor(CursorMove::Back),
                Motion::Right => ta.move_cursor(CursorMove::Forward),
                Motion::Up => ta.move_cursor(CursorMove::Up),
                Motion::Down => ta.move_cursor(CursorMove::Down),
                Motion::WordForward => ta.move_cursor(CursorMove::WordForward),
                Motion::WordBack => ta.move_cursor(CursorMove::WordBack),
                Motion::WordEnd => ta.move_cursor(CursorMove::WordEnd),
                Motion::LineStart => ta.move_cursor(CursorMove::Head),
                Motion::FirstNonBlank => Self::first_non_blank(ta),
                Motion::LineEnd => ta.move_cursor(CursorMove::End),
                Motion::FileStart => ta.move_cursor(CursorMove::Top),
                Motion::FileEnd => ta.move_cursor(CursorMove::Bottom),
                Motion::ParagraphForward => ta.move_cursor(CursorMove::ParagraphForward),
                Motion::ParagraphBack => ta.move_cursor(CursorMove::ParagraphBack),
                Motion::MatchingPair => Self::match_pair(ta), // Task 9
                Motion::FindChar { ch, till, forward } => {
                    Self::find_char(ta, ch, till, forward); // Task 7
                }
            }
        }
    }

    fn first_non_blank(ta: &mut TextArea<'static>) {
        let (row, _) = super::cursor_tuple(ta);
        if let Some(line) = ta.lines().get(row) {
            let n = line.chars().take_while(|c| c.is_whitespace()).count();
            ta.move_cursor(CursorMove::Jump(row as u16, n as u16));
        }
    }

    /// Jump to the bracket that matches the one under the cursor (single-line).
    /// Opening bracket → search forward with depth counting to the matching
    /// close; closing bracket → search backward to the matching open.
    /// No-op if the cursor is not on a bracket or no match exists on the line.
    fn match_pair(ta: &mut TextArea<'static>) {
        let (row, col) = super::cursor_tuple(ta);
        let Some(line) = ta.lines().get(row).cloned() else { return };
        let chars: Vec<char> = line.chars().collect();
        let Some(&here) = chars.get(col) else { return };
        let pairs = [('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')];
        // open → search forward with depth counting to the matching close
        if let Some(&(_, close)) = pairs.iter().find(|&&(o, _)| o == here) {
            let mut depth = 0i32;
            for i in col..chars.len() {
                if chars[i] == here { depth += 1; }
                else if chars[i] == close { depth -= 1; if depth == 0 {
                    ta.move_cursor(CursorMove::Jump(row as u16, i as u16));
                    return;
                }}
            }
        // close → search backward with depth counting to the matching open
        } else if let Some(&(open, _)) = pairs.iter().find(|&&(_, c)| c == here) {
            let mut depth = 0i32;
            for i in (0..=col).rev() {
                if chars[i] == here { depth += 1; }
                else if chars[i] == open { depth -= 1; if depth == 0 {
                    ta.move_cursor(CursorMove::Jump(row as u16, i as u16));
                    return;
                }}
            }
        }
    }

    /// Find the next occurrence of `ch` on the current line.
    /// `forward`: search right from col+1; `!forward`: search left from col-1.
    /// `till`: stop one column short of the target (t/T behaviour).
    fn find_char(ta: &mut TextArea<'static>, ch: char, till: bool, forward: bool) {
        let (row, col) = super::cursor_tuple(ta);
        let Some(line) = ta.lines().get(row).cloned() else { return };
        let chars: Vec<char> = line.chars().collect();
        if forward {
            let start = col + 1;
            if let Some(pos) = (start..chars.len()).find(|&i| chars[i] == ch) {
                let target = if till { pos.saturating_sub(1) } else { pos };
                ta.move_cursor(CursorMove::Jump(row as u16, target as u16));
            }
        } else {
            if let Some(pos) = (0..col).rev().find(|&i| chars[i] == ch) {
                let target = if till { pos + 1 } else { pos };
                ta.move_cursor(CursorMove::Jump(row as u16, target as u16));
            }
        }
    }

    // ── Plan 2 Task 4: operator framework ───────────────────────────────────

    fn outcome_for(&self, op: Operator) -> VimKeyOutcome {
        match op {
            Operator::Yank => VimKeyOutcome::CursorOnly, // yank doesn't change text
            _ => VimKeyOutcome::TextMutated,
        }
    }

    /// Operate over the range from the cursor through `motion` (× count).
    fn apply_operator_motion(
        &mut self,
        op: Operator,
        m: Motion,
        count: usize,
        ta: &mut TextArea<'static>,
    ) {
        ta.start_selection();
        // Vim `cw`/`cW` semantics: change + word-forward uses word-end (not
        // word-start of the next word), so the trailing space is preserved.
        // This is vim's well-known `cw = ce` behaviour. Other operators (dw, yw)
        // use the motion as-is (including the trailing space).
        let effective_motion = if op == Operator::Change {
            match m {
                Motion::WordForward => Motion::WordEnd,
                other => other,
            }
        } else {
            m
        };
        self.apply_motion(effective_motion, count, ta);
        // WordEnd lands ON the last char of the word; ratatui selections are
        // exclusive of the cursor position, so [anchor, cursor) would miss that
        // last char.  Advance one extra position to make the selection inclusive
        // for all operators (de, ce, ye, and the cw=ce substitution path).
        if matches!(effective_motion, Motion::WordEnd) {
            ta.move_cursor(CursorMove::Forward);
        }
        // For Change, pass the actual command so dot-repeat captures the right
        // motion. The dummy OperateMotion(op, Motion::Right, 1) is replaced here.
        if op == Operator::Change && !self.replaying {
            ta.cut();
            self.register = RegisterKind::Charwise;
            self.enter_insert_capture(Command::OperateMotion(op, m, count), ta);
        } else {
            self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
        }
    }

    fn apply_operator_linewise(
        &mut self,
        op: Operator,
        count: usize,
        ta: &mut TextArea<'static>,
    ) {
        let (r0, _) = super::cursor_tuple(ta);
        let last = ta.lines().len().saturating_sub(1);
        let r1 = (r0 + count.saturating_sub(1)).min(last);

        // Register content: the line bodies plus a trailing newline (linewise).
        let body: String = ta.lines()[r0..=r1].join("\n");
        let register_text = format!("{body}\n");

        match op {
            Operator::Yank => {
                ta.set_yank_text(register_text);
                self.register = RegisterKind::Linewise;
                // cursor stays at start of first yanked line
                ta.move_cursor(CursorMove::Jump(r0 as u16, 0));
            }
            Operator::Delete | Operator::Change => {
                // Select the lines. Include the trailing newline if there is a
                // line after r1; otherwise (last line) include the PRECEDING
                // newline so no empty remnant is left.
                if r1 < last {
                    ta.move_cursor(CursorMove::Jump(r0 as u16, 0));
                    ta.start_selection();
                    ta.move_cursor(CursorMove::Jump((r1 + 1) as u16, 0));
                } else if r0 > 0 {
                    let prev_end = ta.lines()[r0 - 1].chars().count();
                    ta.move_cursor(CursorMove::Jump((r0 - 1) as u16, prev_end as u16));
                    ta.start_selection();
                    let end = ta.lines()[r1].chars().count();
                    ta.move_cursor(CursorMove::Jump(r1 as u16, end as u16));
                } else {
                    // whole buffer: select everything, leaving one empty line
                    ta.move_cursor(CursorMove::Jump(0, 0));
                    ta.start_selection();
                    let end = ta.lines()[r1].chars().count();
                    ta.move_cursor(CursorMove::Jump(r1 as u16, end as u16));
                }
                ta.cut();
                // cut() overwrote the yank buffer with the selected text (which
                // may include a leading newline on the last-line path); restore
                // the proper linewise register content.
                ta.set_yank_text(register_text);
                self.register = RegisterKind::Linewise;
                if op == Operator::Change {
                    // cc: open a fresh empty line to type into, at the right spot
                    if r0 == 0 && r1 == last {
                        // whole-buffer case: cut() left [""], the cursor is already
                        // at (0,0) on an empty line — no extra newline needed.
                        ta.move_cursor(CursorMove::Jump(0, 0));
                    } else if r0 > 0 && r1 == last {
                        // we consumed the preceding newline; add a line back
                        ta.move_cursor(CursorMove::End);
                        ta.insert_newline();
                    } else {
                        ta.insert_newline();
                        ta.move_cursor(CursorMove::Up);
                    }
                    self.mode = EditorMode::Insert;
                    self.insert_capture = Some(InsertCapture {
                        command: Command::OperateLine(op, count),
                        start: super::cursor_tuple(ta),
                    });
                }
            }
            Operator::Indent | Operator::Outdent => {
                // Linewise indent/outdent triggered by e.g. ">>" reaching
                // apply_operator_linewise is handled via normal_char's direct
                // indent_lines path. This arm is a safety net; it should not
                // normally be reached (>> goes through the doubled-operator path).
                let outdent = op == Operator::Outdent;
                let (r0, _) = super::cursor_tuple(ta);
                self.indent_lines(outdent, count, ta);
                ta.move_cursor(CursorMove::Jump(r0 as u16, 0));
            }
        }
    }

    fn apply_operator_to_line_end(&mut self, op: Operator, ta: &mut TextArea<'static>) {
        ta.start_selection();
        ta.move_cursor(CursorMove::End);
        // Task 11: for Change (C), use the correct command so dot-repeat works.
        if op == Operator::Change && !self.replaying {
            ta.cut();
            self.register = RegisterKind::Charwise;
            self.enter_insert_capture(Command::OperateToLineEnd(op), ta);
        } else {
            self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
        }
    }

    /// Indent (add 4 spaces) or outdent (remove up to 4 leading spaces) the
    /// cursor's line, then repeat for `count` lines total (moving down after
    /// each). Used by `>>`, `<<`, and the visual `>`/`<` operators.
    fn indent_lines(&self, outdent: bool, count: usize, ta: &mut TextArea<'static>) {
        for _ in 0..count.max(1) {
            ta.move_cursor(CursorMove::Head);
            if outdent {
                // Remove up to 4 leading spaces.
                let (row, _) = super::cursor_tuple(ta);
                let n = ta
                    .lines()
                    .get(row)
                    .map(|l| l.chars().take(4).take_while(|c| *c == ' ').count())
                    .unwrap_or(0);
                for _ in 0..n {
                    ta.delete_next_char();
                }
            } else {
                ta.insert_str("    ");
            }
            ta.move_cursor(CursorMove::Down);
        }
    }

    fn apply_operator_on_selection(
        &mut self,
        op: Operator,
        kind: RegisterKind,
        ta: &mut TextArea<'static>,
    ) {
        match op {
            Operator::Yank => {
                let start = ta.selection_range().map(|(s, _)| s);
                ta.copy();
                self.register = kind;
                ta.cancel_selection();
                if let Some((r, c)) = start {
                    ta.move_cursor(CursorMove::Jump(r as u16, c as u16));
                }
            }
            Operator::Delete => {
                ta.cut();
                self.register = kind;
            }
            Operator::Change => {
                ta.cut();
                self.register = kind;
                // Task 11: during replay, the `replay` branch inserts text
                // directly instead of re-entering Insert mode. Only call
                // enter_insert_capture when NOT replaying.
                if !self.replaying {
                    self.enter_insert_capture(Command::OperateMotion(op, Motion::Right, 1), ta);
                }
            }
            Operator::Indent | Operator::Outdent => {
                // Compute the selected row range, cancel the selection, then
                // indent/outdent those rows. This covers operator+motion (e.g.
                // `>j`) and visual `>`/`<` (which call this via handle_visual).
                let outdent = op == Operator::Outdent;
                let (rows, start_row) = if let Some(((sr, _), (er, _))) = ta.selection_range() {
                    (er.saturating_sub(sr) + 1, sr)
                } else {
                    let (r, _) = super::cursor_tuple(ta);
                    (1, r)
                };
                ta.cancel_selection();
                ta.move_cursor(CursorMove::Jump(start_row as u16, 0));
                self.indent_lines(outdent, rows, ta);
            }
        }
    }

    /// Slice the buffer text between two cursor positions (row, col), inclusive
    /// of `start` and exclusive of `end`. Works across lines: the result for a
    /// two-line insert is `"line1_suffix\nline2_prefix"`. Returns `""` when
    /// `end <= start`.
    fn text_between(lines: &[String], start: (usize, usize), end: (usize, usize)) -> String {
        if end <= start {
            return String::new();
        }
        let (sr, sc) = start;
        let (er, ec) = end;
        if sr == er {
            return lines
                .get(sr)
                .map(|l| l.chars().skip(sc).take(ec.saturating_sub(sc)).collect())
                .unwrap_or_default();
        }
        let mut out = String::new();
        if let Some(l) = lines.get(sr) {
            out.extend(l.chars().skip(sc));
        }
        out.push('\n');
        for r in (sr + 1)..er {
            if let Some(l) = lines.get(r) {
                out.push_str(l);
            }
            out.push('\n');
        }
        if let Some(l) = lines.get(er) {
            out.extend(l.chars().take(ec));
        }
        out
    }

    fn enter_insert_capture(&mut self, command: Command, ta: &TextArea<'static>) {
        self.mode = EditorMode::Insert;
        self.insert_capture = Some(InsertCapture {
            command,
            start: super::cursor_tuple(ta),
        });
    }

    // ── Plan 2 Task 11: dot-repeat helpers ──────────────────────────────────

    /// Record a completed mutating command in `last_change` (no inserted text).
    /// Called at every mutating, non-insert completion point.
    fn record(&mut self, command: Command) {
        self.last_change = Some(Change { command, inserted: None });
    }

    /// Replay a stored `Change` (the dot-repeat implementation).
    fn replay(&mut self, change: Change, ta: &mut TextArea<'static>) {
        match change.command {
            Command::DeleteChar { forward, count } => {
                for _ in 0..count {
                    if forward {
                        ta.delete_next_char();
                    } else {
                        ta.delete_char();
                    }
                }
            }
            Command::OperateMotion(op, m, count) => {
                let saved_inserted = change.inserted.clone();
                self.apply_operator_motion(op, m, count, ta);
                // For Change replays: insert the previously-typed text and
                // stay in Normal (replaying guard prevents re-entering Insert).
                if op == Operator::Change {
                    if let Some(text) = saved_inserted {
                        ta.insert_str(&text);
                        self.mode = EditorMode::Normal;
                    }
                }
            }
            Command::OperateObject(op, obj) => {
                self.apply_operator_object(op, obj, ta);
                if let (Operator::Change, Some(text)) = (op, change.inserted) {
                    ta.insert_str(&text);
                    self.mode = EditorMode::Normal;
                }
            }
            Command::ReplaceChar(ch) => {
                self.replace_char(ch, ta);
            }
            Command::JoinLines(n) => {
                for _ in 0..n.max(1) {
                    Self::join_line(ta);
                }
            }
            Command::ToggleCase(n) => {
                for _ in 0..n {
                    Self::toggle_case_at_cursor(ta);
                }
            }
            Command::Paste { after, count } => {
                self.paste(after, count, ta);
            }
            // OperateLine (dd/cc/yy), SubstituteLine, SubstituteChar, Undo, Redo:
            // best-effort / v1 no-op for dot-repeat. Linewise change-repeat is a
            // later enhancement. Undo/Redo repeat is intentionally skipped (vim
            // itself doesn't dot-repeat undo).
            _other => {}
        }
    }

    // ── Plan 2 Task 5: paste p/P ─────────────────────────────────────────────

    fn paste(&mut self, after: bool, count: usize, ta: &mut TextArea<'static>) {
        let text = ta.yank_text();
        if text.is_empty() {
            return;
        }
        match self.register {
            RegisterKind::Linewise => {
                let body = text.strip_suffix('\n').unwrap_or(&text);
                let n = count.max(1);
                if after {
                    ta.move_cursor(CursorMove::End);
                    for _ in 0..n {
                        ta.insert_newline();
                        ta.insert_str(body);
                    }
                } else {
                    ta.move_cursor(CursorMove::Head);
                    for _ in 0..n {
                        ta.insert_str(body);
                        ta.insert_newline();
                    }
                }
            }
            RegisterKind::Charwise => {
                if after {
                    let (row, col) = super::cursor_tuple(ta);
                    let len = ta.lines().get(row).map(|l| l.chars().count()).unwrap_or(col);
                    ta.move_cursor(CursorMove::Jump(row as u16, (col + 1).min(len) as u16));
                }
                for _ in 0..count.max(1) {
                    ta.insert_str(&text);
                }
            }
        }
    }

    // ── normal_char ──────────────────────────────────────────────────────────

    fn normal_char(&mut self, c: char, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        // Task 2: consume count digits first
        if self.accumulate_count(c) {
            return VimKeyOutcome::NoOp;
        }

        // Task 3: gg prefix
        if c == 'g' {
            if self.pending_g {
                self.pending_g = false;
                let cnt = self.take_count();
                if let Some(op) = self.pending_operator.take() {
                    self.apply_operator_motion(op, Motion::FileStart, cnt, ta);
                    if op != Operator::Change {
                        self.record(Command::OperateMotion(op, Motion::FileStart, cnt));
                    }
                    self.clear_pending();
                    return self.outcome_for(op);
                }
                self.apply_motion(Motion::FileStart, 1, ta);
                self.clear_pending();
                return VimKeyOutcome::CursorOnly;
            }
            self.pending_g = true;
            return VimKeyOutcome::NoOp;
        }

        // Task 4: operator-entry (d/c/y set pending; doubled → linewise; D/C/Y → to line end)
        let op_for_char = match c {
            'd' => Some(Operator::Delete),
            'c' => Some(Operator::Change),
            'y' => Some(Operator::Yank),
            _ => None,
        };
        if let Some(op) = op_for_char {
            if self.pending_operator == Some(op) {
                // doubled operator → linewise on `count` lines
                let cnt = self.take_count();
                self.apply_operator_linewise(op, cnt, ta);
                // Task 11: Change records on Esc via capture; others record here.
                if op != Operator::Change {
                    self.record(Command::OperateLine(op, cnt));
                }
                self.clear_pending();
                return self.outcome_for(op);
            }
            self.pending_operator = Some(op);
            return VimKeyOutcome::NoOp;
        }
        // D / C / Y → operator to line end
        if let Some(op) = match c {
            'D' => Some(Operator::Delete),
            'C' => Some(Operator::Change),
            'Y' => Some(Operator::Yank),
            _ => None,
        } {
            self.apply_operator_to_line_end(op, ta);
            // Task 11: C enters Insert (capture path owns last_change); D/Y record.
            if op != Operator::Change {
                self.record(Command::OperateToLineEnd(op));
            }
            self.clear_pending();
            return self.outcome_for(op);
        }

        // Task 12: >`>`/`<`< indent/outdent
        // First `>` or `<` sets pending_operator (Indent/Outdent) and returns NoOp.
        // Second matching char (doubled `>>` / `<<`) executes indent_lines on
        // `count` lines and clears pending.
        // If a non-matching key follows (e.g. `>j`), the operator is already set
        // and the motion dispatch below calls apply_operator_motion, which invokes
        // apply_operator_on_selection(Indent/Outdent, …) — also correct.
        if c == '>' || c == '<' {
            let outdent = c == '<';
            if (outdent && self.pending_operator == Some(Operator::Outdent))
                || (!outdent && self.pending_operator == Some(Operator::Indent))
            {
                // Doubled operator → indent the cursor's line `count` times.
                let cnt = self.take_count();
                self.indent_lines(outdent, cnt, ta);
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            self.pending_operator = Some(if outdent { Operator::Outdent } else { Operator::Indent });
            return VimKeyOutcome::NoOp;
        }

        // Task 5: paste p/P
        if c == 'p' || c == 'P' {
            let after = c == 'p';
            let cnt = self.take_count();
            self.paste(after, cnt, ta);
            self.record(Command::Paste { after, count: cnt });
            self.clear_pending();
            return VimKeyOutcome::TextMutated;
        }

        // Task 7: f/F/t/T — set pending_find (captures pending_operator so df, works)
        if let Some((till, forward)) = match c {
            'f' => Some((false, true)),
            'F' => Some((false, false)),
            't' => Some((true, true)),
            'T' => Some((true, false)),
            _ => None,
        } {
            self.pending_find = Some(PendingFind {
                operator: self.pending_operator.take(),
                till,
                forward,
            });
            return VimKeyOutcome::NoOp;
        }

        // Task 7: ; and , — repeat last find (same / opposite direction)
        if c == ';' || c == ',' {
            if let Some((ch, till, fwd)) = self.last_find {
                let forward = if c == ';' { fwd } else { !fwd };
                let cnt = self.take_count();
                self.apply_motion(Motion::FindChar { ch, till, forward }, cnt, ta);
            }
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }

        // Task 8: text object parsing — intercepted here, BEFORE the motion
        // dispatch, so that the object char (e.g. `w` in `diw`) is not consumed
        // as a motion. The `i`/`a` interception for `pending_object_kind` must
        // also be before insert-entry so that `di`/`ci`/`yi` work correctly.
        if self.pending_operator.is_some() {
            if c == 'i' || c == 'a' {
                self.pending_object_kind = Some(c == 'a');
                return VimKeyOutcome::NoOp;
            }
            if let Some(around) = self.pending_object_kind.take() {
                if let Some(obj) = Self::object_for_char(c, around) {
                    let op = self.pending_operator.take().unwrap();
                    self.apply_operator_object(op, obj, ta);
                    // Task 11: Change records on Esc via capture; others record here.
                    if op != Operator::Change {
                        self.record(Command::OperateObject(op, obj));
                    }
                    self.clear_pending();
                    return self.outcome_for(op);
                }
                // Unrecognised object char — clear state and fall through as NoOp.
                self.clear_pending();
                return VimKeyOutcome::NoOp;
            }
        }

        // Task 3: motion dispatch (count-aware)
        let count = self.pending_count.unwrap_or(1);
        if let Some(m) = Self::motion_for_char(c) {
            // If an operator is pending, this motion forms a range (Task 4).
            if let Some(op) = self.pending_operator.take() {
                self.apply_operator_motion(op, m, count, ta);
                // Task 11: record for non-Change ops (Change records on Esc via capture).
                // For Change, enter_insert_capture stores the command; last_change is set
                // when the user presses Esc.
                if op != Operator::Change {
                    self.record(Command::OperateMotion(op, m, count));
                }
                self.clear_pending();
                return self.outcome_for(op);
            }
            self.apply_motion(m, count, ta);
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }

        // Task 6: single-key edits
        match c {
            'x' => {
                let cnt = self.take_count();
                for _ in 0..cnt {
                    ta.delete_next_char();
                }
                self.record(Command::DeleteChar { forward: true, count: cnt });
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            'X' => {
                let cnt = self.take_count();
                for _ in 0..cnt {
                    ta.delete_char();
                }
                self.record(Command::DeleteChar { forward: false, count: cnt });
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            'r' => {
                self.pending_replace = true;
                return VimKeyOutcome::NoOp;
            }
            's' => {
                let cnt = self.take_count();
                for _ in 0..cnt {
                    ta.delete_next_char();
                }
                self.enter_insert_capture(Command::SubstituteChar(cnt), ta);
                self.clear_pending();
                return VimKeyOutcome::CursorOnly;
            }
            'S' => {
                ta.move_cursor(CursorMove::Head);
                ta.start_selection();
                ta.move_cursor(CursorMove::End);
                ta.cut();
                self.enter_insert_capture(Command::SubstituteLine, ta);
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            'J' => {
                let cnt = self.take_count().max(2) - 1;
                for _ in 0..cnt {
                    Self::join_line(ta);
                }
                self.record(Command::JoinLines(cnt));
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            '~' => {
                let cnt = self.take_count();
                for _ in 0..cnt {
                    Self::toggle_case_at_cursor(ta);
                }
                self.record(Command::ToggleCase(cnt));
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            'u' => {
                let cnt = self.take_count();
                for _ in 0..cnt {
                    ta.undo();
                }
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            _ => {}
        }

        // Task 11: `.` — replay the last mutating change (dot-repeat).
        if c == '.' {
            if let Some(change) = self.last_change.clone() {
                self.replaying = true;
                self.replay(change, ta);
                self.replaying = false;
            }
            self.clear_pending();
            return VimKeyOutcome::TextMutated;
        }

        // Task 10: v/V — enter Visual / Visual-line mode.
        if c == 'v' {
            ta.start_selection();
            self.mode = EditorMode::Visual;
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }
        if c == 'V' {
            ta.move_cursor(CursorMove::Head);
            ta.start_selection();
            ta.move_cursor(CursorMove::End);
            self.mode = EditorMode::VisualLine;
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }

        // Plan 3 Task 3: host actions — `:` `/` `?` `n` `N`.
        // These were previously NoOp; now they emit Host signals for the
        // component to turn into AppEvent / find-bar calls (adr/0012).
        // Note: `?` backward-first is deferred; `/` and `?` both open the
        // find bar for v1 — `n`/`N` navigate both directions.
        match c {
            ':' => { self.clear_pending(); return VimKeyOutcome::Host(VimHostAction::OpenPalette); }
            '/' => { self.clear_pending(); return VimKeyOutcome::Host(VimHostAction::OpenSearch { forward: true }); }
            '?' => { self.clear_pending(); return VimKeyOutcome::Host(VimHostAction::OpenSearch { forward: false }); }
            'n' => { self.clear_pending(); return VimKeyOutcome::Host(VimHostAction::SearchNext); }
            'N' => { self.clear_pending(); return VimKeyOutcome::Host(VimHostAction::SearchPrev); }
            _ => {}
        }

        // Insert-entry keys (from Plan 1, kept intact)
        // NOTE: i/a only reach here when NO operator is pending — operator + i/a
        // is handled above by the Task 8 text-object block.
        match c {
            'i' => self.enter_insert(ta, None),
            'a' => self.enter_insert(ta, Some(CursorMove::Forward)),
            'I' => self.enter_insert(ta, Some(CursorMove::Head)),
            'A' => self.enter_insert(ta, Some(CursorMove::End)),
            'o' => self.open_line(ta, false),
            'O' => self.open_line(ta, true),
            _ => { self.clear_pending(); VimKeyOutcome::NoOp }
        }
    }

    // ── Plan 2 Task 8: text object helpers ──────────────────────────────────

    /// Map an object char (e.g. `w`, `(`, `"`) to a `TextObject`.
    fn object_for_char(c: char, around: bool) -> Option<TextObject> {
        match c {
            'w' => Some(TextObject::Word { around }),
            '(' | ')' | 'b' => Some(TextObject::Pair { open: '(', close: ')', around }),
            '{' | '}' | 'B' => Some(TextObject::Pair { open: '{', close: '}', around }),
            '[' | ']' => Some(TextObject::Pair { open: '[', close: ']', around }),
            '<' | '>' => Some(TextObject::Pair { open: '<', close: '>', around }),
            '"' => Some(TextObject::Quote { ch: '"', around }),
            '\'' => Some(TextObject::Quote { ch: '\'', around }),
            '`' => Some(TextObject::Quote { ch: '`', around }),
            _ => None,
        }
    }

    /// Apply `op` over the text object `obj` at the current cursor position.
    fn apply_operator_object(
        &mut self,
        op: Operator,
        obj: TextObject,
        ta: &mut TextArea<'static>,
    ) {
        let (row, col) = super::cursor_tuple(ta);
        let Some(line) = ta.lines().get(row).cloned() else { return };
        let chars: Vec<char> = line.chars().collect();
        let Some((start, end)) = Self::object_range(&chars, col, obj) else { return };
        // Select [start, end) on this row via Jump, then apply the operator.
        ta.move_cursor(CursorMove::Jump(row as u16, start as u16));
        ta.start_selection();
        ta.move_cursor(CursorMove::Jump(row as u16, end as u16));
        // Task 11: for Change, use the correct command so dot-repeat captures it.
        if op == Operator::Change && !self.replaying {
            ta.cut();
            self.register = RegisterKind::Charwise;
            self.enter_insert_capture(Command::OperateObject(op, obj), ta);
        } else {
            self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
        }
    }

    /// Find the innermost enclosing pair `(open, close)` around `col`.
    /// If the cursor is on an open bracket, that bracket is the enclosing open.
    /// Otherwise scans left with depth counting (closing chars raise depth) to
    /// find the nearest unmatched open, then scans right from that open with
    /// depth counting to find the matching close.
    fn find_enclosing_pair(
        chars: &[char],
        col: usize,
        open: char,
        close: char,
    ) -> Option<(usize, usize)> {
        // Locate the open bracket that encloses col.
        let open_idx = if chars.get(col) == Some(&open) {
            col
        } else {
            let mut depth = 0usize;
            let mut found = None;
            for i in (0..col).rev() {
                if chars[i] == close {
                    depth += 1;
                } else if chars[i] == open {
                    if depth == 0 {
                        found = Some(i);
                        break;
                    }
                    depth -= 1;
                }
            }
            found?
        };
        // Find the matching close bracket scanning right from open_idx+1.
        let mut depth = 0usize;
        let mut close_idx = None;
        for i in (open_idx + 1)..chars.len() {
            if chars[i] == open {
                depth += 1;
            } else if chars[i] == close {
                if depth == 0 {
                    close_idx = Some(i);
                    break;
                }
                depth -= 1;
            }
        }
        Some((open_idx, close_idx?))
    }

    /// Returns the half-open `[start, end)` char range for `obj` centred at
    /// `col` within `chars`.
    ///
    /// NOTE: text objects are **single-line** in this implementation.
    /// Multi-line pair/quote spans are a later enhancement.
    fn object_range(chars: &[char], col: usize, obj: TextObject) -> Option<(usize, usize)> {
        if chars.is_empty() || col >= chars.len() {
            return None;
        }
        match obj {
            TextObject::Word { around } => {
                let is_word = |c: char| c.is_alphanumeric() || c == '_';
                // Expand left to the start of the word.
                let mut s = col;
                while s > 0 && is_word(chars[s - 1]) {
                    s -= 1;
                }
                // Expand right past the end of the word.
                let mut e = col;
                while e < chars.len() && is_word(chars[e]) {
                    e += 1;
                }
                if around {
                    // Also consume trailing whitespace (vim `aw` behaviour).
                    while e < chars.len() && chars[e].is_whitespace() {
                        e += 1;
                    }
                }
                Some((s, e))
            }
            TextObject::Quote { ch, around } => {
                // Collect all positions of the quote character on this line.
                let positions: Vec<usize> = chars
                    .iter()
                    .enumerate()
                    .filter(|&(_, &c)| c == ch)
                    .map(|(i, _)| i)
                    .collect();
                // Find the pair that strictly contains the cursor (p[0] <= col <= p[1]).
                // Cursor in the gap between two quoted spans returns None (no-op).
                let pair = positions
                    .chunks(2)
                    .find(|p| p.len() == 2 && p[0] <= col && col <= p[1])?;
                let (o, c) = (pair[0], pair[1]);
                if around {
                    Some((o, c + 1))
                } else {
                    Some((o + 1, c))
                }
            }
            TextObject::Pair { open, close, around } => {
                let (o, c) = Self::find_enclosing_pair(chars, col, open, close)?;
                if around {
                    Some((o, c + 1))
                } else {
                    Some((o + 1, c))
                }
            }
        }
    }

    fn enter_insert(
        &mut self,
        ta: &mut TextArea<'static>,
        pre_move: Option<CursorMove>,
    ) -> VimKeyOutcome {
        if let Some(m) = pre_move {
            ta.move_cursor(m);
        }
        self.mode = EditorMode::Insert;
        self.clear_pending();
        VimKeyOutcome::CursorOnly
    }

    fn open_line(&mut self, ta: &mut TextArea<'static>, above: bool) -> VimKeyOutcome {
        if above {
            ta.move_cursor(CursorMove::Head);
            ta.insert_newline();
            ta.move_cursor(CursorMove::Up);
        } else {
            ta.move_cursor(CursorMove::End);
            ta.insert_newline();
        }
        self.mode = EditorMode::Insert;
        self.clear_pending();
        VimKeyOutcome::TextMutated
    }

    // ── Plan 2 Task 6: single-key edit helpers ───────────────────────────────

    /// Replace the char under the cursor with `c`, stay in Normal mode.
    fn replace_char(&mut self, c: char, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        if ta.delete_next_char() {
            ta.insert_char(c);
            ta.move_cursor(CursorMove::Back);
            self.last_change = Some(Change { command: Command::ReplaceChar(c), inserted: None });
            VimKeyOutcome::TextMutated
        } else {
            VimKeyOutcome::NoOp
        }
    }

    /// Join the next line up onto the current one by removing the trailing newline.
    // TODO(J): real vim inserts a space between the joined lines and strips the
    //          next line's leading whitespace; this just deletes the newline.
    fn join_line(ta: &mut TextArea<'static>) {
        ta.move_cursor(CursorMove::End);
        ta.delete_next_char(); // removes the newline, joining the next line up
    }

    /// Toggle the case of the char under the cursor and advance one char.
    fn toggle_case_at_cursor(ta: &mut TextArea<'static>) {
        let (row, col) = super::cursor_tuple(ta);
        if let Some(line) = ta.lines().get(row) {
            if let Some(ch) = line.chars().nth(col) {
                let flipped: String = if ch.is_uppercase() {
                    ch.to_lowercase().collect()
                } else {
                    ch.to_uppercase().collect()
                };
                ta.delete_next_char();
                ta.insert_str(&flipped);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui_textarea::TextArea;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }
    fn ta() -> TextArea<'static> {
        TextArea::from(["hello world", "second line"])
    }

    // ── Plan 1 tests (must stay green) ──────────────────────────────────────

    #[test]
    fn i_enters_insert_mode() {
        let mut e = VimEngine::default();
        let mut t = ta();
        let out = e.handle_key(&key('i'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Insert);
        assert_eq!(out, VimKeyOutcome::CursorOnly);
    }

    #[test]
    fn esc_returns_to_normal_and_steps_back() {
        let mut e = VimEngine::default();
        let mut t = ta();
        e.handle_key(&key('i'), &mut t);
        t.move_cursor(ratatui_textarea::CursorMove::Forward);
        t.move_cursor(ratatui_textarea::CursorMove::Forward);
        let col_before = super::super::cursor_tuple(&t).1;
        let out = e.handle_key(&esc(), &mut t);
        assert_eq!(*e.mode(), EditorMode::Normal);
        assert_eq!(out, VimKeyOutcome::CursorOnly);
        assert_eq!(super::super::cursor_tuple(&t).1, col_before - 1);
    }

    #[test]
    fn insert_mode_passes_through() {
        let mut e = VimEngine::default();
        let mut t = ta();
        e.handle_key(&key('i'), &mut t);
        let out = e.handle_key(&key('x'), &mut t);
        assert_eq!(out, VimKeyOutcome::PassThrough);
    }

    #[test]
    fn l_moves_right_cursor_only() {
        let mut e = VimEngine::default();
        let mut t = ta();
        let out = e.handle_key(&key('l'), &mut t);
        assert_eq!(out, VimKeyOutcome::CursorOnly);
        assert_eq!(super::super::cursor_tuple(&t), (0, 1));
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    #[test]
    fn a_enters_insert_after_cursor() {
        let mut e = VimEngine::default();
        let mut t = ta();
        e.handle_key(&key('a'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Insert);
        assert_eq!(super::super::cursor_tuple(&t), (0, 1));
    }

    #[test]
    fn o_opens_line_below_in_insert() {
        let mut e = VimEngine::default();
        let mut t = ta();
        let out = e.handle_key(&key('o'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Insert);
        assert_eq!(out, VimKeyOutcome::TextMutated);
        assert_eq!(t.lines().len(), 3);
        assert_eq!(super::super::cursor_tuple(&t).0, 1);
    }

    #[test]
    fn reset_returns_to_normal_from_insert() {
        let mut e = VimEngine::default();
        let mut t = ta();
        e.handle_key(&key('i'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Insert);
        e.reset_to_normal();
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    #[test]
    fn unknown_normal_key_is_noop() {
        let mut e = VimEngine::default();
        let mut t = ta();
        let out = e.handle_key(&key('z'), &mut t);
        assert_eq!(out, VimKeyOutcome::NoOp);
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    // ── Plan 2 Task 2 tests ──────────────────────────────────────────────────

    #[test]
    fn count_accumulates_then_moves() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abcdef"]);
        e.handle_key(&key('3'), &mut t);
        e.handle_key(&key('l'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 3));
        // pending cleared after the motion
        e.handle_key(&key('l'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 4));
    }

    #[test]
    fn zero_without_count_is_line_start() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abcdef"]);
        e.handle_key(&key('l'), &mut t);
        e.handle_key(&key('l'), &mut t);
        e.handle_key(&key('0'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 0));
    }

    // ── Plan 2 Task 3 tests ──────────────────────────────────────────────────

    #[test]
    #[allow(non_snake_case)]
    fn gg_and_G_jump_file_ends() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('G'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t).0, 2);
        e.handle_key(&key('g'), &mut t);
        e.handle_key(&key('g'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t).0, 0);
    }

    #[test]
    fn pending_g_cancels_on_unmapped_key() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('G'), &mut t);            // go to last line
        assert_eq!(super::super::cursor_tuple(&t).0, 2);
        e.handle_key(&key('g'), &mut t);            // start gg
        e.handle_key(&key('z'), &mut t);            // unmapped → should cancel pending g
        e.handle_key(&key('g'), &mut t);            // lone g, NOT gg
        assert_eq!(super::super::cursor_tuple(&t).0, 2, "stray g after cancelled prefix must not jump to file start");
    }

    #[test]
    fn pending_g_cleared_through_insert() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('G'), &mut t);
        e.handle_key(&key('g'), &mut t);            // start gg
        e.handle_key(&key('a'), &mut t);            // enter insert (should clear pending_g)
        e.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut t);
        e.handle_key(&key('g'), &mut t);            // lone g
        assert_eq!(super::super::cursor_tuple(&t).0, 2, "g after insert must not complete a stale gg");
    }

    // ── Plan 2 Task 4 tests ──────────────────────────────────────────────────

    #[test]
    fn dw_deletes_word() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello world"]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('w'), &mut t);
        assert_eq!(t.lines(), &["world"]);
    }

    #[test]
    fn dd_deletes_line_linewise() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('d'), &mut t);
        assert_eq!(t.lines(), &["two", "three"]);
        assert_eq!(e.register, RegisterKind::Linewise);
    }

    #[test]
    fn yy_then_p_duplicates_line() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two"]);
        e.handle_key(&key('y'), &mut t);
        e.handle_key(&key('y'), &mut t);
        e.handle_key(&key('p'), &mut t);
        assert_eq!(t.lines(), &["one", "one", "two"]);
    }

    #[test]
    fn cw_deletes_word_and_enters_insert() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello world"]);
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('w'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Insert);
        // Vim `cw` = `ce`: deletes up to end of word (exclusive of trailing
        // space), so " world" remains (space preserved). This matches vim's
        // actual cw = ce behaviour.
        assert_eq!(t.lines(), &[" world"]);
    }

    // ── Plan 2 Task 5 tests ──────────────────────────────────────────────────

    #[test]
    fn charwise_p_pastes_after_cursor() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc"]);
        // yank the first char with `yl`
        e.handle_key(&key('y'), &mut t);
        e.handle_key(&key('l'), &mut t);
        e.handle_key(&key('p'), &mut t);
        assert_eq!(t.lines(), &["aabc"]);
    }

    #[test]
    fn dd_on_last_line_removes_it() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('G'), &mut t); // to last line
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('d'), &mut t);
        assert_eq!(t.lines(), &["one", "two"]);
    }

    #[test]
    fn dd_on_only_line_leaves_empty() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["only"]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('d'), &mut t);
        assert_eq!(t.lines(), &[""]);
    }

    #[test]
    fn linewise_2p_inserts_two_copies() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two"]);
        e.handle_key(&key('y'), &mut t);
        e.handle_key(&key('y'), &mut t); // yank "one" linewise
        e.handle_key(&key('2'), &mut t);
        e.handle_key(&key('p'), &mut t);
        assert_eq!(t.lines(), &["one", "one", "one", "two"]);
    }

    #[test]
    fn yy_last_line_then_p_duplicates() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two"]);
        e.handle_key(&key('G'), &mut t); // last line "two"
        e.handle_key(&key('y'), &mut t);
        e.handle_key(&key('y'), &mut t);
        e.handle_key(&key('p'), &mut t);
        assert_eq!(t.lines(), &["one", "two", "two"]);
    }

    // ── Plan 2 Task 6 tests ──────────────────────────────────────────────────

    #[test]
    fn x_deletes_char_under_cursor() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc"]);
        e.handle_key(&key('x'), &mut t);
        assert_eq!(t.lines(), &["bc"]);
    }

    #[test]
    fn r_replaces_char() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc"]);
        e.handle_key(&key('r'), &mut t);
        e.handle_key(&key('Z'), &mut t);
        assert_eq!(t.lines(), &["Zbc"]);
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    #[test]
    fn u_undoes_last_edit() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc"]);
        e.handle_key(&key('x'), &mut t);
        e.handle_key(&key('u'), &mut t);
        assert_eq!(t.lines(), &["abc"]);
    }

    #[test]
    fn tilde_toggles_case() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc"]);
        e.handle_key(&key('~'), &mut t);
        assert_eq!(t.lines(), &["Abc"]);
    }

    // ── Plan 2 Task 7 tests ──────────────────────────────────────────────────

    #[test]
    fn f_moves_to_char() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello, world"]);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key(','), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 5));
    }

    #[test]
    fn df_deletes_through_char() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello, world"]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key(','), &mut t);
        assert_eq!(t.lines(), &[" world"]);
    }

    #[test]
    fn t_stops_before_char() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello, world"]);
        e.handle_key(&key('t'), &mut t);
        e.handle_key(&key(','), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 4)); // on 'o', before ','
    }

    #[test]
    fn semicolon_repeats_find() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["a.b.c.d"]);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('.'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t).1, 1);
        e.handle_key(&key(';'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t).1, 3);
    }

    // ── Plan 2 Task 8 tests ──────────────────────────────────────────────────

    #[test]
    fn diw_deletes_inner_word() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar baz"]);
        // cursor on 'b' of "bar"
        e.handle_key(&key('w'), &mut t);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('i'), &mut t);
        e.handle_key(&key('w'), &mut t);
        assert_eq!(t.lines(), &["foo  baz"]);
    }

    #[test]
    fn ci_quote_changes_inside_quotes() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["say \"hi\" now"]);
        // move onto the text inside quotes: f then h lands on 'h' (col 5)
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('h'), &mut t);
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('i'), &mut t);
        e.handle_key(&key('"'), &mut t);
        assert_eq!(t.lines(), &["say \"\" now"]);
        assert_eq!(*e.mode(), EditorMode::Insert);
    }

    #[test]
    fn di_paren_deletes_inside_parens() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo(bar)baz"]);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('('), &mut t); // cursor on '('
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('i'), &mut t);
        e.handle_key(&key('('), &mut t);
        assert_eq!(t.lines(), &["foo()baz"]);
    }

    #[test]
    fn daw_deletes_word_and_trailing_space() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar baz"]);
        e.handle_key(&key('w'), &mut t); // onto "bar"
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('a'), &mut t);
        e.handle_key(&key('w'), &mut t);
        assert_eq!(t.lines(), &["foo baz"]);
    }

    // ── Plan 2 Task 9 tests ──────────────────────────────────────────────────

    #[test]
    fn percent_jumps_to_matching_paren() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo(bar)baz"]);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('('), &mut t); // cursor on '('
        e.handle_key(&key('%'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 7)); // matching ')'
    }

    #[test]
    fn percent_jumps_back_from_close() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo(bar)baz"]);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key(')'), &mut t); // cursor on ')'
        e.handle_key(&key('%'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 3)); // back to '('
    }

    #[test]
    fn percent_handles_nested() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["(a(b)c)"]);
        // cursor on outer '(' at col 0
        e.handle_key(&key('%'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 6)); // matching outer ')'
    }

    // ── Plan 2 Task 10 tests ─────────────────────────────────────────────────

    /// Charwise Visual is inclusive of the cursor char. `v` + 2×`l` leaves
    /// the cursor on col 2 ('l'); the inclusive range covers cols 0,1,2 = "hel".
    #[test]
    fn v_motion_d_deletes_selection() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello"]);
        e.handle_key(&key('v'), &mut t);   // anchor col 0
        e.handle_key(&key('l'), &mut t);   // cursor → col 1
        e.handle_key(&key('l'), &mut t);   // cursor → col 2, inclusive covers "hel"
        e.handle_key(&key('d'), &mut t);   // delete "hel"
        assert_eq!(t.lines(), &["lo"]); // inclusive: deletes cols 0,1,2 ("hel")
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    #[test]
    #[allow(non_snake_case)]
    fn V_then_d_deletes_line() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two"]);
        e.handle_key(&key('V'), &mut t);
        e.handle_key(&key('d'), &mut t);
        assert_eq!(t.lines(), &["two"]);
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    /// Inclusive yank: v + l (cursor col 1) yanks "he" (2 chars, inclusive).
    /// After p pastes the yanked text, buffer grew by 2.
    #[test]
    fn visual_y_yanks_and_returns_to_normal() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello"]);
        e.handle_key(&key('v'), &mut t);   // anchor col 0
        e.handle_key(&key('l'), &mut t);   // cursor col 1, inclusive selection "he"
        e.handle_key(&key('y'), &mut t);   // yank "he" (2 chars), mode → Normal
        assert_eq!(*e.mode(), EditorMode::Normal);
        // p pastes the yanked "he" after current cursor
        let before_len: usize = t.lines().iter().map(|l| l.len()).sum();
        e.handle_key(&key('p'), &mut t);
        let after_len: usize = t.lines().iter().map(|l| l.len()).sum();
        // buffer grew by exactly 2 chars (the yanked "he")
        assert_eq!(after_len, before_len + 2);
    }

    #[test]
    fn visual_esc_cancels_and_returns_normal() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello"]);
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('l'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Visual);
        e.handle_key(&esc(), &mut t);
        assert_eq!(*e.mode(), EditorMode::Normal);
        // selection should be cancelled
        assert!(t.selection_range().is_none());
    }

    #[test]
    fn visual_c_enters_insert_after_delete() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello"]);
        e.handle_key(&key('v'), &mut t);   // anchor col 0
        e.handle_key(&key('l'), &mut t);   // cursor col 1, inclusive covers "he"
        e.handle_key(&key('c'), &mut t);   // delete "he" (inclusive), enter Insert
        assert_eq!(*e.mode(), EditorMode::Insert);
        assert_eq!(t.lines(), &["llo"]); // inclusive: deletes cols 0,1 ("he")
    }

    // ── Plan 2 Task 12 tests ─────────────────────────────────────────────────

    #[test]
    fn indent_line_adds_spaces() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["x"]);
        e.handle_key(&key('>'), &mut t);
        e.handle_key(&key('>'), &mut t);
        assert_eq!(t.lines(), &["    x"]);
    }

    #[test]
    fn outdent_removes_spaces() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["        x"]); // 8 spaces
        e.handle_key(&key('<'), &mut t);
        e.handle_key(&key('<'), &mut t);
        assert_eq!(t.lines(), &["    x"]); // removed 4
    }

    #[test]
    fn pending_hint_shows_operator_and_count() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc"]);
        e.handle_key(&key('2'), &mut t);
        e.handle_key(&key('d'), &mut t);
        assert_eq!(e.pending_hint().as_deref(), Some("2d"));
    }

    // ── Plan 2 Task 11 tests ─────────────────────────────────────────────────

    #[test]
    fn dot_repeats_x() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abcdef"]);
        e.handle_key(&key('x'), &mut t);
        e.handle_key(&key('.'), &mut t);
        assert_eq!(t.lines(), &["cdef"]);
    }

    #[test]
    fn dot_repeats_dw() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one two three four"]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('w'), &mut t); // delete "one "
        e.handle_key(&key('.'), &mut t); // delete "two "
        assert_eq!(t.lines(), &["three four"]);
    }

    #[test]
    fn dot_repeats_change_with_typed_text() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar"]);
        // cw -> type "X" -> Esc, then move to next word and dot
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('w'), &mut t); // cw: deletes "foo" (cw=ce keeps trailing space), enters Insert at col 0
        // simulate the user typing "X" via the host PassThrough path:
        t.insert_str("X");
        e.handle_key(&esc(), &mut t);     // capture "X"
        e.handle_key(&key('w'), &mut t);  // move to "bar"
        e.handle_key(&key('.'), &mut t);  // repeat cw+X
        assert_eq!(t.lines(), &["X X"]);
    }

    #[test]
    fn dot_repeats_multiline_change() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar"]);
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('w'), &mut t); // cw on "foo" → Insert at col 0
        t.insert_str("a");
        t.insert_newline();
        t.insert_str("b");               // typed "a\nb"
        e.handle_key(&esc(), &mut t);    // captures "a\nb"
        // Buffer is now ["a", "b bar"]; cursor stepped back to col 0 of row 1 ("b bar").
        // Confirm the multi-line buffer state from the insert:
        assert_eq!(t.lines(), &["a", "b bar"]);

        // Verify replay: position on "bar", run `.`, should produce "a\nb" again.
        // Move to word "bar" (it is at col 2 of row 1).
        e.handle_key(&key('w'), &mut t);  // move to "bar" (word-forward from "b" → "bar")
        e.handle_key(&key('.'), &mut t);  // replay: cw on "bar" → insert "a\nb"
        // After replay the buffer should have "a\nb" inserted in place of "bar":
        // row 1 was "b bar", cw from "bar" removes "bar", inserts "a\nb" → ["a", "b a", "b"]
        assert!(t.lines().len() >= 3, "replay of multiline insert should produce >=3 lines: {:?}", t.lines());
    }

    // ── Plan 3 Task 4: space_leads predicate tests ───────────────────────────

    #[test]
    fn space_leads_only_in_clean_normal() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["x"]);
        assert!(e.space_leads());
        e.handle_key(&key('d'), &mut t); // operator pending
        assert!(!e.space_leads());
        e.handle_key(&key('w'), &mut t); // completes dw, clears pending
        assert!(e.space_leads());
        e.handle_key(&key('i'), &mut t); // insert
        assert!(!e.space_leads());
    }

    // ── Plan 3 Task 3: host-action tests ────────────────────────────────────

    #[test]
    fn colon_emits_open_palette() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["x"]);
        assert_eq!(e.handle_key(&key(':'), &mut t), VimKeyOutcome::Host(VimHostAction::OpenPalette));
    }

    #[test]
    fn slash_emits_open_search_forward() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["x"]);
        assert_eq!(e.handle_key(&key('/'), &mut t), VimKeyOutcome::Host(VimHostAction::OpenSearch { forward: true }));
    }

    #[test]
    #[allow(non_snake_case)]
    fn n_and_N_emit_search_nav() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["x"]);
        assert_eq!(e.handle_key(&key('n'), &mut t), VimKeyOutcome::Host(VimHostAction::SearchNext));
        assert_eq!(e.handle_key(&key('N'), &mut t), VimKeyOutcome::Host(VimHostAction::SearchPrev));
    }

    // ── Plan 3 Task 5: mouse → Visual mode tests ────────────────────────────

    #[test]
    fn mouse_selection_enters_and_leaves_visual() {
        let mut e = VimEngine::default();
        e.sync_mouse_selection(true);
        assert_eq!(*e.mode(), EditorMode::Visual);
        e.sync_mouse_selection(false);
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    #[test]
    fn mouse_no_selection_in_normal_stays_normal() {
        let mut e = VimEngine::default();
        e.sync_mouse_selection(false);
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    #[test]
    fn mouse_does_not_disturb_insert() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["x"]);
        e.handle_key(&key('i'), &mut t); // Insert
        e.sync_mouse_selection(true);
        assert_eq!(*e.mode(), EditorMode::Insert); // mouse doesn't yank Insert into Visual
    }

    // ── Bug-fix regression tests ─────────────────────────────────────────────

    #[test]
    fn di_paren_on_empty_line_does_not_panic() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from([""]); // empty line
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('i'), &mut t);
        e.handle_key(&key('('), &mut t); // must not panic; no-op
        assert_eq!(t.lines(), &[""]);
    }

    #[test]
    fn esc_clears_pending_g_in_normal() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('G'), &mut t); // last line
        assert_eq!(super::super::cursor_tuple(&t).0, 2);
        e.handle_key(&key('g'), &mut t);                                  // start gg
        e.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut t); // cancel
        e.handle_key(&key('g'), &mut t);                                  // lone g
        assert_eq!(super::super::cursor_tuple(&t).0, 2, "Esc must cancel pending g");
    }

    #[test]
    fn esc_clears_pending_operator_object_in_normal() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar baz"]);
        e.handle_key(&key('d'), &mut t); // operator pending
        e.handle_key(&key('i'), &mut t); // object kind pending (NOT insert — operator pending)
        e.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut t); // cancel
        // buffer unchanged (no diw happened)
        assert_eq!(t.lines(), &["foo bar baz"]);
        // and we're back to clean Normal: a plain motion works, mode still Normal
        e.handle_key(&key('w'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Normal);
        assert_eq!(t.lines(), &["foo bar baz"], "w after Esc must be a motion, not diw");
    }

    // ── Bug A: di( on nested parens ─────────────────────────────────────────

    #[test]
    fn di_paren_nested_selects_inner_of_outer() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["((x))"]);
        // cursor at col 0 (outer '(')
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('i'), &mut t);
        e.handle_key(&key('('), &mut t);
        assert_eq!(t.lines(), &["()"]); // outer kept, inner content "(x)" deleted
    }

    #[test]
    fn di_paren_from_inside_nested() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["((x))"]);
        e.handle_key(&key('l'), &mut t); // col1 (inner '(')
        e.handle_key(&key('l'), &mut t); // col2 ('x')
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('i'), &mut t);
        e.handle_key(&key('('), &mut t);
        assert_eq!(t.lines(), &["(())"]); // inner content "x" deleted
    }

    // ── Bug B: di" in gap between pairs ─────────────────────────────────────

    #[test]
    fn di_quote_in_gap_is_noop() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["\"foo\" \"bar\""]);
        // move cursor to the space (col 5) between the two strings
        for _ in 0..5 { e.handle_key(&key('l'), &mut t); }
        assert_eq!(super::super::cursor_tuple(&t).1, 5);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('i'), &mut t);
        e.handle_key(&key('"'), &mut t);
        assert_eq!(t.lines(), &["\"foo\" \"bar\""]); // unchanged (no-op)
    }

    #[test]
    fn di_quote_inside_still_works() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["\"foo\" \"bar\""]);
        for _ in 0..7 { e.handle_key(&key('l'), &mut t); } // inside "bar"
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('i'), &mut t);
        e.handle_key(&key('"'), &mut t);
        assert_eq!(t.lines(), &["\"foo\" \"\""]); // bar deleted, foo intact
    }

    // ── Bug C: df<last-char> must not join next line ─────────────────────────

    #[test]
    fn df_last_char_does_not_join_next_line() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc", "xyz"]);
        // cursor at (0,0); df c  → delete through the 'c' (last char of line 0)
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('c'), &mut t);
        assert_eq!(t.lines(), &["", "xyz"]); // line 0 emptied, newline + line 1 intact
    }

    // ── Bug D: cc on a single-line buffer ────────────────────────────────────

    #[test]
    fn cc_single_line_leaves_one_empty_line() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello"]);
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('c'), &mut t);
        assert_eq!(t.lines(), &[""]);
        assert_eq!(*e.mode(), EditorMode::Insert);
    }

    #[test]
    fn cc_middle_line_still_works() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one","two","three"]);
        e.handle_key(&key('j'), &mut t); // line "two"
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('c'), &mut t);
        assert_eq!(t.lines(), &["one","","three"]);
        assert_eq!(*e.mode(), EditorMode::Insert);
    }

    // ── Bug E: r on empty line must be no-op ────────────────────────────────

    #[test]
    fn r_on_empty_line_is_noop() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from([""]);
        e.handle_key(&key('r'), &mut t);
        let out = e.handle_key(&key('Z'), &mut t);
        assert_eq!(out, VimKeyOutcome::NoOp);
        assert_eq!(t.lines(), &[""]);
    }

    // ── P2.G: charwise Visual inclusive tests ────────────────────────────────

    #[test]
    fn visual_v_then_d_deletes_char_under_cursor() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc"]);
        e.handle_key(&key('v'), &mut t); // select just 'a' (cursor col0)
        e.handle_key(&key('d'), &mut t);
        assert_eq!(t.lines(), &["bc"]); // 'a' deleted (inclusive of cursor char)
    }

    #[test]
    fn visual_e_then_d_inclusive() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello world"]);
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('e'), &mut t); // cursor on 'o' col4
        e.handle_key(&key('d'), &mut t);
        assert_eq!(t.lines(), &[" world"]); // "hello" deleted inclusive
    }

    // ── Bug fix: vim `e` must land ON the last word char (inclusive) ─────────

    #[test]
    fn e_lands_on_last_word_char() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello world"]);
        e.handle_key(&key('e'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 4)); // 'o', last char of "hello"
    }

    #[test]
    fn e_twice_reaches_second_word_end() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello world"]);
        e.handle_key(&key('e'), &mut t);
        e.handle_key(&key('e'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 10)); // 'd', last char of "world"
    }

    #[test]
    fn de_deletes_to_word_end_inclusive() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello world"]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('e'), &mut t);
        assert_eq!(t.lines(), &[" world"]); // deletes "hello" inclusive of 'o'
    }

    // ── Bug fix: vim yank leaves cursor at selection start; charwise p never wraps ──

    #[test]
    fn visual_y_leaves_cursor_at_selection_start() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar", "baz"]);
        for _ in 0..4 { e.handle_key(&key('l'), &mut t); } // onto 'b' of "bar" (col 4)
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('e'), &mut t); // select "bar"
        e.handle_key(&key('y'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 4)); // cursor at start of selection, not the end
    }

    #[test]
    fn charwise_p_after_eol_word_does_not_touch_next_line() {
        // reproduce the user's bug: yank an end-of-line word, paste, must NOT hit the line below
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar", "baz"]);
        for _ in 0..4 { e.handle_key(&key('l'), &mut t); } // 'b' of "bar"
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('e'), &mut t); // select "bar" (end of line 0)
        e.handle_key(&key('y'), &mut t); // yank; cursor → col 4
        e.handle_key(&key('p'), &mut t); // paste after cursor char 'b'
        assert_eq!(t.lines()[1], "baz");           // line below UNTOUCHED
        assert_eq!(t.lines().len(), 2);            // no new line, no merge
        assert_eq!(t.lines()[0], "foo bbarar");    // "bar" pasted after 'b' on line 0 (vim p-after)
    }

    #[test]
    fn charwise_p_at_line_end_appends_same_line() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["ab", "cd"]);
        // yank "ab" charwise via v e y
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('e'), &mut t); // select "ab"
        e.handle_key(&key('y'), &mut t); // cursor → col 0
        e.handle_key(&key('$'), &mut t); // to last char of line 0 ('b')
        e.handle_key(&key('p'), &mut t); // append "ab" after 'b'
        assert_eq!(t.lines()[0], "abab");
        assert_eq!(t.lines()[1], "cd"); // line below untouched
    }

    // ── Visual p: replace selection with register ────────────────────────────

    #[test]
    fn visual_p_replaces_charwise_selection() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar"]);
        // yank "foo" (v e y at col 0) → register = "foo", cursor back to col 0
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('e'), &mut t);
        e.handle_key(&key('y'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 0));
        // select "bar" and paste over it
        for _ in 0..4 { e.handle_key(&key('l'), &mut t); } // onto 'b' (col 4)
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('e'), &mut t); // select "bar"
        e.handle_key(&key('p'), &mut t);
        assert_eq!(t.lines(), &["foo foo"]); // "bar" replaced by "foo"
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    #[test]
    fn visual_p_register_preserved_for_repeat() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["aa bb cc"]);
        // yank "aa"
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('e'), &mut t);
        e.handle_key(&key('y'), &mut t); // reg = "aa", cursor col 0
        // replace "bb"
        for _ in 0..3 { e.handle_key(&key('l'), &mut t); } // col 3 'b'
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('e'), &mut t);
        e.handle_key(&key('p'), &mut t); // "bb" -> "aa"
        assert_eq!(t.lines(), &["aa aa cc"]);
        // replace "cc" with the SAME register (preserved)
        // cursor is at start of the just-pasted "aa" (col 3); move to "cc"
        for _ in 0..3 { e.handle_key(&key('l'), &mut t); } // toward "cc" (col 6 'c')
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('e'), &mut t);
        e.handle_key(&key('p'), &mut t);
        assert_eq!(t.lines(), &["aa aa aa"]);
    }
}

