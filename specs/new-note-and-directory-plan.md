# Plan: Intentional "New note" and "New directory"

Implements `specs/new-note-and-directory.md`. Read that first; this file covers order, files and checks.

Items marked **verify** are assumptions I could not confirm from a read of the code. Check them before building on them.

## Findings that shape the plan

- `nfs::create_note_exclusive` and `nfs::create_directory` already create missing parent directories (`create_dir_all`), and return `AlreadyExists`. Core needs no new filesystem code for creation.
- `NoteVault::create_note` is exclusive (`NoteExists`). The shared `file_ops::create_note(.., None)` is not: it uses `load_or_create_note`, so a new function is needed.
- The TUI crate (`kimun_notes`) holds the logic the GUI reuses (`kimun_notes::logic::file_ops`, `::message`, `ActionShortcuts`). Shared work goes in `tui/src/logic`.
- `file_ops.rs` already has `availability(vault, candidate)` and `ValidationState`, used by the rename and move dialogs for live validation. Reuse them for the live path preview.
- Leader `n d` is "daily", and `n t` is "from template" (currently `Soon("templates — coming soon")`). New directory takes `n f`.
- The config migration chain (`config_migration.rs`, `CURRENT_CONFIG_VERSION = 7`) has a `still_at_default` helper and a v3 to v4 migration that moved FollowLink to `Ctrl+N`. The same pattern handles the move to `Ctrl+U` as a v7 to v8 step.
- The F2 menu is per-selected-entry, so it is not touched.

## Slices

Three independently mergeable slices, in this order. Each ends with the checks listed under "Checks".

### Slice 1: core and shared logic (no UI)

**Core** (`core/src/lib.rs`, `core/src/nfs/vault_path.rs`, `core/src/error.rs`)

1. Read `VaultPath::new`, `note_path_from`, `is_valid`, `with_note_extension` and `get_clean_name`, and the invalid-character rules. **Verify** what they already do for lowercasing, extension handling and rejecting characters invalid on Windows/macOS/Linux, so the new function reuses them and adds nothing duplicate.
2. Add `NoteVault::resolve_new_note_path(base: &VaultPath, input: &str) -> Result<VaultPath, VaultError>`:
   - empty or whitespace-only input is an error;
   - input ending in `/` is an error for notes (message points to New directory);
   - joins `base` and `input` (input is relative to `base`, no hardcoded `/` or `.md`), applies the existing note-path cleanup, validates characters.
3. Add `NoteVault::resolve_new_directory_path(base, input)` with the same rules, and a directory result.
4. Add an `InvalidName`-style `VaultError` variant if none fits (check `error.rs` first). Messages must read well inline in a dialog.
5. Tests in core: normalization and lowercasing, nested input creating parents (via `create_note`), exists errors for both kinds, empty or whitespace input, trailing `/`, invalid characters, an intermediate component that is an existing non-note file (**verify** the error from `create_dir_all` is mapped to something readable and not a raw IO error).

**Shared logic** (`tui/src/logic/file_ops.rs`, tests in `file_ops/tests.rs`)

6. `new_note_target(vault, base, input) -> Result<VaultPath, String>` and `new_directory_target(...)`: thin wrappers over the core resolvers returning `String` errors like the rest of the file.
7. `create_new_note(vault, path, content: Option<String>) -> Result<NoteOutcome, String>`: exclusive via `vault.create_note(path, content.unwrap_or_default())`, `NoteOutcome { created: true }`, and `NoteExists` becomes an "already exists" message. `content` is the templates seam.
8. `create_directory(vault, path) -> Result<FileOp, String>`.
9. **Verify** how `FileOp::Created(path)` is handled in the TUI sidebar (`tui/src/components/sidebar.rs`, `dir_browser.rs`) and the GUI (`gui/src/workspace/files.rs:482`) for a directory path. If it reveals and selects a directory correctly, reuse it. If not, add `FileOp::DirectoryCreated(VaultPath)` and handle it in both.
10. A helper `new_entry_base(browser_dir: Option<VaultPath>, open_note: Option<&VaultPath>) -> VaultPath` for the default target directory (browser dir, else the open note's parent, else root). Unit test the three cases.
11. Tests: parents created, exists error leaves the existing note untouched, outcome announced.

### Slice 2: TUI

**Actions and keys**

1. `tui/src/logic/actions.rs` and `tui/src/keys/action_shortcuts.rs`: add `NewNote` and `NewDirectory`. Update every table that lists an action: the enum, the label (`"New note"`, `"New directory"`), the category (Notes, next to `NewJournal`), the string and parse tables, and the ordering table.
2. `tui/src/settings/mod.rs` default bindings: `NewNote` on `Ctrl+N`, `FollowLink` on `Ctrl+U`. Update the comment on the FollowLink line, and the commented default list near line 127 if it lists these. `NewDirectory` on `Alt+N`: **verify** that `Alt+letter` is accepted by the binding layer and arrives on terminals (see `keys/reachability.rs` and `key_combo.rs`). If the binding layer rejects it, stop and ask before choosing another key.
3. `config_migration.rs`: add `migrate_to_v8`: if `FollowLink` `still_at_default(.., Ctrl+N)`, rewrite it to `Ctrl+U`. Bump `CURRENT_CONFIG_VERSION` to 8. A user who customized FollowLink keeps it. **Verify** `merge_missing_default_bindings` does not bind `NewNote` to `Ctrl+N` when another action already holds that chord (and fix it if it does), so a customized FollowLink on `Ctrl+N` keeps working and `NewNote` is simply unbound there. Tests: untouched default migrates, customized stays, idempotent, and a duplicate-chord check across all defaults.
4. Update every comment, hint and test that says FollowLink is `Ctrl+N` (`editor.rs:1841`, `editor_input.rs`, `attachment_view.rs`, `footer_bar.rs`, `text_editor/mod.rs`) and any footer or help hint text. Grep for `Ctrl+N` and `ctrl&N` after changing, and review each hit; the serde fixtures in `keys/mod.rs` that use `NewJournal = ["ctrl&N"]` are only sample data and can stay.
5. Hardcoded Ctrl+Enter follow stays as is.

**Dialogs** (`tui/src/components/dialogs/`)

6. `new_note_dialog.rs`, modeled on `create_note_dialog.rs` and `quick_note_modal.rs` (text input, hint chips via `confirm_hints`, mouse handling via the shared modal pattern):
   - fields: base dir, input text, resolved path display, error, optional `initial` text and `content`;
   - on each edit, re-resolve with `new_note_target` and check `availability`, and show the resolved path, or the error inline;
   - Enter creates via `create_new_note`, then `announce_and_open`; errors show inline and the dialog stays open;
   - Escape cancels.
7. `new_directory_dialog.rs`: the same without the open step; Enter runs `create_directory` and announces.
8. Register both in `dialogs/mod.rs` and the `ActiveDialog` plumbing. Add `FileOp::ShowNewNote { base, initial }` and `FileOp::ShowNewDirectory { base }` (`logic/message.rs`) so any component can request them.
9. Render tests with the existing dialog test style (see `create_note_dialog` tests).

**Wiring**

10. `app_screen/editor_input.rs` and `app_screen/browse.rs` (which already handle `NewJournal` globally): map `ActionShortcuts::NewNote` and `NewDirectory` to the dialog requests, computing the base with `new_entry_base`.
11. `logic/leader_route.rs`: `A::NoteNew` now opens the dialog (replacing the "type a name — Enter creates" flash in `editor.rs:1937`). Add `NoteNewDirectory` to the leader actions and tree in `keys/leader.rs` as `n f` ("new directory"), and route it. Update `leader_route/tests.rs` and the tests that enumerate the leader tree. Leave `n t` ("from template", currently "coming soon") alone.
12. Command palette (`logic/palette.rs`) and keyboard help (`logic/keyboard_help.rs`): list both actions with their effective bindings.
13. Find the search-field "Create: …" suggestion (start with `components/file_list.rs`, `components/note_browser/mod.rs`, `components/sidebar.rs`). Replace its direct create with `FileOp::ShowNewNote { base, initial: typed text }`.
14. Optional, last: a clickable `[+]` chip in the browser header that opens a two-item menu. Skip it if there is no room in the header.

### Slice 3: GUI

1. `gui/src/keymap.rs`: `A::NewNote => &["primary+n"]`, `A::NewDirectory => &["primary+shift+n"]`. **Verify** no clash with the OS or the kit (a new window or tab binding). Read `gui/src/keymap/tests.rs` around line 494 and 1175 (`ctrl+n`); they reference the TUI's `Ctrl+N` and likely need updating.
2. `gui/src/workspace/modal.rs`: add `ModalKind::NewNote` and `ModalKind::NewDirectory` (with their `ActionShortcuts`), and handle them wherever `QuickNote` is matched (`workspace.rs` dispatch, the busy check at `modal.rs:229`).
3. New file `gui/src/workspace/new_entry.rs` (or extend `create.rs`; choose by size), modeled on `CreateNoteBody` and `QuickNoteDialog`: kit `Dialog`, `Input`, `OpStatus`, `error_row`, `footer`, `path_line` from `file_dialogs`. Live resolved path and error, Enter creates and opens, Escape cancels, typing clears an error, buttons wait while busy (as in the current dialogs). The directory dialog is the same without the open step. Accept an `initial` string and a base directory.
4. `workspace.rs`: dispatch the two actions through `toggle_modal`, computing the base via `new_entry_base` from the FILES model's current directory and the active note.
5. `gui/src/workspace/palette.rs` and `keyboard_help.rs`: list both actions (see the `QuickNote` entries).
6. `gui/src/workspace/files.rs`:
   - `+` icon button in the existing header row beside the sort chip (`FILES_SORT_ID` area), with an id for tests and a tooltip; it emits a new `FilesEvent`.
   - The menu: two items, each with the effective shortcut from the keymap. Reuse the kit's menu/popup pattern; **verify** one exists (look at how the entry context menu is shown) and use it. It is disabled when there is no vault or the vault is read-only.
   - Reroute the search-field create suggestion to open the New note dialog with the typed text.
7. Tests (`*_tests.rs` beside the module): both dialogs (resolution, exists error stays open, success opens), the `+` menu (items, shortcuts shown, disabled state), the search suggestion routing, and `FileOp::Created` revealing a directory.

### Docs and release notes (with slice 2, updated in slice 3)

- `docs/content`: add "Creating notes and directories" (shortcuts, leader keys, `+` menu, search-field flow); update the keyboard reference, including FollowLink on `Ctrl+U`.
- Release notes: the TUI FollowLink move from `Ctrl+N` to `Ctrl+U`, and that existing configs are migrated when FollowLink was never customized. Follow the `release-notes/` and `release-plz.toml` conventions already in the repo (**verify** the format before writing).

## Checks (each slice)

- `cargo fmt --check`, `cargo clippy --workspace --all-targets`, `cargo test -p kimun_core`, `cargo test -p kimun_notes`; for slice 3 also the gui crate tests (**verify** the GUI crate builds in this environment; it may need system libraries).
- `python3 .github/scripts/check-host-fs.py` (the TUI must not call `rename`, `create_dir*` and the like directly; creation goes only through core).
- Grep for `Ctrl+N` and `ctrl&N` after slice 2.
- Manual: run the TUI against a scratch vault: create a note in the root, in a nested new path, a duplicate (inline error), a directory, via the search suggestion, and via the leader keys. Check that follow-link still works on `Ctrl+U` and that a config with the old default is migrated.

## Risks

- `Alt+N` may not be deliverable or bindable on some terminals; the leader `n f` and the palette are the fallback, so this does not block the feature.
- The FollowLink change is user-visible. The migration covers only untouched defaults, so a user with a customized FollowLink is not changed.
- Case-insensitive collisions and lowercasing depend on existing `VaultPath` behavior (verify in slice 1 before the UI relies on it).
