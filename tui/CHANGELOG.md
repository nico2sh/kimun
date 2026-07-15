# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.20.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.19.5...kimun-notes-v0.20.0) - 2026-07-15

### Added

- sqlite vector store
- server config in ui
- optional embedding
- semantic search working
- semantic search
- rag wiring

### Fixed

- bug hunt
- no semantic search when no server config
- fmt
- update and publish client
- semantic search collapsing to a single result
- working semantic search
- issues with absolute and relative paths

### Other

- hardening the server
- clippy
- server documentation
- panels deepened
- app events grouped
- renamed client to server reference instead of rag
- rag server renamed to kimun server
- dependencies upgrade

## [0.19.5](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.19.4...kimun-notes-v0.19.5) - 2026-06-18

### Added

- attachments are a first class file in the browser now

### Fixed

- removed duplication

## [0.19.4](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.19.3...kimun-notes-v0.19.4) - 2026-06-17

### Fixed

- better error messages in cli
- small bugs found
- proper count of matches in preview
- clean

### Other

- fmt
- consolidate error messages from mcp and skills/cli
- extract the preview from the query
- added tests for normal mode parsing
- unified highlighting
- single query resolution path

## [0.19.3](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.19.2...kimun-notes-v0.19.3) - 2026-06-16

### Fixed

- reuse function
- bug for zero width clusters
- emoji proof footer
- emoji proof text area
- include selection without changing selection
- more cases excluding cursor on selection
- in vim mode, cursor position is included in selection

### Other

- Merge pull request #154 from nico2sh/cursor_vim
- fmt

## [0.19.2](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.19.1...kimun-notes-v0.19.2) - 2026-06-16

### Added

- mouse option to disable mouse interactions

### Fixed

- preferences with F4

### Other

- fmt
- added instructions to reindex in skills and mcp if a note has been changed externally
- better interdependency definition with core

## [0.19.1](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.19.0...kimun-notes-v0.19.1) - 2026-06-15

### Fixed

- cleanup nvim
- cleanup

### Other

- Merge pull request #146 from nico2sh/nvim_indent
- fmt
- ctrl R in nvim
- extract nvim core
- update instructions on script

## [0.19.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.18.0...kimun-notes-v0.19.0) - 2026-06-14

### Added

- generic update provider
- update preference
- update dialog
- update notif
- arm linux builds
- build with correct version
- version check module

### Fixed

- cr

### Other

- Merge pull request #143 from nico2sh/auto-update
- clippy
- fmt
- cleanup for updates

## [0.18.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.17.0...kimun-notes-v0.18.0) - 2026-06-13

### Added

- bouncing umlaut dots in welcome banner
- replace welcome logo with Kimün ascii wordmark
- block-art logo on welcome step, natural text wrapping, aligned summary block
- onboarding welcome step, capped dialog size, centered layout
- route first run (no workspace) to onboarding; handle OnboardingFinished
- onboarding summary step, atomic finish commit, quit/discard flows
- onboarding theme step (live preview) and editor-backend step (nvim probe)
- onboarding nerd-fonts step with glyph self-test and live icon preview
- onboarding workspace step — suggestion, browser with mkdir, rerun list
- OnboardingScreen skeleton — dialog frame, step state machine, navigation
- app.onboarding leader action (guided setup) under +vault, palette-visible
- OpenOnboarding/OnboardingFinished events, default workspace suggestion

### Fixed

- final intro animation
- small bugfixes
- column-align nerd-font glyphs with ascii counterparts
- keep marker-column alignment in nerd-fonts sample rows
- clear stale flash message on onboarding step transitions
- reject path separators in create_dir, honest test name
- hints for focus

### Other

- fmt
- bouncing times fixed and final
- bouncing times
- fmt
- drop stale allow, snap selection off disabled nvim, e2e walkthrough test
- clean up scratch config files in onboarding finish tests
- cover invalid workspace name rejection in onboarding name edit
- extract FileBrowserState into components::dir_browser, add create_dir
- Merge pull request #139 from nico2sh/hints

## [0.17.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.16.1...kimun-notes-v0.17.0) - 2026-06-11

### Added

- new vim commands
- vim Visual p/P replaces selection with register
- mouse-drag enters vim Visual mode
- Space as vim Normal-mode leader (intrinsic, pending-safe)
- vim : / ? n N route to palette + find bar
- command palette resolves exact vim Ex aliases (:w/:q)
- note.save + app.quit leader actions with vim aliases
- vim pending-command footer hint + indent >>/<<
- vim dot-repeat (.) with insert-delta capture
- vim Visual + Visual-line modes
- vim % matching-pair jump (single-line)
- vim text objects iw/aw/i"/a"/i(/a( (single-line)
- vim find-char f/F/t/T + ;/,
- vim edits x/X/s/S/r/J/~ + u/Ctrl-r
- vim operators d/c/y, dd/cc/yy, D/C/Y, paste p/P
- reified vim command model, counts, motion resolution
- reset vim mode to Normal on note open
- block cursor in normal mode, bar in insert (SetCursorStyle)
- generalize footer mode label to vim backend
- route keys through VimEngine in handle_input
- VimEngine skeleton — normal motions, insert entry, esc
- TextareaBackend + InputInterpreter, vim settings variant

### Fixed

- concurrent tests fix
- cursor on focus change in vim mode
- horizontal scroll on text input
- indent cursor behavior corrected
- proper indent with >>
- motion issues
- small bug fixes
- vim search Enter confirms+closes bar; n/N navigate persisted pattern
- route keys to find bar before vim engine (vim / search input)
- vim visual-p yanks replaced selection; Esc in Normal clears stray selection
- vim yank leaves cursor at selection start; charwise p never wraps to next line
- charwise Visual selection inclusive of cursor char (operators + highlight)
- vim e/de/ce land on last word char (was off by one)
- vim dot-repeat captures full multi-line insert delta at Esc (was single-line/fragile)
- vim text-object nesting/quote-gap, df newline, cc single-line, r no-op
- drop command-palette Ex-alias layer (overrode selected row); keep real save/quit entries
- vim object-range panic on empty line + Esc clears pending in Normal

### Other

- Merge pull request #137 from nico2sh/vim
- cluppy
- clippy and fmt
- vim mode in docs
- better vim structure
- Jump-based vim motions, alloc-free cursor-shape classify, shared motion table
- clean up unused vim Command variants and lint warnings
- rename NvimMode to shared EditorMode

## [0.16.1](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.16.0...kimun-notes-v0.16.1) - 2026-06-10

### Fixed

- avoid using Send on appscreen
- upgrade dependencies

### Other

- clippy

## [0.16.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.15.0...kimun-notes-v0.16.0) - 2026-06-09

### Added

- *(editor)* retarget-in-place on open-note rename, targeted sidebar row update
- *(editor)* mark open note on open, retitle sidebar row on save
- *(sidebar)* track open note, stamp marker, targeted title/rename row updates
- *(file_list)* is_open flag, display_title helper, accent glyph for open note
- *(search_list)* add update_rows in-place mutation seam

### Fixed

- *(search_list)* refresh leading row on query change for reload sources; dedupe sidebar test helpers
- *(editor,sidebar)* refresh drawers + journal_date on rename, guard reload failure
- *(editor)* abort in-flight autosave on open-note rename to avoid resurrecting old path
- *(sidebar)* collapse if-lets (clippy), restore hit-test comment, assert renamed filename
- small correctness
- new note refreshes browse sidebar
- create journal from browse screen

### Other

- cargo fmt
- *(search_list)* recompute display only when poll drains new rows
- *(file_list)* assert open-note glyph is accent-colored
- *(search_list)* cover update_rows re-filter of the visible view
- Update README.md
- updated shortcuts in readme

## [0.15.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.14.0...kimun-notes-v0.15.0) - 2026-06-08

### Added

- mouse actions
- command palette
- hint on leader key
- leader key
- sections enabled for strip
- activity rail stub

### Fixed

- multiline selection indent fixed
- mouse position cursor correctly continue lists and indents
- editor small refactor
- open drawer at current dir
- small fixes
- ghostty option arrow nav
- small desig improvements
- icons in results
- fixed preferences shortcuts
- better open by search highlights
- bugfixes
- settings from keys
- ctrl+enter on preview notes
- vault icon

### Other

- cargo fmt
- screenshot width
- query text help and correct space behavior in search
- config icon
- Merge pull request #127 from nico2sh/revamp
- screenshot size
- updated readme with screenshot
- tunued shortcuts
- cargo fmt
- renamed rail labels
- settings to preferences
- texteditor backend separation
- better panels
- window chrome
- custom leader keys
- colors in themes
- Merge branch 'main' of github.com:nico2sh/kimun

## [0.14.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.13.0...kimun-notes-v0.14.0) - 2026-06-04

### Added

- new themes
- scrolling the preview, scrolls the content

### Fixed

- correctness
- clicking on header collapses preview

### Other

- small restructure of themes definition

## [0.13.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.12.0...kimun-notes-v0.13.0) - 2026-06-04

### Added

- autosurround with closing characters on selection

### Fixed

- fixes for autosurround
- scrol anywhere in single place
- fmt
- code review
- proper {} relacement
- consistency
- small fixes
- saved search works
- fmt
- focus on panels
- fmt and clippy
- code review 2

### Other

- bare note operator expands to current open note
- docs references in code
- drop remaining ADR references from tui comments
- NoteDetails revamp
- refactor the db layer

## [0.12.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.11.3...kimun-notes-v0.12.0) - 2026-06-03

### Added

- *(tui)* inline ?name saved-search expansion with sticky breadcrumb
- *(tui)* open sort dialog and route sort events from editor
- *(tui)* register ActiveDialog::Sort variant
- *(tui)* add SortDialog overlay component
- *(tui)* query panel sort via query rewrite; drop sort override
- *(tui)* sidebar directory grouping + apply_sort via dialog
- *(tui)* add SortTarget and sort dialog events
- *(tui)* add group_directories setting; bind OpenSortDialog to Ctrl+N
- *(tui)* replace sort field/order actions with OpenSortDialog
- *(tui)* add reverse SortField/SortOrder setting conversions
- *(tui)* save query from the Ctrl+K note browser; add save/searches hints
- *(tui)* ActiveDialog impls Overlay; migrate dialog app-message handling; rename CloseDialog->CloseOverlay
- *(tui)* add CloseOverlay event; modals impl Overlay and emit it
- *(tui)* add OverlayHost + Overlay trait (single-slot, focus save/restore)
- *(tui)* note-name autocomplete on </>/= operators (ADR-0005 alphabet)
- *(core)* query operator alphabet (ADR-0005) + forward-links filter
- *(tui)* SearchList .intercept + render/render_query/render_autocomplete + handle_mouse
- *(tui)* autocomplete host inside SearchList (one canonical snapshot)
- *(tui)* SuggestionSource port (SuggestionItem) — autocomplete decoupled from the vault
- *(tui)* SearchList Filter enum (SourceOrder/Fuzzy/Rank) + display indices
- *(tui)* SearchList keyboard nav + KeyReaction verdict
- *(tui)* SearchList requery aborts the prior load (generation guard)
- *(tui)* SearchList engine skeleton + generation-stamped one-shot load
- *(tui)* SearchList seams — SearchRow, RowSource, Emit, Loaded
- *(tui)* open saved searches modal; apply selection to query panel
- *(tui)* saved searches modal — filter + numeric quick-select + virtual backlinks entry
- *(tui)* SaveCurrentQuery opens save dialog; persist SaveSearchConfirmed via core
- *(tui)* save-search name dialog
- *(tui)* rename ToggleBacklinks->ToggleQueryPanel (+legacy alias); add OpenSavedSearches & SaveCurrentQuery actions
- *(tui)* query panel — editable query line sourced from search_notes
- expose SearchTerms; generalise context preview to query-needle highlighting
- *(tui)* SearchQuery autocomplete mode — > suggests note names + {note}
- *(tui)* LinkFilter (>/->) autocomplete trigger detection
- *(tui)* {note} query-variable resolution
- --preview / preview dry-run for note replace
- regex support for note replace
- *(mcp)* add overwrite_note/replace_in_note/delete_note tools
- *(cli)* add note overwrite/replace/delete with backups
- *(core)* add link query operator (>/lk:) and search optimizations
- *(editor)* draw blockquote bar gutter, reveal raw '> ' on edit
- *(editor)* compute blockquote gutter insets and feed them to wrap
- *(editor)* per-row left inset in word wrap for the blockquote gutter
- *(editor)* reveal blockquote marker in wrap mask only on cursor row
- *(editor)* store blockquote depth + sigil end on ParsedLine
- *(editor)* paint code_bg box behind fenced and indented blocks
- *(editor)* cache per-row code-box width at the view layer
- *(editor)* add code_block_ranges_from_kinds (fenced + indented)
- *(theme)* add blockquote_bar and code_bg colors

### Fixed

- expand spaces on notes placeholder
- cargo fmt
- resolve relative path on note check
- fmt
- better event management
- hints on the sidebar focus
- *(tui)* use sort_by_key for directiveless query order (clippy)
- *(tui)* correct sort defaults, query ordering, saved-search title, render cost
- *(tui)* sync query input bar on programmatic query change
- *(tui)* resolve {note} query variable in the Ctrl+K note browser
- *(tui)* rebind SaveCurrentQuery to Ctrl+D (Ctrl-only; Ctrl+Shift unreliable on some terminals)
- *(tui)* suppress overlay-openers under capture-all; drop redundant modal CloseOverlay; remove dead OverlayMsg::Close
- *(tui)* keep Query-panel focus after saved-search select (guard CloseOverlay restore)
- *(tui)* log sidebar listing failures; skip QueryPanel startup search on empty note
- *(tui)* SearchList leading_row works for streamed sources, query-fresh and pinned
- code-review low — forward-link highlight, dedup link normalization, trigger scan, prefix-table tests
- code-review high+medium — stale CLI docs/skill alphabet, popup operator sigil, forward-link indexes
- *(tui)* Enter accepts autocomplete in the query panel
- *(tui)* saved-search delete ordering, note-browser preview refresh, sidebar dir filtering
- *(tui)* restore interactive sort-cycling in the sidebar (CycleSortField/SortReverseOrder)
- *(tui)* SearchList mouse hit-test honours visual_height; close popup on mouse
- *(tui)* order saved-search delete before reload; drop stale dead_code allow
- apply bold/italic to nested links and wikilinks
- render wikilinks and inline formatting inside headers
- close review holes — rename locking, atomic append, fail-closed backup, MCP polish
- *(core)* atomic backups, once-daily purge, backups via shared vault
- *(core)* back up rename/move backlink victims; harden index exclusion
- *(editor)* a less-indented line ends the indented code block
- *(editor)* exclude trailing blank line from indented code blocks
- *(theme)* give the ANSI theme a visible code-block background
- *(workspace)* deterministic config serialization order
- *(editor)* blockquote selection/click + code-block tab alignment
- *(editor)* draw blockquote bar on lazy-continuation lines
- *(editor)* account for blockquote gutter in mouse click mapping
- *(editor)* cursor no longer sticks on the blockquote '>' marker

### Other

- Merge pull request #115 from nico2sh/rel_paths
- *(tui)* SidebarComponent::from_settings, shared by both screens
- *(tui)* symmetric overlay dismissal (close-side of Candidate B)
- *(tui)* centralize overlay presentation; fix OpenJournal overlay
- *(tui)* deepen persistent panels behind a Panel seam
- *(tui)* tidy sort dialog plumbing (review items 5-8)
- cargo fmt
- *(tui)* persist sort default only for sidebar; test journal branch
- *(tui)* fix stale BacklinkSource doc comment after sort removal
- cargo fmt
- *(tui)* move Overlay trait to components; make OverlayHost generic over focus (store Focus, drop u8 token)
- *(tui)* OverlayKind::label, ActiveDialog Show* constructors, fold Show*/CloseOverlay into owned match (drop path clones)
- *(tui)* remove CloseNoteBrowser/CloseSavedSearches/CloseDialog; drop redundant Component impls
- *(tui)* EditorScreen routes overlays through OverlayHost; collapse Focus enum; delete DialogManager
- *(tui)* code-review cleanup — debounce builder knob, dedup centered_rect, drop dead search_str/list_rect, consistent mouse-rect contract
- *(tui)* robust poll_until_idle for vault-backed loads (fix parallel-suite flake)
- *(tui)* collision-free temp_vault nonce (atomic counter + pid)
- *(tui)* sweep remaining refs to ADR-0005 alphabet (virtual entry, CLI/MCP help, docs)
- rustfmt SearchList refactor
- *(tui)* retire FileListComponent — list engine absorbed by SearchList
- *(tui)* sidebar hosts SearchList (streamed source + Filter::Fuzzy)
- *(tui)* Query panel hosts SearchList; expand/preview compose on top; drop BacklinksLoaded
- *(tui)* saved searches modal hosts SearchList (Filter::Rank + leading_row)
- *(tui)* note browser hosts SearchList; providers are RowSources
- rustfmt across saved-searches feature files
- *(tui)* regression tests for {note}-gated query-panel refresh on navigation
- cargo fmt for note-modify/backup code
- *(mcp)* cover overwrite/replace/delete tools
- Merge pull request #100 from nico2sh/feat/editor-bq-code-styling
- *(editor)* simplify tab-width, code-box rebuild, indented-blank trim
- *(editor)* fenced code text uses fg, matching indented code
- *(editor)* dedup + single-source the blockquote/code-block helpers
- *(editor)* cargo fmt blockquote + code-box changes

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
