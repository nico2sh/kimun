+++
title = "Semantic Search & Ask (Server)"
weight = 23
+++

# Semantic search and Ask

> This feature is experimental. It can change between releases.

Normal search finds exact words. The optional Kimün server adds two things:

- Semantic search finds notes by meaning. A search for "how do I deploy" also finds a note about the "release procedure".
- Ask answers a question in plain language, using your notes. The answer shows which notes it used.

You do not need the server to use Kimün. When you connect a server, the new views appear by themselves.

The server never reads your files. Kimün sends it the text of your notes. The server can run on your own computer, so your notes stay with you.

## Ask a question

1. Open ASK in the rail, or press `F6`.
2. Type a question and press `Enter`.
3. Read the answer. The markers `[1]`, `[2]` point to the notes that it used. Click a marker to see that note.
4. Type a follow-up question in the same box.

With an answer selected, press `y` to copy it, `e` to save it as a new note, or `r` to ask again. Ask needs a language model (LLM) that you set up in the server. Without one, you get semantic search only.

Ask can also answer questions about time, such as "what did I write last week?". It uses the dates of your notes. After you upgrade the server, Kimün sends your notes to it again once, so the server can store dates, tags and links. This takes a while on a large vault.

## Set up

Install the server with the script. The option `--service` starts the server at login. If the server stops, the option restarts it:

```sh
curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service
```

It works on Linux and on macOS with Apple silicon.

On Windows, there is no install script. Use one of these ways:

- Docker. This is the simplest way. The [server README](https://github.com/nico2sh/kimun/tree/main/server) has the command.
- WSL2 (Linux inside Windows). Open a WSL2 terminal and run the same install command. WSL2 must have systemd turned on for the `--service` option.

Intel Macs have no prebuilt server. Use Docker there. The server README also describes the install with Cargo.

Then tell Kimün where the server is. Open Preferences, go to Server, and enter the address. The default local address is `http://localhost:7573`. To turn the feature off, leave the address empty.

You can also set the address from a terminal:

```sh
kimun server use http://localhost:7573
kimun server use https://kimun.example.com --token SECRET
kimun server use --off
kimun server status
```

- `kimun server use` saves the address. Give `--token` for a server that needs one.
- A token that you type on the command line stays in your shell history. Preferences has no token field. If you share the computer or your shell keeps its history, start the command with a space (when your shell is set to ignore such commands), or remove the line from the history afterwards.
- When you change the address and give no `--token`, Kimün clears the saved token. It keeps the token only for the same address.
- `kimun server use --off` removes the address and the token.
- `kimun server status` shows the state of the server and the next step. It checks the address and not the token. A wrong token shows up only in a failed search, with a message that the server rejected the token.

The command only writes the settings file. It does not install or start the server.

Restart your AI app after you change the address. The MCP server reads the address at its start.

### What the AI tools do with the server

The tool `semantic_search` uses the server. The tool `vault_info` reports its state: `none`, `stopped`, `unreachable`, `unconfigured`, `semantic-only`, or `full`. The state `stopped` means that the install script put the server on this computer and it does not answer. When the server is not ready, the tools tell the AI app the next step, and the AI app passes it to you. The tools never start the server. See [MCP server](@/using-kimun/ai-mcp-server.md#server-state).

To change the server's own settings, such as the model, click `Open server settings` in the same place. It opens the server's page in your browser.

The footer shows the connection: `rag: online`, `rag: syncing`, `rag: offline`, or `rag: not configured`.

If you share the server with other people, or open it to the network, read the security notes in the server README first. The server uses plain HTTP, so it is meant for a trusted network.

## Filter a search

A program can call the server's `POST /api/embeddings` search with filters. A request with no filter works as before. These fields are optional:

- `date_from` and `date_to`: a range of days, written `YYYY-MM-DD`. Give both or neither. The start must not be after the end.
- `date_key`: which date to use, such as `journal` for journal notes. It needs `date_from` and `date_to`.
- `folder`: only notes inside this folder.
- `tags`: only notes that have all of these tags.

The server reports `search_filters: true` on its `/health` page when it accepts these fields. An older server does not report it.

If the server uses Qdrant as its store, it refuses any filter and returns an error. The default SQLite store supports all filters.

## Update the server

When a new server version exists, the desktop app tells you. What happens next depends on how you installed the server.

If you installed it with the script and the `--service` option, and the server runs on your computer, the message has an `Update now` button. Preferences, Server shows the same button. The app downloads the new version, checks its signature, and restarts the server. If the new server does not start, the app puts the old one back and tells you.

For every other server, update it the way you installed it. This includes a server on another computer, a Docker container, a server on Windows, and a server that you installed with Cargo. To update a server that you installed with the script, run the same command again. The script restarts the server for you.

If you installed the script version before this feature existed, the button does not appear yet. Run the install command once. After that, the button works.

Some server updates change how the server stores your notes. After such an update, the server starts with an empty store, and Kimün sends your notes to it again. The server then embeds the notes again, which can take several minutes for a large vault. Until that ends, search and Ask find nothing for that vault. The Jobs page of the server web UI shows the progress. Do not restart the server while a job runs, because a restart stops the job and Kimün has to start it again.
