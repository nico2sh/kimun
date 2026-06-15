# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.24](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.23...kimun_core-v0.2.24) - 2026-06-15

### Other

- update Cargo.toml dependencies

## [0.2.23](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.22...kimun_core-v0.2.23) - 2026-06-15

### Other

- update Cargo.toml dependencies

## [0.2.22](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.21...kimun_core-v0.2.22) - 2026-06-14

### Other

- update Cargo.toml dependencies

## [0.2.21](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.20...kimun_core-v0.2.21) - 2026-06-13

### Other

- update Cargo.toml dependencies

## [0.2.20](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.19...kimun_core-v0.2.20) - 2026-06-11

### Other

- update Cargo.toml dependencies

## [0.2.19](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.18...kimun_core-v0.2.19) - 2026-06-10

### Fixed

- upgrade dependencies

## [0.2.18](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.17...kimun_core-v0.2.18) - 2026-06-09

### Fixed

- new note refreshes browse sidebar

### Other

- cargo fmt

## [0.2.17](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.16...kimun_core-v0.2.17) - 2026-06-08

### Added

- command palette

### Other

- cargo fmt

## [0.2.16](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.15...kimun_core-v0.2.16) - 2026-06-04

### Other

- update Cargo.toml dependencies

## [0.2.15](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.14...kimun_core-v0.2.15) - 2026-06-04

### Fixed

- fmt
- code review
- proper {} relacement
- consistency
- fmt and clippy
- code review 2
- code review

### Other

- docs references in code
- drop ADR references from rustdoc
- refresh core README against current API
- document all public items in core (28% → 100% coverage)
- fix unresolved intra-doc links in core
- improve rewrite link pipeline
- NoteDetails revamp
- refactor the db layer

## [0.2.14](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.13...kimun_core-v0.2.14) - 2026-06-03

### Added

- *(core)* saved-search prefix suggestion lookup
- *(tui)* query panel sort via query rewrite; drop sort override
- *(core)* add with_order_directive query rewrite helper
- *(core)* wildcard (*) support for the / path operator
- *(core)* wildcard (*) support for the = / name: filename operator
- *(core)* query operator alphabet (ADR-0005) + forward-links filter
- expose SearchTerms; generalise context preview to query-needle highlighting
- *(core)* NoteVault saved-search list/save/delete/rename API
- *(core)* SavedSearch model + .kimun/saved-searches.toml read/write
- *(core)* add FSError::SerializationError for saved-search (de)serialization
- --preview / preview dry-run for note replace
- regex support for note replace
- *(mcp)* add overwrite_note/replace_in_note/delete_note tools
- *(core)* note overwrite/replace/delete with automated-edit backups
- *(core)* add link query operator (>/lk:) and search optimizations

### Fixed

- clippy
- expand spaces on notes placeholder
- resolve relative path on note check
- *(tui)* correct sort defaults, query ordering, saved-search title, render cost
- code-review low — forward-link highlight, dedup link normalization, trigger scan, prefix-table tests
- code-review high+medium — stale CLI docs/skill alphabet, popup operator sigil, forward-link indexes
- close review holes — rename locking, atomic append, fail-closed backup, MCP polish
- *(core)* serialize per-note writes to prevent lost updates
- *(core)* atomic backups, once-daily purge, backups via shared vault
- *(core)* back up rename/move backlink victims; harden index exclusion

### Other

- *(tui)* tidy sort dialog plumbing (review items 5-8)
- cargo fmt
- *(core)* derive both order prefixes from ORDER_LETTER; test multi-directive strip
- rustfmt across saved-searches feature files
- cargo fmt for note-modify/backup code

### Added

- *(core)* link query operator `>` / `lk:` (and exclusion `->` / `-lk:`) to filter notes by the notes they link to (backlinks); matches by note name with optional extension, path disambiguation, and `*` wildcards
- *(core)* indexed `dest_name` column on the `links` table so the link filter matches by name with an index lookup instead of a full-table scan; bumps the cache version (existing vaults reindex on next launch)

### Fixed

- *(core)* search: multiple same-type filename/path filters (`@a @b`, `/x /y`) now AND together like the other operators and the documented precedence, instead of OR-ing
- *(core)* search: combining a section filter with a section exclusion (`<work -<draft`) no longer returns zero results — breadcrumb exclusions now use a robust `NOT IN` subquery instead of an unreliable in-MATCH negation

## [0.2.13](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.12...kimun_core-v0.2.13) - 2026-05-29

### Fixed

- *(core)* ExclusionZones contains* guards out-of-range offsets

### Other

- *(autocomplete)* cache exclusion zones per buffer revision

## [0.2.12](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.11...kimun_core-v0.2.12) - 2026-05-26

### Fixed

- *(indexing)* suppress hashtag extraction inside images + wikilink display text
- added benches, ignore hashtags with double ##

### Other

- cargo fmt
- *(indexing)* single-walk get_chunks_and_links — 39-57% faster

## [0.2.11](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.10...kimun_core-v0.2.11) - 2026-05-25

### Added

- *(autocomplete)* trigger detection + core exclusion-zone helper
- *(core)* add prefix-search APIs for note/tag autocomplete

### Fixed

- *(autocomplete)* batch 3 — popup hardening + wikilink in code

### Other

- cargo fmt --all
- exclusions now use the prefix - before the operators in search

## [0.2.10](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.9...kimun_core-v0.2.10) - 2026-05-25

### Added

- *(cli)* add `kimun labels` command to list vault labels with note counts
- *(core/db)* label-join in search SQL with INTERSECT semantics
- *(core/search)* parse #label and lb:label query syntax
- *(core)* NoteVault::list_labels and NoteVault::notes_with_label
- *(core/db)* persist hashtag labels via NoteBatch and delete on note removal
- *(core/note)* exclude code spans from hashtag extraction
- *(core/note)* add code_char_ranges helper for hashtag exclusion
- *(core/db)* add labels table and bump schema to 0.6

### Fixed

- *(cli/metadata)* delegate hashtag extraction to core for parity with index
- *(core/note)* skip hashtag extraction inside wikilinks + document all exclusion zones
- *(core)* symmetric word-boundary check + sync spec/MCP docs with extraction rules
- *(core)* Unicode-aware label word boundary + cover bare-label-prefix test gap
- *(core/search)* drop empty terms in from_query_string to prevent FTS4 empty-phrase error
- *(core/db)* sanitize FTS4 metacharacters in user-supplied search terms
- *(core)* CRLF frontmatter detection + ESCAPE on filename LIKE search
- *(core)* extend LIKE escape to all path queries, skip frontmatter hashtags, cap query input
- *(core/search)* dedupe labels and excluded_labels in from_query_string
- *(tui/editor)* delegate label detection to core, skip labels inside links/code/fences, widen elem_index to u16
- *(core/db)* escape LIKE patterns, drop redundant index, narrow ON CONFLICT, refactor NOTE_COLUMNS
- *(core/note)* skip hashtag extraction inside links, HTML, and after label chars
- *(core/db)* keep labels in sync on rename/delete operations
- *(core/db)* use slice::from_ref for clippy::cloned_ref_to_slice_refs

### Other

- cargo fmt --all

## [0.2.9](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.8...kimun_core-v0.2.9) - 2026-05-18

### Other

- update Cargo.toml dependencies

## [0.2.8](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.7...kimun_core-v0.2.8) - 2026-05-09

### Added

- paste images

### Other

- better url detection

## [0.2.7](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.6...kimun_core-v0.2.7) - 2026-05-05

### Added

- *(core)* NoteVault::new accepts VaultConfig
- *(core)* introduce VaultConfig builder
- *(core)* add validate_filename for workspace-name checks

### Fixed

- better chunk separator and other refactors

### Other

- *(core)* update README for VaultConfig API
- cargo fmt + clippy fix in app.rs vault construction
- *(core)* VaultDB::new takes an explicit db_path
- *(core)* extract nfs filename rules to shared module
- db version bump
- format

## [0.2.6](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.5...kimun_core-v0.2.6) - 2026-04-11

### Other

- cleanup release flow

## [0.2.5](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.4...kimun_core-v0.2.5) - 2026-04-11

### Added

- *(core)* add quick_note method to NoteVault

### Fixed

- efficiency

### Other

- cleanup and hide pub functions
- cleanup and hide pub functions

## [0.2.4](https://github.com/nico2sh/kimun/compare/kimun_core-v0.2.3...kimun_core-v0.2.4) - 2026-04-09

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

- refactor dialogs
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
