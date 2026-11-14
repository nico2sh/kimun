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

## Set up

Install the server with the script:

```sh
curl -fsSL https://kimun.2co.dev/install-server.sh | sh
```

It works on Linux and on macOS with Apple silicon. For Docker, Windows, and other installs, read the [server README](https://github.com/nico2sh/kimun/tree/main/server).

Then tell Kimün where the server is. Open Preferences, go to Server, and enter the address. The default local address is `http://localhost:7573`. To turn the feature off, leave the address empty.

The footer shows the connection: `rag: online`, `rag: syncing`, `rag: offline`, or `rag: not configured`.

If you share the server with other people, or open it to the network, read the security notes in the server README first. The server uses plain HTTP, so it is meant for a trusted network.
