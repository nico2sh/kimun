# Text Editor Context

The TUI's editing surface. Runs on one of two interchangeable backends behind a
single enum seam, and renders Markdown live. This file fixes the vocabulary for
the seams between the backend, the snapshot, and the render layer.

## Language

### Backends

**Backend** (`BackendState`):
The editing engine driving the buffer — either the in-process `TextArea` or an
external `nvim --embed` process. An enum with inherent op-methods, not a trait
(see ADR-0009).
_Avoid_: driver, engine, provider, adapter (the enum is not a trait seam).

**Snapshot** (`NvimSnapshot` / `EditorSnapshot`):
A cheap, read-only view of backend state at one instant — lines, cursor, mode,
selection, revision. The render layer consumes a snapshot; it never reaches into
the backend directly.
_Avoid_: state, view-model, buffer-copy.

### The decode seam

**Decode seam** (`nvim_decode`):
The single place that turns nvim's wire format into a `DecodedState`. Owns every
wire fact: positional array shape, byte-offset → char-index conversion, 1-indexed
→ 0-indexed shifts, mode-string → `EditorMode`, and `getpos('v')` visual-mark
math. Pure — testable with literal values, no nvim process.
_Avoid_: parse, deserialize, marshal.

**DecodedState**:
The decoded result of one state query — either `Command { cmdline }` or
`Content { mode, lines, cursor, visual_selection }`. Mirrors the two shapes the
Lua query can return. Distinct from the live snapshot: it carries no revision
bookkeeping.
_Avoid_: nvim-state, raw-state.

**Apply** (`apply_lua_state`):
Merging a `DecodedState` into the live snapshot. Owns the stateful bookkeeping
decode cannot: the `in_flight` gate and the `content_gen`/`dirty` counters.
_Avoid_: update, sync, commit.

### Host glue

**NvimHost** (`nvim_host`):
The host-side policy that sits between the `NvimBackend` and the app — the
pending-`Z` intercept and the per-frame sync. Owns the one piece of host-side
nvim state (the pending-`Z` flag). The thin stateful shell over
`classify_nvim_key`.
_Avoid_: nvim-controller, nvim-manager, bridge.

**classify_nvim_key**:
The pure decision at the heart of `NvimHost`: given pending-Z, key, mode and
command line, returns an `NvimKeyDecision` (`BufferZ` / `Quit{save,esc_nvim}` /
`ReplayZThenForward` / `Forward`). Testable with no nvim process — same shape as
the [decode seam](#the-decode-seam).
_Avoid_: handle-key, dispatch, route.

### Revisions

**content_gen**:
A counter on the snapshot bumped only when buffer *lines* change (not on cursor
moves). The render layer mirrors it to gate parse-cache rebuilds, so a pure
cursor move reuses the cached parse.
_Avoid_: version, generation-id, dirty-counter.

### Render coupling

**TAB_STOP**:
Visual columns per tab stop. The single source of truth (`markdown/mod.rs`) from
which the nvim backend sets nvim's `tabstop`, so the renderer's tab math and
nvim's column math can never diverge.
_Avoid_: tab-width, indent-size.
