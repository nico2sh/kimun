# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0](https://github.com/nico2sh/kimun/compare/kimun_server-v0.3.0...kimun_server-v0.4.0) - 2026-07-20

### Added

- *(server)* thread conversation history through /api/answer
- *(server)* thread conversation history through the LLM client
- *(server)* add citation numbering to RAG prompt

### Fixed

- fmt
- *(server)* anchor terse follow-ups to the assistant's last message
- *(ask)* resolve 7 holistic-review findings in the Ask workspace
- *(ask)* condition RAG retrieval on conversation for terse follow-ups
- *(ask)* correct Ask conversation, provenance, reader wrap, and empty-title prompt

### Other

- Merge pull request #172 from nico2sh/ask-rag
- Merge branch 'worktree-arch-strong-batch' into ask-rag
- *(ask)* make citation↔source pairing an explicit ordinal contract
- *(ask)* cleanup batch — dedupe, derive, test gaps

## [0.3.0](https://github.com/nico2sh/kimun/compare/kimun_server-v0.2.0...kimun_server-v0.3.0) - 2026-07-16

### Added

- dynamic top_k with reranked results
- score configurable
- test queries show truncated lists
- restart server
- improved gap detection
- dynamic context cuts
- embedded models from http

### Fixed

- addressing infinite/non-infinite normalized scores
- UI improvements
- sanitize scores on rerank
- small correctness

### Other

- fixed docs
- rerank trait manages the cutoff
- tuned the cutoff algorithms
- merge locally
- embedding api examples

## [0.2.0](https://github.com/nico2sh/kimun/compare/kimun_server-v0.1.0...kimun_server-v0.2.0) - 2026-07-15

### Added

- server logs in the ui

## [0.1.0](https://github.com/nico2sh/kimun/releases/tag/kimun_server-v0.1.0) - 2026-07-15

### Added

- query time report
- sqlite vector store
- config default for server
- local llm models config suppor
- optional embedding

### Fixed

- bug hunt
- description of embedding models
- small issue sin the web ui
- style on the server backend

### Other

- hardening the server
- server documentation
- embed models sort
- renamed server config file
- clippy
- updated context
