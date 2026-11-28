---
name: kimun-wiki
description: Use when the vault is a wiki and the user asks to save a source, update the wiki, answer from it, or check it
---

# Kimün wiki

A wiki is a set of notes that you write and keep up to date. The user adds sources, which are raw notes such as saved web pages. You read each source and write wiki pages from it. Kimün protects the sources and checks the facts. You do the work that needs judgment.

Each step below names the MCP tool first and the command second. Use whichever your app has. The two give the same result.

## Start

1. Call `vault_info`. Read the lines that start with `wiki`.
2. If it says `wiki: not declared`, the vault is not a wiki. Tell the user to run `kimun wiki init`, and stop. Do not run it unasked.
3. If it says `wiki: declaration error`, tell the user the message, and stop.
4. Otherwise note the folders: `wiki sources`, `wiki pages`, `wiki log`, `wiki schema`, and `wiki required properties`.
5. Read the schema note with `show_note`. It holds the rules of this wiki, and it can differ from this skill. When they differ, follow the schema note.

Do not change the schema note unless the user asks.

## Ingest a source

1. Get the source into the sources folder.
   - For a web page, call `wiki_clip` with the https address (`kimun wiki clip URL`). It saves the page and gives the path. See "Save a web page".
   - For text the user gave you, call `create_note` in the sources folder. Then call `wiki_stamp` (`kimun wiki stamp`). It adds `content_hash` and `ingested_at`.
2. Read the whole source with `show_note`. If the source is long, list its headings with `get_chunks` and an empty `section`. Then read it one section at a time.
3. Find the pages that the source touches. Use `wiki_catalogue` (`kimun wiki catalogue`) and `search_notes`.
4. Update those pages. If a concept has no page, create one in the wiki folder.
5. On every page that you touch, set the required properties. A page needs `summary`, `sources`, and `updated` unless the declaration says otherwise.
   - `summary` is one short sentence.
   - `sources` lists the source notes as wikilinks. Quote each link, as in `["[[a-source]]"]`.
   - `updated` is today's date, as `YYYY-MM-DD`. `vault_info` gives the date.
6. Link each new page from a parent page, so that it is not an orphan.
7. Add one entry at the end of the log note with `append_note`. The entry has the date, the source, the pages you touched, and the main finding. Write the pages as wikilinks. A link from the log also keeps a top-level page from being an orphan.
8. Call `wiki_check` and fix what it names. See "Check the wiki".

With the command line, text that starts with `---` must come from standard input, as in `printf '...' | kimun note create PATH`. The command reads such text as a flag otherwise. To list a folder, use `kimun notes --path /sources/`, with the slashes.

Give each wiki page a name that differs from every source name. If a page and a source share a name, the link to it is ambiguous, and the check cannot find stale pages for that source.

## Save a web page

`wiki_clip` saves one page as a source note. The note name is the date and the title. The note has the properties `source_url`, `ingested_at`, `content_type`, and `content_hash`, and its first heading is the page title.

- It takes https addresses only. It refuses addresses on private or local networks, such as `localhost` and `192.168.1.5`.
- It takes HTML, Markdown, and plain text. It refuses PDF files and other kinds.
- The page can have at most 20 MB, and an HTML page at most 2 MB.
- It does not run JavaScript. A page that needs a login, or that builds its text with a script, gives little or no content.

If a clip fails, tell the user the message and the reason. Do not try the same address again. If the page needs a login or a script, ask the user to paste the text, and save it as a source with `create_note`.

A second clip of the same address makes a second note with the ending `_0`. Before you clip, call `list_notes` on the sources folder. If the page is already there, use that note.

## Never edit a source

Kimün refuses to overwrite, replace, append to, move, rename, or delete a note in the sources folder. The message names the note and `.kimun/wiki.toml`. If you see it, do not look for a way around it. Tell the user. If a source is wrong, save a corrected copy as a new source and say which one replaces the old one.

## Answer a question

1. Call `wiki_catalogue`. It gives each page with its title, type, and summary. Choose the pages that fit the question.
2. Read those pages with `show_note`. When you need more, follow their links with `get_outlinks`.
3. Answer from the wiki. Put the page path next to each claim.
4. If the wiki is silent or unclear, read the sources that the pages list. Say which part of the answer comes from a source and not from a page.
5. If you find a gap or a mistake, tell the user, and offer to fix the wiki.

For the general method to search and read notes, use the `kimun-ask` skill.

## Check the wiki

Call `wiki_check` (`kimun wiki check`). Each line gives the level, the rule, the note, and a message. If you changed files by hand and not through Kimün, run `kimun workspace reindex` first.

| Rule | Fix |
| --- | --- |
| `orphan` | Link the page from a parent page, or ask the user whether to remove it. |
| `unresolved_link` | Create the missing page, or fix the link. |
| `ambiguous_link` | Rename a note, or write the full path in the link, as in `[[/sources/interview]]`. A full path also works in the `sources` property. |
| `missing_property` | Add the property to the page. |
| `missing_source` | Fix the item in `sources`, or save the source. |
| `source_modified` | A source changed after it was saved. Tell the user. Read the new text and update the pages that cite it. |
| `stale_page` | A source is newer than the page. Read the source, update the page, and set `updated` to today. |
| `unstamped` | This is information only. Call `wiki_stamp` for a source that you trust. |

The check cannot find these problems. When you check, look for these:

- Two pages that contradict each other.
- A claim that the cited sources do not support.
- A claim that is out of date.
- A topic that the sources cover and the wiki covers too thinly.

Fix what you can. Report the rest to the user, with the page paths. Add an entry to the log for each check that led to changes.

## Report

At the end of each task, tell the user which pages you created or changed, which sources you saved, and which problems are still open.
