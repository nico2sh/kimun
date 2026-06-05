# Phase 04 — Status bar v2

**Objective:** Make the two-line status bar **context-aware** — a focus indicator (not a vim mode) plus hints that follow focus and cursor context.

**Read first:** spec §7 (status bar), §5.2 (helper layer), Appendix A.5.

**Tasks**
1. **Line 1 — context + actions:** left-side **focus-context indicator**: `⌨ EDITOR` when a text field holds the cursor, `≣ LIST` when a list/rail is focused. Follow it with the most relevant key hints for that context; right-align global hints (`Ctrl-S save`, etc.).
2. Make hints **dynamic by cursor context** in the editor: e.g. cursor on a `[[wikilink]]` adds `⏎ follow` and `Ctrl-K m c copy link`.
3. **Line 2 — document state:** path · `ln/col` · `● modified` / `✓ saved` · backlink count · git status · (in query contexts) index match count.
4. Introduce a small **hint registry** so each focus/context can declare its hints in one place (the leader engine and which-key in 05/06 will reuse this).

**Acceptance** (spec §7)
- [ ] Line-1 indicator + hints change with focus and cursor context.
- [ ] **No "mode" is ever displayed** — only focus context.
- [ ] Line 2 shows live document state.

**Out of scope:** the which-key overlay (06). This phase is just the persistent status bar.

**Done when:** moving focus and moving the cursor over links/tags visibly updates the hints, and the indicator only ever shows `⌨ EDITOR` / `≣ LIST`.
