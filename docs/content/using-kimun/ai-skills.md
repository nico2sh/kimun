+++
title = "CLI Skill"
weight = 21
+++

# CLI Skill

Kimün ships with a skill for AI coding assistants. A skill is a short reference file that tells an AI agent which CLI commands are available and how to use them correctly, so it can manage your notes on your behalf without guessing.

With the skill installed, an AI assistant can:

- Create, append, overwrite, replace text in, and delete notes
- Log entries to your journal as part of a session
- Search your vault for context before starting a task
- Read specific notes on request

Destructive edits are [backed up automatically](@/using-kimun/cli.md#backups), so an agent's mistakes can be undone.

## Installation

### Claude Code

Copy the skill to your Claude skills directory:

```sh
cp -r skills/kimun-cli ~/.claude/skills
```

Claude Code picks it up automatically, with no further configuration. From any session, Claude can invoke `kimun note create`, `kimun note append`, `kimun journal`, `kimun search`, and related commands on your behalf.

### Other AI tools

Copy `skills/kimun-cli/SKILL.md` to wherever your tool loads skills from and follow that tool's skill installation instructions.

| Tool | Skills directory |
|------|-----------------|
| Claude Code | `~/.claude/skills/` |
| Codex | `~/.agents/skills/` |
| Gemini CLI | Check your `GEMINI.md` or tool documentation |

## What the skill teaches

The skill covers the CLI commands that are useful for automation:

- **Write commands**: `note create`, `note append`, `note overwrite`, `note replace`, `note delete`, and `journal`, including stdin piping, `--force` for destructive operations, and path resolution rules
- **Search**: query syntax (`@`, `>`, `-` modifiers) and output formats (`--format json`, `--format paths`)
- **Read**: `note show` and `notes` listing with JSON output
- **Common patterns**: recipes for logging findings, capturing command output, and searching for context

It also documents behavior an agent needs to know. For example, `create` fails if a note exists while `append` is always safe, that `overwrite` and `delete` require `--force`, that `replace` needs a unique match (or `--all`), and that every destructive edit is backed up first.

## Example session

Once installed, you can ask Claude Code things like:

> "Log today's standup notes to my journal"

> "Search my notes for anything about authentication and summarize what you find"

> "Create a note in inbox/ with the output of this command"

Claude runs the kimun CLI directly, so the notes end up in your vault as plain Markdown files.

## The skill file

The skill lives at `skills/kimun-cli/SKILL.md` in this repository. It follows the [agentskills specification](https://agentskills.io/specification) and works with any tool that supports that format.
