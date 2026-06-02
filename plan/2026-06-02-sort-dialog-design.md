# Sort dialog — design

Date: 2026-06-02
Status: approved (brainstorm)

## Problem

The sidebar has two sort keybindings: `CycleSortField` (Ctrl+N, toggles
Name/Title) and `SortReverseOrder` (Ctrl+R, toggles order). The query/backlinks
panel shares the same two-shortcut pattern. This is awkward and doesn't expose a
way to group directories.

Replace the two shortcuts with **one** that opens a sort dialog. The dialog
exposes sort field, order, and (sidebar only) a "group directories" toggle.

## Scope

- **Sidebar** (`tui/src/components/sidebar.rs`)
- **Query/backlinks panel** (`tui/src/components/backlinks_panel.rs`)

Directory grouping applies to the sidebar only (the query panel has no
directory entries).

## Keybinding

- Remove actions `CycleSortField` and `SortReverseOrder` from
  `tui/src/keys/action_shortcuts.rs` and their defaults in
  `tui/src/settings/mod.rs`.
- Add one action `OpenSortDialog`. Default keybind: **Ctrl+N** (reuses the freed
  `CycleSortField` slot; Ctrl+S is unavailable — already bound to Strikethrough).
  Ctrl+R is freed entirely. The action is dispatched centrally in `editor.rs`
  (not panel-intercepted); it opens the dialog targeting whichever panel is
  focused (sidebar or query panel).
- Keep legacy config aliases: `"CycleSortField"` and `"SortReverseOrder"` parse to
  `OpenSortDialog` (mirrors the existing `ToggleBacklinks` → `ToggleQueryPanel`
  alias) so existing config files don't error.

## Dialog component

New `tui/src/components/dialogs/sort_dialog.rs`, routed through `ActiveDialog`
(`dialogs/mod.rs`). Follows the `help_dialog` overlay pattern: `Clear` widget +
`fixed_centered_rect()` + `Block::default()`, closes on `Enter`/`Esc` via
`AppEvent::CloseOverlay`.

Rows (navigable ↑/↓, value changed in place with Space or ←/→):

- **Sort by**: Name / Title
- **Order**: Ascending ↑ / Descending ↓
- **Group directories**: On / Off  — *sidebar context only; hidden for the query
  panel.* Grouping clusters directories first (then notes/attachments).

Keys:

- `Enter` — confirm and close the dialog
- `Esc` — close the dialog
- `s` — save current selection as the default (context-aware, see Settings)

Both `Enter` and `Esc` close via `AppEvent::CloseOverlay`. Because changes apply
live as rows are toggled, neither discards the selection.

Changes apply **live** to the owning component as the user toggles them. `s`
additionally persists to settings.

The dialog is opened with a context describing which component owns it (sidebar
vs query panel) and the component's current sort state, so it can drive the
correct live target and show/hide the group-directories row.

## Sidebar wiring

- Add a shared `group_dirs: Arc<Mutex<bool>>` flag alongside the existing
  `sort: Arc<Mutex<(SortField, SortOrder)>>`.
- Dialog mutates the shared sort tuple and/or `group_dirs`, then triggers a
  reload.
- Implement grouping in `DirListingSource::load`: after draining entries,
  if `group_dirs` is on, partition into directories vs notes/attachments, sort
  each group by field/order, then emit directories first (after the `Up` row).
  When off, keep the current single combined sort.
- Remove the old `cycle_sort()` / `reverse_sort()` methods and their combo
  interception; replace with `OpenSortDialog` interception.

## Query panel wiring

- Remove the panel's separate UI-sort override (`sort` state + `sort_entries`
  callback). The **query string becomes the single source of truth** for sort.
- The dialog rewrites the query string: strip any existing order directive and
  append the new one (`or:title` / `-or:title` / `or:file` / `-or:file`).
- Add a core function (DSL knowledge stays in core, `core/src/db/search_terms.rs`)
  that replaces/inserts/strips the order directive in a query string, returning
  the rewritten string.
- Panel calls `set_query()` with the rewritten string → reload.
- No group-directories row for this context.

## Settings

- Add `group_directories: bool` (single global flag, shared by sidebar
  contexts).
- **Save as default** (`s` in dialog) is **context-aware**:
  - Sidebar showing a **journal** directory → write field/order to the journal
    sort defaults (`journal_sort_field` / `journal_sort_order`).
  - Sidebar showing a **non-journal** directory → write to the normal sort
    defaults (`default_sort_field` / `default_sort_order`).
  - `group_directories` is written as the single global flag regardless of
    context.
  - Query panel → persist the order choice in whatever the panel's default-query
    mechanism is (or no-op if it has none; resolve during implementation).

## Testing

- **Core**: order-directive rewrite fn — insert into a query with no directive,
  replace an existing one, strip + re-add, round-trip serialization for all four
  field/order combinations.
- **TUI**:
  - Sidebar grouped load: directories emitted first, each group sorted by
    field/order; ungrouped load matches current behavior.
  - Dialog toggle → mutates the correct shared state and triggers reload.
  - Query panel: dialog selection produces the correct rewritten query string
    via `set_query()`.

## Out of scope

- New sort fields (date/created/modified) — keep Name/Title only (matches the
  query DSL, which only supports Title/FileName).
- Dirs-last option — grouping is dirs-first only.
