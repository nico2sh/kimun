# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.11.3](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.11.2...kimun-notes-v0.11.3) - 2026-05-29

### Fixed

- *(editor)* recompute spliced region's reset_boundaries
- *(editor)* drop slice sentinels when merging reset_boundaries
- *(editor)* wrap on grapheme-cluster boundaries by display width
- *(editor)* run incremental-splice verify in release for non-provable tiers
- *(editor)* guard async-parse placeholder + redraw on first open
- *(editor)* clear last_splice_path on full-rebuild + cleanup post-soak debt
- *(editor)* narrow §3.0 to ListMarker + lazy_depth==1 after 100k soak
- *(editor)* abort the autosave handle on try_save timeout
- *(editor)* empty-stack undo/redo/delete is a true no-op
- *(editor)* text_revision starts at 1 and skips 0 on wraparound
- *(editor)* drop dead cursor_code_block field + obsolete test
- *(editor)* guard parsed_cache + visual_lines on stale cursor
- *(editor)* cap try_save awaits at 5s to prevent quit hang
- *(main-loop)* drain-break uses screen_generation, not ScreenKind
- *(editor)* only drop autosave_task on completion when it's finished
- *(editor)* mark_saved_at_revision no-ops on stale completion
- *(editor)* nvim path bumps text_revision so autosave loses no edits
- *(event-handler)* try_next panics on channel disconnect
- *(editor)* nvim path bumps cursor counter, not text_revision
- *(main-loop)* break event drain when screen identity changes
- *(editor)* autosave completion uses revision, not text equality
- *(autocomplete)* abort in-flight query on close + drop
- *(editor)* clear dirty marker on set_text no-op reload
- *(editor)* only bump text_revision when buffer truly changes
- *(editor)* serialize autosave + recover from panicked tasks
- *(editor)* don't clear Nvim dirty flag on divergent mark_saved
- *(editor)* render every fenced code block with code-block style
- *(editor)* split fence cache into text-keyed list + cursor lookup
- *(editor)* track is_dirty against text_revision, not edit_generation

### Other

- *(autocomplete)* cargo fmt lazy-zone changes
- *(autocomplete)* compute exclusion zones lazily
- *(editor)* collapse nested ifs into let-chains (clippy)
- *(editor)* cargo fmt parse_state accessor call sites
- *(editor)* expose last_parse_was_incremental via accessor
- *(editor)* assert reset_boundary Blank invariant in splice
- *(editor)* shrink MarkdownEditorView interface
- *(editor)* model parse cache as ParseState enum
- *(editor)* drop write-only widener telemetry counters
- *(editor)* consolidate block-opener heuristics into one classifier
- *(editor)* linear-merge reset_boundaries on splice instead of re-sort
- *(editor)* collapse widener to two tiers, drop IntraConstruct
- cargo fmt
- *(editor)* demote IntraConstruct verify to debug+env-only (PR 3)
- *(editor)* three-tier widener + lazy-guard relax (PR 2 of 2)
- *(editor)* intra_construct_boundaries field + tracking (PR 1 of 2)
- *(editor)* ship parse-reset-boundaries-v2 hybrid widener
- fix clippy warnings
- cargo fmt
- *(editor)* typing-path overhaul — review fixes, async parse, snapshot, task slot
- tui benches
- *(editor)* fast-path bail in sync_autocomplete
- *(editor)* full-cycle bench for view.update + Clone on MarkdownEditorView
- *(editor)* incremental WordWrapLayout in Gate 2
- *(editor)* add WordWrapLayout::splice_range for damage-based wrap
- *(editor)* incremental rendered_cache rebuild on text changes
- *(editor)* use iter_batched for incremental bench
- *(editor)* criterion benches for parse + wrap
- *(editor)* structural-marker fallback + proptest harness
- *(editor)* explicit tests for G1 (nested list), G3 (hashtag in fence), G8 (empty/1-line)
- *(editor)* lock in correctness for structural-edit shapes
- *(editor)* debug-only correctness assertion for incremental parses
- *(editor)* wire try_incremental_parse into Gate 1
- *(editor)* store ParsedBuffer in view; derive fence_ranges from kinds
- *(editor)* derive fence ranges from LineConstructKind
- *(editor)* fix widen_up over-pull across blank-separated lists
- *(editor)* add widen_to_safe with cap-aware widening
- *(editor)* tighten compute_damage_range fast path to O(window)
- *(editor)* add compute_damage_range with cursor-row hint
- *(editor)* harden ParsedBuffer::splice invariants + tests
- *(editor)* add ParsedBuffer::parse_range and splice
- *(editor)* tidy ParsedBuffer::parse kinds-population
- *(editor)* populate LineConstructKind during ParsedBuffer::parse
- *(editor)* convert ParsedBuffer to a struct with lines + kinds fields
- *(editor)* scaffold parse_incremental module
- close the three remaining low-priority coverage gaps
- *(editor)* pin NoOp shortcut + nonzero initial text_revision
- *(editor)* document the mark_saved vs mark_saved_at_revision asymmetry
- *(event-handler)* try_next panic message names the right sender
- *(autocomplete)* tighten DEFAULT_DEBOUNCE comment
- *(editor)* document NvimBackend::set_text TOCTOU contract
- *(autocomplete)* skip cache write on revision==0 sentinel
- *(autocomplete)* cache buffer text alongside exclusion zones
- *(autocomplete)* cache exclusion zones per buffer revision
- *(main-loop)* coalesce queued events between draws
- *(autocomplete)* debounce refinement queries by 80ms
- *(editor)* slice-bounded grapheme walk in render_with
- *(editor)* gate view caches on text_revision, not edit_generation
- *(editor)* avoid per-keystroke buffer snapshots
- *(editor)* make periodic autosave non-blocking
- *(editor)* cache is_dirty via saved_generation

## [0.11.2](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.11.1...kimun-notes-v0.11.2) - 2026-05-26

### Fixed

- added benches, ignore hashtags with double ##

## [0.11.1](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.11.0...kimun-notes-v0.11.1) - 2026-05-25

### Added

- *(autocomplete)* wire hashtag popup into search box
- *(autocomplete)* wire wikilink + hashtag popup into editor
- *(autocomplete)* host trait + controller wiring
- *(autocomplete)* popup state machine, key handling, ratatui widget
- *(autocomplete)* trigger detection + core exclusion-zone helper

### Fixed

- *(autocomplete)* batch 4 — round-3 low-severity cleanup
- *(autocomplete)* batch 3 — popup hardening + wikilink in code
- *(autocomplete)* batch 2 — host integration + focus
- *(autocomplete)* batch 1 — controller behavior + redraw + cancellation
- *(autocomplete)* activate after nvim → textarea fallback
- *(autocomplete)* disable exclusion-zone check in search box
- *(autocomplete)* tight clamp + fresh anchor at render time
- *(autocomplete)* split sync (on edit) from refresh-if-open (on move)
- *(autocomplete)* close popup when find bar opens
- *(autocomplete)* empty popup is not interactive
- *(autocomplete)* consume stale wikilink target on accept
- *(autocomplete)* disable column-0 header rule in search box
- *(autocomplete)* detect text edits by buffer length, not edit_generation
- *(autocomplete)* only open on text edit, dismiss on note open

### Other

- collapse nested ifs to fix clippy warnings
- cargo fmt --all
- *(autocomplete)* collapse nested ifs, drop dead code, use saturating_sub
- exclusions now use the prefix - before the operators in search

## [0.11.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.10.0...kimun-notes-v0.11.0) - 2026-05-25

### Added

- *(cli)* add `kimun labels` command to list vault labels with note counts
- *(tui/editor)* follow-link on #label opens search modal pre-filled
- *(tui/note_browser)* accept pre-filled initial query on open
- *(tui/editor)* highlight #hashtag spans as Label elements

### Fixed

- *(cli/metadata)* lowercase frontmatter tags for case parity with body labels
- *(cli/metadata)* delegate hashtag extraction to core for parity with index
- *(core)* symmetric word-boundary check + sync spec/MCP docs with extraction rules
- *(tui/editor)* delegate label detection to core, skip labels inside links/code/fences, widen elem_index to u16
- *(tui/note_browser)* single schedule_load in with_initial_query, eliminating empty-load race
- bad shortcuts config don't break config

### Other

- cargo fmt --all
- tasks in readme
- *(mcp)* include [[wikilink]] in search_notes hashtag-exclusion list
- *(mcp)* mention #label / lb:label syntax in search_notes tool description

## [0.10.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.9.0...kimun-notes-v0.10.0) - 2026-05-18

### Added

- text find

### Other

- Merge pull request #87 from nico2sh/new_ta
- simplify
- format
- refactor
- updated textarea

## [0.9.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.8.0...kimun-notes-v0.9.0) - 2026-05-09

### Added

- *(themes)* add ANSI dark and light built-in themes

### Fixed

- reset on panel
- mapping of ansi colors to ratatui's

### Other

- Merge pull request #77 from MGross21/feat/ansi-colors
- *(themes)* simplify ANSI theme — built-in only, not a color format
- Merge remote-tracking branch 'origin' into feat/ansi-colors

## [0.8.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.7.3...kimun-notes-v0.8.0) - 2026-05-05

### Added

- *(tui)* omit last_paths from config writes
- *(tui)* v2->v3 migration moves DB and extracts history
- *(tui)* place SQLite cache in config dir per workspace
- *(tui)* add per-workspace history file module
- *(tui)* validate workspace name on add_workspace
- *(tui)* add cache_dir and history_dir to AppSettings
- *(core)* NoteVault::new accepts VaultConfig

### Fixed

- *(tui)* workspace rename/remove relocate cache and history files
- *(tui)* wire cache_path_for through every NoteVault::new
- *(tui)* lowercase workspace names + back up config before v3 migration
- *(tui)* clean up tmp file on history write failure
- better chunk separator and other refactors
- fixed sidebar focus on click

### Other

- cargo fmt
- gitignore *.kimuncache and untrack existing ones
- updated the example config
- *(tui)* extract path-extension constants and join helper
- *(tui)* polish history + migration internals
- cargo fmt + clippy fix in app.rs vault construction
- *(tui)* route add_path_history through history file
- align all config filename references with actual written filename
- small refactor

## [0.7.3](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.7.2...kimun-notes-v0.7.3) - 2026-04-21

### Added

- use ParsedBuffer in view; delete detect_list_marker
- add ParsedBuffer::parse for whole-buffer markdown parsing

### Fixed

- detect list marker by scanning from col 0

### Other

- fix pre-existing clippy 1.95 lints
- collapse match guards in ParsedBuffer::parse
- cargo fmt
- explain line_starts sentinel intent
- extract shared list_marker_len helper
- pin setext heading and multi-line blockquote behaviour
- read list_sigil_end from ParsedLine instead of re-detecting
- add list_sigil_end field to ParsedLine
- add nested list rendering tests (currently failing)

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
