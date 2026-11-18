# Plan: No "Create:" row in the desktop search browser

Implements `specs/search-browser-no-create-row.md`. One small slice, one commit.

## Findings

- `resolving_search_source_with_create` has one caller: `gui/src/workspace/search.rs:1042`. The TUI uses `resolving_search_source`.
- `SearchNotesProvider.create_dir` is `Some` only through that constructor. `leading_row` returns `None` when `create_dir` is `None`, so dropping the field removes the row with no other behavior change.
- `create_row` is also used by `FileFinderProvider` (`pickers/finder.rs`) and the file browser. Do not touch it.
- `open_note_dir()` (`gui/src/workspace/files.rs`) has other callers, so it stays.

## Steps

1. **Logic** (`tui/src/logic/pickers/search.rs`)
   1. Remove `create_dir` from `SearchNotesProvider` and from `new`.
   2. Remove the `leading_row` override on its `RowSource` impl (the trait default returns `None`; **verify** the default).
   3. Delete `resolving_search_source_with_create` and drop it from the re-export in `tui/src/logic/pickers/mod.rs:21`.
   4. Remove the `create_row` import if nothing else in the file uses it.
   5. Fix the doc comments that mention the desktop browser's `Create:` row.
2. **GUI** (`gui/src/workspace/search.rs`)
   1. Call `resolving_search_source(vault.clone(), recents, open.clone())`.
   2. Update the module docs and the `search_open` comment as listed in the spec.
3. **Tests**
   1. `tui/src/logic/pickers/search.rs`: replace the create-variant test with a "no leading row for a typed query" test.
   2. `gui/src/workspace/search_tests.rs`: rewrite the three `CreateNote` cases (rows, nothing-matches, preview Up) as described in the spec. For nothing-matches, assert no rows and no selection. For the preview test, Up from the first match keeps it selected.
   3. Check `search_tests.rs` helpers (the `Create: {path}` row formatter near line 110) and remove what becomes dead.
4. **Docs**: `grep -rn "Create:" docs/content` and confirm no page describes it in the search browser (the two hits found describe the filter and finder, which stay).

## Checks

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings` (watch for unused imports and dead code left behind)
- `cargo test -p kimun_notes` (logic, finder, file browser)
- `cargo test -p` the GUI crate's package, for `search_tests`, `picker_tests` and `files_tests`. **Verify** the package name in `gui/Cargo.toml`; GUI tests may need system libraries this container lacks, in which case say so rather than skip silently.
- Run `.github/scripts/check-host-fs.py` only if a filesystem call was touched (none expected).

## Manual check

In the desktop app: Ctrl+K and type `#tag` or `zebra`, and confirm no `Create:` row. Then confirm that the file browser filter and `Ctrl+O` still show it.

## Risks

- Low. The change deletes an optional row and its plumbing. The main risk is a GUI test that relied on `Create:` being selectable to drive focus; step 3.2 covers those.
