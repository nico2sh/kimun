#!/usr/bin/env bash
# Tests for release-dist.sh: five platforms assembled the way the build job does it,
# packaged the way the release job does it, then checked by the verifier.
# Run: bash .github/scripts/test-release-dist.sh
# shellcheck disable=SC2015  # `[ A ] && [ B ] || fail ...` is intended here: fail when either test is false
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
v=9.9.9
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fail() { echo "FAIL: $*"; exit 1; }
ok() { echo "ok: $*"; }

art="$work/release-artifacts"
mkdir -p "$art"
for p in linux-x64 linux-arm64 macos-x64 macos-arm64 windows-x64; do
  ext=""; case "$p" in windows-*) ext=".exe" ;; esac
  bin="$work/bin-$p"; mkdir -p "$bin"
  echo "tui $p" >"$bin/kimun$ext"; echo "gui $p" >"$bin/kimun-gui$ext"
  out="$work/out-$p"
  bash "$here/release-dist.sh" assemble "$p" "$bin" "$out"
  [ -f "$out/dist/kimun$ext" ] && [ -f "$out/dist-gui/kimun-gui$ext" ] || fail "$p: the binaries are not in dist/ and dist-gui/"
  case "$p" in
    linux-*) [ -f "$out/dist-gui/share/applications/kimun.desktop" ] && [ -f "$out/dist-gui/share/icons/hicolor/512x512/apps/kimun.png" ] || fail "$p: no launcher files" ;;
    *) [ ! -e "$out/dist-gui/share" ] || fail "$p: launcher files outside Linux" ;;
  esac
  [ ! -e "$out/dist/share" ] || fail "$p: the terminal app got launcher files"
  mv "$out/dist" "$art/dist-$p"; mv "$out/dist-gui" "$art/distgui-$p"
done
ok "assemble: dist/ and dist-gui/ per platform, launcher files on Linux only"

bash "$here/release-dist.sh" package "$art" "$v" >/dev/null
EXPECT_DMG=0 bash "$here/verify-release-assets.sh" "$art" "$v" >/dev/null || fail "the packaged release does not verify"
ok "package: both apps, five platforms, checksums; the verifier is satisfied"

# With signed macOS apps among the artifacts.
for p in macos-x64 macos-arm64; do mkdir -p "$art/dmg-$p"; echo "dmg $p" >"$art/dmg-$p/kimun-gui-$v-$p.dmg"; done
bash "$here/release-dist.sh" package "$art" "$v" >/dev/null
EXPECT_DMG=1 bash "$here/verify-release-assets.sh" "$art" "$v" >/dev/null || fail "the release with .dmg files does not verify"
grep -q "kimun-gui-$v-macos-arm64.dmg" "$art/checksums-sha256.txt" || fail "a .dmg is not checksummed"
ok "package: signed macOS .dmg files are copied in and checksummed"

# A stable download (no version in the name) per archive and .dmg: the same bytes, the
# same checksum, and none for the raw binaries.
for p in linux-x64 linux-arm64 macos-x64 macos-arm64 windows-x64; do
  arc="tar.gz"; case "$p" in windows-*) arc="zip" ;; esac
  for app in kimun kimun-gui; do
    cmp -s "$art/$app-$v-$p.$arc" "$art/$app-$p.$arc" || fail "$app-$p.$arc is missing or differs from its versioned file"
    a="$(awk -v f="$app-$p.$arc" '$2 == f {print $1}' "$art/checksums-sha256.txt")"
    b="$(awk -v f="$app-$v-$p.$arc" '$2 == f {print $1}' "$art/checksums-sha256.txt")"
    [ "${#a}" = 64 ] && [ "$a" = "$b" ] || fail "$app-$p.$arc has no checksum equal to its versioned file's"
  done
done
for p in macos-x64 macos-arm64; do
  cmp -s "$art/kimun-gui-$v-$p.dmg" "$art/kimun-gui-$p.dmg" || fail "kimun-gui-$p.dmg is missing or differs from its versioned file"
  grep -q "  kimun-gui-$p.dmg\$" "$art/checksums-sha256.txt" || fail "kimun-gui-$p.dmg is not checksummed"
done
[ ! -e "$art/kimun-macos-arm64.dmg" ] || fail "the terminal app got a .dmg stable download"
for f in "$art"/kimun-*-linux-x64 "$art"/kimun-*-windows-x64.exe; do
  case "$(basename "$f")" in *"-$v-"*) ;; *) fail "a raw binary got a stable download: $f" ;; esac
done
[ "$(grep -c '  kimun-gui-linux-x64.tar.gz$' "$art/checksums-sha256.txt")" = 1 ] || fail "a stable download is checksummed more than once"
ok "stable downloads: same bytes and checksum as the versioned archives, none for raw binaries"

# The Homebrew job (build.yml, publish-homebrew) picks the terminal app's checksum by
# pattern. The stable downloads share the platform suffix, so each platform must still
# match exactly one line. Keep this pattern the same as the workflow's.
for p in macos-arm64 macos-x64 linux-x64 linux-arm64; do
  n="$(grep -cE "  kimun-[0-9][^ ]*-$p\.tar\.gz\$" "$art/checksums-sha256.txt")"
  [ "$n" = 1 ] || fail "the Homebrew pattern matches $n lines for $p, not 1"
done
ok "the Homebrew checksum pattern still finds one line per platform"

# Without signed macOS apps there is no .dmg stable download.
nodmg="$work/nodmg"; mkdir -p "$nodmg"
for d in "$art"/dist-* "$art"/distgui-*; do cp -r "$d" "$nodmg/"; done
bash "$here/release-dist.sh" package "$nodmg" "$v" >/dev/null
[ -z "$(ls "$nodmg"/*.dmg 2>/dev/null)" ] || fail "a .dmg exists without a signed app"
ok "no signed app: no .dmg stable download"

# The checksum lines are exactly what the installer and the updater read.
line="$(awk -v f="kimun-gui-$v-linux-x64.tar.gz" '$2 == f {print $1}' "$art/checksums-sha256.txt")"
[ "${#line}" = 64 ] || fail "no checksum the installer can find"
ok "checksums are two-column sha256sum lines, found by exact name"

# Missing input is an error, not an empty release.
empty="$work/empty"; mkdir -p "$empty"
if bash "$here/release-dist.sh" package "$empty" "$v" >/dev/null 2>&1; then fail "an empty artifacts dir packaged"; fi
ok "no build artifacts: an error"
echo "release-dist.sh behaves"
