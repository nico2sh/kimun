# Kimün — Evolution Spec & Build Prompt

> **How to use this doc:** it's both a human-readable spec and a prompt you can hand to a coding agent. Fill in the two placeholders in §0, then implement section by section. Each section ends with **Acceptance** bullets you can check off. Nothing here changes the *data model* (local markdown vault, index, query language) — it's a UI/UX evolution of the existing TUI.

---

## 0. Context the implementer needs

- **App:** Kimün — a terminal (TUI) note-taking app over a local vault of markdown files (Obsidian-like: indexed vault, wikilinks `[[…]]`, `#tags`, backlinks, and a query language with operators).
- **Stack:** `‹FILL IN: e.g. Rust + ratatui / Go + bubbletea / …›`
- **Repo entry points:** `‹FILL IN: where the layout, input handling, and render loop live›`
- **Today's layout (baseline):** three vertical panels — left = file/note browser, center = editor, right = query bar + results (expandable preview). Both side panels toggle. Gruvbox-dark styling, panel titles drawn into the top border, **green border = focused panel**, **teal bar = selection**, aqua underlined wikilinks, two-line status bar, `⊙ work` workspace badge top-right.
- **Goal:** evolve to an **activity-rail layout** with a unified **leader-key** command system, a **which-key help overlay**, **query syntax highlighting**, and a richer **editor screen** — while **keeping the existing telescope-style search modal**.

**Hard constraints**
- Terminal-friendly: monospace grid, box-drawing borders, no reliance on hover. Every keyboard action has a mouse equivalent and vice-versa.
- **Non-modal editor.** Do **not** introduce vim-style Normal/Insert modes. Behaviour is driven by **focus** (which panel/field holds the cursor), which is already shown by the green border.
- Mouse support level: click-to-focus/select, scroll, click wikilinks/tags, drag dividers to resize, right-click context menus. (No need for elaborate drag-drop beyond file move.)

---

## 1. Design tokens — theme roles (Gruvbox Dark = reference default)

> **The app is themeable.** Gruvbox Dark below is the **reference/default theme**, not a hardcoded palette. Define every color as a **semantic role**; widgets reference roles only (`theme.focus_border`, `theme.selection_bg`, …), never raw hex. A theme is just a table that binds these roles to colors, and the user can switch themes at runtime (`Ctrl-K v c` → theme picker). Ship Gruvbox Dark as the default and at least one alternate (e.g. a light theme) to prove the abstraction holds.

Use truecolor where the terminal supports it; otherwise map each role to the ANSI-16 slot in the right column (the standard gruvbox terminal mapping, satisfying the "classic 16-color" compatibility goal). Themes may target truecolor, 256, or 16-color and the role layer stays the same.

| Role | Hex | ANSI-16 |
|---|---|---|
| `bg` (app background) | `#282828` | 0 / bg |
| `bg_hard` (modal/input bg) | `#1d2021` | 0 |
| `bg_soft` (rows, hr) | `#3c3836` | 8 |
| `border_dim` (unfocused) | `#665c54` | 8 (bright black) |
| `fg` (body text) | `#ebdbb2` | 7 |
| `fg_bright` (titles) | `#fbf1c7` | 15 |
| `gray` (muted/meta) | `#928374` | 8 |
| `red` (negation, errors) | `#fb4934` | 9 |
| `green` (**focus border**, OK) | `#b8bb26` | 10 |
| `yellow` (fields, keycaps) | `#fabd2f` | 11 |
| `blue` (wikilink target) | `#83a598` | 12 |
| `purple` (numbers) | `#d3869b` | 13 |
| `aqua` (tags, links, groups) | `#8ec07c` | 14 |
| `orange` (operators, accents) | `#fe8019` | bright (256: 208) |
| `selection` (teal bar) | `#3a5f63` bg / `#fbf1c7` fg | 6 bg |

**Visual vocabulary (keep & apply everywhere):**
- Panel = single-line box; **title embedded in the top border**: `┌─ Editor · 2026-04-11.md ───────┐`.
- Focused panel border = `green`; unfocused = `border_dim`.
- Selection within a list = full-width `selection` bar, text recolored to `fg_bright`.
- A separate **cursor** indicator (the item the keyboard will act on) = thin green outline / `›` marker, distinct from selection bar.
- Two-line status bar pinned to the bottom, full width.

**Acceptance**
- [ ] A single theme module exposes these as **named roles**; no raw hex in any widget.
- [ ] Themes are switchable at runtime; Gruvbox Dark is the default + at least one alternate ships.
- [ ] Truecolor and 16-color terminals both render with correct role semantics.

---

## 2. Target layout

```
┌─ Kimün ──  vault/journal/2026-04-11.md ─────────────────────  ⊙ work ─┐
│▎FILES│┌─ Files · journal/ ─┐┌─ Editor · 2026-04-11.md ─────────────────┐│
│ FIND ││ /filter…           ││ # 2026-04-11                              ││
│ TAGS ││ ▸ ..               ││ ## Standup                                ││
│ LINKS││ ▤ 2026-04-11   ◀sel││ - Today: feature flag rollout #rollout    ││
│ OUTL ││ ▤ 2026-04-10       ││ see [[search-caching]] …                  ││
│      ││ …                  ││                                           ││
│ CFG  │└────────────────────┘└───────────────────────────────────────────┘│
├───────────────────────────────────────────────────────────────────────────┤
│ ⌨ EDITOR   Ctrl-K menu   Ctrl-F find   ⏎ follow link        Ctrl-S save    │
│ journal/2026-04-11.md · ln 42 col 18 · ● modified · 2 backlinks · git ✓   │
└───────────────────────────────────────────────────────────────────────────┘
```

Regions, left → right:
1. **Activity Rail** — fixed-width icon strip (≈ 7 cols incl. border). Always visible. §3.
2. **Drawer** — one panel that renders whichever rail item is active. Toggle with `Ctrl-B`. Resizable (drag divider / `Ctrl-K w` resize cmds). §4.
3. **Editor** — fills remaining width; expands to full width when the drawer is hidden. §5.
4. **Title bar** (top) — workspace badge + breadcrumb of the open note. §7.
5. **Status bar** (bottom, 2 lines). §7.
6. **Telescope modal** — floats centered over everything when invoked. §6. **(kept from current app.)**

**Migration note:** today's *left browser* becomes the **FILES** drawer; today's *right query panel* becomes the **FIND** drawer. Net effect: the editor reclaims width because only one drawer is open at a time.

**Acceptance**
- [ ] Exactly one drawer panel renders at a time, selected by the rail.
- [ ] Hiding the drawer (`Ctrl-B`) gives the full remaining width to the editor.
- [ ] Focus cycles Rail → Drawer → Editor with `Tab` / `Shift-Tab`; focused region shows the green border.

---

## 3. Activity Rail

A vertical strip of icon + tiny-label cells. Click selects (and reveals the drawer); the active cell shows a green left-edge bar and green glyph.

| Order | Glyph | Label | Opens | Leader |
|---|---|---|---|---|
| 1 | `▤` | FILES | file/note browser drawer | `Ctrl-K o f` |
| 2 | `⌕` | FIND | query workspace drawer (persistent) | `Ctrl-K o q` |
| 3 | `#` | TAGS | tag browser drawer | `Ctrl-K o t` |
| 4 | `↩` | LINKS | links drawer (backlinks / outgoing / unlinked) | `Ctrl-K o k` |
| 5 | `≡` | OUTL | outline (headings) of current note | `Ctrl-K o l` |
| — (spacer pushes last item to bottom) | | | | |
| 6 | `⚙` | CFG | settings / keymap / theme | `Ctrl-K v c` |

Behaviour:
- Click a rail item → drawer switches to it and the drawer takes focus. Clicking the **active** item toggles the drawer closed/open.
- Keyboard: the leader paths above; also `Ctrl-B` toggles the drawer showing the last view.
- The rail itself is focusable (j/k or ↑/↓ to move, `Enter` to open) but most users drive it via clicks or leader keys.

**Acceptance**
- [ ] Active rail item is visually distinct (green edge bar + glyph).
- [ ] Each item opens its drawer via both click and leader path.
- [ ] CFG is pinned to the bottom of the rail.

---

## 4. Drawer panels

All drawers share the panel chrome (title-in-border, green border when focused) and the **rich list row** format used in the current app:

```
▤ Auth Flow Meeting              04-08
  attendees: maria, david              ← optional secondary line
  2026-04-08.md                        ← dim italic filename
```

### 4.1 FILES (was the left browser)
- Breadcrumb of current dir at top (clickable segments), file count on the right.
- Inline **filter field** (`/` to focus) that fuzzy-filters the current directory by name.
- `..` row to ascend; directories in `blue`, files in `fg`.
- Optional "Pinned" section below a `hr`.
- Enter/double-click opens in the editor. `Space` (list focused) opens the leader menu — see §8.

### 4.2 FIND (was the right query panel — **persistent**, complements the telescope modal)
- A **syntax-highlighted query input** at top (see §9), focused with `/`.
- Results list below; each result expandable to an inline preview snippet with match context.
- Sort control (e.g. `sort: modified ↓`).
- Distinction from telescope: FIND is the *docked, stays-open* query workspace; the **telescope modal (§6) is the ephemeral quick-search** you pop with `Ctrl-F`. Both share the same query parser and highlighter.

### 4.3 TAGS
- Flat or hierarchical list of `#tags` with counts; click/Enter runs the corresponding tag query (populates FIND or telescope).

### 4.4 LINKS (for the open note)
- Tabbed sub-views (`b` backlinks · `o` outgoing · `u` unlinked mentions · `g` local graph teaser).
- Each entry: note title + filename; Enter opens it.

### 4.5 OUTLINE
- Headings of the current note as an indented tree; Enter jumps the editor to that heading.

**Acceptance**
- [ ] All five drawers render with shared chrome and the rich-row format.
- [ ] FIND uses the §9 highlighter and shares the parser with telescope.
- [ ] LINKS and OUTLINE reflect the **currently open** note and update on note switch.

---

## 5. Editor screen

The editor renders the markdown source with **inline syntax highlighting** (it stays an editable text buffer — this is highlight-on-source, not a rendered/preview mode).

### 5.1 Markdown highlights
| Element | Treatment |
|---|---|
| `# / ## / ###` headings | bold; H1/H2 `fg_bright`, H3 `yellow` |
| **bold**, *italic* | weight / dim variation |
| `- ` / `* ` bullets | bullet glyph in `gray` |
| `1.` ordered | number in `purple` |
| `` `code` `` / fenced | `aqua` on `bg_soft` |
| `> blockquote` | `gray`, left guide `▏` |
| `[[wikilink]]` | `blue`, underlined, **clickable**; `Ctrl-Enter` follows when cursor is on it |
| `#tag` | `aqua`, clickable → runs tag query |
| `- [ ] / - [x]` tasks | checkbox glyph; done items dimmed/struck |
| search match (when arriving from a query) | `yellow` highlight span on the matched text |
| cursor | block cursor (`fg` bg / `bg` fg) |

### 5.2 Helpers, help text & tips (the "helper" layer)
- **Contextual key hints** live in **status line 1** and change with focus and what's under the cursor (e.g. cursor on a `[[link]]` → show `⏎ follow  Ctrl-K m c copy link`).
- **Link/tag affordance:** when the cursor enters a wikilink/tag, briefly surface its target/where-it-goes in status line 2 (e.g. `→ people/maria.md · 3 backlinks`).
- **Empty-note tip:** a new/empty note shows dim helper text in the body (e.g. *"Type to start · `[[` to link · `#` to tag · `Ctrl-K` for commands"*) that disappears on first keystroke.
- **First-run / discoverability:** pressing the leader gateway always reveals the which-key overlay (§8b) — that is the primary "help text" surface; plus `Ctrl-K ?` opens the full cheatsheet.

**Acceptance**
- [ ] All elements in 5.1 are highlighted live as the buffer changes.
- [ ] Wikilinks/tags are clickable and `Ctrl-Enter`-followable; status hints update with cursor context.
- [ ] Empty-note tip shows and dismisses correctly.

---

## 6. Telescope search modal (KEEP — spec for parity)

Preserve the existing modal; align its styling and make it the destination of "list-style" leader leaves.

- Centered floating panel (~75% width), `bg`, **green** border, drop emphasis over a dimmed app.
- **Top:** syntax-highlighted input (§9) with a left prefix glyph (`⌕` for query, `›` for commands) and a right-aligned result count.
- **Body split:** left = results list (~45%); right = **live preview** of the highlighted result (note render with match context, filename + match count header).
- **Footer:** key hints — `↑↓ move · ⏎ open · Ctrl-Space open in split · Tab mark · Esc close`.
- Invocations: `Ctrl-P` = command palette (commands, fuzzy); `Ctrl-F` = query/search (full query syntax); `Ctrl-O` = quick-open file. Same widget, different scope + prefix.

**Acceptance**
- [ ] Modal renders list + live preview and shares the parser/highlighter with FIND.
- [ ] `Ctrl-P` / `Ctrl-F` / `Ctrl-O` open it pre-scoped with the right prefix.
- [ ] `Esc` closes and restores prior focus; selection unchanged underneath.

---

## 7. Title bar & status bar

**Title bar (top, 1 line):** `Kimün` label · breadcrumb of open note (clickable segments) · right-aligned workspace badge `⊙ work`.

**Status bar (bottom, 2 lines):**
- **Line 1 — context + actions:** a **focus-context indicator** (not a vim mode): `⌨ EDITOR` when a text field holds the cursor, `≣ LIST` when a list/panel is focused. Followed by the most relevant key hints for that context; right-aligned global hints (`Ctrl-S save`, etc.).
- **Line 2 — document state:** path · `ln/col` · `● modified` / `✓ saved` · backlink count · git status · index match count where relevant.

**Acceptance**
- [ ] Line-1 context indicator and hints change with focus and cursor context.
- [ ] No "mode" is ever shown; only focus context.

---

## 8. Leader-key system

### 8a. Triggers (non-modal)
- **Universal gateway: `Ctrl-K`** (configurable). Works in *every* context, including while typing in the editor. This is the VS Code chord-prefix / Emacs `C-x` model — familiar to non-vim users, no modes.
- **Bonus trigger: bare `Space`** opens the *same* menu **only when a non-text panel (a list/rail) is focused**, where Space has no typing job. In any text field (editor, filter, query) Space inserts a space as normal.
- `Esc` cancels a pending sequence and returns focus to the editor. `Backspace` steps up one level in the sequence.
- **Timeout:** after the gateway, if the user hesitates `‹~400ms, configurable›` show the which-key overlay (§8b). Typing the full sequence quickly fires without ever showing the overlay.

### 8b. which-key help overlay
A popup docked **above the status bar**, full width, **green** border:
- **Header:** the pressed sequence as a keycap (e.g. `Ctrl-K` → then `Ctrl-K f`), a caption (`leader — pick a group` / `+find`), and right-aligned `Esc cancel · BkSp up a level`.
- **Body:** a multi-column grid of `key → target` rows. `key` in `yellow`; **groups** shown as `→ +find` in `aqua`; **leaf actions** shown as `→ description` in `fg`.
- Pressing a group key replaces the grid with that group's next level (live drill-down). The overlay is the canonical, always-correct shortcut documentation.
- `Ctrl-K ?` opens a full scrollable cheatsheet of the entire tree.

### 8c. Keymap

**Tier 0 — Ctrl/Fn (always-on; the only flat shortcuts).** Keep this list short — only things you'd press *while typing*:

| Key | Action |
|---|---|
| `Ctrl-K` | **leader gateway → menu** |
| `Ctrl-P` | command palette (telescope) |
| `Ctrl-O` | quick-open file (telescope) |
| `Ctrl-F` | query / search (telescope) |
| `Ctrl-S` | save |
| `Ctrl-B` | toggle drawer |
| `Ctrl-Enter` | follow link at cursor |
| `Tab` / `Shift-Tab` | cycle panel focus |

**Tier 1 — `Ctrl-K …` tree (group → action; `Space …` in lists).** Double the letter for each group's most-common action (e.g. `Ctrl-K f f`).

| Path | Group → actions |
|---|---|
| `Ctrl-K f` | **+find** — `f` files · `g` grep/query · `t` tags · `b` backlinks · `r` recent · `h` headings |
| `Ctrl-K n` | **+note** — `n` new · `d` daily · `t` from template · `r` rename · `m` move · `D` delete |
| `Ctrl-K l` | **+links** — `b` backlinks · `o` outgoing · `u` unlinked · `g` local graph |
| `Ctrl-K o` | **+open drawer** — `f` files · `q` find · `t` tags · `k` links · `l` outline |
| `Ctrl-K g` | **+git/sync** — `s` status · `p` sync/push · `l` log · `d` diff |
| `Ctrl-K v` | **+vault** — `s` switch vault · `r` reindex · `c` config |
| `Ctrl-K w` | **+window** — drawer/editor split, focus moves, `z` zen, resize |
| `Ctrl-K ?` | help / full cheatsheet |

**Tier 2 — `Ctrl-K m …` (this-note branch).** Note-scoped verbs live under a branch instead of a bare `,` (which would type a comma in the editor):

| Path | Action |
|---|---|
| `Ctrl-K m t` | toggle todo / checkbox |
| `Ctrl-K m p` | render preview |
| `Ctrl-K m c` | copy wikilink to this note |
| `Ctrl-K m e` | export (md / pdf / html) |
| `Ctrl-K m r` | rename across backlinks |
| `Ctrl-K m y` | yank note path |

> **Principle:** every leaf that *lists things* (files, tags, backlinks, headings…) simply opens the **telescope picker pre-scoped** to that set. One picker, many labelled doors — so the leader tree is mostly navigation, not bespoke UIs.

**Acceptance**
- [ ] `Ctrl-K` opens the menu in every context, including mid-typing in the editor.
- [ ] `Space` opens it only when a list/rail is focused; types a space in any text field.
- [ ] which-key overlay appears on hesitation, drills into groups, and matches the table above exactly.
- [ ] `Esc` cancels, `Backspace` steps up; fast full sequences fire without showing the overlay.
- [ ] The gateway key and timeout are configurable.

---

## 9. Query syntax highlighting

Tokenize the query input (in **FIND** and the **telescope** modal) and color by token class. Map these onto the **actual** grammar your parser already produces — the names below are illustrative; the **color roles** are the spec.

| Token class | Examples | Color |
|---|---|---|
| Boolean / set operators | `AND` `OR` `NOT` | `orange`, bold |
| Field / qualifier keys | `tag:` `after:` `before:` `to:` `links:` `path:` `sort:` | `yellow` |
| Tag value | `#meeting` | `aqua` |
| Wikilink value | `[[maria]]` | `blue` |
| Quoted / literal string | `"refresh token"` | `green` |
| Date / number literal | `2026-04-01` `42` | `purple` |
| Negation | `-archived` `-tag:#wip` | `red` |
| Grouping / punctuation | `(` `)` `,` `:` | `gray` |
| Plain term | bare words | `fg` |

Behaviour:
- Highlight **as the user types**, incrementally.
- On a **parse error**, underline/red the offending span and show a one-line reason in the modal footer or FIND header — never block typing.
- Offer completion for field keys, tag values, and link targets (drop-down or inline ghost text) where feasible.

**Acceptance**
- [ ] Live tokenized highlighting in both FIND and telescope, identical rules.
- [ ] Invalid spans are marked without blocking input; a reason is surfaced.
- [ ] Color roles match the table and the rest of the theme.

---

## 10. Mouse interactions (parity with keyboard)

| Gesture | Result |
|---|---|
| click | focus that panel / select item |
| double-click | open note in editor |
| click `[[link]]` | follow wikilink |
| click `#tag` | run tag query |
| right-click | context menu (file & note ops) |
| drag divider | resize drawer ↔ editor |
| scroll | scroll the focused pane |
| drag row | move file / reorder |
| click breadcrumb segment | jump up the tree |
| click rail item | switch/toggle drawer |

**Acceptance**
- [ ] Every Tier-0/Tier-1 action reachable by keyboard has a mouse path and vice-versa.

---

## 11. Suggested build order

1. **Theme module** (§1) — roles, truecolor + 16-color.
2. **Layout shell** (§2) — rail + single drawer + editor + title/status; focus cycling & green-border focus.
3. **Activity rail + drawers** (§3, §4) — port existing browser → FILES, existing query panel → FIND.
4. **Status bar v2** (§7) — focus-context indicator + contextual hints.
5. **Leader engine** (§8a) — gateway, focus-aware Space, sequence state machine, Esc/Backspace, timeout.
6. **which-key overlay** (§8b) + cheatsheet, wired to the keymap tree (§8c).
7. **Query highlighter** (§9) in FIND, then reuse in telescope.
8. **Telescope alignment** (§6) — list+preview, scoped invocations, shared parser.
9. **Editor highlight + helpers** (§5) — markdown spans, clickable links/tags, contextual tips.
10. **Mouse parity pass** (§10) + config surface for keys/timeout/theme.

---

## 12. Out of scope (don't build now)
- Multi-pane editor splits beyond a single editor (leave `Ctrl-K w v` as a stub).
- Graph visualization beyond a LINKS teaser.
- Remote/sync backends — git status display only.
- Any modal (Normal/Insert) editing model.

---

## 13. Global acceptance checklist
- [ ] Layout: rail + one drawer + editor; drawer toggles; editor reclaims width.
- [ ] Non-modal throughout; focus (green border) is the only "state".
- [ ] `Ctrl-K` leader works everywhere; `Space` leads in lists; which-key overlay matches the tree.
- [ ] Query syntax highlighting live in FIND and telescope.
- [ ] Editor highlights markdown, links, tags; contextual helpers/tips present.
- [ ] Telescope modal preserved with list + live preview.
- [ ] Gruvbox-dark theme via role tokens; 16-color fallback intact.
- [ ] Full keyboard ↔ mouse parity.

---

## Appendix A — ASCII mockups (the actionable layout reference)

> For a TUI these are more useful than pixel screenshots: they map directly onto the character grid. Reference PNGs for color/feel live in `handoff/` (`target-activity-rail.png`, `target-leader-whichkey.png`) plus your current app screenshot as the "before".

### A.1 Full screen — Activity Rail + drawer + editor
```
┌─ Kimün ──  vault / journal / 2026-04-11.md ───────────────────────  ⊙ work ─┐
│┌────┐┌─ Files · journal/ ──┐┌─ Editor · 2026-04-11.md ───────────────────────┐│
││▎▤  ││ vault/ journal     5 ││ # 2026-04-11                                   ││
││ FI ││ / filter…           ││ ## Standup                                     ││
││────││ ▸ ..                ││ - Today: feature flag rollout #rollout         ││
││ ⌕  ││▌▤ 2026-04-11   2★   ││ Deployed shadow mode — see [[search-caching]]. ││
││ FN ││  Saturday, Apr 11   ││ 1. 15,000 requests processed                   ││
││ #  ││  2026-04-11.md      ││ 2. Cache agreement rate: 99.7%                 ││
││ TG ││ ▤ 2026-04-10        ││                                                ││
││ ↩  ││  2026-04-10.md      ││ ## Notes                                       ││
││ LK ││ ▤ 2026-04-09        ││ [[carlos]] flagged Redis 7.2 #investigate █    ││
││ ≡  ││ …                   ││                                                ││
││ OL ││                     ││                                                ││
││    │└─────────────────────┘└────────────────────────────────────────────────┘│
││ ⚙  │                                                                          │
│└────┘                                                                          │
├────────────────────────────────────────────────────────────────────────────────┤
│ ⌨ EDITOR   Ctrl-K menu   Ctrl-F find   ⏎ follow link              Ctrl-S save   │
│ journal/2026-04-11.md · ln 42 col 18 · ● modified · 2 backlinks · git ✓         │
└────────────────────────────────────────────────────────────────────────────────┘
   ▎ = active rail item (green edge)   ▌ = selection bar (teal)   █ = cursor
   green border = focused panel        dim border = unfocused
```

### A.2 which-key overlay (docked above status bar, after `Ctrl-K`)
```
├────────────────────────────────────────────────────────────────────────────────┤
│ Ctrl-K  leader — pick a group · or Space when a list is focused  Esc · BkSp up   │
│ f → +find        n → +note        l → +links        o → +open drawer             │
│ m → +this note   v → +vault       g → +git/sync     ? → help / cheatsheet        │
├────────────────────────────────────────────────────────────────────────────────┤
│ ⌨ EDITOR   cursor in text → Ctrl-K leads          focus a list → Space leads too │
│ tip: keep typing — Ctrl-K f f fires instantly; the menu only waits for hesitation│
└────────────────────────────────────────────────────────────────────────────────┘
keycaps (f n l o m v g ?) = yellow · +group labels = aqua · drilling into f redraws
the grid with: f files · g grep · t tags · b backlinks · r recent · h headings
```

### A.3 Telescope modal (kept) — list + live preview
```
        ┌────────────────────────────────────────────────────────────────┐
        │ ⌕ tag:#meeting after:2026-04-01 -archived            11 results  │
        ├──────────────────────────────┬─────────────────────────────────┤
        │▌Auth Flow Meeting      04-08  │ preview  2026-04-08.md  3 matches│
        │ Sprint Planning        04-10  │ # Auth Flow Meeting              │
        │ Manager 1:1 Notes      04-05  │ attendees: [[maria]], [[david]]  │
        │ Roadmap Sync           04-03  │ ## Decisions                     │
        │ Observability Proposal 04-02  │ - Rotate refresh tokens every 24h│
        │                               │ - [[maria]] owns migration       │
        │                               │ …token ⟨rotation⟩ with maria…    │
        ├──────────────────────────────┴─────────────────────────────────┤
        │ ↑↓ move · ⏎ open · Ctrl-Space split · Tab mark · Esc close       │
        └────────────────────────────────────────────────────────────────┘
   query line uses §9 highlighting · ⟨rotation⟩ = matched span (yellow)
   border = green · invoked by Ctrl-F (query) / Ctrl-P (cmds, prefix ›) / Ctrl-O (files)
```

### A.4 Query input token coloring (FIND header & telescope input)
```
   tag:#meeting  after:2026-04-01  AND  links:[[maria]]  -tag:#archived
   └──┬─┘└──┬──┘  └─┬─┘└────┬────┘  └┬┘  └─┬─┘└──┬──┘     └┬┘└─┬┘└──┬──┘
   field  tag-val  field  date    op   field link-val   neg field tag-val
   yellow  aqua    yellow purple  orng yellow blue        red yellow aqua
```

### A.5 Editor helper states
```
Empty note (dim ghost text, clears on first keystroke):
┌─ Editor · untitled.md ───────────────────────────────┐
│ Type to start · [[ to link · # to tag · Ctrl-K cmds   │  ← gray, italic
│ █                                                     │
└───────────────────────────────────────────────────────┘

Cursor on a wikilink (status line reflects context):
│ … see [[search-caching]]█ …                           │
status 1: ⌨ EDITOR   ⏎ follow   Ctrl-K m c copy link
status 2: → projects/search-caching.md · 4 backlinks
```

