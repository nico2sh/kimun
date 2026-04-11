# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.2](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.7.1...kimun-notes-v0.7.2) - 2026-04-11

### Other

- cleanup release flow

## [0.7.1](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.7.0...kimun-notes-v0.7.1) - 2026-04-11

### Added

- version cli

### Other

- Merge pull request #69 from nico2sh/docs

## [0.7.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.6.0...kimun-notes-v0.7.0) - 2026-04-11

### Added

- relative paths in config workspaces
- multiple workspaces
- *(tui)* add type-ahead directory jumping to file browser
- *(tui)* replace VaultSection with WorkspacesSection in Settings
- *(tui)* add WorkspacesSection settings component
- *(tui)* wire workspace switcher into editor and main loop
- *(tui)* add WorkspaceSwitcherModal dialog
- *(tui)* add SwitchWorkspace shortcut (F4) and WorkspaceSwitched event
- *(tui)* wire BacklinksPanel into EditorScreen with Ctrl+E toggle
- *(tui)* add BacklinksPanel rendering
- *(tui)* add BacklinksPanel input handling and expand logic
- *(tui)* add ToggleBacklinks shortcut and backlinks events
- *(tui)* add BacklinksPanel component with context extraction
- quick note
- *(mcp)* add triage_inbox prompt for organizing inbox notes
- *(mcp)* add quick_note tool
- *(tui)* wire QuickNote shortcut (Ctrl+W) to editor screen
- *(tui)* add QuickNoteModal dialog component
- *(cli)* add kimun note triage command to list inbox notes
- *(cli)* add kimun note quick command
- *(config)* add inbox_path to workspace config

### Fixed

- small refactor
- init vault on workspace switch
- efficiency

### Other

- fixed docs
- gif of the app in action
- readme features
- examples
- workspace info
- clean up SharedSettings migration, remove workarounds
- update SettingsScreen to use SharedSettings
- update EditorScreen to use SharedSettings
- introduce SharedSettings, update App, events, StartScreen, BrowseScreen
- updated readme
- fixed docs
- tags
- added more about the quick note

## [0.6.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.5.0...kimun-notes-v0.6.0) - 2026-04-09

### Added

- *(editor)* open HelpDialog on F1; consume all unbound F-keys
- *(dialogs)* add HelpDialog and ActiveDialog::Help variant
- *(keys)* add ShortcutCategory, category(), and label() to ActionShortcuts
- *(tui)* replace simplelog with tracing; add init_logging and always-on log file
- *(tui)* render vault conflict error as a styled close-only dialog
- *(tui)* handle VaultConflict — clear vault path and show settings with error
- *(tui)* emit VaultConflict on CaseConflict instead of IndexingDone
- *(tui)* wire ScreenEvent::OpenSettingsWithError in switch_screen
- *(tui)* add VaultConflict event and SettingsScreen::new_with_error
- *(tui)* add AppSettings::clear_workspace for vault conflict handling

### Fixed

- improve shortcut consistency
- *(editor)* wire SearchNotes to open the note browser
- *(logging)* remove stderr log layer that corrupted the TUI display
- *(dialogs)* mention PgUp/PgDn in help modal footer hint
- address code review findings
- *(core,tui)* detect case conflicts in recreate_index and all settings reindex paths

### Other

- refactor dialogs
- one more todo item
- *(tui)* migrate log:: calls to tracing::
- *(tui)* swap simplelog for tracing ecosystem
- *(tui)* clarify VaultConflict and new_with_error doc comments
- improve clear_workspace() test coverage
- small cleanup removing hardcoded .md reference in the tui

## [0.5.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.4.0...kimun-notes-v0.5.0) - 2026-04-05

### Added

- mcp research

### Other

- improve the mcp prompts

## [0.4.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.3.9...kimun-notes-v0.4.0) - 2026-04-04

### Added

- *(mcp)* add link_suggestions prompt
- *(mcp)* add weekly_review prompt
- *(mcp)* implement rename_note and move_note tools
- *(mcp)* implement get_outlinks tool
- *(mcp)* implement brainstorm prompt
- *(mcp)* implement research_note prompt
- *(mcp)* implement find_connections prompt
- *(mcp)* implement daily_review prompt
- *(mcp)* implement MCP resources (list and read)
- *(mcp)* implement get_backlinks and get_chunks tools
- *(mcp)* implement journal tool
- *(mcp)* implement search_notes and list_notes tools
- *(mcp)* implement create_note, show_note, append_note tools
- *(mcp)* scaffold KimunHandler with all tool stubs and CLI wiring

### Fixed

- save note doesn't mess with path cases, plus tests
- no sidebar refresh on note load
- *(mcp)* distinguish domain errors from I/O failures in rename_note/move_note
- *(mcp)* use VaultPath methods instead of hardcoded .md and slash trimming
- *(mcp)* address code review — remove unused import, use .values(), propagate brainstorm errors
- *(mcp)* advertise resources capability; fix portable binary path in smoke test
- *(mcp)* remove unnecessary clone, strengthen append test assertion

### Other

- update MCP docs with new tools/prompts, add MCP section to README
- *(mcp)* update smoke tests for 11 tools and 6 prompts
- *(mcp)* add prompts/list smoke test asserting all 4 prompt names
- *(mcp)* split mcp.rs into mcp/ directory, scaffold prompt infrastructure
- *(mcp)* integration smoke test for tools/list over stdio
- *(deps)* add rmcp 1.3 for MCP server
- Update README.md
- Update README.md
- Update README.md

## [0.3.9](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.3.8...kimun-notes-v0.3.9) - 2026-04-02

### Added

- emoji support, not breaking lines

### Fixed

- tabs are properly rendered in nvim mode
- redraw on terminal resize
- *(test)* drain channel before asserting CloseDialog in esc test

### Other

- readme
- gihooks
- add shareable commit-msg hook and contributing docs

## [0.3.8](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.3.7...kimun-notes-v0.3.8) - 2026-04-01

### Other

- Merge pull request #49 from nico2sh/warns

## [0.3.7](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.3.6...kimun-notes-v0.3.7) - 2026-04-01

### Other

- logo
- logo
- improved docs
- Merge branch 'main' of github.com:nico2sh/notes
- badge
- Update README.md
- Update README.md
