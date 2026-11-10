#!/usr/bin/env bash
# Sign a release's checksums-sha256.txt with minisign: writes checksums-sha256.txt.minisig
# next to it. One signature covers every file, because every file's sha256 is in that list.
#
#   sign-release.sh DIR
#
# Environment:
#   MINISIGN_SECRET_KEY       the secret key file's contents (the base64 two-line file)
#   MINISIGN_PASSWORD         its password (empty: the key has none)
#   REQUIRE_SIGNATURE=1       a missing key is an error (a real release), else a warning and
#                             no signature (a fork, a dry run)
#
# The public half is release-signing.pub at the repository root, pinned into the updater and
# the installers. See .github/RELEASING.md, "Signing".
set -euo pipefail

dir="${1:?release directory}"
sums="$dir/checksums-sha256.txt"
[ -f "$sums" ] || { echo "::error::$sums not found"; exit 1; }

if [ -z "${MINISIGN_SECRET_KEY:-}" ]; then
  if [ "${REQUIRE_SIGNATURE:-0}" = 1 ]; then
    echo "::error::MINISIGN_SECRET_KEY is not set: this release cannot be signed"
    exit 1
  fi
  echo "::warning::MINISIGN_SECRET_KEY is not set: the release is NOT signed"
  exit 0
fi
command -v minisign >/dev/null || { echo "::error::minisign is not installed"; exit 1; }

key="$(mktemp)"
trap 'rm -f "$key"' EXIT
umask 077
printf '%s\n' "$MINISIGN_SECRET_KEY" >"$key"

# minisign reads the password from stdin (a key made with -W has none: it still reads a line).
printf '%s\n' "${MINISIGN_PASSWORD:-}" | minisign -S -s "$key" -m "$sums" -x "$sums.minisig" >/dev/null
[ -s "$sums.minisig" ] || { echo "::error::minisign wrote no signature"; exit 1; }
echo "signed $(basename "$sums") -> $(basename "$sums").minisig"
