# Agentic Ask: design

Date: 2026-11-25
Status: Approved in conversation. Written for review.
Decision record: `adr/0065-the-answer-is-a-bounded-tool-loop-over-pushed-data.md`

## Intent

Ask answers questions from the vault's notes. Today it fails in two common cases.

1. A question about a time ("what happened last week") does not retrieve the journal entries of that time. Vector similarity cannot filter by date.
2. Some answers need a note that looks relevant only after the first results are read. Examples are a linked note and the rest of a note. Retrieval runs once, so the assistant never fetches it.

The goal is an assistant that finds the notes that fit the request. When it adds knowledge of its own, it must say so.

## Goals

- Time questions retrieve the right notes.
- The assistant can look a second time: read a whole note, follow links.
- A reader can always tell which parts of an answer come from the notes and which do not.
- The user sees what the assistant did, while it works and afterward.
- A simple question costs about what it costs now.

## Non-goals

- No change to how notes are embedded or chunked.
- No client-side or callback tools. The loop runs in the server.
- No structured answer parts (from notes or from model). Disclosure is text.
- No migration or schema version. Few servers exist, and a one-time re-push is fine.
- No fixed question set in the first version.

## Anti-goals

- An answer that looks confident while no note matched.
- A loop that runs long on a simple question, or that cannot be stopped by the step cap.
- A broken Ask on a model that cannot use tools.
- A tool that reads another vault.

## Approaches

The chosen shape is a bounded tool loop in the server. The alternatives and the reasons for the rejections are in the ADR: a single planning call, a loop in the client, callback tools, a text protocol, and modified time as a date. The single planning call is the cheapest shape and is what the fallback resembles.

## Design

### The loop

1. The client sends the question, the prior turns, and its local date.
2. The server runs the first `search` on its own, using the conditioned query that exists today (the last turn folded into the search text). The context cut applies to its results as it does now.
3. The model receives the question, the date, and those results. The model can call tools, for at most 5 tool rounds.
   - The server runs at most 4 tool calls in one turn. It answers each extra call with an error result that says the call was not run. Extra calls do not count as malformed. The trail records how many calls ran.
4. When the model stops calling tools, or the cap is reached, it writes the answer.
5. Sources are the chunks gathered and used, merged and numbered once. `[n]` citations point into that list.

### Tools

All tools are locked to the asking vault's collection. Results are size-capped, and a capped result says so.

- `search`: query text, plus optional filters. Note-date range, with a property key or "journal". Folder. Tags. Returns ranked chunks.
- `list_date_keys`: the date property keys present in the vault.
- `read_note`: a note by path, whole or one section.
- `related_notes`: for a note, the notes it links to and the notes that link to it.

### Note date

A note date is the date in a journal filename, or the value of any date-typed property. Each keeps the name of its source ("journal", `due`, `created`). A note with neither has no note date and never matches a date filter. The glossary entry is **Note date** in `CONTEXT.md`.

### Pushed data

Kimun pushes per note: its note dates with source names, its tags, and its outgoing links. The folder is not a pushed field. The server takes it from the pushed path. Clients push links as written. When a tool runs, the server resolves them. The server builds the reverse direction (backlinks) from them. The fields come from the note text. When a property or a link changes, the full-text hash that reconcile compares already changes. The server still never reads the vault.

When this feature ships, the hash does not change. On SQLite (the default store) the server drops and recreates its `chunks` table on the first start, because the column layout differs. There is no migration. Every client then re-pushes all notes once, and older notes get dates, tags and links. The cost is one full re-embedding. On Qdrant, older points keep no metadata (and lose their journal date on read) until their note changes. Qdrant also refuses filtered searches.

The vector store keeps `date` today for the journal date only. It must keep several named dates per note, and the filter must support a range on a named key.

### Today's date

The client sends its local date. A request without it falls back to the server clock. The prompt gives the model the date. When a range is not clear, the model prefers the previous seven full days ending yesterday, and states the range it used.

### Disclosure

- Citations mark claims from the notes, as now.
- The prompt requires the label "Not from your notes:" on passages from the model's own knowledge.
- When the loop ends with no relevant chunk, the server adds a fixed notice that no notes matched, so the whole answer reads as model-only.
- When the final answer cites none of the notes, the server adds a second fixed notice, `NO_CITATION_NOTICE`, because the first search always keeps at least one chunk.

### Trail

Each search and each read becomes one plain-words step, for example "Searching journal entries from Nov 17 to Nov 23". The steps live in the turn. A new streaming endpoint sends each step as it happens, then the final answer with sources and the full trail. The existing `/api/answer` stays, runs the same loop, and returns the full trail in its JSON body. The glossary entry is **Trail** in `CONTEXT.md`.

### Tool calling and the fallback

The LLM client moves from "prompt in, text out" to a message and tools exchange, written once for each wire: OpenAI-compatible, Anthropic, Gemini. If an endpoint rejects tools, the server answers in a single pass as it does today. If the model returns malformed calls twice in a row, the server also answers in a single pass. The trail records that the loop was skipped.

If the single-pass answer fails after the loop gathered more chunks than the first search, the server tries once more. The retry uses only the first-search chunks, with the same numbers. The trail records the retry. If the retry also fails, the request fails with that error.

### Limits

- One turn runs at most 4 tool calls.
- One answer has at most 5 tool rounds.
- One call to the model fails after 10 minutes.
- A whole answer has no time limit. One answer can make up to 8 model calls. It can therefore take longer than 10 minutes in total. If the app stops waiting, the server cancels a streamed answer. A polled answer keeps running until it finishes.

## Delivery order

1. Pushed fields, vector store filter, and the tools, tested without a model.
2. The loop and the tool-calling client on each wire, with the fallback, tested with a fake model that returns scripted calls.
3. Today's date in the request, the disclosure rules, and the trail in the one-shot response.
4. The streaming endpoint.
5. The trail in the TUI, the GUI and the web UI, one at a time.

## Tests

These tests are deterministic and run in CI. They cover these points:

- Date-range filters and named-key filters.
- Tool results and their size cap.
- The step cap and the first automatic search.
- The fallback on rejected tools and on malformed calls.
- The no-match notice and the vault lock.

The model's judgment is not tested. A fixed question set on a real vault comes later, once real trails show which failures matter.

## Left to the builder

- Exact prompt wording, including the "Not from your notes:" label rule.
- The size cap for tool results and the exact step wording.
- The event format of the streaming endpoint.
- How a section is named in `read_note`.
- Whether `related_notes` returns note titles or paths only.

## Calls made without you

These two were my proposals, stated in the conversation. You did not rule on them.


- Tool results are size-capped.
- Every tool is locked to the asking vault's collection.
