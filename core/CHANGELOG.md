# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.4](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.3...kimun_core-v0.2.4) - 2026-04-07

### Added

- *(core)* add app_log_dir() returning platform-specific log directory
- *(core)* add app_log_dir() returning platform-specific log directory
- *(core)* reject control chars, Windows reserved names, trailing dots/spaces in vault paths
- *(core)* reject vault on case-insensitive path conflicts

### Fixed

- *(core)* skip case-duplicate assertions on case-insensitive filesystems
- *(core)* append app dir name to Windows fallback path in app_log_dir
- address code review findings
- *(core,tui)* detect case conflicts in recreate_index and all settings reindex paths

### Other

- small cleanup removing hardcoded .md reference in the tui

## [0.2.3](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.2...kimun_core-v0.2.3) - 2026-04-04

### Fixed

- save note doesn't mess with path cases, plus tests
- no sidebar refresh on note load
- resolve case insensitive paths

### Other

- core readme
- comment on render
- *(core)* get_markdown_and_links takes &VaultPath instead of &NoteDetails

## [0.2.2](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.1...kimun_core-v0.2.2) - 2026-04-01

### Other

- Merge pull request #49 from nico2sh/warns

## [0.2.1](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.0...kimun_core-v0.2.1) - 2026-04-01

### Other

- homebrew
