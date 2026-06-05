# Phase 02 — Layout shell + focus model

**Objective:** Replace the three-panel layout with the target shell: **activity rail + one drawer + editor**, a top title bar, and a two-line status bar. Establish the **focus model** (non-modal).

**Read first:** spec §2 (Target layout), §7 (title/status bars), Appendix A.1 (ASCII). Reference `../target-activity-rail.png`.

**Tasks**
1. Build the region skeleton, left→right: **Activity Rail** (fixed ~7 cols), **Drawer** (one panel, resizable, hideable), **Editor** (fills remainder). Add the **title bar** (top) and **status bar** (bottom, 2 lines).
2. Render only **one drawer panel at a time** (content comes in Phase 03 — stub each rail view with a placeholder for now).
3. **Focus model:** track which region/field holds focus. Draw the focused panel with `focus_border` (green), others with `border_dim`. `Tab` / `Shift-Tab` cycles Rail → Drawer → Editor.
4. **Drawer toggle** (`Ctrl-B`): hiding it gives the full remaining width to the editor; showing it restores the last view.
5. **Divider** between drawer and editor is drag-resizable (mouse) with sensible min/max widths.
6. Panel chrome helper: a reusable "panel" that draws a border with its **title embedded in the top border** (`┌─ Title ──┐`) and colors the border by focus state.
7. Title bar: `Kimün` + breadcrumb of the open note + right-aligned workspace badge `⊙ work`. Status bar: two lines wired but content can be minimal (Phase 04 enriches it).

**Acceptance** (spec §2, §7)
- [ ] Exactly one drawer renders at a time; editor reclaims width when it's hidden.
- [ ] Focus cycles Rail→Drawer→Editor; focused region shows the green border.
- [ ] Non-modal: there is **no** Normal/Insert state anywhere — focus is the only state.
- [ ] Divider drag resizes drawer↔editor.

**Out of scope:** real drawer contents (Phase 03), rich status hints (Phase 04), leader keys (Phase 05).

**Done when:** you can Tab between rail/drawer/editor, toggle and resize the drawer, and the editor still edits the current note inside the new shell.
