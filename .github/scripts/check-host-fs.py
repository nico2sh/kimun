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

# Braces that are not code structure. Counting these desynced the depth
# tracking below, and a `#[cfg(test)]` block that never appears to close
# swallows the rest of the file — the gate failing open, silently.
BRACE_NOISE = re.compile(
    r'r#+"[^"]*"#+'  # raw string with hashes
    r'|r"[^"]*"'  # raw string
    r'|"(?:\\.|[^"\\])*"'  # string literal
    r"|'(?:\\.|[^'\\])'"  # char literal
    r"|//.*"  # line comment
)


def brace_delta(line: str) -> int:
    """`{` minus `}`, ignoring literals and comments."""
    code = BRACE_NOISE.sub("", line)
    return code.count("{") - code.count("}")


class UnterminatedTestBlock(Exception):
    """A `#[cfg(test)]` block whose braces never balanced before EOF.

    Raised rather than swallowed: the alternative is to keep stripping to the
    end of the file, which exempts every line after the block from the check
    and reports success. A gate that cannot tell where test code ends has to
    say so.
    """

    def __init__(self, line: int) -> None:
        super().__init__(f"unterminated #[cfg(test)] block opened at line {line}")
        self.line = line


def strip_test_code(source: str) -> list[tuple[int, str]]:
    """Lines outside `#[cfg(test)]` items, as (1-based line number, text).

    The attribute heads one of two shapes, and they end differently:

        #[cfg(test)]        #[cfg(test)]
        mod tests {         mod test_support;
            …
        }

    Whichever of `{` or `;` comes first says which one this is. Assuming the
    braced form swallowed everything up to the end of the *next* braced item,
    so a file opening with `#[cfg(test)] mod test_support;` — `tui/src/main.rs`
    and `tui/src/lib.rs` both do — had its first real function exempted from
    the check.
    """
    kept: list[tuple[int, str]] = []
    lines = source.split("\n")
    i = 0
    while i < len(lines):
        stripped = lines[i].lstrip()
        if not stripped.startswith("#[cfg(test)]"):
            kept.append((i + 1, lines[i]))
            i += 1
            continue
        # Scan for the token that terminates the item, starting after the
        # attribute itself so a one-line `#[cfg(test)] mod tests { … }` counts.
        opened_at = i + 1
        text = BRACE_NOISE.sub("", stripped[len("#[cfg(test)]") :])
        braced = False
        while i < len(lines):
            brace, semi = text.find("{"), text.find(";")
            if brace != -1 and (semi == -1 or brace < semi):
                braced = True
                break
            if semi != -1:
                i += 1
                break
            i += 1
            text = BRACE_NOISE.sub("", lines[i]) if i < len(lines) else ""
        if not braced:
            continue
        depth = 0
        closed = False
        while i < len(lines):
            depth += brace_delta(lines[i])
            i += 1
            if depth <= 0:
                closed = True
                break
        if not closed:
            raise UnterminatedTestBlock(opened_at)
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
        try:
            lines = strip_test_code(source)
        except UnterminatedTestBlock as e:
            # Reported as an offence rather than skipped: everything after the
            # block would otherwise go unchecked and the run would still pass.
            found.append(f"{rel}:{e.line}: {e}")
            continue
        for number, line in lines:
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
