+++
title = "CLI Skill"
weight = 21
+++

# CLI Skill

Kimün includes a skill for AI coding assistants. A skill is a short reference file. It tells an AI agent which CLI commands exist and how to use them correctly. The agent can then manage your notes for you without guessing.

With the skill installed, an AI assistant can do these tasks:

- Create, append, overwrite, replace text in, and delete notes
- Log entries to your journal during a session
- Search your vault for context before it starts a task
- Read specific notes when you ask

Kimün [backs up destructive edits automatically](@/using-kimun/cli.md#backups), so you can undo the mistakes of an agent.

## Installation

### Claude Code

Copy the skill to your Claude skills directory:

```sh
cp -r skills/kimun-cli ~/.claude/skills
```

Claude Code loads the skill automatically. No other configuration is necessary. In any session, Claude can run `kimun note create`, `kimun note append`, `kimun journal`, `kimun search`, and related commands for you.

### Other AI tools

Copy `skills/kimun-cli/SKILL.md` to the place where your tool loads skills. Then follow the skill installation instructions of that tool.

| Tool | Skills directory |
|------|-----------------|
| Claude Code | `~/.claude/skills/` |
| Codex | `~/.agents/skills/` |
| Gemini CLI | Check your `GEMINI.md` or tool documentation |

## What the skill teaches

The skill covers the CLI commands that help with automation:

- Write commands: `note create`, `note append`, `note overwrite`, `note replace`, `note delete`, and `journal`. This includes stdin piping, `--force` for destructive operations, and the rules for path resolution.
- Search: the query syntax (the `@`, `>`, and `-` modifiers) and the output formats (`--format json`, `--format paths`).
- Read: `note show`, and `notes` listing with JSON output.
- Common patterns: recipes for logging findings, for capturing command output, and for searching for context.

The skill also documents behavior that an agent must know. `create` fails if the note exists, but `append` is always safe. `overwrite` and `delete` require `--force`. `replace` needs a unique match, or `--all`. Kimün backs up every destructive edit first.

## Example session

After you install the skill, you can ask Claude Code questions like these:

> "Log today's standup notes to my journal"

> "Search my notes for anything about authentication and summarize what you find"

> "Create a note in inbox/ with the output of this command"

Claude runs the kimun CLI directly. The notes go into your vault as plain Markdown files.

## The skill file

The skill is at `skills/kimun-cli/SKILL.md` in this repository. It follows the [agentskills specification](https://agentskills.io/specification), and it works with any tool that supports that format.
