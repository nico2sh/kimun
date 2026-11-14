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

Install the server with the script. The option `--service` starts the server when you log in and restarts it if it stops:

```sh
curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service
```

It works on Linux and on macOS with Apple silicon. For Docker, Windows, and other installs, read the [server README](https://github.com/nico2sh/kimun/tree/main/server).

Then tell Kimün where the server is. Open Preferences, go to Server, and enter the address. The default local address is `http://localhost:7573`. To turn the feature off, leave the address empty.

To change the server's own settings, such as the model, click `Open server settings` in the same place. It opens the server's page in your browser.

The footer shows the connection: `rag: online`, `rag: syncing`, `rag: offline`, or `rag: not configured`.

If you share the server with other people, or open it to the network, read the security notes in the server README first. The server uses plain HTTP, so it is meant for a trusted network.

## Update the server

When a new server version exists, the desktop app tells you. What happens next depends on how you installed the server.

If you installed it with the script and the `--service` option, and the server runs on your computer, the message has an `Update now` button. Preferences, Server shows the same button. The app downloads the new version, checks its signature, and restarts the server. If the new server does not start, the app puts the old one back and tells you.

For every other server, update it the way you installed it. This includes a server on another computer, a Docker container, and a server that you installed with Cargo. To update a server that you installed with the script, run the same command again. The script restarts the server for you.

If you installed the script version before this feature existed, the button does not appear yet. Run the install command once. After that, the button works.
