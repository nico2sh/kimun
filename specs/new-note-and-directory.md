# Spec: Intentional "New note" and "New directory"

Status: draft. Applies to the TUI and the GUI.

## Goals

1. An intentional way to create a note, instead of only the search-field suggestion in the file browser.
   It is a single creation path, so templates can later hook in.
2. A way to create a directory. Today the only route is creating a note inside a new subdirectory.

## Non-goals

- Template UI, template storage or template config. Only a seam is provided (see Core).
- Changing the quick note or the journal flows.

## Decisions

| Topic | Decision |
|---|---|
| New-note UI | A dedicated modal dialog with a name/path input. It shows the resolved target path live. |
| Default target dir | The file browser's current directory. If the browser has no current directory, the open note's directory. If neither, the vault root. |
| Directory choice | Both dialogs show the target directory (the browsed one by default) and let the user pick another: Tab moves to a filter over every vault directory (a shared `DirectoryPicker`), Enter picks, Esc goes back. The typed name is relative to the chosen directory. |
| GUI folder choice | `Change…` beside the folder line opens the shared folder choice (a filter and a list) inside the dialog, rather than Tab: Tab keeps moving between the dialog's fields. Esc closes the list first. |
| Name input | A vault-relative path. Missing directories are created automatically. Extension and casing are normalized by core, never hardcoded in a frontend. |
| Existing note | An inline error ("already exists"). The dialog stays open and nothing is overwritten or opened. |
| After note create | Open the note in the editor. The file browser reveals and selects it. |
| New directory UI | A separate dialog that takes a directory name or relative path. |
| After directory create | The file browser reveals and selects the directory. Nothing else opens. |
| Search-field suggestion | Kept. Choosing it opens the New note dialog pre-filled with the search text, so there is one creation path. |
| Scope | The shortcuts work globally in the main workspace (editor, file browser, any drawer). |
| Templates | A seam only (optional initial content). No template UI. |
| Discoverability | A single `+` button in the FILES header opens a two-item menu (New note, New directory). Each item shows its shortcut. No labels and no second button. |

## Core

`kimun_core` already provides the primitives:

- `NoteVault::create_note(path, text)` is exclusive and returns `VaultError::NoteExists`.
- `NoteVault::create_directory(path)` returns `VaultError::DirectoryExists`.

Add or confirm in core (all path handling stays in core, per `CLAUDE.md`):

- An exclusive create that errors on an existing note. The TUI's `file_ops::create_note(.., None)` calls `load_or_create_note`, which loads an existing note, so it can't be reused for this dialog (it stays for follow-link).
- One function to turn user-typed text plus a base directory into a validated `VaultPath` for a note.
  It reuses the existing note-path cleanup (no hardcoded `.md` or `/`), rejects characters invalid on any of the three OSes, and lowercases.
  The TUI and GUI both use it, both for the live preview and for the actual creation.
- A matching function for directory paths.
- Creating a note whose parent directories are missing creates them first.
  Check that `nfs::create_note_exclusive` already does this; if it doesn't, add it in core and test it.
- The creation entry point takes `content: Option<String>` (empty by default).
  This is the templates seam: a later template feature supplies the content and nothing else changes.
  This matches the `content` argument the TUI `CreateNoteDialog` and the GUI create dialog already carry.
- Typed errors the frontends map to inline messages: invalid name, note exists, directory exists, name collides with an existing note or directory of the other kind.

Tests: core unit tests for path normalization, auto-creating parents, collisions, and invalid characters on the cross-platform set.

## Shared logic

The TUI keeps its file-operation logic in `tui/src/logic/file_ops` and the GUI uses `kimun_notes::logic::file_ops`.
Add `create_note_from_input` and `create_directory_from_input` there, each wrapping the core calls above and returning an outcome that:

- announces the created entry so the file browser can reveal and select it (`FileOp::Created` or an equivalent for directories),
- for notes, also opens the note (`announce_and_open`).

## TUI

### Dialogs

- `NewNoteDialog` is modeled on the existing `CreateNoteDialog` and `QuickNoteModal`.
  It has a text input, a read-only line showing the resolved target path, an inline error row, and the standard confirm/cancel hint chips.
  Enter creates and opens, and Escape cancels.
  It accepts optional initial text, used by the search-field suggestion.
- `NewDirectoryDialog` has the same shape but takes only a directory name and has no editor open step.
- Both use the shared modal chrome.

### Keys

| Action | Default chord | Leader | Notes |
|---|---|---|---|
| New note | `Ctrl+N` | `n n` (exists) | The leader route changes from "open FILES filter and flash a hint" to opening the dialog. |
| New directory | `Alt+N` | `n f` (new, "folder") | `n d` is already "daily". |
| FollowLink | `Ctrl+U` | unchanged | Moved off `Ctrl+N`. `Ctrl+Enter` still follows on kitty-protocol terminals (hardcoded in the editor screen). |

Both new actions are added to the action/shortcut enum, the command palette, the keyboard help, and the docs under `docs/`.
Both are rebindable through the existing keymap.

### Breaking change: FollowLink moves from `Ctrl+N` to `Ctrl+U`

- Update every reference to FollowLink being `Ctrl+N`: comments in `editor.rs`, `editor_input.rs`, `attachment_view.rs`, `footer_bar.rs`, `text_editor/mod.rs`, the footer/help hint text, and the `ctrl+n` expectation in `gui/src/keymap/tests.rs` (check what it asserts there).
- The default key table lives in `tui/src/settings/mod.rs` (`FollowLink` moves to `KeyStrike::KeyU`; `NewNote` takes `KeyStrike::KeyN`).
- Existing `config.toml` files written on first run may already contain `FollowLink = ["ctrl&N"]`.
  For those users the defaults would give two actions on one chord.
  This must be resolved during implementation. Options: a config migration that rewrites a persisted default FollowLink, or keeping the user's explicit binding and not auto-binding `NewNote` when its default chord is taken.
  The implementation plan picks one, and the release notes call this out either way.
- Ctrl letters in use, so none are reused: A C V X Z Y (editor), D K O P Q J T R B H L W E F, and now N (new note) and U (follow link).

### File browser

- The F2 menu acts on a selected entry (delete/rename/move), so it does not get the new actions. TUI discoverability is the command palette, the leader tree and the keyboard help.
- The search-field "Create: …" suggestion is rerouted to open `NewNoteDialog` pre-filled with the typed text, replacing its direct create.
- On creation, the browser navigates to the parent (if needed) and selects the new entry.
- Optional, not required for the first version: a clickable `[+]` chip in the browser header that opens the same two-item menu (the TUI already handles mouse clicks on hint chips). Skip it if the header is too tight. The palette and leader tree cover discoverability.

## GUI

### Dialogs

- Use the existing kit `Dialog` pattern from `gui/src/workspace/create.rs`, with the same `OpStatus` (busy/error) and footer helpers.
- The New note dialog has the same elements as the TUI one: input, live resolved path, inline error, Cancel and Create.
  Enter creates and opens it, and Escape cancels.
  Typing clears a shown error.
  While creating, the buttons wait and Escape does nothing, as in the current dialogs.
- The New directory dialog has the same shape, with no open step.
- Both register as modal kinds in `gui/src/workspace/modal.rs`.

### Keys

| Action | Default | Notes |
|---|---|---|
| New note | `primary+n` | `primary` is Cmd on macOS and Ctrl elsewhere. |
| New directory | `primary+shift+n` | |

- Add the actions in `gui/src/keymap.rs` next to `NewJournal` and `QuickNote`, and in the command palette and keyboard help.
- Check `primary+n` and `primary+shift+n` for conflicts with the OS or the gpui kit (new window) before committing to them.

### File browser

- The search-field create suggestion opens the New note dialog pre-filled with the typed text.
- On creation, FILES announces and reveals the entry (`FileOp::Created`) and selects it.

### `+` menu

- A single icon-only `+` button sits in the FILES panel header, in the existing header row (with the sort/filter controls), so it adds no row. It has a tooltip ("New…").
- Clicking it opens a small menu with two items: *New note* and *New directory*. Each item shows its shortcut on the right (the effective binding from the keymap, not a hardcoded string).
- Choosing an item opens the matching dialog, the same as the shortcut does.
- The target directory is the one the browser is showing, as for the shortcuts.
- It is disabled whenever the shortcuts are unavailable (no open vault, read-only).

## Later, not in scope

- A right-click context menu on a directory with "New note here" and "New directory here", which sets the target explicitly.

## Edge cases

- Input that is empty or only whitespace: the Create button/Enter is disabled or shows an inline error. Nothing is created.
- Input ending in `/` for a note: invalid for a note, with an inline error telling the user to use New directory.
- A note path that collides with an existing directory, or a directory that collides with a note: inline error from core.
- Names with characters invalid on any of the three OSes: inline error.
- Vault not open or read-only: the actions are unavailable, and the dialogs are never opened.
- Case collisions: paths are case-insensitive and lowercased, so `Foo` and `foo` collide and the existing-note error applies.
- Creating a directory that already exists: inline "already exists" error. It is not silent.

## Docs and release notes

- `docs/content`: add a short "Creating notes and directories" section covering both shortcuts, the leader keys, and the search-field flow.
  Update the keyboard reference and the `FollowLink` chord.
- Release notes: call out the TUI FollowLink move from `Ctrl+N` to `Ctrl+U`.

## Testing

- Core: unit tests as above.
- Shared logic: tests for `create_note_from_input` and `create_directory_from_input` (parents, collisions, outcomes).
- TUI: dialog tests (key handling, error display, pre-fill from the search suggestion), leader routing tests for `n n` and `n f`, keymap default tests for the new bindings, and a test that no two default actions share a chord.
- GUI: `*_tests.rs` in `gui/src/workspace` for both dialogs, the keymap tests, the search-suggestion routing, and the `+` menu (items, shown shortcuts, disabled state).

## Open items to settle during planning

1. Resolved: a v7 to v8 config migration (see the plan) rewrites a FollowLink still at the `Ctrl+N` default to `Ctrl+U`.
2. Exact wording of the dialog titles and hints.
3. Whether `primary+n` clashes with anything in the GUI kit.
