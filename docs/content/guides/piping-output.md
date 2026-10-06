+++
title = "Piping Output"
weight = 2
+++

# Piping Output

Kimün's CLI reads from stdin and writes plain text or JSON to stdout, so it works with Unix pipes. This guide shows common ways to combine kimun with tools such as `less`, `bat`, and `fzf`.

## Basic piping

### Pipe search results into `kimun note show`

Find a note and display it:

```sh
# Find a note and display it
kimun search "standup" | head -1 | kimun note show

# Or use the path directly
kimun note show journal/2024-01-15
```

`kimun note show` accepts a path via stdin (one path per line) or as an argument.

## Viewing output

### Pipe into a pager

Page through search results or note content:

```sh
kimun note show journal/2024-01-15 | less
```

For syntax-highlighted viewing (requires `bat` to be installed):

```sh
kimun note show journal/2024-01-15 | bat
```

Combine JSON output with a pager:

```sh
kimun search "project" --format json | jq '.' | less
```

## Interactive selection with `fzf`

[`fzf`](https://github.com/junegunn/fzf) is a command-line fuzzy finder. Use it with kimun to pick notes interactively.

### Interactively pick a note and display it

```sh
# Pick from all notes
kimun notes --format paths | fzf | kimun note show

# Pick from search results
kimun search "meeting" --format paths | fzf | kimun note show
```

### Preview note content while selecting

Use fzf's `--preview` option to show note content:

```sh
kimun notes --format paths | fzf --preview 'kimun note show {}' | kimun note show
```

## Shell aliases and functions

Add these to your `~/.zshrc` or `~/.bashrc`:

### Quick capture

Save a thought from the terminal with one short command:

```sh
# One-letter alias for instant capture
alias q='kimun note quick'

# Usage:
# q "look into caching strategy"
# q "call dentist tomorrow"
# echo "deploy at $(date)" | q
```

### Quick note picker

```sh
# Pick from all notes
alias kn='kimun notes --format paths | fzf | kimun note show'
```

### Search with preview

```sh
# Search and preview results
ks() {
  kimun search "$1" --format paths | fzf --preview 'kimun note show {}' | kimun note show
}

# Usage: ks "query"
```

### Open most recently modified note

```sh
# Show the most recently changed note
alias klast='kimun notes --format json | jq -r ".notes | sort_by(.modified) | last | .path" | kimun note show'
```

### Review inbox

```sh
# List what's in the inbox
alias ki='kimun note triage'
```

## Piping into the journal

When stdin is not a terminal, `kimun journal` appends the piped input to your daily entry. Use this to log command output:

```sh
# Timestamped log line
echo "$(date +%H:%M) — build succeeded" | kimun journal

# Capture command output
./run-tests.sh | tail -1 | kimun journal

# Pipe search results as a journal entry
kimun search "todo" --format paths | kimun journal

# Log to a specific date
echo "Late entry" | kimun journal --date 2024-01-15
```

Run it from cron to log something every day:

```sh
@daily echo "$(hostname): $(uptime)" | kimun journal
```

## Tips

Pipes work with both plain text and JSON output. To filter on specific fields, use `--format json` with `jq` (see [Scripting with JSON](@/guides/scripting.md)). Try a pipeline on the command line before saving it as an alias.
