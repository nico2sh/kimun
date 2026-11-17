# Plan: Query builder

Implements `specs/query-builder.md`. Items marked **verify** were read from a code survey and must be confirmed against the code before the step starts.

## Findings that shape the work

- `SearchTerms` (`core/src/index/search_terms.rs`) is bucketed: one `Vec` per prefix plus an `excluded_*` twin. Token order across buckets is not kept, which is fine because everything is ANDed. Order directives are kept in order (`order_by`).
- Several inputs are dropped silently today: an unterminated quote, a bare prefix with no value, a malformed property filter. The builder can't round-trip those without a new bucket.
- There is no serializer back to a string. The helpers that exist are `quote_query_term`, `with_order_directive` and `strip_order_directive`. Property keys and values have their own quoting (`PropertyFilter::parse`). **verify**: the exact rules for a quoted key (`%"due date"<…`), a quoted value, and the `wildcard` flag on `PropertyTest::Compare` (an unquoted value allows `*`, a quoted one is literal).
- `?name` is a presentation-layer sigil. Core reads it as a plain full-text term, so it round-trips as a text term. **verify** that `quote_query_term` does not touch it and that the parser doesn't treat `?` specially.
- `link_filter_suggestions` (`tui/src/logic/autocomplete/engine.rs`) already puts the `{note}` variable first in the `<` / `>` / `=` popups. It shows as the literal `{note}`, and `SuggestionItem` has only `display` and `secondary`, so a friendly label needs a new field.
- `NoteVault::property_keys()` already returns the distinct indexed keys, sorted. `SuggestionSource` has no property method, and the `%` operator has no autocomplete trigger today.
- `QuerySyntax` is a `Scope::Query` action (F1), and `Scope::Query` shadows `Scope::Everywhere` where a query is typed. A `Build query` action can use the same scope. Chords in use that start with `primary+`: `, c d e enter f j k n o q r s shift+b shift+enter shift+f shift+n shift+p shift+w t w`. `primary+b` is free in the table. **verify** that it isn't used by the editor or vim layer in the query scope, and that the kit doesn't claim it.
- Syntax help (`open_query_help`, `gui/src/workspace/search.rs`) is the template for a simple dialog. A dialog with inputs follows `new_entry.rs` / `saved_searches.rs`. **verify** the `ModalKind` and `Slot` details in `modal.rs`.

## Order of work

Each step builds and passes its tests before the next starts.

### 1. Core: `SearchTerms::to_query_string` and `unparsed`

Files: `core/src/index/search_terms.rs` (tests alongside).

1. Add `pub unparsed: Vec<String>` to `SearchTerms` and fill it from the tokenizer for every token it discards today. Keep the index ignoring it (**verify** that no code outside `search_terms.rs` destructures `SearchTerms` exhaustively; if some does, update it).
2. Add `to_query_string(&self) -> String`:
   - bare terms, then each bucket with its short prefix, then excluded buckets with `-`, then properties, then order directives, then `unparsed` verbatim;
   - quoting via `quote_query_term`, and the property rules found above.
3. Tests:
   - a corpus covering every operator, negation, quoting with spaces, wildcards, property operators, the exists form, quoted property keys, sort directives (title, file, `%key`, descending), `?name`, and duplicate labels;
   - for each: `parse(to_query_string(parse(q)))` equals `parse(q)` (compare through a `PartialEq`-style helper or the debug form, since `SearchTerms` derives only `Debug`/`Default`);
   - unterminated quote, bare prefix and malformed property land in `unparsed`, and the SQL the index builds is unchanged by their presence;
   - the 8 KB truncation still applies on parse.

Check: `cargo test -p kimun_core`, `cargo clippy -p kimun_core`.

### 2. Shared logic: suggestions and the "current open note" label

Files: `tui/src/logic/autocomplete/source.rs`, `engine.rs`, `state.rs`.

1. `SuggestionItem` / `Suggestion`: add an optional `label` (shown) separate from `display` (inserted). **verify** how accept uses `display`, so the inserted text stays `{note}`.
2. `link_filter_suggestions` gives the `{note}` item the label "current open note". **verify** how user-facing strings are handled in this crate (a constant, or an i18n table) and follow it.
3. `SuggestionSource::properties_by_prefix(prefix, limit)` with an empty default, implemented in `VaultSuggestions` over `NoteVault::property_keys()` (filtered by prefix in search form, so case and accents are ignored, as in the property matching). Put the prefix filtering in core if there's no suitable function (CLAUDE.md: path and content logic in core).
4. Optionally add a `%` autocomplete trigger to the query field using the same source. **Decision at this step**: only if the trigger code stays small. Otherwise leave it and note it as a follow-up. The builder doesn't depend on it.
5. Tests: the engine test module already covers link-filter results. Add cases for the label, for `{note}` being inserted, and for the property source.

Check: `cargo test -p kimun_notes` (the crate name may differ, **verify**), plus the TUI autocomplete tests, since the TUI shares this code. The TUI popup must keep showing the `{note}` item correctly with the new label.

### 3. Shared logic: live count

Files: `tui/src/logic/query/` (next to `QueryPageSource`).

1. A helper that takes a query string and the query context, resolves `{note}` and `?name` the way the Find view does (reuse that code path, don't duplicate it), runs the query through `QueryPageSource`, and returns a count capped at 500 plus a flag for "capped".
2. Returns `None` for an empty query and an error state for a failure, so the dialog shows nothing in those cases.
3. Tests for: empty, capped, no matches, an unresolved `{note}` with no open note.

### 4. GUI: the dialog

Files: new `gui/src/workspace/query_builder.rs` (plus `query_builder_tests.rs` per the local test convention), `modal.rs`, `keymap.rs`.

1. `ModalKind::QueryBuilder`, with the `Slot` choice described in the spec (`Stacked` over the search browser, `Own` from Find). Register the chord in `ModalKind::chord()` only if that is how the other dialogs do it. **verify**.
2. A row model owned by the dialog, built from `SearchTerms` (one row per term, in a fixed bucket order: text, name, heading, folder, tag, links, forward links, property, raw, then sort). Rows convert back into a `SearchTerms` and then `to_query_string()`. The dialog contains no string concatenation of operator characters.
3. Row UI: Include/Exclude toggle, type dropdown, value input, remove. The value input depends on the type:
   - Tag: `InputAutocomplete` with the tag source.
   - Note name, Links to, Linked from: the note source with "current open note" first.
   - Folder: the existing `DirectoryChoice`.
   - Property: key (property source), operator dropdown (including "exists"), value.
   - Text, Heading, Raw: plain inputs.
4. Sort control: key dropdown (Title, Filename, a property) and direction, reusing the sort dialog's logic and `OrderField` (`sort.rs`). **verify** what can be shared without opening the sort dialog.
5. Footer: Cancel and Apply (`Ctrl+Enter` applies, Escape closes a picker first, then the dialog).
6. Preview line with the live count, debounced about 250 ms, using step 3. Incomplete rows (empty value) are skipped and marked.
7. Keymap: add the `Build query` action, default `primary+b`, scoped like `QuerySyntax`. Add it to the palette and the keyboard help. Run the keymap conflict tests (**verify** that a "no two defaults share a chord" test exists; the new-note work added one for the TUI, and the GUI has `gui/src/keymap/tests.rs`).
8. Tests: opening from a query pre-fills the rows, adding and removing rows, Apply and Cancel, raw rows survive, an empty builder clears the query, the `{note}` option, an incomplete row is skipped, the disabled state without a vault.

### 5. GUI: entry points

Files: `gui/src/workspace/find.rs`, `search.rs`.

1. `Build…` button beside `Syntax` in the Find toolbar, emitting a `FindEvent::Build` handled next to `FindEvent::Help`.
2. `Build…` in the Search dialog header, beside the syntax help.
3. Apply writes the query into the field. In Find it runs at once, in the Search dialog it goes through the normal debounce. Cancel returns focus to the field.
4. Tests: both buttons open the same dialog, Apply updates the field, Cancel doesn't.
5. **verify** the `find.rs` module doc claim ("no autocomplete, no colouring") against the code. Fix the doc comment if it is stale.

### 6. Docs

- `docs/content/using-kimun/search.md`: a short "Query builder" section, including the shortcut and the "current open note" option. End-user language only.
- Keyboard reference: add the new chord if one exists in the docs.
- Release notes: mention the builder, the new label, and the `Build query` shortcut.

## Files expected to change

| Area | Files |
|---|---|
| Core | `core/src/index/search_terms.rs` |
| Shared logic | `tui/src/logic/autocomplete/{source,engine,state}.rs`, `tui/src/logic/query/*` |
| GUI | `gui/src/workspace/{query_builder.rs, modal.rs, find.rs, search.rs}`, `gui/src/keymap.rs` (and its tests), the palette and keyboard-help data |
| Docs | `docs/content/using-kimun/search.md` |

## Checks before finishing

- `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo fmt --check`.
- `.github/scripts/check-host-fs.py` (the host-fs rule in `CLAUDE.md`): the new code must not call `rename`, `copy`, `remove_*`, `create_dir*` and so on.
- Launch the GUI once (the `run` skill) and walk through: open Find, type a query with a tag, a folder and a property, click `Build…`, check the rows, change one, Apply, and check the field and results. Repeat from the Search dialog. Try a query with `?saved` and an unterminated quote.
- The TUI's query popup still works after step 2.

## Risks

- **Parser change (step 1)** is the riskiest part, since search results depend on it. Mitigation: `unparsed` is write-only for the index, and the round-trip corpus is written before the serializer.
- **Autocomplete label (step 2)** touches code the TUI shares. Mitigation: the label is optional, and TUI tests run in that step.
- **Many rows**: dialog height and scrolling are decided in step 4 against the help dialog's sizing.
