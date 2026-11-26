# Agent setup: the server as a soft dependency

Status: design, agreed with the owner in conversation on 2026-11-26. Not built.

## Intent

The agent connector works without a Kimün server. A server with an embedder adds search by meaning (`semantic_search`) and no LLM key is needed. The docs and the tools must say so. A user must be able to set up the MCP server, the skills and the server with a few short commands. The user must also be able to keep them current. When the server is missing or stopped, the agent must tell the user what to do. Nothing starts or installs the server for the user.

## Findings that shaped the design

- The CLI cannot set the server address. `kimun_server_url` and `kimun_server_token` live in the shared `config.toml`. The only way in is Preferences in an app, or a hand edit. There is no way to turn the feature off without editing the file.
- The skills are not in the install scripts or the release archives. The docs say `cp -r skills/kimun-ask ~/.claude/skills`. That fails for a user who has only the binary.
- The skills format is the open Agent Skills standard. The widely used installer is the Skills CLI (`npx skills add owner/repo`, with `update` and `check`). The repository already uses it (`skills-lock.json`, `.agents/skills`). The folder `skills/` at the repository root has the layout that tool expects. This was not run against our repository. Check it first.
- `install-server.sh` writes a marker, `server-install.toml`, next to `server.toml` (ADR 0059). It records the install folder and the service kind: `systemd`, `launchd` or none. The Linux unit is `kimun-server.service`. The macOS label is `dev.2co.kimun-server`.
- The MCP process reads the settings once at start. If an address is set at that time, it starts the sync loop. Otherwise it does not.
- Homebrew and Cargo own their binaries. `kimun update` only notifies on those channels.

## Goals

- One page that shows what works without a server and what a server adds, with the three-step quick start.
- Short commands for each step, and a way back (turn the feature off).
- Tools that report the server state with an exact next step.
- Skills that users can install and update with the standard tool.

## Non-goals

- Starting, installing or restarting the server from the MCP process, a tool or a skill.
- Probing a default address and using a server that nobody configured. The sync then sends the text of the notes to a service that the user did not choose.
- Editing the files that configure other apps.
- A marker, a record of installs, a status command, a start-up check or a refresh after `kimun update` for skills. The Skills CLI covers updates.
- Skills that are stubs reading the method from the binary at use time.

## Decisions

### Server states and hints

- A new state, **Stopped**, is in `CONTEXT.md`. When a marker shows an install here, the configured address points at this machine, and the server does not answer, the state is Stopped. A marker with no service still counts.
- `vault_info` and `semantic_search` report one of these states, each with a hint that names the next step:
  - No server installed and no address (no marker): the install line `curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service`, then `kimun server use http://localhost:7573`. On Windows there is no script, so the hint points to the Docker and WSL2 docs.
  - A marker and no address: `kimun server use http://localhost:7573`.
  - Stopped: the start command for this system. Linux: `systemctl --user start kimun-server.service`. macOS: the `launchctl` command for `dev.2co.kimun-server`. No service: run the binary at the install folder in a terminal, or run the installer again with `--service`.
  - Address set and nothing answers, with no sign of a local install: the server does not answer.
  - The existing states unconfigured, semantic-only and full keep their hints.
- Every hint says that the agent must tell the user and must not run the command. After any change of the address, the hint says to restart the agent app.
- The MCP server never starts the server. This is a change of the user's machine that the user must choose, and a tool call must not make it.

### What the agent says

- Once per session, the agent mentions the server in one sentence. It does this only when it matters: it tried `semantic_search`, or the question is about meaning. The sentence says what the tool reported and gives the command. It never changes or delays the answer. For exact words, tags, links and date questions it says nothing.
- This rule goes in the full skill and in the tool hints. It does not go in the short block of the server instructions.
- The skill carries one line for an older binary: if `vault_info` or `semantic_search` does not exist, this Kimün build is older than the skill. Tell the user to update Kimün and use `search_notes` with the query syntax meanwhile.

### Commands

- `kimun mcp --print-config` prints the JSON block for the MCP settings on standard output, so that pasting or piping it gives valid JSON. The intro sentence and the `claude mcp add` line go to standard error (ruling P6 during the build). When `kimun` on the path is the running binary, both use the bare `kimun`. Otherwise both use the absolute path. `kimun mcp` without the option still runs the server, because agent apps run exactly that.
- `kimun server use <url> [--token T]` saves the address and token, then checks the server and prints the state and hint. Even when the server does not answer, it saves the address. It says to restart the agent app.
- `kimun server use --off` clears the address and the token.
- `kimun server status` prints the same state and hint as `vault_info`.
- `kimun skill install [--only ask|cli] [--dir PATH]` is the fallback for users without Node. It writes both skills from copies built into the binary. The known targets are the Claude Code folder (`~/.claude/skills`) and the Codex folder (`~/.agents/skills`). The Codex path in the docs is unconfirmed, so check it. It refuses to overwrite an existing folder. It never writes through a symlink. `--force` replaces a plain folder and prints what it replaced. It writes no marker and keeps no record.
- The main way to install skills is `npx skills add nico2sh/kimun`, and `npx skills update` to refresh.

### Documentation

- `ai.md` becomes the start page. It opens with a table of what works without a server and what needs one. Then it gives the quick start in three steps.
  1. The MCP server: `claude mcp add kimun -- kimun mcp`, and `kimun mcp --print-config` for other apps.
  2. The skills: `npx skills add nico2sh/kimun`, and `kimun skill install` without Node.
  3. The optional server: the install line, `kimun server use` and `kimun server status`.
- `ai-mcp-server.md` lists the tools, marks `semantic_search` as needing a server, and describes `vault_info`.
- `ai-skills.md` covers the two skills, the standard tool and the fallback.
- `server.md` adds the CLI way to set the address next to Preferences, and the agent use of the server.
- `cli.md` documents the new commands.
- The docs ship with the commands. They must not describe a command that does not exist yet.

## Constraints

- Follow `CLAUDE.md`. Home-folder rules for Windows and Unix, and any write outside the vault, live in `kimun_core::system`. `check-host-fs.py` enforces this.
- The MCP code stays in `tui/src/cli/commands/mcp/`. The `server` and `skill` commands get their own modules under `tui/src/cli/commands/`.
- When the crate is published, `include_str!` cannot reach outside the `tui` crate. The embedded skill text must live inside the crate. The repository `skills/` folder stays for the Skills CLI. A CI check must keep both equal, as `check-agent-guidance.py` does for the guidance.
- The server address is written the way `kimun workspace use` writes its pointer, through the shared settings writer.

## Left to the builder

- The exact hint text and the platform checks.
- The exact flags and their names.
- Where the embedded skill text lives, and how the CI check compares it.
- The `launchctl` command for the start hint.

## Follow-ups

- If the Skills CLI supports it, pin the skills to release tags. Not verified.
- Semantic search in the CLI, and `ask_notes`, from the first spec.
