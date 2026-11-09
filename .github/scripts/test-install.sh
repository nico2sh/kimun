#!/usr/bin/env bash
# Self-test for install.sh against a fake release served from a local directory
# (file:// URLs, a pinned version): the terminal app, the desktop app and its
# Linux launcher files, the shared marker, a bad checksum, an archive with no
# launcher files, the options. Linux only (the desktop files are Linux's).
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

echo "install.sh behaves"
