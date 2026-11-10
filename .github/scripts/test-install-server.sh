#!/usr/bin/env bash
# Self-test for install-server.sh against a fake release served from local files
# (file:// URLs, no network): the version, the urls and the checksum from the release
# manifest, a wrong checksum, and the GitHub API fallback when the manifest cannot be
# used. Linux only. Run: bash .github/scripts/test-install-server.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
if [ "$(uname -s)" != "Linux" ]; then echo "skipped: not Linux"; exit 0; fi
case "$(uname -m)" in
  x86_64 | amd64) platform=linux-x64 ;;
  aarch64 | arm64) platform=linux-arm64 ;;
  *) echo "skipped: unsupported architecture"; exit 0 ;;
esac
command -v python3 >/dev/null || { echo "skipped: needs python3 (the manifest generator)"; exit 0; }

v=9.9.9
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fail() { echo "FAIL: $*"; exit 1; }
ok() { echo "ok: $*"; }

# 0. install.sh and install-server.sh are each fetched alone (curl | sh), so neither can source a
# shared file: the awk readers of latest.json are copied in both. They must not drift.
readers() { # SCRIPT -> the two awk readers, without indentation
  awk '/^ *manifest_version\(\)/ { on = 1 } /^# Read the manifest/ { on = 0 } on { sub(/^ +/, ""); print }' "$1"
}
[ -n "$(readers install.sh)" ] || fail "found no manifest readers in install.sh"
[ "$(readers install.sh)" = "$(readers install-server.sh)" ] \
  || fail "the latest.json readers in install.sh and install-server.sh differ: keep them identical"
ok "install.sh and install-server.sh read latest.json with the same code"

# The signature check is copied the same way. And the key they pin must be the committed one.
sigblock() { awk '/^# --- release signature/ { on = 1 } /^# --- detect platform/ { on = 0 } on' "$1" | grep -v '^FIRST_SIGNED='; }
[ -n "$(sigblock install.sh)" ] || fail "found no signature block in install.sh"
[ "$(sigblock install.sh)" = "$(sigblock install-server.sh)" ] \
  || fail "the signature checks in install.sh and install-server.sh differ: keep them identical"
if [ -f release-signing.pub ]; then
  key="$(sed -n 2p release-signing.pub)"
  for f in install.sh install-server.sh; do
    grep -qx "SIGNING_PUBKEY=\"$key\"" "$f" || fail "$f does not pin the key in release-signing.pub"
  done
  ok "the installers pin the key in release-signing.pub"
else
  echo "note: release-signing.pub is not committed yet: the installers pin no key"
fi
ok "install.sh and install-server.sh check signatures with the same code"

# A release: the server's archive for every platform the script can pick, and checksums.
rel="$work/rel"; mkdir -p "$rel"
for p in linux-x64 linux-arm64 macos-arm64; do
  stage="$(mktemp -d)"
  printf '#!/bin/sh\necho kimun-server %s\n' "$v" >"$stage/kimun-server"; chmod +x "$stage/kimun-server"
  tar -czf "$rel/kimun-server-$v-$p.tar.gz" -C "$stage" .
  rm -rf "$stage"
done
(cd "$rel" && sha256sum kimun-server-*.tar.gz >checksums-sha256.txt)

# The manifest, written by the real generator from a fake releases list.
python3 - "$rel" "$work/releases.json" <<'PY'
import json, os, sys
rel, out = sys.argv[1], sys.argv[2]
assets = [{"name": n, "browser_download_url": f"file://{rel}/{n}"} for n in sorted(os.listdir(rel))]
json.dump([[{"tag_name": "kimun_server-v9.9.9", "draft": False, "published_at": "2026-01-01T00:00:00Z",
             "html_url": "https://example.test/server", "assets": assets}]], open(out, "w"))
PY
python3 -I .github/scripts/release_manifest.py "$work/releases.json" "$work/latest.json" >/dev/null 2>&1

run() { # HOME MANIFEST_URL [extra env VAR=value ...]
  local home="$1" manifest="$2"; shift 2
  env HOME="$home" KIMUN_SERVER_INSTALL_DIR="$home/bin" KIMUN_MANIFEST_URL="$manifest" \
    KIMUN_RELEASES_API="file://$work/no-such-api.json" "$@" sh install-server.sh
}

# 1. From the manifest alone: no API (it points at nothing), no download base.
h="$work/h1"; mkdir -p "$h"
out="$(run "$h" "file://$work/latest.json" 2>&1)" || fail "install from the manifest failed: $out"
[ -x "$h/bin/kimun-server" ] || fail "no binary installed"
echo "$out" | grep -q "Latest version: $v" || fail "version not taken from the manifest: $out"
echo "$out" | grep -qi "warning" && fail "manifest install warned: $out"
ok "manifest: the version, the url and the checksum come from it, no API call"

# 2. Its sha256 is the one enforced.
sed "s/\"sha256\": \"[0-9a-f]*\"/\"sha256\": \"$(printf '0%.0s' $(seq 64))\"/" "$work/latest.json" >"$work/bad.json"
h="$work/h2"; mkdir -p "$h"
out="$(run "$h" "file://$work/bad.json" 2>&1)" && fail "a wrong manifest checksum was accepted"
echo "$out" | grep -q "checksum mismatch" || fail "wrong manifest checksum said: $out"
[ ! -e "$h/bin/kimun-server" ] || fail "a wrong manifest checksum still installed"
ok "manifest: its checksum is enforced"

# 3. Fallback: unreachable, another schema, no server in it. A warning, then the API (a file).
printf '[{"tag_name":"kimun-notes-v9.0.0"},{"tag_name":"kimun_server-v9.9.9-beta.1"},{"tag_name":"kimun_server-v%s"}]\n' "$v" >"$work/api.json"
fallback() { # NAME MANIFEST_URL WHY-PATTERN
  h="$work/fb-$1"; mkdir -p "$h"
  out="$(env HOME="$h" KIMUN_SERVER_INSTALL_DIR="$h/bin" KIMUN_MANIFEST_URL="$2" \
    KIMUN_RELEASES_API="file://$work/api.json" KIMUN_DOWNLOAD_BASE="file://$rel" sh install-server.sh 2>&1)" \
    || fail "fallback ($1) failed: $out"
  echo "$out" | grep -q "warning: .*$3.*using the GitHub API" || fail "fallback ($1) did not say why: $out"
  [ -x "$h/bin/kimun-server" ] || fail "fallback ($1): no binary"
}
fallback unreachable "file://$work/no-such-manifest.json" "cannot reach"
sed 's/"schema": 1/"schema": 2/' "$work/latest.json" >"$work/schema2.json"
fallback schema "file://$work/schema2.json" "format"
printf '{\n  "schema": 1\n}\n' >"$work/empty.json"
fallback empty "file://$work/empty.json" "names no server release"
ok "manifest unreachable, unknown or empty: a warning, then the GitHub API"

# 4. Nothing to fall back to either: a plain error.
h="$work/h4"; mkdir -p "$h"
out="$(run "$h" "file://$work/no-such-manifest.json" 2>&1)" && fail "installed with no manifest and no API"
echo "$out" | grep -q "could not reach the GitHub releases API" || fail "no manifest, no API said: $out"
ok "no manifest and no API: a plain error"

# 12. The release signature (minisign), against a throwaway key written into a copy of the script.
if ! command -v minisign >/dev/null; then
  echo "skipped: the signature tests need minisign"
else
  minisign -G -W -f -p "$work/s.pub" -s "$work/s.key" >/dev/null
  pub="$(sed -n 2p "$work/s.pub")"
  signed_rel() { # DIR: a signed copy of the release
    rm -rf "$1"; cp -r "$rel" "$1"
    minisign -S -s "$work/s.key" -m "$1/checksums-sha256.txt" -x "$1/checksums-sha256.txt.minisig" >/dev/null
  }
  patched() { # NAME FIRST_SIGNED: a copy of the script that trusts the throwaway key
    sed -e "s|^SIGNING_PUBKEY=.*|SIGNING_PUBKEY=\"$pub\"|" -e "s|^FIRST_SIGNED=.*|FIRST_SIGNED=\"$2\"|" install-server.sh >"$work/$1.sh"
  }
  run_signed() { # NAME REL HOME [PATH]: install into a fresh home; prints the output
    mkdir -p "$3"
    PATH="${4:-$PATH}" HOME="$3" KIMUN_SERVER_INSTALL_DIR="$3/bin" KIMUN_MANIFEST_URL="file://$work/latest.json" KIMUN_RELEASES_API="file://$work/no-such-api.json" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$2" \
      "$(command -v sh)" "$work/$1.sh" 2>&1
  }

  # a. Signed, and the key is right: installs; the digest is the signed list's.
  signed_rel "$work/srel"; patched sig-ok 0.0.1
  out="$(run_signed sig-ok "$work/srel" "$work/sg-a")" || fail "a validly signed release was refused: $out"
  [ -x "$work/sg-a/bin/kimun-server" ] || fail "signed install: no binary"
  echo "$out" | grep -qi "warning" && fail "a validly signed release warned: $out"
  ok "signed release: installs, no warning"

  # b. The list was edited after signing (archive and list changed together): refused, nothing installed.
  signed_rel "$work/trel"
  tmpd="$(mktemp -d)"; printf '#!/bin/sh\necho evil\n' >"$tmpd/kimun-server"; chmod +x "$tmpd/kimun-server"
  arch="$work/trel/kimun-server-$v-$platform.tar.gz"
  tar -czf "$arch" -C "$tmpd" .
  (cd "$work/trel" && grep -v "$(basename "$arch")" checksums-sha256.txt >c.new; sha256sum "$(basename "$arch")" >>c.new; mv c.new checksums-sha256.txt)
  out="$(run_signed sig-ok "$work/trel" "$work/sg-b")" && fail "an edited checksums list was accepted"
  echo "$out" | grep -q "NOT valid" || fail "an edited list said: $out"
  [ ! -e "$work/sg-b/bin/kimun-server" ] || fail "an edited list still installed"
  ok "a signed list that was changed is refused"

  # c. A release that must be signed and has no signature: refused.
  patched sig-req 0.0.1
  out="$(run_signed sig-req "$rel" "$work/sg-c")" && fail "an unsigned release was accepted although it must be signed"
  echo "$out" | grep -q "should be signed" || fail "unsigned-but-required said: $out"
  ok "a release at or above the first signed version must carry a signature"

  # d. An older release (before the first signed one), unsigned: installs, quietly.
  patched sig-old 99.0.0
  out="$(run_signed sig-old "$rel" "$work/sg-d")" || fail "an old unsigned release was refused: $out"
  echo "$out" | grep -qi "warning" && fail "an old unsigned release warned: $out"
  ok "a release before the first signed one needs no signature"

  # e. No minisign on the machine: a signed release installs on its sha256 and says it was not checked.
  nm="$work/nominisign"; mkdir -p "$nm"
  for tool in sh uname curl cat mktemp rm awk sed grep tar gzip sha256sum mkdir mv chmod head dirname tr cut sort id ls cp date ln; do
    command -v "$tool" >/dev/null && ln -sf "$(command -v "$tool")" "$nm/$tool"
  done
  out="$(run_signed sig-ok "$work/srel" "$work/sg-e" "$nm")" || fail "no minisign: install failed: $out"
  echo "$out" | grep -q "minisign is not installed" && echo "$out" | grep -q "not checked" || fail "no minisign: no warning: $out"
  ok "without minisign: installs on the sha256 alone, with a warning"
fi

echo "install-server.sh behaves"
