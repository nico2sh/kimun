#!/usr/bin/env python3
"""The static release manifest: `latest.json`, published at https://kimun.2co.dev/latest.json.

    release_manifest.py RELEASES_JSON OUT

RELEASES_JSON is the GitHub releases list (`gh api --paginate --slurp .../releases`, a list of
pages, or one plain list; `-` reads stdin). OUT is the file written.

For each release series (the app, the server) it names the newest *stable* published release
and every file of it with its URL and sha256, so the installers and the updater read one small
file instead of paging through the GitHub API (which lists only the newest 100 releases across
all three tag series, and allows 60 unauthenticated calls an hour per address).

    {"schema": 1,
     "app":    {"version": "0.24.8", "published": "...", "page": "...",
                "assets": {"kimun-0.24.8-linux-x64.tar.gz": {"url": "...", "sha256": "..."}, ...}},
     "server": {...}}

Assets are keyed by their exact file name, the name every consumer already builds, so no naming
rule is repeated here. The sha256 comes from the release's own `checksums-sha256.txt`.

A series that cannot be described is left out, with a warning on stderr, never filled in from an
older release (that would hide the problem): its absence makes a consumer fall back to the GitHub
API. A stable release means: not a draft, and a tag whose version has no hyphen. (A draft is what
release-plz creates until the build has attached the files, see release-plz.toml.) The exit status
is 1 only when no series could be described.
"""
import json
import re
import sys
import time
import urllib.request
from typing import Callable

SCHEMA = 1
CHECKSUMS = "checksums-sha256.txt"
# manifest key -> tag prefix. kimun_core has no assets and no installer, so it is not here.
SERIES = {"app": "kimun-notes-v", "server": "kimun_server-v"}
SHA256 = re.compile(r"^[0-9a-f]{64}$")


class SeriesError(Exception):
    """One series cannot be described; the message says why."""


def flatten(data) -> list:
    """The releases as one list: `gh api --slurp` gives a list of pages."""
    if data and all(isinstance(p, list) for p in data):
        return [r for page in data for r in page]
    return list(data)


def parse_checksums(text: str) -> dict:
    """name -> sha256 from sha256sum's two-column format (`hash  name`, or `hash *name`)."""
    out = {}
    for line in text.splitlines():
        fields = line.strip().split(None, 1)
        if len(fields) != 2:
            continue
        digest, name = fields[0].lower(), fields[1].lstrip("*")
        if SHA256.match(digest):
            out[name] = digest
    return out


def newest_stable(releases: list, prefix: str):
    """The first (newest) published release of the series with no pre-release suffix."""
    for r in releases:
        tag = r.get("tag_name", "")
        if not tag.startswith(prefix) or r.get("draft"):
            continue
        version = tag[len(prefix):]
        if "-" in version:
            continue
        return version, r
    return None


def describe(version: str, release: dict, fetch_text: Callable[[str], str]) -> dict:
    assets = {a["name"]: a["browser_download_url"] for a in release.get("assets", [])}
    if CHECKSUMS not in assets:
        raise SeriesError(f"release {version} has no {CHECKSUMS}")
    try:
        sums = parse_checksums(fetch_text(assets[CHECKSUMS]))
    except Exception as e:  # network, decoding: the caller needs the reason, not a traceback
        raise SeriesError(f"cannot read {CHECKSUMS} of {version}: {e}") from e
    # Only files that are both attached and checksummed: a name in only one of the two
    # cannot be installed safely.
    listed = {
        name: {"url": assets[name], "sha256": sums[name]}
        for name in sorted(assets)
        if name != CHECKSUMS and name in sums
    }
    if not listed:
        raise SeriesError(f"release {version} has no file with a checksum")
    return {
        "version": version,
        "published": release.get("published_at") or "",
        "page": release.get("html_url") or "",
        "assets": listed,
    }


def build(releases: list, fetch_text: Callable[[str], str], warn: Callable[[str], None]) -> dict:
    manifest = {"schema": SCHEMA}
    for key, prefix in SERIES.items():
        found = newest_stable(releases, prefix)
        if found is None:
            warn(f"{key}: no stable published release with tag {prefix}*")
            continue
        version, release = found
        try:
            manifest[key] = describe(version, release, fetch_text)
        except SeriesError as e:
            warn(f"{key}: left out: {e}")
    return manifest


def http_text(url: str) -> str:
    last = None
    for attempt in range(3):
        try:
            with urllib.request.urlopen(url, timeout=30) as r:
                return r.read().decode("utf-8")
        except Exception as e:
            last = e
            time.sleep(2 * (attempt + 1))
    raise last


def main(argv: list) -> int:
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    src, out = argv[1], argv[2]
    raw = sys.stdin.read() if src == "-" else open(src, encoding="utf-8").read()
    warn = lambda m: print(f"::warning::latest.json: {m}", file=sys.stderr)
    manifest = build(flatten(json.loads(raw)), http_text, warn)
    if len(manifest) == 1:  # only "schema"
        print("::error::latest.json: no release series could be described", file=sys.stderr)
        return 1
    with open(out, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")
    print(f"wrote {out}: " + ", ".join(f"{k} {manifest[k]['version']}" for k in SERIES if k in manifest))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
