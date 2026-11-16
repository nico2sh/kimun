# Spec: Default shortcuts for the TUI and the GUI

Status: implemented. Applies to both frontends.

## Goals

1. The two frontends match as far as the TUI allows. The TUI can only rely on `Ctrl+<letter>` and function keys.
2. Defaults follow what other apps do, so Kimün feels familiar.
3. One family of chords per kind of action. A layout move uses the same chord in the sidebar and in the editor splits.

## Rules

1. **Letter parity.** The GUI's `P+<letter>` matches the TUI's `Ctrl+<letter>` for the same action. `P` is the primary key: Cmd on macOS, Ctrl elsewhere.
2. **Standards win** where a chord has a near-universal meaning: Save, Find, Open, Quit, New tab, Close tab. The TUI picks another key on its side.
3. **Layout is the Alt family.** In the GUI every layout chord is `Alt` (`Cmd+Alt` on macOS, where Option types text). It acts on the focused leaf: a sidebar slot or an editor group.
4. **Differences the TUI forces are accepted** (see below). They are not bugs.
5. **Formatting stays on the leader.** No `P+B`, `P+I` or `P+K` for bold, italic or link. `P+K` is search notes (a quick finder, as in Slack and Notion). Editors use `P+Shift+F` for global search, so that is added as an alias.

## Final set

`P` = Cmd on macOS, Ctrl elsewhere. A dash means no default.

### Shared

| Action | TUI | GUI | Change |
|---|---|---|---|
| New note | Ctrl+N | P+N | none |
| New directory | `Ctrl+B n f` (alias Alt+N) | P+Shift+N | TUI: the leader is the real default; Alt+N stays as an alias |
| Save now | Ctrl+S | P+S | TUI: new |
| Open note | Ctrl+O | P+O | none |
| Search notes | Ctrl+K | P+K, P+Shift+F | GUI: alias added |
| Command palette | Ctrl+P | P+Shift+P | none |
| Leader | Ctrl+B | P+Shift+B | none |
| Today's journal | Ctrl+J | P+J | none |
| Open FILES | Ctrl+E | P+E | none |
| Find in buffer | Ctrl+F | P+F | none |
| Replace in buffer | none | Ctrl+H (Cmd+Alt+F on macOS) | none |
| Follow link | Ctrl+U, Ctrl+Enter | P+Enter | none |
| Quit | Ctrl+Q | P+Q | none |
| Sort dialog | Ctrl+R | P+R | none |
| Save query | Ctrl+D | P+D | none |
| Copy row | Ctrl+Y | P+C | none |
| Settings | F4, Ctrl+, | F4, P+, | none |
| File menu | F2 | F2 | none |
| Saved searches, workspace, Ask | F3, F5, F6 | F3, F5, F6 | none |
| Help | F1 | F1 | none |

### GUI layout family (Alt, or Cmd+Alt on macOS)

| Action | Chord |
|---|---|
| Focus region | Alt+Arrow |
| Move view to the neighbour | Alt+Shift+Arrow |
| Toggle left drawer | Alt+L |
| Toggle right drawer | Alt+R |
| Split right / down | Alt+Shift+R / Alt+Shift+D |
| Close slot | Alt+Shift+W |
| Zoom | Alt+M (was P+Shift+M) |
| Toggle rail | Alt+A (was P+Shift+A) |
| Focus editor / focus sidebar | Alt+E / Alt+F (Cmd+Alt+E / Cmd+Alt+S on macOS) |

### GUI documents (browser chords, not the Alt family)

New tab `P+T`, close tab `P+W`, next / previous tab `Ctrl+Tab` / `Ctrl+Shift+Tab`.
Quick note is `P+Shift+W`, since `P+W` closes a tab.

## Accepted differences

The TUI cannot use these, so the frontends differ:

| Action | TUI | GUI | Why |
|---|---|---|---|
| Quick note | Ctrl+W | P+Shift+W | `P+W` closes a tab; the TUI has no Shift |
| Toggle left drawer | Ctrl+T | Alt+L | `P+T` is a new tab |
| Directional focus | Ctrl+H / Ctrl+L (left, right) | Alt+Arrow | the TUI has Ctrl+letter only |
| Command palette | Ctrl+P | P+Shift+P | VS Code's chord in the GUI |

`Ctrl+H` and `Ctrl+J` can arrive as Backspace and Enter on old terminals without the kitty keyboard protocol.
The defaults stay: the trouble is limited to old terminals.

## Implementation notes

- TUI: add `Ctrl+S` to `default_keybindings` (`tui/src/settings/mod.rs`). `merge_missing_default_bindings` adds it to existing configs when the chord is free. Raw mode means flow control does not eat it.
- GUI: add `primary+shift+f` to `SearchNotes`, and move `ToggleZoom` and `ToggleRail` to `alt+m` / `alt+a` (`cmd+alt+m` / `cmd+alt+a` on macOS) in `gui/src/keymap.rs`. Check they do not collide with the vim rule (Alt+letter is not read by the engine).
- Existing users with a customised chord keep it. A default that finds its chord taken stays unbound.
- Docs: update `docs/content/using-kimun/keybindings.md`, `tui.md` and `desktop-app.md`.
- Release notes: the GUI zoom and rail chords move.

Focus editor and focus sidebar are targets, not a cycle: E is the editor, F the sidebar. The TUI's Ctrl+L / Ctrl+H are the same two actions, labelled by direction.

## Open items

1. **Missing-chord warning** (from the earlier review): when a default cannot bind because the user's config holds the chord, log a warning that names the action holding it. Decide when this is implemented.
2. **Back and forward:** `Alt+Left` and `Alt+Right` mean back and forward in browsers and IDEs. If note history navigation is added, it needs other chords.
