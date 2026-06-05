# Phase 05 — Leader engine

**Objective:** Implement the **non-modal leader-key** engine: a universal `Ctrl-K` gateway plus a focus-gated `Space`, driving a configurable key-sequence tree. (The visible which-key menu is Phase 06 — here, build the input state machine and wire the actions.)

**Read first:** spec §8a (triggers), §8c (keymap tree), §0 hard constraints (non-modal).

**Tasks**
1. **Gateway:** `Ctrl-K` (configurable) starts a leader sequence in **every** context, including while typing in the editor.
2. **Bonus trigger:** bare `Space` starts the **same** sequence **only when a non-text panel (list/rail) is focused**. In any text field, Space inserts a space — verify it never leaks.
3. **Sequence state machine:** after the gateway, consume keys against the tree; `Esc` cancels and returns focus to the editor; `Backspace` steps up one level; an invalid key gives gentle feedback and stays in the menu.
4. **Timeout:** configurable (`~400ms` default) — only used to *reveal* the which-key overlay in Phase 06. Full sequences typed faster than the timeout must fire **without** waiting.
5. **Wire the tree (§8c):** Tier-1 groups `f n l o g v w` and Tier-2 `m` branch, mapping leaves to real actions (open drawers `o`, find/grep/tags via the picker, note ops, git, vault/theme, window). Where a target lands in a later phase (telescope picker, theme picker), call a stub that's easy to swap.
6. **Config surface:** gateway key + timeout read from config (full config UI is Phase 10; a config struct + defaults is enough now).

**Acceptance** (spec §8a, §8c)
- [ ] `Ctrl-K` opens the sequence in every context incl. mid-typing in the editor.
- [ ] `Space` leads only when a list/rail is focused; types a space in text fields.
- [ ] `Esc` cancels, `Backspace` steps up; fast full sequences fire without delay.
- [ ] Tree matches §8c; leaves invoke real actions or clearly-named stubs.
- [ ] Gateway key + timeout are configurable.

**Out of scope:** drawing the which-key menu/cheatsheet (06). Logic only here, but it must already *function* (e.g. `Ctrl-K o f` opens the FILES drawer).

**Done when:** every path in §8c executes from the keyboard, with correct focus-gated `Space`, even though no menu is drawn yet.
