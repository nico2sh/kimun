#!/usr/bin/env bash
# Check a release's assets before anything is published (and in the manual dry run).
#
#   verify-release-assets.sh DIR VERSION
#
# For each of the five platforms and each app (the terminal app `kimun`, the desktop
# app `kimun-gui`): the raw binary and the archive exist and are not empty, both
# have a line in checksums-sha256.txt, and the archive holds the binary at its root.
# The desktop app's Linux archives also carry the launcher entry and the icons
# (a `share/` tree that mirrors an install prefix). The desktop app's signed macOS
# `kimun-gui-<v>-macos-<arch>.dmg` files are checked too: required when EXPECT_DMG=1
# (the Apple signing secrets exist), optional otherwise, but never without a checksum.
# Finally every checksum is recomputed. Fails with a ::error:: line per problem.
set -uo pipefail

dir="${1:?usage: verify-release-assets.sh DIR VERSION}"
version="${2:?usage: verify-release-assets.sh DIR VERSION}"
cd "$dir" || exit 1

# PLATFORMS narrows the check to what a local rehearsal built (default: all five).
platforms="${PLATFORMS:-linux-x64 linux-arm64 macos-x64 macos-arm64 windows-x64}"
failed=0
err() { echo "::error::$*"; failed=1; }

# The paths in an archive, without a leading ./
listing() {
  case "$1" in
    *.zip) unzip -Z1 "$1" ;;
    *.tar.gz) tar -tzf "$1" ;;
  esac | sed 's|^\./||'
}

[ -s checksums-sha256.txt ] || { err "checksums-sha256.txt is missing or empty"; exit 1; }

expected=0
for p in $platforms; do
  ext=""
  arc="tar.gz"
  case "$p" in windows-*) ext=".exe"; arc="zip" ;; esac
  for app in kimun kimun-gui; do
    base="$app-$version-$p"
    for f in "$base$ext" "$base.$arc"; do
      expected=$((expected + 1))
      [ -s "$f" ] || { err "missing or empty asset: $f"; continue; }
      grep -qE "^[0-9a-f]{64}  $f\$" checksums-sha256.txt || err "no checksum line for $f"
    done
    if [ -s "$base.$arc" ]; then
      listing "$base.$arc" | grep -qx "$app$ext" || err "$base.$arc does not hold $app$ext at its root"
    fi
  done
  case "$p" in
    linux-*)
      gui="kimun-gui-$version-$p.$arc"
      if [ -s "$gui" ]; then
        files="$(listing "$gui")"
        for want in share/applications/kimun.desktop share/icons/hicolor/512x512/apps/kimun.png share/icons/hicolor/16x16/apps/kimun.png; do
          echo "$files" | grep -qx "$want" || err "$gui does not hold $want"
        done
      fi
      ;;
  esac
done

for p in macos-x64 macos-arm64; do
  f="kimun-gui-$version-$p.dmg"
  if [ -e "$f" ]; then
    [ -s "$f" ] || err "empty asset: $f"
    grep -qE "^[0-9a-f]{64}  $f\$" checksums-sha256.txt || err "no checksum line for $f"
  elif [ "${EXPECT_DMG:-0}" = 1 ]; then
    err "missing asset: $f (the Apple signing secrets are set, so the signed app is expected)"
  fi
done

# What the raw binaries really are, when `file` can tell (a placeholder in a test is
# skipped): the right system and CPU for the name, and on Windows the desktop app a GUI
# program (no console window) and the terminal app a console one. A cross-build that
# came out for the wrong target, or a missing windows_subsystem, is caught here.
if command -v file >/dev/null 2>&1; then
  for p in $platforms; do
    ext=""
    case "$p" in windows-*) ext=".exe" ;; esac
    for app in kimun kimun-gui; do
      f="$app-$version-$p$ext"
      [ -s "$f" ] || continue
      kind="$(file -b "$f")"
      case "$p:$kind" in
        linux-x64:ELF*) case "$kind" in *x86-64*) ;; *) err "$f is not an x86-64 ELF: $kind" ;; esac ;;
        linux-arm64:ELF*) case "$kind" in *aarch64*) ;; *) err "$f is not an aarch64 ELF: $kind" ;; esac ;;
        macos-x64:Mach-O*) case "$kind" in *x86_64*) ;; *) err "$f is not an x86_64 Mach-O: $kind" ;; esac ;;
        macos-arm64:Mach-O*) case "$kind" in *arm64*) ;; *) err "$f is not an arm64 Mach-O: $kind" ;; esac ;;
        windows-x64:PE32+*)
          case "$kind" in *x86-64*) ;; *) err "$f is not an x86-64 PE: $kind" ;; esac
          if [ "$app" = kimun-gui ]; then
            case "$kind" in *"(GUI)"*) ;; *) err "$f is a console program: the desktop app must be a GUI program (windows_subsystem): $kind" ;; esac
          else
            case "$kind" in *"(console)"*) ;; *) err "$f should be a console program: $kind" ;; esac
          fi
          ;;
        *:ELF* | *:Mach-O* | *:PE32*) err "$f does not look like a $p binary: $kind" ;;
      esac
    done
  done
fi

# Nothing else under the app's names: a stray or misnamed asset would be uploaded too.
for f in kimun-*; do
  [ -e "$f" ] || continue
  known=0
  for p in $platforms; do
    ext=""
    arc="tar.gz"
    case "$p" in windows-*) ext=".exe"; arc="zip" ;; esac
    for app in kimun kimun-gui; do
      case "$f" in "$app-$version-$p$ext" | "$app-$version-$p.$arc") known=1 ;; esac
      case "$f" in "kimun-gui-$version-$p.dmg") known=1 ;; esac
    done
  done
  [ "$known" = 1 ] || err "unexpected asset: $f"
done

# Every digest is recomputed.
sha256sum --check --quiet checksums-sha256.txt || err "a checksum does not match its file"

if [ "$failed" = 0 ]; then
  echo "release assets for $version: $expected files, checksums good"
fi
exit "$failed"
