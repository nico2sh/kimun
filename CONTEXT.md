# Kimün

Domain language for kimün — a note-taking app split into **core** (file ops, indexing, notes) and a **TUI** (interaction and presentation). This file records terms whose meaning is not obvious from the code alone; general programming concepts do not belong here.

## Language

### Editor parsing

**Parse state**:
The TUI editor's per-buffer parse cache, which is either a **Real parse** or a **Placeholder parse**. Modelled as the `ParseState` enum in the markdown editor view; the distinction exists only to keep typing responsive on large buffers.
_Avoid_: parse mode, parse status

**Real parse**:
A fully-styled `ParsedBuffer` produced synchronously by `pulldown-cmark`. The only parse state on which an incremental **splice** is legal.

**Placeholder parse**:
A structurally-correct but unstyled `ParsedBuffer` installed synchronously when a large-buffer edit trips the incremental cap, so the frame paints immediately; the real parse is deferred to a background task and swapped in when it lands. Splicing into a placeholder is forbidden — its all-`Plain` line kinds would defeat the structural guards and accept a wrong splice.
_Avoid_: stub parse, fake parse, temp buffer

### Editor rendering

The editor is WYSIWYG-ish: it shows styled markdown, not raw source, except on the line currently being edited.

**Sigil**:
The markdown marker characters that signal a construct rather than being read as prose — `#` for headings, `>` for blockquotes, the list bullet/number, the backtick/tilde code fences. The styled view hides or mutes them so the prose reads cleanly.
_Avoid_: marker token, syntax char

**Reveal**:
When the cursor sits on a styled construct, the editor drops the styling for that line and shows the raw markdown (sigils included, muted) so it can be edited directly. The cursor leaving re-applies the styled form. The element-scoped form of this is an **expanded element**.

**Blockquote bar**:
The vertical `│` gutter the editor paints in place of the `>` sigils of a blockquote. One bar per nesting depth, repeated on wrapped continuation rows so the quote reads as a single left-edged block. Replaced by the raw `> ` on the line being edited (see **Reveal**).

**Code box**:
The background rectangle the editor paints behind a code block (fenced or indented). Sized to the block's widest line and capped at the editor width — a box hugging the code, not a full-width band.

### Search

**Note link**:
A note→note reference inside a note's body — either a `[[wikilink]]` or a markdown link resolving to a vault note. Attachments, images, and external URLs are *not* note links. Only note links participate in the **link filter**.

**Link filter**:
The search operator that selects notes by the note links they contain. "Notes that link to X" means notes whose body contains a note link **to** X — i.e. X's backlinks, not X's outgoing links. The target is matched by note name (extension optional, case-insensitive, `*` wildcards), across any folder unless a path is given to disambiguate.
_Avoid_: backlink search (correct, but ambiguous about direction when read quickly)

### Note editing

**Automated edit**:
A note mutation performed through the CLI or the MCP server rather than the TUI editor. Automated edits produce a **backup**; interactive TUI edits do not (the editor carries its own version history).
_Avoid_: programmatic write, headless edit

**Append**:
Adding text to the end of a note, leaving existing content intact. The only additive write; never destructive.

**Overwrite**:
Replacing a note's **entire** body with new content. Distinct from append (additive) and replace (partial).
_Avoid_: write, save (too generic — they don't signal that the old body is discarded)

**Replace**:
A targeted edit that swaps an existing substring for new text, leaving the rest of the note intact. The match must be unambiguous unless every occurrence is explicitly targeted. Distinct from overwrite (whole body).
_Avoid_: find-and-replace (implies regex/global semantics by default), edit

**Backup**:
A pre-change copy of a note, taken automatically before an automated edit overwrites or removes its content, retained for later recovery and reclaimed once it ages out. Kept in a hidden directory inside the vault, so it is excluded from the index but travels with the notes when the vault is copied.
_Avoid_: snapshot, version (those imply the TUI's own history, which is separate)
