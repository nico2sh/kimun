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

/// How a motion forms an operator range (vim `:h exclusive`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpanKind {
    /// Half-open `[start, end)` char range.
    Exclusive,
    /// Includes the char at `end` (`[start, end]`).
    Inclusive,
    /// Whole lines from `start.row` through `end.row`.
    Linewise,
}

/// A text object (`iw`, `a"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObject {
    Word { around: bool },
    Pair { open: char, close: char, around: bool },
    Quote { ch: char, around: bool },
}

/// Where an insert-entry command places the cursor before entering Insert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertEntry {
    Here,      // i
    After,     // a
    LineStart, // I
    LineEnd,   // A
    OpenBelow, // o
    OpenAbove, // O
}

/// The fully-parsed unit of work (adr/0011). `apply` is the only door that
/// mutates the buffer; dot-repeat (and future macros) replay these values
/// through that same door, so first press and replay cannot diverge.
#[derive(Debug, Clone)]
pub enum Command {
    Move(Motion, usize),
    OperateMotion(Operator, Motion, usize), // e.g. 2dw
    OperateLine(Operator, usize),           // dd / cc / yy with count
    OperateObject(Operator, TextObject),    // diw, ci"
    OperateToLineEnd(Operator),             // D / C / Y
    IndentLines { outdent: bool, count: usize }, // >> / <<
    DeleteChar { forward: bool, count: usize }, // x / X
    ReplaceChar(char),                      // r<ch>
    SubstituteChar(usize),                  // s
    SubstituteLine,                         // S
    JoinLines(usize),                       // J
    ToggleCase(usize),                      // ~
    Paste { after: bool, count: usize },    // p / P
    Undo(usize),                            // u
    Redo(usize),                            // Ctrl-r
    EnterInsert(InsertEntry),               // i a I A o O
    EnterVisual { line: bool },             // v / V
    Repeat,                                 // .
}

/// What one Normal-mode key parsed into. Parsing never touches the buffer;
/// `Cmd` is the only variant that leads to mutation — via `apply`.
enum Parsed {
    /// Accumulated pending state; wait for more keys.
    Pending,
    Cmd(Command),
    Host(VimHostAction),
    /// Esc — pending state cleared, host-side selection cleanup applies.
    Cancel,
    /// Unmapped key.
    Nothing,
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

/// One register's value — content and kind live together so they cannot
/// desync (adr/0011: the register is internal vim state, kept separate from
/// the textarea's yank buffer and the OS clipboard).
#[derive(Debug, Clone)]
struct RegisterValue {
    text: String,
    kind: RegisterKind,
}

/// The engine-owned register file. Only the unnamed register exists today;
/// named registers (v2) add a map alongside without touching operator code.
#[derive(Debug, Default)]
struct Registers {
    unnamed: Option<RegisterValue>,
}

impl Registers {
    /// Vim rule: every yank AND every delete/change fills the unnamed
    /// register. Empty text never overwrites it (a no-op delete keeps the
    /// previous content, matching vim).
    fn fill(&mut self, text: String, kind: RegisterKind) {
        if text.is_empty() {
            return;
        }
        self.unnamed = Some(RegisterValue { text, kind });
    }

    fn read(&self) -> Option<&RegisterValue> {
        self.unnamed.as_ref()
    }
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
    /// Count typed BEFORE the operator (`2` in `2d3w`); multiplied with the
    /// motion count at completion (vim: `2d3w` deletes 6 words).
    pending_op_count: Option<usize>,
    pending_operator: Option<Operator>,
    pending_g: bool,              // first key of `gg`
    pending_find: Option<PendingFind>,
    pending_replace: bool,        // awaiting the char after `r`
    pending_object_kind: Option<bool>, // Some(around): saw `i`/`a` after operator
    last_find: Option<(char, bool, bool)>, // (ch, till, forward) for ; and ,
    registers: Registers,
    /// The last mutating command + captured insert delta, for `.` (adr/0011).
    last_change: Option<Change>,
    /// While in Insert via a vim command, the text typed is accumulated here
    /// (resulting delta) so `.` can replay it.
    insert_capture: Option<InsertCapture>,
}

impl Default for VimEngine {
    fn default() -> Self {
        Self {
            mode: EditorMode::Normal,
            pending_count: None,
            pending_op_count: None,
            pending_operator: None,
            pending_g: false,
            pending_find: None,
            pending_replace: false,
            pending_object_kind: None,
            last_find: None,
            registers: Registers::default(),
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

    /// The in-progress command sequence, for the footer hint (e.g. "2d", "f").
    /// Returns `None` when nothing is pending (no display needed).
    pub fn pending_hint(&self) -> Option<String> {
        // Fast path: nothing pending — skip all allocation (common idle-frame case).
        if self.pending_count.is_none()
            && self.pending_op_count.is_none()
            && self.pending_operator.is_none()
            && !self.pending_g
            && !self.pending_replace
            && self.pending_find.is_none()
        {
            return None;
        }
        let mut s = String::new();
        if let Some(n) = self.pending_op_count {
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
        if let Some(n) = self.pending_count {
            s.push_str(&n.to_string());
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
        // A capture from an interrupted Insert (e.g. note switch mid-`cw`)
        // must not survive: execute() skips dot-recording while one is live,
        // which would silently disable `.` for every later change.
        self.insert_capture = None;
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
            && self.pending_op_count.is_none()
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
        // f/F/t/T target pending: resolve as a selection-extending motion
        // (`vf,` extends the selection through the ',').
        if self.pending_find.is_some() {
            if let KeyCode::Char(ch) = key.code {
                let pf = self.pending_find.take().expect("checked is_some");
                self.last_find = Some((ch, pf.till, pf.forward));
                let cnt = self.take_count();
                let motion = Motion::FindChar { ch, till: pf.till, forward: pf.forward };
                self.apply_motion(motion, cnt, ta);
                return VimKeyOutcome::CursorOnly;
            }
            self.pending_find = None;
            self.clear_pending();
            return VimKeyOutcome::NoOp;
        }

        // Object char pending (after `i`/`a` in charwise Visual): re-aim the
        // selection at the text object under the cursor (`vi(`, `va"`).
        if let Some(around) = self.pending_object_kind.take() {
            if let KeyCode::Char(ch) = key.code {
                if let Some(obj) = Self::object_for_char(ch, around) {
                    Self::select_object_visual(obj, ta);
                    self.clear_pending();
                    return VimKeyOutcome::CursorOnly;
                }
            }
            self.clear_pending();
            return VimKeyOutcome::NoOp;
        }

        // Esc: cancel selection and return to Normal.
        if key.code == KeyCode::Esc {
            ta.cancel_selection();
            self.mode = EditorMode::Normal;
            self.clear_pending();
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
                self.apply_operator_linewise(op, count, None, ta);
            } else {
                // Charwise Visual: vim selection is inclusive of the char under
                // the cursor — re-select through select_range's inclusive end.
                if let Some((start, end)) = ta.selection_range() {
                    ta.cancel_selection();
                    Self::select_range(ta, start, end, true);
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
        // The register is engine-owned, so the cut below cannot clobber it.
        if c == 'p' || c == 'P' {
            let Some(reg) = self.registers.read().cloned() else {
                ta.cancel_selection();
                self.mode = EditorMode::Normal;
                return VimKeyOutcome::CursorOnly;
            };
            let text = reg.text;
            if self.mode == EditorMode::VisualLine {
                // VisualLine: delete the selected whole lines, then paste the
                // saved content. The delete fills the register with the deleted
                // lines — vim swap behavior — while `text` keeps the original.
                let (start_row, end_row) = if let Some(((sr, _), (er, _))) = ta.selection_range() {
                    (sr, er)
                } else {
                    let (r, _) = super::cursor_tuple(ta);
                    (r, r)
                };
                ta.cancel_selection();
                ta.move_cursor(CursorMove::Jump(start_row as u16, 0));
                let count = end_row - start_row + 1;
                self.apply_operator_linewise(Operator::Delete, count, None, ta);
                let body = text.strip_suffix('\n').unwrap_or(&text);
                ta.move_cursor(CursorMove::Head);
                ta.insert_str(body);
                ta.insert_newline();
                ta.move_cursor(CursorMove::Up);
            } else {
                // Charwise: make an inclusive selection, delete it, and fill
                // the register with the deleted text (vim swap: the replaced
                // selection enters the register), then insert the saved `text`.
                if let Some((start, end)) = ta.selection_range() {
                    ta.cancel_selection();
                    Self::select_range(ta, start, end, true);
                }
                ta.cut(); // cursor lands at the deletion gap
                self.fill_from_textarea(ta, RegisterKind::Charwise);
                // Record where the paste starts so we can leave the cursor there
                // (vim visual-p leaves cursor at the start of the pasted text).
                let paste_start = super::cursor_tuple(ta);
                ta.insert_str(&text); // insert the SAVED content, not the yank buffer
                ta.move_cursor(CursorMove::Jump(paste_start.0 as u16, paste_start.1 as u16));
            }
            self.mode = EditorMode::Normal;
            self.clear_pending();
            return VimKeyOutcome::TextMutated;
        }

        // 'o': swap cursor and anchor (vim: move to the other end of the
        // selection so it can be extended from there).
        if c == 'o' {
            if let Some((start, end)) = ta.selection_range() {
                let cur = super::cursor_tuple(ta);
                let other = if cur == end { start } else { end };
                ta.cancel_selection();
                ta.move_cursor(CursorMove::Jump(cur.0 as u16, cur.1 as u16));
                ta.start_selection();
                ta.move_cursor(CursorMove::Jump(other.0 as u16, other.1 as u16));
            }
            return VimKeyOutcome::CursorOnly;
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

        // gg: extend the selection to the file start.
        if c == 'g' {
            if self.pending_g {
                self.pending_g = false;
                self.apply_motion(Motion::FileStart, 1, ta);
                self.clear_pending();
                return VimKeyOutcome::CursorOnly;
            }
            self.pending_g = true;
            return VimKeyOutcome::NoOp;
        }

        // f/F/t/T: pend a selection-extending find.
        if let Some((till, forward)) = match c {
            'f' => Some((false, true)),
            'F' => Some((false, false)),
            't' => Some((true, true)),
            'T' => Some((true, false)),
            _ => None,
        } {
            self.pending_find = Some(PendingFind { operator: None, till, forward });
            return VimKeyOutcome::NoOp;
        }

        // ; and , repeat the last find, extending the selection.
        if c == ';' || c == ',' {
            if let Some((ch, till, fwd)) = self.last_find {
                let forward = if c == ';' { fwd } else { !fwd };
                let cnt = self.take_count();
                self.apply_motion(Motion::FindChar { ch, till, forward }, cnt, ta);
            }
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }

        // i/a: text-object selection (charwise Visual only — `vi(`, `va"`).
        if (c == 'i' || c == 'a') && self.mode == EditorMode::Visual {
            self.pending_object_kind = Some(c == 'a');
            return VimKeyOutcome::NoOp;
        }

        // Motions extend the selection.
        if let Some(m) = Self::motion_for_char(c) {
            let count = self.take_count();
            self.apply_motion(m, count, ta);
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }

        self.clear_pending();
        VimKeyOutcome::NoOp
    }

    /// Re-aim the charwise visual selection at the text object under the
    /// cursor. The selection end is left ON the object's last char (visual
    /// selections are inclusive; the operator's inclusive `+1` restores the
    /// half-open range `object_range` computed).
    fn select_object_visual(obj: TextObject, ta: &mut TextArea<'static>) {
        let (row, col) = super::cursor_tuple(ta);
        let Some(line) = ta.lines().get(row).cloned() else { return };
        let chars: Vec<char> = line.chars().collect();
        let Some((start, end)) = Self::object_range(&chars, col, obj) else { return };
        if start >= end {
            // Empty object (vi( on "()"): collapsing to one char would make
            // the operator's inclusive +1 grab the closing delimiter. No-op.
            return;
        }
        ta.cancel_selection();
        let end_incl = end.saturating_sub(1).max(start);
        ta.move_cursor(CursorMove::Jump(row as u16, start as u16));
        ta.start_selection();
        ta.move_cursor(CursorMove::Jump(row as u16, end_incl as u16));
    }

    fn handle_insert(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        if key.code == KeyCode::Esc {
            self.mode = EditorMode::Normal;
            // Compute the inserted text once at Esc, slicing from the start cursor
            // recorded when Insert began to the current cursor (multi-line aware).
            if let Some(cap) = self.insert_capture.take() {
                let end = super::cursor_tuple(ta);
                let inserted = Self::text_between(ta.lines(), cap.start, end);
                // An aborted plain insert (i/a/I/A then Esc, nothing typed)
                // is not a change in vim — don't clobber the dot register.
                // o/O still record (opening the line IS the change), as do
                // Change-family commands (cw/s/cc — the cut happened).
                let aborted_plain = inserted.is_empty()
                    && matches!(
                        cap.command,
                        Command::EnterInsert(
                            InsertEntry::Here
                                | InsertEntry::After
                                | InsertEntry::LineStart
                                | InsertEntry::LineEnd
                        )
                    );
                if !aborted_plain {
                    self.last_change = Some(Change {
                        command: cap.command,
                        inserted: Some(inserted),
                    });
                }
            }
            if super::cursor_tuple(ta).1 > 0 {
                ta.move_cursor(CursorMove::Back);
            }
            return VimKeyOutcome::CursorOnly;
        }
        VimKeyOutcome::PassThrough
    }

    fn handle_normal(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        match self.parse_normal(key) {
            Parsed::Pending | Parsed::Nothing => VimKeyOutcome::NoOp,
            Parsed::Cancel => {
                // Esc also cancels any stray textarea selection left live in
                // Normal mode (e.g. the auto-surround PassThrough path).
                ta.cancel_selection();
                VimKeyOutcome::CursorOnly
            }
            Parsed::Host(action) => {
                self.clear_pending();
                VimKeyOutcome::Host(action)
            }
            Parsed::Cmd(cmd) => self.execute(cmd, ta),
        }
    }

    /// Parse one Normal-mode key into a `Parsed` value. Pure pending-state
    /// accumulation — never touches the buffer (adr/0011).
    fn parse_normal(&mut self, key: &KeyEvent) -> Parsed {
        // r<ch>: consume the replacement char.
        if self.pending_replace {
            self.pending_replace = false;
            if let KeyCode::Char(c) = key.code {
                return Parsed::Cmd(Command::ReplaceChar(c));
            }
            return Parsed::Nothing; // Esc etc cancels
        }

        // f/F/t/T: consume the find target char.
        if self.pending_find.is_some() {
            if let KeyCode::Char(ch) = key.code {
                let pf = self.pending_find.take().expect("checked is_some");
                self.last_find = Some((ch, pf.till, pf.forward));
                let motion = Motion::FindChar { ch, till: pf.till, forward: pf.forward };
                return match pf.operator {
                    Some(op) => {
                        Parsed::Cmd(Command::OperateMotion(op, motion, self.take_total_count()))
                    }
                    None => Parsed::Cmd(Command::Move(motion, self.take_count())),
                };
            }
            self.pending_find = None;
            self.clear_pending(); // non-char (e.g. Esc) cancels — drop counts too
            return Parsed::Nothing;
        }

        // Esc cancels any pending sequence (operator, counts, g, i/a object).
        if key.code == KeyCode::Esc {
            self.clear_pending();
            return Parsed::Cancel;
        }

        // Ctrl-r → redo (before the plain filter so it isn't stripped).
        if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Parsed::Cmd(Command::Redo(self.take_total_count()));
        }

        let plain = key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT;
        match key.code {
            KeyCode::Char(c) if plain => self.parse_normal_char(c),
            KeyCode::Left => Parsed::Cmd(Command::Move(Motion::Left, 1)),
            KeyCode::Right => Parsed::Cmd(Command::Move(Motion::Right, 1)),
            KeyCode::Up => Parsed::Cmd(Command::Move(Motion::Up, 1)),
            KeyCode::Down => Parsed::Cmd(Command::Move(Motion::Down, 1)),
            _ => Parsed::Nothing,
        }
    }

    /// Run a freshly-parsed command through the one mutation door, recording
    /// it for `.` when it is a repeatable change. Change-family commands
    /// defer recording to Esc (the insert capture owns it).
    fn execute(&mut self, cmd: Command, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        let outcome = self.apply(&cmd, None, ta);
        if outcome != VimKeyOutcome::NoOp
            && Self::repeatable(&cmd)
            && self.insert_capture.is_none()
        {
            self.record(cmd);
        }
        self.clear_pending();
        outcome
    }

    /// Whether `.` repeats this command. Motions, undo/redo, yanks, mode
    /// changes and `.` itself are not changes (vim semantics).
    fn repeatable(cmd: &Command) -> bool {
        match cmd {
            Command::Move(..)
            | Command::Undo(_)
            | Command::Redo(_)
            | Command::EnterVisual { .. }
            | Command::Repeat => false,
            Command::OperateMotion(op, ..)
            | Command::OperateLine(op, _)
            | Command::OperateObject(op, _)
            | Command::OperateToLineEnd(op) => *op != Operator::Yank,
            _ => true,
        }
    }

    /// The only door that mutates the buffer for Normal-mode commands.
    /// `inserted` is the captured Insert-mode delta when replaying a
    /// Change-family command (dot-repeat); `None` on a first press, which
    /// enters Insert mode and starts capturing instead.
    fn apply(
        &mut self,
        cmd: &Command,
        inserted: Option<&str>,
        ta: &mut TextArea<'static>,
    ) -> VimKeyOutcome {
        match *cmd {
            Command::Move(m, n) => {
                self.apply_motion(m, n, ta);
                VimKeyOutcome::CursorOnly
            }
            Command::OperateMotion(op, m, n) => {
                if self.apply_operator_motion(op, m, n, inserted, ta) {
                    self.outcome_for(op)
                } else {
                    VimKeyOutcome::NoOp
                }
            }
            Command::OperateLine(op, n) => {
                self.apply_operator_linewise(op, n, inserted, ta);
                self.outcome_for(op)
            }
            Command::OperateObject(op, obj) => {
                if self.apply_operator_object(op, obj, inserted, ta) {
                    self.outcome_for(op)
                } else {
                    VimKeyOutcome::NoOp
                }
            }
            Command::OperateToLineEnd(op) => {
                self.apply_operator_to_line_end(op, inserted, ta);
                self.outcome_for(op)
            }
            Command::IndentLines { outdent, count } => {
                self.indent_lines(outdent, count, ta);
                VimKeyOutcome::TextMutated
            }
            Command::DeleteChar { forward, count } => {
                if self.delete_chars(forward, count, ta) {
                    VimKeyOutcome::TextMutated
                } else {
                    VimKeyOutcome::NoOp
                }
            }
            Command::ReplaceChar(c) => self.replace_char(c, ta),
            Command::SubstituteChar(n) => {
                // vim `s` enters Insert even on an empty line; the delete's
                // success only matters for the outcome's mutation signal.
                let deleted = self.delete_chars(true, n, ta);
                self.finish_insert_entry(cmd, inserted, ta);
                if deleted || inserted.is_some() {
                    VimKeyOutcome::TextMutated
                } else {
                    VimKeyOutcome::CursorOnly
                }
            }
            Command::SubstituteLine => {
                // Linewise register fill (vim: S puts the whole line in the
                // unnamed register, linewise), computed before the cut.
                let (row, _) = super::cursor_tuple(ta);
                if let Some(text) = ta.lines().get(row).map(|l| format!("{l}\n")) {
                    self.registers.fill(text, RegisterKind::Linewise);
                }
                ta.move_cursor(CursorMove::Head);
                ta.start_selection();
                ta.move_cursor(CursorMove::End);
                ta.cut();
                self.finish_insert_entry(cmd, inserted, ta);
                VimKeyOutcome::TextMutated
            }
            Command::JoinLines(n) => {
                for _ in 0..n.max(1) {
                    Self::join_line(ta);
                }
                VimKeyOutcome::TextMutated
            }
            Command::ToggleCase(n) => {
                for _ in 0..n {
                    Self::toggle_case_at_cursor(ta);
                }
                VimKeyOutcome::TextMutated
            }
            Command::Paste { after, count } => {
                if self.paste(after, count, ta) {
                    VimKeyOutcome::TextMutated
                } else {
                    VimKeyOutcome::NoOp
                }
            }
            Command::Undo(n) => {
                for _ in 0..n {
                    ta.undo();
                }
                VimKeyOutcome::TextMutated
            }
            Command::Redo(n) => {
                for _ in 0..n {
                    ta.redo();
                }
                VimKeyOutcome::TextMutated
            }
            Command::EnterInsert(entry) => self.apply_enter_insert(entry, cmd, inserted, ta),
            Command::EnterVisual { line } => {
                if line {
                    ta.move_cursor(CursorMove::Head);
                    ta.start_selection();
                    ta.move_cursor(CursorMove::End);
                    self.mode = EditorMode::VisualLine;
                } else {
                    ta.start_selection();
                    self.mode = EditorMode::Visual;
                }
                VimKeyOutcome::CursorOnly
            }
            Command::Repeat => match self.last_change.clone() {
                Some(change) => self.apply(&change.command, change.inserted.as_deref(), ta),
                None => VimKeyOutcome::NoOp,
            },
        }
    }

    /// Shared tail of every command that ends in Insert mode: on a first
    /// press, enter Insert and start capturing the typed delta; on replay,
    /// insert the captured text directly and stay in Normal.
    fn finish_insert_entry(
        &mut self,
        cmd: &Command,
        inserted: Option<&str>,
        ta: &mut TextArea<'static>,
    ) {
        match inserted {
            Some(text) => {
                ta.insert_str(text);
                self.mode = EditorMode::Normal;
            }
            None => self.enter_insert_capture(cmd.clone(), ta),
        }
    }

    fn apply_enter_insert(
        &mut self,
        entry: InsertEntry,
        cmd: &Command,
        inserted: Option<&str>,
        ta: &mut TextArea<'static>,
    ) -> VimKeyOutcome {
        let opened_line = match entry {
            InsertEntry::Here => false,
            InsertEntry::After => {
                ta.move_cursor(CursorMove::Forward);
                false
            }
            InsertEntry::LineStart => {
                ta.move_cursor(CursorMove::Head);
                false
            }
            InsertEntry::LineEnd => {
                ta.move_cursor(CursorMove::End);
                false
            }
            InsertEntry::OpenBelow => {
                ta.move_cursor(CursorMove::End);
                ta.insert_newline();
                true
            }
            InsertEntry::OpenAbove => {
                ta.move_cursor(CursorMove::Head);
                ta.insert_newline();
                ta.move_cursor(CursorMove::Up);
                true
            }
        };
        match inserted {
            Some(text) => {
                ta.insert_str(text);
                self.mode = EditorMode::Normal;
                if super::cursor_tuple(ta).1 > 0 {
                    ta.move_cursor(CursorMove::Back);
                }
                VimKeyOutcome::TextMutated
            }
            None => {
                self.enter_insert_capture(cmd.clone(), ta);
                if opened_line {
                    VimKeyOutcome::TextMutated
                } else {
                    VimKeyOutcome::CursorOnly
                }
            }
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

    /// Operator-scoped count × motion-scoped count (vim: `2d3w` = 6 words).
    fn take_total_count(&mut self) -> usize {
        let op_n = self.pending_op_count.take().unwrap_or(1);
        op_n * self.pending_count.take().unwrap_or(1)
    }

    fn clear_pending(&mut self) {
        self.pending_count = None;
        self.pending_op_count = None;
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

    /// Where `motion` (× count) would land, as a position value — no net
    /// cursor mutation (the cursor is restored before returning).
    fn resolve_motion(
        &self,
        motion: Motion,
        count: usize,
        ta: &mut TextArea<'static>,
    ) -> (usize, usize) {
        let saved = super::cursor_tuple(ta);
        self.apply_motion(motion, count, ta);
        let target = super::cursor_tuple(ta);
        ta.move_cursor(CursorMove::Jump(saved.0 as u16, saved.1 as u16));
        target
    }

    /// Vim's motion classification: how a motion forms an operator range.
    /// (`:h exclusive` — every vim motion is exclusive, inclusive, or
    /// linewise when consumed by an operator.)
    fn kind_of(motion: Motion) -> SpanKind {
        match motion {
            Motion::Up | Motion::Down | Motion::FileStart | Motion::FileEnd => SpanKind::Linewise,
            Motion::WordEnd | Motion::MatchingPair => SpanKind::Inclusive,
            // d$ deletes through the last char (vim: $ is inclusive).
            Motion::LineEnd => SpanKind::Inclusive,
            // f/t are inclusive; F/T (backward) are exclusive.
            Motion::FindChar { forward: true, .. } => SpanKind::Inclusive,
            _ => SpanKind::Exclusive,
        }
    }

    /// Select `[start, end]` (inclusive) or `[start, end)` on the textarea.
    /// The single home of the vim-inclusive → ratatui-half-open `+1`
    /// conversion, clamped to the end line's length.
    fn select_range(
        ta: &mut TextArea<'static>,
        start: (usize, usize),
        end: (usize, usize),
        inclusive: bool,
    ) {
        let (er, ec) = end;
        let end_col = if inclusive {
            let len = ta.lines().get(er).map(|l| l.chars().count()).unwrap_or(ec);
            (ec + 1).min(len)
        } else {
            ec
        };
        ta.move_cursor(CursorMove::Jump(start.0 as u16, start.1 as u16));
        ta.start_selection();
        ta.move_cursor(CursorMove::Jump(er as u16, end_col as u16));
    }

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
    /// The range's shape is the motion's `SpanKind`: linewise motions (j/k,
    /// gg/G) operate on whole lines, inclusive motions (e, f/t, %, $) take
    /// the char they land on, exclusive motions stop short of it.
    /// Returns `false` when the motion failed and the whole operation was a
    /// vim no-op (nothing deleted, no Insert entry, register untouched).
    fn apply_operator_motion(
        &mut self,
        op: Operator,
        m: Motion,
        count: usize,
        inserted: Option<&str>,
        ta: &mut TextArea<'static>,
    ) -> bool {
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
        let origin = super::cursor_tuple(ta);
        let target = self.resolve_motion(effective_motion, count, ta);
        match Self::kind_of(effective_motion) {
            SpanKind::Linewise => {
                // j/k must actually traverse `count` rows; at a buffer edge
                // the motion fails and vim no-ops the whole operation (dj on
                // the last line deletes nothing). gg/G always resolve —
                // operating on the current line is valid for them.
                if matches!(effective_motion, Motion::Up | Motion::Down)
                    && origin.0.abs_diff(target.0) < count
                {
                    return false;
                }
                let top = origin.0.min(target.0);
                let lines = origin.0.abs_diff(target.0) + 1;
                ta.move_cursor(CursorMove::Jump(top as u16, 0));
                self.apply_operator_linewise(op, lines, inserted, ta);
                true
            }
            kind => {
                if target == origin
                    && (kind == SpanKind::Exclusive
                        || matches!(
                            effective_motion,
                            Motion::FindChar { .. } | Motion::MatchingPair
                        ))
                {
                    // Failed find/pair-match or zero-width exclusive range:
                    // vim no-op — nothing deleted, no Insert, register kept.
                    return false;
                }
                let (start, end) = if target < origin {
                    (target, origin)
                } else {
                    (origin, target)
                };
                Self::select_range(ta, start, end, kind == SpanKind::Inclusive);
                // For Change, capture under the actual command (original
                // motion, not the cw=ce substitute) so `.` replays it right.
                if op == Operator::Change {
                    ta.cut();
                    self.fill_from_textarea(ta, RegisterKind::Charwise);
                    self.finish_insert_entry(&Command::OperateMotion(op, m, count), inserted, ta);
                } else {
                    self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
                }
                true
            }
        }
    }

    fn apply_operator_linewise(
        &mut self,
        op: Operator,
        count: usize,
        inserted: Option<&str>,
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
                self.registers.fill(register_text, RegisterKind::Linewise);
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
                // The cut selection may include a leading newline on the
                // last-line path; fill the register with the proper linewise
                // content computed above instead.
                self.registers.fill(register_text, RegisterKind::Linewise);
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
                    self.finish_insert_entry(&Command::OperateLine(op, count), inserted, ta);
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

    fn apply_operator_to_line_end(
        &mut self,
        op: Operator,
        inserted: Option<&str>,
        ta: &mut TextArea<'static>,
    ) {
        ta.start_selection();
        ta.move_cursor(CursorMove::End);
        if op == Operator::Change {
            ta.cut();
            self.fill_from_textarea(ta, RegisterKind::Charwise);
            self.finish_insert_entry(&Command::OperateToLineEnd(op), inserted, ta);
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

    /// Capture the text the textarea just cut/copied (its yank buffer) into
    /// the engine's unnamed register. The textarea yank buffer is only a
    /// transport here — the engine never reads it back at paste time.
    fn fill_from_textarea(&mut self, ta: &TextArea<'static>, kind: RegisterKind) {
        self.registers.fill(ta.yank_text(), kind);
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
                self.fill_from_textarea(ta, kind);
                ta.cancel_selection();
                if let Some((r, c)) = start {
                    ta.move_cursor(CursorMove::Jump(r as u16, c as u16));
                }
            }
            Operator::Delete => {
                ta.cut();
                self.fill_from_textarea(ta, kind);
            }
            Operator::Change => {
                ta.cut();
                self.fill_from_textarea(ta, kind);
                // Visual `c` only reaches here (first press by definition);
                // the capture command is a stand-in for the visual range.
                self.enter_insert_capture(Command::OperateMotion(op, Motion::Right, 1), ta);
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

    // ── Plan 2 Task 5: paste p/P ─────────────────────────────────────────────

    /// Returns `false` when the register is empty (nothing pasted).
    fn paste(&mut self, after: bool, count: usize, ta: &mut TextArea<'static>) -> bool {
        let Some(reg) = self.registers.read().cloned() else {
            return false;
        };
        let text = reg.text;
        match reg.kind {
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
        true
    }

    // ── parse_normal_char: pure Normal-mode key parser ───────────────────────

    /// Parse one plain Normal-mode char. Pure pending-state accumulation —
    /// commands come out as values; nothing here touches the buffer.
    fn parse_normal_char(&mut self, c: char) -> Parsed {
        // Count digits accumulate first.
        if self.accumulate_count(c) {
            return Parsed::Pending;
        }

        // gg prefix.
        if c == 'g' {
            if self.pending_g {
                self.pending_g = false;
                let cnt = self.take_total_count();
                return match self.pending_operator.take() {
                    Some(op) => Parsed::Cmd(Command::OperateMotion(op, Motion::FileStart, cnt)),
                    None => Parsed::Cmd(Command::Move(Motion::FileStart, 1)),
                };
            }
            self.pending_g = true;
            return Parsed::Pending;
        }

        // Operator entry (d/c/y set pending; doubled → linewise).
        let op_for_char = match c {
            'd' => Some(Operator::Delete),
            'c' => Some(Operator::Change),
            'y' => Some(Operator::Yank),
            _ => None,
        };
        if let Some(op) = op_for_char {
            if self.pending_operator == Some(op) {
                return Parsed::Cmd(Command::OperateLine(op, self.take_total_count()));
            }
            self.pending_operator = Some(op);
            // A count typed so far scopes to the operator; the motion gets
            // its own accumulator (vim multiplies the two).
            self.pending_op_count = self.pending_count.take();
            return Parsed::Pending;
        }
        // D / C / Y → operator to line end.
        if let Some(op) = match c {
            'D' => Some(Operator::Delete),
            'C' => Some(Operator::Change),
            'Y' => Some(Operator::Yank),
            _ => None,
        } {
            return Parsed::Cmd(Command::OperateToLineEnd(op));
        }

        // >>/<< indent/outdent: first key sets the pending operator; the
        // doubled key completes linewise. A motion after the first key (e.g.
        // `>j`) instead forms a range via the motion dispatch below.
        if c == '>' || c == '<' {
            let outdent = c == '<';
            if (outdent && self.pending_operator == Some(Operator::Outdent))
                || (!outdent && self.pending_operator == Some(Operator::Indent))
            {
                self.pending_operator = None;
                return Parsed::Cmd(Command::IndentLines {
                    outdent,
                    count: self.take_total_count(),
                });
            }
            self.pending_operator = Some(if outdent { Operator::Outdent } else { Operator::Indent });
            self.pending_op_count = self.pending_count.take();
            return Parsed::Pending;
        }

        // Paste.
        if c == 'p' || c == 'P' {
            return Parsed::Cmd(Command::Paste {
                after: c == 'p',
                count: self.take_count(),
            });
        }

        // f/F/t/T — pend the find target (captures the operator so `df,` works).
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
            return Parsed::Pending;
        }

        // ; and , — repeat last find (same / opposite direction); with a
        // pending operator (`d;`) forms a range like any motion.
        if c == ';' || c == ',' {
            if let Some((ch, till, fwd)) = self.last_find {
                let forward = if c == ';' { fwd } else { !fwd };
                let motion = Motion::FindChar { ch, till, forward };
                return match self.pending_operator.take() {
                    Some(op) => {
                        Parsed::Cmd(Command::OperateMotion(op, motion, self.take_total_count()))
                    }
                    None => Parsed::Cmd(Command::Move(motion, self.take_count())),
                };
            }
            self.clear_pending();
            return Parsed::Nothing;
        }

        // Text objects — intercepted BEFORE the motion dispatch so the object
        // char (`w` in `diw`) is not consumed as a motion, and before
        // insert-entry so `di`/`ci`/`yi` never enter Insert.
        if self.pending_operator.is_some() {
            if c == 'i' || c == 'a' {
                self.pending_object_kind = Some(c == 'a');
                return Parsed::Pending;
            }
            if let Some(around) = self.pending_object_kind.take() {
                if let Some(obj) = Self::object_for_char(c, around) {
                    let op = self.pending_operator.take().expect("checked is_some");
                    return Parsed::Cmd(Command::OperateObject(op, obj));
                }
                self.clear_pending();
                return Parsed::Nothing;
            }
        }

        // Motion dispatch (count-aware; with a pending operator, forms a range).
        if let Some(m) = Self::motion_for_char(c) {
            return match self.pending_operator.take() {
                Some(op) => Parsed::Cmd(Command::OperateMotion(op, m, self.take_total_count())),
                None => Parsed::Cmd(Command::Move(m, self.take_count())),
            };
        }

        // Single-key edits, dot, visual entry, host actions, insert entry.
        // NOTE: i/a only reach here when NO operator is pending — operator +
        // i/a is the text-object path above.
        let cmd = match c {
            'x' => Command::DeleteChar { forward: true, count: self.take_count() },
            'X' => Command::DeleteChar { forward: false, count: self.take_count() },
            'r' => {
                self.pending_replace = true;
                return Parsed::Pending;
            }
            's' => Command::SubstituteChar(self.take_count()),
            'S' => Command::SubstituteLine,
            'J' => Command::JoinLines(self.take_count().max(2) - 1),
            '~' => Command::ToggleCase(self.take_count()),
            'u' => Command::Undo(self.take_count()),
            '.' => Command::Repeat,
            'v' => Command::EnterVisual { line: false },
            'V' => Command::EnterVisual { line: true },
            'i' => Command::EnterInsert(InsertEntry::Here),
            'a' => Command::EnterInsert(InsertEntry::After),
            'I' => Command::EnterInsert(InsertEntry::LineStart),
            'A' => Command::EnterInsert(InsertEntry::LineEnd),
            'o' => Command::EnterInsert(InsertEntry::OpenBelow),
            'O' => Command::EnterInsert(InsertEntry::OpenAbove),
            // Host actions — `:` `/` `?` `n` `N` (adr/0012). `?` backward-first
            // is deferred; `/` and `?` both open the find bar for v1.
            ':' => return Parsed::Host(VimHostAction::OpenPalette),
            '/' => return Parsed::Host(VimHostAction::OpenSearch { forward: true }),
            '?' => return Parsed::Host(VimHostAction::OpenSearch { forward: false }),
            'n' => return Parsed::Host(VimHostAction::SearchNext),
            'N' => return Parsed::Host(VimHostAction::SearchPrev),
            _ => {
                self.clear_pending();
                return Parsed::Nothing;
            }
        };
        Parsed::Cmd(cmd)
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
    /// Returns `false` when no object exists at the cursor (vim no-op).
    fn apply_operator_object(
        &mut self,
        op: Operator,
        obj: TextObject,
        inserted: Option<&str>,
        ta: &mut TextArea<'static>,
    ) -> bool {
        let (row, col) = super::cursor_tuple(ta);
        let Some(line) = ta.lines().get(row).cloned() else {
            return false;
        };
        let chars: Vec<char> = line.chars().collect();
        let Some((start, end)) = Self::object_range(&chars, col, obj) else {
            return false;
        };
        // Select [start, end) on this row via Jump, then apply the operator.
        ta.move_cursor(CursorMove::Jump(row as u16, start as u16));
        ta.start_selection();
        ta.move_cursor(CursorMove::Jump(row as u16, end as u16));
        if op == Operator::Change {
            ta.cut();
            self.fill_from_textarea(ta, RegisterKind::Charwise);
            self.finish_insert_entry(&Command::OperateObject(op, obj), inserted, ta);
        } else {
            self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
        }
        true
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

    // ── Plan 2 Task 6: single-key edit helpers ───────────────────────────────

    /// Delete `count` chars at the cursor (`forward`: under-and-after, vim
    /// `x`; otherwise before, vim `X`), clamped to the current line — vim's
    /// x/X never join lines — filling the unnamed register with the deleted
    /// text (vim rule: every delete fills the register; `xp` swaps chars).
    /// Returns `false` when nothing was deleted (empty line, X at col 0).
    fn delete_chars(&mut self, forward: bool, count: usize, ta: &mut TextArea<'static>) -> bool {
        let (row, col) = super::cursor_tuple(ta);
        let Some(line) = ta.lines().get(row).cloned() else {
            return false;
        };
        let line_len = line.chars().count();
        let (n, start) = if forward {
            (count.min(line_len.saturating_sub(col)), col)
        } else {
            let n = count.min(col);
            (n, col - n)
        };
        let deleted: String = line.chars().skip(start).take(n).collect();
        self.registers.fill(deleted, RegisterKind::Charwise);
        for _ in 0..n {
            if forward {
                ta.delete_next_char();
            } else {
                ta.delete_char();
            }
        }
        n > 0
    }

    /// Replace the char under the cursor with `c`, stay in Normal mode.
    fn replace_char(&mut self, c: char, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        if ta.delete_next_char() {
            ta.insert_char(c);
            ta.move_cursor(CursorMove::Back);
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
        let reg = e.registers.read().expect("dd must fill the register");
        assert_eq!(reg.kind, RegisterKind::Linewise);
        assert_eq!(reg.text, "one\n");
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
    fn visual_p_yanks_replaced_selection() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar"]);
        // yank "foo"
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('e'), &mut t);
        e.handle_key(&key('y'), &mut t); // reg = "foo", cursor col 0
        // select "bar" and paste over it
        for _ in 0..4 { e.handle_key(&key('l'), &mut t); } // col 4 'b'
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('e'), &mut t);
        e.handle_key(&key('p'), &mut t); // "bar" replaced by "foo"; "bar" now yanked
        assert_eq!(t.lines(), &["foo foo"]);
        // now paste the replaced "bar" at end of line to prove it's in the register
        e.handle_key(&key('$'), &mut t);   // last char ('o', col 6)
        e.handle_key(&key('p'), &mut t);   // append "bar" after it
        assert_eq!(t.lines(), &["foo foobar"]);
    }

    // ── Review fixes: failed-op no-ops, dot-register protection ─────────────

    #[test]
    fn reset_to_normal_clears_insert_capture() {
        // regression: stale capture from interrupted cw silently disabled
        // dot-recording for every later change
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar"]);
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('w'), &mut t); // Insert, capture live
        e.reset_to_normal(); // note switch mid-insert
        e.handle_key(&key('x'), &mut t); // must record (deletes ' ')
        e.handle_key(&key('.'), &mut t); // must repeat x (deletes 'b')
        assert_eq!(t.lines(), &["ar"]); // cw left " bar"; x then . removed 2 chars
    }

    #[test]
    fn dj_on_last_line_is_noop() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["only line"]);
        e.handle_key(&key('y'), &mut t);
        e.handle_key(&key('y'), &mut t); // register = line
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('j'), &mut t); // motion fails → whole op no-op
        assert_eq!(t.lines(), &["only line"]);
        assert_eq!(e.registers.read().unwrap().text, "only line\n"); // register kept
    }

    #[test]
    fn dk_on_first_line_is_noop() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two"]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('k'), &mut t);
        assert_eq!(t.lines(), &["one", "two"]);
    }

    #[test]
    fn failed_find_op_does_not_clobber_dot() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abcdef"]);
        e.handle_key(&key('x'), &mut t); // real change: delete 'a'
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('z'), &mut t); // failed find — must not record
        e.handle_key(&key('.'), &mut t); // repeats x, not the failed dfz
        assert_eq!(t.lines(), &["cdef"]);
    }

    #[test]
    fn noop_x_does_not_clobber_dot() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one two three", ""]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('w'), &mut t); // delete "one "
        e.handle_key(&key('j'), &mut t); // empty line
        let out = e.handle_key(&key('x'), &mut t); // no-op
        assert_eq!(out, VimKeyOutcome::NoOp); // host must not bump content
        e.handle_key(&key('k'), &mut t);
        e.handle_key(&key('.'), &mut t); // repeats dw, not the no-op x
        assert_eq!(t.lines(), &["three", ""]);
    }

    #[test]
    fn d_percent_without_pair_is_noop() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc"]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('%'), &mut t); // no bracket under cursor
        assert_eq!(t.lines(), &["abc"]);
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('%'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Normal); // failed c% must not enter Insert
    }

    #[test]
    fn visual_inner_empty_pair_is_noop() {
        // regression: vi( on "()" widened onto the ')' and deleted it
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo()bar"]);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('('), &mut t); // cursor on '('
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('i'), &mut t);
        e.handle_key(&key('('), &mut t); // empty object: selection unchanged
        e.handle_key(&esc(), &mut t);
        assert_eq!(t.lines(), &["foo()bar"]);
    }

    #[test]
    fn aborted_insert_keeps_dot_register() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc"]);
        e.handle_key(&key('x'), &mut t); // real change
        e.handle_key(&key('i'), &mut t); // changed mind
        e.handle_key(&esc(), &mut t); // nothing typed — not a change
        e.handle_key(&key('.'), &mut t); // must repeat x
        assert_eq!(t.lines(), &["c"]);
    }

    #[test]
    fn o_then_esc_is_still_dot_repeatable() {
        // vim: o<Esc> IS a change (the opened line); `.` opens another
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["x"]);
        e.handle_key(&key('o'), &mut t);
        e.handle_key(&esc(), &mut t);
        e.handle_key(&key('.'), &mut t);
        assert_eq!(t.lines().len(), 3);
    }

    // ── Visual mode: shared motion/object machinery ──────────────────────────

    #[test]
    fn visual_inner_object_then_delete() {
        // vi( selects inside the parens; d deletes it
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo(bar)baz"]);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('a'), &mut t); // cursor on 'a' of "bar" (col 5)
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('i'), &mut t);
        e.handle_key(&key('('), &mut t); // select "bar"
        e.handle_key(&key('d'), &mut t);
        assert_eq!(t.lines(), &["foo()baz"]);
    }

    #[test]
    fn visual_around_quote_then_yank() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["say \"hi\" now"]);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('h'), &mut t); // inside quotes
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('a'), &mut t);
        e.handle_key(&key('"'), &mut t); // select "\"hi\""
        e.handle_key(&key('y'), &mut t);
        let reg = e.registers.read().unwrap();
        assert_eq!(reg.text, "\"hi\"");
    }

    #[test]
    fn visual_find_extends_selection() {
        // vf, then d deletes through the ','
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello, world"]);
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key(','), &mut t); // cursor on ',' — selection covers "hello,"
        e.handle_key(&key('d'), &mut t);
        assert_eq!(t.lines(), &[" world"]);
    }

    #[test]
    fn visual_gg_extends_to_file_start() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('j'), &mut t);
        e.handle_key(&key('j'), &mut t); // row 2
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('g'), &mut t);
        e.handle_key(&key('g'), &mut t); // extend to (0,0)
        e.handle_key(&key('d'), &mut t); // delete from 't' of "three" back to start
        assert_eq!(t.lines(), &["hree"]);
    }

    #[test]
    fn visual_o_swaps_selection_ends() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abcde"]);
        e.handle_key(&key('l'), &mut t);
        e.handle_key(&key('l'), &mut t); // col 2 ('c')
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('l'), &mut t); // select c..d, cursor at 'd' (col 3)
        e.handle_key(&key('o'), &mut t); // cursor swaps to 'c' (col 2)
        assert_eq!(super::super::cursor_tuple(&t), (0, 2));
        e.handle_key(&key('h'), &mut t); // extend left from the anchor end
        e.handle_key(&key('d'), &mut t); // delete b..d inclusive
        assert_eq!(t.lines(), &["ae"]);
    }

    // ── Command spine: dot-repeat through the one apply() door ───────────────

    #[test]
    fn dot_repeats_cc_with_typed_text() {
        // previously a silent no-op (replay's `_other` arm)
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two"]);
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('c'), &mut t); // cc on "one"
        t.insert_str("X");
        e.handle_key(&esc(), &mut t); // line 0 = "X"
        e.handle_key(&key('j'), &mut t); // onto "two"
        e.handle_key(&key('.'), &mut t); // repeat cc+X
        assert_eq!(t.lines(), &["X", "X"]);
    }

    #[test]
    fn dot_repeats_substitute_char() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["ab cd"]);
        e.handle_key(&key('s'), &mut t); // delete 'a', Insert
        t.insert_str("Z");
        e.handle_key(&esc(), &mut t); // "Zb cd"
        e.handle_key(&key('w'), &mut t); // onto 'c'
        e.handle_key(&key('.'), &mut t); // repeat s+Z on 'c'
        assert_eq!(t.lines(), &["Zb Zd"]);
    }

    #[test]
    fn dot_repeats_plain_insert() {
        // vim: `ihello<Esc>` then `.` inserts "hello" again
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["world"]);
        e.handle_key(&key('i'), &mut t);
        t.insert_str("ab");
        e.handle_key(&esc(), &mut t); // "abworld", cursor on 'b'
        e.handle_key(&key('.'), &mut t); // insert "ab" again before 'b'
        assert_eq!(t.lines(), &["aabbworld"]);
    }

    #[test]
    fn dot_repeats_indent() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["x"]);
        e.handle_key(&key('>'), &mut t);
        e.handle_key(&key('>'), &mut t); // indent
        e.handle_key(&key('.'), &mut t); // repeat
        assert_eq!(t.lines(), &["        x"]);
    }

    #[test]
    fn dot_does_not_repeat_yank() {
        // vim: `.` repeats the last CHANGE; a yank after it must not displace it
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc"]);
        e.handle_key(&key('x'), &mut t); // delete 'a' (the change)
        e.handle_key(&key('y'), &mut t);
        e.handle_key(&key('l'), &mut t); // yank 'b' — not a change
        e.handle_key(&key('.'), &mut t); // must repeat x, not the yank
        assert_eq!(t.lines(), &["c"]);
    }

    // ── Range model: motion SpanKind classification + count composition ─────

    #[test]
    fn counts_before_and_after_operator_multiply() {
        // vim: 2d3w = 6 words, not count "23"
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["a b c d e f g"]);
        e.handle_key(&key('2'), &mut t);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('3'), &mut t);
        e.handle_key(&key('w'), &mut t);
        assert_eq!(t.lines(), &["g"]); // six words deleted
    }

    #[test]
    fn dj_deletes_two_whole_lines_linewise() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('l'), &mut t); // col 1 — must not matter (linewise)
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('j'), &mut t);
        assert_eq!(t.lines(), &["three"]);
        let reg = e.registers.read().unwrap();
        assert_eq!(reg.kind, RegisterKind::Linewise);
        assert_eq!(reg.text, "one\ntwo\n");
    }

    #[test]
    fn dk_deletes_two_whole_lines_upward() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('j'), &mut t); // row 1
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('k'), &mut t);
        assert_eq!(t.lines(), &["three"]);
    }

    #[test]
    #[allow(non_snake_case)]
    fn dG_deletes_to_file_end_linewise() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('j'), &mut t); // row 1
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('G'), &mut t);
        assert_eq!(t.lines(), &["one"]);
    }

    #[test]
    fn d_gg_deletes_to_file_start_linewise() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('j'), &mut t); // row 1
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('g'), &mut t);
        e.handle_key(&key('g'), &mut t);
        assert_eq!(t.lines(), &["three"]);
    }

    #[test]
    fn dt_deletes_up_to_but_not_including_target() {
        // vim t is inclusive of the char BEFORE the target: dtx on "abx" → "x"
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abx"]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('t'), &mut t);
        e.handle_key(&key('x'), &mut t);
        assert_eq!(t.lines(), &["x"]);
    }

    #[test]
    fn failed_find_with_operator_is_noop() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello"]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('z'), &mut t); // no 'z' on the line
        assert_eq!(t.lines(), &["hello"]); // nothing deleted
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('z'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Normal); // failed cf must not enter Insert
    }

    #[test]
    fn d_semicolon_repeats_find_as_operator_range() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["a.b.c"]);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('.'), &mut t); // cursor on first '.' (col 1)
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key(';'), &mut t); // delete through next '.' (inclusive)
        assert_eq!(t.lines(), &["ac"]);
    }

    #[test]
    fn cj_changes_two_lines_and_enters_insert() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two", "three"]);
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('j'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Insert);
        assert_eq!(t.lines(), &["", "three"]); // both lines gone, fresh empty line
    }

    // ── Register file: engine-owned unnamed register ────────────────────────

    #[test]
    fn x_then_p_swaps_chars() {
        // the classic vim `xp` idiom: x fills the register with the deleted char
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["ab"]);
        e.handle_key(&key('x'), &mut t); // delete 'a' → register "a"; line "b"
        e.handle_key(&key('p'), &mut t); // paste "a" after 'b'
        assert_eq!(t.lines(), &["ba"]);
    }

    #[test]
    fn x_at_line_end_does_not_join_next_line() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["ab", "cd"]);
        e.handle_key(&key('l'), &mut t); // onto 'b' (last char)
        e.handle_key(&key('3'), &mut t);
        e.handle_key(&key('x'), &mut t); // vim: deletes only 'b', never the newline
        assert_eq!(t.lines(), &["a", "cd"]);
    }

    #[test]
    fn s_fills_register_with_deleted_char() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abc"]);
        e.handle_key(&key('s'), &mut t); // delete 'a', enter Insert
        assert_eq!(*e.mode(), EditorMode::Insert);
        let reg = e.registers.read().expect("s must fill the register");
        assert_eq!(reg.text, "a");
        assert_eq!(reg.kind, RegisterKind::Charwise);
    }

    #[test]
    #[allow(non_snake_case)]
    fn S_fills_register_linewise_no_kind_desync() {
        // regression: S used to cut() (charwise content) while the engine kept
        // a stale Linewise kind from a prior yy — kind and content desynced.
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two"]);
        e.handle_key(&key('y'), &mut t);
        e.handle_key(&key('y'), &mut t); // register = "one\n" linewise
        e.handle_key(&key('j'), &mut t);
        e.handle_key(&key('S'), &mut t); // substitute line "two"
        let reg = e.registers.read().expect("S must fill the register");
        assert_eq!(reg.text, "two\n");
        assert_eq!(reg.kind, RegisterKind::Linewise);
    }

    #[test]
    fn dw_fills_register_charwise() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one two"]);
        e.handle_key(&key('d'), &mut t);
        e.handle_key(&key('w'), &mut t); // delete "one "
        let reg = e.registers.read().expect("dw must fill the register");
        assert_eq!(reg.text, "one ");
        assert_eq!(reg.kind, RegisterKind::Charwise);
        // and p pastes it back charwise
        e.handle_key(&key('p'), &mut t);
        assert_eq!(t.lines(), &["tone wo"]); // "one " pasted after 't'
    }

    #[test]
    fn empty_delete_keeps_previous_register() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["ab", ""]);
        e.handle_key(&key('y'), &mut t);
        e.handle_key(&key('l'), &mut t); // yank "a" charwise
        e.handle_key(&key('j'), &mut t); // empty line
        e.handle_key(&key('x'), &mut t); // no-op delete (empty line)
        let reg = e.registers.read().expect("register must survive a no-op delete");
        assert_eq!(reg.text, "a");
    }

    #[test]
    fn esc_in_normal_clears_stray_selection() {
        let mut e = VimEngine::default(); // Normal mode
        let mut t = TextArea::from(["hello world"]);
        // simulate a live selection while in Normal mode (as auto-surround/mouse-sync can leave)
        t.start_selection();
        t.move_cursor(ratatui_textarea::CursorMove::Forward);
        t.move_cursor(ratatui_textarea::CursorMove::Forward);
        assert!(t.selection_range().is_some());
        let out = e.handle_key(&esc(), &mut t);
        assert!(t.selection_range().is_none(), "Esc in Normal must cancel a stray selection");
        assert_eq!(out, VimKeyOutcome::CursorOnly);
        assert_eq!(*e.mode(), EditorMode::Normal);
    }
}

