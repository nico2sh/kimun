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
