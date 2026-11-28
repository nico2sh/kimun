#!/usr/bin/env python3
"""Keep the agent guidance in one place.

The single source is tui/src/cli/commands/mcp/guidance.md. Two copies must
follow it:

  skill   — skills/kimun-ask/SKILL.md carries the full text after its
            frontmatter, verbatim.
  server  — the MCP server instructions (tui/src/cli/commands/mcp/mod.rs) must
            build from the short block. The source passes when it includes the
            file with `include_str!("guidance.md")`. A source that holds its own
            copy of the text passes only when the short block appears in it
            unchanged.

The terminal app embeds all three skills for `kimun skill install`. Three more
checks keep the embedded files equal to the repository skills, byte for byte:

  cli     — skills/kimun-cli/SKILL.md equals tui/src/cli/commands/skill/kimun-cli.md.
  ask     — skills/kimun-ask/SKILL.md equals
            tui/src/cli/commands/skill/kimun-ask.frontmatter.md followed by
            guidance.md.
  wiki    — skills/kimun-wiki/SKILL.md equals tui/src/cli/commands/skill/kimun-wiki.md.

Run from the repo root, or pass the eight paths: guidance skill server
cli-skill cli-embedded ask-frontmatter wiki-skill wiki-embedded.
Exits 1 and says what differs.
"""

from __future__ import annotations
import re
import sys
from pathlib import Path

GUIDANCE = "tui/src/cli/commands/mcp/guidance.md"
SKILL = "skills/kimun-ask/SKILL.md"
SERVER = "tui/src/cli/commands/mcp/mod.rs"
CLI_SKILL = "skills/kimun-cli/SKILL.md"
CLI_EMBEDDED = "tui/src/cli/commands/skill/kimun-cli.md"
ASK_FRONTMATTER = "tui/src/cli/commands/skill/kimun-ask.frontmatter.md"
WIKI_SKILL = "skills/kimun-wiki/SKILL.md"
WIKI_EMBEDDED = "tui/src/cli/commands/skill/kimun-wiki.md"
PATHS = [
    GUIDANCE,
    SKILL,
    SERVER,
    CLI_SKILL,
    CLI_EMBEDDED,
    ASK_FRONTMATTER,
    WIKI_SKILL,
    WIKI_EMBEDDED,
]

SHORT = re.compile(r"<!-- short:start -->\n?(.*?)\n?<!-- short:end -->", re.S)
INCLUDE = 'include_str!("guidance.md")'


def skill_body(text: str) -> str:
    """The text after the frontmatter."""
    m = re.match(r"---\n.*?\n---\n", text, re.S)
    return text[m.end():] if m else text


def problems(
    guidance: str,
    skill: str,
    server: str,
    cli_skill: str,
    cli_embedded: str,
    ask_frontmatter: str,
    wiki_skill: str,
    wiki_embedded: str,
) -> list[str]:
    found = []
    short = SHORT.search(guidance)
    if short is None:
        return ["guidance.md has no short block (short:start / short:end)"]
    if skill_body(skill).strip() != guidance.strip():
        found.append("SKILL.md body differs from guidance.md")
    if INCLUDE not in server and short.group(1).strip() not in server:
        found.append(
            "the server instructions neither include guidance.md "
            "nor hold the short block text"
        )
    if cli_skill != cli_embedded:
        found.append("skills/kimun-cli/SKILL.md differs from the embedded kimun-cli.md")
    if skill != ask_frontmatter + guidance:
        found.append(
            "skills/kimun-ask/SKILL.md differs from "
            "kimun-ask.frontmatter.md followed by guidance.md"
        )
    if wiki_skill != wiki_embedded:
        found.append("skills/kimun-wiki/SKILL.md differs from the embedded kimun-wiki.md")
    return found


def main(argv: list[str]) -> int:
    paths = argv[1:9] if len(argv) >= 9 else PATHS
    try:
        # Bytes, decoded without newline translation, so the copies compare exactly.
        texts = [Path(p).read_bytes().decode("utf-8") for p in paths]
    except (OSError, UnicodeDecodeError) as e:
        print(f"check-agent-guidance: {e}")
        return 1
    found = problems(*texts)
    for line in found:
        print(f"check-agent-guidance: {line}")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
