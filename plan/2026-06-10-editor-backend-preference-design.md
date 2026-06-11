# Editor backend selection in the Preferences window

Date: 2026-06-10 · Status: approved

## Goal

Let the user change the `editor_backend` setting (textarea / vim / nvim) from
the TUI Preferences window, without editing `config.toml` by hand.

## Decisions

- **Placement**: a second row in the existing **Editor** section (next to the
  autosave slider). No new sidebar section.
- **Apply timing**: on save, effective the next time a note is opened. The
  existing `PreferencesSaved` flow (save to disk → rebuild → Start screen)
  already reconstructs the backend on the next editor open; no live hot-swap.

## Design

### `tui/src/components/preferences/editor_section.rs`

`EditorSection` gains:

- `pub editor_backend: EditorBackendSetting`
- `selected_row: usize` — 0 = Autosave Interval, 1 = Editor Backend

Keys:

- `↑/↓` (`k`/`j`): switch row (Sorting-section pattern)
- `←/→` (`h`/`l`): change selected row's value — autosave ±5s (unchanged),
  backend cycles Textarea → Vim → Nvim (← reverses)
- `Enter`/`Space`: cycles backend when its row is selected

Render: two label+value rows; selected row gets the accent style; backend row
shows `◀  Vim (built-in)  ▶` with labels `Textarea` / `Vim (built-in)` /
`Nvim (external)` and a dim hint "applies when a note is opened". Nvim with no
binary silently falls back to textarea (existing `from_settings` behavior).

### `tui/src/app_screen/preferences.rs`

- Constructor passes `s.editor_backend` to `EditorSection::new`.
- The `PreferencesSection::Editor` input arm also writes
  `settings.editor_backend = editor_section.editor_backend`.

## Out of scope

Live backend switch of an open editor; editing `nvim_path` from the UI.

## Tests

In `editor_section.rs`: backend cycle wraps both directions; row navigation
routes `←/→` to the selected row's value; autosave behavior unchanged on row 0;
Enter cycles backend only on its row.
