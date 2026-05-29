# Plan — `ParseState` enum (deepen the editor parse cache)

Status: ready to implement. Scope: `tui/src/components/text_editor/view.rs` only (+ a CONTEXT.md term already added).

## Why

`MarkdownEditorView` encodes one state machine in two coupled fields plus a bool guard:

- `parsed_buffer: ParsedBuffer` — real *or* placeholder, indistinguishable by type (view.rs:56)
- `placeholder_active: bool` — "current buffer is a placeholder; splicing forbidden" (view.rs:104)
- `pending_full_parse: Option<u64>` — "a background parse spawn is still owed" (view.rs:98)

The bool is honored by hand at the splice gate (view.rs:219). The doc comment (view.rs:99-104) warns that forgetting it accepts a **wrong splice** — the placeholder's all-`Plain` line kinds defeat the structural guards. That is a live, representable correctness hazard.

The two flags have *different lifetimes* (spawn owed vs. splice forbidden), with a real window where `placeholder_active && pending.is_none()` (task in flight) — so they are not redundant, but a single enum subsumes both plus the take-once trick.

Domain term recorded: **Placeholder parse** / **Real parse** in `CONTEXT.md`.

## Target type

```rust
enum ParseState {
    Real(ParsedBuffer),
    Placeholder { buf: ParsedBuffer, gen: u64, spawned: bool },
}

impl ParseState {
    // State-agnostic reads (render + Gate 2 use the buffer in both states;
    // placeholder has valid row counts). Replaces ~47 `self.parsed_buffer.X`.
    fn buf(&self) -> &ParsedBuffer {
        match self { Self::Real(b) | Self::Placeholder { buf: b, .. } => b }
    }
    fn buf_mut(&mut self) -> &mut ParsedBuffer {
        match self { Self::Real(b) | Self::Placeholder { buf: b, .. } => b }
    }
    fn is_placeholder(&self) -> bool { matches!(self, Self::Placeholder { .. }) }

    /// Splice is only meaningful on a Real parse. Called only after the
    /// `is_placeholder()` gate in Gate 1 has declined the incremental path
    /// for placeholders, so the Placeholder arm is unreachable.
    fn splice_real(&mut self, range: Range<usize>, slice: ParsedBuffer) {
        match self {
            Self::Real(b) => b.splice(range, slice),
            Self::Placeholder { .. } => {
                debug_assert!(false, "splice on placeholder parse");
            }
        }
    }
}
```

Note on the guarantee: the borrow checker forces a runtime re-match in `splice_real` (we can't hold `&mut parse_state` across the `&self` call to `try_incremental_parse`). The compile-time win is that **the bare `ParsedBuffer::splice` is no longer reachable from Gate 1** — splicing now goes only through `ParseState::splice_real`, whose Placeholder arm is `debug_assert!(false)`. The hazard moves from "remember the bool" to "one guarded method"; the `unreachable` documents the invariant.

## Edits in `view.rs`

1. **Struct (view.rs:56, 98, 104)** — delete `parsed_buffer`, `pending_full_parse`, `placeholder_active`; add `parse_state: ParseState`. Move the placeholder doc comment onto the enum.

2. **`new()` (view.rs:145, 156-157)** — `parse_state: ParseState::Real(ParsedBuffer::placeholder(&[]))` (preserves today's `placeholder_active: false` initial state — an empty buffer, spliceable). Drop the two flag inits.

3. **Gate 1 (view.rs:218-254)** — replace the `if self.placeholder_active` guard and the splice/full branches:
   ```rust
   let incremental = if self.parse_state.is_placeholder() {
       None
   } else {
       self.try_incremental_parse(lines, cursor)
   };
   self.last_text_change = match incremental {
       Some((range, slice, path)) => {
           self.parse_state.splice_real(range.clone(), slice);
           self.last_parse_was_incremental = true;
           self.last_splice_path = Some(path);
           TextChangeKind::Incremental(range)
       }
       None => {
           if lines.len() >= Self::LARGE_BUFFER_THRESHOLD {
               self.parse_state = ParseState::Placeholder {
                   buf: ParsedBuffer::placeholder(lines),
                   gen: generation,
                   spawned: false,
               };
           } else {
               self.parse_state = ParseState::Real(ParsedBuffer::parse(lines));
           }
           self.last_parse_was_incremental = false;
           self.last_splice_path = None;
           TextChangeKind::Full
       }
   };
   ```

4. **`take_pending_full_parse` (view.rs:175-177)**:
   ```rust
   pub fn take_pending_full_parse(&mut self) -> Option<u64> {
       if let ParseState::Placeholder { gen, spawned, .. } = &mut self.parse_state {
           if !*spawned { *spawned = true; return Some(*gen); }
       }
       None
   }
   ```

5. **`install_full_parse` (view.rs:184-197)** — stale-guard unchanged (still on `last_seen_generation`):
   ```rust
   if generation != self.last_seen_generation { return; }
   self.parse_state = ParseState::Real(buf);
   self.fence_ranges = fence_ranges_from_kinds(self.parse_state.buf().kinds.as_slice());
   self.last_text_change = TextChangeKind::Full;
   self.last_layout_generation = u64::MAX;
   ```

6. **~47 agnostic reads** — mechanically rewrite `self.parsed_buffer.{kinds,lines,lazy_depth,reset_boundaries}` → `self.parse_state.buf().…` (view.rs:191, 226→via splice_real, 259-336, 410, 430, 530, 537, 549, 609, 675, 708-709, 714, 747-753, 777, …). The verifier block (255-284) reads via `buf()` too.

## Tests

- **Update** the 4 assertions reading `v.placeholder_active` (view.rs:1477, 1487, 1496 + the `!`-form) → `v.parse_state.is_placeholder()`. `take_pending_full_parse` assertions (1478, 1489, 1529, 1553) unchanged.
- **Add**: `splice_real` on a `Placeholder` debug-asserts (covers the now-unrepresentable wrong-splice).
- **Add**: placeholder → install_full_parse → Real transition leaves `is_placeholder() == false` and a spliceable buffer.
- Existing `edit_while_placeholder_active_refuses_incremental_and_rearms` (view.rs:1467) already covers the re-arm path; it should pass unchanged after the rename.

## Verification

`cargo test -p kimun` (text_editor suite). Optionally run with `KIMUN_VIEW_VERIFY_INCREMENTAL=1` to exercise the splice verifier against the new match.

---

## Follow-up A — shrink the interface (small, independent)

- **A1**: drop `pub` on `last_parse_was_incremental` (view.rs:81) and `last_splice_path` (view.rs:87). All readers are tests inside `view.rs`; `pub` is the only leak. No `UpdateReport` — there is no external consumer. (The previewed returned-report would churn ~13 in-module tests and break the sticky-across-cursor-only semantics those tests rely on.)
- **A2**: fold `visual_scroll_offset` (read by mod.rs:1716, 1723) into a `click_at_screen(screen_row, vcol)` method that adds the offset internally; make the field private. Locality: screen→logical mapping stops leaking the scroll offset to mod.rs.
- **Deferred**: collapse the redundant `last_parse_was_incremental` (== `last_splice_path.is_some()` at every write site) into one field — 13 test edits for marginal gain.

## Deferred C — proof-typed widen range

`WidenResult::Widened(ResetBoundedRange)` minted only by `expand_to_reset_boundary`. **Not worth it yet**: one minting site, zero external callers (hypothetical seam), and it closes only the "caller passed a bad range" hole — it does **not** retire the debug verifier (view.rs:255-284), which guards derivation/splice/parse_range bugs *inside* the machinery the type trusts. Revisit if a second range-minting caller appears. Candidate for an ADR if it keeps getting re-suggested.
