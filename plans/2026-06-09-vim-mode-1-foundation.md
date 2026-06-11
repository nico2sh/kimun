# Vim Mode — Plan 1: Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a built-in `vim` editor backend skeleton — a modal state machine (Normal/Insert) over the existing `TextArea`, selectable via `editor_backend = "vim"`, with insert-mode delegating to the full textarea feature set, plus mode indication (footer label + cursor shape).

**Architecture:** Vim mode is the *same `TextArea` storage with a different input interpreter*, not a new top-level `BackendState` variant (see `adr/0012`). `BackendState::Textarea(TextArea)` becomes `BackendState::Textarea(TextareaBackend { ta, input })` where `input: InputInterpreter` is `Direct` (today's behavior) or `Vim(VimEngine)`. Accessors (`as_textarea`/`as_textarea_mut`) return the inner `TextArea` for both, so every textarea feature works in vim mode for free; only key dispatch branches on the interpreter. Insert mode falls through to the existing `handle_textarea_key`; Normal mode runs the engine. This plan delivers the skeleton: mode transitions + cursor motions + insert entry. Operators, counts, text objects, dot-repeat, visual mode, and the command-line family come in Plans 2 and 3.

**Tech Stack:** Rust, `ratatui` 0.30, `ratatui-textarea` 0.9.1, `crossterm` 0.29, `serde`/TOML. Tests via `cargo test -p kimun-tui`.

**Scope note:** This is Plan 1 of 3. Plan 2 = command engine (reified `Command` model, operators/motions/counts/text-objects/dot-repeat/`f t`/`%`/visual). Plan 3 = command-line + leader (`/ ? n N`, `:` palette + Ex-aliases, `note.save`/`app.quit` actions, Space-leader).

**Decisions referenced:** `adr/0011` (reified command model — lands in Plan 2), `adr/0012` (vim wraps textarea via `InputInterpreter`), `CONTEXT.md` (`Editor backend`, `Editing mode`).

---

## File Structure

- `tui/src/components/text_editor/snapshot.rs` — rename `NvimMode` → `EditorMode`; keep `from_nvim_str` as an nvim-only constructor. (modify)
- `tui/src/components/text_editor/backend.rs` — introduce `TextareaBackend` + `InputInterpreter`; rewrite `BackendState::Textarea` arm and the enum methods/accessors; `from_settings` builds the vim interpreter; add `mode_label()`. (modify)
- `tui/src/components/text_editor/vim.rs` — **new** — `VimEngine`, `VimKeyOutcome`; pure over `&mut TextArea`. (create)
- `tui/src/components/text_editor/mod.rs` — update the 9 direct `BackendState::Textarea(` match sites to `.ta`; wire the vim dispatch branch into `handle_input`; generalize the footer label in `hint_shortcuts`. (modify)
- `tui/src/components/text_editor/view.rs` — set terminal cursor *shape* by mode at the cursor-render site. (modify)
- `tui/src/settings/mod.rs` — `EditorBackendSetting::Vim` (serde `"vim"`). (modify)

---

## Task 1: Rename `NvimMode` → shared `EditorMode`

Generalize the modal-state enum so both the nvim backend and the new vim engine speak the same modes (CONTEXT.md "Editing mode"). The only nvim-specific piece — parsing `nvim_get_mode` strings — stays as a named constructor.

**Files:**
- Modify: `tui/src/components/text_editor/snapshot.rs` (enum at line 163, `NvimSnapshot.mode` at 119, `from_nvim_str` at 186, tests at 198+)
- Modify: `tui/src/components/text_editor/backend.rs` (use at line 13, `matches!` at 547)
- Modify: `tui/src/components/text_editor/mod.rs` (import at line 104)

- [ ] **Step 1: Rename the enum and its impl in `snapshot.rs`**

In `tui/src/components/text_editor/snapshot.rs`, rename the type `NvimMode` to `EditorMode` (enum definition ~line 163 and `impl NvimMode` ~line 173). Keep all variants and methods identical:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum EditorMode {
    Normal,
    Insert,
    Visual,
    VisualLine,
    Command,
    Other(String),
}

impl EditorMode {
    pub fn label(&self) -> &str {
        match self {
            EditorMode::Normal => "NORMAL",
            EditorMode::Insert => "INSERT",
            EditorMode::Visual => "VISUAL",
            EditorMode::VisualLine => "V-LINE",
            EditorMode::Command => "COMMAND",
            EditorMode::Other(_) => "OTHER",
        }
    }

    /// Parse the one- or two-character mode string returned by `nvim_get_mode`.
    /// Nvim-only: the vim engine sets its mode directly, never through this.
    pub fn from_nvim_str(s: &str) -> Self {
        match s {
            "n" | "no" | "nov" | "noV" | "no\x16" => EditorMode::Normal,
            "i" => EditorMode::Insert,
            "v" => EditorMode::Visual,
            "V" => EditorMode::VisualLine,
            "c" => EditorMode::Command,
            other => EditorMode::Other(other.to_string()),
        }
    }
}
```

- [ ] **Step 2: Update remaining references in `snapshot.rs`**

In the same file, change `pub mode: NvimMode` (~line 119) to `pub mode: EditorMode`; in `Default for NvimSnapshot` change `mode: NvimMode::Normal` to `mode: EditorMode::Normal`; in `footer_label` change `self.mode == NvimMode::Command` to `EditorMode::Command`. Update the unit tests (`mode_label_*`, `footer_label_*`) to use `EditorMode::`.

- [ ] **Step 3: Update `backend.rs` and `mod.rs` references**

In `backend.rs`: change the `use super::snapshot::{NvimMode, NvimSnapshot};` (line 13) to `{EditorMode, NvimSnapshot}`, and the `matches!(mode, NvimMode::Visual | NvimMode::VisualLine)` (line 547) to `EditorMode::`. In `mod.rs`: change `use self::snapshot::{EditorSnapshot, NvimMode};` (line 104) to `{EditorSnapshot, EditorMode}`.

- [ ] **Step 4: Verify it compiles (mechanical rename, no behavior change)**

Run: `cargo build -p kimun-tui 2>&1 | tail -20`
Expected: clean build. If any `NvimMode` remains, the error names the file:line — fix it.

- [ ] **Step 5: Run the snapshot tests**

Run: `cargo test -p kimun-tui --lib snapshot 2>&1 | tail -20`
Expected: PASS (`mode_label_*`, `footer_label_*`).

- [ ] **Step 6: Commit**

```bash
git add tui/src/components/text_editor/
git commit -m "refactor: rename NvimMode to shared EditorMode"
```

---

## Task 2: `TextareaBackend` + `InputInterpreter` wrapper

Split the storage axis from the input-interpretation axis (adr/0012). `BackendState::Textarea` stops holding a bare `TextArea` and holds a `TextareaBackend { ta, input }`. The accessors absorb the `.ta`, so the 22 `as_textarea*()` call sites in `mod.rs` are unaffected; only the enum methods and the 9 direct match sites change.

**Files:**
- Modify: `tui/src/components/text_editor/backend.rs` (enum at 75, methods 80–160)
- Modify: `tui/src/components/text_editor/mod.rs` (9 direct match sites: 40, 653, 714, 788, 797, 881, 2085, 2300, 2456)
- Create: `tui/src/components/text_editor/vim.rs` (a stub `VimEngine` so this task compiles; fleshed out in Task 3)

- [ ] **Step 1: Add the `vim` module with a stub engine**

Create `tui/src/components/text_editor/vim.rs`:

```rust
//! Built-in vim emulation: a modal input interpreter over a `TextArea`.
//! Pure over `&mut TextArea` — no component state, no async (adr/0012).

use super::snapshot::EditorMode;

/// Modal vim state layered over the textarea buffer.
#[derive(Debug)]
pub struct VimEngine {
    mode: EditorMode,
}

impl Default for VimEngine {
    fn default() -> Self {
        // Notes open in Normal mode (vim convention).
        Self { mode: EditorMode::Normal }
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
}
```

Register the module: in `tui/src/components/text_editor/mod.rs`, next to the other `pub mod` / `mod` declarations near the top (line 5 has `pub mod parse_incremental;`), add:

```rust
mod vim;
```

- [ ] **Step 2: Define `InputInterpreter` and `TextareaBackend` in `backend.rs`**

In `tui/src/components/text_editor/backend.rs`, add near the top (after the imports, before `BackendState`):

```rust
use super::vim::VimEngine;

/// How key events are translated into edits on a `TextArea` (adr/0012).
#[derive(Debug, Default)]
pub enum InputInterpreter {
    /// Today's behavior: keys go straight to the textarea.
    #[default]
    Direct,
    /// Built-in vim emulation.
    Vim(VimEngine),
}

/// The in-process textarea storage plus its input interpreter.
pub struct TextareaBackend {
    pub ta: TextArea<'static>,
    pub input: InputInterpreter,
}

impl TextareaBackend {
    pub fn direct(ta: TextArea<'static>) -> Self {
        Self { ta, input: InputInterpreter::Direct }
    }
    pub fn vim(ta: TextArea<'static>) -> Self {
        Self { ta, input: InputInterpreter::Vim(VimEngine::default()) }
    }
}
```

- [ ] **Step 3: Change the enum variant to hold `TextareaBackend`**

In `backend.rs`, change the variant (line 76) from `Textarea(TextArea<'static>)` to:

```rust
    Textarea(TextareaBackend),
```

- [ ] **Step 4: Update the enum methods/accessors to thread `.ta`**

In `backend.rs`, update each `BackendState::Textarea(...)` arm so the `TextArea` is reached through `.ta`:

```rust
    pub fn is_textarea(&self) -> bool {
        matches!(self, BackendState::Textarea(_))
    }

    pub fn as_textarea(&self) -> Option<&TextArea<'static>> {
        match self {
            BackendState::Textarea(tb) => Some(&tb.ta),
            BackendState::Nvim(_) => None,
        }
    }

    pub fn as_textarea_mut(&mut self) -> Option<&mut TextArea<'static>> {
        match self {
            BackendState::Textarea(tb) => Some(&mut tb.ta),
            BackendState::Nvim(_) => None,
        }
    }

    pub fn text(&self) -> String {
        match self {
            BackendState::Textarea(tb) => tb.ta.lines().join("\n"),
            BackendState::Nvim(nvim) => nvim.snapshot().lines.join("\n"),
        }
    }

    pub fn cursor(&self) -> (usize, usize) {
        match self {
            BackendState::Textarea(tb) => super::cursor_tuple(&tb.ta),
            BackendState::Nvim(nvim) => {
                let snap = nvim.snapshot();
                let max_row = snap.lines.len().saturating_sub(1);
                (snap.cursor.0.min(max_row), snap.cursor.1)
            }
        }
    }
```

In `recover_from_dead_nvim` (line 142), change the assignment to wrap in `TextareaBackend`:

```rust
        *self = BackendState::Textarea(TextareaBackend::direct(TextArea::from(
            fallback_text.lines(),
        )));
```

- [ ] **Step 5: Build the vim interpreter in `from_settings`**

In `backend.rs` `from_settings` (line 146), keep the nvim branch, and make the textarea branch honor the new `Vim` setting (the enum value is added in Step 8; reference it now):

```rust
    pub fn from_settings(
        editor_backend: &EditorBackendSetting,
        nvim_path: Option<&PathBuf>,
    ) -> Self {
        if matches!(editor_backend, EditorBackendSetting::Nvim) {
            match NvimBackend::new(nvim_path) {
                Ok(backend) => return BackendState::Nvim(backend),
                Err(e) => {
                    tracing::warn!("nvim backend unavailable, falling back to textarea: {e}")
                }
            }
        }
        let tb = if matches!(editor_backend, EditorBackendSetting::Vim) {
            TextareaBackend::vim(TextArea::default())
        } else {
            TextareaBackend::direct(TextArea::default())
        };
        BackendState::Textarea(tb)
    }
```

- [ ] **Step 6: Update the 9 direct match sites in `mod.rs`**

Find every direct match: Run `grep -nE "BackendState::Textarea\(" tui/src/components/text_editor/mod.rs`. Each `BackendState::Textarea(ta) => …` becomes `BackendState::Textarea(tb) =>` with `ta` replaced by `tb.ta` (or `&tb.ta` / `&mut tb.ta` to match the original binding). The exact sites and edits:

- Line 40 (`snapshot_from_backend`): `BackendState::Textarea(tb) => { … tb.ta … }`
- Line 653 (`lines`): `BackendState::Textarea(tb) => tb.ta.lines(),`
- Line 714: `BackendState::Textarea(tb) => { … tb.ta … }`
- Line 788 (`is_dirty`): the arm binds `_`, unchanged: `BackendState::Textarea(_) => self.saved_content_rev != Some(self.content_revision),`
- Line 797: `BackendState::Textarea(tb) => { … tb.ta … }`
- Line 881: `BackendState::Textarea(tb) => { … tb.ta … }`
- Line 2085 (render phase 1): binds `_`, unchanged.
- Line 2300 (`get_ta` test helper): `BackendState::Textarea(tb) => &mut tb.ta,`
- Line 2456 (test): `if let BackendState::Textarea(tb) = &editor.backend { …tb.ta… }`

Read each site before editing to preserve `&`/`&mut`/method calls exactly.

- [ ] **Step 7: Verify the wrapper compiles**

Run: `cargo build -p kimun-tui 2>&1 | tail -25`
Expected: clean (the `EditorBackendSetting::Vim` reference in Step 5 will error until Step 8 — do Step 8 before building, or expect that single error).

- [ ] **Step 8: Add the `Vim` settings variant**

In `tui/src/settings/mod.rs`, add the variant to `EditorBackendSetting` (line 46):

```rust
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditorBackendSetting {
    #[default]
    Textarea,
    Nvim,
    Vim,
}
```

- [ ] **Step 9: Write a TOML round-trip test for the new variant**

In `tui/src/settings/mod.rs`, in the settings test module (near the existing config round-trip tests), add:

```rust
    #[test]
    fn editor_backend_vim_roundtrips_through_toml() {
        let v = EditorBackendSetting::Vim;
        let s = toml::to_string(&v).unwrap();
        assert_eq!(s.trim(), "\"vim\"");
        let back: EditorBackendSetting = toml::from_str(&s).unwrap();
        assert_eq!(back, EditorBackendSetting::Vim);
    }
```

- [ ] **Step 10: Build and run settings + editor tests**

Run: `cargo test -p kimun-tui --lib settings::tests::editor_backend_vim_roundtrips_through_toml 2>&1 | tail -10`
Expected: PASS.
Run: `cargo build -p kimun-tui 2>&1 | tail -20`
Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add tui/src/
git commit -m "feat: TextareaBackend + InputInterpreter, vim settings variant"
```

---

## Task 3: `VimEngine` — modal transitions + cursor motions

Flesh out the engine: Normal-mode motions (cursor-only) and insert-entry commands (`i a I A o O`), plus `Esc` back to Normal. Pure over `&mut TextArea`, unit-tested with no component. Operators/edits/counts/visual are **Plan 2**.

**Files:**
- Modify: `tui/src/components/text_editor/vim.rs`
- Test: same file (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests for outcomes + transitions**

Append to `tui/src/components/text_editor/vim.rs`:

```rust
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
        // move right a couple cols in insert (simulated by direct cursor move)
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
    fn unknown_normal_key_is_noop() {
        let mut e = VimEngine::default();
        let mut t = ta();
        let out = e.handle_key(&key('z'), &mut t);
        assert_eq!(out, VimKeyOutcome::NoOp);
        assert_eq!(*e.mode(), EditorMode::Normal);
    }
}
```

- [ ] **Step 2: Run the tests to confirm they fail to compile**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -15`
Expected: FAIL — `VimKeyOutcome` and `handle_key` are not defined yet.

- [ ] **Step 3: Implement `VimKeyOutcome` and `handle_key`**

In `tui/src/components/text_editor/vim.rs`, add the import and the outcome enum at the top (after `use super::snapshot::EditorMode;`):

```rust
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
```

Add the dispatch method to `impl VimEngine`:

```rust
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
            // Vim steps the cursor left when leaving insert (unless at col 0).
            if super::cursor_tuple(ta).1 > 0 {
                ta.move_cursor(CursorMove::Back);
            }
            return VimKeyOutcome::CursorOnly;
        }
        VimKeyOutcome::PassThrough
    }

    fn handle_normal(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        // Only plain (no Ctrl/Alt) char keys and the arrow/Esc keys are vim
        // commands here; modified chords fall through as NoOp so app-level
        // Ctrl shortcuts (handled upstream) are never shadowed.
        let plain = key.modifiers == KeyModifiers::NONE
            || key.modifiers == KeyModifiers::SHIFT;
        match key.code {
            KeyCode::Char(c) if plain => self.normal_char(c, ta),
            KeyCode::Left => self.motion(CursorMove::Back, ta),
            KeyCode::Right => self.motion(CursorMove::Forward, ta),
            KeyCode::Up => self.motion(CursorMove::Up, ta),
            KeyCode::Down => self.motion(CursorMove::Down, ta),
            _ => VimKeyOutcome::NoOp,
        }
    }

    fn motion(&self, m: CursorMove, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        ta.move_cursor(m);
        VimKeyOutcome::CursorOnly
    }

    fn normal_char(&mut self, c: char, ta: &mut TextArea<'static>) -> VimKeyOutcome {
        match c {
            // Motions (cursor-only).
            'h' => self.motion(CursorMove::Back, ta),
            'l' => self.motion(CursorMove::Forward, ta),
            'k' => self.motion(CursorMove::Up, ta),
            'j' => self.motion(CursorMove::Down, ta),
            'w' => self.motion(CursorMove::WordForward, ta),
            'b' => self.motion(CursorMove::WordBack, ta),
            'e' => self.motion(CursorMove::WordEnd, ta),
            '0' => self.motion(CursorMove::Head, ta),
            '$' => self.motion(CursorMove::End, ta),
            '^' => self.motion(CursorMove::Head, ta), // refined to first-non-blank in Plan 2
            'G' => self.motion(CursorMove::Bottom, ta),
            // Insert-entry.
            'i' => self.enter_insert(ta, None),
            'a' => self.enter_insert(ta, Some(CursorMove::Forward)),
            'I' => self.enter_insert(ta, Some(CursorMove::Head)),
            'A' => self.enter_insert(ta, Some(CursorMove::End)),
            'o' => self.open_line(ta, false),
            'O' => self.open_line(ta, true),
            _ => VimKeyOutcome::NoOp,
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
        VimKeyOutcome::TextMutated
    }
```

- [ ] **Step 4: Run the engine tests to verify they pass**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -20`
Expected: PASS (all 7 tests). If `CursorMove::WordEnd`/`WordBack` names mismatch, the compiler will say so — they are confirmed present in `ratatui-textarea` 0.9.1.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/text_editor/vim.rs
git commit -m "feat: VimEngine skeleton — normal motions, insert entry, esc"
```

---

## Task 4: Wire vim dispatch into `handle_input`

Route keys through the engine when the interpreter is `Vim`, before the existing textarea path. `PassThrough` (Insert mode) falls into the existing direct path so typing, autocomplete, auto-surround and smart-Enter keep working unchanged. Normal-mode outcomes map to the existing content/cursor bump helpers.

**Files:**
- Modify: `tui/src/components/text_editor/mod.rs` (`handle_input` Key arm, lines 1962–2017)
- Modify: `tui/src/components/text_editor/backend.rs` (helper to reach the interpreter)
- Test: `tui/src/components/text_editor/mod.rs` (`#[cfg(test)]`)

- [ ] **Step 1: Add a `BackendState` helper that runs the vim engine**

In `backend.rs`, add a method that, when the active backend is a vim interpreter, runs the engine and returns the outcome (None when not vim, so the caller falls through to the direct path):

```rust
    /// If the active backend is the vim interpreter, run it for this key and
    /// return the outcome. Returns `None` for Direct / Nvim backends.
    pub fn vim_handle_key(
        &mut self,
        key: &ratatui::crossterm::event::KeyEvent,
    ) -> Option<super::vim::VimKeyOutcome> {
        match self {
            BackendState::Textarea(TextareaBackend {
                ta,
                input: InputInterpreter::Vim(engine),
            }) => Some(engine.handle_key(key, ta)),
            _ => None,
        }
    }
```

- [ ] **Step 2: Write the failing component test**

In `mod.rs` tests, add (use the existing test-construction helpers; `editor_with_backend` below mirrors `get_ta`'s setup — if a vim-constructing helper already exists, use it instead):

```rust
    #[test]
    fn vim_normal_i_then_typing_inserts_text() {
        let mut editor = TextEditorComponent::new(KeyBindings::default(), &{
            let mut s = AppSettings::default();
            s.editor_backend = crate::settings::EditorBackendSetting::Vim;
            s
        });
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        // In Normal mode, 'x' is unmapped → no text change.
        editor.handle_input(&InputEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)), &tx);
        assert_eq!(editor.get_text(), "");
        // 'i' enters Insert; then 'x' types a literal x via the direct path.
        editor.handle_input(&InputEvent::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)), &tx);
        editor.handle_input(&InputEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)), &tx);
        assert_eq!(editor.get_text(), "x");
    }
```

- [ ] **Step 3: Run it to confirm failure**

Run: `cargo test -p kimun-tui --lib vim_normal_i_then_typing_inserts_text 2>&1 | tail -15`
Expected: FAIL — in Normal mode `x` currently passes to the direct path and types `x` (assert on empty fails), proving the branch isn't wired.

- [ ] **Step 4: Insert the vim branch in `handle_input`**

In `mod.rs` `handle_input`, inside the `InputEvent::Key(key)` arm, immediately **after** the autocomplete popup-probe block and **before** the `if let Some(state) = self.handle_nvim_key(key, tx)` line (line 1994), add:

```rust
                // Vim interpreter: Normal/Visual consume the key here; Insert
                // mode returns PassThrough and falls into the direct path below
                // so typing, autocomplete, auto-surround and smart-Enter all
                // keep working (adr/0012).
                if let Some(outcome) = self.backend.vim_handle_key(key) {
                    use self::vim::VimKeyOutcome;
                    match outcome {
                        VimKeyOutcome::TextMutated => {
                            self.selection =
                                self.backend.as_textarea().and_then(|_| None); // motions clear no selection yet
                            self.bump_content();
                            return EventState::Consumed;
                        }
                        VimKeyOutcome::CursorOnly => {
                            self.refresh_autocomplete_if_open();
                            self.edit_generation = self.edit_generation.wrapping_add(1);
                            return EventState::Consumed;
                        }
                        VimKeyOutcome::NoOp => return EventState::Consumed,
                        VimKeyOutcome::PassThrough => { /* fall through to direct path */ }
                    }
                }
```

Note: `bump_content()` is the existing helper that bumps `content_revision` (grep `fn bump_content` to confirm its name; it is the method the direct edit handlers already call). If the helper is named differently, use the existing one. The `self.selection = … None` line is a placeholder for "vim motions don't set a selection yet" — Visual mode wiring lands in Plan 2; for now set `self.selection = None` on `TextMutated`.

Replace that placeholder line with simply:

```rust
                            self.selection = None;
```

- [ ] **Step 5: Run the component test to verify it passes**

Run: `cargo test -p kimun-tui --lib vim_normal_i_then_typing_inserts_text 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 6: Run the full editor test suite (no regressions on Direct/Nvim)**

Run: `cargo test -p kimun-tui --lib text_editor 2>&1 | tail -25`
Expected: PASS — the `Direct` interpreter returns `None` from `vim_handle_key`, so existing behavior is untouched.

- [ ] **Step 7: Commit**

```bash
git add tui/src/components/text_editor/
git commit -m "feat: route keys through VimEngine in handle_input"
```

---

## Task 5: Footer mode label for vim

Generalize the footer's modal-label slot so it works for the vim interpreter, not just nvim. Add `BackendState::mode_label() -> Option<String>` and have `hint_shortcuts` consume it.

**Files:**
- Modify: `tui/src/components/text_editor/backend.rs` (new `mode_label`)
- Modify: `tui/src/components/text_editor/mod.rs` (`hint_shortcuts`, lines 2227–2245)

- [ ] **Step 1: Write the failing test for `mode_label`**

In `backend.rs` tests (add a `#[cfg(test)] mod tests` if none exists, else append):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_textarea::TextArea;

    #[test]
    fn direct_backend_has_no_mode_label() {
        let b = BackendState::Textarea(TextareaBackend::direct(TextArea::default()));
        assert_eq!(b.mode_label(), None);
    }

    #[test]
    fn vim_backend_reports_normal_label() {
        let b = BackendState::Textarea(TextareaBackend::vim(TextArea::default()));
        assert_eq!(b.mode_label().as_deref(), Some("NORMAL"));
    }
}
```

- [ ] **Step 2: Run to confirm failure**

Run: `cargo test -p kimun-tui --lib backend::tests 2>&1 | tail -15`
Expected: FAIL — `mode_label` not defined.

- [ ] **Step 3: Implement `mode_label`**

In `backend.rs` `impl BackendState`, add:

```rust
    /// The footer modal-mode label, when the backend has one (nvim, or the
    /// vim interpreter). `None` for the plain Direct textarea.
    pub fn mode_label(&self) -> Option<String> {
        match self {
            BackendState::Textarea(TextareaBackend {
                input: InputInterpreter::Vim(engine),
                ..
            }) => Some(engine.mode_label()),
            BackendState::Textarea(_) => None,
            BackendState::Nvim(nvim) => Some(nvim.snapshot().footer_label()),
        }
    }
```

- [ ] **Step 4: Consume it in `hint_shortcuts`**

In `mod.rs` `hint_shortcuts` (line 2227), replace the nvim-only branch:

```rust
        // Prepend the modal-mode label (nvim or vim) as the first "hint".
        if let Some(label) = self.backend.mode_label() {
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
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p kimun-tui --lib backend::tests 2>&1 | tail -10`
Expected: PASS.
Run: `cargo build -p kimun-tui 2>&1 | tail -10`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add tui/src/components/text_editor/
git commit -m "feat: generalize footer mode label to vim backend"
```

---

## Task 6: Cursor shape by mode

Show a block cursor in Normal mode and a bar in Insert mode via crossterm `SetCursorStyle`, set at the cursor-render site. This is the first use of `SetCursorStyle` in the codebase; some terminals ignore it (graceful — the footer still shows the mode).

**Files:**
- Modify: `tui/src/components/text_editor/view.rs` (cursor-render site, line 1008)
- Modify: `tui/src/components/text_editor/mod.rs` (`render`, pass the desired shape into the view)

- [ ] **Step 1: Pass the desired cursor shape into the view render**

The view positions the cursor (`view.rs:1008`). It needs to know whether to request a block (Normal/Visual) or bar (Insert/Direct). Add a parameter to the view's render method. First, in `mod.rs` `render` (where it calls `self.view.render(f, editor_rect, theme, editor_focused)` ~line 2148), compute the shape from the backend and pass it. Add a small enum in `view.rs`:

```rust
/// Terminal cursor shape the editor requests while focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    Bar,
    Block,
}
```

In `mod.rs`, before the `self.view.render(...)` call, derive it:

```rust
        use self::view::CursorShape;
        let cursor_shape = match self.backend.mode_label().as_deref() {
            // Block in any vim-ish non-insert mode; bar in Insert and Direct.
            Some(m) if m != "INSERT" => CursorShape::Block,
            _ => CursorShape::Bar,
        };
```

Change the `self.view.render(...)` signature/call to take `cursor_shape` as an added argument (append it to the existing parameter list).

- [ ] **Step 2: Apply the shape at the cursor-render site**

In `view.rs`, update the render method signature to accept `cursor_shape: CursorShape`, and at line 1008 (right after `f.set_cursor_position(...)`), emit the style on the frame's backend. ratatui re-exports crossterm; use the buffer/terminal command via `f.set_cursor_position` neighbor. Since ratatui's `Frame` doesn't expose cursor-style directly, queue the command through stdout at render time:

```rust
                f.set_cursor_position(Position { x: cx, y: cy });
                self.last_cursor_screen = Some((cx, cy));
                use ratatui::crossterm::cursor::SetCursorStyle;
                let style = match cursor_shape {
                    CursorShape::Block => SetCursorStyle::SteadyBlock,
                    CursorShape::Bar => SetCursorStyle::SteadyBar,
                };
                let _ = ratatui::crossterm::execute!(std::io::stdout(), style);
```

Note: `execute!` to `stdout()` here is consistent with how a TUI app issues cursor-style commands; if the app uses an alternate writer, route through it instead. Verify there is no double-buffering conflict by running the app (Step 4).

- [ ] **Step 3: Build**

Run: `cargo build -p kimun-tui 2>&1 | tail -15`
Expected: clean. Fix any signature mismatch on the `view.render` callers (there may be a test caller — pass `CursorShape::Bar`).

- [ ] **Step 4: Manual verification (terminal-dependent, no unit test)**

Run the app against a scratch vault with `editor_backend = "vim"` in config, open a note: the cursor is a block; press `i` → it becomes a bar; `Esc` → block again. The footer reads NORMAL/INSERT in step. (If your terminal ignores `SetCursorStyle`, the footer still flips — acceptable per adr/0012.)

Run: `cargo run -p kimun-tui -- --help 2>&1 | tail -5` (smoke that the binary builds/links).

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/text_editor/
git commit -m "feat: block cursor in normal mode, bar in insert (SetCursorStyle)"
```

---

## Task 7: Reset to Normal mode on note open

When a new note is loaded into a vim-backed editor, the mode must reset to Normal (a note opened mid-insert from a previous note should not carry Insert mode over).

**Files:**
- Modify: `tui/src/components/text_editor/backend.rs` (`set_text` path / a reset hook)
- Modify: `tui/src/components/text_editor/mod.rs` (wherever `set_text` is called on open — confirm via grep)
- Test: `tui/src/components/text_editor/vim.rs`

- [ ] **Step 1: Add a `reset_to_normal` on the engine + a backend passthrough**

In `vim.rs` `impl VimEngine`, add:

```rust
    pub fn reset_to_normal(&mut self) {
        self.mode = EditorMode::Normal;
    }
```

In `backend.rs` `impl BackendState`, add:

```rust
    /// Reset the vim interpreter to Normal mode (called when a fresh note is
    /// loaded). No-op for Direct / Nvim backends.
    pub fn vim_reset_to_normal(&mut self) {
        if let BackendState::Textarea(TextareaBackend {
            input: InputInterpreter::Vim(engine),
            ..
        }) = self
        {
            engine.reset_to_normal();
        }
    }
```

- [ ] **Step 2: Write the failing test**

In `vim.rs` tests:

```rust
    #[test]
    fn reset_returns_to_normal_from_insert() {
        let mut e = VimEngine::default();
        let mut t = ta();
        e.handle_key(&key('i'), &mut t);
        assert_eq!(*e.mode(), EditorMode::Insert);
        e.reset_to_normal();
        assert_eq!(*e.mode(), EditorMode::Normal);
    }
```

- [ ] **Step 3: Run to confirm pass after implementing**

Run: `cargo test -p kimun-tui --lib text_editor::vim::tests::reset_returns_to_normal_from_insert 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 4: Call the reset where notes are loaded**

Find the editor's note-load entry point: Run `grep -nE "fn set_text|\.set_text\(|fn load|fn open_note" tui/src/components/text_editor/mod.rs`. In the public method that loads note content into the editor (the one that calls the backend's `set_text` / rebuilds the textarea), add `self.backend.vim_reset_to_normal();` right after the content is installed. Read the method first to place it after the buffer is set, not before.

- [ ] **Step 5: Build + full editor tests**

Run: `cargo test -p kimun-tui --lib text_editor 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add tui/src/components/text_editor/
git commit -m "feat: reset vim mode to Normal on note open"
```

---

## Self-Review

**Spec coverage (Plan 1 scope only):**
- `vim` config variant + TOML round-trip — Task 2 ✅
- `NvimMode`→`EditorMode` generalization, `from_nvim_str` kept nvim-only — Task 1 ✅
- `InputInterpreter`/`TextareaBackend` encoding (adr/0012) — Task 2 ✅
- Modal transitions (Normal↔Insert, `i a I A o O`, `Esc`) + cursor motions — Task 3 ✅
- Insert-mode delegation to existing textarea path (auto-surround/smart-Enter/autocomplete for free) — Task 4 ✅
- Footer mode label — Task 5 ✅
- Cursor shape by mode — Task 6 ✅
- Normal on open — Task 7 ✅
- **Deferred to Plan 2** (correctly out of Plan 1 scope): operators `d c y`, `x s S r J p P u Ctrl-r`, counts, `f F t T ; ,`, text objects, `%`, `gg`, `> <`, `~`, `{ }`, dot-repeat, visual mode, pending-command footer hint, register.
- **Deferred to Plan 3**: `/ ? n N`, `:`→palette + Ex-aliases, `note.save`/`app.quit` actions + `vim_aliases`, Space-leader, mouse→Visual, auto-surround-in-Visual.

**Placeholder scan:** Step 4 of Task 4 flagged one inline placeholder and gave the concrete replacement (`self.selection = None;`). Tasks 6-Step2 and 7-Step4 require reading the exact call site first (cursor-style writer, note-load method) — these are "read then place" instructions with the exact code to insert, not TODOs.

**Type consistency:** `VimKeyOutcome` (TextMutated/CursorOnly/NoOp/PassThrough) used identically in Tasks 3–4. `EditorMode` (renamed in Task 1) used in Tasks 1,3,5,7. `TextareaBackend`/`InputInterpreter` defined in Task 2, matched in Tasks 4,5,7. `mode_label()` returns `Option<String>` in Task 5, consumed as `.as_deref()` in Tasks 5,6. `cursor_tuple` (existing free fn) used in Task 3.

**Open verification points for the implementer:** confirm the exact name of the content-bump helper (`bump_content`) and the note-load method via the greps embedded in Tasks 4 and 7; confirm `AppSettings::default()` exists for the test in Task 4 (else use the existing test settings constructor).
