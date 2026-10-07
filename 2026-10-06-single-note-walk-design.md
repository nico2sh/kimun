# One walk over the unrewritten note — design

Status: approved design, not yet planned.
Follows: the OUTLINE fixes (commits `8d01730f`, `0eff87fc`, `2b68e08c`),
which made every heading API share one heading walk but left that walk
parsing a *rewritten* copy of the note.

## Goal

Every whole-note extractor in core — chunks, chunks+links, title, headings,
tags, links and the rendered markdown — reads the note through **one**
pulldown-cmark pass over the note **as written**, with wikilinks and
hashtags rendered inside that pass instead of by regex rewrites before it.

Why:

- **Structure.** Today the text is rewritten (wikilinks collapsed, hashtag
  `#`s stripped) *before* it is parsed, and a rewrite can change the block
  structure: `#1. Intro` over `---` is a setext H2 as written, but after the
  `#` is stripped it parses as an ordered list item plus a thematic break,
  and the heading vanishes from the OUTLINE, the CLI headers and the chunk
  breadcrumbs.
- **Offsets.** Positions taken from rewritten text are not positions in the
  note. Heading rows needed a remapping (`DroppedBreaks`) for wikilinks that
  span lines; with the note parsed as written, offsets are exact.
- **Consistency by construction.** Today there are several walks and
  several rewrites that agree only by care: the plain chunker renders
  `[[a|#b]]` as `b`, the indexing walk as `#b`; a wikilink inside a code
  span is a link for MCP and the LINKS drawer (regex) but text for the
  heading walk. One walk makes those disagreements impossible.
- **Cost.** The plain paths parse twice (the hashtag clean-up runs its own
  code-range parse); `NoteMetadata::of` walks once for tags and again for
  headings.

## Scope

In:

| API | Today | After |
|---|---|---|
| `get_content_chunks` / `NoteDetails::get_content_chunks` | regex pre-pass + parse | view of the walk |
| `get_chunks_and_links` (indexing) | wikilink-collapse regex + `walk_indexing_events` | view of the walk |
| `extract_title` | parse of the raw body, no link/tag rendering | view of the walk |
| `note_headings` / `extract_outline` | collapse + tracked rows + parse | view of the walk |
| `heading_display_text(line)` | per-line collapse + parse | the walk over that one line |
| `heading_section_range` | via `heading_display_text` | unchanged (follows it) |
| `get_markdown_and_links` (and `NoteVault::get_markdown_and_links`) | wikilink regex, md-link regex, hashtag regex + three range scans | splice at ranges the walk recorded |
| `extract_labels` / `note_tags` (inline part) | from `get_markdown_and_links` links | tags from the walk |
| CLI `extract_links` (`tui/src/cli/metadata_extractor.rs`) | `scan::link_char_spans` over the whole note | links from the walk |
| `NoteMetadata::of` | separate tag and heading passes | one walk shared by both |

Out:

- **The editor's live, per-line helpers** — `scan::wikilink_char_spans`,
  `scan::link_char_spans` as used by the editor, `ExclusionZones`,
  `is_inside_exclusion_zone`, `is_inside_code_link_or_frontmatter`,
  `label_matches` (public). They run per keystroke on one line, must react
  to half-typed syntax (`[[foo` with no closing brackets) that a markdown
  parser treats as text, and lack the block context a whole-note parse
  exists for. They already share the one rule that matters —
  `wikilink_parts` — with the walk.
- **`replace_note_links`** (rename rewrites) — a write path with its own
  tests; not an extractor.
- **The server** — no code change (see *Index and server*).

## Architecture

### `core/src/note/walk.rs`

A new crate-private module holding the single walk. `content_extractor.rs`
is over 3,000 lines and the walk is the piece everything depends on, so it
gets its own file; `TextLine`/`TextLines` move with it.

```rust
pub(crate) struct NoteWalk {
    /// Rendered lines, as `TextLines` builds them today. `TextLine::Header`
    /// carries its byte offset in the note.
    pub lines: Vec<TextLine>,
    /// Every link pulldown reports, in order.
    pub links: Vec<WalkLink>,
    /// Every hashtag in prose, in order.
    pub tags: Vec<WalkTag>,
    /// Byte of the note where the body starts (after BOM and frontmatter).
    pub body_start: usize,
    /// The frontmatter text, as `remove_frontmatter` returns it today.
    pub frontmatter: String,
}

pub(crate) struct WalkLink {
    pub kind: WalkLinkKind,        // Wiki, WikiEmbed, Inline, Reference, Autolink, Image
    pub target: String,            // wikilink target via wikilink_parts; else the destination
    pub label: String,             // the link's text as written: a wikilink's display
                                   // part, `[label]` of a markdown link, an image's alt,
                                   // an autolink's address
    pub range: Range<usize>,       // source bytes of the whole link, in the note
}

pub(crate) struct WalkTag {
    pub name: String,              // without `#`
    pub range: Range<usize>,       // source bytes of `#name`, in the note
}

pub(crate) fn walk(note: &str) -> NoteWalk;
```

All ranges and offsets are in the **note as given** (frontmatter included),
so callers never translate between coordinate systems; rows come from
counting `\n` in the note up to an offset.

### The pass

1. Find where the body starts in the note as given — after a closed
   frontmatter block (`frontmatter_end_byte`), after only the first line of
   an unclosed fence (`frontmatter_delimiter`), or after a BOM — and parse
   the slice `note[body_start..]` itself, never a re-joined copy, so
   offsets are exact and CRLF survives to be normalized per text event.
   Record `body_start` and `frontmatter` (as `remove_frontmatter` returns
   it today).
2. Compute hashtag candidates with `label_matches_inner` over the body —
   the regex and its word-boundary/`##` rules are unchanged.
3. Run one `Parser::new_ext(body, Options::ENABLE_WIKILINKS)
   .into_offset_iter()` and feed every event to `TextLines`, with these
   additions:
   - **Wikilink** (`Tag::Link`/`Tag::Image` with `LinkType::WikiLink`):
     read the link's source span, strip `[[`/`![[` and `]]`, apply
     `wikilink_parts` for target and display text, append the display text
     to the current line, and ignore pulldown's own events up to the
     matching `End`. Record a `WalkLink` (`Wiki` or `WikiEmbed`).
   - **Other links and images**: record a `WalkLink` with pulldown's
     destination; their text events flow into the line as today.
   - **Hashtags**: a candidate is a tag when its range lies inside a prose
     `Text` event outside code blocks, link text and image text. Its `#` is
     dropped from the line text and a `WalkTag` is recorded. Inside link or
     image text it stays verbatim and is not recorded. When a text event's
     source and decoded text differ in length (HTML entities), its tags are
     recorded but the line keeps the `#` — today's rule.
4. `lines.finish()`.

### Views

| View | Built from |
|---|---|
| chunks | `chunks_from_text_lines(lines)` + the `FrontMatter` chunk, as today |
| chunks + links | chunks; `NoteLink::note` for `Wiki`/`WikiEmbed` links whose target `VaultPath::is_valid`; note links from other kinds via `emit_md_note_link` (as the indexing walk does today); `NoteLink::hashtag` per tag |
| title | today's `extract_title` rule over `lines` |
| headings | `Header` lines (non-empty text) → `NoteHeading { level, text, line }`, `line` = row of its offset |
| `heading_display_text(line)` | strip block markers (as today), `walk(line)`, first `Header` |
| rendered markdown | copy the note, splicing at recorded ranges: wikilinks → `[text](note path)` when valid, else as written (`WikiEmbed` gets a leading `!`); inline links → destination resolved as today; tags → `[#name](#name)`; images and everything else verbatim (images are resolved afterwards by `process_image_links`, as today) |
| links for the LINKS drawer / MCP | from the same `WalkLink`s, with today's classification (url / note / vault path); `Image` links are not returned here — image handling stays in `NoteVault::get_markdown_and_links` → `process_image_links`, unchanged |
| tags | `tags` names (plus frontmatter tags, as `note_tags` does today) |

### Removed

`loop_events`, `walk_indexing_events` (merged into the walk),
`collapse_wikilinks_with_display_ranges`, `collapse_inline_links` and
`collapse_inline_links_tracking_lines`, `process_wikilinks_tracking_lines`,
`DroppedBreaks`, `cleanup_hashtags` / `cleanup_hashtags_with_ranges`,
`extract_outline` (folded into the headings view), and the
`process_wikilinks` / `MD_LINK_RX` rewrites and
`code_char_ranges` / `md_link_char_ranges` / `md_wikilink_char_ranges`
scans **on these paths**. Anything the editor's `ExclusionZones` still
calls stays. All removed items are crate-private: deleted, not deprecated.

## Rendering rules

1. **Wikilink** `[[…]]` — found by pulldown, so never inside code. Target
   and display from `wikilink_parts` on the source (`[[a|b|c]]` shows `b`).
   A note link when `VaultPath::is_valid(target)`. `![[x]]` behaves as
   `[[x]]`: shows its display text, records the same link.
2. **Other links** (inline, reference, autolink) — all recorded, as the
   index already does. The markdown rewrite changes only *inline* links'
   destinations; reference links and autolinks are left as written for the
   renderer.
3. **Hashtags** — as in *The pass*, step 3.
4. **Everything else** — emphasis, inline HTML, `<br>`, list items,
   headings in lists and quotes, setext headings — keeps today's
   `TextLines` rendering. A setext heading whose text spans several lines is
   still named by its first line.

## Behaviour changes

Intended; each gets a focused test, and the snapshot diff (see *Testing*)
must contain exactly these and nothing else.

| Case | Today | After |
|---|---|---|
| `#1. Intro` over `---` | heading lost (OUTLINE, CLI, breadcrumbs) | H2 "1. Intro" everywhere |
| `` `[[x]]` `` in a code span or code block | collapsed to `` `x` ``; a link for MCP, LINKS and the index | stays code; not a link anywhere |
| `[x](y.md)` in a code span or code block | rewritten and recorded by `get_markdown_and_links` | left alone, not recorded |
| `[[a\|#b]]` | plain chunks "b", index "#b" | "#b" everywhere; not a tag |
| `[[#tag]]` | plain chunks "tag" | "#tag"; not a tag, not a link |
| `[[\|b]]`, `[[]]` | "b" / "" | literal text, not links |
| reference links, autolinks (not email autolinks, which stay unrecorded) | indexed, missing from LINKS, MCP, tags | recorded everywhere |
| inline link with a `<…>` destination or a title, `[t](<a b.md>)`, `[t](x.md "T")` | left as written, not listed | still left as written in the rendered markdown, now listed as links |
| a wikilink inside an HTML block (a block-level tag such as `<div>`/`<details>` starting a line, or any tag alone on its line), e.g. `<details>…See [[hidden]]…</details>` | collapsed in chunk text, linked everywhere | not a link anywhere: HTML is not markdown (as for hashtags); the HTML text is kept as written. Formatting tags inside a line (`see <b>[[note]]</b>`) are inline HTML and do not affect the link |
| markup or entities in a wikilink alias, `[[a\|*Emph* name]]`, `[[c\|a &amp; b]]` | rendered (`Emph name`, `a & b`) | shown as written (`*Emph* name`, `a &amp; b`) — the alias is source text |
| a hashtag touching a wikilink, `[[c]]#t2`, `#t3[[d]]` | index missed `t2`, recorded bogus `t3d` | `t2` and `t3`, as the other extractors already had |
| a wikilink inside image alt text, `![x [[a]] #t](p.png)` | an image; `#t` not a tag | per CommonMark the image degrades to text; `#t` is a tag (rendered markdown unchanged) |
| a URL fragment in an autolink, `<https://x.com/#frag>` | `frag` a tag in `extract_labels`/LINKS | not a tag (link text) |
| a hashtag pulldown splits at a flanking `_`, `#my_tag_`, `#tag_`, `#_x` | never indexed; but a tag in `note_tags`, LINKS and the rendered markdown | not a tag anywhere — the other extractors now agree with the index |
| an escaped wikilink, `\[[a]]` | a link (index, LINKS); rendered `\[a](a.md)` | not a link anywhere; text `[[a]]` — the backslash escapes it, as CommonMark reads it |
| markup inside a plain wikilink, `[[a *b* c]]` | chunk text `a b c` | `a *b* c` — the wikilink's text is shown as written, like an alias |
| a markdown link inside an HTML block, `<details>[doc](doc.md)</details>` | listed by LINKS/MCP/CLI, rewritten; not indexed | not a link anywhere, left as written |
| a markdown link with unencoded spaces in its destination, `[David H](../Work/People/David H.md)` | listed by LINKS/MCP/CLI and followed by the editor; not indexed | a link everywhere (index, LINKS, MCP, CLI), found with the same pattern the editor highlights; rendered as `[David H](<…/David H.md>)` so previewers show it; chunk text unchanged; one whose destination contains a code span (``[t](a `b c`.md)``) is not a link |
| a wikilink to a name with spaces, `[[DEM Platform]]` (rendered markdown) | `[DEM Platform](dem platform.md)` — not a link to a CommonMark previewer | `[DEM Platform](<dem platform.md>)` — a real link |
| a wikilink inside inline HTML (a comment `<!-- [[x]] -->`, an attribute `<a title="[[x]]">`) or as a reference definition's destination `[r]: [[x]]` | indexed and listed | not a link anywhere: HTML is not markdown; a definition's destination is not a wikilink |
| a wikilink used as an inline link's destination, `[t]([[a]])` | a link to `a` (regex collapsed it first) | not a link: per CommonMark the destination is literal text |
| an escaped hashtag, `\#esc` (plain chunk text) | `\esc` | `esc` (still a tag, as before) |
| a `#` right after `&`, as in an HTML entity pasted from the web, `it&#39;s` | tag `39` (and highlighted in the editor) | not a tag anywhere, editor included |
| an escaped spaced link, `\[x](a b.md)` | listed by LINKS/MCP/CLI | not a link anywhere (the backslash escapes it) |
| a hashtag inside a spaced-destination link's label or image alt, `[Dave #x](d e.md)`, `![a #t](p q.png)` | chunk text keeps `#x`; not a tag | not a tag; chunk text `x` (as for other link text handled by the walk) |
| a heading ending in inline HTML, `# a <kbd>` | heading `a ` (trailing space) | `a` — trimmed like any heading |
| a note opening with an HTML block, `<x>` (title) | `<x>\n` | `<x>` — trimmed like any title |
| an image embed, `![[pic.png]]`, `![[sub/pic.png]]` | rendered `![pic.png](pic.png.md)` — a broken image | rendered `![pic.png](pic.png)` / `![…](sub/pic.png)`: an embed whose target looks like an image keeps it as written for the image pipeline to resolve; any other embed (`![[v1.2]]`, `![[doc.pdf]]`) still renders as a note path |
| a wikilink to a section or block, `[[note#section]]`, `[[Plan#Goals\|goals]]`, `[[a^blk]]`, or with padding, `[[ spaced ]]` | not indexed; listed raw by the CLI | a link everywhere: the target is the note with the `#…`/`^…` part and surrounding spaces stripped (as the editor follows it); the rendered markdown keeps the fragment, `[goals](plan.md#Goals)`; the CLI lists the target as written |
| a wikilink whose target is still not a vault path after that (`[[#tag]]`) | listed raw by the CLI | not listed anywhere |
| an embed with a fragment, `![[pic.png#x]]`, `![[e^b]]` | left as written | renders without the fragment (`![…](pic.png)`, `![…](e.md)`); indexed like the same embed without it |
| a wikilink with an empty display part, `[[a\|]]`, `[[a#s\|]]` | an invisible `[](a.md)`, or left as written with a fragment | the rendered link text falls back to the target as written, `[a](a.md)`, `[a#s](a.md#s)` (chunk text unchanged) |
| a reference definition to an anchor, `[top]: #anchor` | `anchor` a tag; definition mangled to `[top]: [#anchor](#anchor)` | not a tag; definition left as written; its uses are links |
| link syntax the old regex misread: `[t](a(b).md)`, `[[Note (draft)]]` below the root, `a [[[tri]]] b`, `[v](d\_e.md)` | `a(b` attachment; `folder/note (draft).md`; no link; `d\_e.md` | `a(b).md`; `note (draft).md`; a link to `tri`; `d_e.md` — as CommonMark/pulldown read them |
| title of `# Sprint #42` | "Sprint #42" | "Sprint 42" |
| title of `# See [[p\|Project]]` | "See [[p\|Project]]" | "See Project" |
| title from a paragraph, e.g. `#inbox call [[bob\|Bob]]` | "#inbox call [[bob\|Bob]]" | "inbox call Bob" (a title renders like its line's chunk text) |
| embed `![[pic.png\|Picture]]` in prose | chunk text "!Picture" | "Picture" (rule 1) |
| a wikilink inside a markdown link's label, `[see [[a]]](x.md)` | both `a` and `x.md` recorded (regex collapsed the wikilink first) | only `a`: per CommonMark a link cannot contain a link, so the outer `[…](x.md)` is text |
| `[[t\n\|s]]` across lines | heading rows below it remapped | rows exact, no remapping |
| a link inside the frontmatter | rewritten and recorded by `get_markdown_and_links` (the index already skipped it) | left as written, not recorded anywhere |

Unchanged: `[[a|b]]` display and target, hashtag word boundaries and the
`##` rule, frontmatter detection (closed, unclosed, `+++`, BOM), per-heading
chunk splitting, the `FrontMatter` chunk, the editor's per-line helpers.

## Index and server

- **Local index.** `VERSION` in `core/src/index/mod.rs` goes 0.17 → 0.18,
  with a history comment in the existing style: chunks, links and tags come
  from one walk over the unrewritten note; wikilinks in code are no longer
  links; reference links are recorded everywhere. A version mismatch
  already triggers a clean reindex — no migration code.
- **Server vector store.** It diffs on the raw-text hash, which does not
  change, so existing embeddings keep their old chunk text until a note is
  edited. Differences are confined to the edge cases above. Document in the
  release notes that a manual server reindex brings everything current; no
  server code change.

## Performance

| Path | Today | After |
|---|---|---|
| plain chunks / headings / title | wikilink regex, hashtag clean-up (own code-range parse + link scans), parse — 2 parses | hashtag candidates, 1 parse |
| indexing | collapse regex, hashtag candidates, 1 parse | hashtag candidates, 1 parse (wikilinks on) |
| `get_markdown_and_links` | 2 regex rewrites, code-range parse, 3 range scans | 1 parse + splice |
| `NoteMetadata::of` | tag pass + heading pass | 1 walk |

Gate: `cargo bench -p kimun_core --bench indexing` before and after; any
regression in the `get_chunks_and_links` group blocks the merge.

Outcome (2026-10-07): the small and hashtag-heavy fixtures ended ~3–6%
slower than `before` (part already at the first walk commit, part from
recognising spaced-destination links); medium/large are unchanged or up to
~20% faster and `get_content_chunks` 5–17% faster. Accepted by the owner:
small notes are already fast, so the absolute cost is negligible.

## Testing

1. **Snapshot first** (plan step 0, before any change). A core test runs
   every extractor — chunks, chunks+links, title, headings, tags, links,
   rendered markdown — over a fixed corpus: the `example/` vault, the
   `core/benches/fixtures` notes, and a new edge-case note with one example
   per *Behaviour changes* row. It writes one plain-text snapshot,
   committed; an env var regenerates it (no new dependency). After the
   change, the diff is reviewed row by row against *Behaviour changes*;
   anything else is a regression.
2. **Consistency properties** over every corpus note:
   - each `note_headings` text equals its chunk's breadcrumb segment (for
     headings with a body);
   - `extract_title` equals the first heading's text when the note opens
     with one;
   - `heading_display_text(row h.line) == h.text` for every ATX heading on
     its own line;
   - inline tags agree across `note_tags`, `get_chunks_and_links` and
     `get_markdown_and_links`;
   - note links agree across `get_chunks_and_links` and
     `get_markdown_and_links`, except multi-segment wikilinks in a note
     below the root: the rendered markdown resolves them against the note's
     folder (as the editor follows them) while the index resolves them from
     the vault root — a pre-existing difference outside this spec.
3. **One focused test per *Behaviour changes* row**, named for the
   behaviour (e.g. `a_wikilink_in_code_is_not_a_link`).
4. **Existing tests** pass unchanged, except those asserting a row of
   *Behaviour changes*; the plan lists each such edit up front.
5. **Index**: existing schema tests cover the `VERSION` bump.
6. **Benches** as in *Performance*; the seven CI gates as usual.
7. **pulldown-cmark 0.13.4 crash guard (temporary).** 0.13.4 panics on an
   `![[` completed as an ordinary image or link before its `]]`
   (`![[]x]()]]`); upstream fixed it in commit ebf31da886 (unreleased).
   Until a release ships it, `pulldown_crash_guard` in `walk.rs` hands
   pulldown a same-length copy where such an `![[` reads `!` then `[[`;
   remove it (and its call) when pulldown-cmark is bumped, keeping
   `pulldown_0_13_4_wikilink_crash_inputs_do_not_panic` and the fuzz tests.

## Docs

User docs (`docs/`) only if a page describes how wikilinks or tags in code
or titles behave; check during the plan, change only what is now wrong.
Release notes carry the server-reindex note.
