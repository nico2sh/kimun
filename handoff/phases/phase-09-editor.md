# Phase 09 — Editor highlights + helpers/tips

**Objective:** Turn the editor into a **highlight-on-source** markdown buffer (still editable, not a preview mode) with clickable links/tags and a contextual **helper layer**.

**Read first:** spec §5 (editor screen), §5.1 (highlights), §5.2 (helpers), Appendix A.5.

**Tasks**
1. **Markdown highlighting** (live as the buffer changes), per §5.1: headings (H1/H2 `fg_bright`, H3 `yellow`), bold/italic, bullets (`gray`), ordered numbers (`purple`), inline/fenced code (`aqua` on `bg_soft`), blockquote (`gray` + `▏` guide), tasks `- [ ]/[x]` (done dimmed/struck), block cursor.
2. **Wikilinks** `[[…]]` → `blue` underlined, **clickable**; `Ctrl-Enter` follows when the cursor is on one. **Tags** `#tag` → `aqua`, clickable → runs tag query.
3. **Search-match emphasis:** when the editor is opened from a query result, highlight the matched span(s) in `yellow`.
4. **Helper layer (§5.2):**
   - Contextual key hints in **status line 1** that change with cursor context (reuse the Phase-04 hint registry).
   - When the cursor enters a link/tag, surface its target in **status line 2** (e.g. `→ people/maria.md · 3 backlinks`).
   - **Empty-note tip:** dim ghost text in the body of a new/empty note (`Type to start · [[ to link · # to tag · Ctrl-K cmds`) that clears on first keystroke.
5. Keep highlighting performant on large notes (incremental / viewport-only).

**Acceptance** (spec §5)
- [ ] All §5.1 elements highlight live.
- [ ] Wikilinks/tags clickable and `Ctrl-Enter`-followable; status hints update with cursor context.
- [ ] Empty-note tip shows and dismisses correctly.

**Out of scope:** a rendered/WYSIWYG preview mode (only `Ctrl-K m p` preview stays a separate action). This phase highlights the *source*.

**Done when:** editing a note shows live markdown highlights, links/tags are actionable, and the helper/tips behave as in Appendix A.5.
