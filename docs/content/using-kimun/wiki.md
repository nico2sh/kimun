+++
title = "Wiki"
weight = 24
+++

# Wiki

> This feature is experimental. It can change between releases.

A wiki is a set of notes that an AI agent writes and keeps up to date for you. The idea comes from a method that Andrej Karpathy described. You collect raw sources, such as web pages and articles. The agent reads each source and writes wiki pages that sum it up. Over time the pages link to each other and form a body of knowledge about your topics.

The method has three parts:

- Sources are raw notes. Nobody changes them after they are saved. They are the ground truth.
- Wiki pages are notes that the agent writes and updates. Each page says which sources it draws on.
- The schema note is a normal note with the rules. You and the agent both follow it.

Kimün does the parts that need no judgment. It saves web pages as sources, protects the sources, and checks the wiki for facts that are wrong. The agent does the parts that need judgment, such as writing pages. Kimün makes no call to an AI model for the wiki.

A vault that is not a wiki does not change. Kimün treats a vault as a wiki only after you set it up.

## Set up

Run this command in a workspace:

```sh
kimun wiki init
```

The command creates these items:

- The file `.kimun/wiki.toml`. It declares the vault as a wiki. It is hidden, so it never appears as a note.
- The folder `sources`, for raw sources.
- The folder `wiki`, for the pages.
- The note `wiki/schema.md`, with starter rules.
- The note `wiki/log.md`, where the agent writes one entry for each piece of work.

The command prints one line for each item. The word `created` means that it made the item. The word `kept` means that the item was already there. The command never overwrites anything, so you can run it again at any time.

To use other folder names, add the flags:

```sh
kimun wiki init --sources raw --wiki notes/wiki
```

The log note and the schema note go inside the wiki folder.

Edit `wiki/schema.md` to fit your needs. It tells the agent how to name pages, which properties to set, and how to ingest a source. The AI agent reads it before it writes.

## The declaration file

The file `.kimun/wiki.toml` holds the rules that Kimün can check. This is the default file:

```toml
sources = "/sources"
wiki = "/wiki"
log = "/wiki/log.md"
schema = "/wiki/schema.md"
required_properties = [
    "summary",
    "sources",
    "updated",
]
```

The keys have these meanings:

| Key | Meaning |
| --- | --- |
| `sources` | The folder of raw sources. Kimün protects this folder. |
| `wiki` | The folder of wiki pages. |
| `log` | The note where the agent logs its work. |
| `schema` | The note that holds the prose rules. |
| `required_properties` | The properties that every wiki page must have. |

Each rule has a limit. The sources folder and the wiki folder must be different, and one must not contain the other. Neither can be the vault root. The log note and the schema note must be different notes, and neither can be inside the sources folder.

An unknown key is an error, so a typo never turns a rule off silently. If the file has a mistake, the commands tell you what to change.

## Sources are protected

When an automated tool changes a note in the sources folder, Kimün refuses. This holds for the `kimun` commands and for the MCP server. These actions on an existing source fail:

- Overwrite or replace text
- Append
- Move or rename
- Delete

This is the message:

```
Error: /sources/bees.md is in the sources folder that .kimun/wiki.toml protects. Automated edits cannot change it.
```

The command exits with code 2. Creating a new source is allowed. You can still edit a source yourself in the editor, because the rule guards against a mistake of the agent and not against you.

## Save a web page

```sh
kimun wiki clip https://example.com/article
```

The command fetches the page, keeps the main content, converts it to Markdown, and saves it as a new source. It prints the path, the number of words that it saved, and the title:

```
Saved /sources/2026-11-28-example-domain (28 words)
Title: Example Domain
```

If the page has 100 words or fewer, the command adds a line that starts with `Warning`. A login page or a block page that a site sends as a normal page is usually this short. Open the note and make sure that it has the text you wanted.

The note name is the date and a short form of the title. The note starts with these properties:

| Property | Value |
| --- | --- |
| `source_url` | The address you gave. |
| `ingested_at` | The date of the clip. |
| `content_type` | The kind of content, such as `text/html`. |
| `content_hash` | A fingerprint of the text. |

The first heading of the note is the title of the page. For a Markdown page, the title is the `title` of its front matter block, or else its first heading, or else the file name of the address. For a plain text page, the title is the `title` of a front matter block, if the text starts with one. Otherwise it is the file name of the address, such as `rfc2616.txt`. If the address has no file name, the title is the host. Kimün removes a front matter block from the saved text of both kinds, because the note has its own properties.

If you clip the same address again, Kimün saves a second note with the ending `_0`. It does not check for duplicates. Look at the sources folder first.

### What the command accepts

The command protects your computer, because an AI agent can choose the address. These limits apply:

- The address must start with `https`. Every redirect must lead to `https` too. A page can redirect at most five times.
- The address must be on the public internet. Addresses on a private or local network are refused, for example `localhost` or `192.168.1.5`. Kimün checks every connection, so a redirect cannot lead to a private address.
- The page can have at most 20 MB. An HTML page can have at most 2 MB of source text.
- Loading must finish in about a minute. The conversion has its own time limit.
- The page must be HTML, Markdown, or plain text. Kimün saves Markdown and plain text as they are. Other kinds, such as PDF files, are refused.
- Kimün reads the page as UTF-8. A page in another encoding can show wrong characters.
- A page that needs a login, or that builds its text with JavaScript, gives little or no content. Kimün does not run JavaScript.

Kimün does not use a proxy from your environment.

When a clip fails, the command exits with code 2 and gives one of these messages:

| Message | Cause |
| --- | --- |
| `Only https addresses can be clipped: ...` | The address starts with `http` or another scheme. |
| `The host ... is on a private or local network and is not allowed.` | The address is not public. |
| `This is not a usable web address: ...` | The address does not parse. |
| `This vault has no wiki declaration. Run kimun wiki init.` | You did not set up the wiki. |
| `The page is larger than the limit of N MB.` | The page is over a size limit. |
| `The page took too long to load.` | A time limit passed. |
| `The address redirected too many times.` | The page redirected more than five times. |
| `The server answered with status N.` | The server gave an error, such as 404. For 401, 403, 429, and 451, the message adds that some sites refuse automated requests. |
| `This kind of content cannot be clipped: ...` | The page is not HTML, Markdown, or plain text. |
| `The page has no readable content.` | Kimün found no text. |
| `The network request failed: ...` | The connection failed. |

## Check the wiki

```sh
kimun wiki check
```

The command reads the index and checks the notes against the rules. It never calls an AI model, so it is fast and gives the same result on every computer. If you changed notes outside Kimün, run `kimun workspace reindex` first.

Each finding is one line. It has the level, the rule, the note, and a message:

```
problem  unresolved_link  /wiki/bees  The link "nowhere" points to no note.
info  unstamped  /sources/bees  The source has no content_hash, so a change to it cannot be found.
7 problems, 1 info, 4 notes checked.
```

A vault without a wiki declaration gets the first three rules. A wiki gets all of them.

| Rule | What it finds | Level |
| --- | --- | --- |
| `orphan` | A note that no other note links to. | problem |
| `unresolved_link` | A link that points to no note. This is also a page that is missing. | problem |
| `ambiguous_link` | A link that matches more than one note. | problem |
| `missing_property` | A wiki page that lacks a required property. | problem |
| `missing_source` | An item in the `sources` property of a page that names no note. | problem |
| `source_modified` | A source whose text no longer matches its `content_hash`. | problem |
| `stale_page` | A page with an `updated` date before the `ingested_at` date of a source that it cites. The same day is not stale. | problem |
| `unstamped` | A source with no `content_hash`, so Kimün has nothing to compare. | info |

Give sources and wiki pages different names. If `/sources/bees` and `/wiki/bees` both exist, the link `[[bees]]` is ambiguous. Then the stale check cannot use that source.

If the command finds a problem, it exits with code 1. An `info` finding alone gives code 0. To get the findings as data, add `--format json`:

```sh
kimun wiki check --format json
```

The JSON has the fields `declared`, `notes_checked`, and `findings`. Each finding has `rule`, `severity`, `path`, and `message`.

To list only the notes that have a finding, add `--format paths`. The command prints each path once, one on each line, and prints nothing when the wiki is clean. The exit code stays the same.

Kimün cannot find contradictions, claims that are out of date, or topics that the wiki covers too thinly. These need judgment. The AI agent checks them with the [skill](@/using-kimun/ai-skills.md).

## Stamp sources

When a source has a `content_hash`, it has a baseline. The command `wiki clip` writes the hash for you. A source that you or a sync tool created has none. Run this command to add the properties:

```sh
kimun wiki stamp
```

The command writes `content_hash` and `ingested_at` into every source that lacks a hash, and prints the path of each one. It uses the current text as the baseline and today as the date. If it cannot stamp a source, it prints that source and exits with code 1. When all sources have a hash, the command prints `Nothing to stamp.`

This is the one write that protection allows on an existing source.

## The catalogue

```sh
kimun wiki catalogue
```

The command lists each wiki page with its path, title, type, and summary. The type comes from the `type` property and the summary from the `summary` property. The list shows at most 200 pages. To change the limit, add `--limit 50`. When there are more pages, the last line says how many. For data, add `--format json`.

## Run the check on a schedule

Kimün has no scheduler, because the app is not always running. Use the scheduler of your system, or the scheduler of your AI agent.

On Linux and macOS, add a line to your crontab with `crontab -e`. This line checks the wiki every Monday at 8:00 and saves the result:

```sh
0 8 * * 1 kimun wiki check --format json > "$HOME/wiki-report.json"
```

Cron uses a short path. If it cannot find `kimun`, write the full path of the program, which `which kimun` shows. The command uses the current workspace. When there are problems, the exit code is 1. Use it to run a command that tells you:

```sh
0 8 * * 1 kimun wiki check > "$HOME/wiki-report.txt" || notify-send "The wiki has problems"
```

With an AI agent that has a scheduler, give it a task such as this one:

> Every Monday, call `wiki_check`. Fix each problem that it names. Then tell me what you fixed and what is still open.

## In the desktop app

The desktop app sets up a wiki and changes its folders. It has no screen for the other wiki commands.

To set up a wiki, open Preferences and choose Wiki. The section comes after Indexing. It shows five fields:

- Sources folder
- Wiki folder
- Log note
- Schema note
- Required properties

The fields start with the defaults of `kimun wiki init`. Separate the required properties with commas. If you change the wiki folder, the log note and the schema note follow it, unless you edited them.

If the vault is not a wiki, the button reads `Set up wiki`. Click it to create the declaration, the folders, and the two notes. Below the button, one line for each folder and note says `Created` or `Kept`.

If the vault is a wiki, the fields show the saved declaration and the button reads `Apply`. Click it to save your changes. Apply creates what is missing and never moves a note. The old folders and their notes stay where they are.

If you change the sources folder, the section tells you before you click. The old folder is no longer protected, and the new folder is. If the schema note exists, Apply does not change its text. Edit it so that it names the new layout.

If the file `.kimun/wiki.toml` has a mistake, the section says so. It shows the words of the file reader in a box with a monospace font, so the pointer under the faulty text lines up. Fix the fields and click `Set up wiki`. The button replaces the file.

If the vault already has a folder with the name of the sources folder, Kimün uses it as the sources folder. From then on, Kimün protects it from automated edits.

The section acts at once. Save and Cancel do not apply to it, because the declaration is stored with your notes.

To set up a wiki with the defaults, open the command palette and run `Set up wiki`. The palette tells you the result in one line:

- `Wiki set up: created ...` lists the items that it made.
- `This vault is already a wiki. Change its folders in Preferences, Wiki.` means that nothing was missing.
- `The wiki declaration has a mistake. Fix it in Preferences, Wiki.` means that the file needs a repair.

If you deleted the wiki folder by accident, open Preferences, Wiki and click `Apply`. You can also run `Set up wiki` in the palette. Both make the folder again.

## Work with an AI agent

The [MCP server](@/using-kimun/ai-mcp-server.md) has one tool for each wiki command: `wiki_init`, `wiki_check`, `wiki_catalogue`, `wiki_stamp`, and `wiki_clip`. The tool `vault_info` tells the agent whether the vault is a wiki and where the folders are. The tools give the same text as the commands.

The `kimun-wiki` [skill](@/using-kimun/ai-skills.md) teaches the agent the method: how to ingest a source, how to answer a question from the wiki, and how to check it.
