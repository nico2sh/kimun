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

# The checksum lines are exactly what the installer and the updater read.
line="$(awk -v f="kimun-gui-$v-linux-x64.tar.gz" '$2 == f {print $1}' "$art/checksums-sha256.txt")"
[ "${#line}" = 64 ] || fail "no checksum the installer can find"
ok "checksums are two-column sha256sum lines, found by exact name"

# Missing input is an error, not an empty release.
empty="$work/empty"; mkdir -p "$empty"
if bash "$here/release-dist.sh" package "$empty" "$v" >/dev/null 2>&1; then fail "an empty artifacts dir packaged"; fi
ok "no build artifacts: an error"
echo "release-dist.sh behaves"
