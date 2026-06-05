# Phase 03 — Activity rail + drawers

**Objective:** Fill the shell with a working **activity rail** and its **drawer panels**, porting today's left browser → FILES and right query panel → FIND.

**Read first:** spec §3 (rail), §4 (drawers), Appendix A.1. Reference `../target-activity-rail.png`. Review the current browser & query-panel code to port their logic.

**Tasks**
1. **Activity rail** with items in order: `▤ FILES`, `⌕ FIND`, `# TAGS`, `↩ LINKS`, `≡ OUTL`, and `⚙ CFG` pinned to the bottom. Active item shows a green left-edge bar + green glyph. Click selects + opens; clicking the active item toggles the drawer.
2. **Shared rich-row** component for lists: `glyph · title · right-meta` with optional secondary line and a dim italic filename line (see §4 sample). Plus a distinct **cursor** indicator (thin green outline / `›`) separate from the **selection bar** (teal).
3. **FILES drawer** (port of left browser): clickable breadcrumb + file count, inline `/` filter, `..` row, dirs in `blue`, files in `fg`, optional Pinned section. Enter/double-click opens in editor.
4. **FIND drawer** (port of right query panel): query input at top (highlighting arrives in Phase 07 — leave a plain input now), results list with expandable inline preview, a sort control. This is the **persistent** query workspace (the ephemeral telescope modal is Phase 08).
5. **TAGS drawer:** list of `#tags` + counts; Enter/click runs the tag query (feeds FIND).
6. **LINKS drawer:** sub-tabs `backlinks / outgoing / unlinked`; entries open notes. Reflects the **currently open** note.
7. **OUTLINE drawer:** headings of the current note; Enter jumps the editor to the heading.
8. Wire rail selection ↔ drawer content; LINKS/OUTLINE update when the open note changes.

**Acceptance** (spec §3, §4)
- [ ] All six rail items present; active item visually distinct; CFG pinned bottom.
- [ ] All five content drawers render with shared chrome + rich rows.
- [ ] FILES and FIND reproduce today's behavior in the new layout.
- [ ] LINKS and OUTLINE track the open note.

**Out of scope:** query highlighting (07), telescope modal (08), leader paths to open drawers (wire in 05/06 — direct click/Tab is enough now).

**Done when:** clicking each rail item shows a functional drawer, and FILES + FIND match the old browser/query behavior.
