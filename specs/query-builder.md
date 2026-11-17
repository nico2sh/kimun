# Spec: Query builder

Status: draft. Applies to the GUI. The model lives in core so the TUI can reuse it later.

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
| Unrepresentable terms | They become `Raw` rows holding the original text, so nothing is lost. The user can edit or remove them. Examples: `?saved` expansions and tokens the lexer can't classify. |
| Unterminated quote | The unterminated token is kept as a `Raw` row, because the parser would discard it. The builder shows a hint on that row. |
| Value inputs | Smart pickers per type (see below). |
| Sort | One optional control: key (Title, Filename, a property name) and direction. Maps to `^key` / `-^key`. It reuses the existing sort logic. |
| Open-note variable | Links to / Linked from rows have an "Open note" toggle as an alternative to naming a note. It maps to the bare `<` / `>` (the `{note}` variable). |
| Live count | Debounced, about 250 ms after the last edit, using the existing query page source. Shows `N notes` or `500+`. Shows nothing for an empty or invalid query. |
| Scope | GUI only. |

## Core

All query-string construction and parsing belongs next to the parser in `core/src/index/search_terms.rs`, so quoting, prefix tables and negation can't drift apart between the builder and the parser. The GUI never concatenates operator characters itself.

Add a query model in core (module and names to settle in planning):

- `QueryTerm`: an enum with one variant per row type (`Text`, `Name`, `Heading`, `Path`, `Tag`, `LinksTo`, `LinkedFrom`, `Property`, `Raw`), each with a `negated` flag. `LinksTo` and `LinkedFrom` take either a note name or "open note". `Property` has a key, an operator (`= != < <= > >=`) and a value, plus the "has property" form with no operator or value.
- `QuerySort`: an optional key and direction.
- `QueryModel`: `{ terms: Vec<QueryTerm>, sort: Option<QuerySort> }`.
- `QueryModel::to_query_string()`: serializes through the existing helpers (`quote_query_term`, the prefix table, `with_order_directive`). It uses the short operator forms, the same as the syntax reference.
- `QueryModel::from_query_string(&str)`: parses via the existing tokenizer (`QueryTermExtractor`, `strip_order_directive`, `query_token_spans`). It never fails: anything it can't map to a typed term becomes `Raw`.

Required properties, all covered by tests:

- **Round-trip**: for every query the parser accepts, `from_query_string(q).to_query_string()` parses to the same `SearchTerms` as `q`. It need not be byte-identical. Raw terms are emitted verbatim.
- **Quoting**: values with spaces or quote characters serialize so the parser reads them back unchanged. This includes quoted property keys.
- **Negation**: `-` is emitted for every term type the parser lets it negate.
- **Wildcards**: `*` in a value is preserved.
- **Query size**: the existing 8 KB truncation still applies.
- Path and note-name handling reuses the existing core functions. No hardcoded `.md` or `/`.

If a note or folder list is needed for the value pickers, it comes from the existing core APIs (`NoteVault::get_directories`, `list_labels`/`suggest_tags_by_prefix`, `suggest_notes_by_prefix`). Nothing in the GUI touches the filesystem.

## Shared logic (`kimun_notes::logic`)

- The picker data sources reuse `SuggestionSource` / `VaultSuggestions` for tags and notes.
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
| Links to / Linked from | Note autocomplete, plus the "Open note" toggle. |
| Property | Three fields: key, operator dropdown, value. The operator can be "exists" (no value), which gives the bare `%key`. |
| Raw | Plain text, shown in a monospace style. |

### Entry points

- `Build…` button next to `Syntax` in the Find toolbar (`find.rs`), emitting an event handled like `FindEvent::Help`.
- `Build…` in the Search dialog's header, next to the syntax help.
- A chord is optional and is settled in planning, together with any conflict check against the keymap.
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

- Core: unit tests for round-trip over a corpus covering every operator, negation, quoting, wildcards, property operators, sort directives, unterminated quotes, and `?name`/unknown tokens (which must come out as `Raw`).
  Include a property-style test: for generated terms, `to_query_string` then `SearchTerms::from_query_string` yields the intended terms.
- Shared logic: tests for the capped count helper and the folder source.
- GUI: `*_tests.rs` in `gui/src/workspace` for the dialog (adding, removing, editing rows, apply and cancel behaviour, raw rows, the Open note toggle, the disabled and incomplete states) and for both entry points.

## Open items to settle during planning

1. Module and type names in core. Check whether `SearchTerms` can be reused as the model instead of adding a new one.
2. Whether a bare `<` / `>` should serialize as `<{note}` or as the bare form. Check what `expand_bare_note_prefixes` and the GUI query context expect.
3. Whether `Build…` gets a default chord, and which one.
4. Whether property keys can be listed from the index for suggestions. If core has no such API, the property key input stays plain text in v1.
5. How the dialog is sized and scrolled when there are many rows (the help dialog uses `HELP_REMS = 30`).
6. The `find.rs` module doc says the field has "no autocomplete, no colouring", while the code wires `InputAutocomplete`. Confirm the current behaviour before the builder reuses it.
