+++
title = "Piping Output"
weight = 2
+++

# Piping Output

Kimun's CLI output is designed to work seamlessly with Unix pipes and other command-line tools. This guide covers common patterns for combining kimun with tools like `less`, `bat`, `fzf`, and more.

## Basic piping

### Pipe search results into `kimun show`

Find a note and display it directly:

```sh
# Find a note and display it
kimun search "standup" | head -1 | kimun show

# Or use the path directly
kimun show journal/2024-01-15
```

`kimun show` accepts a path via stdin (one path per line) or as an argument.

## Viewing output

### Pipe into a pager

Display search results or note content with pagination:

```sh
kimun show journal/2024-01-15 | less
```

For syntax-highlighted viewing (requires `bat` to be installed):

```sh
kimun show journal/2024-01-15 | bat
```

Combine JSON output with a pager:

```sh
kimun search "project" --format json | jq '.' | less
```

## Interactive selection with `fzf`

[`fzf`](https://github.com/junegunn/fzf) is a command-line fuzzy finder that pairs perfectly with kimun for interactive note selection.

### Interactively pick a note and display it

```sh
# Pick from all notes
kimun notes | fzf | kimun show

# Pick from search results
kimun search "meeting" | fzf | kimun show
```

### Preview note content while selecting

Use fzf's `--preview` option to show note content:

```sh
kimun notes | fzf --preview 'kimun show {}' | kimun show
```

## Shell aliases and functions

Add these to your `~/.zshrc` or `~/.bashrc` for quick access:

### Quick note picker

```sh
# Pick from all notes
alias kn='kimun notes | fzf | kimun show'
```

### Search with preview

```sh
# Search and preview results
ks() {
  kimun search "$1" | fzf --preview 'kimun show {}' | kimun show
}

# Usage: ks "query"
```

### Open most recently modified note

```sh
# Show the most recently changed note
alias klast='kimun notes --format json | jq -r ".notes | sort_by(.modified) | last | .path" | kimun show'
```

## Tips

- Pipes work with both plain text and JSON output
- Use `--format json` with tools like `jq` for advanced filtering
- Combine multiple pipes to build complex workflows
- Test piped commands without committing them to aliases first
