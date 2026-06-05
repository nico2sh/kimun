# Phase 10 — Mouse parity + config surface

**Objective:** Final pass — guarantee **keyboard ↔ mouse parity** everywhere, and expose **config** for keys, timeout, and theme.

**Read first:** spec §10 (mouse), §8a (gateway/timeout config), §1 (themes), §13 (global checklist).

**Tasks**
1. **Mouse parity audit** (§10) — make sure each works:
   - click = focus panel / select item
   - double-click = open note in editor
   - click `[[link]]` = follow · click `#tag` = run tag query
   - right-click = context menu (file & note ops)
   - drag divider = resize drawer↔editor
   - scroll = scroll focused pane
   - drag row = move file / reorder
   - click breadcrumb segment = jump up the tree
   - click rail item = switch/toggle drawer
   Cross-check: every Tier-0/Tier-1 keyboard action has a mouse path and vice-versa.
2. **Config surface:** a real config file/section for leader **gateway key**, which-key **timeout**, **default theme**, and any keybinding overrides. Surface theme + key info in the `⚙ CFG` drawer and the `Ctrl-K ?` cheatsheet.
3. **Context menus:** implement the right-click menu for files (rename/move/delete/new) and notes (copy link, export, etc.), mirroring the `Ctrl-K m` branch.
4. **Final sweep against the global acceptance checklist (§13).**

**Acceptance** (spec §10, §13)
- [ ] Full keyboard ↔ mouse parity confirmed item by item.
- [ ] Gateway key, timeout, and theme are user-configurable and documented.
- [ ] All §13 global checklist items pass.

**Out of scope:** the §12 out-of-scope items (multi-pane splits, graph viz, remote sync) stay stubbed.

**Done when:** the app satisfies the entire §13 checklist and a mouse-only and keyboard-only user can each drive every feature.
