# Kimün

Domain language for kimün — a note-taking app split into **core** (file ops, indexing, notes) and a **TUI** (interaction and presentation). This file records terms whose meaning is not obvious from the code alone; general programming concepts do not belong here.

## Language

### Editor backend

**Editor backend**:
Which engine drives the TUI text editor, chosen in config (`editor_backend`): **textarea** (the built-in ratatui-textarea), **nvim** (an external neovim process), or **vim** (built-in vim emulation — a textarea buffer plus a modal state machine, no external process). One config axis, three values.
_Avoid_: editor engine, editor mode (collides with **editing mode**).

**Editing mode**:
The active modal state inside a vim-style backend — Normal, Insert, Visual, Visual-line, Command. Shared by the **nvim** and **vim** backends (the `EditorMode` enum); the **textarea** backend has none. Distinct from the **editor backend**, which selects the engine, not the state within it.
_Avoid_: vim mode (ambiguous — backend or state?), NvimMode (the superseded nvim-only name).

### Vim emulation

**Vim command**:
The reified unit of work in the **vim** editor backend's engine (the `Command` enum): keys parse into a command value, and `apply` is the *only* door that mutates the buffer (adr/0011). Dot-repeat replays the recorded command (plus its captured insert delta) through that same door, so first press and replay cannot diverge; macros (v2) replay a longer log of them. Parsing (`parse_normal`) is pure pending-state accumulation and never touches the buffer.
_Avoid_: action, keystroke handler (those describe the superseded imperative form).

**Unnamed register**:
The engine-owned register of the **vim** editor backend: text and its kind (charwise/linewise) stored together as one value, filled by every yank *and* every delete/change (vim rule — so `xp` swaps chars). Kept separate from the textarea's yank buffer (a transport only, never read at paste time) and from the OS clipboard (the Ctrl-c/v path). Named registers (v2) add a map alongside.
_Avoid_: yank buffer (the textarea's, not the register), clipboard.

**Span kind**:
Vim's motion classification (`:h exclusive`) as carried by the engine: a motion consumed by an operator forms an **exclusive**, **inclusive**, or **linewise** range (`SpanKind`). j/k and gg/G are linewise (so `dj` deletes whole lines), e/f/t/%/$ are inclusive, the rest exclusive. `select_range` is the single home of the vim-inclusive → ratatui-half-open `+1` conversion.
_Avoid_: motion type (too generic), inclusivity flag scattered per call-site (the pre-range-model shape).

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
A search operator that selects notes by the note links between them, in one of two directions (see ADR-0005 for the full operator alphabet). The arrow points **relative to the named note**:
- **Backlinks** — `<X` / `lk:X` — notes whose body contains a note link **to** X (links pointing *into* X).
- **Forward links** — `>X` / `fwd:X` — the notes that **X links to** (links pointing *out* of X).
The target is matched by note name (extension optional, case-insensitive, `*` wildcards), across any folder unless a path is given to disambiguate.
_Avoid_: backlink search (names only one direction), `>`/`@` (the pre-ADR-0005 chars).

**Query variable**:
A `{name}` placeholder inside a query that the TUI resolves to a runtime value before handing a plain query string to core. Core's query language has no notion of these — substitution happens entirely in the presentation layer. The first variable is `{note}`, the **clean name** of the note currently open in the editor; a bare `<` typed in the query panel is sugar that expands to `<{note}`. Backlinks of the current note are therefore just the query `<{note}`.
_Avoid_: macro, token (too generic), current-note placeholder (only describes one variable)

**Saved search**:
A named query persisted in the vault under `.kimun/`, so it travels with the notes when the vault is copied (same rationale as **Backup**). The query string is stored verbatim, including any **query variable** like `{note}`, and re-resolved each time it runs. Core owns reading and writing them; the TUI only presents and resolves them.
_Avoid_: bookmark, smart folder, filter (too generic)

**Saved Searches modal**:
A global picker, opened by a single key binding, listing the vault's **saved searches** for keyboard selection (arrows/enter plus numeric quick-select 1–9). Picking one runs it in the **Query panel**. Distinct from the Ctrl+K **note browser**, which finds individual notes rather than choosing a query. It is one **SearchList** surface, not to be confused with the module itself.
_Avoid_: query menu

**Saved-search expansion**:
The inline alternative to the **Saved Searches modal**: a leading `?` typed in a query input (the **Query panel** and the **note browser**) opens an autocomplete over **saved search** names; accepting one replaces the whole field with that search's stored query — verbatim, query variables intact — so it is then editable like any query. `?` is a presentation-layer sigil only; core's query language never sees it (same boundary as the **query variable**; see `adr/0006`). The expansion pins a **saved-search breadcrumb**.
_Avoid_: saved-search operator (it is not a core query operator), saved-search reference (we expand, not reference).

**Saved-search breadcrumb**:
The name label a **saved-search expansion** pins on the query input's top border, recording which saved search the current query came from. Sticky provenance: it survives edits to the expanded query (gaining an "edited" marker once the text diverges from the stored query — any divergence counts, including the order directive, since the stored query is saved verbatim) and clears only when the field is emptied or another saved search is expanded. Saving the live query re-pins it to the saved search just written — the saved identity, not the original expansion, is the provenance from then on (so the "edited" marker drops on an update, and the name switches on a save-as-new).
_Avoid_: title (the Query panel already has a query-reflective title; the breadcrumb is a distinct provenance tag).

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
The right-hand panel of the editor. Shows the list of notes matching an active query, with the same expandable list/preview affordances as the rest of the app. Backlinks are not a distinct feature here — they are the default query `<{note}`, so a freshly opened panel shows the current note's backlinks. The panel title reflects the active query (reads "Backlinks" when the query is `<{note}`).
_Avoid_: backlinks panel (now only the default state), search panel / search sidebar (collide with Ctrl+K and the left-sidebar search box)

### TUI surfaces

**Panel**:
A persistent surface in the editor screen's column layout — the **Activity rail**, the **Drawer**, or the editor — exactly one focused at a time. Distinct from an **Overlay**, which is transient and modal.
_Avoid_: pane, view, widget, component (too generic).

**Activity rail**:
The fixed-width icon strip on the far left of the editor screen. Each cell names a **drawer view**; selecting a cell (click, ↑/↓ + Enter, or later a leader path) switches the **Drawer** to that view. The active cell shows a green edge bar; CFG is pinned to the bottom.
_Avoid_: sidebar (the pre-rail name for the file browser), icon bar, toolbar.

**Drawer**:
The single panel between the **Activity rail** and the editor; renders whichever **drawer view** is active. Toggleable (Ctrl-B) — hiding it gives its width to the editor — and divider-drag resizable. Exactly one drawer renders at a time.
_Avoid_: sidebar / Query panel as panel names (they are now drawer views), side panel.

**Drawer view**:
What the **Drawer** can show: FILES (the file browser, formerly the sidebar), FIND (the **Query panel**), TAGS, LINKS, OUTLINE, CFG. The rail and the drawer stay in step through the view, not through panel identities.

**Open-note marker**:
The accent recoloring of a FILES-list row's type glyph that flags the note currently open in the editor. Lives only in the editor's FILES **drawer view** — Browse never has an open note. Driven by the sidebar's tracked open-note path, matched by `is_like`, and re-applied after every listing (re)load. Distinct from selection (the navigation cursor's row highlight): a row can be selected, open, both, or neither.
_Avoid_: active note (collides with the focused **Panel** / selection), current note (that is the `{note}` **query variable** — the open note's clean name).

**PanelSet**:
The fixed left→right collection of the editor screen's **Panels** (rail → drawer → editor); owns which panel is focused, drawer visibility and width, and focus cycling, and routes input and render to the focused panel. Focus cycles over the visible panels, wrapping at both ends. The persistent-surface counterpart to the **OverlayHost**.
_Avoid_: panel manager, layout, panel stack.

**Overlay**:
A transient, modal surface drawn on top of the **Panels** — the **note browser**, the **Saved Searches modal**, or a dialog. Captures all input while open; closing restores focus to the panel that opened it.
_Avoid_: popup, modal (names only some), dialog (names only one kind).

**OverlayHost**:
The single-slot owner of the active **Overlay**; saves the opener panel's focus on open and returns it on close. The transient-surface counterpart to the **PanelSet**.
_Avoid_: dialog manager (the superseded name), overlay stack (it is single-slot).

### Indexing

**NoteIndex**:
The one core module owning the searchable index of the vault — search, suggestions, backlinks, and the index's own lifecycle (schema versioning, self-heal on open). Its interface speaks in notes, queries, and **note links**; SQLite, sqlx, transactions, and schema migrations are implementation and never cross the interface. Atomicity is carried by composite operations (apply an **IndexDiff**; rename a note together with its rewritten backlinks) rather than by exposing transactions.
_Avoid_: db, VaultDB, database (they name the implementation, not the role)

**Index self-heal**:
On open, the **NoteIndex** silently recreates its schema when the stored index is missing, outdated, or invalid — leaving a valid but empty index that the next sync pass fills. Callers get a single readiness probe (`index_ready`): false when the index was just healed (or never filled), so fast paths like the CLI `note` command can refuse to run against an empty index. There is no public status enum.
_Avoid_: DBStatus (the superseded public enum), force rebuild (the deleted file-deletion variant)

**IndexDiff**:
The batch of note changes — to add, to modify, to delete — that a vault sync walk produces and `NoteIndex::apply` consumes in one atomic operation. Owned by the **NoteIndex** interface: it is the currency crossing that seam, not a walker by-product.
_Avoid_: NoteListResults (the superseded visitor type), results

**LinkRewrite**:
The one core module that rewrites every **note link** pointing at a renamed note. Three compiler-enforced stages — *scout* (one index query for the linking notes), *prepare* (read each, rewrite links in memory, take fail-closed **backups**), *commit* (write the rewritten notes, rewrite the renamed note's self-links at its new path, return the entries for the index commit) — with the caller's filesystem rename sitting between prepare and commit. Each stage consumes the previous, so running them out of order is a compile error, not a broken vault.
_Avoid_: backlink rewriting (names one half; self-links are the other), rename helper

**VaultSync**:
The one core module that brings the **NoteIndex** in step with the vault on disk. One call runs the whole pipeline — read the cached entries, walk the subtree in parallel, diff against the cache under a validation mode, apply the **IndexDiff**, optionally streaming discovered entries to the caller as they are found. The parallel walker, its thread-state plumbing, and the async/blocking bridge are implementation and never cross the interface.
_Avoid_: visitor, walker, indexer (each names an internal part, not the module)

### Note content

**Note details**:
The one public door to whole-note extraction — title, indexable content data, heading chunks, links, rendered markdown — as `NoteDetails`: methods over a loaded note's owned text, plus borrowed-text associated functions (`*_of`) for bulk paths (indexing) that must not clone. The markdown extractor behind it is internal to the note module and is never named outside it.
_Avoid_: content extractor (the implementation, not the door)

**Scan helpers**:
The `note::scan` module — live text analysis over editor buffer fragments: link/wikilink spans, exclusion zones (code, frontmatter, links), label tokens, URL classification. The presentation layer drives WYSIWYG behaviour with these on text being edited; they take arbitrary text fragments, not notes. Whole-note extraction belongs to **Note details** instead.
_Avoid_: span helpers / zone helpers (each names a part), parser utilities

### Note editing

**Auto-surround**:
Typing an opening pair character (`(` `[` `{` `<`) or a symmetric one (`"` `'` `` ` `` `*` `_` `~`) while a selection is active wraps the selection in the pair instead of replacing it. The selection stays on the inner text afterwards, so wraps chain — `[` `[` builds a wikilink, `*` `*` builds bold. Closing characters do not wrap; they replace, as any other key. Textarea backend only.
_Avoid_: auto-pair, auto-close (those mean inserting the closing char while typing without a selection — a different feature kimün does not have)

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
