#!/usr/bin/env python3
"""Hand-written release notes: one file per app version, `release-notes/<version>.md`.

The release page shows that file as it is, instead of the changelog generated from
commit messages (which stays in `tui/CHANGELOG.md` as the commit-level history).

    release_notes.py version             print the app's current version (tui/Cargo.toml)
    release_notes.py scaffold [VERSION]  create the empty notes file, unless it exists
    release_notes.py check [VERSION]     exit 1 while the file is missing or still the template
                                         (prints a CI error saying why)
    release_notes.py ready [VERSION]     the same test, silent: exit 0 when the notes are written
    release_notes.py unreleased [VERSION] exit 0 while no release tag exists for VERSION (so
                                         a release PR that does not release the app needs no notes)
    release_notes.py path [VERSION]      print the file's path

VERSION defaults to the app's current version. The app is the `kimun-notes` crate;
the server has its own releases and its own tags, and no notes file yet.
"""
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
NOTES_DIR = ROOT / "release-notes"
MANIFEST = ROOT / "tui" / "Cargo.toml"

TEMPLATE = (
    "<!-- Release notes for {version}. Write them here, in plain words for the people "
    "who use the app: what is new, what changed, what to know when upgrading. "
    "Delete this comment. The release page shows this file as it is. -->\n"
)

COMMENT = re.compile(r"<!--.*?-->", re.DOTALL)


def current_version() -> str:
    with MANIFEST.open("rb") as f:
        return tomllib.load(f)["package"]["version"]


def notes_path(version: str, notes_dir: Path = NOTES_DIR) -> Path:
    return notes_dir / f"{version}.md"


def scaffold(version: str, notes_dir: Path = NOTES_DIR) -> bool:
    """Create the empty file; False (and nothing touched) when it exists."""
    path = notes_path(version, notes_dir)
    if path.exists():
        return False
    notes_dir.mkdir(parents=True, exist_ok=True)
    path.write_text(TEMPLATE.format(version=version), encoding="utf-8")
    return True


def written(text: str) -> bool:
    """Whether `text` says anything: more than whitespace once HTML comments are gone."""
    return bool(COMMENT.sub("", text).strip())


def problem(version: str, notes_dir: Path = NOTES_DIR) -> str | None:
    """Why the notes for `version` are not ready, or None when they are."""
    path = notes_path(version, notes_dir)
    rel = f"release-notes/{version}.md"
    if not path.exists():
        return f"{rel} is missing: write the release notes for {version}"
    if not written(path.read_text(encoding="utf-8")):
        return f"{rel} is still the empty template: write the release notes for {version}"
    return None


TAG_PREFIX = "kimun-notes-v"


def is_released(version: str, tags: list[str]) -> bool:
    """Whether `tags` hold the app's release tag for `version`."""
    return f"{TAG_PREFIX}{version}" in tags


def git_tags() -> list[str]:
    out = subprocess.run(
        ["git", "tag", "--list", f"{TAG_PREFIX}*"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return out.stdout.split()


def main(argv: list[str]) -> int:
    if not argv:
        print(__doc__)
        return 2
    command, rest = argv[0], argv[1:]
    version = rest[0] if rest else current_version()
    if command == "version":
        print(current_version())
    elif command == "path":
        print(notes_path(version).relative_to(ROOT))
    elif command == "scaffold":
        created = scaffold(version)
        print(f"{'created' if created else 'kept'} release-notes/{version}.md")
    elif command == "ready":
        return 0 if problem(version) is None else 1
    elif command == "unreleased":
        return 1 if is_released(version, git_tags()) else 0
    elif command == "check":
        why = problem(version)
        if why:
            print(f"::error::{why}")
            return 1
        print(f"release-notes/{version}.md is written")
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
