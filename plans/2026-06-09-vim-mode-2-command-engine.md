# Vim Mode — Plan 2: Command Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Grow the `VimEngine` skeleton (Plan 1) into a real editing engine: a reified command model with counts, motions, operators (`d c y`), register/paste, the common single-key edits, `f/F/t/T`, text objects, `%`, Visual/Visual-line modes, dot-repeat, and indent.

**Architecture:** Per `adr/0011`, keys parse into a reified `Command` value before anything mutates the buffer: a pending-state accumulator collects count/operator/find, resolves to a `Command`, and `apply` executes it over `&mut TextArea`. The **last mutating command** is stored and re-applied by `.`; insert edits are captured as the resulting **text delta**, not keystrokes. Operators are realized with the textarea's selection primitives (`start_selection` + motion + `cut`/`copy`). The unnamed register is the textarea's internal yank buffer (`yank_text`/`set_yank_text`) plus an engine `linewise` flag — kept separate from the OS clipboard, which stays on `Ctrl-c/v` in the existing direct path.

**Tech Stack:** Rust, `ratatui-textarea` 0.9.1 (`move_cursor`/`CursorMove`, `start_selection`/`selection_range`/`cancel_selection`, `cut`/`copy`/`paste`, `yank_text`/`set_yank_text`, `delete_str`, `undo`/`redo`, `insert_str`/`insert_newline`/`insert_tab`). Tests: `cargo test -p kimun-tui --lib text_editor::vim`.

**Prereq:** Plan 1 merged (`VimEngine`, `VimKeyOutcome`, `InputInterpreter`, dispatch in `handle_input`, `EditorMode`). Decisions: `adr/0011`, `adr/0012`.

**Out of scope (Plan 3):** `/ ? n N`, `:`→palette + Ex-aliases, `note.save`/`app.quit`, Space-leader, mouse→Visual. Out of scope entirely: named registers, marks, macros (deferred — but the dot-repeat machinery in Task 11 is built so macros are an additive extension, per adr/0011).

---

## File Structure

All work is in `tui/src/components/text_editor/vim.rs` unless noted. The engine stays **pure over `&mut TextArea`** — no component state, no async. Two new fields on `VimEngine` carry pending parse state and the dot-repeat record. The host (`mod.rs` dispatch from Plan 1) already maps `VimKeyOutcome` to bump helpers and is touched only where Visual selection must mirror into `self.selection` (Task 10).

Consider splitting `vim.rs` into a `vim/` directory if it passes ~600 lines: `vim/mod.rs` (engine + dispatch), `vim/motion.rs` (motion→`CursorMove` + range resolution), `vim/textobject.rs` (text-object ranges). Do this split lazily, only when the file grows unwieldy (follow the codebase's existing module conventions).

---

## Task 1: Command model + pending-state fields

Introduce the reified types and the accumulator fields. No behavior change yet — the existing single-key dispatch from Plan 1 keeps working; this lays the types the rest of the plan fills in.

**Files:** Modify `vim.rs`.

- [ ] **Step 1: Add the command/motion/operator/text-object types**

At the top of `vim.rs` (after existing imports), add:

```rust
/// A cursor motion. Operators consume a motion to form a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left, Right, Up, Down,
    WordForward, WordBack, WordEnd,
    LineStart, FirstNonBlank, LineEnd,
    FileStart, FileEnd,
    ParagraphForward, ParagraphBack,
    MatchingPair,                 // %
    FindChar { ch: char, till: bool, forward: bool }, // f/F/t/T
}

/// An operator awaiting a motion or text object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator { Delete, Change, Yank, Indent, Outdent }

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
    Move(Motion, usize),                       // motion, count
    OperateMotion(Operator, Motion, usize),    // e.g. 2dw
    OperateLine(Operator, usize),              // dd / cc / yy / >> with count
    OperateObject(Operator, TextObject),       // diw, ci"
    OperateToLineEnd(Operator),                // D / C / Y
    DeleteChar { forward: bool, count: usize },// x / X
    ReplaceChar(char),                         // r<ch>
    SubstituteChar(usize),                     // s
    SubstituteLine,                            // S
    JoinLines(usize),                          // J
    ToggleCase(usize),                         // ~
    Paste { after: bool, count: usize },       // p / P
    Undo(usize),
    Redo(usize),
}
```

- [ ] **Step 2: Add pending-state + dot-repeat fields to `VimEngine`**

Replace the `VimEngine` struct (from Plan 1) with:

```rust
#[derive(Debug)]
pub struct VimEngine {
    mode: EditorMode,
    pending_count: Option<usize>,
    pending_operator: Option<Operator>,
    pending_g: bool,                 // first key of `gg`
    pending_find: Option<PendingFind>,
    pending_replace: bool,           // awaiting the char after `r`
    pending_object_kind: Option<bool>, // Some(around): saw `i`/`a` after an operator
    last_find: Option<(char, bool, bool)>, // (ch, till, forward) for ; and ,
    register: RegisterKind,
    /// The last mutating command + captured insert delta, for `.` (adr/0011).
    last_change: Option<Change>,
    /// While in Insert via a vim command, the text typed is accumulated here
    /// (resulting delta) so `.` can replay it.
    insert_capture: Option<InsertCapture>,
}

#[derive(Debug, Clone, Copy)]
struct PendingFind { operator: Option<Operator>, till: bool, forward: bool }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterKind { Charwise, Linewise }

#[derive(Debug, Clone)]
struct Change { command: Command, inserted: Option<String> }

#[derive(Debug, Clone)]
struct InsertCapture { command: Command, start_len: usize, text: String }

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
```

- [ ] **Step 3: Build (types only, dispatch still the Plan 1 version)**

Run: `cargo build -p kimun-tui 2>&1 | tail -15`
Expected: clean, with `dead_code` warnings for the new fields/variants (acceptable — they're filled in next tasks). If warnings are denied in this crate, add `#[allow(dead_code)]` on the new items and remove it as each is used.

- [ ] **Step 4: Commit**

```bash
git add tui/src/components/text_editor/vim.rs
git commit -m "feat: reified vim Command model + pending-state fields"
```

---

## Task 2: Counts + a clearing helper

Accumulate leading digits into `pending_count` (with `0` as LineStart only when no count is pending), and add a `clear_pending` helper used after every completed command.

**Files:** Modify `vim.rs`.

- [ ] **Step 1: Write failing tests**

In `vim.rs` tests:

```rust
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
```

- [ ] **Step 2: Implement count handling + helpers**

Add to `impl VimEngine`:

```rust
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
```

Wire it at the top of `normal_char` (from Plan 1): before the big `match c`, add:

```rust
        if self.accumulate_count(c) {
            return VimKeyOutcome::NoOp; // consumed; awaiting the verb/motion
        }
```

Update the `'0'` arm and motions to use the count via `take_count()` — Task 3 generalizes motions, so leave the Plan-1 single-step motions for now (they ignore count until Task 3 replaces them).

- [ ] **Step 3: Run tests**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -15`
Expected: `count_accumulates_then_moves` FAILS (motions don't yet honor count) — that's expected; it passes after Task 3. `zero_without_count_is_line_start` PASSES.

Mark `count_accumulates_then_moves` with `#[ignore = "count honored in Task 3"]` for now, or proceed to Task 3 immediately and unignore. Recommended: proceed to Task 3 before committing.

- [ ] **Step 4: Commit (after Task 3 green) — see Task 3 Step 5.**

---

## Task 3: Motion resolution honoring counts

Replace Plan 1's ad-hoc motions with one `apply_motion(motion, count, ta)` that maps to `CursorMove`, repeated `count` times. Add `w/b/e`, `W/B/E` (WORD variants reuse word motions for v1), `0/^/$`, `gg/G`, `{/}`. (`f/F/t/T` and `%` arrive in Tasks 7 and 9.)

**Files:** Modify `vim.rs`.

- [ ] **Step 1: Implement `apply_motion`**

Add to `impl VimEngine`:

```rust
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
                Motion::MatchingPair => Self::match_pair(ta),       // implemented Task 9
                Motion::FindChar { ch, till, forward } => {
                    Self::find_char(ta, ch, till, forward);          // implemented Task 7
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
```

Add stubs so it compiles (filled in later tasks):

```rust
    fn match_pair(_ta: &mut TextArea<'static>) { /* Task 9 */ }
    fn find_char(_ta: &mut TextArea<'static>, _ch: char, _till: bool, _forward: bool) { /* Task 7 */ }
```

- [ ] **Step 2: Route Normal-mode motions through `apply_motion` with count**

Rewrite the motion arms of `normal_char` to build a `Motion` + count and call `apply_motion`. Replace the Plan-1 motion arms:

```rust
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
                return VimKeyOutcome::TextMutated;
            }
            self.apply_motion(m, count, ta);
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }
```

Handle `gg` (the `g` prefix): before the motion match, add:

```rust
        if c == 'g' {
            if self.pending_g {
                self.pending_g = false;
                let cnt = self.take_count();
                if let Some(op) = self.pending_operator.take() {
                    self.apply_operator_motion(op, Motion::FileStart, cnt, ta);
                    self.clear_pending();
                    return VimKeyOutcome::TextMutated;
                }
                self.apply_motion(Motion::FileStart, 1, ta);
                self.clear_pending();
                return VimKeyOutcome::CursorOnly;
            }
            self.pending_g = true;
            return VimKeyOutcome::NoOp;
        }
```

Add a temporary stub for the operator path so this compiles (Task 4 implements it):

```rust
    fn apply_operator_motion(&mut self, _op: Operator, m: Motion, count: usize, ta: &mut TextArea<'static>) {
        // Placeholder until Task 4: just move (no-op delete). Replaced in Task 4.
        self.apply_motion(m, count, ta);
    }
```

- [ ] **Step 3: Unignore the count test and add motion tests**

Remove the `#[ignore]` from `count_accumulates_then_moves`. Add:

```rust
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
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -20`
Expected: PASS (`count_accumulates_then_moves`, `gg_and_G_jump_file_ends`, plus Plan 1 tests).

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/text_editor/vim.rs
git commit -m "feat: count-aware motion resolution (w/b/e, gg/G, {/}, 0/^/$)"
```

---

## Task 4: Operators `d c y` via selection + register

Implement the operator framework: `d`/`c`/`y` set `pending_operator`; a following motion forms a range (selection → cut/copy); doubled (`dd`/`cc`/`yy`) operate linewise; `D`/`C`/`Y` to line end. `c` enters Insert after deleting. Yank/delete fill the register (textarea yank buffer + `linewise` flag).

**Files:** Modify `vim.rs`.

- [ ] **Step 1: Write failing tests**

```rust
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
```

(`yy_then_p_duplicates_line` also exercises Task 5's paste — implement Task 5 alongside or mark it `#[ignore]` until Task 5.)

- [ ] **Step 2: Implement the operator entry + range application**

Add the operator-entry handling in `normal_char` (before the motion match, after the `g` handling):

```rust
        let op = match c {
            'd' => Some(Operator::Delete),
            'c' => Some(Operator::Change),
            'y' => Some(Operator::Yank),
            _ => None,
        };
        if let Some(op) = op {
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
```

Replace the Task-3 stub `apply_operator_motion` with the real version, plus the helpers:

```rust
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
        self.apply_motion(m, count, ta);
        self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
    }

    fn apply_operator_linewise(&mut self, op: Operator, count: usize, ta: &mut TextArea<'static>) {
        ta.move_cursor(CursorMove::Head);
        ta.start_selection();
        for _ in 0..count.saturating_sub(1) {
            ta.move_cursor(CursorMove::Down);
        }
        ta.move_cursor(CursorMove::End);
        // extend through the trailing newline so the whole line(s) go
        ta.move_cursor(CursorMove::Forward);
        self.apply_operator_on_selection(op, RegisterKind::Linewise, ta);
        if op == Operator::Change {
            // cc leaves an empty line to type into
            ta.insert_newline();
            ta.move_cursor(CursorMove::Up);
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
```

Add a thin Insert-entry that also starts dot-capture (Task 11 reads `insert_capture`); for now a minimal version:

```rust
    fn enter_insert_capture(&mut self, command: Command) {
        self.mode = EditorMode::Insert;
        self.insert_capture = Some(InsertCapture {
            command,
            start_len: 0,
            text: String::new(),
        });
    }
```

- [ ] **Step 3: Run tests (paste test may be ignored until Task 5)**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -25`
Expected: `dw`, `dd`, `cw` PASS; `yy_then_p` PASS only after Task 5.

- [ ] **Step 4: Commit**

```bash
git add tui/src/components/text_editor/vim.rs
git commit -m "feat: vim operators d/c/y + dd/cc/yy + D/C/Y"
```

---

## Task 5: Register paste `p` / `P`

Insert the register contents: charwise pastes after (`p`) / before (`P`) the cursor; linewise opens a new line below/above. Uses the textarea yank buffer (set by Task 4) and the `linewise` flag.

**Files:** Modify `vim.rs`.

- [ ] **Step 1: Implement `p`/`P` in `normal_char`**

Add arms (before the fallthrough `_ => NoOp`):

```rust
        if c == 'p' || c == 'P' {
            let after = c == 'p';
            let cnt = self.take_count();
            self.paste(after, cnt, ta);
            self.clear_pending();
            return VimKeyOutcome::TextMutated;
        }
```

Add the method:

```rust
    fn paste(&mut self, after: bool, count: usize, ta: &mut TextArea<'static>) {
        let text = ta.yank_text();
        if text.is_empty() {
            return;
        }
        match self.register {
            RegisterKind::Linewise => {
                if after {
                    ta.move_cursor(CursorMove::End);
                    ta.insert_newline();
                } else {
                    ta.move_cursor(CursorMove::Head);
                    ta.insert_newline();
                    ta.move_cursor(CursorMove::Up);
                }
                let body = text.strip_suffix('\n').unwrap_or(&text);
                for i in 0..count.max(1) {
                    if i > 0 {
                        ta.insert_newline();
                    }
                    ta.insert_str(body);
                    ta.move_cursor(CursorMove::Head);
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
```

- [ ] **Step 2: Unignore `yy_then_p_duplicates_line` and add a charwise paste test**

```rust
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -20`
Expected: PASS (`yy_then_p_duplicates_line`, `charwise_p_pastes_after_cursor`).

- [ ] **Step 4: Commit**

```bash
git add tui/src/components/text_editor/vim.rs
git commit -m "feat: vim paste p/P (charwise + linewise register)"
```

---

## Task 6: Single-key edits `x X s S r J ~` + undo/redo

**Files:** Modify `vim.rs`.

- [ ] **Step 1: Write failing tests**

```rust
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
```

- [ ] **Step 2: Implement the edits**

Add arms in `normal_char` (before fallthrough). For `r`, set `pending_replace` and consume the next key at the top of `handle_normal`:

At the very top of `handle_normal` (before the `plain` computation), add:

```rust
        if self.pending_replace {
            self.pending_replace = false;
            if let KeyCode::Char(c) = key.code {
                return self.replace_char(c, ta);
            }
            return VimKeyOutcome::NoOp; // Esc etc cancels
        }
```

Arms in `normal_char`:

```rust
        match c {
            'x' => {
                let cnt = self.take_count();
                for _ in 0..cnt { ta.delete_next_char(); }
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            'X' => {
                let cnt = self.take_count();
                for _ in 0..cnt { ta.delete_char(); }
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            'r' => { self.pending_replace = true; return VimKeyOutcome::NoOp; }
            's' => {
                let cnt = self.take_count();
                for _ in 0..cnt { ta.delete_next_char(); }
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
                for _ in 0..cnt { Self::join_line(ta); }
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            '~' => {
                let cnt = self.take_count();
                for _ in 0..cnt { Self::toggle_case_at_cursor(ta); }
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            'u' => {
                let cnt = self.take_count();
                for _ in 0..cnt { ta.undo(); }
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            _ => {}
        }
```

Add the helpers:

```rust
    fn replace_char(&mut self, c: char, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        if ta.delete_next_char() {
            ta.insert_char(c);
            ta.move_cursor(CursorMove::Back);
        }
        self.last_change = Some(Change { command: Command::ReplaceChar(c), inserted: None });
        VimKeyOutcome::TextMutated
    }

    fn join_line(ta: &mut TextArea<'static>) {
        ta.move_cursor(CursorMove::End);
        ta.delete_next_char(); // removes the newline, joining the next line up
    }

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
```

For `Ctrl-r` redo, handle it in `handle_normal` before the `plain` filter:

```rust
        if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let cnt = self.take_count();
            for _ in 0..cnt { ta.redo(); }
            self.clear_pending();
            return VimKeyOutcome::TextMutated;
        }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -20`
Expected: PASS (`x`, `r`, `u`, plus prior).

- [ ] **Step 4: Commit**

```bash
git add tui/src/components/text_editor/vim.rs
git commit -m "feat: vim edits x/X/s/S/r/J/~ + u/Ctrl-r"
```

---

## Task 7: Find-char `f F t T` + `;` `,`

A pending-char state: after `f/F/t/T`, the next char is the target. `t/T` stop one short. `;`/`,` repeat the last find (same/opposite direction). Works standalone and as an operator target (`df,`).

**Files:** Modify `vim.rs`.

- [ ] **Step 1: Write failing tests**

```rust
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
```

- [ ] **Step 2: Implement pending-find**

At the top of `handle_normal` (after the `pending_replace` block), add:

```rust
        if let Some(pf) = self.pending_find.take() {
            if let KeyCode::Char(ch) = key.code {
                self.last_find = Some((ch, pf.till, pf.forward));
                let motion = Motion::FindChar { ch, till: pf.till, forward: pf.forward };
                let cnt = self.take_count();
                if let Some(op) = pf.operator {
                    ta.start_selection();
                    self.apply_motion(motion, cnt, ta);
                    // f is inclusive of the target: extend one more for delete
                    if !pf.till && pf.forward { ta.move_cursor(CursorMove::Forward); }
                    self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
                    self.clear_pending();
                    return self.outcome_for(op);
                }
                self.apply_motion(motion, cnt, ta);
                self.clear_pending();
                return VimKeyOutcome::CursorOnly;
            }
            return VimKeyOutcome::NoOp; // non-char cancels
        }
```

In `normal_char`, add the `f/F/t/T` and `;`/`,` arms (before fallthrough). They capture the pending operator so `df,` threads it:

```rust
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
        if c == ';' || c == ',' {
            if let Some((ch, till, fwd)) = self.last_find {
                let forward = if c == ';' { fwd } else { !fwd };
                let cnt = self.take_count();
                self.apply_motion(Motion::FindChar { ch, till, forward }, cnt, ta);
            }
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }
```

Replace the Task-3 `find_char` stub with the implementation:

```rust
    fn find_char(ta: &mut TextArea<'static>, ch: char, till: bool, forward: bool) {
        let (row, col) = super::cursor_tuple(ta);
        let Some(line) = ta.lines().get(row).cloned() else { return };
        let chars: Vec<char> = line.chars().collect();
        if forward {
            let start = col + 1;
            if let Some(pos) = (start..chars.len()).find(|&i| chars[i] == ch) {
                let target = if till { pos.saturating_sub(1) } else { pos };
                for _ in col..target { ta.move_cursor(CursorMove::Forward); }
            }
        } else {
            if let Some(pos) = (0..col).rev().find(|&i| chars[i] == ch) {
                let target = if till { pos + 1 } else { pos };
                for _ in target..col { ta.move_cursor(CursorMove::Back); }
            }
        }
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -20`
Expected: PASS (`f_moves_to_char`, `df_deletes_through_char`).

- [ ] **Step 4: Commit**

```bash
git add tui/src/components/text_editor/vim.rs
git commit -m "feat: vim find-char f/F/t/T + ;/,"
```

---

## Task 8: Text objects `iw aw i" a" i( a(` …

After an operator, `i`/`a` + an object char selects a computed range. Pairs: `( ) b`, `{ } B`, `[ ]`, `< >`; quotes `" ' \``; word `w`.

**Files:** Modify `vim.rs`.

- [ ] **Step 1: Write failing tests**

```rust
    #[test]
    fn diw_deletes_inner_word() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar baz"]);
        // cursor on 'b' of bar
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
        // move onto the text inside quotes
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('h'), &mut t);
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('i'), &mut t);
        e.handle_key(&key('"'), &mut t);
        assert_eq!(t.lines(), &["say \"\" now"]);
        assert_eq!(*e.mode(), EditorMode::Insert);
    }
```

- [ ] **Step 2: Implement object parsing + range selection**

In `handle_normal`, after the find-pending block, add object-pending handling. When an operator is pending and `i`/`a` is pressed, set `pending_object_kind`; the next key is the object char:

In `normal_char`, before the insert-entry (`i`/`a`) handling, branch on a pending operator:

```rust
        if self.pending_operator.is_some() {
            if c == 'i' || c == 'a' {
                self.pending_object_kind = Some(c == 'a');
                return VimKeyOutcome::NoOp;
            }
            if let Some(around) = self.pending_object_kind.take() {
                if let Some(obj) = Self::object_for_char(c, around) {
                    let op = self.pending_operator.take().unwrap();
                    self.apply_operator_object(op, obj, ta);
                    self.clear_pending();
                    return self.outcome_for(op);
                }
            }
        }
```

(Place this block **above** the generic `i`/`a` insert-entry arms so `di`/`ci` aren't intercepted as insert-entry.)

Add the helpers:

```rust
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

    fn apply_operator_object(&mut self, op: Operator, obj: TextObject, ta: &mut TextArea<'static>) {
        let (row, col) = super::cursor_tuple(ta);
        let Some(line) = ta.lines().get(row).cloned() else { return };
        let chars: Vec<char> = line.chars().collect();
        let Some((start, end)) = Self::object_range(&chars, col, obj) else { return };
        // select [start, end) on this row via Jump
        ta.move_cursor(CursorMove::Jump(row as u16, start as u16));
        ta.start_selection();
        ta.move_cursor(CursorMove::Jump(row as u16, end as u16));
        self.apply_operator_on_selection(op, RegisterKind::Charwise, ta);
    }

    /// Returns the half-open `[start, end)` char range on a single line.
    fn object_range(chars: &[char], col: usize, obj: TextObject) -> Option<(usize, usize)> {
        match obj {
            TextObject::Word { around } => {
                let is_word = |c: char| c.is_alphanumeric() || c == '_';
                let mut s = col;
                while s > 0 && is_word(chars[s - 1]) { s -= 1; }
                let mut e = col;
                while e < chars.len() && is_word(chars[e]) { e += 1; }
                if around {
                    while e < chars.len() && chars[e].is_whitespace() { e += 1; }
                }
                Some((s, e))
            }
            TextObject::Quote { ch, around } => {
                let positions: Vec<usize> =
                    chars.iter().enumerate().filter(|(_, &c)| c == ch).map(|(i, _)| i).collect();
                // find the pair surrounding/after the cursor
                let pair = positions.chunks(2).find(|p| p.len() == 2 && p[1] >= col)?;
                let (o, c) = (pair[0], pair[1]);
                if around { Some((o, c + 1)) } else { Some((o + 1, c)) }
            }
            TextObject::Pair { open, close, around } => {
                let o = (0..=col).rev().find(|&i| chars[i] == open)?;
                let c = (col..chars.len()).find(|&i| chars[i] == close)?;
                if around { Some((o, c + 1)) } else { Some((o + 1, c)) }
            }
        }
    }
```

Note: v1 text objects are **single-line** (matches a notes editor's common case). Multi-line pairs are a later enhancement; document this limitation in a code comment.

- [ ] **Step 3: Run tests**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -20`
Expected: PASS (`diw_deletes_inner_word`, `ci_quote_changes_inside_quotes`).

- [ ] **Step 4: Commit**

```bash
git add tui/src/components/text_editor/vim.rs
git commit -m "feat: vim text objects iw/aw/i\"/a\"/i(/a( (single-line)"
```

---

## Task 9: Matching-pair jump `%`

**Files:** Modify `vim.rs`.

- [ ] **Step 1: Write failing test**

```rust
    #[test]
    fn percent_jumps_to_matching_paren() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo(bar)baz"]);
        e.handle_key(&key('f'), &mut t);
        e.handle_key(&key('('), &mut t);
        e.handle_key(&key('%'), &mut t);
        assert_eq!(super::super::cursor_tuple(&t), (0, 7));
    }
```

- [ ] **Step 2: Replace the Task-3 `match_pair` stub**

```rust
    fn match_pair(ta: &mut TextArea<'static>) {
        let (row, col) = super::cursor_tuple(ta);
        let Some(line) = ta.lines().get(row).cloned() else { return };
        let chars: Vec<char> = line.chars().collect();
        let Some(&here) = chars.get(col) else { return };
        let pairs = [('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')];
        // open → search forward; close → search backward
        if let Some(&(_, close)) = pairs.iter().find(|&&(o, _)| o == here) {
            let mut depth = 0i32;
            for i in col..chars.len() {
                if chars[i] == here { depth += 1; }
                else if chars[i] == close { depth -= 1; if depth == 0 {
                    for _ in col..i { ta.move_cursor(CursorMove::Forward); }
                    return;
                }}
            }
        } else if let Some(&(open, _)) = pairs.iter().find(|&&(_, c)| c == here) {
            let mut depth = 0i32;
            for i in (0..=col).rev() {
                if chars[i] == here { depth += 1; }
                else if chars[i] == open { depth -= 1; if depth == 0 {
                    for _ in i..col { ta.move_cursor(CursorMove::Back); }
                    return;
                }}
            }
        }
    }
```

- [ ] **Step 3: Run test + commit**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -15`
Expected: PASS.

```bash
git add tui/src/components/text_editor/vim.rs
git commit -m "feat: vim % matching-pair jump (single-line)"
```

---

## Task 10: Visual + Visual-line modes

`v`/`V` enter Visual/Visual-line. Motions extend the selection; `o` swaps the active end; `d c y x` operate on it; `>`/`<` indent (Task 13). A pair char wraps the selection via auto-surround — but that is handled in the **host** (the existing direct-path auto-surround), reached because in Visual mode a bare pair char returns `PassThrough` only when nothing is pending (see adr decision, Q11). The engine mirrors its selection so the host renders it (`self.selection`).

**Files:** Modify `vim.rs`; modify `mod.rs` (mirror selection after engine calls in Visual mode).

- [ ] **Step 1: Write failing tests**

```rust
    #[test]
    fn v_motion_d_deletes_selection() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["hello"]);
        e.handle_key(&key('v'), &mut t);
        e.handle_key(&key('l'), &mut t); // extend over 'h','e'
        e.handle_key(&key('l'), &mut t);
        e.handle_key(&key('d'), &mut t);
        assert_eq!(t.lines(), &["lo"]);
        assert_eq!(*e.mode(), EditorMode::Normal);
    }

    #[test]
    fn V_then_d_deletes_line() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["one", "two"]);
        e.handle_key(&key('V'), &mut t);
        e.handle_key(&key('d'), &mut t);
        assert_eq!(t.lines(), &["two"]);
    }
```

- [ ] **Step 2: Implement Visual handling**

Add `v`/`V` arms to `normal_char`:

```rust
        if c == 'v' {
            ta.start_selection();
            self.mode = EditorMode::Visual;
            return VimKeyOutcome::CursorOnly;
        }
        if c == 'V' {
            ta.move_cursor(CursorMove::Head);
            ta.start_selection();
            ta.move_cursor(CursorMove::End);
            self.mode = EditorMode::VisualLine;
            return VimKeyOutcome::CursorOnly;
        }
```

Add a `handle_visual` dispatch and route to it from `handle_key`:

```rust
    pub fn handle_key(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        match self.mode {
            EditorMode::Insert => self.handle_insert(key, ta),
            EditorMode::Visual | EditorMode::VisualLine => self.handle_visual(key, ta),
            _ => self.handle_normal(key, ta),
        }
    }

    fn handle_visual(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        if key.code == KeyCode::Esc {
            ta.cancel_selection();
            self.mode = EditorMode::Normal;
            return VimKeyOutcome::CursorOnly;
        }
        let plain = key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT;
        let KeyCode::Char(c) = key.code else {
            // arrows extend selection
            match key.code {
                KeyCode::Left => { ta.move_cursor(CursorMove::Back); return VimKeyOutcome::CursorOnly; }
                KeyCode::Right => { ta.move_cursor(CursorMove::Forward); return VimKeyOutcome::CursorOnly; }
                KeyCode::Up => { ta.move_cursor(CursorMove::Up); return VimKeyOutcome::CursorOnly; }
                KeyCode::Down => { ta.move_cursor(CursorMove::Down); return VimKeyOutcome::CursorOnly; }
                _ => return VimKeyOutcome::NoOp,
            }
        };
        if !plain { return VimKeyOutcome::NoOp; }
        if self.accumulate_count(c) { return VimKeyOutcome::NoOp; }
        // operators on the live selection
        let op = match c {
            'd' | 'x' => Some(Operator::Delete),
            'c' | 's' => Some(Operator::Change),
            'y' => Some(Operator::Yank),
            _ => None,
        };
        if let Some(op) = op {
            let kind = if self.mode == EditorMode::VisualLine {
                RegisterKind::Linewise
            } else {
                RegisterKind::Charwise
            };
            self.apply_operator_on_selection(op, kind, ta);
            self.mode = if op == Operator::Change { EditorMode::Insert } else { EditorMode::Normal };
            self.clear_pending();
            return self.outcome_for(op);
        }
        if c == 'o' {
            // swap selection end: re-anchor (textarea has no direct swap; restart)
            // Minimal v1: cancel + restart from current pos is wrong; instead
            // move is enough for charwise — leave as a documented no-op for v1.
            return VimKeyOutcome::NoOp;
        }
        // auto-surround pair chars: defer to host direct path (Q11) only when
        // nothing is pending — selection is live, host wraps it.
        if matches!(c, '(' | '[' | '{' | '<' | '"' | '\'' | '`' | '*' | '_' | '~') {
            self.mode = EditorMode::Normal; // host wraps; we leave visual
            return VimKeyOutcome::PassThrough;
        }
        // a motion extends the selection
        let count = self.pending_count.unwrap_or(1);
        let motion = match c {
            'h' => Some(Motion::Left), 'l' => Some(Motion::Right),
            'k' => Some(Motion::Up), 'j' => Some(Motion::Down),
            'w' | 'W' => Some(Motion::WordForward), 'b' | 'B' => Some(Motion::WordBack),
            'e' | 'E' => Some(Motion::WordEnd),
            '0' => Some(Motion::LineStart), '^' => Some(Motion::FirstNonBlank),
            '$' => Some(Motion::LineEnd), 'G' => Some(Motion::FileEnd),
            '{' => Some(Motion::ParagraphBack), '}' => Some(Motion::ParagraphForward),
            '%' => Some(Motion::MatchingPair),
            _ => None,
        };
        if let Some(m) = motion {
            self.apply_motion(m, count, ta);
            self.clear_pending();
            return VimKeyOutcome::CursorOnly;
        }
        VimKeyOutcome::NoOp
    }
```

- [ ] **Step 3: Mirror Visual selection into `self.selection` in `mod.rs`**

In `mod.rs` `handle_input`, in the `VimKeyOutcome::CursorOnly` arm (from Plan 1 Task 4), update `self.selection` from the textarea each call so Visual renders:

```rust
                        VimKeyOutcome::CursorOnly => {
                            self.selection = self
                                .backend
                                .as_textarea()
                                .and_then(|ta| ta.selection_range());
                            self.refresh_autocomplete_if_open();
                            self.edit_generation = self.edit_generation.wrapping_add(1);
                            return EventState::Consumed;
                        }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p kimun-tui --lib text_editor 2>&1 | tail -20`
Expected: PASS (`v_motion_d_deletes_selection`, `V_then_d_deletes_line`).

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/text_editor/
git commit -m "feat: vim Visual + Visual-line modes"
```

---

## Task 11: Dot-repeat `.`

Record the last mutating command + captured insert delta; `.` re-applies it. Insert capture: when entering Insert via a vim command, snapshot; on `Esc`, diff to get the resulting delta and store it in `last_change`.

**Files:** Modify `vim.rs`.

- [ ] **Step 1: Write failing tests**

```rust
    #[test]
    fn dot_repeats_x() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["abcdef"]);
        e.handle_key(&key('x'), &mut t);
        e.handle_key(&key('.'), &mut t);
        assert_eq!(t.lines(), &["cdef"]);
    }

    #[test]
    fn dot_repeats_change_with_typed_text() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["foo bar"]);
        // cw -> type "X" -> Esc, then move to next word and dot
        e.handle_key(&key('c'), &mut t);
        e.handle_key(&key('w'), &mut t);
        // simulate insert-mode typing via the host pass-through path:
        e.note_inserted_text(&mut t, "X");
        e.handle_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), &mut t);
        e.handle_key(&key('w'), &mut t);
        e.handle_key(&key('.'), &mut t);
        assert_eq!(t.lines(), &["X X"]);
    }
```

- [ ] **Step 2: Record `last_change` on mutating commands**

Wherever a mutating, non-insert command completes (`x`, `X`, `J`, `~`, `p`, operator-motion, operator-line, operator-object, `r`), set `self.last_change`. Add a small helper and call it at those return points:

```rust
    fn record(&mut self, command: Command) {
        self.last_change = Some(Change { command, inserted: None });
    }
```

For example in the `x` arm: before `return VimKeyOutcome::TextMutated;`, add `self.record(Command::DeleteChar { forward: true, count: cnt });`. Do the equivalent for each mutating arm with its matching `Command` variant.

- [ ] **Step 3: Capture insert deltas**

Add a host-callable hook the dispatch invokes when an inserted character reaches the textarea via PassThrough. In `mod.rs`, after the direct path runs for a vim Insert keystroke, call `engine.note_inserted_textarea(ta)` — simplest: capture by buffer length diff. Implement on the engine:

```rust
    /// Called by the host after Insert-mode pass-through edits, with the
    /// current buffer text, so the engine can accumulate the insert delta.
    pub fn note_inserted_text(&mut self, ta: &TextArea<'static>, _typed: &str) {
        if let Some(cap) = self.insert_capture.as_mut() {
            // Re-derive the delta as the text on the current line from the
            // change start; v1 captures single-line inserts (notes editing).
            let (row, col) = super::cursor_tuple(ta);
            if let Some(line) = ta.lines().get(row) {
                let start = col.saturating_sub(cap.text.chars().count());
                cap.text = line.chars().skip(start).take(col - start).collect();
            }
        }
    }
```

(For the test, `note_inserted_text` is called directly with the typed text; in the app, the host calls it after each Insert pass-through. The robust delta is computed on `Esc`, Step 4.)

- [ ] **Step 4: Finalize capture on `Esc`**

In `handle_insert`, on `Esc`, fold the capture into `last_change`:

```rust
    fn handle_insert(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        if key.code == KeyCode::Esc {
            self.mode = EditorMode::Normal;
            if let Some(cap) = self.insert_capture.take() {
                self.last_change = Some(Change {
                    command: cap.command,
                    inserted: Some(cap.text),
                });
            }
            if super::cursor_tuple(ta).1 > 0 {
                ta.move_cursor(CursorMove::Back);
            }
            return VimKeyOutcome::CursorOnly;
        }
        VimKeyOutcome::PassThrough
    }
```

- [ ] **Step 5: Implement `.`**

Add to `normal_char` (before fallthrough):

```rust
        if c == '.' {
            if let Some(change) = self.last_change.clone() {
                self.replay(change, ta);
            }
            self.clear_pending();
            return VimKeyOutcome::TextMutated;
        }
```

Add `replay`, dispatching on the stored `Command` and re-inserting `inserted` for change/substitute commands:

```rust
    fn replay(&mut self, change: Change, ta: &mut TextArea<'static>) {
        match change.command {
            Command::DeleteChar { forward, count } => {
                for _ in 0..count {
                    if forward { ta.delete_next_char(); } else { ta.delete_char(); }
                }
            }
            Command::OperateMotion(op, m, count) => {
                self.apply_operator_motion(op, m, count, ta);
                if let (Operator::Change, Some(text)) = (op, &change.inserted) {
                    ta.insert_str(text);
                    self.mode = EditorMode::Normal;
                }
            }
            Command::OperateObject(op, obj) => {
                self.apply_operator_object(op, obj, ta);
                if let (Operator::Change, Some(text)) = (op, &change.inserted) {
                    ta.insert_str(text);
                    self.mode = EditorMode::Normal;
                }
            }
            Command::ReplaceChar(ch) => { self.replace_char(ch, ta); }
            Command::JoinLines(n) => { for _ in 0..n.max(1) { Self::join_line(ta); } }
            Command::ToggleCase(n) => { for _ in 0..n { Self::toggle_case_at_cursor(ta); } }
            Command::Paste { after, count } => { self.paste(after, count, ta); }
            other => { let _ = other; /* OperateLine/SubstituteLine etc: v1 best-effort */ }
        }
    }
```

For `OperateMotion`/`OperateObject` with `Change`, ensure `apply_operator_on_selection` does **not** re-enter Insert during replay — guard `enter_insert_capture` so replay inserts directly. Simplest: in `replay`, the `Change` branch inserts text itself; suppress the engine's insert-mode entry by checking a `replaying` flag. Add `replaying: bool` to the engine, set it around `replay`, and in `apply_operator_on_selection`'s `Change` arm: `if !self.replaying { self.enter_insert_capture(...) } else { /* caller re-inserts */ }`.

- [ ] **Step 6: Run tests**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -20`
Expected: PASS (`dot_repeats_x`, `dot_repeats_change_with_typed_text`).

- [ ] **Step 7: Wire the host capture call**

In `mod.rs` `handle_input`, after the direct path runs for a key while the backend is vim-in-insert, call the capture. After the `let result = self.handle_textarea_key(key, tx);` block (Plan 1 Task 4 fall-through), add:

```rust
                if self.content_revision != text_rev_before {
                    if let Some(ta) = self.backend.as_textarea() {
                        let snapshot_ta = ta; // borrow for capture
                        // capture insert delta for dot-repeat
                    }
                }
```

Concretely, expose an engine accessor on the backend and call it:

```rust
    // in backend.rs:
    pub fn vim_note_insert(&mut self) {
        if let BackendState::Textarea(TextareaBackend { ta, input: InputInterpreter::Vim(e) }) = self {
            e.note_inserted_text(ta, "");
        }
    }
```

and in `mod.rs` after the content changed on the direct path: `self.backend.vim_note_insert();`.

- [ ] **Step 8: Run full suite + commit**

Run: `cargo test -p kimun-tui --lib text_editor 2>&1 | tail -20`
Expected: PASS.

```bash
git add tui/src/components/text_editor/
git commit -m "feat: vim dot-repeat (.) with insert-delta capture"
```

---

## Task 12: Pending-command footer hint + indent `> <`

Two small finishers: show the in-progress sequence (count/operator/find/`g`) in the footer, and implement indent/outdent in Normal (`>>`/`<<`) and Visual.

**Files:** Modify `vim.rs` (pending string, indent), `backend.rs` (expose pending), `mod.rs` (footer).

- [ ] **Step 1: Pending-command string**

Add to `impl VimEngine`:

```rust
    /// The in-progress command sequence, for the footer hint (e.g. "2d", "f").
    pub fn pending_hint(&self) -> Option<String> {
        let mut s = String::new();
        if let Some(n) = self.pending_count { s.push_str(&n.to_string()); }
        if let Some(op) = self.pending_operator {
            s.push(match op {
                Operator::Delete => 'd', Operator::Change => 'c', Operator::Yank => 'y',
                Operator::Indent => '>', Operator::Outdent => '<',
            });
        }
        if self.pending_g { s.push('g'); }
        if self.pending_replace { s.push('r'); }
        if self.pending_find.is_some() { s.push('f'); }
        if s.is_empty() { None } else { Some(s) }
    }
```

- [ ] **Step 2: Expose via backend + show in footer**

In `backend.rs`:

```rust
    pub fn vim_pending_hint(&self) -> Option<String> {
        match self {
            BackendState::Textarea(TextareaBackend { input: InputInterpreter::Vim(e), .. }) => e.pending_hint(),
            _ => None,
        }
    }
```

In `mod.rs` `hint_shortcuts`, when a `mode_label` is present, append the pending hint to the label cell:

```rust
        if let Some(mut label) = self.backend.mode_label() {
            if let Some(p) = self.backend.vim_pending_hint() {
                label = format!("{label}  {p}");
            }
            let mut hints = vec![(String::new(), label)];
            // …rest unchanged…
```

- [ ] **Step 3: Implement indent operators**

Add `>`/`<` handling in `normal_char` (doubled `>>`/`<<`) and in `handle_visual`. Use the textarea's tab insert / line-head logic. Minimal Normal-mode implementation:

```rust
        if c == '>' || c == '<' {
            let outdent = c == '<';
            if (outdent && self.pending_operator == Some(Operator::Outdent))
                || (!outdent && self.pending_operator == Some(Operator::Indent))
            {
                let cnt = self.take_count();
                self.indent_lines(outdent, cnt, ta);
                self.clear_pending();
                return VimKeyOutcome::TextMutated;
            }
            self.pending_operator = Some(if outdent { Operator::Outdent } else { Operator::Indent });
            return VimKeyOutcome::NoOp;
        }
```

```rust
    fn indent_lines(&self, outdent: bool, count: usize, ta: &mut TextArea<'static>) {
        for _ in 0..count.max(1) {
            ta.move_cursor(CursorMove::Head);
            if outdent {
                // remove up to 4 leading spaces
                let (row, _) = super::cursor_tuple(ta);
                let n = ta.lines().get(row)
                    .map(|l| l.chars().take(4).take_while(|c| *c == ' ').count())
                    .unwrap_or(0);
                for _ in 0..n { ta.delete_next_char(); }
            } else {
                ta.insert_str("    ");
            }
            ta.move_cursor(CursorMove::Down);
        }
    }
```

In `handle_visual`, add a `>`/`<` arm that calls `indent_lines(outdent, line_count, ta)` over the selected lines then returns to Normal.

- [ ] **Step 4: Tests + run**

```rust
    #[test]
    fn indent_line_adds_spaces() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["x"]);
        e.handle_key(&key('>'), &mut t);
        e.handle_key(&key('>'), &mut t);
        assert_eq!(t.lines(), &["    x"]);
    }
```

Run: `cargo test -p kimun-tui --lib text_editor 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/text_editor/
git commit -m "feat: vim pending-command footer hint + indent >>/<<"
```

---

## Self-Review

**Spec coverage (Plan 2 scope):** counts (T2/T3), motions w/b/e/0/^/$/gg/G/{/}/% (T3/T9), operators d/c/y + dd/cc/yy + D/C/Y (T4), register paste p/P (T5), x/X/s/S/r/J/~/u/Ctrl-r (T6), f/F/t/T/;/, (T7), text objects (T8), Visual/Visual-line (T10), dot-repeat (T11), pending hint + indent (T12). Auto-surround-in-Visual deferred to host (Plan 3 confirms the PassThrough path). ✅

**Deferred (correct):** named registers, marks, macros (T11 leaves the door open per adr/0011); multi-line text objects / multi-line `%` (documented single-line limitation in T8/T9).

**Placeholder scan:** The Visual `o` swap is a documented v1 no-op (T10) — flagged, not a hidden TODO. Insert-delta capture (T11) is single-line by design, stated. Task 2 Step 3 deliberately leaves one test temporarily failing with explicit instruction to proceed to Task 3 — not a placeholder, a sequencing note.

**Type consistency:** `Command`, `Motion`, `Operator`, `TextObject`, `RegisterKind`, `Change`, `InsertCapture` defined T1, used consistently T3–T11. `apply_motion`, `apply_operator_motion`, `apply_operator_on_selection`, `apply_operator_object`, `apply_operator_linewise`, `enter_insert_capture`, `record`, `replay`, `paste`, `find_char`, `match_pair`, `object_range` — each defined once, referenced thereafter. `VimKeyOutcome` (Plan 1) consumed unchanged. `super::cursor_tuple` used throughout.

**Implementer verification points:** confirm `CursorMove::Jump(u16,u16)` clamps as expected for `object_range`/Visual (test on multi-byte lines); confirm `TextArea::cut()`/`copy()` set the yank buffer read by `yank_text()` (they do in 0.9.1); the `replaying` flag (T11 Step 5) must be added to the struct + `Default` + `clear` — add it when implementing T11.
