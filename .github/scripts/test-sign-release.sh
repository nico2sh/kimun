#!/usr/bin/env bash
# Self-test for sign-release.sh with a throwaway key: it signs checksums-sha256.txt, the
# signature verifies and breaks when the file changes, and a missing key is an error only
# when REQUIRE_SIGNATURE=1. Needs minisign. Run: bash .github/scripts/test-sign-release.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
command -v minisign >/dev/null || { echo "skipped: needs minisign"; exit 0; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fail() { echo "FAIL: $*"; exit 1; }
ok() { echo "ok: $*"; }

rel="$work/rel"; mkdir -p "$rel"
printf '%s  kimun-9.9.9-linux-x64.tar.gz\n' "$(printf a | sha256sum | cut -d' ' -f1)" >"$rel/checksums-sha256.txt"

# A key without a password, and one with.
minisign -G -W -f -p "$work/open.pub" -s "$work/open.key" >/dev/null
printf 'hunter2\nhunter2\n' | minisign -G -f -p "$work/locked.pub" -s "$work/locked.key" >/dev/null

# 1. Signs, and the signature verifies against the public key.
MINISIGN_SECRET_KEY="$(cat "$work/open.key")" bash .github/scripts/sign-release.sh "$rel" >/dev/null
[ -s "$rel/checksums-sha256.txt.minisig" ] || fail "no signature written"
minisign -V -p "$work/open.pub" -m "$rel/checksums-sha256.txt" >/dev/null || fail "signature does not verify"
ok "signs checksums-sha256.txt; the signature verifies"

# 2. A changed file no longer verifies; another key does not verify it.
cp "$rel/checksums-sha256.txt" "$work/sums.bak"
echo "tampered" >>"$rel/checksums-sha256.txt"
minisign -V -p "$work/open.pub" -m "$rel/checksums-sha256.txt" >/dev/null 2>&1 && fail "a changed file still verifies"
cp "$work/sums.bak" "$rel/checksums-sha256.txt"
minisign -V -p "$work/locked.pub" -m "$rel/checksums-sha256.txt" >/dev/null 2>&1 && fail "verified with the wrong key"
ok "tampering or the wrong key is rejected"

# 3. A key with a password.
rm "$rel/checksums-sha256.txt.minisig"
MINISIGN_SECRET_KEY="$(cat "$work/locked.key")" MINISIGN_PASSWORD=hunter2 bash .github/scripts/sign-release.sh "$rel" >/dev/null
minisign -V -p "$work/locked.pub" -m "$rel/checksums-sha256.txt" >/dev/null || fail "password key: no valid signature"
rm "$rel/checksums-sha256.txt.minisig"
MINISIGN_SECRET_KEY="$(cat "$work/locked.key")" MINISIGN_PASSWORD=wrong bash .github/scripts/sign-release.sh "$rel" >/dev/null 2>&1 && fail "a wrong password signed"
[ ! -s "$rel/checksums-sha256.txt.minisig" ] || fail "a wrong password left a signature"
ok "a password-protected key works, a wrong password does not"

# 4. No key: a warning and no signature, unless a signature is required.
out="$(env -u MINISIGN_SECRET_KEY bash .github/scripts/sign-release.sh "$rel" 2>&1)" || fail "no key, not required, failed: $out"
echo "$out" | grep -q "NOT signed" || fail "no key did not warn: $out"
[ ! -e "$rel/checksums-sha256.txt.minisig" ] || fail "no key still wrote a signature"
env -u MINISIGN_SECRET_KEY REQUIRE_SIGNATURE=1 bash .github/scripts/sign-release.sh "$rel" >/dev/null 2>&1 && fail "no key was accepted when required"
ok "no key: a warning, or an error when REQUIRE_SIGNATURE=1"

echo "sign-release.sh behaves"
