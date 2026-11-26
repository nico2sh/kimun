<!-- short:start -->
Answer questions about the notes with this method.
1. For a time reference, call `vault_info`, choose a date range, and state the range in the answer.
2. For an open question, try `semantic_search` first. If the tool reports no server, use `search_notes`. For exact words, labels or properties, use `search_notes`.
3. Decide from the result list which notes to read. Read only those. Prefer `get_chunks` for one section of a long note.
4. When a note points to another note, follow it with `get_outlinks` and `get_backlinks`.
5. Cite the note paths. Say which parts of the answer do not come from the notes.
Dates are `YYYY-MM-DD` and both ends of a range are inclusive. Resolve phrases such as "last week" yourself, because the tools never parse them. A date range needs `date_key`.
<!-- short:end -->

# Answering questions about Kimün notes

Use this method to answer questions about the notes in a Kimün vault. It works with the Kimün MCP tools. It also works with the `kimun` command line.

## Method

1. For a time reference, call `vault_info`. It reports today's date and the date keys you can use. Choose a date range and state the range in your answer.
2. For an open question, try `semantic_search` first. If the tool reports that no server is set up, use `search_notes`. For exact words, labels or properties, use `search_notes`.
3. Decide from the result list which notes to read. Read only those notes. Use `get_chunks` to read one section of a long note. Use `show_note` to read a whole note.
4. When a note points to another note, follow the link. Use `get_outlinks` for the notes it links to. Use `get_backlinks` for the notes that link to it.
5. Cite the note paths in your answer. Say which parts of the answer do not come from the notes.

## Rules for dates

- Write dates as `YYYY-MM-DD`. Both ends of a range are inclusive.
- Resolve phrases such as "last week" or "in March" yourself. Use the date from `vault_info`. The tools never parse these phrases.
- A date range needs `date_key`. Use a key that `vault_info` lists, such as `journal`.

## Stale index

Search results come from an index. If someone edited a note outside Kimün, the index can be stale. When a result looks wrong or old, tell the user. Ask the user to run `kimun workspace reindex`.

## Recipes

Each recipe has a task and a tool sequence.

### research_topic

Task: collect what the notes say about a topic and write a short summary.

1. Call `semantic_search` with the topic. If it reports no server, call `search_notes` with the topic.
2. Pick the notes that match best. Read them with `get_chunks` or `show_note`.
3. Follow the links with `get_outlinks` and `get_backlinks` for notes that add to the topic.
4. Write the summary. Cite the note paths.

### research_note

Task: explain one note and place it among the other notes.

1. Read the note with `show_note`.
2. Call `get_outlinks` and `get_backlinks` for the note.
3. Call `semantic_search` with the main idea of the note. If it reports no server, call `search_notes` with its key terms.
4. Read the most relevant results. Explain how they relate to the note. Cite the note paths.

### find_connections

Task: find links between two topics or two notes that the notes do not state yet.

1. Call `semantic_search` for each topic. If it reports no server, call `search_notes` for each topic.
2. Compare the two result lists. Note the notes that appear in both.
3. Call `get_outlinks` and `get_backlinks` on the shared notes. Read the sections that connect the topics with `get_chunks`.
4. Report each connection with the note paths that support it.

### link_suggestions

Task: suggest links to add to a note. A suggestion is a note that is close in meaning and is not linked yet.

1. Call `get_outlinks` and `get_backlinks` for the note. Keep both lists.
2. Call `semantic_search` with the main idea of the note.
3. Remove the note itself, the notes it links to, and the notes that link to it.
4. Read the notes that remain. Suggest the ones that fit, and say why. Cite the note paths.

### brainstorm

Task: produce new ideas from the notes on a topic.

1. Call `semantic_search` with the topic. If it reports no server, call `search_notes`.
2. Read the best notes with `get_chunks` or `show_note`.
3. Follow the links with `get_outlinks` and `get_backlinks` for related notes.
4. List new ideas. Mark each idea that comes from a note with the note path. Mark each idea that is your own.

## Command line path

The command line has no semantic search. Use `kimun search` for word search, properties, labels and links. Use `kimun note show` to read a note.

- Word search: `kimun search "query"`. Add `--format paths` for a list of paths.
- Properties: `kimun search "%status=active"`. Compare dates with `%due<2026-05-01`.
- Labels: `kimun search "#label"`. List all labels with `kimun labels`.
- Links: `kimun search "<note"` finds the notes that link to `note`. `kimun search ">note"` finds the notes that `note` links to.
- Read a note: `kimun note show "path/to/note"`.
- Journal by date: `kimun journal show --date YYYY-MM-DD` shows one day. For a month, use a path filter such as `kimun search "/journal/2026-11"`. Use `kimun notes --path "journal/"` to list the journal.
- Date range on a property: put two comparisons in one query, such as `kimun search "%due>=2026-11-01 %due<=2026-11-30"`. The query syntax has no range on the date in a journal note's name. For journal dates, use a folder prefix such as `kimun search "/journal/2026-11"`. For one day, use `kimun journal show --date YYYY-MM-DD`.
- After an edit outside Kimün, run `kimun workspace reindex`.
