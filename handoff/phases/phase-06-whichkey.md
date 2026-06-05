# Phase 06 — which-key overlay + cheatsheet

**Objective:** Make the leader tree **self-documenting** — a which-key popup that appears on hesitation and drills into groups, plus a full cheatsheet and the theme/settings picker.

**Read first:** spec §8b (overlay), §8c (tree), Appendix A.2. Reference `../target-leader-whichkey.png`. Depends on Phase 05.

**Tasks**
1. **Overlay** docked **above the status bar**, full width, `focus_border` (green) border. Shown when the Phase-05 timeout elapses mid-sequence; hidden the instant the sequence completes or cancels.
2. **Header:** the pressed sequence as keycaps (`Ctrl-K` → `Ctrl-K f`), a caption (`leader — pick a group` / `+find`), right-aligned `Esc cancel · BkSp up a level`.
3. **Body grid:** multi-column `key → target` rows. `key` in `yellow`; **group** targets as `→ +find` in `aqua`; **leaf** targets as `→ description` in `fg`. Pressing a group key **redraws** the grid with that group's next level (live drill-down) — driven by the same tree as Phase 05, no duplicated data.
4. **Cheatsheet:** `Ctrl-K ?` opens a full scrollable view of the entire tree.
5. **Settings/theme picker:** implement `Ctrl-K v c` → a picker that lists themes (from Phase 01) and applies on select; this replaces the temporary theme-switch from Phase 01.
6. Ensure the overlay and status hints (Phase 04) read from the **same** hint/keymap source.

**Acceptance** (spec §8b, §8c)
- [ ] Overlay appears on hesitation, drills into groups, hides on completion/cancel.
- [ ] Contents match §8c exactly and come from the single keymap source.
- [ ] `Ctrl-K ?` shows the full cheatsheet; `Ctrl-K v c` switches theme live.

**Out of scope:** query highlighting (07), telescope (08).

**Done when:** a newcomer can press `Ctrl-K`, read the menu, drill into any group, and discover/run every command without prior knowledge.
