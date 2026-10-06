# One walk over the unrewritten note — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every whole-note extractor in `kimun_core` reads the note through one pulldown-cmark pass over the note as written, with wikilinks and hashtags rendered inside that pass.

**Architecture:** A new crate-private module `core/src/note/walk.rs` owns the single walk (`walk(note) -> NoteWalk`) and the line builder it feeds (`TextLine`/`TextLines`, moved there). Chunks, chunks+links, title, headings, `heading_display_text`, the markdown rewrite, tags and link targets become views over a `NoteWalk`; the regex rewrites and the second walk are deleted.

**Tech Stack:** Rust 2021, `pulldown-cmark` 0.13.4 (`Options::ENABLE_WIKILINKS`, `into_offset_iter`), `regex` (hashtag candidates only), criterion (bench gate).

**Spec:** `2026-10-06-single-note-walk-design.md` (repo root). Read it first; this plan argues from it.

## Global Constraints

- All changes stay inside `kimun_core` except `tui/src/cli/metadata_extractor.rs` (Task 4). No new dependencies.
- Public signatures unchanged: `NoteDetails` methods, `note_headings`, `note_tags`, `extract_labels`, `scan::heading_display_text`, `scan::heading_section_range`, `NoteVault::get_markdown_and_links`. One addition: `pub fn note_link_targets(text: &str) -> Vec<String>` in `core/src/note/mod.rs`.
- Every offset/range in `walk.rs` is a byte offset into the note **as given** (frontmatter and BOM included).
- Walked text is a slice of the note, never a re-joined copy; `\r\n` is normalized to `\n` per text event.
- The editor's per-line helpers (`wikilink_char_spans`, `link_char_spans`, `ExclusionZones`, `is_inside_*`, public `label_matches`) and `replace_note_links` are **not** touched.
- Index `VERSION` in `core/src/index/mod.rs` goes `"0.17"` → `"0.18"` (Task 5).
- The user commits; agents do **not** run `git commit`. Each task ends with a checkpoint report instead.
- Gates before reporting a task done (from the repo's CI): `cargo fmt --all -- --check`, `python3 .github/scripts/check-host-fs.py`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo check --profile bench --workspace --all-targets`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace`, `cargo test --workspace`. Touch sources first so clippy/doc actually re-run: `find core server tui -name '*.rs' -exec touch {} +`.
- Doc comments on `pub` items must not link to private items (the docs gate fails on it).

## Review Focus

1. **CRLF notes** — headings report the right row and no chunk, title or heading text contains `\r`. Pinned in Task 2 (`crlf_is_normalized_and_offsets_stay_exact`) and Task 3 (`headings_report_rows_in_a_crlf_note`).
2. **Degenerate notes** — empty note, BOM only, frontmatter only, unclosed fence with no newline: no panic, empty outputs. Pinned in Task 2 (`degenerate_notes_walk_to_nothing`).
3. **Multi-byte text next to tags and links** — byte slicing at recorded ranges must land on char boundaries (`café [[ü|ö]] #a`). Pinned in Task 2 (`multibyte_text_around_links_and_tags`).
4. **Nested links** — an image inside a link, brackets inside a label: label extraction and link-depth tracking (`[![a #t](i.png)](x.md)`). Pinned in Task 2 (`an_image_inside_a_link_is_link_text`).
5. **Indexing throughput** — the walk must not be slower than today's indexing walk. Pinned in Task 5 (criterion baseline compare).

---

### Task 0: Baseline snapshot and bench baseline

Freeze today's output of every extractor before anything changes, so every later diff can be reviewed line by line.

**Files:**
- Create: `core/tests/extraction_snapshot.rs`
- Create: `core/tests/fixtures/walk/edge_cases.md`
- Create: `core/tests/fixtures/walk/title_tag.md`
- Create: `core/tests/fixtures/walk/title_wikilink.md`
- Create: `core/tests/fixtures/walk/setext_hashtag.md`
- Create: `core/tests/fixtures/walk/frontmatter_links.md`
- Create (generated): `core/tests/snapshots/extraction.txt`

**Interfaces:**
- Consumes: public `kimun_core` API only (`NoteDetails`, `note::note_headings`, `note::note_tags`).
- Produces: `extraction_snapshot` test and the committed snapshot every later task diffs against; a criterion baseline named `before`.

- [ ] **Step 1: Write the edge-case fixtures**

`core/tests/fixtures/walk/edge_cases.md`:

````markdown
# Edge cases

Code span `[[x]]` and code link `[y](y.md)` stay code.

Alias tag [[a|#b]] and invalid [[#tag]] and empties [[|b]] [[]].

A reference [ref][r] and an autolink <https://e.x>.

Embed ![[pic.png|Picture]] and plain [[note|Shown]].

Broken across lines [[target
|Shown]] then text.

## After the broken link

body

[r]: other.md
````

`core/tests/fixtures/walk/title_tag.md`:

```markdown
# Sprint #42

body
```

`core/tests/fixtures/walk/title_wikilink.md`:

```markdown
# See [[proj|Project]]

body
```

`core/tests/fixtures/walk/setext_hashtag.md`:

```markdown
#1. Intro
---

body under the setext heading
```

`core/tests/fixtures/walk/frontmatter_links.md`:

```markdown
---
related: see [docs](other.md) and [[refnote]]
---
body with [[real]]
```

- [ ] **Step 2: Write the snapshot test**

`core/tests/extraction_snapshot.rs`:

```rust
//! Snapshot of every whole-note extractor over a fixed corpus: the
//! `example/` vault, the bench fixtures and `tests/fixtures/walk`. Taken
//! before the single-walk change (`2026-10-06-single-note-walk-design.md`)
//! so every behaviour change shows up as a reviewed diff.
//!
//! Regenerate with `UPDATE_SNAPSHOT=1 cargo test -p kimun_core --test extraction_snapshot`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kimun_core::nfs::VaultPath;
use kimun_core::note::{self, NoteDetails, NoteLink};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// `(name, text)` for every corpus note, sorted by name. Line endings are
/// normalized so a CRLF checkout snapshots the same.
pub fn corpus() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect(&root.join("../example"), &mut files);
    collect(&root.join("benches/fixtures"), &mut files);
    collect(&root.join("tests/fixtures/walk"), &mut files);
    let mut notes: Vec<(String, String)> = files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string()
                .replace('\\', "/");
            (name, text)
        })
        .collect();
    notes.sort();
    notes
}

fn link_line(link: &NoteLink) -> String {
    format!("{:?} {:?} {:?}", link.ltype, link.raw_link, link.text)
}

fn render(name: &str, text: &str) -> String {
    let path = VaultPath::new("folder/note.md");
    let details = NoteDetails::new(&path, text);
    let mut out = String::new();
    writeln!(out, "=== {name}").unwrap();
    writeln!(out, "-- title {:?}", details.get_title()).unwrap();
    writeln!(out, "-- headings").unwrap();
    for h in note::note_headings(text) {
        writeln!(out, "{} {} {:?}", h.level, h.line, h.text).unwrap();
    }
    writeln!(out, "-- chunks").unwrap();
    for c in details.get_content_chunks() {
        writeln!(out, "[{}] {:?}", c.breadcrumb, c.text).unwrap();
    }
    let (index_chunks, index_links) = details.get_chunks_and_links();
    writeln!(out, "-- index chunks").unwrap();
    for c in index_chunks {
        writeln!(out, "[{}] {:?}", c.breadcrumb, c.text).unwrap();
    }
    writeln!(out, "-- index links (sorted)").unwrap();
    let mut lines: Vec<String> = index_links.iter().map(link_line).collect();
    lines.sort();
    for l in lines {
        writeln!(out, "{l}").unwrap();
    }
    writeln!(out, "-- tags {:?}", note::note_tags(text)).unwrap();
    let (md, md_links) = details.get_markdown_and_links();
    writeln!(out, "-- markdown links").unwrap();
    for l in &md_links {
        writeln!(out, "{}", link_line(l)).unwrap();
    }
    writeln!(out, "-- markdown").unwrap();
    out.push_str(&md);
    out.push_str("\n\n");
    out
}

#[test]
fn extraction_snapshot() {
    let actual: String = corpus().iter().map(|(n, t)| render(n, t)).collect();
    let snapshot = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/extraction.txt");
    if std::env::var_os("UPDATE_SNAPSHOT").is_some() {
        std::fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        std::fs::write(&snapshot, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&snapshot)
        .expect("no snapshot yet: run with UPDATE_SNAPSHOT=1")
        .replace("\r\n", "\n");
    if expected != actual {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("extraction.actual.txt");
        std::fs::write(&out, &actual).unwrap();
        panic!(
            "extraction output changed — review: diff {} {}",
            snapshot.display(),
            out.display()
        );
    }
}
```

- [ ] **Step 3: Generate the baseline snapshot**

Run: `UPDATE_SNAPSHOT=1 cargo test -p kimun_core --test extraction_snapshot`
Expected: PASS, and `core/tests/snapshots/extraction.txt` exists.

- [ ] **Step 4: Confirm it is stable**

Run: `cargo test -p kimun_core --test extraction_snapshot`
Expected: PASS (no `UPDATE_SNAPSHOT`).

Open `core/tests/snapshots/extraction.txt` and confirm the edge-case notes show today's behaviour — e.g. `setext_hashtag.md` lists **no** heading under `-- headings`, `title_tag.md` has title `"Sprint #42"`. These are the "Today" column of the spec's *Behaviour changes* table.

- [ ] **Step 5: Save the bench baseline**

Run: `cargo bench -p kimun_core --bench indexing -- --save-baseline before`
Expected: criterion completes and prints timings for `get_chunks_and_links/*` and `get_content_chunks/*`.

- [ ] **Step 6: Checkpoint**

Run the gates (Global Constraints). Report to the user: snapshot generated (size in lines), bench baseline saved. Do not commit.

---

### Task 1: Move the line builder into `walk.rs`

A pure move, no behaviour change: `TextLine`, `TextLines`, `parse_tag`, `parse_tag_end`, `chunks_from_text_lines`, `join_breadcrumb` go to a new `core/src/note/walk.rs`.

**Files:**
- Create: `core/src/note/walk.rs`
- Modify: `core/src/note/mod.rs` (add `mod walk;`)
- Modify: `core/src/note/content_extractor.rs` (remove the moved items; import them)

**Interfaces:**
- Produces (all `pub(in crate::note)` in `walk.rs`): `enum TextLine { Empty, Header(u8, String, usize), Text(String), ListItem(u8, String) }` with methods `append_text`, `to_text`, `trim`; `struct TextLines<'a>` with `push(&mut self, event: Event<'a>, start: usize, text: impl FnOnce(&str) -> String)` and `finish(self) -> Vec<TextLine>`; `fn chunks_from_text_lines(lines: Vec<TextLine>) -> Vec<ContentChunk>`.

- [ ] **Step 1: Create `walk.rs` with the moved code**

Cut these items from `core/src/note/content_extractor.rs` and paste them, unchanged in body, into `core/src/note/walk.rs`:
`enum TextLine` + `impl TextLine`, `struct TextLines` + `impl TextLines`, `fn parse_tag`, `fn parse_tag_end`, `fn chunks_from_text_lines`, `fn join_breadcrumb`.

Change their visibility to `pub(in crate::note)` (enum, struct, the three methods `append_text`/`to_text`/`trim`, `push`, `finish`, `chunks_from_text_lines`). `parse_tag`, `parse_tag_end`, `join_breadcrumb` stay private to `walk.rs`.

Top of `walk.rs`:

```rust
//! The one pass over a note every whole-note extractor reads — see
//! `2026-10-06-single-note-walk-design.md`. Until the walk lands, this module
//! holds the line builder (`TextLine`/`TextLines`) the extractors share.

use log::debug;
use pulldown_cmark::{Event, Tag, TagEnd};

use super::ContentChunk;
```

Fix the doc comment on `TextLines` (it names `loop_events`/`walk_indexing_events`, which still exist in `content_extractor`): write them as plain backticked names, not intra-doc links.

- [ ] **Step 2: Wire the module**

In `core/src/note/mod.rs`, next to `pub(crate) mod content_extractor;`:

```rust
mod walk;
```

In `core/src/note/content_extractor.rs`, add to the imports:

```rust
use super::walk::{chunks_from_text_lines, TextLine, TextLines};
```

- [ ] **Step 3: Build and test**

Run: `cargo test -p kimun_core`
Expected: PASS, including `extraction_snapshot` (unchanged output). If the compiler reports an unused import (`Tag`, `TagEnd`, `debug`) in `content_extractor.rs`, remove only that import.

- [ ] **Step 4: Checkpoint**

Run the gates. Report: pure move, snapshot unchanged. Do not commit.

---

### Task 2: The walk

Add `walk()` and its types to `walk.rs`, with unit tests. Nothing calls it yet.

**Files:**
- Modify: `core/src/note/walk.rs`
- Modify: `core/src/note/content_extractor.rs` (visibility only)

**Interfaces:**
- Consumes (make these `pub(in crate::note)` in `content_extractor.rs`): `fn frontmatter_end_byte(text: &str) -> usize`, `fn remove_frontmatter<S: AsRef<str>>(text: S) -> (String, String)`, `fn wikilink_parts(inner: &str) -> (&str, &str)`. Already reachable: `frontmatter_delimiter`, `split_bom` (`pub(in crate::note)`), `label_matches_inner` (`pub(crate)`).
- Produces (all `pub(in crate::note)` in `walk.rs`):

```rust
pub(in crate::note) enum WalkLinkKind { Wiki, WikiEmbed, Inline, Reference, Autolink, Image }
pub(in crate::note) struct WalkLink { pub kind: WalkLinkKind, pub target: String, pub label: String, pub range: Range<usize> }
pub(in crate::note) struct WalkTag { pub name: String, pub range: Range<usize> }
pub(in crate::note) struct NoteWalk { pub lines: Vec<TextLine>, pub links: Vec<WalkLink>, pub tags: Vec<WalkTag>, pub body_start: usize, pub frontmatter: String }
pub(in crate::note) fn walk(note: &str) -> NoteWalk
```

- [ ] **Step 1: Write the failing tests**

Append to `core/src/note/walk.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn headers(w: &NoteWalk) -> Vec<(u8, String, usize)> {
        w.lines
            .iter()
            .filter_map(|l| match l {
                TextLine::Header(level, text, start) => Some((*level, text.clone(), *start)),
                _ => None,
            })
            .collect()
    }

    fn text(w: &NoteWalk) -> String {
        w.lines
            .iter()
            .map(TextLine::to_text)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn links(w: &NoteWalk) -> Vec<(WalkLinkKind, &str, &str)> {
        w.links
            .iter()
            .map(|l| (l.kind, l.target.as_str(), l.label.as_str()))
            .collect()
    }

    #[test]
    fn offsets_are_in_the_note_as_given() {
        let w = walk("---\nt: x\n---\n# A\n");
        assert_eq!(w.body_start, 13);
        assert_eq!(headers(&w), [(1, "A".to_string(), 13)]);
        assert_eq!(w.frontmatter, "t: x");
    }

    #[test]
    fn an_unclosed_fence_drops_only_its_first_line() {
        let w = walk("---\n# A\n");
        assert_eq!(w.body_start, 4);
        assert_eq!(headers(&w), [(1, "A".to_string(), 4)]);
        assert_eq!(w.frontmatter, "");
    }

    #[test]
    fn a_byte_order_mark_is_not_body() {
        let w = walk("\u{feff}# A\n");
        assert_eq!(w.body_start, 3);
        assert_eq!(headers(&w), [(1, "A".to_string(), 3)]);
    }

    #[test]
    fn crlf_is_normalized_and_offsets_stay_exact() {
        let note = "+++\r\na = 1\r\n+++\r\n# A\r\nline one\r\n```\r\ncode\r\n```\r\n";
        let w = walk(note);
        assert_eq!(&note[w.body_start..w.body_start + 3], "# A");
        assert_eq!(headers(&w), [(1, "A".to_string(), w.body_start)]);
        assert!(!text(&w).contains('\r'), "{:?}", text(&w));
    }

    #[test]
    fn degenerate_notes_walk_to_nothing() {
        for note in ["", "\u{feff}", "---", "---\n", "---\na: 1\n---\n", "+++\n+++"] {
            let w = walk(note);
            assert!(headers(&w).is_empty(), "{note:?}");
            assert!(w.links.is_empty() && w.tags.is_empty(), "{note:?}");
        }
    }

    #[test]
    fn a_wikilink_shows_its_display_text_and_records_its_parts() {
        let w = walk("see [[proj|Project|extra]] now");
        assert_eq!(text(&w), "see Project now");
        assert_eq!(links(&w), [(WalkLinkKind::Wiki, "proj", "Project")]);
        assert_eq!(w.links[0].range, 4..26);
    }

    #[test]
    fn a_wikilink_in_code_is_not_a_link() {
        let w = walk("`[[x]]` and\n\n```\n[[y]]\n```\n");
        assert!(w.links.is_empty());
        assert!(text(&w).contains("`[[x]]`"));
        assert!(text(&w).contains("[[y]]"));
    }

    #[test]
    fn an_embed_is_a_wikilink_showing_its_display_text() {
        let w = walk("![[pic.png|Picture]]");
        assert_eq!(links(&w), [(WalkLinkKind::WikiEmbed, "pic.png", "Picture")]);
        assert_eq!(text(&w), "Picture");
    }

    #[test]
    fn degenerate_wikilinks_are_text() {
        let w = walk("[[|b]] and [[]]");
        assert!(w.links.is_empty());
        assert_eq!(text(&w), "[[|b]] and [[]]");
    }

    #[test]
    fn an_empty_display_part_shows_nothing() {
        let w = walk("a [[t|]] b");
        assert_eq!(links(&w), [(WalkLinkKind::Wiki, "t", "")]);
        assert_eq!(text(&w), "a  b");
    }

    #[test]
    fn a_wikilink_across_lines_keeps_later_offsets_exact() {
        let note = "# A\n[[target\n|Shown]]\n# B\n";
        let w = walk(note);
        let b = headers(&w)[1].2;
        assert_eq!(&note[b..b + 3], "# B");
    }

    #[test]
    fn markdown_links_keep_their_label_as_written() {
        let note = "[**bold** [x]](a.md) [ref][r] <https://e.x> ![alt](i.png)\n\n[r]: b.md\n";
        let w = walk(note);
        assert_eq!(
            links(&w),
            [
                (WalkLinkKind::Inline, "a.md", "**bold** [x]"),
                (WalkLinkKind::Reference, "b.md", "ref"),
                (WalkLinkKind::Autolink, "https://e.x", "https://e.x"),
                (WalkLinkKind::Image, "i.png", "alt"),
            ]
        );
        assert_eq!(&note[w.links[0].range.clone()], "[**bold** [x]](a.md)");
    }

    #[test]
    fn a_hashtag_in_prose_is_a_tag_and_loses_its_hash() {
        let w = walk("a #one b");
        assert_eq!(text(&w), "a one b");
        assert_eq!(
            w.tags,
            [WalkTag {
                name: "one".to_string(),
                range: 2..6
            }]
        );
    }

    #[test]
    fn a_hashtag_in_link_text_or_code_is_not_a_tag() {
        let w = walk("[x #a](y.md) [[t|#b]] `#c`\n\n```\n#d\n```\n");
        assert!(w.tags.is_empty(), "{:?}", w.tags);
        let t = text(&w);
        for kept in ["x #a", "#b", "`#c`", "#d"] {
            assert!(t.contains(kept), "{kept} in {t:?}");
        }
    }

    #[test]
    fn a_hashtag_that_looks_like_a_list_marker_keeps_its_setext_heading() {
        let w = walk("#1. Intro\n---\n");
        assert_eq!(headers(&w), [(2, "1. Intro".to_string(), 0)]);
        assert_eq!(w.tags[0].name, "1");
    }

    #[test]
    fn multibyte_text_around_links_and_tags() {
        let note = "café [[ü|ö]] #a ñ [é](x.md)";
        let w = walk(note);
        assert_eq!(text(&w), "café ö a ñ é");
        for link in &w.links {
            let _ = &note[link.range.clone()];
        }
        assert_eq!(&note[w.tags[0].range.clone()], "#a");
    }

    #[test]
    fn an_image_inside_a_link_is_link_text() {
        let w = walk("[![a #t](i.png)](x.md) #out");
        assert_eq!(
            links(&w),
            [
                (WalkLinkKind::Inline, "x.md", "![a #t](i.png)"),
                (WalkLinkKind::Image, "i.png", "a #t"),
            ]
        );
        assert_eq!(w.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["out"]);
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p kimun_core --lib note::walk`
Expected: FAIL to compile — `walk`, `NoteWalk`, `WalkLink`, `WalkLinkKind`, `WalkTag` not found.

- [ ] **Step 3: Open up the helpers the walk needs**

In `core/src/note/content_extractor.rs` change to `pub(in crate::note)`: `fn frontmatter_end_byte`, `fn remove_frontmatter`, `fn wikilink_parts`.

- [ ] **Step 4: Implement the walk**

In `core/src/note/walk.rs`, extend the imports:

```rust
use std::ops::Range;

use log::debug;
use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

use super::content_extractor::{
    frontmatter_delimiter, frontmatter_end_byte, label_matches_inner, remove_frontmatter,
    split_bom, wikilink_parts,
};
use super::ContentChunk;
```

Add, above the moved `TextLine` code:

```rust
/// What kind of link a [`WalkLink`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::note) enum WalkLinkKind {
    /// `[[target|text]]`.
    Wiki,
    /// `![[target|text]]`.
    WikiEmbed,
    /// `[label](dest)`.
    Inline,
    /// `[label][ref]`, `[label][]`, `[label]` resolved against a definition.
    Reference,
    /// `<https://…>` or `<mail@…>`.
    Autolink,
    /// `![alt](dest)` and its reference forms.
    Image,
}

/// A link the walk found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::note) struct WalkLink {
    pub kind: WalkLinkKind,
    /// A wikilink's target per `wikilink_parts`; otherwise the destination
    /// as pulldown resolved it.
    pub target: String,
    /// The link's text as written: a wikilink's display part, the `[label]`
    /// of a markdown link or image, an autolink's address.
    pub label: String,
    /// Source bytes of the whole link, in the note.
    pub range: Range<usize>,
}

/// A hashtag in prose: its name without `#`, and the source bytes of
/// `#name` in the note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::note) struct WalkTag {
    pub name: String,
    pub range: Range<usize>,
}

/// Everything one pass over a note yields; every whole-note extractor is a
/// view of it. All offsets index the note as given.
pub(in crate::note) struct NoteWalk {
    pub lines: Vec<TextLine>,
    pub links: Vec<WalkLink>,
    pub tags: Vec<WalkTag>,
    pub body_start: usize,
    /// The frontmatter text, as `remove_frontmatter` returns it.
    pub frontmatter: String,
}

/// Byte of `note` where the body starts: after a closed frontmatter block,
/// after only the first line of an unclosed fence (an unclosed fence is not
/// frontmatter), or after a byte-order mark.
fn body_start(note: &str) -> usize {
    match frontmatter_end_byte(note) {
        0 => match frontmatter_delimiter(note) {
            Some((_, after_first_line)) => after_first_line,
            None => split_bom(note).0.len(),
        },
        end => end,
    }
}

/// The text between a link's first `[` and its matching `]`, honouring
/// nesting and backslash escapes; the rest of `src` when unmatched.
fn bracketed_label(src: &str) -> &str {
    let Some(open) = src.find('[') else {
        return src;
    };
    let mut depth = 0usize;
    let mut escaped = false;
    for (i, c) in src[open..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return &src[open + 1..open + i];
                }
            }
            _ => {}
        }
    }
    &src[open + 1..]
}

fn wikilink_kind(event: &Event) -> Option<WalkLinkKind> {
    match event {
        Event::Start(Tag::Link {
            link_type: LinkType::WikiLink { .. },
            ..
        }) => Some(WalkLinkKind::Wiki),
        Event::Start(Tag::Image {
            link_type: LinkType::WikiLink { .. },
            ..
        }) => Some(WalkLinkKind::WikiEmbed),
        _ => None,
    }
}

/// A prose text event's line text. Hashtag candidates lying wholly inside
/// it become tags and lose their `#`, unless it is link text (kept as
/// written, not tags). When the source and decoded text differ in length
/// (HTML entities) the tags are recorded but the `#` stays.
fn prose_text(
    decoded: &str,
    range: Range<usize>,
    body: &str,
    body_start: usize,
    candidates: &[(Range<usize>, &str)],
    in_link: bool,
    tags: &mut Vec<WalkTag>,
) -> String {
    let first = candidates.partition_point(|(r, _)| r.start < range.start);
    let inside: Vec<&(Range<usize>, &str)> = candidates[first..]
        .iter()
        .take_while(|(r, _)| r.end <= range.end)
        .collect();
    if inside.is_empty() || in_link {
        return decoded.replace("\r\n", "\n");
    }
    for (r, name) in &inside {
        tags.push(WalkTag {
            name: name.to_string(),
            range: body_start + r.start..body_start + r.end,
        });
    }
    let source = &body[range.clone()];
    if source.len() != decoded.len() {
        return decoded.replace("\r\n", "\n");
    }
    let mut out = String::with_capacity(source.len());
    let mut last = range.start;
    for (r, _) in &inside {
        out.push_str(&body[last..r.start]);
        out.push_str(&body[r.start + 1..r.end]);
        last = r.end;
    }
    out.push_str(&body[last..range.end]);
    out.replace("\r\n", "\n")
}

/// One pass over `note` as written — see the module docs.
pub(in crate::note) fn walk(note: &str) -> NoteWalk {
    let body_start = body_start(note);
    let body = &note[body_start..];
    let candidates: Vec<(Range<usize>, &str)> = label_matches_inner(body)
        .map(|m| (m.byte_start..m.byte_end, m.name))
        .collect();

    let mut lines = TextLines::default();
    let mut links = Vec::new();
    let mut tags = Vec::new();
    let mut code_depth = 0u32;
    // Open non-wiki links and images: text inside them is link text.
    let mut link_depth = 0u32;
    // Inside a wikilink, pulldown's own events are skipped up to its end:
    // the display text comes from `wikilink_parts` on the source.
    let mut in_wikilink = false;

    for (event, range) in Parser::new_ext(body, Options::ENABLE_WIKILINKS).into_offset_iter() {
        let start = body_start + range.start;
        if in_wikilink {
            if matches!(event, Event::End(TagEnd::Link | TagEnd::Image)) {
                in_wikilink = false;
                lines.push(event, start, str::to_string);
            }
            continue;
        }
        if let Some(kind) = wikilink_kind(&event) {
            let src = &body[range.clone()];
            let inner = src.strip_prefix('!').unwrap_or(src);
            let inner = inner
                .strip_prefix("[[")
                .and_then(|s| s.strip_suffix("]]"))
                .unwrap_or(inner);
            let (target, label) = wikilink_parts(inner);
            links.push(WalkLink {
                kind,
                target: target.to_string(),
                label: label.to_string(),
                range: start..body_start + range.end,
            });
            let display = label.to_string();
            lines.push(event, start, str::to_string);
            lines.push(Event::Text(CowStr::from(display)), start, str::to_string);
            in_wikilink = true;
            continue;
        }
        match &event {
            Event::Start(Tag::CodeBlock(_)) => code_depth += 1,
            Event::End(TagEnd::CodeBlock) => code_depth = code_depth.saturating_sub(1),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                link_depth += 1;
                let src = &body[range.clone()];
                let (kind, label) = match link_type {
                    LinkType::Autolink | LinkType::Email => {
                        (WalkLinkKind::Autolink, dest_url.to_string())
                    }
                    LinkType::Inline => (WalkLinkKind::Inline, bracketed_label(src).to_string()),
                    _ => (WalkLinkKind::Reference, bracketed_label(src).to_string()),
                };
                links.push(WalkLink {
                    kind,
                    target: dest_url.to_string(),
                    label,
                    range: start..body_start + range.end,
                });
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                link_depth += 1;
                links.push(WalkLink {
                    kind: WalkLinkKind::Image,
                    target: dest_url.to_string(),
                    label: bracketed_label(&body[range.clone()]).to_string(),
                    range: start..body_start + range.end,
                });
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                link_depth = link_depth.saturating_sub(1)
            }
            _ => {}
        }
        let in_code = code_depth > 0;
        let in_link = link_depth > 0;
        lines.push(event, start, |decoded| {
            if in_code {
                decoded.replace("\r\n", "\n")
            } else {
                prose_text(
                    decoded,
                    range.clone(),
                    body,
                    body_start,
                    &candidates,
                    in_link,
                    &mut tags,
                )
            }
        });
    }

    NoteWalk {
        lines: lines.finish(),
        links,
        tags,
        body_start,
        frontmatter: remove_frontmatter(note).0,
    }
}
```

Note on `in_link`: the `Start(Link)` event itself bumps `link_depth` before `push`, and the matching `End` drops it, so only text between them counts as link text.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p kimun_core --lib note::walk`
Expected: PASS (17 tests). If `degenerate_notes_walk_to_nothing` fails on `"+++\n+++"`, check `body_start` returns `frontmatter_end_byte`'s value for it (a closed empty TOML block) — fix in `body_start`, not the test.

Run: `cargo test -p kimun_core`
Expected: PASS, `extraction_snapshot` unchanged (nothing calls `walk` yet). `cargo clippy` will flag `walk` and its types as dead code until Task 3: add `#[allow(dead_code)]` on `walk`, `NoteWalk`, `WalkLink`, `WalkTag`, `WalkLinkKind` **with a comment `// wired in Task 3`**, and remove those attributes in Task 3.

- [ ] **Step 6: Checkpoint**

Run the gates. Report: walk added, 17 unit tests, snapshot unchanged. Do not commit.

---

### Task 3: Chunks, title, headings and `heading_display_text` read the walk

**Files:**
- Modify: `core/src/note/walk.rs` (views: `chunks`, `title`, `headings`, `index_links`; resolver helpers)
- Modify: `core/src/note/content_extractor.rs` (rewire, delete the old walk and rewrites)
- Modify: `core/src/note/mod.rs` (`note_headings`)
- Modify: `core/tests/snapshots/extraction.txt` (regenerated after review)

**Interfaces:**
- Consumes: `walk`, `NoteWalk`, `WalkLink`, `WalkLinkKind`, `WalkTag` (Task 2); `chunks_from_text_lines` (Task 1).
- Produces (methods on `NoteWalk`, `pub(in crate::note)`):
  - `fn chunks(&self) -> Vec<ContentChunk>`
  - `fn title(&self) -> String`
  - `fn headings(&self, note: &str) -> Vec<NoteHeading>`
  - `fn index_links(&self, ref_path: &VaultPath) -> Vec<NoteLink>`
  - free fn `pub(in crate::note) fn resolve_md_link(dest: &str, label: &str, ref_path: &VaultPath) -> (String, Option<NoteLink>)` — used again in Task 4.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `core/src/note/walk.rs`:

```rust
    #[test]
    fn headings_report_rows_in_a_crlf_note() {
        let note = "---\r\nt: x\r\n---\r\n# A\r\nbody\r\n\r\n## B\r\n";
        let rows: Vec<(String, usize)> = walk(note)
            .headings(note)
            .into_iter()
            .map(|h| (h.text, h.line))
            .collect();
        assert_eq!(rows, [("A".to_string(), 3), ("B".to_string(), 6)]);
    }

    #[test]
    fn headings_rows_survive_a_wikilink_across_lines() {
        let note = "# A\n[[target\n|Shown]]\n# B\n";
        let rows: Vec<usize> = walk(note).headings(note).iter().map(|h| h.line).collect();
        assert_eq!(rows, [0, 3]);
    }

    #[test]
    fn the_title_renders_like_the_heading() {
        assert_eq!(walk("# Sprint #42\nbody").title(), "Sprint 42");
        assert_eq!(walk("# See [[proj|Project]]\n").title(), "See Project");
    }

    #[test]
    fn index_links_are_in_document_order() {
        let note = "#first [[a]] then [b](https://x.y) and [c](c.md) #last";
        let got: Vec<String> = walk(note)
            .index_links(&VaultPath::new("n.md"))
            .into_iter()
            .map(|l| l.raw_link)
            .collect();
        assert_eq!(
            got,
            [
                "#first".to_string(),
                VaultPath::note_path_from("a").to_string(),
                VaultPath::new("c.md").to_string(),
                "#last".to_string(),
            ]
        );
    }
```

And at the top of the `tests` module add `use crate::nfs::VaultPath;`.

Add to the `test` module in `core/src/note/content_extractor.rs`:

```rust
    #[test]
    fn a_wikilink_in_code_is_not_an_indexed_link() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let (chunks, links) = super::get_chunks_and_links(&path, "see `[[x]]` and [[y]]");
        let notes: Vec<String> = links
            .iter()
            .filter_map(|l| match &l.ltype {
                super::super::LinkType::Note(p) => Some(p.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(notes, [crate::nfs::VaultPath::note_path_from("y").to_string()]);
        assert!(chunks[0].text.contains("`[[x]]`"), "{:?}", chunks[0].text);
    }

    #[test]
    fn both_chunk_paths_render_a_tag_in_a_wikilink_alias_alike() {
        let path = crate::nfs::VaultPath::note_path_from("/n.md");
        let plain = super::get_content_chunks("see [[a|#b]]");
        let (indexed, _) = super::get_chunks_and_links(&path, "see [[a|#b]]");
        assert_eq!(plain, indexed);
        assert_eq!(plain[0].text, "see #b");
    }

    #[test]
    fn a_hashtag_that_looks_like_a_list_marker_keeps_its_setext_heading() {
        let headings = crate::note::note_headings("#1. Intro\n---\n\nbody\n");
        assert_eq!(headings.len(), 1);
        assert_eq!(headings[0].text, "1. Intro");
        assert_eq!(headings[0].level, 2);
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p kimun_core --lib -- headings_report_rows headings_rows_survive the_title_renders index_links_are_in a_wikilink_in_code_is_not both_chunk_paths a_hashtag_that_looks`
Expected: FAIL — `chunks`/`title`/`headings`/`index_links` not found on `NoteWalk` (compile error).

- [ ] **Step 3: Add the views**

In `core/src/note/walk.rs` imports add:

```rust
use crate::nfs::VaultPath;

use super::content_extractor::is_remote_url;
use super::{LinkType, NoteHeading, NoteLink};
```

Add:

```rust
/// Where a markdown link destination points, as the rendered markdown
/// writes it, plus the link it records: a URL, a note, or another vault
/// path (relative destinations resolve against `ref_path`'s directory).
/// `None` for a destination that is neither a URL nor a valid vault path.
pub(in crate::note) fn resolve_md_link(
    dest: &str,
    label: &str,
    ref_path: &VaultPath,
) -> (String, Option<NoteLink>) {
    if is_remote_url(dest) {
        return (dest.to_string(), Some(NoteLink::url(dest, label)));
    }
    if !VaultPath::is_valid(dest) {
        return (dest.to_string(), None);
    }
    let path = VaultPath::new(dest);
    if path.is_note_file() {
        return (path.to_string(), Some(NoteLink::note(&path, label)));
    }
    let base = if ref_path.is_note() {
        ref_path.get_parent_path().0
    } else {
        ref_path.to_owned()
    };
    let abs = base.append(&path).flatten();
    let link = if abs.is_note() {
        NoteLink::note(&abs, label)
    } else {
        NoteLink::vault_path(&abs, label)
    };
    (abs.to_string(), Some(link))
}

impl NoteWalk {
    /// Heading-chunked content, plus the `FrontMatter` chunk.
    pub(in crate::note) fn chunks(&self) -> Vec<ContentChunk> {
        let mut chunks = chunks_from_text_lines(self.lines.clone());
        if !self.frontmatter.is_empty() {
            chunks.push(ContentChunk {
                breadcrumb: "FrontMatter".to_string(),
                text: self.frontmatter.clone(),
            });
        }
        chunks
    }

    /// The first non-empty line: a heading's text, a paragraph's, or a list
    /// item's first line.
    pub(in crate::note) fn title(&self) -> String {
        self.lines
            .iter()
            .find_map(|line| match line {
                TextLine::Empty => None,
                TextLine::Header(_, text, _) => Some(text.to_owned()),
                TextLine::Text(text) => Some(text.to_owned()),
                // A wrapped item names the note by its first line only.
                TextLine::ListItem(_, text) => text.lines().next().map(str::to_owned),
            })
            .unwrap_or_default()
    }

    /// Every non-empty heading, with the row of `note` (the text this walk
    /// read) it starts on.
    pub(in crate::note) fn headings(&self, note: &str) -> Vec<NoteHeading> {
        let breaks: Vec<usize> = note.match_indices('\n').map(|(at, _)| at).collect();
        self.lines
            .iter()
            .filter_map(|line| match line {
                TextLine::Header(level, text, start) if !text.is_empty() => Some(NoteHeading {
                    level: *level,
                    text: text.clone(),
                    line: breaks.partition_point(|&at| at < *start),
                }),
                _ => None,
            })
            .collect()
    }

    /// The links the index records, in document order: wikilinks to valid
    /// vault paths, markdown links that resolve to a note, and hashtags.
    pub(in crate::note) fn index_links(&self, ref_path: &VaultPath) -> Vec<NoteLink> {
        let mut found: Vec<(usize, NoteLink)> = Vec::new();
        for link in &self.links {
            match link.kind {
                WalkLinkKind::Wiki | WalkLinkKind::WikiEmbed => {
                    if VaultPath::is_valid(&link.target) {
                        let path = VaultPath::note_path_from(&link.target);
                        found.push((link.range.start, NoteLink::note(&path, &link.label)));
                    }
                }
                WalkLinkKind::Inline | WalkLinkKind::Reference | WalkLinkKind::Autolink => {
                    // The index stores no link text; skip the allocation.
                    if let (_, Some(note)) = resolve_md_link(&link.target, "", ref_path) {
                        if matches!(note.ltype, LinkType::Note(_)) {
                            found.push((link.range.start, note));
                        }
                    }
                }
                WalkLinkKind::Image => {}
            }
        }
        found.extend(
            self.tags
                .iter()
                .map(|tag| (tag.range.start, NoteLink::hashtag(&tag.name))),
        );
        found.sort_by_key(|(at, _)| *at);
        found.into_iter().map(|(_, link)| link).collect()
    }
}
```

Remove the `#[allow(dead_code)]` attributes added in Task 2.

- [ ] **Step 4: Rewire `content_extractor.rs` and `mod.rs`**

In `core/src/note/content_extractor.rs`:

```rust
pub fn get_chunks_and_links<S: AsRef<str>>(
    reference_path: &VaultPath,
    md_text: S,
) -> (Vec<ContentChunk>, Vec<super::NoteLink>) {
    let walked = walk(md_text.as_ref());
    (walked.chunks(), walked.index_links(reference_path))
}

pub fn get_content_chunks<S: AsRef<str>>(md_text: S) -> Vec<ContentChunk> {
    walk(md_text.as_ref()).chunks()
}

pub fn extract_title<S: AsRef<str>>(md_text: S) -> String {
    walk(md_text.as_ref()).title()
}

pub fn heading_display_text(line: &str) -> Option<String> {
    let line = strip_block_markers(line);
    if !line.starts_with('#') {
        return None;
    }
    walk(line).lines.into_iter().find_map(|text_line| match text_line {
        TextLine::Header(_, text, _) => Some(text),
        _ => None,
    })
}
```

Keep the existing doc comments on these functions; update `get_chunks_and_links`'s doc to: "Chunks and the links the index records (note links and hashtags), from one walk over the note — see `walk.rs`." Add `use super::walk::walk;` to the imports.

In `core/src/note/mod.rs`, replace the body of `note_headings`:

```rust
pub fn note_headings(text: &str) -> Vec<NoteHeading> {
    walk::walk(text).headings(text)
}
```

Delete from `content_extractor.rs`: `walk_indexing_events`, `emit_md_note_link`, `strip_hashtags_in_text_event`, `collapse_wikilinks_with_display_ranges`, `collapse_inline_links`, `collapse_inline_links_tracking_lines`, `DroppedBreaks` (+ impl), `process_wikilinks_tracking_lines`, `extract_outline`, `loop_events`, `parse_text`, `split_frontmatter` (fold its body back into `remove_frontmatter`: same logic, returning `(frontmatter.join("\n"), content.join("\n"))` / `(String::new(), frontmatter.join("\n"))` and the BOM-stripped text when there is no fence).

In the `test` module, the helpers `assert_outline_lines` and `extract_headings` call `crate::note::content_extractor::extract_outline(...)`. Add this helper at the top of the module and point both at it (replace `crate::note::content_extractor::extract_outline(` with `extract_outline(`):

```rust
    fn extract_outline(text: &str) -> Vec<(u8, String, usize)> {
        crate::note::note_headings(text)
            .into_iter()
            .map(|h| (h.level, h.text, h.line))
            .collect()
    }
```

- [ ] **Step 5: Build, then remove what is now dead**

Run: `cargo clippy -p kimun_core --all-targets -- -D warnings`
Expected: errors only of the form "function `X` is never used". For each, delete `X` **if** it is one of: `process_wikilinks`, `cleanup_hashtags`, `cleanup_hashtags_with_ranges`. Any other dead item (e.g. `code_char_ranges`, `md_link_char_ranges`, `md_wikilink_char_ranges`, `MD_LINK_RX`) is still used by `ExclusionZones`, `link_char_spans` or `get_markdown_and_links` and must not be reported dead; if it is, stop and report it rather than deleting.

Re-run until clean.

- [ ] **Step 6: Run the new and existing tests**

Run: `cargo test -p kimun_core --lib`
Expected: PASS. If an existing test fails, it must assert a row of the spec's *Behaviour changes* table; edit only its expected value and add a comment `// spec: <row>`. A failure that maps to no row is a bug in the walk — fix the walk.

- [ ] **Step 7: Review the snapshot diff**

Run: `cargo test -p kimun_core --test extraction_snapshot`
Expected: FAIL with the diff command. Run the printed `diff` and check every hunk against *Behaviour changes*. Expected changes at this task (chunks, index chunks, index links, title, headings only — `-- tags`, `-- markdown links`, `-- markdown` sections must be **unchanged**):
- `setext_hashtag.md`: heading `2 0 "1. Intro"` appears; chunks gain breadcrumb `1. Intro`.
- `title_tag.md`: title `"Sprint 42"`; `title_wikilink.md`: title `"See Project"`.
- `edge_cases.md`: `` `[[x]]` `` stays in chunk text; `[[a|#b]]` renders `#b` in **both** chunk sections; `[[|b]] [[]]` stay literal; the `[[x]]` code link is gone from index links; heading `## After the broken link` keeps its row.
- Notes in `example/` change only where they contain one of those constructs.

Anything else: fix the walk, not the snapshot. When the diff is exactly the expected set:

Run: `UPDATE_SNAPSHOT=1 cargo test -p kimun_core --test extraction_snapshot`

- [ ] **Step 8: Checkpoint**

Run the gates. Report: views wired, old walk and rewrites deleted, snapshot diff reviewed (list the hunks by note). Do not commit.

---

### Task 4: Markdown rewrite, tags and link targets read the walk

**Files:**
- Modify: `core/src/note/walk.rs` (`render_markdown`, `tag_names`)
- Modify: `core/src/note/content_extractor.rs` (`get_markdown_and_links`)
- Modify: `core/src/note/mod.rs` (`extract_labels`, `tags_with`, `NoteMetadata::of`, new `note_link_targets`)
- Modify: `tui/src/cli/metadata_extractor.rs` (`extract_links`)
- Modify: `core/tests/snapshots/extraction.txt` (regenerated after review)

**Interfaces:**
- Consumes: `walk`, `NoteWalk::headings`, `resolve_md_link` (Tasks 2–3).
- Produces:
  - `NoteWalk::render_markdown(&self, note: &str, ref_path: &VaultPath) -> (String, Vec<NoteLink>)`
  - `NoteWalk::tag_names(&self) -> Vec<String>` (lowercased, sorted, distinct)
  - `pub fn note_link_targets(text: &str) -> Vec<String>` in `core/src/note/mod.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `core/src/note/walk.rs`:

```rust
    #[test]
    fn the_markdown_rewrite_splices_links_and_tags_in_place() {
        let note = "---\nrel: [[fm]]\n---\nsee [[p|P]] and [t](https://x.y) #tag `[[c]]` `[z](z.md)` ![[e]]\n";
        let (md, links) = walk(note).render_markdown(note, &VaultPath::new("dir/n.md"));
        let p = VaultPath::note_path_from("p");
        let e = VaultPath::note_path_from("e");
        assert_eq!(
            md,
            format!(
                "---\nrel: [[fm]]\n---\nsee [P]({p}) and [t](https://x.y) [#tag](#tag) `[[c]]` `[z](z.md)` ![e]({e})\n"
            )
        );
        let got: Vec<(String, String)> = links
            .into_iter()
            .map(|l| (l.raw_link, l.text))
            .collect();
        assert_eq!(
            got,
            [
                (p.to_string(), "P".to_string()),
                ("https://x.y".to_string(), "t".to_string()),
                ("#tag".to_string(), "tag".to_string()),
            ]
        );
    }

    #[test]
    fn reference_links_and_autolinks_are_recorded_but_not_rewritten() {
        let note = "[r][d] <https://a.b>\n\n[d]: https://c.d\n";
        let (md, links) = walk(note).render_markdown(note, &VaultPath::new("n.md"));
        assert_eq!(md, note);
        let raw: Vec<String> = links.into_iter().map(|l| l.raw_link).collect();
        assert_eq!(raw, ["https://c.d", "https://a.b"]);
    }

    #[test]
    fn tag_names_are_lowercased_sorted_and_distinct() {
        assert_eq!(walk("#B #a #b `#c` [x #d](y.md)").tag_names(), ["a", "b"]);
    }
```

Add to `tui/src/cli/metadata_extractor.rs`'s `tests` module:

```rust
    #[test]
    fn links_come_from_the_note_walk() {
        let links = extract_links("see [[a|A]] and [b](b.md) and `[[code]]` <https://c.d>");
        assert_eq!(links, ["a", "b.md", "https://c.d"]);
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p kimun_core --lib -- the_markdown_rewrite reference_links_and_autolinks tag_names_are`
Expected: FAIL — methods not found.

Run: `cargo test -p kimun-notes --lib links_come_from_the_note_walk`
Expected: FAIL — today's regex spans include `code` (from `` `[[code]]` ``) and miss the autolink.

- [ ] **Step 3: Implement the views**

Add to `impl NoteWalk` in `core/src/note/walk.rs`:

```rust
    /// The note rewritten for a renderer, plus its links: valid wikilinks
    /// become `[text](note path)` (`![…](…)` for an embed, left to the image
    /// pipeline, not listed), inline links get their destination resolved,
    /// hashtags become `[#tag](#tag)`. Reference links and autolinks are
    /// listed but left as written. Links come in document order, hashtags
    /// after them. Everything outside a recorded range — frontmatter, code,
    /// images — is copied verbatim.
    pub(in crate::note) fn render_markdown(
        &self,
        note: &str,
        ref_path: &VaultPath,
    ) -> (String, Vec<NoteLink>) {
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut links = Vec::new();
        for link in &self.links {
            match link.kind {
                WalkLinkKind::Wiki | WalkLinkKind::WikiEmbed => {
                    if !VaultPath::is_valid(&link.target) {
                        continue;
                    }
                    let path = VaultPath::note_path_from(&link.target);
                    let bang = if link.kind == WalkLinkKind::WikiEmbed {
                        "!"
                    } else {
                        links.push(NoteLink::note(&path, &link.label));
                        ""
                    };
                    edits.push((link.range.clone(), format!("{bang}[{}]({path})", link.label)));
                }
                WalkLinkKind::Inline => {
                    let (dest, found) = resolve_md_link(&link.target, &link.label, ref_path);
                    links.extend(found);
                    edits.push((link.range.clone(), format!("[{}]({dest})", link.label)));
                }
                WalkLinkKind::Reference | WalkLinkKind::Autolink => {
                    links.extend(resolve_md_link(&link.target, &link.label, ref_path).1);
                }
                WalkLinkKind::Image => {}
            }
        }
        for tag in &self.tags {
            links.push(NoteLink::hashtag(&tag.name));
            edits.push((tag.range.clone(), format!("[#{0}](#{0})", tag.name)));
        }
        edits.sort_by_key(|(range, _)| range.start);
        let mut out = String::with_capacity(note.len());
        let mut last = 0;
        for (range, replacement) in edits {
            out.push_str(&note[last..range.start]);
            out.push_str(&replacement);
            last = range.end;
        }
        out.push_str(&note[last..]);
        (out, links)
    }

    /// Hashtag names, lowercased, sorted and distinct.
    pub(in crate::note) fn tag_names(&self) -> Vec<String> {
        let names: std::collections::BTreeSet<String> =
            self.tags.iter().map(|t| t.name.to_lowercase()).collect();
        names.into_iter().collect()
    }
```

- [ ] **Step 4: Rewire the callers**

In `core/src/note/content_extractor.rs`, replace the body of `get_markdown_and_links`:

```rust
pub(crate) fn get_markdown_and_links<S: AsRef<str>>(
    reference_path: &VaultPath,
    md_text: S,
) -> (String, Vec<NoteLink>) {
    let note = md_text.as_ref();
    walk(note).render_markdown(note, reference_path)
}
```

In `core/src/note/mod.rs`:

```rust
pub fn extract_labels(text: &str) -> Vec<String> {
    walk::walk(text).tag_names()
}

pub fn note_tags(text: &str) -> Vec<String> {
    tags_with(walk::walk(text).tag_names(), &NoteDetails::property_set_of(text))
}

/// `labels` (a note's inline hashtags) plus the `tags` of its already-parsed
/// frontmatter, sorted and distinct.
fn tags_with(labels: Vec<String>, frontmatter: &properties::PropertySet) -> Vec<String> {
    let mut tags: std::collections::BTreeSet<String> = labels.into_iter().collect();
    tags.extend(frontmatter.tags());
    tags.into_iter().collect()
}

/// Every link target of a note in document order — wikilink targets, markdown
/// and image destinations, autolinks — frontmatter and code skipped.
pub fn note_link_targets(text: &str) -> Vec<String> {
    walk::walk(text).links.into_iter().map(|l| l.target).collect()
}
```

and in `NoteMetadata::of`:

```rust
    pub fn of(text: &str) -> Self {
        let frontmatter = NoteDetails::property_set_of(text);
        let walked = walk::walk(text);
        Self {
            tags: tags_with(walked.tag_names(), &frontmatter),
            properties: frontmatter.into_entries(),
            headings: walked.headings(text),
        }
    }
```

Keep the existing doc comments; update `extract_labels`'s to say the labels come from the note walk (frontmatter, code, HTML and link text skipped).

In `tui/src/cli/metadata_extractor.rs`:

```rust
pub fn extract_links(content: &str) -> Vec<String> {
    kimun_core::note::note_link_targets(content)
}
```

- [ ] **Step 5: Remove what is now dead**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Delete items reported unused **only** from this list: `process_wikilinks`, `md_link_char_ranges`, `md_wikilink_char_ranges`, `code_char_ranges`, `MD_LINK_RX`, `cleanup_hashtags`, `cleanup_hashtags_with_ranges`. Items `ExclusionZones` or `link_char_spans` still use are not reported — keep them. Anything reported that is not on the list (e.g. `frontmatter_end_byte`, which `walk.rs` uses) means a wiring mistake: stop and fix the wiring. Re-run until clean.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p kimun_core --lib && cargo test -p kimun-notes --lib`
Expected: PASS. Same rule as Task 3 Step 6 for any existing failure: only *Behaviour changes* rows may change an expectation, with a `// spec: <row>` comment.

- [ ] **Step 7: Review the snapshot diff**

Run: `cargo test -p kimun_core --test extraction_snapshot`
Expected: FAIL with the diff command. Only the `-- tags`, `-- markdown links` and `-- markdown` sections may change, and only as:
- `edge_cases.md`: `` `[[x]]` `` and `` `[y](y.md)` `` no longer rewritten or listed; the reference link and autolink appear in `-- markdown links`; `[[a|#b]]` / `[[#tag]]` produce no tag.
- `frontmatter_links.md`: `[docs](other.md)` and `[[refnote]]` inside the frontmatter no longer rewritten or listed.
- `example/` notes only where they contain those constructs.

Then: `UPDATE_SNAPSHOT=1 cargo test -p kimun_core --test extraction_snapshot`

- [ ] **Step 8: Checkpoint**

Run the gates. Report: rendering, tags, CLI links wired; dead code removed (list it); snapshot diff reviewed. Do not commit.

---

### Task 5: Index version, consistency properties, bench gate, docs

**Files:**
- Modify: `core/src/index/mod.rs:136-144` (`VERSION` + history comment)
- Modify: `core/tests/extraction_snapshot.rs` (consistency test)
- Modify: `docs/` pages only if they describe wikilinks/tags in code, titles or frontmatter links (Step 5)

**Interfaces:**
- Consumes: everything above, through the public API only.

- [ ] **Step 1: Write the consistency test**

Append to `core/tests/extraction_snapshot.rs`:

```rust
#[test]
fn every_extractor_agrees_with_the_others() {
    use kimun_core::note::scan::heading_display_text;
    use kimun_core::note::LinkType;

    let path = VaultPath::new("folder/note.md");
    for (name, text) in corpus() {
        let details = NoteDetails::new(&path, &text);
        let headings = note::note_headings(&text);
        let chunks = details.get_content_chunks();
        let (index_chunks, index_links) = details.get_chunks_and_links();
        assert_eq!(chunks, index_chunks, "{name}: chunk paths disagree");

        // Every breadcrumb's last segment is a listed heading's text.
        for chunk in chunks.iter().filter(|c| c.breadcrumb != "FrontMatter") {
            if let Some(last) = chunk.breadcrumb_last() {
                assert!(
                    headings.iter().any(|h| h.text == last),
                    "{name}: breadcrumb {last:?} is no heading"
                );
            }
        }

        // An ATX heading on its own line renders the same alone (a setext
        // heading cannot: its underline is on the next row).
        let rows: Vec<&str> = text.lines().collect();
        let is_atx = |row: &str| {
            let hashes = row.len() - row.trim_start_matches('#').len();
            (1..=6).contains(&hashes) && matches!(row[hashes..].chars().next(), None | Some(' ' | '\t'))
        };
        for h in &headings {
            let row = rows[h.line];
            if is_atx(row) {
                assert_eq!(
                    heading_display_text(row).as_deref(),
                    Some(h.text.as_str()),
                    "{name}: row {}",
                    h.line
                );
            }
        }

        // The title is the first heading's text when the note opens with one
        // (notes with frontmatter are skipped: their first rows are the block).
        let has_frontmatter = text.starts_with("---") || text.starts_with("+++");
        if let Some(first) = headings.first() {
            let first_row = rows.iter().position(|r| !r.trim().is_empty());
            if !has_frontmatter && first_row == Some(first.line) {
                assert_eq!(details.get_title(), first.text, "{name}: title");
            }
        }

        // Tags agree across the index, the markdown rewrite and note_tags.
        let (_, md_links) = details.get_markdown_and_links();
        let tags_of = |links: &[NoteLink]| -> std::collections::BTreeSet<String> {
            links
                .iter()
                .filter(|l| matches!(l.ltype, LinkType::Hashtag))
                .map(|l| l.text.to_lowercase())
                .collect()
        };
        assert_eq!(tags_of(&index_links), tags_of(&md_links), "{name}: tags");
        assert_eq!(
            tags_of(&index_links),
            note::extract_labels(&text).into_iter().collect(),
            "{name}: extract_labels"
        );

        // Note links agree (embeds are images in the rewrite, so the rewrite
        // lists a subset of the index's notes).
        let notes_of = |links: &[NoteLink]| -> std::collections::BTreeSet<String> {
            links
                .iter()
                .filter_map(|l| match &l.ltype {
                    LinkType::Note(p) => Some(p.to_string()),
                    _ => None,
                })
                .collect()
        };
        assert!(
            notes_of(&md_links).is_subset(&notes_of(&index_links)),
            "{name}: rewrite lists a note link the index lacks"
        );
    }
}
```

The title check only applies when the note's first non-blank row is its first heading; a note opening with a paragraph is titled by that paragraph.

- [ ] **Step 2: Run it**

Run: `cargo test -p kimun_core --test extraction_snapshot every_extractor_agrees`
Expected: PASS. A failure names the note and the property — fix the walk view that disagrees.

- [ ] **Step 3: Bump the index version**

In `core/src/index/mod.rs`, after the `// 0.17:` history lines:

```rust
// 0.18: chunks, links and tags come from one walk over the note as written
//       (wikilinks parsed by pulldown-cmark, hashtags handled inside it):
//       wikilinks in code are no longer links, reference links and autolinks
//       are recorded, a `#`-prefixed setext heading is kept, and a title
//       renders like its heading. Bump forces a clean reindex.
const VERSION: &str = "0.18";
```

Run: `cargo test -p kimun_core --lib index`
Expected: PASS.

- [ ] **Step 4: Bench gate**

Run: `cargo bench -p kimun_core --bench indexing -- --baseline before`
Expected: criterion reports no regression ("No change in performance detected" or "Performance has improved") for every `get_chunks_and_links/*` entry. A reported regression blocks: profile the walk (likely `prose_text` or `chunks()` cloning `lines`) before going on.

- [ ] **Step 5: Docs**

Run: `grep -rn -i -E "wikilink|hashtag|\\[\\[|title" docs/ | grep -v "^docs/.*\\.(png|svg)"`
For each hit, check whether it states something the *Behaviour changes* table made untrue (a wikilink in code being a link, titles keeping `#tags`, links in frontmatter being listed). Edit only those sentences. If none, record "no doc changes needed" in the report.

- [ ] **Step 6: Full gates and checkpoint**

Run every gate from Global Constraints, with the `touch` first. Then re-run `cargo fmt --all -- --check` last.
Report to the user: version bumped, consistency test green, bench comparison (numbers per fixture), docs touched or not, all gates green. Note for the release: a server vector store keeps old chunk text until a note is edited; a manual server reindex refreshes it. Do not commit.
