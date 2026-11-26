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

Run from the repo root, or pass the three paths: guidance skill server.
Exits 1 and says what differs.
"""

from __future__ import annotations
import re
import sys
from pathlib import Path

GUIDANCE = "tui/src/cli/commands/mcp/guidance.md"
SKILL = "skills/kimun-ask/SKILL.md"
SERVER = "tui/src/cli/commands/mcp/mod.rs"

SHORT = re.compile(r"<!-- short:start -->\n?(.*?)\n?<!-- short:end -->", re.S)
INCLUDE = 'include_str!("guidance.md")'


def skill_body(text: str) -> str:
    """The text after the frontmatter."""
    m = re.match(r"---\n.*?\n---\n", text, re.S)
    return text[m.end():] if m else text


def problems(guidance: str, skill: str, server: str) -> list[str]:
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
    return found


def main(argv: list[str]) -> int:
    paths = argv[1:4] if len(argv) >= 4 else [GUIDANCE, SKILL, SERVER]
    try:
        texts = [Path(p).read_text() for p in paths]
    except OSError as e:
        print(f"check-agent-guidance: {e}")
        return 1
    found = problems(*texts)
    for line in found:
        print(f"check-agent-guidance: {line}")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
