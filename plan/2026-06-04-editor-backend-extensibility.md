# Editor backend extensibility — design

Date: 2026-06-04
Status: draft

## Problem

The text editor has two backends behind `BackendState`
(`tui/src/components/text_editor/backend.rs`): `Textarea(TextArea)` and
`Nvim(NvimBackend)`. A third backend is planned: a **vim mode** built on
`ratatui-textarea` — a key-input state machine (Normal/Insert/Visual, operators,
counts) driving the same `TextArea` widget, in the spirit of the `vim.rs`
example in the ratatui-textarea repo.

The current structure is not ready for that:

1. **~50 non-exhaustive destructures** in `mod.rs` of the form
   `let BackendState::Textarea(ta) = &mut self.backend else { return }` gate
   selection, find bar, surround, indent, smart-enter, mouse, clipboard, and
   autocomplete. A new textarea-flavored variant would *silently* lose every
   one of these features — the compiler can't flag let-else fallthroughs.
2. **`mod.rs` (3081 lines) mixes four concerns**: component orchestration +
   public API, ~800 lines of textarea editing ops, nvim key interception
   (ZZ/ZQ, `:wq` routing, dead-process recovery), and the find bar.
3. **Capabilities are implicit**: `matches!(backend, Textarea(_))` decides
   whether autocomplete, the find bar, and mouse handling exist. There is no
   single place where a backend declares what it supports.
4. **Footer modal label is nvim-special-cased** in `hint_shortcuts()`
   (`mod.rs:2013`) — a vim mode needs the same mode-label slot.

What is already right and must not change:

- `EditorSnapshot` (`snapshot.rs`) is the neutral seam. `view.rs`, the
  autocomplete host, and the markdown parse consume snapshots, never the
  backend. Rendering needs zero work for a new backend.
- `snapshot_from_backend()` is the single snapshot producer (one exhaustive
  match).
- The revision/dirty protocol (textarea bumps `content_revision` directly;
  nvim mirrors `snap.content_gen` at the render sync point) is documented and
  stays as-is.

## Key design decision

**Vim mode is not a new top-level `BackendState` variant.** It is the same
`TextArea` storage with a different *input interpreter*. Split the two axes
that `BackendState` currently conflates:

- **storage/widget axis**: where the buffer lives (`TextArea` vs embedded nvim
  process) — this is what `BackendState` keeps expressing.
- **input-interpretation axis**: how key events are translated into edits —
  new concept, local to the Textarea variant.

```rust
pub enum BackendState {
    Textarea(TextareaBackend),
    Nvim(NvimBackend),
}

pub struct TextareaBackend {
    pub ta: TextArea<'static>,
    pub input: InputInterpreter,
}

pub enum InputInterpreter {
    Direct,            // today's behavior
    Vim(VimEngine),    // phase 2
}
```

Every feature that needs `&TextArea` / `&mut TextArea` (snapshot borrowing,
selection rendering, clipboard, find bar, mouse, autocomplete, surround,
indent, smart-enter) keeps working for vim mode for free, because the
accessors (below) return `Some` for both interpreters. Only key dispatch
branches on the interpreter.

**Rejected alternatives:**

- `Box<dyn EditorBackend>` trait object — borrowed-snapshot lifetimes don't
  fit object safety, the capability sets differ wildly (async RPC vs sync
  widget), and there would be exactly two impls. Enum + `Option` accessors is
  the idiomatic shape here.
- Third enum variant `VimTextarea(TextArea, VimEngine)` — duplicates the
  storage axis; all ~50 feature sites would need a second match arm each, and
  every future textarea-flavored feature would have to remember both arms.

## Phase 1 — mechanical prep (no behavior change)

### 1a. Accessors on `BackendState` (`backend.rs`)

```rust
impl BackendState {
    pub fn textarea(&self) -> Option<&TextArea<'static>>;
    pub fn textarea_mut(&mut self) -> Option<&mut TextArea<'static>>;
    pub fn nvim(&self) -> Option<&NvimBackend>;
}
```

Replace the ~50 `let BackendState::Textarea(ta) = …` destructures in `mod.rs`
with `let Some(ta) = self.backend.textarea_mut() else { … }`. The accessors
live on `BackendState` (not `TextEditorComponent`) so the existing
field-disjoint borrow splits (`&self.backend` together with
`&mut self.autocomplete` / `&mut self.view`) keep compiling — this constraint
is load-bearing; several call sites inline free functions specifically for it
(`snapshot_from_backend`, `build_editor_host_snapshot`).

Exhaustive matches (`snapshot_from_backend`, `set_text`, `get_text`,
`is_dirty`, `lines`, `link_at_cursor`, `paste_text`, `render` phase 1,
`hint_shortcuts`) stay exhaustive matches — they are the compiler-enforced
checklist for any future variant and must not be flattened into accessors.

### 1b. Capabilities (`backend.rs`)

```rust
pub struct BackendCaps {
    pub autocomplete: bool,  // popup + set_vault activation
    pub find_bar: bool,      // Ctrl+F search
    pub mouse: bool,         // click/drag selection
    pub modal_footer: bool,  // footer shows a mode label
}

impl BackendState {
    pub fn caps(&self) -> BackendCaps;
}
```

A method (not a constant per variant) so vim mode can later make answers
mode-dependent (e.g. autocomplete only in Insert mode). Replace the implicit
gates:

- `set_vault` / `ensure_autocomplete_for_textarea` (`mod.rs:502`, `:518`)
- `build_editor_host_snapshot` (`mod.rs:374`)
- `open_or_advance_search` (`mod.rs:1338`)
- `handle_mouse` (`mod.rs:1755`)
- `hint_shortcuts` nvim branch (`mod.rs:2013`)

### 1c. Module split (`mod.rs` → three files)

- `textarea_input.rs` — `handle_textarea_key`, `handle_mouse`, the editing
  ops (`indent_lines`, `smart_enter`, `wrap_selection`, `apply_text_action`,
  clipboard fns), the find bar (`SearchState`, `render_search_bar`,
  `handle_search_key`, `search_advance`, …), and the textarea-only helpers
  (`selection_text`, `set_selection`, `surround_pair`, `cursor_move!`).
- `nvim_input.rs` — `handle_nvim_key`, ZZ/ZQ + `:wq` interception,
  `maybe_recover_from_dead_nvim`. Move the `nvim_pending_z` flag off
  `TextEditorComponent` into this module's state (or onto `NvimBackend`).
- `mod.rs` keeps: `TextEditorComponent` + public API, `handle_input` dispatch,
  `render`, snapshot producers, autocomplete wiring.

Pure code motion; functions become methods/free functions taking
`&mut TextEditorComponent` or the split-borrow pieces they already use.

### 1d. Generalize the footer mode label

`hint_shortcuts` currently asks `NvimSnapshot::footer_label()`. Add
`BackendState::mode_label() -> Option<String>` (None for non-modal backends)
and have `hint_shortcuts` consume it. Vim mode later plugs in here without
touching the footer again.

### 1e. Verification

`cargo test -p kimun-tui` green, zero behavior change. The test helper
`get_ta` (`mod.rs:2068`) switches to the new accessor.

## Phase 2 — vim engine skeleton

New `tui/src/components/text_editor/vim.rs`:

```rust
pub struct VimEngine {
    mode: VimMode,            // Normal | Insert | Visual | VisualLine | OperatorPending(op)
    pending_count: Option<u32>,
}

pub enum VimKeyOutcome {
    TextMutated,   // -> bump_content()
    CursorOnly,    // -> bump_cursor()
    NoOp,
    PassThrough,   // Insert mode: defer to handle_textarea_key
}

impl VimEngine {
    pub fn handle_key(&mut self, key: &KeyEvent, ta: &mut TextArea<'static>) -> VimKeyOutcome;
    pub fn mode_label(&self) -> &str;   // "NORMAL" / "INSERT" / "VISUAL" / …
}
```

- The engine is **pure over `&mut TextArea`** — no component state, no async,
  no channels. Port the dispatch pattern from ratatui-textarea's `vim.rs`
  example (transition function over `(mode, pending, key)`), mapping motions
  to `CursorMove`, operators to `start_selection` + motion + `cut`/`copy`,
  visual mode to the textarea selection.
- Dispatch in `handle_input`: before `handle_textarea_key`, if the interpreter
  is `Vim`, run `engine.handle_key`. `PassThrough` (Insert mode) falls into
  the existing direct path so typed text, autocomplete sync, surround, and
  smart-enter all keep working; Normal/Visual consume the key and map the
  outcome to `bump_content`/`bump_cursor` exactly like `ShortcutOutcome` does
  today.
- Mirror the textarea selection into `self.selection` after each engine call
  (same as the direct path) so visual mode renders through the existing
  selection pipeline.
- Settings: add `EditorBackendSetting::TextareaVim` (serde name `"vim"`),
  constructed in `BackendState::from_settings`. Round-trip TOML test next to
  the existing ones (`settings/mod.rs:894`).
- Footer: `mode_label()` wired via phase 1d. Reuse the label strings; do NOT
  reuse `NvimMode` (it's coupled to `nvim_get_mode` strings) — `VimMode` is
  its own enum.
- Caps: `find_bar: true` (Ctrl+F keeps working; vim-native `/` is out of
  scope), `mouse: true`, `autocomplete: true` in Insert mode only,
  `modal_footer: true`.
- Dirty/revision: unchanged — vim mode is just the Textarea protocol
  (`saved_content_rev` vs `content_revision`).

### Testing (phase 2)

Engine unit tests are plain `TextArea` + key-sequence assertions, no component
needed: `dd` deletes line, `x` deletes char, counts (`3w`), `i`/`Esc` mode
transitions, `v` + motion + `d`, outcome classification (motion = CursorOnly,
`x` = TextMutated, `Esc` in Normal = NoOp). Component-level: vim backend
construction from settings, footer label, autocomplete suppressed in Normal
mode, Insert-mode typing opens wikilink popup.

## Phase 3 — vim feature growth (later, incremental)

Operators `d`/`y`/`c` + motions, registers/clipboard integration (`y` →
existing `copy_selection_to_clipboard`), `.` repeat, `u`/`Ctrl+R` mapped to
textarea undo/redo, `o`/`O`/`A`/`I` insert variants. Each lands as an engine
transition + unit test; no component or rendering work expected.

## Out of scope

- Vim cmdline (`:w`, `:q`) — kimun's autosave + focus model makes these
  unnecessary; revisit only if users ask.
- Vim-native search (`/`, `n`/`N`) — Ctrl+F find bar covers it.
- Any change to `EditorSnapshot`, `view.rs`, the autocomplete host protocol,
  or the nvim RPC backend.
