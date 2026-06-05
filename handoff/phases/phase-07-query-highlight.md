# Phase 07 — Query syntax highlighter

**Objective:** Live, tokenized **syntax highlighting** for the query language, first in the FIND drawer input. Build it as a reusable component (Phase 08 reuses it in the telescope modal).

**Read first:** spec §9 (query highlighting), Appendix A.4. Inspect the existing **query parser/AST** — map highlighting onto the *real* grammar, not the illustrative names.

**Tasks**
1. **Tokenize** the query string into classes and color by **role** (§9 table): operators `AND/OR/NOT` → `orange` bold; field keys (`tag:` `after:` `links:` `sort:` …) → `yellow`; tag values `#x` → `aqua`; wikilink values `[[x]]` → `blue`; quoted strings → `green`; date/number literals → `purple`; negation `-x` → `red`; grouping/punctuation → `gray`; bare terms → `fg`. **Reuse the existing parser** where possible rather than a second grammar.
2. **Incremental:** re-highlight as the user types, cheaply.
3. **Error handling:** on parse error, mark the offending span (red underline) and show a one-line reason in the FIND header — **never block typing**.
4. **Completion (if feasible):** suggest field keys, tag values, and link targets via inline ghost text or a small dropdown.
5. Package as a **reusable input widget** (highlighter + optional completion) so the telescope modal can drop it in unchanged.

**Acceptance** (spec §9)
- [ ] Live tokenized highlighting in the FIND input; roles match §9.
- [ ] Invalid spans marked with a reason, without blocking input.
- [ ] Implemented as a shared widget ready for telescope reuse.

**Out of scope:** the telescope modal itself (08) — but keep the widget decoupled so 08 is a drop-in.

**Done when:** typing a query in FIND highlights tokens correctly in real time and surfaces parse errors inline.
