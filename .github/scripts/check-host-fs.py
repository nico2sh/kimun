#!/usr/bin/env python3
"""Enforce where filesystem operations may live.

Two rules, both stated in CLAUDE.md:

  core  — direct `std::fs`/`tokio::fs` calls belong in `core/src/nfs` (vault-scoped,
          addressed by VaultPath) or `core/src/system` (host-scoped). Nowhere else.
  tui   — no filesystem operation whose behaviour differs by platform, or that
          carries an invariant (atomicity, sidecars, permissions). Those go through
          `kimun_core::system`. Plain reads are fine: `read_to_string` behaves the
          same everywhere, and wrapping it would buy ceremony rather than a rule.

Test code is exempt: a test creating and deleting its own `TempDir` fixtures is
not the thing these rules are about, and routing fixtures through the app's own
abstractions makes tests confirm themselves.

A floor, not a proof — `OpenOptions::new().write(true).truncate(true)` passes it.
Run from the repo root; exits non-zero and prints every offending line.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# Operations that differ by platform or carry an invariant.
GUARDED = [
    "rename",
    "remove_file",
    "remove_dir",
    "remove_dir_all",
    "create_dir",
    "create_dir_all",
    "copy",
    "set_permissions",
    "canonicalize",
    "write",
]

TUI_PATTERN = re.compile(
    r"(?:std|tokio)::fs::(?:" + "|".join(GUARDED) + r")\b"
    r"|(?<![\w:])fs::(?:" + "|".join(GUARDED) + r")\("
    r"|\.canonicalize\(\)"
)

# Any direct filesystem call at all, for the core rule.
CORE_PATTERN = re.compile(r"(?:std|tokio)::fs::\w+|(?<![\w:])fs::\w+\(")

CORE_ALLOWED = ("core/src/nfs/", "core/src/system/")

COMMENT = re.compile(r"^\s*//")


def strip_test_code(source: str) -> list[tuple[int, str]]:
    """Lines outside `#[cfg(test)]` blocks, as (1-based line number, text)."""
    kept: list[tuple[int, str]] = []
    lines = source.split("\n")
    i = 0
    while i < len(lines):
        if lines[i].lstrip().startswith("#[cfg(test)]"):
            i += 1
            depth = 0
            opened = False
            while i < len(lines):
                depth += lines[i].count("{") - lines[i].count("}")
                if "{" in lines[i]:
                    opened = True
                i += 1
                if opened and depth <= 0:
                    break
            continue
        kept.append((i + 1, lines[i]))
        i += 1
    return kept


def offenders(root: Path, pattern: re.Pattern[str], skip: tuple[str, ...] = ()) -> list[str]:
    found = []
    for path in sorted(root.rglob("*.rs")):
        rel = path.as_posix()
        if any(rel.startswith(prefix) for prefix in skip):
            continue
        source = path.read_text(encoding="utf-8")
        # A whole module gated with an inner attribute is test code too.
        if "#![cfg(test)]" in source:
            continue
        for number, line in strip_test_code(source):
            if COMMENT.match(line):
                continue
            if pattern.search(line):
                found.append(f"{rel}:{number}: {line.strip()}")
    return found


def main() -> int:
    failures = 0

    core = offenders(Path("core/src"), CORE_PATTERN, skip=CORE_ALLOWED)
    if core:
        print("::error::core filesystem calls belong in core/src/nfs or core/src/system:")
        print("\n".join(core))
        failures += 1

    tui = offenders(Path("tui/src"), TUI_PATTERN)
    if tui:
        print("::error::these belong in kimun_core::system (or nfs, for vault paths):")
        print("\n".join(tui))
        failures += 1

    if failures == 0:
        print("filesystem operations are where they belong")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
