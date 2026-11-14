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

# The key the installers pin: empty while signing is not live, else the committed one. All
# three scripts must agree (all empty, or all the key in release-signing.pub).
pinned() { # FILE -> the pinned key, or nothing
  # shellcheck disable=SC2016  # the single quotes are on purpose: the text is a sed program
  sed -n -e 's/^SIGNING_PUBKEY="\(.*\)"$/\1/p' -e 's/^\$signingPubKey = "\(.*\)"\r\{0,1\}$/\1/p' "$1"
}
pins="$(for f in install.sh install-server.sh install.ps1; do printf '[%s]\n' "$(pinned "$f")"; done | sort -u)"
[ "$(printf '%s\n' "$pins" | wc -l)" = 1 ] || fail "install.sh, install-server.sh and install.ps1 pin different keys: $pins"
key="$(pinned install.sh)"
if [ -z "$key" ]; then
  ok "the installers pin no key yet: signature checking is off"
else
  [ -f release-signing.pub ] || fail "the installers pin a key but release-signing.pub is missing"
  [ "$key" = "$(sed -n 2p release-signing.pub)" ] || fail "the installers pin a key that is not in release-signing.pub"
  ok "the installers pin the key in release-signing.pub"
fi

# The Rust side of the server update (tui/src/server_manage) names the same key, the same systemd
# unit and the same launchd label as this installer. A drift would make the app restart something
# else, or reject every release. Rust cannot read files outside its crate in a packaged build, so
# the check lives here.
rs=tui/src/server_manage
[ "$(sed -n 's/^pub const SIGNING_KEY: &str = "\(.*\)";$/\1/p' $rs/release.rs)" = "$(sed -n 2p release-signing.pub)" ] \
  || fail "SIGNING_KEY in $rs/release.rs is not the key in release-signing.pub"
unit="$(sed -n 's/^const UNIT: &str = "\(.*\)";$/\1/p' $rs/service.rs)"
label="$(sed -n 's/^const LAUNCHD_LABEL: &str = "\(.*\)";$/\1/p' $rs/service.rs)"
grep -q "^SYSTEMD_UNIT=\"\$HOME/.config/systemd/user/$unit\"$" install-server.sh || fail "the app names a different systemd unit than install-server.sh: $unit"
grep -q "^LAUNCHD_LABEL=\"$label\"$" install-server.sh || fail "the app names a different launchd label than install-server.sh: $label"
ok "the app (server_manage) and install-server.sh agree on the key, the unit and the label"

# The tests below use fake, unsigned releases, so they run an unpinned copy of the installer. The
# signature tests (12) write their own key into a copy of it.
bare="$work/install-server.sh"
sed 's/^SIGNING_PUBKEY=".*"$/SIGNING_PUBKEY=""/' install-server.sh >"$bare"

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
    KIMUN_RELEASES_API="file://$work/no-such-api.json" "$@" sh "$bare"
}

# 1. From the manifest alone: no API (it points at nothing), no download base.
h="$work/h1"; mkdir -p "$h"
out="$(run "$h" "file://$work/latest.json" 2>&1)" || fail "install from the manifest failed: $out"
[ -x "$h/bin/kimun-server" ] || fail "no binary installed"
echo "$out" | grep -q "Latest version: $v" || fail "version not taken from the manifest: $out"
echo "$out" | grep -qi "warning" && fail "manifest install warned: $out"
ok "manifest: the version, the url and the checksum come from it, no API call"

# 1b. The install marker: written by every run, with no service when none was set up.
marker="$h/.config/kimun/server-install.toml"
[ -f "$marker" ] || fail "no install marker written"
grep -q "^install_dir = \"$h/bin\"$" "$marker" || fail "the marker names the wrong install_dir: $(cat "$marker")"
grep -q '^service = "none"$' "$marker" || fail "the marker should say service = none: $(cat "$marker")"
ok "marker: written with the install folder and no service"

# 1c. A server that was installed before the marker existed gets one by running the script
# again, even though the binary is already current.
rm -f "$marker"
out="$(run "$h" "file://$work/latest.json" 2>&1)" || fail "re-run on a current install failed: $out"
echo "$out" | grep -q "already installed" || fail "the re-run did not see the install as current: $out"
[ -f "$marker" ] || fail "a re-run on a current install did not write the marker"
ok "marker: a re-run on a current install writes it"

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
    KIMUN_RELEASES_API="file://$work/api.json" KIMUN_DOWNLOAD_BASE="file://$rel" sh "$bare" 2>&1)" \
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
  patched() { # NAME: a copy of the script that trusts the throwaway key
    sed -e "s|^SIGNING_PUBKEY=.*|SIGNING_PUBKEY=\"$pub\"|" "$bare" >"$work/$1.sh"
  }
  run_signed() { # NAME REL HOME [PATH]: install into a fresh home; prints the output
    mkdir -p "$3"
    PATH="${4:-$PATH}" HOME="$3" KIMUN_SERVER_INSTALL_DIR="$3/bin" KIMUN_MANIFEST_URL="file://$work/latest.json" KIMUN_RELEASES_API="file://$work/no-such-api.json" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$2" \
      "$(command -v sh)" "$work/$1.sh" 2>&1
  }

  # a. Signed, and the key is right: installs; the digest is the signed list's.
  signed_rel "$work/srel"; patched sig-ok
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

  # c. A release with no signature: refused (the latest release is what this installer takes).
  out="$(run_signed sig-ok "$rel" "$work/sg-c")" && fail "an unsigned release was accepted although a key is pinned"
  echo "$out" | grep -q "should be signed" || fail "unsigned said: $out"
  ok "with a key pinned, a release without a signature is refused"

  # d. No key pinned (signing not live): an unsigned release installs, quietly.
  cp "$bare" "$work/sig-off.sh"
  out="$(run_signed sig-off "$rel" "$work/sg-d")" || fail "signing off: an unsigned release was refused: $out"
  echo "$out" | grep -qi "warning" && fail "signing off: an unsigned release warned: $out"
  ok "with no key pinned, nothing is checked"

  # e. No minisign on the machine: a release installs on its sha256 and says it was not checked.
  nm="$work/nominisign"; mkdir -p "$nm"
  for tool in sh uname curl cat mktemp rm awk sed grep tar gzip sha256sum mkdir mv chmod head dirname tr cut sort id ls cp date ln; do
    command -v "$tool" >/dev/null && ln -sf "$(command -v "$tool")" "$nm/$tool"
  done
  out="$(run_signed sig-ok "$work/srel" "$work/sg-e" "$nm")" || fail "no minisign: install failed: $out"
  # shellcheck disable=SC2015  # the first test is a plain grep: the failure message only runs when a check fails
  echo "$out" | grep -q "minisign is not installed" && echo "$out" | grep -q "not checked" || fail "no minisign: no warning: $out"
  ok "without minisign: installs on the sha256 alone, with a warning"
fi

# 13. --uninstall: stops and removes the user service, removes the binary, keeps the configuration
# and the data. systemctl is a stub that logs its calls.
sb="$work/sysbin"; mkdir -p "$sb"
# shellcheck disable=SC2016  # the single quotes are on purpose: this is the text of a stub script
# is-active answers from SYSTEMCTL_ACTIVE (set = running) and is not logged; SYSTEMCTL_FAIL makes the rest fail.
cat >"$sb/systemctl" <<'STUB'
#!/bin/sh
case "$*" in *is-active*) [ -n "${SYSTEMCTL_ACTIVE:-}" ]; exit ;; esac
echo "systemctl $*" >>"$SYSTEMCTL_LOG"
case "$*" in *restart*) [ -z "${SYSTEMCTL_FAIL_RESTART:-}" ]; exit ;; esac
[ -z "${SYSTEMCTL_FAIL:-}" ]
STUB
chmod +x "$sb/systemctl"
uninstall_server() { # HOME [extra env ...]
  local h="$1"; shift
  env PATH="$sb:$PATH" HOME="$h" SYSTEMCTL_LOG="$h/systemctl.log" "$@" sh "$bare" --uninstall
}
if [ "$(uname -s)" = Linux ]; then
  h="$work/u1"; mkdir -p "$h/bin" "$h/.config/systemd/user" "$h/.config/kimun" "$h/.local/share/kimun/rag_sqlite"
  echo x >"$h/bin/kimun-server"; echo unit >"$h/.config/systemd/user/kimun-server.service"
  echo 'keep = "me"' >"$h/.config/kimun/server.toml"; echo db >"$h/.local/share/kimun/rag_sqlite/data"
  out="$(uninstall_server "$h" KIMUN_SERVER_INSTALL_DIR="$h/bin" 2>&1)" || fail "server uninstall failed: $out"
  [ ! -e "$h/bin/kimun-server" ] || fail "uninstall left the binary"
  [ ! -e "$h/.config/systemd/user/kimun-server.service" ] || fail "uninstall left the unit"
  grep -q "stop kimun-server.service" "$h/systemctl.log" || fail "the service was not stopped"
  grep -q "disable kimun-server.service" "$h/systemctl.log" || fail "the service was not disabled"
  grep -q 'keep = "me"' "$h/.config/kimun/server.toml" || fail "uninstall touched server.toml"
  [ -f "$h/.local/share/kimun/rag_sqlite/data" ] || fail "uninstall touched the data"
  ok "server --uninstall: service stopped and removed, binary removed, config and data kept"
  out="$(uninstall_server "$h" KIMUN_SERVER_INSTALL_DIR="$h/bin" 2>&1)" || fail "a second server uninstall failed: $out"
  echo "$out" | grep -q "Nothing to remove" || fail "a second uninstall said: $out"
  ok "server --uninstall with nothing installed: a plain message, exit 0"
  # Without --service ever used: only the binary goes, and systemctl is not called.
  h="$work/u2"; mkdir -p "$h/bin"; echo x >"$h/bin/kimun-server"
  uninstall_server "$h" KIMUN_SERVER_INSTALL_DIR="$h/bin" >/dev/null 2>&1 || fail "uninstall without a service failed"
  [ ! -e "$h/bin/kimun-server" ] || fail "uninstall without a service left the binary"
  [ ! -e "$h/systemctl.log" ] || fail "systemctl was called although no service was set up"
  ok "server --uninstall without a service: only the binary, no systemctl"
  # A running unit whose file is gone is still stopped.
  h="$work/u3"; mkdir -p "$h/bin"; echo x >"$h/bin/kimun-server"
  out="$(uninstall_server "$h" KIMUN_SERVER_INSTALL_DIR="$h/bin" SYSTEMCTL_ACTIVE=1 2>&1)" || fail "uninstall with a running unit failed: $out"
  grep -q "stop kimun-server.service" "$h/systemctl.log" || fail "a running unit without a file was not stopped"
  [ ! -e "$h/bin/kimun-server" ] || fail "uninstall left the binary"
  ok "server --uninstall: a running unit with no unit file is still stopped"
  # A failed stop is a warning, and the uninstall still finishes.
  h="$work/u4"; mkdir -p "$h/bin" "$h/.config/systemd/user"; echo x >"$h/bin/kimun-server"; echo unit >"$h/.config/systemd/user/kimun-server.service"
  out="$(uninstall_server "$h" KIMUN_SERVER_INSTALL_DIR="$h/bin" SYSTEMCTL_FAIL=1 2>&1)" || fail "uninstall failed when systemctl failed: $out"
  echo "$out" | grep -q "could not stop kimun-server.service" || fail "a failed stop was not reported: $out"
  [ ! -e "$h/bin/kimun-server" ] || fail "uninstall left the binary after a failed stop"
  ok "server --uninstall: a failed service stop is reported, the rest still goes"
fi
if sh "$bare" --uninstall --service >/dev/null 2>&1; then fail "--uninstall --service was accepted"; fi
ok "server --uninstall cannot be combined with --service"

# 14. --service: the service is restarted when it was running (the bug: `enable --now` leaves a
# running unit alone, so the old server kept serving), and the marker names the service kind.
if [ "$(uname -s)" = Linux ]; then
  run_service() { # HOME [extra env ...]
    local h="$1"; shift
    env PATH="$sb:$PATH" HOME="$h" SYSTEMCTL_LOG="$h/systemctl.log" KIMUN_SERVER_INSTALL_DIR="$h/bin" \
      KIMUN_MANIFEST_URL="file://$work/latest.json" KIMUN_RELEASES_API="file://$work/no-such-api.json" \
      "$@" sh "$bare" --service
  }
  h="$work/s1"; mkdir -p "$h"
  out="$(run_service "$h" 2>&1)" || fail "first --service install failed: $out"
  grep -q "enable --now kimun-server.service" "$h/systemctl.log" || fail "the service was not enabled: $(cat "$h/systemctl.log")"
  grep -q "restart kimun-server.service" "$h/systemctl.log" && fail "a new service was restarted as well as started"
  grep -q '^service = "systemd"$' "$h/.config/kimun/server-install.toml" || fail "the marker should say systemd"
  ok "--service: a new service is enabled and started, the marker says systemd"

  : >"$h/systemctl.log"
  out="$(run_service "$h" SYSTEMCTL_ACTIVE=1 2>&1)" || fail "--service re-run on a running service failed: $out"
  grep -q "restart kimun-server.service" "$h/systemctl.log" || fail "a running service was not restarted: $(cat "$h/systemctl.log")"
  ok "--service: a service that was running is restarted"

  : >"$h/systemctl.log"
  out="$(run_service "$h" SYSTEMCTL_ACTIVE=1 SYSTEMCTL_FAIL_RESTART=1 2>&1)" && fail "a failed restart was reported as success"
  echo "$out" | grep -q "could not restart" || fail "a failed restart said: $out"
  ok "--service: a failed restart is an error, not silence"

  # Without --service an updated binary still restarts an existing unit (unchanged behavior).
  # And uninstall removes the marker.
  out="$(uninstall_server "$h" KIMUN_SERVER_INSTALL_DIR="$h/bin" 2>&1)" || fail "uninstall after --service failed: $out"
  [ ! -e "$h/.config/kimun/server-install.toml" ] || fail "uninstall left the marker"
  ok "server --uninstall removes the install marker"
fi

echo "install-server.sh behaves"
