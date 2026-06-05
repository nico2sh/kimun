# Phase 01 — Theme roles & 16-color fallback

**Objective:** Introduce a single theming layer of *semantic roles* so the rest of the rebuild never hardcodes color. Gruvbox Dark is the reference **default**, but the app must support switching themes at runtime.

**Read first:** spec §1 (Design tokens — theme roles). Locate where the current app sets colors today.

**Tasks**
1. Create a `Theme` abstraction: a table mapping **named roles** → color. Roles needed now: `bg`, `bg_hard`, `bg_soft`, `border_dim`, `focus_border`, `fg`, `fg_bright`, `gray`, `selection_bg`, `selection_fg`, `cursor`, and accents `red`, `green`, `yellow`, `blue`, `purple`, `aqua`, `orange`.
2. Implement **Gruvbox Dark** binding these roles to the hexes in §1, plus the **ANSI-16 fallback** mapping for non-truecolor terminals. Detect terminal color depth and pick truecolor / 256 / 16 automatically.
3. Add **one alternate theme** (e.g. a light variant) to prove the abstraction — even a rough one.
4. Add a runtime **theme switch** entry point (a function + a temporary keybinding or command is fine; the real picker lands in Phase 06 via `Ctrl-K v c`).
5. Refactor existing widgets to read from `theme.<role>` instead of literals. Grep for raw color usage and replace.

**Acceptance** (spec §1)
- [ ] Single theme module exposes named roles; **no raw hex in any widget**.
- [ ] Themes switchable at runtime; Gruvbox Dark default + ≥1 alternate.
- [ ] Correct rendering on truecolor and 16-color terminals.

**Out of scope:** the visual layout changes (Phase 02+), the settings UI (Phase 06). Don't restyle panels yet — just route existing colors through roles.

**Done when:** the app looks the same as today but every color comes from a role, and flipping to the alternate theme visibly recolors the whole UI.
