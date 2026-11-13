+++
title = "CLI Skill"
weight = 21
+++

# CLI skill

A skill is a short guide file that teaches an AI coding assistant how to use a tool. The Kimün skill teaches the `kimun` commands. Then the assistant can manage your notes during a session.

With the skill, you can ask:

> "Log today's standup notes to my journal."

> "Search my notes for authentication and summarize what you find."

> "Create a note in inbox/ with the output of this command."

## Install

For Claude Code, copy the skill folder to your skills folder:

```sh
cp -r skills/kimun-cli ~/.claude/skills
```

For another tool, copy `skills/kimun-cli/SKILL.md` to the place where that tool loads skills. For Codex, this is `~/.agents/skills/`.

## Safety

The assistant must add `--force` to overwrite or delete a note. Kimün saves a backup of the old text first. See [Backups](@/using-kimun/cli.md#backups).
