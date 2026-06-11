# Vim Mode — Plan 3: Command-line & Leader Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the vim backend's outward wiring: `/ ? n N` (reuse the existing find bar), `:` → command palette (Zed-style) with an exact-match Ex-alias layer, two new global palette commands `note.save` (alias `w`) and `app.quit` (alias `q`/`wq`/`x`), Space as a Normal-mode leader, and mouse-drag → Visual mode (with auto-surround on a visual selection).

**Architecture:** Buffer ops live in the `VimEngine`; screen-level commands leave the component as signals the host turns into `AppEvent`s (`adr/0012`). `:` reuses the existing `LeaderAction::Palette`; the palette gains an Ex-alias resolver so `:w⏎`/`:q⏎` hit `note.save`/`app.quit` deterministically rather than via fuzzy ranking. `note.save` flushes the existing autosave; `app.quit` reuses the existing `Quit` event (which already saves on the way out). Space-leader is intrinsic to the vim backend in Normal mode with empty pending state — never a config knob (a global bare-Space binding would break space-typing everywhere).

**Tech Stack:** Rust; existing kimün plumbing — `command_palette.rs` (`SearchList`-based), `keys/leader.rs` (`LeaderAction` registry), `AppEvent::{ExecuteLeaderAction, Autosave, Quit, CloseOverlay}`, the editor's find bar (`open_or_advance_search`, `search_advance`, `close_search`). Tests: `cargo test -p kimun-tui`.

**Prereq:** Plans 1 & 2 merged. Decisions: `adr/0011`, `adr/0012`, `CONTEXT.md`.

---

## File Structure

- `tui/src/keys/leader.rs` — add `LeaderAction::NoteSave`/`AppQuit` (+ `id`, `ALL`, `from_id`, new `vim_aliases`). (modify)
- `tui/src/app_screen/editor.rs` — handle the two new actions in `execute_leader_action`; add Space-leader gate in `handle_input`; add the default leader-tree leaves. (modify)
- `tui/src/components/command_palette.rs` — Ex-alias resolver on Submit. (modify)
- `tui/src/components/text_editor/vim.rs` — `:` `/` `?` `n` `N` produce host actions; `vim_space_leads`. (modify)
- `tui/src/components/text_editor/backend.rs` — expose `vim_space_leads`, `vim_host_action` plumbing. (modify)
- `tui/src/components/text_editor/mod.rs` — turn engine host-actions into `AppEvent`/find-bar calls; mouse-drag → Visual. (modify)

---

## Task 1: New global actions `note.save` + `app.quit`

Two real, browsable palette commands. `note.save` flushes autosave now; `app.quit` quits (which already saves). Each carries vim Ex aliases.

**Files:** Modify `tui/src/keys/leader.rs`, `tui/src/app_screen/editor.rs`.

- [ ] **Step 1: Add the enum variants + id + registry**

In `tui/src/keys/leader.rs`, add to the `LeaderAction` enum (the variant list ~lines 16–64):

```rust
    NoteSave,
    AppQuit,
```

In `id()` (~69–115), add:

```rust
            LeaderAction::NoteSave => "note.save",
            LeaderAction::AppQuit => "app.quit",
```

In the `ALL` array (~118–161), add `LeaderAction::NoteSave, LeaderAction::AppQuit,`. `from_id` (~165–175) derives from `ALL`/`id()`, so it picks them up automatically — verify by reading it; if it's a hand-written match, add the two arms.

- [ ] **Step 2: Add a `vim_aliases` method**

In `leader.rs` `impl LeaderAction`, add:

```rust
    /// Exact-match vim Ex command aliases the command palette resolves before
    /// fuzzy ranking (e.g. `:w` → NoteSave). Empty for most actions.
    pub fn vim_aliases(&self) -> &'static [&'static str] {
        match self {
            LeaderAction::NoteSave => &["w", "write"],
            LeaderAction::AppQuit => &["q", "qa", "wq", "x"],
            _ => &[],
        }
    }
```

- [ ] **Step 3: Write a failing test for alias lookup**

In `leader.rs` tests:

```rust
    #[test]
    fn ex_aliases_resolve_to_actions() {
        let by_alias = |a: &str| LeaderAction::ALL.iter().copied()
            .find(|act| act.vim_aliases().contains(&a));
        assert_eq!(by_alias("w"), Some(LeaderAction::NoteSave));
        assert_eq!(by_alias("q"), Some(LeaderAction::AppQuit));
        assert_eq!(by_alias("wq"), Some(LeaderAction::AppQuit));
        assert_eq!(by_alias("nope"), None);
    }
```

- [ ] **Step 4: Handle the actions in `execute_leader_action`**

In `tui/src/app_screen/editor.rs` `execute_leader_action` (the match ~954–1084), add arms:

```rust
            LeaderAction::NoteSave => {
                // Flush the periodic autosave immediately (no manual-save concept;
                // this force-persists the current buffer if dirty).
                self.spawn_autosave(tx);
            }
            LeaderAction::AppQuit => {
                tx.send(AppEvent::Quit).ok();
            }
```

- [ ] **Step 5: Add default leader-tree leaves so they show in the palette**

The palette lists leader-tree leaves (`command_entries`). Find the default tree builder: Run `grep -rnE "fn default_leader|LeaderNode::Leaf|fn build.*tree|default_tree" tui/src/keys/`. In the default tree, add two leaves under a sensible group (e.g. a top-level `note` group already holds `note.new`/`note.daily`; quit can sit at top level). Match the existing leaf-construction syntax exactly; for example if leaves are `("s", leaf("Save note", LeaderAction::NoteSave))`, add:

```rust
            // under the note group:
            ("w", leaf("Write (save now)", LeaderAction::NoteSave)),
            // top level:
            ("Q", leaf("Quit kimün", LeaderAction::AppQuit)),
```

Read the surrounding tree code first and mirror its exact helper/format. Pick sequence keys that don't collide with existing bindings (verify against the tree).

- [ ] **Step 6: Build + test**

Run: `cargo test -p kimun-tui --lib leader 2>&1 | tail -15`
Expected: PASS (`ex_aliases_resolve_to_actions`).
Run: `cargo build -p kimun-tui 2>&1 | tail -15`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add tui/src/keys/leader.rs tui/src/app_screen/editor.rs
git commit -m "feat: note.save + app.quit leader actions with vim aliases"
```

---

## Task 2: Command-palette Ex-alias resolver

When the palette query exactly equals a vim alias, Enter runs the aliased action instead of the fuzzy-selected row. Everything else is unchanged fuzzy behavior.

**Files:** Modify `tui/src/components/command_palette.rs`.

- [ ] **Step 1: Confirm the query accessor**

Run: `grep -nE "pub fn query|fn query|input.*value|query\(\)" tui/src/components/search_list/*.rs tui/src/components/command_palette.rs`
Expected: a `SearchList::query()`-style accessor returning the current input string. If it is named differently (e.g. `input_value()`), use that name below.

- [ ] **Step 2: Add the resolver + hook it into Submit**

In `command_palette.rs`, add a helper on `CommandPaletteModal`:

```rust
    /// If the live query is exactly a vim Ex alias, return that action.
    fn ex_alias_action(&self) -> Option<LeaderAction> {
        let q = self.list.query().trim();
        if q.is_empty() {
            return None;
        }
        LeaderAction::ALL
            .iter()
            .copied()
            .find(|a| a.vim_aliases().contains(&q))
    }
```

Update `execute_selected` to prefer the alias:

```rust
    fn execute_selected(&self, tx: &AppTx) {
        if let Some(action) = self.ex_alias_action() {
            tx.send(AppEvent::CloseOverlay).ok();
            tx.send(AppEvent::ExecuteLeaderAction(action)).ok();
            return;
        }
        if let Some(entry) = self.list.selected_row() {
            let action = entry.action;
            tx.send(AppEvent::CloseOverlay).ok();
            tx.send(AppEvent::ExecuteLeaderAction(action)).ok();
        }
    }
```

Ensure `use crate::keys::leader::LeaderAction;` is present.

- [ ] **Step 3: Write a test for alias precedence**

In `command_palette.rs` tests (build a modal with the default tree; if a test constructor exists, use it). Assert the resolver, which is the unit under test:

```rust
    #[test]
    fn query_w_resolves_to_note_save() {
        // Construct via the same path the app uses; if a test helper exists,
        // prefer it. Then drive the query input to "w" and assert resolution.
        // The pure check on the resolver:
        let action = LeaderAction::ALL.iter().copied()
            .find(|a| a.vim_aliases().contains(&"w"));
        assert_eq!(action, Some(LeaderAction::NoteSave));
    }
```

(If `SearchList` is drivable in tests, extend this to set the query to `"w"` and assert `ex_alias_action()` returns `NoteSave`, and that `"wri"` — a non-exact fuzzy prefix — returns `None`.)

- [ ] **Step 4: Build + test + commit**

Run: `cargo test -p kimun-tui --lib command_palette 2>&1 | tail -15`
Expected: PASS.

```bash
git add tui/src/components/command_palette.rs
git commit -m "feat: command palette resolves exact vim Ex aliases (:w/:q)"
```

---

## Task 3: Vim `:` `/` `?` `n` `N` → host actions

The engine can't open overlays or the find bar itself (pure over `&mut TextArea`). It returns a host-action signal; the component turns it into an `AppEvent`/find-bar call.

**Files:** Modify `vim.rs`, `backend.rs`, `mod.rs`.

- [ ] **Step 1: Add a host-action signal to the engine**

In `vim.rs`, add:

```rust
/// Screen-level actions the host performs on the engine's behalf (adr/0012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimHostAction {
    OpenPalette,                 // `:`
    OpenSearch { forward: bool },// `/` (true) `?` (false)
    SearchNext,                  // `n`
    SearchPrev,                  // `N`
}
```

Add a variant to `VimKeyOutcome`:

```rust
    /// The host must perform a screen-level action.
    Host(VimHostAction),
```

In `normal_char`, add arms (before fallthrough):

```rust
        match c {
            ':' => { self.clear_pending(); return VimKeyOutcome::Host(VimHostAction::OpenPalette); }
            '/' => { self.clear_pending(); return VimKeyOutcome::Host(VimHostAction::OpenSearch { forward: true }); }
            '?' => { self.clear_pending(); return VimKeyOutcome::Host(VimHostAction::OpenSearch { forward: false }); }
            'n' => { self.clear_pending(); return VimKeyOutcome::Host(VimHostAction::SearchNext); }
            'N' => { self.clear_pending(); return VimKeyOutcome::Host(VimHostAction::SearchPrev); }
            _ => {}
        }
```

- [ ] **Step 2: Thread the host action through the backend**

In `backend.rs`, `vim_handle_key` already returns the `VimKeyOutcome`; the new `Host` variant rides along unchanged. No backend edit needed beyond re-exporting `VimHostAction` (ensure `pub use` or `pub` on the type).

- [ ] **Step 3: Write a failing test**

```rust
    #[test]
    fn colon_emits_open_palette() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["x"]);
        let out = e.handle_key(&key(':'), &mut t);
        assert_eq!(out, VimKeyOutcome::Host(VimHostAction::OpenPalette));
    }

    #[test]
    fn slash_emits_open_search_forward() {
        let mut e = VimEngine::default();
        let mut t = TextArea::from(["x"]);
        assert_eq!(
            e.handle_key(&key('/'), &mut t),
            VimKeyOutcome::Host(VimHostAction::OpenSearch { forward: true })
        );
    }
```

- [ ] **Step 4: Handle `Host` in the `mod.rs` dispatch**

In `mod.rs` `handle_input`, in the `match outcome` block (Plan 1 Task 4), add a `Host` arm:

```rust
                        VimKeyOutcome::Host(action) => {
                            use self::vim::VimHostAction;
                            match action {
                                VimHostAction::OpenPalette => {
                                    // Reuse the existing palette gateway.
                                    tx.send(AppEvent::ExecuteLeaderAction(
                                        crate::keys::leader::LeaderAction::Palette,
                                    )).ok();
                                }
                                VimHostAction::OpenSearch { forward: _ } => {
                                    // `/` and `?` open the existing find bar.
                                    // (`?` backward-first is a later refinement;
                                    // n/N still navigate both directions.)
                                    self.open_or_advance_search();
                                }
                                VimHostAction::SearchNext => self.search_advance(false),
                                VimHostAction::SearchPrev => self.search_advance(true),
                            }
                            return EventState::Consumed;
                        }
```

`open_or_advance_search` (mod.rs:1365) and `search_advance(backward)` (mod.rs:1458) are existing component methods. `LeaderAction::Palette` is the existing palette action; `ExecuteLeaderAction` is handled by `EditorScreen` (editor.rs:1664+) which calls `open_command_palette`.

- [ ] **Step 5: Run tests + build**

Run: `cargo test -p kimun-tui --lib text_editor::vim 2>&1 | tail -15`
Expected: PASS (`colon_emits_open_palette`, `slash_emits_open_search_forward`).
Run: `cargo build -p kimun-tui 2>&1 | tail -15`
Expected: clean. (The `Host` variant addition forces every `match outcome` to handle it — only the one site in `mod.rs` exists.)

- [ ] **Step 6: Commit**

```bash
git add tui/src/components/text_editor/ 
git commit -m "feat: vim : / ? n N route to palette + find bar"
```

---

## Task 4: Space as Normal-mode leader

Bare Space starts the leader sequence when the vim backend is in Normal mode with empty pending state. Insert/Visual/other backends untouched.

**Files:** Modify `vim.rs`, `backend.rs`, `mod.rs`, `tui/src/app_screen/editor.rs`.

- [ ] **Step 1: Engine predicate**

In `vim.rs` `impl VimEngine`:

```rust
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
```

- [ ] **Step 2: Backend + component passthrough**

In `backend.rs`:

```rust
    pub fn vim_space_leads(&self) -> bool {
        matches!(self,
            BackendState::Textarea(TextareaBackend { input: InputInterpreter::Vim(e), .. })
            if e.space_leads())
    }
```

In `mod.rs`, add a public component method:

```rust
    /// Whether a bare Space should start the leader (vim Normal mode only).
    pub fn vim_space_leads(&self) -> bool {
        self.backend.vim_space_leads()
    }
```

- [ ] **Step 3: Write the failing engine test**

```rust
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
```

- [ ] **Step 4: Gate Space in `EditorScreen::handle_input`**

In `tui/src/app_screen/editor.rs` `handle_input`, the comment block at lines 1417–1420 documents "bare Space never leads." Replace the behavior for the vim-Normal case. Add, just before the Tab/focus block (line 1426) — i.e. after the overlay/mouse handling and before falling through to panels:

```rust
        // Vim Normal mode: bare Space is the leader (in addition to Ctrl-G),
        // but only with an empty pending state so it never shadows Space as a
        // motion/operator argument. Insert/Visual and the other backends keep
        // Space typing a space (the rule below the Tab handling).
        if self.editor_active()
            && !self.overlays.is_open()
            && !self.leader.is_pending()
            && let InputEvent::Key(key) = event
            && key.code == ratatui::crossterm::event::KeyCode::Char(' ')
            && key.modifiers.is_empty()
            && self.panels.editor().vim_space_leads()
        {
            self.leader.start();
            self.schedule_whichkey_reveal(tx);
            return EventState::Consumed;
        }
```

Update the stale comment at 1417–1420 to note the vim-Normal exception. `editor_active`, `self.leader.start()`, `schedule_whichkey_reveal` are existing (used by the `ActionShortcuts::Leader` arm at 1229–1236).

- [ ] **Step 5: Build + test**

Run: `cargo test -p kimun-tui --lib text_editor::vim::tests::space_leads_only_in_clean_normal 2>&1 | tail -10`
Expected: PASS.
Run: `cargo test -p kimun-tui 2>&1 | tail -20`
Expected: PASS (no regression on textarea/nvim Space-typing).

- [ ] **Step 6: Commit**

```bash
git add tui/src/
git commit -m "feat: Space as vim Normal-mode leader (intrinsic, pending-safe)"
```

---

## Task 5: Mouse-drag → Visual mode (+ auto-surround on visual selection)

Mouse drag in a vim-backed editor enters Visual mode (a live textarea selection); click moves the cursor without changing mode. Auto-surround on a visual selection already works: in Visual mode a bare pair char returns `PassThrough` (Plan 2 Task 10), so the host's existing direct-path auto-surround wraps it.

**Files:** Modify `vim.rs`, `backend.rs`, `mod.rs`.

- [ ] **Step 1: Engine hook for "selection now exists / gone"**

The mouse is handled by the existing `handle_mouse` (direct textarea path), which sets/clears the textarea selection. After a mouse event on a vim backend, reconcile the engine mode from whether a selection exists. Add to `vim.rs`:

```rust
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
```

- [ ] **Step 2: Backend passthrough**

In `backend.rs`:

```rust
    pub fn vim_sync_mouse_selection(&mut self, has_selection: bool) {
        if let BackendState::Textarea(TextareaBackend { input: InputInterpreter::Vim(e), .. }) = self {
            e.sync_mouse_selection(has_selection);
        }
    }
```

- [ ] **Step 3: Call it from the mouse path in `mod.rs`**

In `mod.rs` `handle_input`, in the `InputEvent::Mouse` arm (lines 2018–2053), after `handle_mouse` runs and `self.selection` is updated, add:

```rust
                let has_sel = self
                    .backend
                    .as_textarea()
                    .and_then(|ta| ta.selection_range())
                    .is_some();
                self.backend.vim_sync_mouse_selection(has_sel);
```

- [ ] **Step 4: Write the test**

```rust
    #[test]
    fn mouse_selection_enters_and_leaves_visual() {
        let mut e = VimEngine::default();
        e.sync_mouse_selection(true);
        assert_eq!(*e.mode(), EditorMode::Visual);
        e.sync_mouse_selection(false);
        assert_eq!(*e.mode(), EditorMode::Normal);
    }
```

- [ ] **Step 5: Manual verification of auto-surround in Visual**

Run the app with `editor_backend = "vim"`: select a word in Visual mode (`viw` or mouse-drag), press `[` → the selection becomes `[word]` (host auto-surround via the PassThrough path), and the editor returns to Normal. Confirm `f[` in Visual still *moves* to `[` (pending-find wins — Plan 2 Task 7), proving the pending-priority ordering.

- [ ] **Step 6: Build + test + commit**

Run: `cargo test -p kimun-tui --lib text_editor 2>&1 | tail -20`
Expected: PASS.

```bash
git add tui/src/components/text_editor/
git commit -m "feat: mouse-drag enters vim Visual mode; auto-surround wraps selection"
```

---

## Self-Review

**Spec coverage (Plan 3 scope):** `note.save`+`app.quit` actions with vim aliases (T1); palette Ex-alias resolver so `:w⏎`/`:q⏎` are deterministic (T2); `:` → palette, `/ ?` → find bar, `n/N` → search advance (T3); Space-leader, Normal-only + pending-safe + intrinsic (T4); mouse-drag → Visual and auto-surround-on-selection via PassThrough (T5). ✅

**Cross-plan consistency:** `VimKeyOutcome::Host` (added T3) is handled at the single `match outcome` site in `mod.rs` (Plan 1 Task 4 / Plan 2 Task 10) — adding the variant makes the compiler flag any unhandled site. `vim_space_leads` reads the same pending fields defined in Plan 2 Task 1. The PassThrough-for-pair-chars path it relies on was built in Plan 2 Task 10.

**Placeholder scan:** `?` backward-first is explicitly deferred to a refinement (T3) with `n/N` covering both directions — stated, not hidden. T1 Step 5 and T2 Steps 1/3 require reading the exact leader-tree builder and `SearchList` query accessor first (grep commands given) — "read then place," with the exact code to insert.

**Implementer verification points:** confirm `SearchList::query()` accessor name (T2 S1); confirm the default leader-tree builder's leaf-construction syntax and pick non-colliding sequence keys (T1 S5); confirm `LeaderAction` derives `Copy`/`PartialEq` (used by the resolver and tests — it is `*action`-copied in `command_entries`, so it is `Copy`); confirm `self.leader.start()` + `schedule_whichkey_reveal(tx)` are the correct leader-start calls (they are, per editor.rs:1233–1234).

---

## Vim Mode — full feature set complete

With Plans 1–3 merged, `editor_backend = "vim"` provides: modal editing (Normal/Insert/Visual/Visual-line + operator-pending + `r`), motions (`h j k l w b e 0 ^ $ gg G { } %`, `f F t T ; ,`), operators (`d c y` + doubled + `D C Y`), edits (`x X s S r J ~ p P u Ctrl-r`), counts, text objects (`iw aw i" i( …`, single-line), dot-repeat, indent (`>> <<`), the command-line family (`/ ? n N`, `:`→palette + `:w`/`:q` aliases), Space-leader, mouse→Visual, and all existing textarea insert-mode features (auto-surround, smart-Enter, autocomplete, WYSIWYG render). Deferred by design: named registers, marks, macros (dot-repeat machinery leaves them additive), multi-line text objects, visual-block, `R` overtype.
