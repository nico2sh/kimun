# TUI support for frontmatter properties — design

Status: approved, two items committed (sort, properties dialog).
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

Both new surfaces work with the keyboard **and** the mouse: every row,
field, suggestion and button is clickable.

## Scope

1. Sort dialog: property sort (the former "batch D") — **committed**.
2. Properties dialog: view, add, edit, rename and remove the open note's
   properties (former candidate D) — **committed**.
3. Shared pieces both need: mouse events routed into dialogs, a clickable
   button row, a key picker with suggestions, `NoteVault::property_keys()`.
4. Remaining candidates (A, B, C, E) stay deferred; see the table at the end.

---

## Shared pieces

### Core reads

- `NoteVault::property_keys() -> Vec<String>`: distinct keys from the
  index's `property_keys` table, sorted. The index stores keys only in
  their search form (lowercased, accents stripped), so that is what this
  returns; it is the form `^%key` sorts and `%key` filters by anyway. Used by the sort dialog's
  Key row and the properties form's Key field (and later candidate B).

### Mouse into dialogs

`ActiveDialog::handle_input` today returns `NotConsumed` for anything that
is not `InputEvent::Key`. It now forwards `InputEvent::Mouse` to a
per-dialog `handle_mouse`; dialogs without one consume and ignore the event
(same visible behaviour as now). Hit-testing follows the existing pattern
(`search_list`, `text_editor`): rects are stored at render time and tested
with `Rect::contains(Position)`. Clicks outside the modal do nothing.

### `components/widgets/button_row.rs`

Renders a row of `[ Label ]` buttons, records each button's rect during
render, and exposes `hit(pos) -> Option<usize>`. Supports a focused button
(for `Tab` navigation) and disabled buttons (rendered dim, never hit).

### `components/widgets/key_picker.rs`

A single-line text field plus a filtered suggestion list built from
`property_keys()`. Filtering is case-insensitive substring. Keys: typing
filters, `↑↓` move in the list, `Enter`/`→` accept the highlighted
suggestion, `Esc` closes the list first (a second `Esc` belongs to the
host). Mouse: click the field to focus it, click a suggestion to accept it.
Free text is always allowed (a key no note has yet).

---

## 1. Property sort in the Ctrl+R sort dialog

### Problem

A query sorted by a property (`%due ^%due`, `or:prop:due`) shows up in the
query panel, but:

- `QueryPanel::current_order` maps `OrderBy::Property { .. }` to
  `(Name, Ascending)` (`tui/src/components/query_panel.rs:414`), so the
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
  - On Property, a second row "Key" appears, holding a key picker. Until a
    key is accepted nothing is emitted, so the live preview never sorts by
    an empty key.
  - A query already sorted by a property opens with Property selected and
    its key filled in.
- Mouse: clicking a row selects it and toggles it (same as `Space`);
  clicking a suggestion accepts it.
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

- Core: `with_order_directive(q, Property("due date"), false)` then
  `SearchTerms::from_query_string` yields `OrderBy::Property { key: "due
  date", asc: false }`, and re-applying a different field replaces it
  instead of piling a second directive.
- `sort_dialog.rs`: cycling reaches Property; no emit before a key is
  accepted; emit carries the key; opening with a property-sorted query
  pre-selects it; a click on a row toggles it; a click on a suggestion
  accepts it.
- `query_panel.rs`: a `^%due` query keeps its directive after the dialog
  toggles order (regression for the drop described above).

---

## 2. Properties dialog

### Write model

The dialog writes to the **file on disk** through core, never to the
editor buffer:

1. Opening the dialog flushes the editor buffer with the same save used on
   leave/quit (`try_save`). If the buffer is still dirty afterwards (the
   save failed or timed out), the dialog does not open and "save failed —
   properties not opened" is flashed.
2. The dialog loads `NoteVault::get_properties(path)`.
3. Each confirmed change (save, delete) is one core call, written
   immediately.
4. After every successful write the dialog sends
   `AppEvent::NoteReloadFromDisk(path)`. The editor reloads the text with
   `get_note_text` + `set_text` + `mark_saved`, as `on_note_renamed`
   already does. The dialog is modal, so the buffer cannot change while
   it is open.

Trade-off accepted: the editor's undo history does not span the reload.

A note with no frontmatter gets a new TOML block (`FrontmatterFormat::Toml`,
same default as the CLI).

### Core additions

- `NoteVault::rename_property(path, old, new) -> Result<(), VaultError>`:
  renames in place, keeping the entry's position and value, in both TOML
  and YAML blocks. Refuses when `new` already exists (same duplicate
  error the parser reports) or `old` is missing.
- No bare-key writes: TOML cannot hold a key without a value, so an empty
  Value field is refused in the form ("value required"). Exception: editing
  a bare key (YAML `due:`) and changing only its key renames it and leaves
  it valueless.

### Opening it

- Leader: `<leader> n p` — "properties", under `+note`.
- Command palette: "Note properties".
- Status bar, line 2: a clickable segment `⊞ 3 props` (`⊞ props` when the
  note has none) whenever a note is open. The footer records the
  segment's rect at render time; the screen's mouse routing checks the
  footer before `PanelSet`. The count lives in `DocMeta`, loaded async
  with the same staleness guard as the backlink count, refreshed on note
  open, save and after any dialog write.
- No note open (or an attachment) → the leader and palette entries flash
  "no note open"; the status segment is absent.

### Layout

One modal with two states, `List` and `Form { mode: Add | Edit(orig_key) }`.

```
┌ Properties: meeting-notes ───────────────┐
│  Key        Type      Value              │
│▶ status     text      draft              │
│  due        date      2026-10-12         │
│  tags       list      work, q4           │
│  priority   number    2                  │
│                                          │
│ [+ Add]  [Edit]  [Delete]       [Close]  │
└ ↑↓ select · Enter edit · a add · d del ──┘

┌ Edit property ───────────────────────────┐
│ Key    [due                ]             │
│ Type   ‹ auto ›   (currently date)       │
│ Value  [2026-10-12         ]             │
│                                          │
│              [Save]  [Cancel]            │
└──────────────────────────────────────────┘
```

### List state

- Columns: key · type · value. Long values are truncated with `…`. A bare
  key shows type `—` and an empty value. Values use `PropertyValue`'s
  `Display` (list items joined with `, `).
- Keys: `↑↓` / `j k` select · `Enter` / `e` edit · `a` add · `d` / `Del`
  delete · `Tab` moves focus between the list and the button row · `Esc`
  closes.
- Mouse: click a row selects it; clicking the already-selected row, or a
  double-click, opens it for editing. The wheel scrolls. Buttons
  `[+ Add] [Edit] [Delete] [Close]`; Edit and Delete are disabled with no
  selection.
- Delete asks inline in the footer: `Delete "due"? y/n` with clickable
  `[Yes] [No]`. `Esc` counts as no.
- Empty state: "No properties. Press a or click + Add."
- Read-only: when the note cannot be read, or a write fails because its
  frontmatter block is malformed (`FSError::InvalidFrontmatter`), the error
  is shown in place of the list and every action except `[Close]` is
  disabled. (`get_properties` reads a malformed block leniently as "no
  properties", so malformation surfaces on the first write; detecting it
  earlier would be format logic in the TUI.)

### Form state

- Fields: Key (key picker) · Type cycler `auto, text, number, bool, date,
  datetime, list` · Value (single-line text; comma-separated for lists).
- Add starts empty. Edit pre-fills key and value; Type starts at `auto`
  and the current kind is shown beside it as a hint.
- Keys: `Tab` / `Shift+Tab` move through fields and buttons · `←→` /
  `Space` on Type cycle it · `Enter` saves from any field, except when
  the key picker's suggestion list is open (then it accepts the
  suggestion) · `Esc` returns to the list without writing.
- Mouse: click a field to focus it (cursor at the end); click `‹` / `›` to
  cycle the type; click a suggestion to accept it; `[Save] [Cancel]`.
- Values are split on commas into `PropertyInput` values only when the
  type is `list` (or the key is a list key core already knows); otherwise
  the field is one value. Type `auto` → no forced kind; any other choice
  → `.forced(Some(kind))`.

### Save logic

- Empty key → inline error "key required", nothing written.
- Add, or Edit with the key unchanged → `set_property_from_input`.
- Edit with the key changed → `rename_property(orig, new)`, then
  `set_property_from_input` on the new key. A failed rename writes
  nothing; a failed set after a successful rename leaves the rename in
  place, reloads the list and shows the error (the file stays valid).
- Delete → `remove_property`.
- Success → back to the list (re-read with `get_properties`), status bar
  flashes "property saved" / "property removed".

### Type mismatch

When core refuses an input saved with type `auto` with
`VaultError::InvalidProperty` (the key holds another type in the vault),
the form shows core's message in
the error colour under Value plus a button
`[Store as <kind> anyway]`. `<kind>` is the type the input infers to:
`PropertyValue::infer` for one value, `list` when comma-splitting gave
several. Pressing it (or `Enter` while it is focused) retries with
`.forced(Some(kind))`. It is a shortcut for choosing that type in the
cycler and saving again.

Other errors (a value that does not parse as the forced type, duplicate
key on rename, IO) are shown the same way, without the button; the form
stays open with the input kept.

### Async plumbing

Core property calls are `async`. Like the move and rename dialogs, the
dialog holds the `Arc<NoteVault>` and spawns each core call itself; the
result comes back as an `OverlayData` variant through
`Overlay::handle_data`. While a call is in flight the buttons are disabled,
so a double click cannot write twice.

### Testing

- Core: `property_keys` (distinct, sorted, folded duplicates collapse);
  `rename_property` (keeps position and value in TOML and YAML, refuses an
  existing target, refuses a missing source).
- `button_row`: hit-testing after a render, a click in the gap between
  buttons hits nothing, disabled buttons are never hit.
- `key_picker`: case-insensitive filtering, `Enter` and click accept,
  `Esc` closes the list before anything else.
- `properties_dialog` (requests checked on the channel, results fed back
  through `handle_data`):
  - list navigation; click selects, second click edits
  - add, edit and delete emit the right request
  - delete confirmation (y, n, Esc, clicks)
  - mismatch shows the button, and the button retries with the forced kind
  - key change → rename then set
  - empty key / empty value errors, no request
  - malformed frontmatter → read-only list
  - buttons disabled while a request is in flight
- Status bar: segment rect recorded; a click inside it emits the open
  event; no segment without an open note.
- Editor: `NoteReloadFromDisk` replaces the buffer and marks it clean.
- One `editor.rs` integration test against a temp vault: open the dialog,
  set a property, check the file and the reloaded buffer.

---

## Docs

`docs/` (user-facing):

- Properties page: new section "Editing properties in the TUI" — how to
  open the dialog (leader, palette, status bar), keys, mouse, the "store
  as … anyway" button.
- Sorting / keybindings pages: property sort in Ctrl+R and the
  `<leader> n p` entry.

## Delivery order

Each step leaves the suite green before the next starts.

1. Core: `OrderField::Property`, `property_keys`.
2. `button_row`, `key_picker`, mouse routed into dialogs.
3. Sort dialog (item 1).
4. Core: `rename_property`.
5. Properties dialog and `NoteReloadFromDisk`.
6. Entry points: leader, palette, status bar segment.
7. Docs.

---

## Deferred candidates

| Id | Item | Notes | Rough effort |
|----|------|-------|--------------|
| A | Properties view in the note drawer | Read-only list of the open note's properties; select a row to jump to it in the frontmatter. Partly covered by the dialog. | 3–4 h |
| B | `%` key autocomplete in the search box | After `%` / `prop:`, suggest keys from `property_keys()`; accept inserts the key quoted by `quote_query_term`. Same controller as the `#` / link suggestions. | 2–3 h |
| C | Value autocomplete after `%key=` | Distinct values for that key from the index (needs a small core read, top-N by frequency). | 2–3 h |
| E | Frontmatter folding / styling in the editor | Dim or fold the `+++` / `---` block; purely presentational. | 2–4 h |

## Resolved questions

- Property sort on the sidebar: no. The sidebar lists directories, not
  index rows; revisit only if the sidebar ever queries the index.
- Properties dialog write target: the file on disk, after flushing the
  buffer, followed by a buffer reload (see "Write model").
