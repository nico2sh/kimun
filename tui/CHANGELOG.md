# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0](https://github.com/nico2sh/kimun/compare/kimun-notes-v0.5.0...kimun-notes-v0.6.0) - 2026-04-05

### Other

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
