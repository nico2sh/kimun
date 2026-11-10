#!/usr/bin/env bash
# Self-test for install.sh against a fake release served from a local directory
# (file:// URLs, a pinned version): the terminal app, the desktop app and its
# Linux launcher files, the shared marker, a bad checksum, an archive with no
# launcher files, the options. The macOS desktop app is covered with stubbed hdiutil, ditto and codesign,
# so the real tools are not exercised here (see rehearse-release.sh for that).
# Run: bash .github/scripts/test-install.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
if [ "$(uname -s)" != "Linux" ]; then echo "skipped: not Linux"; exit 0; fi
case "$(uname -m)" in
  x86_64 | amd64) platform=linux-x64 ;;
  aarch64 | arm64) platform=linux-arm64 ;;
  *) echo "skipped: unsupported architecture"; exit 0 ;;
esac

v=9.9.9
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fail() { echo "FAIL: $*"; exit 1; }
ok() { echo "ok: $*"; }

# A release: both apps' archives for every platform the script can pick, and the
# checksums file (so the Rosetta test finds the arm64 one).
make_release() { # DIR [--no-share] [--no-gui]
  local out="$1" stage p
  mkdir -p "$out"
  for p in linux-x64 linux-arm64 macos-x64 macos-arm64; do
    stage="$(mktemp -d)"; printf '#!/bin/sh\necho kimun %s\n' "$p" >"$stage/kimun"; chmod +x "$stage/kimun"
    tar -czf "$out/kimun-$v-$p.tar.gz" -C "$stage" .
    rm -rf "$stage"
    [ "${3:-}" = "--no-gui" ] && continue
    stage="$(mktemp -d)"; printf '#!/bin/sh\necho kimun-gui %s\n' "$p" >"$stage/kimun-gui"; chmod +x "$stage/kimun-gui"
    if [ "${2:-}" != "--no-share" ]; then
      mkdir -p "$stage/share/applications" "$stage/share/icons/hicolor/512x512/apps"
      cp assets/packaging/linux/kimun.desktop "$stage/share/applications/"
      cp assets/packaging/linux/icons/hicolor/512x512/apps/kimun.png "$stage/share/icons/hicolor/512x512/apps/"
    fi
    tar -czf "$out/kimun-gui-$v-$p.tar.gz" -C "$stage" .
    rm -rf "$stage"
  done
  (cd "$out" && sha256sum kimun-*.tar.gz >checksums-sha256.txt)
}

# Run the installer against a release dir, in a fresh home.
install_in() { # HOME RELEASE_DIR [args...]  (extra env from the caller)
  local home="$1" rel="$2"; shift 2
  HOME="$home" KIMUN_INSTALL_DIR="$home/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$rel" \
    sh install.sh "$@"
}

rel="$work/rel"; make_release "$rel"

# 1. The terminal app: its binary and the marker, and none of the desktop files.
h="$work/h1"; mkdir -p "$h"
install_in "$h" "$rel" >/dev/null
[ -x "$h/bin/kimun" ] || fail "terminal binary not installed"
[ ! -e "$h/bin/kimun-gui" ] || fail "the terminal install put kimun-gui in place"
grep -q 'channel = "script"' "$h/.config/kimun/install.toml" || fail "marker missing"
grep -q "version = \"$v\"" "$h/.config/kimun/install.toml" || fail "marker version"
[ ! -e "$h/.local/share" ] || fail "the terminal install made desktop files"
ok "terminal app: binary + marker, no desktop files"

# 2. The desktop app: binary, marker, launcher entry (absolute Exec) and icons.
h="$work/h2"; mkdir -p "$h"
install_in "$h" "$rel" --gui >/dev/null
[ -x "$h/bin/kimun-gui" ] || fail "desktop binary not installed"
[ ! -e "$h/bin/kimun" ] || fail "the desktop install put kimun in place"
grep -q 'channel = "script"' "$h/.config/kimun/install.toml" || fail "marker missing (gui)"
entry="$h/.local/share/applications/kimun.desktop"
[ -f "$entry" ] || fail "launcher entry not installed"
grep -qx "Exec=\"$h/bin/kimun-gui\" %F" "$entry" || fail "Exec not rewritten to the absolute path: $(grep '^Exec=' "$entry")"
grep -qx "Icon=kimun" "$entry" && grep -qx "StartupWMClass=kimun" "$entry" || fail "entry lost its Icon/StartupWMClass"
[ -f "$h/.local/share/icons/hicolor/512x512/apps/kimun.png" ] || fail "icon not installed"
command -v desktop-file-validate >/dev/null && { desktop-file-validate "$entry" || fail "the installed entry does not validate"; }
ok "desktop app: binary, marker, launcher entry (absolute Exec), icons"

# 3. XDG_DATA_HOME is honoured.
h="$work/h3"; mkdir -p "$h"
XDG_DATA_HOME="$work/xdg" install_in "$h" "$rel" --gui >/dev/null
[ -f "$work/xdg/applications/kimun.desktop" ] && [ ! -e "$h/.local/share" ] || fail "XDG_DATA_HOME ignored"
ok "XDG_DATA_HOME honoured"

# 3a. An install dir with sed-special characters reaches the Exec line intact.
h="$work/h3a & b|c"; mkdir -p "$h"
install_in "$h" "$rel" --gui >/dev/null || fail "install into a dir with & and | failed"
grep -qxF "Exec=\"$h/bin/kimun-gui\" %F" "$h/.local/share/applications/kimun.desktop" \
  || fail "Exec corrupted: $(grep '^Exec=' "$h/.local/share/applications/kimun.desktop")"
ok "Exec survives & and | in the install path"

# 3b. An unusable data dir only costs the launcher: the install still completes
# (binary + marker) with a warning.
h="$work/h3b"; mkdir -p "$h"; : >"$work/notadir"
out="$(XDG_DATA_HOME="$work/notadir/x" install_in "$h" "$rel" --gui 2>&1)" || fail "a launcher failure aborted the install: $out"
[ -x "$h/bin/kimun-gui" ] || fail "binary missing after a launcher failure"
grep -q 'channel = "script"' "$h/.config/kimun/install.toml" || fail "marker missing after a launcher failure"
printf '%s' "$out" | grep -q '^warning:' || fail "no warning for the skipped launcher: $out"
ok "launcher failure warns; binary and marker still installed"

# 4. Both apps side by side, one marker.
h="$work/h4"; mkdir -p "$h"
install_in "$h" "$rel" >/dev/null; install_in "$h" "$rel" --gui >/dev/null
[ -x "$h/bin/kimun" ] && [ -x "$h/bin/kimun-gui" ] || fail "both binaries should be installed"
[ "$(grep -c '^channel' "$h/.config/kimun/install.toml")" = 1 ] || fail "one marker, one channel line"
ok "both apps side by side, one marker"

# 5. A bad checksum installs nothing.
bad="$work/bad"; make_release "$bad"; printf 'tampered' >>"$bad/kimun-gui-$v-$platform.tar.gz"
h="$work/h5"; mkdir -p "$h"
if install_in "$h" "$bad" --gui >/dev/null 2>&1; then fail "a tampered archive installed"; fi
[ ! -e "$h/bin/kimun-gui" ] && [ ! -e "$h/.local/share" ] && [ ! -e "$h/.config" ] || fail "a failed install left files behind"
ok "a bad checksum installs nothing"

# 6. An archive without launcher files (macOS-like): the binary, no launcher dirs.
ns="$work/noshare"; make_release "$ns" --no-share
h="$work/h6"; mkdir -p "$h"
install_in "$h" "$ns" --gui >/dev/null
[ -x "$h/bin/kimun-gui" ] && [ ! -e "$h/.local/share" ] || fail "no-share archive"
ok "an archive without launcher files still installs the binary"

# 7. A release with no desktop build says so, plainly, for the desktop app only.
ng="$work/nogui"; make_release "$ng" "" --no-gui
h="$work/h7"; mkdir -p "$h"
out="$(install_in "$h" "$ng" --gui 2>&1)" && fail "a release without a desktop build installed it"
echo "$out" | grep -q "no desktop app build for $platform" || fail "unfriendly error: $out"
out="$(install_in "$h" "$ng" 2>&1)" || fail "the terminal app should still install from that release"
ok "a release without a desktop build: a plain message, the terminal app unaffected"
h="$work/h7b"; mkdir -p "$h"
if out="$(HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$work/nowhere" sh install.sh 2>&1)"; then fail "a missing release installed"; fi
echo "$out" | grep -q "failed to download" || fail "missing release error: $out"
ok "a missing release: a failed-download message"

# 8. Platform detection with a fake uname / sysctl on PATH: Rosetta, Intel Mac, Windows shells.
fakebin() { # DIR OS ARCH ARM64 TRANSLATED
  mkdir -p "$1"
  printf '#!/bin/sh\ncase "$1" in -s) echo %s;; -m) echo %s;; *) echo %s;; esac\n' "$2" "$3" "$2" >"$1/uname"
  printf '#!/bin/sh\ncase "$2" in hw.optional.arm64) echo %s;; sysctl.proc_translated) echo %s;; *) exit 1;; esac\n' "$4" "$5" >"$1/sysctl"
  chmod +x "$1/uname" "$1/sysctl"
}
detected() { # DIR -> the platform install.sh picked
  local h; h="$(mktemp -d -p "$work")"
  PATH="$1:$PATH" HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$rel" \
    sh install.sh 2>&1 | sed -n 's/^Detected platform: //p'
}
fakebin "$work/f1" Darwin x86_64 1 0; [ "$(detected "$work/f1")" = macos-arm64 ] || fail "Rosetta (arm64 capable) did not pick arm64"
fakebin "$work/f2" Darwin x86_64 0 1; [ "$(detected "$work/f2")" = macos-arm64 ] || fail "a translated process did not pick arm64"
fakebin "$work/f3" Darwin x86_64 0 0; [ "$(detected "$work/f3")" = macos-x64 ] || fail "an Intel Mac did not pick x64"
fakebin "$work/f4" Darwin arm64 1 0;  [ "$(detected "$work/f4")" = macos-arm64 ] || fail "Apple Silicon did not pick arm64"
ok "macOS: Rosetta and translated shells take the native arm64 build; Intel stays x64"
fakebin "$work/f5" MINGW64_NT-10.0 x86_64 0 0
out="$(PATH="$work/f5:$PATH" HOME="$work/h" sh install.sh 2>&1)" && fail "Windows shell installed"
echo "$out" | grep -q "windows-x64.zip" || fail "Windows hint missing the zip: $out"
ok "a Windows shell is pointed at the zip"

# 8b. The macOS desktop app: --gui on a (fake) Mac takes the .dmg, mounts it, copies Kimun.app. The
# stubs: uname/sysctl say Darwin; hdiutil "mounts" a .dmg that is a tar of its contents; ditto is cp;
# codesign passes unless CODESIGN_FAIL is set, and, like the real one, also fails when the bundle
# path does not end in .app or when no -R requirement is given.
macbin="$work/macbin"; fakebin "$macbin" Darwin arm64 1 0
cat >"$macbin/hdiutil" <<'STUB'
#!/bin/sh
case "$1" in
  attach) for last; do :; done; while [ $# -gt 0 ]; do [ "$1" = -mountpoint ] && mp="$2"; shift; done
          tar -xf "$last" -C "$mp" || exit 1; echo "$mp" >"${HDIUTIL_LOG:-/dev/null}" ;;
  detach) echo "detach $*" >>"${HDIUTIL_LOG:-/dev/null}" ;;
esac
STUB
printf '#!/bin/sh\ncp -R "$1" "$2"\n' >"$macbin/ditto"
cat >"$macbin/codesign" <<'STUB'
#!/bin/sh
[ -z "${CODESIGN_FAIL:-}" ] || exit 1
req=0; for last; do [ "$prev" = -R ] && req=1; prev="$last"; done
[ "$req" = 1 ] || exit 1
case "$last" in *.app) exit 0 ;; *) exit 1 ;; esac
STUB
chmod +x "$macbin/hdiutil" "$macbin/ditto" "$macbin/codesign"
macrel="$work/macrel"; make_release "$macrel"
stage="$(mktemp -d)"; mkdir -p "$stage/Kimun.app/Contents/MacOS"; printf '#!/bin/sh\necho app\n' >"$stage/Kimun.app/Contents/MacOS/kimun-gui"; chmod +x "$stage/Kimun.app/Contents/MacOS/kimun-gui"
ln -s /Applications "$stage/Applications"
tar -cf "$macrel/kimun-gui-$v-macos-arm64.dmg" -C "$stage" .; rm -rf "$stage"
(cd "$macrel" && sha256sum kimun-*.tar.gz kimun-*.dmg >checksums-sha256.txt)
mac_install() { # HOME [args]
  local h="$1"; shift
  PATH="$macbin:$PATH" HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$macrel" \
    HDIUTIL_LOG="$h/hdiutil.log" sh install.sh --gui "$@"
}
h="$work/m1"; mkdir -p "$h"
mac_install "$h" >"$work/m1.out" 2>&1 || fail "macOS app install failed: $(cat "$work/m1.out")"
[ -x "$h/Applications/Kimun.app/Contents/MacOS/kimun-gui" ] || fail "Kimun.app not in ~/Applications: $(cat "$work/m1.out"; find "$h" -maxdepth 3)"
[ ! -e "$h/Applications/.Kimun.new" ] || fail "staging copy left behind"
[ ! -e "$h/bin/kimun-gui" ] || fail "the macOS app install also put a raw binary in place"
[ ! -e "$h/.config/kimun/install.toml" ] || fail "the macOS app install wrote a marker"
grep -q '^detach' "$h/hdiutil.log" || fail "the .dmg was not detached"
ok "macOS --gui: Kimun.app copied to ~/Applications, .dmg detached, no marker, no raw binary"
# A bad signature: refused, and the app already there is untouched.
h="$work/m2"; mkdir -p "$h/Applications/Kimun.app"; echo old >"$h/Applications/Kimun.app/old"
if out="$(CODESIGN_FAIL=1 mac_install "$h" 2>&1)"; then fail "a bad signature was accepted"; fi
echo "$out" | grep -q "not validly signed with an Apple Developer ID" || fail "bad signature said: $out"
[ -f "$h/Applications/Kimun.app/old" ] || fail "a bad signature replaced the installed app"
[ ! -e "$h/Applications/.Kimun.new" ] || fail "staging copy left after a bad signature"
ok "macOS --gui: a bad signature is refused and the installed app stays"
# A re-install replaces the app, and KIMUN_APP_DIR moves it.
h="$work/m3"; mkdir -p "$h/Applications/Kimun.app"; echo old >"$h/Applications/Kimun.app/old"
mac_install "$h" >/dev/null 2>&1 || fail "re-install failed"
[ ! -e "$h/Applications/Kimun.app/old" ] || fail "re-install kept stale files"
KIMUN_APP_DIR="$h/elsewhere" mac_install "$h" >/dev/null 2>&1 || fail "KIMUN_APP_DIR install failed"
[ -d "$h/elsewhere/Kimun.app" ] || fail "KIMUN_APP_DIR ignored"
ok "macOS --gui: a re-install replaces the app; KIMUN_APP_DIR is honored"
# A corrupted .dmg: checksum mismatch, nothing installed.
cp -R "$macrel" "$work/macbad"; echo x >>"$work/macbad/kimun-gui-$v-macos-arm64.dmg"
h="$work/m4"; mkdir -p "$h"
if PATH="$macbin:$PATH" HOME="$h" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$work/macbad" sh install.sh --gui >/dev/null 2>&1; then fail "a corrupt .dmg installed"; fi
[ ! -e "$h/Applications/Kimun.app" ] || fail "a corrupt .dmg still installed"
ok "macOS --gui: a corrupt .dmg fails the checksum"
# A release with no .dmg: the raw binary, with a note.
rm "$macrel/kimun-gui-$v-macos-arm64.dmg"; (cd "$macrel" && sha256sum kimun-*.tar.gz >checksums-sha256.txt)
h="$work/m5"; mkdir -p "$h"
out="$(mac_install "$h" 2>&1)" || fail "no-.dmg release failed: $out"
[ -x "$h/bin/kimun-gui" ] || fail "no-.dmg release: raw binary missing"
echo "$out" | grep -q "no macOS app" || fail "no-.dmg release did not say so: $out"
ok "macOS --gui: a release with no .dmg falls back to the raw binary"
# The terminal app on a Mac is not affected.
h="$work/m6"; mkdir -p "$h"
PATH="$macbin:$PATH" HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$macrel" sh install.sh >/dev/null 2>&1 || fail "terminal install on a Mac failed"
[ -x "$h/bin/kimun" ] && [ ! -e "$h/Applications" ] || fail "terminal install on a Mac changed"
ok "macOS: the terminal app install is unchanged"

# 9. Missing tools are named up front.
nt="$work/notar"; mkdir -p "$nt"; for tool in sh uname; do ln -s "$(command -v $tool)" "$nt/$tool"; done
out="$(PATH="$nt" HOME="$work/h" sh install.sh 2>&1)" && fail "ran without curl, wget or tar"
echo "$out" | grep -q "needs curl or wget" || fail "missing downloader not named: $out"
for tool in curl cat mktemp rm awk sed grep; do ln -sf "$(command -v $tool)" "$nt/$tool"; done
out="$(PATH="$nt" HOME="$work/h" sh install.sh 2>&1)" && fail "ran without tar"
echo "$out" | grep -q "needs tar" || fail "missing tar not named: $out"
ok "missing tools are named before anything is downloaded"

# 10. Options.
if sh install.sh --nope >/dev/null 2>&1; then fail "unknown option accepted"; fi
sh install.sh --help | grep -q -- "--gui" || fail "--help does not mention --gui"
cat install.sh | sh -s -- --help | grep -qi "gui" || fail "--help when piped"
ok "options"

# 11. The release manifest (latest.json, built by release_manifest.py) and the GitHub API fallback.
# No network: the manifest and the fake API are files, and the real generator writes the manifest.
if ! command -v python3 >/dev/null; then
  echo "skipped: the manifest tests need python3"
else
  python3 - "$rel" "$work/releases.json" <<'PY'
import json, os, sys
rel, out = sys.argv[1], sys.argv[2]
assets = [{"name": n, "browser_download_url": f"file://{rel}/{n}"} for n in sorted(os.listdir(rel))]
json.dump([[{"tag_name": "kimun-notes-v9.9.9", "draft": False, "published_at": "2026-01-01T00:00:00Z",
             "html_url": "https://example.test/9.9.9", "assets": assets}]], open(out, "w"))
PY
  python3 -I .github/scripts/release_manifest.py "$work/releases.json" "$work/latest.json" >/dev/null 2>&1
  # Unpinned, the files not redirected: everything must come from the manifest's own urls.
  install_unpinned() { # HOME MANIFEST_URL [args...]  (extra env from the caller)
    local home="$1" manifest="$2"; shift 2
    HOME="$home" KIMUN_INSTALL_DIR="$home/bin" KIMUN_MANIFEST_URL="$manifest" \
      KIMUN_RELEASES_API="file://$work/no-such-api.json" sh install.sh "$@"
  }

  h="$work/m1"; mkdir -p "$h"
  out="$(install_unpinned "$h" "file://$work/latest.json" 2>&1)" || fail "install from the manifest failed: $out"
  [ -x "$h/bin/kimun" ] || fail "manifest install: no binary"
  grep -q "version = \"$v\"" "$h/.config/kimun/install.toml" || fail "manifest install: marker version"
  echo "$out" | grep -qi "warning" && fail "manifest install warned: $out"
  h="$work/m1g"; mkdir -p "$h"
  install_unpinned "$h" "file://$work/latest.json" --gui >/dev/null 2>&1 || fail "desktop install from the manifest failed"
  [ -x "$h/bin/kimun-gui" ] || fail "manifest install: no desktop binary"
  ok "manifest: the version, the urls and the checksum come from it, no API call"

  # The manifest's sha256 is the one checked: change it and the install is refused.
  sed "s/\"sha256\": \"[0-9a-f]*\"/\"sha256\": \"$(printf '0%.0s' $(seq 64))\"/" "$work/latest.json" >"$work/bad.json"
  h="$work/m2"; mkdir -p "$h"
  out="$(install_unpinned "$h" "file://$work/bad.json" 2>&1)" && fail "a wrong manifest checksum was accepted"
  echo "$out" | grep -q "checksum mismatch" || fail "wrong manifest checksum said: $out"
  [ ! -e "$h/bin/kimun" ] || fail "a wrong manifest checksum still installed"
  ok "manifest: its checksum is enforced"

  # Fallback: unreachable, another schema, no app in it. Each one warns and uses the API (a file here).
  fallback() { # NAME MANIFEST_URL WHY-PATTERN
    h="$work/fb-$1"; mkdir -p "$h"
    out="$(HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_MANIFEST_URL="$2" KIMUN_RELEASES_API="file://$work/api.json" \
      KIMUN_DOWNLOAD_BASE="file://$rel" sh install.sh 2>&1)" || fail "fallback ($1) failed: $out"
    echo "$out" | grep -q "warning: .*$3.*using the GitHub API" || fail "fallback ($1) did not say why: $out"
    [ -x "$h/bin/kimun" ] || fail "fallback ($1): no binary"
    grep -q "version = \"$v\"" "$h/.config/kimun/install.toml" || fail "fallback ($1): marker version"
  }
  printf '[{"tag_name":"kimun_core-v9.0.0"},{"tag_name":"kimun-notes-v9.9.9-beta.1"},{"tag_name":"kimun-notes-v%s"}]\n' "$v" >"$work/api.json"
  fallback unreachable "file://$work/no-such-manifest.json" "cannot reach"
  sed 's/"schema": 1/"schema": 2/' "$work/latest.json" >"$work/schema2.json"
  fallback schema "file://$work/schema2.json" "format"
  printf '{\n  "schema": 1\n}\n' >"$work/empty.json"
  fallback empty "file://$work/empty.json" "names no app release"
  ok "manifest unreachable, unknown or empty: a warning, then the GitHub API"

  # With nothing to fall back to either, the failure says so.
  h="$work/m3"; mkdir -p "$h"
  out="$(install_unpinned "$h" "file://$work/no-such-manifest.json" 2>&1)" && fail "installed with no manifest and no API"
  echo "$out" | grep -q "could not reach the GitHub releases API" || fail "no manifest, no API said: $out"
  ok "no manifest and no API: a plain error"
fi

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
    sed -e "s|^SIGNING_PUBKEY=.*|SIGNING_PUBKEY=\"$pub\"|" install.sh >"$work/$1.sh"
  }
  run_signed() { # NAME REL HOME [PATH]: install into a fresh home; prints the output
    mkdir -p "$3"
    PATH="${4:-$PATH}" HOME="$3" KIMUN_INSTALL_DIR="$3/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$2" \
      "$(command -v sh)" "$work/$1.sh" 2>&1
  }

  # a. Signed, and the key is right: installs; the digest is the signed list's.
  signed_rel "$work/srel"; patched sig-ok
  out="$(run_signed sig-ok "$work/srel" "$work/sg-a")" || fail "a validly signed release was refused: $out"
  [ -x "$work/sg-a/bin/kimun" ] || fail "signed install: no binary"
  echo "$out" | grep -qi "warning" && fail "a validly signed release warned: $out"
  ok "signed release: installs, no warning"

  # b. The list was edited after signing (archive and list changed together): refused, nothing installed.
  signed_rel "$work/trel"
  tmpd="$(mktemp -d)"; printf '#!/bin/sh\necho evil\n' >"$tmpd/kimun"; chmod +x "$tmpd/kimun"
  arch="$work/trel/kimun-$v-$platform.tar.gz"
  tar -czf "$arch" -C "$tmpd" .
  (cd "$work/trel" && grep -v "$(basename "$arch")" checksums-sha256.txt >c.new; sha256sum "$(basename "$arch")" >>c.new; mv c.new checksums-sha256.txt)
  out="$(run_signed sig-ok "$work/trel" "$work/sg-b")" && fail "an edited checksums list was accepted"
  echo "$out" | grep -q "NOT valid" || fail "an edited list said: $out"
  [ ! -e "$work/sg-b/bin/kimun" ] || fail "an edited list still installed"
  ok "a signed list that was changed is refused"

  # c. A pinned version with no signature (it may predate signing): installs, with a warning.
  out="$(run_signed sig-ok "$rel" "$work/sg-c")" || fail "a pinned unsigned release was refused: $out"
  echo "$out" | grep -q "no signature" || fail "a pinned unsigned release did not warn: $out"
  ok "a pinned version without a signature installs on its sha256, with a warning"

  # d. The latest release (not pinned) with no signature: refused, nothing installed.
  h="$work/sg-d"; mkdir -p "$h"
  out="$(HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_MANIFEST_URL="file://$work/latest.json" KIMUN_RELEASES_API="file://$work/no-such-api.json" \
    KIMUN_DOWNLOAD_BASE="file://$rel" sh "$work/sig-ok.sh" 2>&1)" && fail "an unsigned latest release was accepted although a key is pinned"
  echo "$out" | grep -q "should be signed" || fail "unsigned latest said: $out"
  [ ! -e "$h/bin/kimun" ] || fail "an unsigned latest release still installed"
  ok "the latest release without a signature is refused"

  # d2. No key pinned (signing not live): nothing is checked, nothing said.
  cp install.sh "$work/sig-off.sh"
  out="$(run_signed sig-off "$rel" "$work/sg-d2")" || fail "signing off: an unsigned release was refused: $out"
  echo "$out" | grep -qi "warning" && fail "signing off: an unsigned release warned: $out"
  ok "with no key pinned, nothing is checked"

  # e. No minisign on the machine: a release installs on its sha256 and says it was not checked.
  nm="$work/nominisign"; mkdir -p "$nm"
  for tool in sh uname curl cat mktemp rm awk sed grep tar gzip sha256sum mkdir mv chmod head dirname tr cut sort id ls cp date ln; do
    command -v "$tool" >/dev/null && ln -sf "$(command -v "$tool")" "$nm/$tool"
  done
  out="$(run_signed sig-ok "$work/srel" "$work/sg-e" "$nm")" || fail "no minisign: install failed: $out"
  echo "$out" | grep -q "minisign is not installed" && echo "$out" | grep -q "not checked" || fail "no minisign: no warning: $out"
  ok "without minisign: installs on the sha256 alone, with a warning"
fi

echo "install.sh behaves"
