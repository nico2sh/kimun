# Widener stress fixtures

Buffer shapes for collecting `KIMUN_DUMP_WIDENER_METRICS` data across
edit patterns. Open this directory as a kimun workspace, edit each
note for a minute or two, exit, capture the dump.

## Files

| Note | Lines | What it stresses |
|---|---|---|
| `long_no_blank_prose.md` | 5000 | Long buffer with NO blank rows. CommonMark lazy paragraph continuation joins every row into one giant paragraph. `reset_boundaries` collapses to `[0, lines.len()]`. Edits should cap-trip the fast path and (currently) fall to `widen_to_safe`. **Watch for `full_cap_trip > 0` AND `incremental_fallback > 0`.** |
| `code_heavy.md` | 600 | Fenced rust blocks every 5 paragraphs. Edits at fence boundaries hit `looks_like_fence_marker` + `IndentedCode` guards. **Watch `full_kind_guard`.** |
| `heavy_lists_loose.md` | 571 | 500 unordered list items with blanks every 7th — classic CommonMark loose list. All items are lazy-continuable. **Watch `full_lazy_depth` dominate.** |
| `blockquotes_lazy.md` | 400 | 100 blockquotes, each with §5.1 paragraph lazy continuation onto non-`>` rows. Edits to the continuation rows exercise the `lazy_depth[R-1]` neighbour check. |
| `indented_code_multichunk.md` | 400 | §4.4 multi-chunk indented code (4-space blocks separated by blanks). The blank-row-inside-indented-code case the v1 boundary detector missed. |
| `mixed_realistic.md` | ~60 | Headings, lists, code, quotes, wikilinks, emphasis — what real notes look like. Realistic baseline. |
| `heterogeneous_lazy_dense.md` | 200 | Round-robin blockquote / indented / list / plain. Dense lazy constructs. |
| `short_simple.md` | 7 | Edge case: tiny note. Mostly first-parse + line-count-change events. |

## How to collect

```bash
KIMUN_DUMP_WIDENER_METRICS=1 cargo run --bin kimun -- --workspace example/work/widener-stress
```

(or whichever flag your kimun build uses to point at a workspace path).

For each note:
1. Open it.
2. Edit for ~60 seconds — type, delete, paste, navigate.
3. Move to the next note or close.

When you exit kimun cleanly, the dump prints to stderr. Save each
session's output:

```bash
KIMUN_DUMP_WIDENER_METRICS=1 cargo run --bin kimun 2> /tmp/session_$(date +%s).log
```

## What to look for

Compare across sessions:

- `fast_path_share` stays at 100% across all shapes → `widen_to_safe` is genuinely dead code → **Option A is safe**.
- `full_cap_trip > 0` on `long_no_blank_prose.md` → fallback still earns its keep on that shape.
- `incremental_fallback > 0` on `long_no_blank_prose.md` → fallback widener succeeded on a real edit → can't remove.
- `full_verify_failed > 0` ever → verify caught a real divergence → can't demote to debug-only.
- `full_lazy_depth` percentage stays high on `heavy_lists_loose.md` + `blockquotes_lazy.md` → confirms the lazy_depth guard is the dominant rejector for structured prose → tighten `reset_boundaries` to encode "adjacent to lazy" instead of a call-site guard.

## Refresh

`long_no_blank_prose.md` is generated:

```bash
(for i in $(seq 1 5000); do echo "Paragraph $i ..."; done) > long_no_blank_prose.md
```

Re-generate from `example/work/widener-stress/` if you want a different
size or content.