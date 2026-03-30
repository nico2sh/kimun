---
name: Serde derives belong in kimun_core
description: When adding serde serialization to core data types, add derives to kimun_core structs directly, not in the tui crate
type: feedback
---

When implementing serialization for core data structures (`NoteEntryData`, `NoteContentData`, `NoteLink`, `LinkType`, `DirectoryDetails`, etc.), add `#[derive(Serialize)]` and `#[serde(...)]` attributes directly to the structs in `kimun_core`, not by creating wrapper/mirror structs in the `tui` crate.

**Why:** Keeps serialization logic co-located with the data types. Avoids duplication. The `tui` crate can still define thin composite structs for responses that don't map 1:1 to a single core type.

**How to apply:** Any time serde needs to be added to a type that originates in `kimun_core`, add the feature flag and derives there. Only create new structs in `tui/src/cli/output.rs` for composite views (e.g. combining note metadata + content + links + backlinks into one response object).
