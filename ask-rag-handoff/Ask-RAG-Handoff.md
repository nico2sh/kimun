# Kimün — Ask (RAG) · Implementation Handoff

Build the **Ask** view: a keyboard-first RAG workspace where a natural-language
question retrieves note chunks from the vector index, sends them to an LLM as
context, and streams back a cited answer the user can review, converse with,
copy, and save as a note.

## Reference prototype
Everything below is realized in the clickable prototype — treat it as the spec of record:
- `Ask (RAG).html` — layout + wiring
- `rag.css` — styles (all colors are Gruvbox tokens from `kimun.css`; **do not hardcode**, use semantic vars)
- `rag-vault.js` — sample vault + **mock** retrieval (the stand-in for the real vector DB)
- `rag-app.js` — all UI logic, state, keyboard model

Open it, `Tab` through the three zones, ask a follow-up, click a citation, press `?`.

---

## 1. Layout
A new activity-rail entry **✦ ASK**. Three panels left→right, matching the other
views (list on the left, working area on the right):

```
[rail] │ Sources (context)  │ Thread (conversation)      │
       │  top_k chunks       │  ...scrolls...             │
       │  + score bars       │  Query composer (docked)   │
```

- **Sources** (left, fixed width): the retrieved chunks for the *selected* turn.
- **Thread** (center/right, flex): scrolling Q/A conversation, composer docked at bottom.
- Focus context = green border (no modes). `Tab`/`Shift-Tab` cycles the three.

## 2. Core flow
1. User types a question in the composer, `Enter` submits (`Shift-Enter` = newline).
2. **Retrieve**: embed the query, vector-search the index, take `top_k` (default 5) chunks.
3. Append a turn with `status: thinking`, populate its `sources`, render immediately.
4. **Generate**: send system prompt + prior turns + `{context, question}` to the LLM; stream tokens into the answer.
5. Parse inline `[n]` citations → clickable superscripts that map to `sources[n-1]`.
6. `status: done` → show source pills + actions.

Follow-ups reuse the same thread; prior Q/A pairs are passed as conversation history.

## 3. Data contracts
```ts
type Chunk = {
  noteId: string; path: string; title: string; mtime: string;
  heading: string;        // section the chunk came from
  text: string;           // the retrieved chunk body
  score: number;          // similarity 0..1 (shown as %)
};
type Turn = {
  id: number; question: string; answer: string;
  sources: Chunk[];       // the top_k for THIS turn
  status: 'thinking' | 'streaming' | 'done';
};
```

### Retrieval — replace the mock
`rag-vault.js#retrieve(query, k)` is a keyword stand-in. Swap for the real path:
embed query → vector search → return `Chunk[]` in the contract above. Chunk-level
scores drive the similarity bars, so return them per chunk, not per note.

### Generation — LLM call
`rag-app.js#generate(question, sources, history)`:
- System prompt: *answer ONLY from the numbered context; cite with `[n]`; preserve
  `[[wikilinks]]`/`#tags`; say so if context is insufficient.*
- Context block = numbered `[i] path — "heading"\n text`.
- History = prior done turns as alternating user/assistant messages.
- **Must stream.** Keep an extractive fallback (top-chunk sentences + citations)
  for offline / error so the view never dead-ends.

## 4. Reviewing sources (key requirement)
The user reviews retrieved note content **without losing the answer**. The Sources
panel has two modes:
- **list**: ranked chunks — rank, title, path, similarity bar, snippet with query
  terms highlighted.
- **reader**: `Enter`/`l`/click a source or a citation → render the *full note*
  with the retrieved chunk highlighted (aqua left rule). The thread/answer stays
  in place on the right. `h`/`Esc` returns to the list.

## 5. Answer as content (key requirement)
Each done answer offers:
- **copy** (`y`) → plaintext answer to clipboard (strip `[n]`/markdown).
- **edit as note** (`e`) → answer becomes an editable buffer styled as note
  content; `Ctrl-S` writes a real note (suggested path `ask/<slug>.md`), `Esc` cancels.
- **regenerate** (`r`) → re-run generation for that turn's question + sources.

## 6. Keyboard model (mouse mirrors everything)
| Scope | Keys |
|---|---|
| Global | `Tab`/`⇧Tab` cycle · `Ctrl-K` leader menu · `Space` leader (in a list) · `i`/`/` to query · `?` help |
| Query | `⏎` ask · `⇧⏎` newline · `Esc` → thread |
| Thread | `j`/`k` turns · `⏎`/`l` → sources · `y` copy · `e` edit · `r` regen |
| Sources | `j`/`k` move · `⏎`/`l` open reader · `h`/`Esc` back · `o` open in editor · `y` yank path |

Leader (`Ctrl-K`) → `a` ask · `n` new conversation · `y` copy · `e` edit · `r` regen · `s` open top source · `?` help. Matches the app's existing which-key tree.

## 7. Theming
Reference default is Gruvbox Dark, but the app is themeable — **every color is a
semantic token** (`--fg`, `--aqua`, `--sel`, `--green`, `--tok-*`, …). No literals.

## 8. Acceptance
- Ask → within one frame a turn appears with `thinking` state and populated sources.
- Answer streams; `[n]` citations are clickable and select the matching source.
- Selecting any turn repopulates the Sources panel with *that* turn's context.
- Reader shows full note with the retrieved chunk highlighted; answer stays visible.
- `y` copies; `e`→edit→`Ctrl-S` persists a note; `r` regenerates.
- Follow-up questions continue the same conversation with history.
- Fully operable by keyboard alone; every action also reachable by mouse.
- LLM/vector failures fall back gracefully (extractive answer, empty-but-labeled sources).

## Out of scope (v1)
Multi-vault ask, per-source relevance re-ranking UI, saved/starred conversations,
streaming citation validation. Note them; don't build them.
