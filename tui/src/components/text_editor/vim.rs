//! Built-in vim emulation: a modal input interpreter over a `TextArea`.
//! Pure over `&mut TextArea` — no component state, no async (adr/0012).

use super::snapshot::EditorMode;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::{CursorMove, TextArea};

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
#[derive(Debug, Clone)]
pub enum Command {
    Move(Motion, usize),                    // motion, count
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
    Undo(usize),
    Redo(usize),
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
    start_len: usize,
    text: String,
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
    #[allow(dead_code)] // Plan 2 Task 8
    pending_object_kind: Option<bool>, // Some(around): saw `i`/`a` after operator
    last_find: Option<(char, bool, bool)>, // (ch, till, forward) for ; and ,
    register: RegisterKind,
    #[allow(dead_code)] // Plan 2 Task 11
    last_change: Option<Change>,
    #[allow(dead_code)] // Plan 2 Task 11
    insert_capture: Option<InsertCapture>,
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

    pub fn reset_to_normal(&mut self) {
        self.mode = EditorMode::Normal;
        self.clear_pending();
    }

    /// Interpret one key. In Insert mode everything except `Esc` is
    /// `PassThrough` (the host runs the existing direct textarea path).
    /// In Normal mode, motions move the cursor and the insert-entry keys
    /// switch to Insert mode.
    pub fn handle_key(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        match self.mode {
            EditorMode::Insert => self.handle_insert(key, ta),
            _ => self.handle_normal(key, ta),
        }
    }

    fn handle_insert(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        if key.code == KeyCode::Esc {
            self.mode = EditorMode::Normal;
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
                    self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
                    self.clear_pending();
                    return self.outcome_for(op);
                }
                self.apply_motion(motion, cnt, ta);
                self.clear_pending();
                return VimKeyOutcome::CursorOnly;
            }
            return VimKeyOutcome::NoOp; // non-char (e.g. Esc) cancels
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
        ta.move_cursor(CursorMove::Head);
        let (row, _) = super::cursor_tuple(ta);
        if let Some(line) = ta.lines().get(row) {
            let n = line.chars().take_while(|c| c.is_whitespace()).count();
            for _ in 0..n {
                ta.move_cursor(CursorMove::Forward);
            }
        }
    }

    /// Stub — implemented in Task 9.
    fn match_pair(_ta: &mut TextArea<'static>) { /* Task 9 */ }

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
                for _ in col..target {
                    ta.move_cursor(CursorMove::Forward);
                }
            }
        } else {
            if let Some(pos) = (0..col).rev().find(|&i| chars[i] == ch) {
                let target = if till { pos + 1 } else { pos };
                for _ in target..col {
                    ta.move_cursor(CursorMove::Back);
                }
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
        let start = super::cursor_tuple(ta);
        ta.start_selection();
        self.apply_motion(m, count, ta);
        self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
        // Yank does not move the cursor in vim — restore to the start position.
        if op == Operator::Yank {
            ta.move_cursor(CursorMove::Jump(start.0 as u16, start.1 as u16));
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
                    if r0 > 0 && r1 == last {
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
                        start_len: 0,
                        text: String::new(),
                    });
                }
            }
            Operator::Indent | Operator::Outdent => { /* Task 13 */ }
        }
    }

    fn apply_operator_to_line_end(&mut self, op: Operator, ta: &mut TextArea<'static>) {
        ta.start_selection();
        ta.move_cursor(CursorMove::End);
        self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
    }

    fn apply_operator_on_selection(
        &mut self,
        op: Operator,
        kind: RegisterKind,
        ta: &mut TextArea<'static>,
    ) {
        match op {
            Operator::Yank => {
                ta.copy();
                self.register = kind;
                ta.cancel_selection();
            }
            Operator::Delete => {
                ta.cut();
                self.register = kind;
            }
            Operator::Change => {
                ta.cut();
                self.register = kind;
                self.enter_insert_capture(Command::OperateMotion(op, Motion::Right, 1));
            }
            Operator::Indent | Operator::Outdent => { /* Task 13 */ }
        }
    }

    fn enter_insert_capture(&mut self, command: Command) {
        self.mode = EditorMode::Insert;
        self.insert_capture = Some(InsertCapture {
            command,
            start_len: 0,
            text: String::new(),
        });
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
                    ta.move_cursor(CursorMove::Forward);
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
            self.clear_pending();
            return self.outcome_for(op);
        }

        // Task 5: paste p/P
        if c == 'p' || c == 'P' {
            let after = c == 'p';
            let cnt = self.take_count();
            self.paste(after, cnt, ta);
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

        // Task 3: motion dispatch (count-aware)
        let count = self.pending_count.unwrap_or(1);
        let motion = match c {
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
        };
        if let Some(m) = motion {
            // If an operator is pending, this motion forms a range (Task 4).
            if let Some(op) = self.pending_operator.take() {
                self.apply_operator_motion(op, m, count, ta);
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
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            'X' => {
                let cnt = self.take_count();
                for _ in 0..cnt {
                    ta.delete_char();
                }
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
                self.enter_insert_capture(Command::SubstituteChar(cnt));
                self.clear_pending();
                return VimKeyOutcome::CursorOnly;
            }
            'S' => {
                ta.move_cursor(CursorMove::Head);
                ta.start_selection();
                ta.move_cursor(CursorMove::End);
                ta.cut();
                self.enter_insert_capture(Command::SubstituteLine);
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            'J' => {
                let cnt = self.take_count().max(2) - 1;
                for _ in 0..cnt {
                    Self::join_line(ta);
                }
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            '~' => {
                let cnt = self.take_count();
                for _ in 0..cnt {
                    Self::toggle_case_at_cursor(ta);
                }
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

        // Insert-entry keys (from Plan 1, kept intact)
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
        }
        self.last_change = Some(Change { command: Command::ReplaceChar(c), inserted: None });
        VimKeyOutcome::TextMutated
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
        assert_eq!(t.lines(), &["world"]);
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
}
