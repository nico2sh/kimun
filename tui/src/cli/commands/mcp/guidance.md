<!-- short:start -->
Answer questions about the notes with this method.
1. For a time reference, call `vault_info`, choose a date range, and state the range in the answer.
2. For an open question, try `semantic_search` first. If the tool reports no server, use `search_notes`. For exact words, labels or properties, use `search_notes`.
3. Decide from the result list which notes to read. Read only those. Prefer `get_chunks` for one section of a long note.
4. When a note points to another note, follow it with `get_outlinks` and `get_backlinks`.
5. Put the note path next to each claim. Say which parts of the answer do not come from the notes.
Dates are `YYYY-MM-DD` and both ends of a range are inclusive. Resolve phrases such as "last week" yourself, because the tools never parse them. When a phrase has more than one reading, such as "last week" or "last month", say which one you chose and name the other. If a reading gives a thin answer, search the other reading too. Say what the other reading adds only if you searched it, and if you did not, say so. A date range needs `date_key`.
Dates live in more than one place. The `journal` key is the day of a journal entry. Other kinds of notes, such as meetings or projects, have their own date keys. When the question names a kind of note, search for that kind and use its date key. If the journal gives a thin answer, try the other keys that `vault_info` lists. A date range finds only notes that carry that date key, so for a topic in a period also search the topic itself.
<!-- short:end -->

# Answering questions about Kimün notes

Use this method to answer questions about the notes in a Kimün vault. It works with the Kimün MCP tools. It also works with the `kimun` command line.

## Method

1. For a time reference, call `vault_info`. It reports today's date and the date keys you can use. Choose a date range and state the range in your answer.
2. For an open question, try `semantic_search` first. If the tool reports that no server is set up, use `search_notes`. For exact words, labels or properties, use `search_notes`.
3. Decide from the result list which notes to read. Read only those notes. Use `get_chunks` with a `section` heading name to read one section of a long note. Pass an empty `section` to list its headings first. Use `show_note` to read a whole note.
4. When a note points to another note, follow the link. Use `get_outlinks` for the notes it links to. Use `get_backlinks` for the notes that link to it.
5. Put the note path next to each claim in your answer. Say which parts of the answer do not come from the notes.

## Rules for dates

- Write dates as `YYYY-MM-DD`. Both ends of a range are inclusive.
- Resolve phrases such as "last week" or "in March" yourself. Use the date from `vault_info`. The tools never parse these phrases.
- When a phrase has more than one reading, say which one you chose and name the other. For example, "last week" can mean the seven days before today or the previous calendar week.
- If a reading gives a thin answer, search the other reading too. Say what the other reading adds only if you searched it. If you did not search it, say so.
- A date range needs `date_key`. Use a key that `vault_info` lists, such as `journal`.
- A date range finds only notes that carry that date key. For a question about a topic in a period, also search the topic itself.
- Dates live in more than one place. The `journal` key is the day of a journal entry. Other kinds of notes, such as meetings or projects, have their own date property.
- When the question names a kind of note, search for that kind by tag, folder or meaning. Use the date key that those notes carry.
- If the journal gives a thin answer, try the other keys that `vault_info` lists.

## Stale index

Search results come from an index. If someone edited a note outside Kimün, the index can be stale. When a result looks wrong or old, tell the user. Ask the user to run `kimun workspace reindex`.

## When the server is missing

Once per session, tell the user about the server. Do this in one sentence. Do it only when it matters: you tried `semantic_search`, or the question is about meaning. Say what the tool reported. Give the command from the hint.

Never delay the answer for this. Never change the answer for this. Answer with `search_notes` and tell the user in the same reply.

For questions about exact words, tags, links or dates, say nothing about the server.

Never run the command yourself. Tell the user to run it.

If `vault_info` or `semantic_search` does not exist, this Kimün build is older than this method. Tell the user to update Kimün. Use `search_notes` with the query syntax meanwhile.

## Recipes

Each recipe has a task and a tool sequence.

### Research a topic

Task: collect what the notes say about a topic and write a short summary.

1. Call `semantic_search` with the topic. If it reports no server, call `search_notes` with the topic.
2. Pick the notes that match best. Read them with `get_chunks` or `show_note`.
3. Follow the links with `get_outlinks` and `get_backlinks` for notes that add to the topic.
4. Write the summary. Cite the note paths.

### Explain one note

Task: explain one note and place it among the other notes.

1. Read the note with `show_note`.
2. Call `get_outlinks` and `get_backlinks` for the note.
3. Call `semantic_search` with the main idea of the note. If it reports no server, call `search_notes` with its key terms.
4. Read the most relevant results. Explain how they relate to the note. Cite the note paths.

### Find connections

Task: find links between two topics or two notes that the notes do not state yet.

1. Call `semantic_search` for each topic. If it reports no server, call `search_notes` for each topic.
2. Compare the two result lists. Note the notes that appear in both.
3. Call `get_outlinks` and `get_backlinks` on the shared notes. Read the sections that connect the topics with `get_chunks`.
4. Report each connection with the note paths that support it.

### Suggest links

Task: suggest links to add to a note. A suggestion is a note that is close in meaning and is not linked yet.

1. Call `get_outlinks` and `get_backlinks` for the note. Keep both lists.
2. Call `semantic_search` with the main idea of the note.
3. Remove the note itself, the notes it links to, and the notes that link to it.
4. Read the notes that remain. Suggest the ones that fit, and say why. Cite the note paths.

### Brainstorm

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
