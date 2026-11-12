#!/usr/bin/env bash
# Self-test for verify-release-assets.sh: a faithful fake release passes, and each
# kind of damage is caught. Run: bash .github/scripts/test-verify-release-assets.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
v=9.9.9
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

make_release() { # DIR: the assets as create-release makes them
  local out="$1" p ext arc app stage
  mkdir -p "$out"
  for p in linux-x64 linux-arm64 macos-x64 macos-arm64 windows-x64; do
    ext=""; arc="tar.gz"
    case "$p" in windows-*) ext=".exe"; arc="zip" ;; esac
    for app in kimun kimun-gui; do
      stage="$(mktemp -d)"
      echo "bin $app $p" >"$stage/$app$ext"
      if [ "$app" = kimun-gui ]; then case "$p" in linux-*)
        mkdir -p "$stage/share/applications" "$stage/share/icons/hicolor/512x512/apps" "$stage/share/icons/hicolor/16x16/apps"
        : >"$stage/share/applications/kimun.desktop"
        echo png >"$stage/share/icons/hicolor/512x512/apps/kimun.png"
        echo png >"$stage/share/icons/hicolor/16x16/apps/kimun.png"
      ;; esac; fi
      if [ "$arc" = zip ]; then (cd "$stage" && zip -qr "$out/$app-$v-$p.zip" .); else tar -czf "$out/$app-$v-$p.tar.gz" -C "$stage" .; fi
      cp "$out/$app-$v-$p.$arc" "$out/$app-$p.$arc"
      cp "$stage/$app$ext" "$out/$app-$v-$p$ext"
      rm -rf "$stage"
    done
  done
  (cd "$out" && sha256sum kimun-* >checksums-sha256.txt)
}

expect_fail() { # NAME DIR: verification must fail
  if bash "$here/verify-release-assets.sh" "$2" "$v" >/dev/null 2>&1; then
    echo "FAIL: damage not caught: $1"; exit 1
  fi
  echo "ok: caught: $1"
}

good="$work/good"; make_release "$good"
bash "$here/verify-release-assets.sh" "$good" "$v" && echo "ok: a faithful release passes"

d="$work/missing-gui-raw"; cp -r "$good" "$d"; rm "$d/kimun-gui-$v-linux-arm64"; expect_fail "a missing desktop binary" "$d"
d="$work/missing-checksum"; cp -r "$good" "$d"; grep -v "kimun-gui-$v-macos-x64.tar.gz" "$good/checksums-sha256.txt" >"$d/checksums-sha256.txt"; expect_fail "a missing checksum line" "$d"
d="$work/bad-digest"; cp -r "$good" "$d"; echo tampered >"$d/kimun-$v-windows-x64.exe"; expect_fail "a changed file" "$d"
d="$work/stray"; cp -r "$good" "$d"; echo x >"$d/kimun-gui-$v-linux-x64.deb"; (cd "$d" && sha256sum kimun-* >checksums-sha256.txt); expect_fail "an unexpected asset" "$d"
d="$work/no-desktop-files"; cp -r "$good" "$d"; s="$(mktemp -d)"; echo bin >"$s/kimun-gui"; tar -czf "$d/kimun-gui-$v-linux-x64.tar.gz" -C "$s" .; cp "$d/kimun-gui-$v-linux-x64.tar.gz" "$d/kimun-gui-linux-x64.tar.gz"; (cd "$d" && sha256sum kimun-* >checksums-sha256.txt); expect_fail "a Linux desktop archive without the launcher files" "$d"
d="$work/wrong-root"; cp -r "$good" "$d"; s="$(mktemp -d)"; mkdir "$s/sub"; echo bin >"$s/sub/kimun"; tar -czf "$d/kimun-$v-macos-arm64.tar.gz" -C "$s" .; cp "$d/kimun-$v-macos-arm64.tar.gz" "$d/kimun-macos-arm64.tar.gz"; (cd "$d" && sha256sum kimun-* >checksums-sha256.txt); expect_fail "a binary not at the archive root" "$d"
# The stable downloads (ADR-0057): present, and the same file as the versioned archive.
d="$work/no-stable"; cp -r "$good" "$d"; rm "$d/kimun-gui-windows-x64.zip"; (cd "$d" && sha256sum kimun-* >checksums-sha256.txt); expect_fail "a missing stable download" "$d"
d="$work/stable-no-sum"; cp -r "$good" "$d"; grep -v "  kimun-linux-arm64.tar.gz\$" "$good/checksums-sha256.txt" >"$d/checksums-sha256.txt"; expect_fail "a stable download without a checksum line" "$d"
d="$work/stable-differs"; cp -r "$good" "$d"; echo other >"$d/kimun-linux-x64.tar.gz"; (cd "$d" && sha256sum kimun-* >checksums-sha256.txt); expect_fail "a stable download that is not the versioned archive" "$d"
# A real binary for the wrong platform is caught (a cross-build gone wrong).
if command -v file >/dev/null 2>&1 && file -b /bin/true | grep -q ELF; then
  case "$(uname -m)" in x86_64) wrong=linux-arm64 ;; *) wrong=linux-x64 ;; esac
  d="$work/wrong-arch"; cp -r "$good" "$d"; cp /bin/true "$d/kimun-gui-$v-$wrong"
  (cd "$d" && sha256sum kimun-* >checksums-sha256.txt)
  expect_fail "a binary built for another CPU than its name says" "$d"
fi

# The signed macOS app: optional unless expected, never without a checksum.
d="$work/with-dmg"; cp -r "$good" "$d"
for p in macos-x64 macos-arm64; do echo dmg >"$d/kimun-gui-$v-$p.dmg"; cp "$d/kimun-gui-$v-$p.dmg" "$d/kimun-gui-$p.dmg"; done
(cd "$d" && sha256sum kimun-* >checksums-sha256.txt)
EXPECT_DMG=1 bash "$here/verify-release-assets.sh" "$d" "$v" >/dev/null && echo "ok: both .dmg files present, as expected"
bash "$here/verify-release-assets.sh" "$d" "$v" >/dev/null && echo "ok: .dmg files are fine when not required"
if EXPECT_DMG=1 bash "$here/verify-release-assets.sh" "$good" "$v" >/dev/null 2>&1; then echo "FAIL: missing .dmg not caught when expected"; exit 1; fi
echo "ok: caught: .dmg files expected but absent"
d="$work/dmg-no-stable"; cp -r "$work/with-dmg" "$d"; rm "$d/kimun-gui-macos-arm64.dmg"; (cd "$d" && sha256sum kimun-* >checksums-sha256.txt)
expect_fail "a .dmg without its stable download" "$d"
d="$work/dmg-stable-differs"; cp -r "$work/with-dmg" "$d"; echo other >"$d/kimun-gui-macos-x64.dmg"; (cd "$d" && sha256sum kimun-* >checksums-sha256.txt)
expect_fail "a .dmg stable download that is not the versioned .dmg" "$d"
d="$work/stable-dmg-alone"; cp -r "$work/with-dmg" "$d"; rm "$d/kimun-gui-$v-macos-x64.dmg"; (cd "$d" && sha256sum kimun-* >checksums-sha256.txt)
expect_fail "a .dmg stable download with no versioned .dmg" "$d"
d="$work/one-dmg"; cp -r "$work/with-dmg" "$d"; rm "$d/kimun-gui-$v-macos-x64.dmg" "$d/kimun-gui-macos-x64.dmg"; (cd "$d" && sha256sum kimun-* >checksums-sha256.txt)
if EXPECT_DMG=1 bash "$here/verify-release-assets.sh" "$d" "$v" >/dev/null 2>&1; then echo "FAIL: one missing .dmg not caught"; exit 1; fi
echo "ok: caught: only one of the two .dmg files"
d="$work/dmg-no-sum"; cp -r "$work/with-dmg" "$d"; (cd "$d" && grep -v '\.dmg' checksums-sha256.txt >c && mv c checksums-sha256.txt)
expect_fail "a .dmg without a checksum line" "$d"
echo "all verify-release-assets checks behave"
