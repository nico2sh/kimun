# Spec: Query builder

Status: draft, open items resolved (see the end). Applies to the GUI. The model is `SearchTerms` in core, so the TUI can reuse it later.

## Goals

1. A friendlier way to write search queries than the raw syntax, for users who don't know or remember the operators.
2. One builder that works from both the Find view (sidebar/drawer) and the Search dialog (`Ctrl+K`).
3. The builder and the raw query field stay interchangeable: a query typed by hand can be opened in the builder, and a built query can be edited by hand.

## Non-goals

- OR, AND or grouping. The query language has none (every term is ANDed), and the builder doesn't invent any.
- A TUI builder. The model is reusable, but only the GUI dialog is specified here.
- Changing the query syntax, the parser's behaviour, or how queries are executed.
- Creating or editing saved searches. The existing Save dialog stays as is.
- Inserting saved searches (`?name`) from a picker. A `?name` term in an opened query is kept as a raw row (see Decisions).

## Decisions

| Topic | Decision |
|---|---|
| Layout | A modal dialog with a list of condition rows, an "Add condition" button, a sort control, a live match count, and a read-only preview of the generated query string. |
| Row shape | `[Include/Exclude] [Type] [value input] [remove]`. Include/Exclude maps to the `-` negation prefix. |
| Types | Text, Note name, Heading, Folder, Tag, Links to, Linked from, Property, Raw. |
| Entry points | A `Build…` button beside `Syntax` in the Find view toolbar and in the Search dialog. Both open the same dialog. |
| Round-trip | Opening the builder parses the current query text into rows. Apply replaces the query text and runs it. Cancel changes nothing. |
| Unrepresentable terms | Tokens the parser would discard (unterminated quote, a prefix with no value, a malformed property filter) are kept by `SearchTerms` and shown as `Raw` rows, so nothing is lost. The user can edit or remove them. A `?name` term is a plain full-text term to core, so it shows as a Text row and round-trips unchanged. |
| Unterminated quote | The token is kept as a `Raw` row. The builder shows a hint on that row. |
| Value inputs | Smart pickers per type (see below). Property keys are suggested from the vault's indexed keys. |
| Sort | One optional control: key (Title, Filename, a property name) and direction. Maps to `^key` / `-^key`. It reuses the existing sort logic. |
| Open-note variable | No toggle. The note autocomplete offers the `{note}` variable as its first option, labelled "current open note". Picking it inserts `{note}`. This applies everywhere the note autocomplete runs (the builder's note inputs and the query field's `<` `>` `=` operators). |
| Live count | Debounced, about 250 ms after the last edit, using the existing query page source. Shows `N notes` or `500+`. Shows nothing for an empty or invalid query. |
| Scope | GUI only. |

## Core

`SearchTerms` is the single entity that holds a query's content. Query-string construction belongs next to the parser in `core/src/index/search_terms.rs`, so quoting, prefix tables and negation can't drift apart. The GUI never concatenates operator characters itself.

Extend `SearchTerms` (no new model type):

- `SearchTerms::to_query_string(&self) -> String`: the inverse of `from_query_string`. It emits the short operator forms, `-` for every `excluded_*` bucket, quotes through `quote_query_term` (and the property-key and property-value quoting rules), and appends order directives. Terms come out grouped by bucket, which is fine because the terms are ANDed.
- `SearchTerms.unparsed: Vec<String>`: a new bucket for tokens the parser discards today (unterminated quote, bare prefix with no value, malformed property filter), kept verbatim. The index ignores it, so search results don't change. `to_query_string` emits them as written.
- Structured edit helpers only if the builder needs them. A row-level view for the GUI is built over the existing buckets, not stored in core.

Required properties, all covered by tests:

- **Round-trip**: for every query the parser accepts, `from_query_string(q).to_query_string()` parses to the same `SearchTerms` as `q`. It need not be byte-identical.
- **Quoting**: values with spaces or quote characters serialize so the parser reads them back unchanged, including quoted property keys and `?name`-style terms.
- **Negation, wildcards, sort**: `-` for every negatable bucket, `*` preserved, and all order directives kept in their original order.
- **Unparsed tokens** survive a round-trip and don't affect the SQL the index builds.
- **Query size**: the existing 8 KB truncation still applies.
- Path and note-name handling reuses the existing core functions. No hardcoded `.md` or `/`.

Property keys for suggestions come from the existing `NoteVault::property_keys()`. Tags and notes come from `list_labels`/`suggest_tags_by_prefix` and `suggest_notes_by_prefix`. Folders come from `NoteVault::get_directories`. Nothing in the GUI touches the filesystem.

## Shared logic (`kimun_notes::logic`)

- The picker data sources reuse `SuggestionSource` / `VaultSuggestions` for tags and notes. Add `properties_by_prefix` (backed by `NoteVault::property_keys()`) to the trait, with an empty default like `saved_searches_by_prefix`.
- `SuggestionItem` gains a display label separate from the inserted text, so the `{note}` item can read "current open note" while inserting `{note}`. `link_filter_suggestions` in the autocomplete engine puts it first, and the label is user-facing text, so it goes through the GUI's string handling.
- Folders: a thin source over the existing `file_ops::list_directories` (already used by `DirectoryChoice`).
- Live count: a small helper over `QueryPageSource` that returns a capped count and takes the same query context the Find view uses, so `{note}` resolves identically.

## GUI

### Dialog

- A new modal kind, `ModalKind::QueryBuilder`, built with the kit `Dialog` pattern via `Workspace::show_modal`.
- When opened from the Search dialog it stacks on it (`Slot::Stacked`), like the syntax help. From Find it takes its own slot.
- Layout, top to bottom:
  1. Condition rows. Each row has an Include/Exclude toggle, a type dropdown, the value input, and a remove button. New rows focus their value input.
  2. `Add condition`.
  3. Sort: key dropdown and direction toggle. It is hidden or disabled when no key is chosen.
  4. Preview: the generated query string, read-only, with the live count beside it.
  5. Footer: `Cancel` and `Apply`.
- Keyboard: Enter in a value input adds a new row only when the row has a value. `Ctrl+Enter` applies. Escape closes the dialog, or closes an open picker first.

### Value inputs by type

| Type | Input |
|---|---|
| Text, Note name, Heading | Plain text. Note name also offers note autocomplete. |
| Tag | Autocomplete from vault tags. |
| Folder | The existing fuzzy directory chooser. |
| Links to / Linked from | Note autocomplete, with "current open note" as the first option. |
| Property | Three fields: key (autocomplete from indexed keys), operator dropdown, value. The operator can be "exists" (no value), which gives the bare `%key`. |
| Raw | Plain text, shown in a monospace style. |

### Entry points

- `Build…` button next to `Syntax` in the Find toolbar (`find.rs`), emitting an event handled like `FindEvent::Help`.
- `Build…` in the Search dialog's header, next to the syntax help.
- A chord `Build query`, default `primary+shift+k`, scoped to where a query is typed (the same scope as `QuerySyntax`). `Ctrl+B` is Bold in the editor and `primary+shift+b` is the leader, so neither is used. The plan includes the keymap conflict check.
- Apply writes the generated string into the query field. In Find it runs the query immediately (as Enter does). In the Search dialog it triggers the normal debounced search.
- If the Build dialog is closed with Cancel, the query field keeps its text and focus returns to it.

### Disabled states

- No open vault: the button is disabled. Live count is hidden when the vault is read-only or unavailable.

## Edge cases

- Empty builder (no terms and no sort): Apply is allowed and clears the query. The preview shows a placeholder.
- A row with an empty value is ignored when generating the query, and Apply never produces a dangling operator. The row is marked as incomplete.
- Values containing spaces, quotes, `*`, or leading operator characters are quoted or escaped by core so the parser reads them back unchanged.
- A query longer than 8 KB: the builder warns that the query is truncated.
- A Property row whose key contains spaces serializes with a quoted key.
- Sort by a property: the property name is free text with property-key suggestions if available, otherwise plain text.
- A query with an order directive in the middle of the string: the directive is read into the sort control wherever it appears.
- Two Include rows of the same type are valid, since terms are ANDed. Contradictory rows (the same tag included and excluded) are allowed, and the count simply shows 0.

## Docs and release notes

- `docs/content/using-kimun/search.md`: add a short "Query builder" section: how to open it, what the row types are, and that it edits the same query text.
- Update the syntax help card (`SYNTAX` in `tui/src/logic/query/syntax.rs`) only if the missing items (bare `<`/`>`, `?name`) are fixed separately. That is not part of this work.
- Release notes: mention the new builder.

## Testing

- Core: unit tests for round-trip over a corpus covering every operator, negation, quoting, wildcards, property operators, sort directives, unterminated quotes, and `?name`/unknown tokens (which must come out in `unparsed` or as plain terms, as described above).
  Include a property-style test: for generated terms, `to_query_string` then `SearchTerms::from_query_string` yields the intended terms.
- Shared logic: tests for the capped count helper and the folder source.
- GUI: `*_tests.rs` in `gui/src/workspace` for the dialog (adding, removing, editing rows, apply and cancel behaviour, raw rows, the `{note}` option, the disabled and incomplete states) and for both entry points.

## Resolved during spec review

1. `SearchTerms` is the model, extended with `to_query_string` and an `unparsed` bucket.
2. The open note is the first autocomplete option, labelled "current open note", inserting `{note}`.
3. `Build…` gets a chord, default `primary+shift+k`, scoped to where a query is typed.
4. Property keys are suggested from the index (`property_keys()`).

## Open items to settle during planning

1. The exact text of the "current open note" label and where its string lives.
2. The `primary+shift+k` conflict check against the keymap and the GUI kit.
3. How the dialog is sized and scrolled when there are many rows (the help dialog uses `HELP_REMS = 30`).
4. The `find.rs` module doc says the field has "no autocomplete, no colouring", while the code wires `InputAutocomplete`. Confirm the current behaviour before the builder reuses it.
5. Whether `unparsed` should also hold tokens the order parser rejects (an unknown `^key`).
