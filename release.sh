#!/usr/bin/env bash
set -euo pipefail

SEMTAG="$(dirname "$0")/semtag"

# Pass-through any semtag flags (e.g. -s patch, -s minor, -s major)
SEMTAG_ARGS=("$@")

# 1. Determine the next version without tagging yet.
VERSION=$("$SEMTAG" final -o "${SEMTAG_ARGS[@]}" | sed 's/^v//')
echo "Releasing version: $VERSION"

# 2. Stamp the version into kimun-notes only (core is versioned manually).
cargo set-version -p kimun-notes "$VERSION"

# 3. Commit the version bump.
git add tui/Cargo.toml Cargo.lock
git commit -m "chore: bump version to $VERSION"

# 4. Tag and push via semtag (also pushes the commit).
"$SEMTAG" final "${SEMTAG_ARGS[@]}"
