# TUI support for frontmatter properties — design

Status: draft, deferred to a later version than the properties release.
Builds on: `2026-10-02-frontmatter-properties-design.md` (core, CLI, MCP and
search DSL are done there; this spec covers only the TUI).

## Goal

Make properties usable from the TUI without typing the search DSL by heart
or editing the frontmatter blind. Core already does all the work (parsing,
typed writes, indexing, `prop:`/`%` filters, `or:prop:`/`^%` sorting); the
TUI only presents it and calls core. Per `CLAUDE.md`, no file or path logic
lands in the TUI: every read and write goes through `NoteVault` /
`kimun_core` functions, and the query string is built with core's
directive/quoting helpers.

## Scope

1. Sort dialog: property sort (the former "batch D") — **committed**.
2. Further items, listed in "Candidate items" below — **to be confirmed**,
   each one decided and specified before it is planned.

---

## 1. Property sort in the Ctrl+R sort dialog

### Problem

A query sorted by a property (`%due ^%due`, `or:prop:due`) shows up in the
query panel, but:

- `QueryPanel::current_order` maps `OrderBy::Property { .. }` to
  `(Name, Ascending)` (`tui/src/components/query_panel.rs:433`), so the
  title and the sort dialog claim the results are sorted by name.
- Opening Ctrl+R and toggling anything rewrites the directive through
  `with_order_directive(query, OrderField, asc)`, which only knows `title`
  and `file` — the property sort is silently dropped.
- There is no way to *choose* a property sort from the dialog.

### Design

**Core (`core/src/index/search_terms.rs`)**

- `OrderField` gains `Property(String)` (the key as written; quoting is
  core's job). `with_order_directive` renders it as `^%key` / `-^%key`
  (descending form — reuse whatever `strip_order_directive` and the
  tokenizer already accept), quoting keys with spaces via the existing
  quoting helper so `strip_order_directive` round-trips it.
- Test: `with_order_directive(q, Property("due date"), false)` then
  `SearchTerms::from_query_string` yields `OrderBy::Property { key: "due
  date", asc: false }`, and re-applying a different field replaces it
  instead of piling a second directive.

**TUI**

- `SortField` gains `Property(String)`. It is query-panel only: the
  sidebar sorts directory listings, which have no index rows, so its
  dialog keeps cycling Name/Title. `SortFieldSetting` (persisted default)
  is unchanged — a property sort is per query, never a saved default.
- `current_order` maps `OrderBy::Property { key, asc }` to
  `(SortField::Property(key), order)`; the panel title shows
  `sorted by <key> ↑/↓`.
- Sort dialog, query-panel target:
  - The "Sort by" row cycles Name → Title → Property.
  - On Property, a second row "Key" appears, with a text field. Typing a
    key filters a list of known keys (from the index, see below); Enter or
    Right accepts. Until a key is chosen nothing is emitted, so the live
    preview never sorts by an empty key.
  - A query already sorted by a property opens with Property selected and
    its key filled in.
- Known keys: a new core read, `NoteVault::property_keys() ->
  Vec<String>` (distinct keys from `property_keys`, display spelling,
  sorted case-insensitively). Also reused by candidate item B.
- `AppEvent::SortChanged` carries `SortField`, so the new variant flows
  through without a new event; `persist = true` with a property field is
  ignored (only the sidebar persists, and it never has one).

### Corner cases

- Key with spaces or accents → quoted by core, sorts the same as typed in
  the search box.
- Key that no note has → empty sort keys; notes keep their relative order
  and "missing values sort last" applies to all (already core behaviour).
- Query with a property sort *and* the user switching back to Name →
  property directive removed, `or:file` added.
- Help dialog (F1) lists the new row.

### Testing

- Core unit tests above.
- `sort_dialog.rs` tests: cycling reaches Property; no emit before a key is
  accepted; emit carries the key; opening with a property-sorted query
  pre-selects it.
- `query_panel.rs` test: a `^%due` query keeps its directive after the
  dialog toggles order (regression for the drop described above).

### Effort

About 2–3 hours.

---

## 2. Candidate items (to confirm)

Each is a proposal. Add, remove or reshape before it gets planned.

| Id | Item | Notes | Rough effort |
|----|------|-------|--------------|
| A | Properties view in the note drawer | Read-only list of the open note's properties (key, type, value) via `NoteDetails::property_set_of`; select a row to jump to it in the frontmatter. | 3–4 h |
| B | `%` key autocomplete in the search box | After `%` / `prop:`, suggest keys from `property_keys()`; accept inserts the key quoted by `quote_query_term`. Same controller as the `#` / link suggestions. | 2–3 h |
| C | Value autocomplete after `%key=` | Distinct values for that key from the index (needs a small core read, top-N by frequency). | 2–3 h |
| D | Set / remove a property from the TUI | Small dialog: key, value(s), optional type; calls `NoteVault::set_property_from_input` / `remove_property`, surfaces the mismatch refusal message and offers "store as <type> anyway". The editor buffer must reload or be patched after the write (no TUI-side file writes). | 4–6 h |
| E | Frontmatter folding / styling in the editor | Dim or fold the `+++` / `---` block; purely presentational. | 2–4 h |

## Open questions

- Should a property sort be offered on the sidebar too (sorting notes in a
  directory by a property)? Not in item 1; it would need the sidebar to
  query the index instead of the directory listing.
- Item D: write against the editor's unsaved buffer or the file on disk?
  Writing to disk with unsaved edits needs a save-first or merge rule.
