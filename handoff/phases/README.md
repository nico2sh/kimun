# Kimün — Phased Build Prompts

Each file in this folder is a **standalone prompt for a single Claude Code session**. Run them in order; each builds on the last and ends in a working, reviewable state.

**Before every session, give the agent:**
1. The relevant phase file (below).
2. `../Kimün — Evolution Spec.md` — the full spec (the phase files reference its sections).
3. Your **current app screenshot** (the "before").
4. For visual phases: `../target-activity-rail.png` and `../target-leader-whichkey.png`.

> **Theme note for every phase:** Gruvbox Dark is the *reference/default* theme — the app is themeable. Always read colors from semantic theme roles (§1), never hardcode hex.

| # | Phase | Spec §§ | Depends on |
|---|---|---|---|
| 01 | Theme roles & 16-color fallback | §1 | — |
| 02 | Layout shell + focus model | §2, §7 | 01 |
| 03 | Activity rail + drawers (port browser & query) | §3, §4 | 02 |
| 04 | Status bar v2 (focus context + hints) | §7 | 02 |
| 05 | Leader engine (Ctrl-K, Space-in-lists) | §8a | 02 |
| 06 | which-key overlay + cheatsheet | §8b, §8c | 05 |
| 07 | Query syntax highlighter | §9 | 03 |
| 08 | Telescope modal alignment | §6 | 03, 07 |
| 09 | Editor highlights + helpers/tips | §5 | 02 |
| 10 | Mouse parity + config surface | §10, §1, §8a | all |

Each prompt has the same shape: **Objective · Read first · Tasks · Acceptance · Out of scope · Done when**. Keep changes scoped to the phase; leave stubs where a later phase takes over.
