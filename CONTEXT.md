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

**Query variable**:
A `{name}` placeholder inside a query that the TUI resolves to a runtime value before handing a plain query string to core. Core's query language has no notion of these — substitution happens entirely in the presentation layer. The first variable is `{note}`, the **clean name** of the note currently open in the editor; a bare `>` typed in the query panel is sugar that expands to `>{note}`. Backlinks of the current note are therefore just the query `>{note}`.
_Avoid_: macro, token (too generic), current-note placeholder (only describes one variable)

**Saved search**:
A named query persisted in the vault under `.kimun/`, so it travels with the notes when the vault is copied (same rationale as **Backup**). The query string is stored verbatim, including any **query variable** like `{note}`, and re-resolved each time it runs. Core owns reading and writing them; the TUI only presents and resolves them.
_Avoid_: bookmark, smart folder, filter (too generic)

**Saved Searches modal**:
A global picker, opened by a single key binding, listing the vault's **saved searches** for keyboard selection (arrows/enter plus numeric quick-select 1–9). Picking one runs it in the **Query panel**. Distinct from the Ctrl+K **note browser**, which finds individual notes rather than choosing a query. It is one **SearchList** surface, not to be confused with the module itself.
_Avoid_: query menu

### TUI search surfaces

**SearchList**:
The one module behind every query-input-over-an-async-loaded-list surface in the TUI — the **note browser**, the **Query panel**, the **Saved Searches modal**, and the directory sidebar. It owns the query input, keyboard navigation, the async-load lifecycle, the autocomplete host, and selection; it emits nothing on its own — callers read the selected row and decide the action. Rich presentation (the Query panel's expand/preview, the note browser's preview pane) composes on top rather than living inside it.
_Avoid_: list widget, search box (each names only a part)

**Row source**:
The seam that supplies a **SearchList** with the rows for a query. Vault-backed in the app (search, backlinks, saved searches, directory listing), in-memory in tests — so a SearchList is exercised without a real vault. Streaming and one-shot delivery are the same source, not different seams.
_Avoid_: provider (too generic), repository

**Search row**:
What a single row must tell its **SearchList** to be listed, filtered, navigated, and drawn — the only thing that varies with the row's type (a note, a saved search, a directory entry). Anything richer is read back by the caller from the selected row.

**Suggestion source**:
The seam that supplies the query input's autocomplete with candidates (note names for `>`, tag labels for `#`), kept separate from the **row source** and from the vault so the autocomplete host is testable in isolation.

**Query panel**:
The right-hand panel of the editor. Shows the list of notes matching an active query, with the same expandable list/preview affordances as the rest of the app. Backlinks are not a distinct feature here — they are the default query `>{note}`, so a freshly opened panel shows the current note's backlinks. The panel title reflects the active query (reads "Backlinks" when the query is `>{note}`).
_Avoid_: backlinks panel (now only the default state), search panel (collides with Ctrl+K and the left-sidebar search box)

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
