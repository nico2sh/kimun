# Phase 08 — Telescope modal alignment

**Objective:** Keep the existing telescope-style search modal, but align it to the new design: **list + live preview**, shared query highlighter, and scoped invocations. Do **not** rebuild it from scratch — refit what's there.

**Read first:** spec §6 (telescope), Appendix A.3. Depends on Phase 03 (drawers/parser) and Phase 07 (highlighter widget). Review the current modal implementation.

**Tasks**
1. **Layout:** centered floating panel (~75% width), `bg_hard`, green border, over a dimmed app. Top = input (drop in the **Phase-07 highlighter widget**) with a left prefix glyph and right-aligned result count. Body = **split**: results list (~45%) + **live preview** (note render + match context, with `filename · N matches` header). Footer = key hints.
2. **Live preview:** as the selection moves, render the highlighted result on the right with matched spans emphasized (`yellow`).
3. **Scoped invocations, one widget:**
   - `Ctrl-P` → command palette (commands, fuzzy, prefix `›`)
   - `Ctrl-F` → query/search (full query syntax, prefix `⌕`)
   - `Ctrl-O` → quick-open file
   Also make the leader leaves that *list things* (§8c: `f g`, `f t`, `l b`, `f h`, …) open this modal **pre-scoped**.
4. **Keys:** `↑↓` move · `⏎` open · `Ctrl-Space` open in split (stub if splits aren't built) · `Tab` mark · `Esc` close and **restore prior focus/selection** untouched.
5. Share the **parser/highlighter** with FIND — no second implementation.

**Acceptance** (spec §6)
- [ ] Modal shows list + live preview; uses the shared highlighter.
- [ ] `Ctrl-P` / `Ctrl-F` / `Ctrl-O` open it pre-scoped with the right prefix; leader list-leaves open it scoped.
- [ ] `Esc` closes and restores prior focus; underlying state unchanged.

**Out of scope:** editor highlighting (09). Keep the modal a *thin* refit over existing logic.

**Done when:** the telescope modal looks and behaves like Appendix A.3, reuses the FIND highlighter, and opens correctly from all entry points.
