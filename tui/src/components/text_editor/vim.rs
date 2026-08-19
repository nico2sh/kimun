//! Built-in vim emulation: a modal input interpreter over the **edit buffer**.
//! Pure over `&mut RopeBuffer` — no component state, no async.

use super::rope_buffer::{CursorMove, RopeBuffer};
use super::snapshot::EditorMode;
use super::vim_objects::{self as objects, TextObject};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Screen-level actions the host performs on the engine's behalf.
///
/// The clipboard variants exist because the engine must not link `arboard`
/// (the OS clipboard is deliberately kept out of the engine) but *is* the only thing
/// that knows the vim-inclusive selection and owns the mode transition. So the
/// engine does the editing and names the clipboard operation; the host performs
/// the I/O and reports the outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VimHostAction {
    OpenPalette, // `:`
    OpenSearch {
        forward: bool,
    }, // `/` (true) `?` (false)
    SearchNext,  // `n`
    SearchPrev,  // `N`
    /// Ctrl-C in Visual: the selection's text, already lifted by the engine.
    /// The buffer is unchanged and the engine has returned to Normal.
    ClipboardCopy(String),
    /// Ctrl-X in Visual: as Copy, but the engine has already removed the text —
    /// the host must bump the content revision.
    ClipboardCut(String),
    /// Ctrl-V: insert the OS clipboard at the cursor. Any visual selection has
    /// already been removed by the engine, so the host inserts into a clean
    /// cursor position.
    ClipboardPaste,
}

/// What a key did, so the host can bump the right revision counters.
#[derive(Debug, Clone, PartialEq, Eq)]
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

// ── Reified command model ────────────────────────────────────────

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
    WordForwardBig,            // W — WORD: any non-blank run
    WordBackBig,               // B
    WordEndBig,                // E
    WordEndBack { big: bool }, // ge / gE
    LineStart,
    FirstNonBlank,
    LastNonBlank, // g_
    LineEnd,
    FileStart,
    FileEnd,
    GotoLine(usize), // 5gg / 5G (1-based)
    ParagraphForward,
    ParagraphBack,
    MatchingPair,                                     // %
    FindChar { ch: char, till: bool, forward: bool }, // f/F/t/T
}

/// An operator awaiting a motion or text object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    Lowercase,  // gu
    Uppercase,  // gU
    ToggleCase, // g~
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

/// The fully-parsed unit of work. `apply` is the only door that
/// mutates the buffer; dot-repeat (and future macros) replay these values
/// through that same door, so first press and replay cannot diverge.
#[derive(Debug, Clone)]
pub enum Command {
    Move(Motion, usize),
    OperateMotion(Operator, Motion, usize),      // e.g. 2dw
    OperateLine(Operator, usize),                // dd / cc / yy with count
    OperateObject(Operator, TextObject),         // diw, ci"
    OperateToLineEnd(Operator),                  // D / C / Y
    IndentLines { outdent: bool, count: usize }, // >> / <<
    DeleteChar { forward: bool, count: usize },  // x / X
    ReplaceChar(char),                           // r<ch>
    SubstituteChar(usize),                       // s
    SubstituteLine,                              // S
    JoinLines { count: usize, spaced: bool },    // J (spaced) / gJ (raw)
    ToggleCase(usize),                           // ~
    Paste { after: bool, count: usize },         // p / P
    Undo(usize),                                 // u
    Redo(usize),                                 // Ctrl-r
    EnterInsert(InsertEntry),                    // i a I A o O
    EnterReplace,                                // R — overwrite until Esc
    EnterVisual { line: bool },                  // v / V
    Repeat,                                      // .
}

/// One key of the g-command grammar (the key AFTER a pending `g`). Produced
/// by `g_key_for`, consumed by both the Normal parser and the Visual handler.
enum GKey {
    /// `gg` — file start, or line N when a count is pending.
    GotoLine,
    /// `ge` / `gE` / `g_` — plain motions.
    Motion(Motion),
    /// `gu` / `gU` / `g~` — case operators.
    CaseOp(Operator),
    /// `gJ` — join without space handling.
    Join,
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

// ── Pending-state helper types ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct PendingFind {
    operator: Option<Operator>,
    till: bool,
    forward: bool,
}

/// A one-key continuation: the parser saw a prefix and waits for exactly one
/// more key. One field holds them all, so every ceremony site (clear,
/// space_leads, the footer hint) checks a single state instead of a drifting
/// list of flags — and a future `q`/`"`/`m` prefix is one new variant.
#[derive(Debug, Clone, Copy)]
enum Awaiting {
    /// `g` — the g-command grammar (`g_key_for`).
    G,
    /// `r` — the replacement char.
    ReplaceChar,
    /// `f`/`F`/`t`/`T` — the find target (the operator was captured at entry).
    Find(PendingFind),
    /// `i`/`a` after an operator (or in charwise Visual) — the object key.
    ObjectScope { around: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterKind {
    Charwise,
    Linewise,
}

/// One register's value — content and kind live together so they cannot
/// desync (the register is internal vim state, kept separate from
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
    // pending-state + dot-repeat fields
    pending_count: Option<usize>,
    /// Count typed BEFORE the operator (`2` in `2d3w`); multiplied with the
    /// motion count at completion (vim: `2d3w` deletes 6 words).
    pending_op_count: Option<usize>,
    pending_operator: Option<Operator>,
    /// The one-key continuation the parser is waiting on (g-prefix, find
    /// target, replace char, object key) — mutually exclusive by type.
    awaiting: Option<Awaiting>,
    last_find: Option<(char, bool, bool)>, // (ch, till, forward) for ; and ,
    registers: Registers,
    /// The last mutating command + captured insert delta, for `.`.
    last_change: Option<Change>,
    /// While in Insert via a vim command, the text typed is accumulated here
    /// (resulting delta) so `.` can replay it.
    insert_capture: Option<InsertCapture>,
    /// Replace mode's restore stack: what each overwritten position held
    /// (`None` = the char was appended past EOL). Backspace pops it.
    replace_stack: Vec<Option<char>>,
}

impl Default for VimEngine {
    fn default() -> Self {
        Self {
            mode: EditorMode::Normal,
            pending_count: None,
            pending_op_count: None,
            pending_operator: None,
            awaiting: None,
            last_find: None,
            registers: Registers::default(),
            last_change: None,
            insert_capture: None,
            replace_stack: Vec::new(),
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
            && self.awaiting.is_none()
        {
            return None;
        }
        let mut s = String::new();
        if let Some(n) = self.pending_op_count {
            s.push_str(&n.to_string());
        }
        if let Some(op) = self.pending_operator {
            s.push_str(match op {
                Operator::Delete => "d",
                Operator::Change => "c",
                Operator::Yank => "y",
                Operator::Indent => ">",
                Operator::Outdent => "<",
                Operator::Lowercase => "gu",
                Operator::Uppercase => "gU",
                Operator::ToggleCase => "g~",
            });
        }
        if let Some(n) = self.pending_count {
            s.push_str(&n.to_string());
        }
        match self.awaiting {
            Some(Awaiting::G) => s.push('g'),
            Some(Awaiting::ReplaceChar) => s.push('r'),
            Some(Awaiting::Find(pf)) => s.push(match (pf.till, pf.forward) {
                (false, true) => 'f',
                (false, false) => 'F',
                (true, true) => 't',
                (true, false) => 'T',
            }),
            Some(Awaiting::ObjectScope { around }) => s.push(if around { 'a' } else { 'i' }),
            None => {}
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
            && self.awaiting.is_none()
    }

    /// Interpret one key. In Insert mode everything except `Esc` is
    /// `PassThrough` (the host runs the existing direct textarea path).
    /// In Visual/VisualLine mode, motions extend the selection; operators
    /// act on the live selection. In Normal mode, motions move the cursor
    /// and the insert-entry keys switch to Insert mode.
    pub fn handle_key(&mut self, key: &KeyEvent, ta: &mut RopeBuffer) -> VimKeyOutcome {
        match self.mode {
            EditorMode::Insert => self.handle_insert(key, ta),
            EditorMode::Replace => self.handle_replace(key, ta),
            EditorMode::Visual | EditorMode::VisualLine => self.handle_visual(key, ta),
            _ => self.handle_normal(key, ta),
        }
    }

    // ── Visual + Visual-line mode handler ────────────────────────────────────

    fn handle_visual(&mut self, key: &KeyEvent, ta: &mut RopeBuffer) -> VimKeyOutcome {
        // One-key continuations consume the next key first: the find target
        // (`vf,` extends through the ','), and the object key after `i`/`a`
        // (`vi(` re-aims the selection at the object). The g continuation is
        // resolved below where the full key context is available.
        // A Ctrl-chord is never the awaited character (`vf` then Ctrl-C must
        // abandon the find, not search for a literal 'c'). Fall through to the
        // handling below, which routes Ctrl-C/X/V to the clipboard chords.
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match self.awaiting {
            Some(Awaiting::Find(pf)) if !ctrl => {
                self.awaiting = None;
                if let KeyCode::Char(ch) = key.code {
                    self.last_find = Some((ch, pf.till, pf.forward));
                    let cnt = self.take_count();
                    let motion = Motion::FindChar {
                        ch,
                        till: pf.till,
                        forward: pf.forward,
                    };
                    self.apply_motion(motion, cnt, ta);
                    return VimKeyOutcome::CursorOnly;
                }
                self.clear_pending();
                return VimKeyOutcome::NoOp;
            }
            Some(Awaiting::ObjectScope { around }) if !ctrl => {
                self.awaiting = None;
                if let KeyCode::Char(ch) = key.code
                    && let Some(obj) = objects::object_for_char(ch, around)
                {
                    Self::select_object_visual(obj, ta);
                    self.clear_pending();
                    return VimKeyOutcome::CursorOnly;
                }
                self.clear_pending();
                return VimKeyOutcome::NoOp;
            }
            _ => {}
        }

        // Esc: cancel selection and return to Normal.
        if key.code == KeyCode::Esc {
            ta.cancel_selection();
            self.mode = EditorMode::Normal;
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }

        // Arrow keys: extend the selection.
        let plain = key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT;
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
        // OS clipboard chords. The engine claims them so the mode and selection
        // transition happens here rather than behind its back — the host used to
        // never see these keys at all, because the filter below swallowed every
        // Ctrl-modified char.
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(c, 'c' | 'x' | 'v') {
            return self.clipboard_chord_visual(c, ta);
        }

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
            // vim visual case ops. `~` stays on the auto-surround
            // PassThrough path below (kimün wraps the selection instead).
            'u' => Some(Operator::Lowercase),
            'U' => Some(Operator::Uppercase),
            _ => None,
        };
        if let Some(op) = op {
            return self.visual_operate(op, ta);
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
                ta.jump_to(start_row, 0);
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
                // cut + insert is two history entries for one keypress; one
                // `edit()` scope makes visual `p` a single undo.
                ta.cut(); // cursor lands at the deletion gap
                self.fill_from_textarea(ta, RegisterKind::Charwise);
                // Record where the paste starts so we can leave the cursor there
                // (vim visual-p leaves cursor at the start of the pasted text).
                let paste_start = super::cursor_tuple(ta);
                ta.edit(|ta| {
                    ta.insert_str(&text); // insert the SAVED content, not the yank buffer
                    ta.jump_to(paste_start.0, paste_start.1);
                });
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
                ta.jump_to(cur.0, cur.1);
                ta.start_selection();
                ta.jump_to(other.0, other.1);
            }
            return VimKeyOutcome::CursorOnly;
        }

        // Visual `>`/`<` — indent/outdent the selected line range.
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
            ta.jump_to(start_row, 0);
            self.indent_lines(outdent, line_count, ta);
            self.mode = EditorMode::Normal;
            self.clear_pending();
            return VimKeyOutcome::TextMutated;
        }

        // Pair chars: set Normal and return PassThrough so the host's existing
        // auto-surround path wraps the selection. Skipped while a `g` is
        // pending — `g~` (case toggle) must reach the g-block below.
        if !matches!(self.awaiting, Some(Awaiting::G))
            && matches!(
                c,
                '(' | '[' | '{' | '<' | '"' | '\'' | '`' | '*' | '_' | '~'
            )
        {
            self.mode = EditorMode::Normal;
            return VimKeyOutcome::PassThrough;
        }

        // g prefix — the same shared g-command grammar as Normal mode,
        // dispatched against the selection. Case ops run on the selection
        // (bare `~` belongs to auto-surround in kimün, so g~ is the visual
        // toggle-case key); gJ joins the selected lines raw.
        if c == 'g' && !matches!(self.awaiting, Some(Awaiting::G)) {
            self.awaiting = Some(Awaiting::G);
            return VimKeyOutcome::NoOp;
        }
        if matches!(self.awaiting, Some(Awaiting::G)) {
            self.awaiting = None;
            return match Self::g_key_for(c) {
                Some(GKey::GotoLine) => {
                    let m = match self.pending_count.take() {
                        Some(n) => Motion::GotoLine(n),
                        None => Motion::FileStart,
                    };
                    self.apply_motion(m, 1, ta);
                    self.clear_pending();
                    VimKeyOutcome::CursorOnly
                }
                Some(GKey::Motion(m)) => {
                    let cnt = self.take_count();
                    self.apply_motion(m, cnt, ta);
                    self.clear_pending();
                    VimKeyOutcome::CursorOnly
                }
                Some(GKey::CaseOp(op)) => self.visual_operate(op, ta),
                Some(GKey::Join) => self.visual_join(false, ta),
                None => {
                    self.clear_pending();
                    VimKeyOutcome::NoOp
                }
            };
        }

        // J: join the selected lines with vim's space handling.
        if c == 'J' {
            return self.visual_join(true, ta);
        }

        // f/F/t/T: pend a selection-extending find.
        if let Some((till, forward)) = Self::find_spec_for(c) {
            self.awaiting = Some(Awaiting::Find(PendingFind {
                operator: None,
                till,
                forward,
            }));
            return VimKeyOutcome::NoOp;
        }

        // ; and , repeat the last find, extending the selection.
        if c == ';' || c == ',' {
            if let Some(motion) = self.repeat_find_motion(c) {
                let cnt = self.take_count();
                self.apply_motion(motion, cnt, ta);
            }
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }

        // i/a: text-object selection (charwise Visual only — `vi(`, `va"`).
        if (c == 'i' || c == 'a') && self.mode == EditorMode::Visual {
            self.awaiting = Some(Awaiting::ObjectScope { around: c == 'a' });
            return VimKeyOutcome::NoOp;
        }

        // Motions extend the selection. 5G extends to line 5 (count = line
        // number, matching the Normal-mode parser); the count is only
        // consumed for 'G' — every other motion keeps it as a repeat.
        if let Some(m) = Self::motion_for_char(c) {
            let m = if c == 'G' {
                match self.pending_count.take() {
                    Some(n) => Motion::GotoLine(n),
                    None => m,
                }
            } else {
                m
            };
            let count = self.take_count();
            self.apply_motion(m, count, ta);
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }

        self.clear_pending();
        VimKeyOutcome::NoOp
    }

    /// Visual `J` / `gJ`: join all selected lines into one (vim), then
    /// return to Normal mode.
    fn visual_join(&mut self, spaced: bool, ta: &mut RopeBuffer) -> VimKeyOutcome {
        let (start_row, end_row) = if let Some(((sr, _), (er, _))) = ta.selection_range() {
            (sr, er)
        } else {
            let (r, _) = super::cursor_tuple(ta);
            (r, r)
        };
        ta.cancel_selection();
        ta.jump_to(start_row, 0);
        let joins = end_row.saturating_sub(start_row).max(1);
        for _ in 0..joins {
            Self::join_line(ta, spaced);
        }
        self.mode = EditorMode::Normal;
        self.clear_pending();
        VimKeyOutcome::TextMutated
    }

    /// Apply `op` to the live visual selection (charwise or linewise) and
    /// leave Visual mode. Shared by the visual operator keys (d/x/c/s/y/u/U)
    /// and `g~`.
    fn visual_operate(&mut self, op: Operator, ta: &mut RopeBuffer) -> VimKeyOutcome {
        if self.mode == EditorMode::VisualLine {
            // VisualLine: operate on whole selected lines, preserving newlines.
            let (start_row, end_row) = if let Some(((sr, _), (er, _))) = ta.selection_range() {
                (sr, er)
            } else {
                let (r, _) = super::cursor_tuple(ta);
                (r, r)
            };
            // Cancel the current selection so apply_operator_linewise can
            // re-anchor from the correct start row.
            ta.cancel_selection();
            ta.jump_to(start_row, 0);
            let count = end_row - start_row + 1;
            self.apply_operator_linewise(op, count, None, ta);
        } else {
            // Charwise Visual: vim selection is inclusive of the char under
            // the cursor — re-select through select_range's inclusive end.
            let range = ta.selection_range();
            if let Some((start, end)) = range {
                ta.cancel_selection();
                Self::select_range(ta, start, end, true);
            }
            if op == Operator::Change {
                // Honest dot-repeat: `.` after a visual change replays a
                // same-sized change from the cursor (vim semantics) —
                // chars on one row, whole lines across rows.
                let capture_cmd = match range {
                    Some(((sr, sc), (er, ec))) if sr == er => Command::OperateMotion(
                        Operator::Change,
                        Motion::Right,
                        ec.saturating_sub(sc) + 1,
                    ),
                    Some(((sr, _), (er, _))) => {
                        Command::OperateLine(Operator::Change, er.saturating_sub(sr) + 1)
                    }
                    None => Command::OperateMotion(Operator::Change, Motion::Right, 1),
                };
                ta.cut();
                self.fill_from_textarea(ta, RegisterKind::Charwise);
                self.finish_insert_entry(&capture_cmd, None, ta);
            } else {
                self.apply_operator_on_selection(op, ta);
            }
        }
        // Change paths own the Insert transition (via the insert capture);
        // everything else returns to Normal here — one writer per transition.
        if op != Operator::Change {
            self.mode = EditorMode::Normal;
        }
        self.clear_pending();
        Self::outcome_for(op)
    }

    /// Ctrl-C / Ctrl-X / Ctrl-V in Visual or Visual-line.
    ///
    /// The text is lifted through the textarea's yank buffer — a transport only.
    /// The **unnamed register is deliberately not filled**: the OS clipboard and
    /// the register are independent channels, so a Ctrl-X must not clobber what
    /// `y` put in the register any more than `dd` may clobber the OS clipboard.
    ///
    /// All three land in Normal, because vim's Ctrl-C is Esc.
    ///
    /// **Paste does not mutate here.** It leaves the range *selected* and lets
    /// the host replace it as part of the insert, so a failed or empty clipboard
    /// read leaves the selection intact. Cutting first would destroy the user's
    /// text with nothing to put in its place, and the host's read can fail for
    /// ordinary reasons (empty clipboard, X11 hiccup).
    fn clipboard_chord_visual(&mut self, c: char, ta: &mut RopeBuffer) -> VimKeyOutcome {
        let linewise = self.mode == EditorMode::VisualLine;
        let Some(((sr, sc), (er, ec))) = ta.selection_range() else {
            self.mode = EditorMode::Normal;
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        };

        // The text the clipboard receives, computed from the line bodies rather
        // than from whatever range the buffer edit happens to consume — the
        // linewise delete below may swallow the *preceding* newline instead of
        // the trailing one, which would be wrong to hand to another application.
        let clipboard_text = if linewise {
            let body: String = ta.joined_rows(sr, er);
            format!("{body}\n")
        } else {
            String::new() // filled from the selection below
        };

        // The range the chord acts on: whole lines for linewise, the
        // vim-inclusive span (the char under the cursor counts) for charwise.
        let select_content = |ta: &mut RopeBuffer| {
            ta.cancel_selection();
            if linewise {
                let end_len = ta.row(er).map(|l| l.chars().count()).unwrap_or(ec);
                Self::select_range(ta, (sr, 0), (er, end_len), false);
            } else {
                Self::select_range(ta, (sr, sc), (er, ec), true);
            }
        };

        let action = match c {
            'c' => {
                select_content(ta);
                ta.copy();
                let text = if linewise {
                    clipboard_text
                } else {
                    ta.yank_text()
                };
                ta.cancel_selection();
                // vim leaves the cursor at the start of a yanked range.
                ta.jump_to(sr, sc);
                VimHostAction::ClipboardCopy(text)
            }
            'x' => {
                let text = if linewise {
                    // Take the newline with the lines, or `dd`'s stray-blank-line
                    // bug reappears on the clipboard path.
                    Self::select_lines_for_delete(ta, sr, er);
                    ta.cut();
                    clipboard_text
                } else {
                    select_content(ta);
                    ta.cut();
                    ta.yank_text()
                };
                VimHostAction::ClipboardCut(text)
            }
            // Paste: leave the range selected and let the host's insert replace
            // it atomically. Nothing is destroyed until there is something to
            // put in its place.
            _ => {
                select_content(ta);
                VimHostAction::ClipboardPaste
            }
        };
        self.mode = EditorMode::Normal;
        self.clear_pending();
        VimKeyOutcome::Host(action)
    }

    /// Re-aim the charwise visual selection at the text object under the
    /// cursor. The selection end is left ON the object's last char (visual
    /// selections are inclusive; the operator's inclusive `+1` restores the
    /// half-open range `object_range` computed).
    fn select_object_visual(obj: TextObject, ta: &mut RopeBuffer) {
        let Some((row, start, end)) = objects::object_range_at_cursor(ta, obj) else {
            return;
        };
        if start >= end {
            // Empty object (vi( on "()"): collapsing to one char would make
            // the operator's inclusive +1 grab the closing delimiter. No-op.
            return;
        }
        ta.cancel_selection();
        // Leave the selection end ON the object's last char (visual
        // selections are inclusive; the operator's +1 restores [start, end)).
        Self::select_range(ta, (row, start), (row, end - 1), false);
    }

    // ── Insert + Replace mode handlers ───────────────────────────────────────

    fn handle_insert(&mut self, key: &KeyEvent, ta: &mut RopeBuffer) -> VimKeyOutcome {
        if key.code == KeyCode::Esc {
            return self.exit_to_normal(ta);
        }
        VimKeyOutcome::PassThrough
    }

    /// Replace (overwrite) mode — vim `R`. Keys are handled by the engine,
    /// never passed to the host textarea path: R is raw overwrite, with no
    /// auto-surround / smart-Enter underneath.
    fn handle_replace(&mut self, key: &KeyEvent, ta: &mut RopeBuffer) -> VimKeyOutcome {
        // A live selection (mouse drag) would make the textarea's delete/
        // insert calls wipe it wholesale on the next keypress — drop it.
        if ta.selection_range().is_some() {
            ta.cancel_selection();
        }
        let plain = key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT;
        match key.code {
            KeyCode::Esc => self.exit_to_normal(ta),
            KeyCode::Enter => {
                ta.insert_newline();
                // The newline starts a fresh replace extent.
                self.replace_stack.clear();
                VimKeyOutcome::TextMutated
            }
            KeyCode::Backspace => {
                // vim's replace stack: Backspace restores what the position
                // held before it was overwritten; an appended char (None) is
                // simply removed. Past the extent it's a plain step back.
                if super::cursor_tuple(ta).1 > 0 {
                    ta.move_cursor(CursorMove::Back);
                    match self.replace_stack.pop() {
                        Some(Some(orig)) => {
                            ta.delete_next_char();
                            ta.insert_char(orig);
                            ta.move_cursor(CursorMove::Back);
                            return VimKeyOutcome::TextMutated;
                        }
                        Some(None) => {
                            ta.delete_next_char();
                            return VimKeyOutcome::TextMutated;
                        }
                        None => {}
                    }
                }
                VimKeyOutcome::CursorOnly
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                // vim allows movement in Replace mode and resets the replace
                // extent — restart the dot capture and the restore stack.
                ta.move_cursor(match key.code {
                    KeyCode::Left => CursorMove::Back,
                    KeyCode::Right => CursorMove::Forward,
                    KeyCode::Up => CursorMove::Up,
                    _ => CursorMove::Down,
                });
                let here = super::cursor_tuple(ta);
                if let Some(cap) = self.insert_capture.as_mut() {
                    cap.start = here;
                }
                self.replace_stack.clear();
                VimKeyOutcome::CursorOnly
            }
            KeyCode::Char(c) if plain => {
                // Record what this position held (None = appended past EOL)
                // so Backspace can restore it.
                let (row, col) = super::cursor_tuple(ta);
                let orig = ta.row(row).and_then(|l| l.chars().nth(col));
                self.replace_stack.push(orig);
                Self::overwrite_char(ta, c);
                VimKeyOutcome::TextMutated
            }
            _ => VimKeyOutcome::NoOp,
        }
    }

    /// Overwrite the char under the cursor (plain insert at EOL — vim R
    /// appends once the line runs out), cursor left after the written char.
    fn overwrite_char(ta: &mut RopeBuffer, ch: char) {
        if ch == '\n' {
            ta.insert_newline();
            return;
        }
        let (row, col) = super::cursor_tuple(ta);
        let len = ta.row_len(row);
        if col < len {
            ta.delete_next_char();
        }
        ta.insert_char(ch);
    }

    /// Esc out of Insert/Replace mode: finalize the dot capture and step the
    /// cursor back (vim).
    fn exit_to_normal(&mut self, ta: &mut RopeBuffer) -> VimKeyOutcome {
        self.mode = EditorMode::Normal;
        self.replace_stack.clear();
        // A stray selection (mouse drag mid-Insert/Replace) must not survive
        // into Normal mode, where motions would silently extend it.
        ta.cancel_selection();
        // Compute the typed text once at Esc, slicing from the start cursor
        // recorded when Insert/Replace began to the current cursor.
        if let Some(cap) = self.insert_capture.take() {
            let end = super::cursor_tuple(ta);
            let inserted = Self::text_between(ta, cap.start, end);
            if !inserted.is_empty() || Self::records_when_empty(&cap.command) {
                self.last_change = Some(Change {
                    command: cap.command,
                    inserted: Some(inserted),
                });
            }
        }
        if super::cursor_tuple(ta).1 > 0 {
            ta.move_cursor(CursorMove::Back);
        }
        VimKeyOutcome::CursorOnly
    }

    // ── Normal mode: keys → parse → Command → execute/apply ───────

    fn handle_normal(&mut self, key: &KeyEvent, ta: &mut RopeBuffer) -> VimKeyOutcome {
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
    /// accumulation — never touches the buffer.
    fn parse_normal(&mut self, key: &KeyEvent) -> Parsed {
        // One-key continuations (g-prefix, find target, replace char, object
        // key) consume the next key before anything else.
        if let Some(aw) = self.awaiting.take() {
            return self.parse_awaiting(aw, key);
        }

        // Esc cancels any pending sequence (operator, counts).
        if key.code == KeyCode::Esc {
            self.clear_pending();
            return Parsed::Cancel;
        }

        // Ctrl-r → redo (before the plain filter so it isn't stripped).
        if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Parsed::Cmd(Command::Redo(self.take_total_count()));
        }

        // OS clipboard chords, likewise ahead of the plain filter.
        // Ctrl-C is vim's Esc: no selection can exist in Normal, so there is
        // nothing to copy and cancelling the pending sequence is the useful
        // meaning. Ctrl-V pastes at the cursor. Ctrl-X has no Normal-mode
        // meaning here (vim's decrement is not emulated) and stays unmapped.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => {
                    self.clear_pending();
                    return Parsed::Cancel;
                }
                KeyCode::Char('v') => {
                    self.clear_pending();
                    return Parsed::Host(VimHostAction::ClipboardPaste);
                }
                _ => {}
            }
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

    /// Consume the single key a continuation was waiting for. Non-char keys
    /// cancel the whole pending sequence (vim); Esc additionally clears any
    /// stray selection via the `Cancel` path.
    fn parse_awaiting(&mut self, aw: Awaiting, key: &KeyEvent) -> Parsed {
        // A Ctrl-chord is never the awaited character — `r` then Ctrl-C must
        // abandon the replace, not overwrite with a literal 'c'. vim's Ctrl-C
        // is Esc, so route it the same way.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            self.clear_pending();
            return if key.code == KeyCode::Char('c') {
                Parsed::Cancel
            } else {
                Parsed::Nothing
            };
        }
        let KeyCode::Char(c) = key.code else {
            self.clear_pending();
            return if key.code == KeyCode::Esc {
                Parsed::Cancel
            } else {
                Parsed::Nothing
            };
        };
        match aw {
            Awaiting::ReplaceChar => Parsed::Cmd(Command::ReplaceChar(c)),
            Awaiting::Find(pf) => {
                self.last_find = Some((c, pf.till, pf.forward));
                let motion = Motion::FindChar {
                    ch: c,
                    till: pf.till,
                    forward: pf.forward,
                };
                match pf.operator {
                    Some(op) => {
                        Parsed::Cmd(Command::OperateMotion(op, motion, self.take_total_count()))
                    }
                    None => Parsed::Cmd(Command::Move(motion, self.take_count())),
                }
            }
            Awaiting::G => self.parse_g_key(c),
            Awaiting::ObjectScope { around } => {
                if let Some(obj) = objects::object_for_char(c, around)
                    && let Some(op) = self.pending_operator.take()
                {
                    self.clear_pending();
                    return Parsed::Cmd(Command::OperateObject(op, obj));
                }
                self.clear_pending();
                Parsed::Nothing
            }
        }
    }

    /// The key after a pending `g`, dispatched through the shared g-command
    /// grammar (`g_key_for`).
    fn parse_g_key(&mut self, c: char) -> Parsed {
        match Self::g_key_for(c) {
            Some(GKey::GotoLine) => {
                // A count is a line number (5gg → line 5), wherever it was
                // typed relative to a pending operator (d5gg / 5dgg).
                let target = self
                    .pending_count
                    .take()
                    .or_else(|| self.pending_op_count.take());
                let m = match target {
                    Some(n) => Motion::GotoLine(n),
                    None => Motion::FileStart,
                };
                match self.pending_operator.take() {
                    Some(op) => Parsed::Cmd(Command::OperateMotion(op, m, 1)),
                    None => Parsed::Cmd(Command::Move(m, 1)),
                }
            }
            Some(GKey::Motion(m)) => match self.pending_operator.take() {
                Some(op) => Parsed::Cmd(Command::OperateMotion(op, m, self.take_total_count())),
                None => Parsed::Cmd(Command::Move(m, self.take_count())),
            },
            Some(GKey::CaseOp(op)) => {
                // gugu / gUgU / g~g~: the doubled g-form runs linewise.
                if self.pending_operator == Some(op) {
                    self.pending_operator = None;
                    return Parsed::Cmd(Command::OperateLine(op, self.take_total_count()));
                }
                self.pending_operator = Some(op);
                self.pending_op_count = self.pending_count.take();
                Parsed::Pending
            }
            Some(GKey::Join) => Parsed::Cmd(Command::JoinLines {
                count: self.take_count().max(2) - 1,
                spaced: false,
            }),
            None => {
                // Unmapped g-sequence aborts the whole pending state (vim).
                self.clear_pending();
                Parsed::Nothing
            }
        }
    }

    // ── parse_normal_char: pure Normal-mode key parser ───────────────────────

    /// Parse one plain Normal-mode char. Pure pending-state accumulation —
    /// commands come out as values; nothing here touches the buffer.
    fn parse_normal_char(&mut self, c: char) -> Parsed {
        // Count digits accumulate first.
        if self.accumulate_count(c) {
            return Parsed::Pending;
        }

        // g prefix — the next key resolves through the g-command grammar.
        if c == 'g' {
            self.awaiting = Some(Awaiting::G);
            return Parsed::Pending;
        }

        // guu / gUU / g~~: the doubled-key form runs the case op linewise.
        if let Some(op) = self.pending_operator {
            let doubles = matches!(
                (op, c),
                (Operator::Lowercase, 'u')
                    | (Operator::Uppercase, 'U')
                    | (Operator::ToggleCase, '~')
            );
            if doubles {
                self.pending_operator = None;
                return Parsed::Cmd(Command::OperateLine(op, self.take_total_count()));
            }
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
            if self.pending_operator.is_some() {
                // A different operator while one is pending aborts (vim).
                self.clear_pending();
                return Parsed::Nothing;
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
            if self.pending_operator.is_some() {
                self.clear_pending();
                return Parsed::Nothing; // dD etc. abort (vim)
            }
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
            if self.pending_operator.is_some() {
                self.clear_pending();
                return Parsed::Nothing; // d> etc. abort (vim)
            }
            self.pending_operator = Some(if outdent {
                Operator::Outdent
            } else {
                Operator::Indent
            });
            self.pending_op_count = self.pending_count.take();
            return Parsed::Pending;
        }

        // Paste.
        if c == 'p' || c == 'P' {
            if self.pending_operator.is_some() {
                self.clear_pending();
                return Parsed::Nothing; // dp etc. abort (vim)
            }
            return Parsed::Cmd(Command::Paste {
                after: c == 'p',
                count: self.take_count(),
            });
        }

        // f/F/t/T — await the find target (captures the operator so `df,` works).
        if let Some((till, forward)) = Self::find_spec_for(c) {
            self.awaiting = Some(Awaiting::Find(PendingFind {
                operator: self.pending_operator.take(),
                till,
                forward,
            }));
            return Parsed::Pending;
        }

        // ; and , — repeat last find (same / opposite direction); with a
        // pending operator (`d;`) forms a range like any motion.
        if c == ';' || c == ',' {
            if let Some(motion) = self.repeat_find_motion(c) {
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

        // Text objects — `i`/`a` with an operator pending awaits the object
        // key (so `di`/`ci`/`yi` never enter Insert; the object char is
        // consumed by parse_awaiting, not the motion dispatch).
        if self.pending_operator.is_some() && (c == 'i' || c == 'a') {
            self.awaiting = Some(Awaiting::ObjectScope { around: c == 'a' });
            return Parsed::Pending;
        }

        // Motion dispatch (count-aware; with a pending operator, forms a range).
        if let Some(m) = Self::motion_for_char(c) {
            // 5G goes to line 5 — the count is a line number, not a repeat.
            if c == 'G' {
                let target = self
                    .pending_count
                    .take()
                    .or_else(|| self.pending_op_count.take());
                let m = match target {
                    Some(n) => Motion::GotoLine(n),
                    None => m,
                };
                return match self.pending_operator.take() {
                    Some(op) => Parsed::Cmd(Command::OperateMotion(op, m, 1)),
                    None => Parsed::Cmd(Command::Move(m, 1)),
                };
            }
            return match self.pending_operator.take() {
                Some(op) => Parsed::Cmd(Command::OperateMotion(op, m, self.take_total_count())),
                None => Parsed::Cmd(Command::Move(m, self.take_count())),
            };
        }

        // A pending operator followed by a key that forms no motion, object,
        // find, or doubled form aborts the whole sequence (vim beeps and
        // cancels) — `gUu` must not run Undo, `dx` must not delete a char.
        if self.pending_operator.is_some() {
            self.clear_pending();
            return Parsed::Nothing;
        }

        // Single-key edits, dot, visual entry, host actions, insert entry.
        // NOTE: i/a only reach here when NO operator is pending — operator +
        // i/a is the text-object path above.
        let cmd = match c {
            'x' => Command::DeleteChar {
                forward: true,
                count: self.take_count(),
            },
            'X' => Command::DeleteChar {
                forward: false,
                count: self.take_count(),
            },
            'r' => {
                self.awaiting = Some(Awaiting::ReplaceChar);
                return Parsed::Pending;
            }
            's' => Command::SubstituteChar(self.take_count()),
            'S' => Command::SubstituteLine,
            'R' => Command::EnterReplace,
            'J' => Command::JoinLines {
                count: self.take_count().max(2) - 1,
                spaced: true,
            },
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
            // Host actions — `:` `/` `?` `n` `N`. `?` backward-first
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

    /// Run a freshly-parsed command through the one mutation door, recording
    /// it for `.` when it is a repeatable change. Change-family commands
    /// defer recording to Esc (the insert capture owns it).
    fn execute(&mut self, cmd: Command, ta: &mut RopeBuffer) -> VimKeyOutcome {
        let outcome = self.apply(&cmd, None, ta);
        if outcome != VimKeyOutcome::NoOp && Self::repeatable(&cmd) && self.insert_capture.is_none()
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
            // Exhaustive on purpose — a new Command variant must decide its
            // dot-repeat policy here, not inherit a silent default.
            Command::IndentLines { .. }
            | Command::DeleteChar { .. }
            | Command::ReplaceChar(_)
            | Command::SubstituteChar(_)
            | Command::SubstituteLine
            | Command::JoinLines { .. }
            | Command::ToggleCase(_)
            | Command::Paste { .. }
            | Command::EnterInsert(_)
            | Command::EnterReplace => true,
        }
    }

    /// Whether Esc with NOTHING typed still records the command for `.`.
    /// Plain insert entries (i/a/I/A, R) don't — an aborted insert is not a
    /// change in vim. o/O do (the opened line IS the change), and the
    /// Change family does (the cut already happened before Insert began).
    /// Exhaustive on purpose, like `repeatable` — a new command must decide.
    fn records_when_empty(cmd: &Command) -> bool {
        match cmd {
            Command::EnterInsert(
                InsertEntry::Here
                | InsertEntry::After
                | InsertEntry::LineStart
                | InsertEntry::LineEnd,
            )
            | Command::EnterReplace => false,
            Command::EnterInsert(InsertEntry::OpenBelow | InsertEntry::OpenAbove)
            | Command::Move(..)
            | Command::OperateMotion(..)
            | Command::OperateLine(..)
            | Command::OperateObject(..)
            | Command::OperateToLineEnd(_)
            | Command::IndentLines { .. }
            | Command::DeleteChar { .. }
            | Command::ReplaceChar(_)
            | Command::SubstituteChar(_)
            | Command::SubstituteLine
            | Command::JoinLines { .. }
            | Command::ToggleCase(_)
            | Command::Paste { .. }
            | Command::Undo(_)
            | Command::Redo(_)
            | Command::EnterVisual { .. }
            | Command::Repeat => true,
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
        ta: &mut RopeBuffer,
    ) -> VimKeyOutcome {
        match *cmd {
            Command::Move(m, n) => {
                self.apply_motion(m, n, ta);
                VimKeyOutcome::CursorOnly
            }
            Command::OperateMotion(op, m, n) => {
                if self.apply_operator_motion(op, m, n, inserted, ta) {
                    Self::outcome_for(op)
                } else {
                    VimKeyOutcome::NoOp
                }
            }
            Command::OperateLine(op, n) => {
                self.apply_operator_linewise(op, n, inserted, ta);
                Self::outcome_for(op)
            }
            Command::OperateObject(op, obj) => {
                if self.apply_operator_object(op, obj, inserted, ta) {
                    Self::outcome_for(op)
                } else {
                    VimKeyOutcome::NoOp
                }
            }
            Command::OperateToLineEnd(op) => {
                self.apply_operator_to_line_end(op, inserted, ta);
                Self::outcome_for(op)
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
                if let Some(text) = ta.row(row).map(|l| format!("{l}\n")) {
                    self.registers.fill(text, RegisterKind::Linewise);
                }
                ta.move_cursor(CursorMove::Head);
                ta.start_selection();
                ta.move_cursor(CursorMove::End);
                ta.cut();
                self.finish_insert_entry(cmd, inserted, ta);
                VimKeyOutcome::TextMutated
            }
            Command::JoinLines { count, spaced } => {
                for _ in 0..count.max(1) {
                    Self::join_line(ta, spaced);
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
                // The buffer groups by extent, so a bare `u` takes the whole
                // action without the engine reporting anything back to the
                // host. A counted `3u` is still entry-wise.
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
            Command::EnterReplace => match inserted {
                Some(text) => {
                    for ch in text.chars() {
                        Self::overwrite_char(ta, ch);
                    }
                    self.mode = EditorMode::Normal;
                    if super::cursor_tuple(ta).1 > 0 {
                        ta.move_cursor(CursorMove::Back);
                    }
                    VimKeyOutcome::TextMutated
                }
                None => {
                    self.enter_insert_capture(cmd.clone(), ta);
                    self.mode = EditorMode::Replace; // the capture helper sets Insert
                    self.replace_stack.clear();
                    VimKeyOutcome::CursorOnly
                }
            },
            Command::EnterVisual { line } => {
                // Anchor at the cursor's current position — vim leaves the
                // cursor exactly where it was on `v`/`V`; the full-line
                // highlight for VisualLine is derived from the row range
                // downstream, not from jumping the cursor to Head/End here.
                ta.start_selection();
                self.mode = if line {
                    EditorMode::VisualLine
                } else {
                    EditorMode::Visual
                };
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
    fn finish_insert_entry(&mut self, cmd: &Command, inserted: Option<&str>, ta: &mut RopeBuffer) {
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
        ta: &mut RopeBuffer,
    ) -> VimKeyOutcome {
        let opened_line = match entry {
            InsertEntry::Here => false,
            InsertEntry::After => {
                ta.move_cursor(CursorMove::Forward);
                false
            }
            InsertEntry::LineStart => {
                // vim I: insert before the FIRST NON-BLANK char, not col 0.
                Self::first_non_blank(ta);
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
            'w' => Some(Motion::WordForward),
            'W' => Some(Motion::WordForwardBig),
            'b' => Some(Motion::WordBack),
            'B' => Some(Motion::WordBackBig),
            'e' => Some(Motion::WordEnd),
            'E' => Some(Motion::WordEndBig),
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

    /// The g-command grammar: what one key after a pending `g` means. Both
    /// the Normal parser and the Visual handler consume this single table
    /// (dispatching per mode), so a new g-command is added exactly once.
    fn g_key_for(c: char) -> Option<GKey> {
        match c {
            'g' => Some(GKey::GotoLine), // gg — file start, or line N with a count
            'e' => Some(GKey::Motion(Motion::WordEndBack { big: false })),
            'E' => Some(GKey::Motion(Motion::WordEndBack { big: true })),
            '_' => Some(GKey::Motion(Motion::LastNonBlank)),
            'u' => Some(GKey::CaseOp(Operator::Lowercase)),
            'U' => Some(GKey::CaseOp(Operator::Uppercase)),
            '~' => Some(GKey::CaseOp(Operator::ToggleCase)),
            'J' => Some(GKey::Join),
            _ => None,
        }
    }

    /// Map a find key to its `(till, forward)` spec. Shared by the Normal
    /// parser and the Visual handler.
    fn find_spec_for(c: char) -> Option<(bool, bool)> {
        match c {
            'f' => Some((false, true)),
            'F' => Some((false, false)),
            't' => Some((true, true)),
            'T' => Some((true, false)),
            _ => None,
        }
    }

    /// The motion `;` / `,` repeats: the last find, same or reversed
    /// direction. Shared by the Normal parser and the Visual handler.
    fn repeat_find_motion(&self, c: char) -> Option<Motion> {
        let (ch, till, fwd) = self.last_find?;
        let forward = if c == ';' { fwd } else { !fwd };
        Some(Motion::FindChar { ch, till, forward })
    }

    // ── count accumulation helpers ───────────────────────────────────────────

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
        self.awaiting = None;
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

    // ── Motion resolution ────────────────────────────────────────────────────

    /// Where `motion` (× count) would land, as a position value — no net
    /// cursor mutation (the cursor is restored before returning).
    fn resolve_motion(&self, motion: Motion, count: usize, ta: &mut RopeBuffer) -> (usize, usize) {
        let saved = super::cursor_tuple(ta);
        self.apply_motion(motion, count, ta);
        let target = super::cursor_tuple(ta);
        ta.jump_to(saved.0, saved.1);
        target
    }

    /// Vim's motion classification: how a motion forms an operator range.
    /// (`:h exclusive` — every vim motion is exclusive, inclusive, or
    /// linewise when consumed by an operator.)
    fn kind_of(motion: Motion) -> SpanKind {
        match motion {
            Motion::Up
            | Motion::Down
            | Motion::FileStart
            | Motion::FileEnd
            | Motion::GotoLine(_) => SpanKind::Linewise,
            Motion::WordEnd
            | Motion::WordEndBig
            | Motion::WordEndBack { .. }
            | Motion::MatchingPair => SpanKind::Inclusive,
            // d$ / dg_ delete through the char they land on (vim: inclusive).
            Motion::LineEnd | Motion::LastNonBlank => SpanKind::Inclusive,
            // f/t are inclusive; F/T (backward) are exclusive.
            Motion::FindChar { forward: true, .. } => SpanKind::Inclusive,
            _ => SpanKind::Exclusive,
        }
    }

    /// Select `[start, end]` (inclusive) or `[start, end)` on the textarea.
    /// The single home of the vim-inclusive → ratatui-half-open `+1`
    /// conversion, clamped to the end line's length.
    fn select_range(
        ta: &mut RopeBuffer,
        start: (usize, usize),
        end: (usize, usize),
        inclusive: bool,
    ) {
        let (er, ec) = end;
        let end_col = if inclusive {
            let len = ta.row(er).map(|l| l.chars().count()).unwrap_or(ec);
            (ec + 1).min(len)
        } else {
            ec
        };
        ta.jump_to(start.0, start.1);
        ta.start_selection();
        ta.jump_to(er, end_col);
    }

    fn apply_motion(&self, motion: Motion, count: usize, ta: &mut RopeBuffer) {
        // Count-finds are atomic in vim: `2fx` with one 'x' fails the WHOLE
        // motion (cursor stays put) — never "as far as possible". Handled
        // outside the per-count loop, which can't express that.
        if let Motion::FindChar { ch, till, forward } = motion {
            Self::find_char_count(ta, ch, till, forward, count);
            return;
        }
        for _ in 0..count.max(1) {
            match motion {
                Motion::Left => ta.move_cursor(CursorMove::Back),
                Motion::Right => ta.move_cursor(CursorMove::Forward),
                Motion::Up => ta.move_cursor(CursorMove::Up),
                Motion::Down => ta.move_cursor(CursorMove::Down),
                Motion::WordForward => ta.move_cursor(CursorMove::WordForward),
                Motion::WordBack => ta.move_cursor(CursorMove::WordBack),
                Motion::WordEnd => ta.move_cursor(CursorMove::WordEnd),
                Motion::WordForwardBig => ta.move_cursor(CursorMove::WordForwardBig),
                Motion::WordBackBig => ta.move_cursor(CursorMove::WordBackBig),
                Motion::WordEndBig => ta.move_cursor(CursorMove::WordEndBig),
                Motion::WordEndBack { big } => ta.move_cursor(CursorMove::WordEndBack { big }),
                Motion::LineStart => ta.move_cursor(CursorMove::Head),
                Motion::FirstNonBlank => Self::first_non_blank(ta),
                Motion::LastNonBlank => Self::last_non_blank(ta),
                Motion::LineEnd => ta.move_cursor(CursorMove::End),
                Motion::FileStart => ta.move_cursor(CursorMove::Top),
                Motion::FileEnd => ta.move_cursor(CursorMove::Bottom),
                Motion::GotoLine(n) => {
                    let last = ta.row_count().saturating_sub(1);
                    let row = n.saturating_sub(1).min(last);
                    ta.jump_to(row, 0);
                }
                Motion::ParagraphForward => ta.move_cursor(CursorMove::ParagraphForward),
                Motion::ParagraphBack => ta.move_cursor(CursorMove::ParagraphBack),
                Motion::MatchingPair => ta.move_cursor(CursorMove::MatchingPair),
                Motion::FindChar { .. } => unreachable!("handled atomically above"),
            }
        }
    }

    fn first_non_blank(ta: &mut RopeBuffer) {
        let (row, _) = super::cursor_tuple(ta);
        if let Some(line) = ta.row(row) {
            let n = line.chars().take_while(|c| c.is_whitespace()).count();
            ta.jump_to(row, n);
        }
    }

    /// `g_` — last non-blank char of the line (no-op on a blank line, vim).
    fn last_non_blank(ta: &mut RopeBuffer) {
        let (row, _) = super::cursor_tuple(ta);
        let idx = ta.row(row).and_then(|line| {
            line.chars()
                .enumerate()
                .filter(|(_, c)| !c.is_whitespace())
                .map(|(i, _)| i)
                .last()
        });
        if let Some(idx) = idx {
            ta.jump_to(row, idx);
        }
    }

    /// Move to the `count`-th occurrence of `ch` on the current line —
    /// atomically: fewer than `count` occurrences fails the whole motion and
    /// the cursor does not move (vim). `forward`: search right from col+1;
    /// otherwise left from col-1. `till`: stop one column short (t/T).
    fn find_char_count(ta: &mut RopeBuffer, ch: char, till: bool, forward: bool, count: usize) {
        let (row, col) = super::cursor_tuple(ta);
        let Some(line) = ta.row(row) else {
            return;
        };
        let chars: Vec<char> = line.chars().collect();
        let n = count.max(1);
        let pos = if forward {
            ((col + 1)..chars.len())
                .filter(|&i| chars[i] == ch)
                .nth(n - 1)
        } else {
            (0..col).rev().filter(|&i| chars[i] == ch).nth(n - 1)
        };
        let Some(pos) = pos else { return };
        let target = if till {
            if forward {
                pos.saturating_sub(1)
            } else {
                pos + 1
            }
        } else {
            pos
        };
        ta.jump_to(row, target);
    }

    // ── Operator framework ───────────────────────────────────────────────────

    fn outcome_for(op: Operator) -> VimKeyOutcome {
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
        ta: &mut RopeBuffer,
    ) -> bool {
        // Vim `cw`/`cW` semantics: change + word-forward uses word-end (not
        // word-start of the next word), so the trailing space is preserved.
        // This is vim's well-known `cw = ce` behaviour. Other operators (dw, yw)
        // use the motion as-is (including the trailing space).
        let effective_motion = if op == Operator::Change {
            match m {
                Motion::WordForward => Motion::WordEnd,
                Motion::WordForwardBig => Motion::WordEndBig, // cW = cE
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
                ta.jump_to(top, 0);
                self.apply_operator_linewise(op, lines, inserted, ta);
                true
            }
            kind => {
                if target == origin
                    && (kind == SpanKind::Exclusive
                        || matches!(
                            effective_motion,
                            // Inclusive motions that signal failure by not
                            // moving: failed find/pair-match, ge at buffer
                            // start, E at buffer end.
                            Motion::FindChar { .. }
                                | Motion::MatchingPair
                                | Motion::WordEndBack { .. }
                                | Motion::WordEndBig
                        ))
                {
                    // Failed motion or zero-width exclusive range: vim no-op
                    // — nothing deleted, no Insert, register kept.
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
                    self.apply_operator_on_selection(op, ta);
                }
                true
            }
        }
    }

    /// Select rows `r0..=r1` for a **linewise delete**, consuming one newline so
    /// no empty remnant is left behind: the trailing newline when a line
    /// follows, otherwise the preceding one (last-line case), and the whole
    /// buffer when it is all of it (which leaves the textarea's mandatory `[""]`).
    ///
    /// Shared by `dd`/`cc` and by the OS-clipboard Ctrl-X, which used to select
    /// only the line *bodies* and so left a stray blank line behind every time.
    fn select_lines_for_delete(ta: &mut RopeBuffer, r0: usize, r1: usize) {
        let last = ta.row_count().saturating_sub(1);
        if r1 < last {
            ta.jump_to(r0, 0);
            ta.start_selection();
            ta.jump_to(r1 + 1, 0);
        } else if r0 > 0 {
            let prev_end = ta.row_len(r0 - 1);
            ta.jump_to(r0 - 1, prev_end);
            ta.start_selection();
            let end = ta.row_len(r1);
            ta.jump_to(r1, end);
        } else {
            ta.jump_to(0, 0);
            ta.start_selection();
            let end = ta.row_len(r1);
            ta.jump_to(r1, end);
        }
    }

    fn apply_operator_linewise(
        &mut self,
        op: Operator,
        count: usize,
        inserted: Option<&str>,
        ta: &mut RopeBuffer,
    ) {
        let (r0, _) = super::cursor_tuple(ta);
        let last = ta.row_count().saturating_sub(1);
        let r1 = (r0 + count.saturating_sub(1)).min(last);

        // Register content: the line bodies plus a trailing newline (linewise).
        let body: String = ta.joined_rows(r0, r1);
        let register_text = format!("{body}\n");

        match op {
            Operator::Yank => {
                self.registers.fill(register_text, RegisterKind::Linewise);
                // cursor stays at start of first yanked line
                ta.jump_to(r0, 0);
            }
            Operator::Delete | Operator::Change => {
                Self::select_lines_for_delete(ta, r0, r1);
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
                        ta.jump_to(0, 0);
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
                self.indent_lines(outdent, count, ta);
            }
            Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
                // guu / gUU / g~~ / guj…: transform whole lines in ONE
                // cut+insert so undo reverts the command in one step, not
                // per line. Case operators never touch the register (vim).
                let transformed = (r0..=r1)
                    .filter_map(|row| ta.row(row))
                    .map(|l| Self::transform_case(&l, op))
                    .collect::<Vec<_>>()
                    .join("\n");
                let end_len = ta.row_len(r1);
                // cut + insert is two history entries; one `edit()` scope makes
                // them one **undo group**, whatever the count turns out to be.
                ta.edit(|ta| {
                    ta.jump_to(r0, 0);
                    ta.start_selection();
                    ta.jump_to(r1, end_len);
                    ta.cut();
                    ta.insert_str(&transformed);
                    ta.jump_to(r0, 0);
                });
            }
        }
    }

    fn apply_operator_to_line_end(
        &mut self,
        op: Operator,
        inserted: Option<&str>,
        ta: &mut RopeBuffer,
    ) {
        ta.start_selection();
        ta.move_cursor(CursorMove::End);
        if op == Operator::Change {
            ta.cut();
            self.fill_from_textarea(ta, RegisterKind::Charwise);
            self.finish_insert_entry(&Command::OperateToLineEnd(op), inserted, ta);
        } else {
            self.apply_operator_on_selection(op, ta);
        }
    }

    /// Indent or outdent the cursor's line by one **indent step**, then repeat
    /// for `count` lines total (moving down after each). Used by `>>`, `<<`, and
    /// the visual `>`/`<` operators.
    ///
    /// The step comes from the buffer rather than a literal here, so vim's `>>`
    /// and the plain backend's Tab move a line by the same amount. (Vim's own
    /// name for this is `shiftwidth`, which is not `tabstop` — see
    /// `DEFAULT_INDENT_WIDTH`.)
    fn indent_lines(&self, outdent: bool, count: usize, ta: &mut RopeBuffer) {
        let step = ta.indent_width() as usize;
        // One vim command is one undo: this pushes an entry per row, so the
        // whole block goes in a single `edit()` scope.
        ta.edit(|ta| {
            let (start_row, start_col) = super::cursor_tuple(ta);
            let mut first_line_delta = 0usize; // indent change on the cursor's own line
            for i in 0..count.max(1) {
                ta.move_cursor(CursorMove::Head);
                if outdent {
                    // Remove up to one step's worth of leading spaces.
                    let (row, _) = super::cursor_tuple(ta);
                    let n = ta
                        .row(row)
                        .map(|l| l.chars().take(step).take_while(|c| *c == ' ').count())
                        .unwrap_or(0);
                    if i == 0 {
                        first_line_delta = n;
                    }
                    for _ in 0..n {
                        ta.delete_next_char();
                    }
                } else {
                    if i == 0 {
                        first_line_delta = step;
                    }
                    ta.insert_str(" ".repeat(step));
                }
                ta.move_cursor(CursorMove::Down);
            }
            // Keep the cursor over the same character it sat on, shifted by the
            // indent change — matches neovim's >> behavior.
            let col = if outdent {
                start_col.saturating_sub(first_line_delta)
            } else {
                start_col + first_line_delta
            };
            ta.jump_to(start_row, col);
        });
    }

    /// Capture the text the textarea just cut/copied (its yank buffer) into
    /// the engine's unnamed register. The textarea yank buffer is only a
    /// transport here — the engine never reads it back at paste time.
    fn fill_from_textarea(&mut self, ta: &RopeBuffer, kind: RegisterKind) {
        self.registers.fill(ta.yank_text(), kind);
    }

    /// Charwise operator over the live selection. Change never reaches here —
    /// every Change path captures its own command before cutting (so `.`
    /// replays the right thing); linewise flows use apply_operator_linewise.
    fn apply_operator_on_selection(&mut self, op: Operator, ta: &mut RopeBuffer) {
        match op {
            Operator::Yank => {
                let start = ta.selection_range().map(|(s, _)| s);
                ta.copy();
                self.fill_from_textarea(ta, RegisterKind::Charwise);
                ta.cancel_selection();
                if let Some((r, c)) = start {
                    ta.jump_to(r, c);
                }
            }
            Operator::Delete | Operator::Change => {
                ta.cut();
                self.fill_from_textarea(ta, RegisterKind::Charwise);
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
                ta.jump_to(start_row, 0);
                self.indent_lines(outdent, rows, ta);
            }
            Operator::Lowercase | Operator::Uppercase | Operator::ToggleCase => {
                // Replace the selection with its case-transformed text and
                // leave the cursor at the start (vim). The cut only passes
                // through the textarea yank buffer — the engine register is
                // deliberately NOT filled (vim: case operators don't yank).
                let start = ta.selection_range().map(|(s, _)| s);
                // cut + insert as one **undo group**.
                ta.edit(|ta| {
                    ta.cut();
                    let transformed = Self::transform_case(&ta.yank_text(), op);
                    ta.insert_str(&transformed);
                });
                if let Some((r, c)) = start {
                    ta.jump_to(r, c);
                }
            }
        }
    }

    /// Flip one char's case. The single home of toggle-case semantics,
    /// shared by bare `~`, visual/operator `g~`, and `transform_case`.
    fn flip_case(ch: char) -> String {
        if ch.is_uppercase() {
            ch.to_lowercase().collect()
        } else {
            ch.to_uppercase().collect()
        }
    }

    fn transform_case(text: &str, op: Operator) -> String {
        match op {
            Operator::Lowercase => text.to_lowercase(),
            Operator::Uppercase => text.to_uppercase(),
            _ => text.chars().map(Self::flip_case).collect(),
        }
    }

    fn enter_insert_capture(&mut self, command: Command, ta: &RopeBuffer) {
        self.mode = EditorMode::Insert;
        self.insert_capture = Some(InsertCapture {
            command,
            start: super::cursor_tuple(ta),
        });
    }

    /// The text between two `(row, col)` positions, or nothing when they are the
    /// wrong way round.
    ///
    /// Was a hand-walked row loop; the engine answers it directly, and a span is
    /// checked against the text it came from rather than assumed to be in range.
    fn text_between(ta: &RopeBuffer, start: (usize, usize), end: (usize, usize)) -> String {
        if end <= start {
            return String::new();
        }
        ta.span_between(start, end)
            .and_then(|span| ta.text().slice(span))
            .map(|text| text.into_owned())
            .unwrap_or_default()
    }

    // ── Dot-repeat recording ─────────────────────────────────────────────────

    /// Record a completed mutating command in `last_change` (no inserted text).
    /// Called at every mutating, non-insert completion point.
    fn record(&mut self, command: Command) {
        self.last_change = Some(Change {
            command,
            inserted: None,
        });
    }

    // ── Paste p/P ────────────────────────────────────────────────────────────

    /// Returns `false` when the register is empty (nothing pasted).
    fn paste(&mut self, after: bool, count: usize, ta: &mut RopeBuffer) -> bool {
        // Borrow, don't clone — the body only mutates `ta`, never `self`,
        // so a large register isn't copied on every p/P.
        let Some(reg) = self.registers.read() else {
            return false;
        };
        let text = &reg.text;
        match reg.kind {
            RegisterKind::Linewise => {
                let body = text.strip_suffix('\n').unwrap_or(text);
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
                    let len = ta.row(row).map(|l| l.chars().count()).unwrap_or(col);
                    ta.jump_to(row, (col + 1).min(len));
                }
                for _ in 0..count.max(1) {
                    ta.insert_str(text);
                }
            }
        }
        true
    }

    // ── Text object helpers ──────────────────────────────────────────────────

    /// Returns `false` when no object exists at the cursor (vim no-op).
    fn apply_operator_object(
        &mut self,
        op: Operator,
        obj: TextObject,
        inserted: Option<&str>,
        ta: &mut RopeBuffer,
    ) -> bool {
        let Some((row, start, end)) = objects::object_range_at_cursor(ta, obj) else {
            return false;
        };
        Self::select_range(ta, (row, start), (row, end), false);
        if op == Operator::Change {
            ta.cut();
            self.fill_from_textarea(ta, RegisterKind::Charwise);
            self.finish_insert_entry(&Command::OperateObject(op, obj), inserted, ta);
        } else {
            self.apply_operator_on_selection(op, ta);
        }
        true
    }

    // ── Single-key edit helpers ──────────────────────────────────────────────

    /// Delete `count` chars at the cursor (`forward`: under-and-after, vim
    /// `x`; otherwise before, vim `X`), clamped to the current line — vim's
    /// x/X never join lines — filling the unnamed register with the deleted
    /// text (vim rule: every delete fills the register; `xp` swaps chars).
    /// Returns `false` when nothing was deleted (empty line, X at col 0).
    fn delete_chars(&mut self, forward: bool, count: usize, ta: &mut RopeBuffer) -> bool {
        let (row, col) = super::cursor_tuple(ta);
        // Borrow, don't clone: all reads of `line` finish before the first
        // mutation, so held-down x on a long line doesn't copy it each press.
        let Some(line) = ta.row(row) else {
            return false;
        };
        // `delete_next_char`/`delete_char` each remove a whole grapheme cluster,
        // so the count that bounds them has to be counted in clusters too.
        // Bounding by scalars let `3x` on a ZWJ emoji — three scalars, one
        // cluster — spend its two remaining steps past the end of the row,
        // joining the next row up and eating into it.
        use unicode_segmentation::UnicodeSegmentation;
        let split = line
            .char_indices()
            .nth(col)
            .map(|(byte, _)| byte)
            .unwrap_or(line.len());
        let (before, after) = line.split_at(split);
        let (n, deleted) = if forward {
            let n = count.min(after.graphemes(true).count());
            (n, after.graphemes(true).take(n).collect::<String>())
        } else {
            let available = before.graphemes(true).count();
            let n = count.min(available);
            (
                n,
                before
                    .graphemes(true)
                    .skip(available - n)
                    .collect::<String>(),
            )
        };
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
    fn replace_char(&mut self, c: char, ta: &mut RopeBuffer) -> VimKeyOutcome {
        if ta.delete_next_char() {
            ta.insert_char(c);
            ta.move_cursor(CursorMove::Back);
            VimKeyOutcome::TextMutated
        } else {
            VimKeyOutcome::NoOp
        }
    }

    /// Join the next line onto the current one. `spaced` (vim `J`): the next
    /// line's leading whitespace is stripped and a single space separates the
    /// parts (none when the current line is empty or already ends in
    /// whitespace), cursor left on the join point. Raw (`gJ`): the newline is
    /// removed verbatim.
    fn join_line(ta: &mut RopeBuffer, spaced: bool) {
        let (row, _) = super::cursor_tuple(ta);
        if row + 1 >= ta.row_count() {
            return;
        }
        let current = ta.row(row).unwrap_or_default();
        let cur_empty = current.is_empty();
        let cur_ends_ws = current.chars().last().is_some_and(|c| c.is_whitespace());
        drop(current);
        ta.move_cursor(CursorMove::End);
        ta.delete_next_char(); // removes the newline
        if !spaced {
            return;
        }
        let (r, c) = super::cursor_tuple(ta);
        let strip = ta
            .row(r)
            .unwrap_or_default()
            .chars()
            .skip(c)
            .take_while(|ch| ch.is_whitespace())
            .count();
        for _ in 0..strip {
            ta.delete_next_char();
        }
        let rest_nonempty = ta.row_len(r) > c;
        if !cur_empty && !cur_ends_ws && rest_nonempty {
            ta.insert_char(' ');
            ta.move_cursor(CursorMove::Back);
        }
    }

    /// Toggle the case of the char under the cursor and advance one char.
    fn toggle_case_at_cursor(ta: &mut RopeBuffer) {
        use unicode_segmentation::UnicodeSegmentation;

        let (row, col) = super::cursor_tuple(ta);
        // `delete_next_char` removes a whole grapheme cluster, so what goes back
        // has to be the whole cluster too. Reading one scalar and re-inserting
        // one scalar destroyed everything after the first: `~` on a decomposed
        // `é` dropped the combining acute, and on a ZWJ emoji collapsed the
        // sequence to its first character.
        let cluster = ta.row(row).and_then(|line| {
            line.chars()
                .skip(col)
                .collect::<String>()
                .graphemes(true)
                .next()
                .map(str::to_owned)
        });
        let Some(cluster) = cluster else {
            return;
        };
        // Case belongs to the base character; the marks that follow ride along
        // unchanged.
        let mut flipped = String::with_capacity(cluster.len());
        let mut scalars = cluster.chars();
        if let Some(base) = scalars.next() {
            flipped.push_str(&Self::flip_case(base));
        }
        flipped.extend(scalars);
        ta.delete_next_char();
        ta.insert_str(&flipped);
    }
}

#[cfg(test)]
#[path = "vim_tests.rs"]
mod tests;
