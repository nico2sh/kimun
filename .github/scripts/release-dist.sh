#!/usr/bin/env bash
# The two shell halves of building a release's assets, shared by the release workflow
# and the local rehearsal (rehearse-release.sh), so what is rehearsed is what runs.
#
#   release-dist.sh assemble PLATFORM BIN_DIR OUT
#       Lay out one platform's build as the build job uploads it: OUT/dist (the
#       terminal app) and OUT/dist-gui (the desktop app: its binary, and on Linux a
#       share/ tree with the launcher entry and icons, mirroring an install prefix).
#       BIN_DIR holds the built kimun[.exe] and kimun-gui[.exe].
#
#   release-dist.sh package ARTIFACTS_DIR VERSION
#       In ARTIFACTS_DIR, which holds dist-<platform>/, distgui-<platform>/ and
#       optionally dmg-<platform>/ directories (the downloaded build artifacts): make
#       every release asset (a raw binary and an archive per app and platform, the
#       signed macOS .dmg files when there are any) and checksums-sha256.txt.
#       Names: kimun[-gui]-<version>-<platform>[.exe|.tar.gz|.zip|.dmg].
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"

assemble() {
  local platform="${1:?platform}" bin_dir="${2:?bin dir}" out="${3:?out dir}" ext=""
  case "$platform" in windows-*) ext=".exe" ;; esac
  mkdir -p "$out/dist" "$out/dist-gui"
  cp "$bin_dir/kimun$ext" "$out/dist/"
  cp "$bin_dir/kimun-gui$ext" "$out/dist-gui/"
  case "$platform" in
    linux-*)
      mkdir -p "$out/dist-gui/share/applications"
      cp "$root/assets/packaging/linux/kimun.desktop" "$out/dist-gui/share/applications/"
      cp -r "$root/assets/packaging/linux/icons" "$out/dist-gui/share/icons"
      ;;
  esac
}

package() {
  local dir="${1:?artifacts dir}" version="${2:?version}"
  local d platform base
  shopt -s nullglob
  cd "$dir"

  # One app's assets from its <prefix>-<platform>/ directories.
  make_assets() { # PREFIX APP
    local prefix="$1" app="$2" dirs
    dirs=("$prefix"-*/)
    if [[ ${#dirs[@]} -eq 0 ]]; then
      echo "::error::no $prefix-* build artifacts found"
      exit 1
    fi
    for d in "${dirs[@]}"; do
      platform="${d%/}"
      platform="${platform#"$prefix"-}"
      base="$app-${version}-${platform}"
      # The raw, uncompressed binary is what in-app self-update downloads: checksum-
      # verified and swapped in place, no archive to unpack.
      if [[ "$platform" == windows-* ]]; then
        (cd "$d" && zip -qr "../${base}.zip" .)
        cp "${d}${app}.exe" "${base}.exe"
      else
        tar -czf "${base}.tar.gz" -C "$d" .
        cp "${d}${app}" "${base}"
      fi
    done
  }
  make_assets dist kimun
  make_assets distgui kimun-gui

  # The signed macOS app, when there is one (artifacts dmg-<platform>).
  for f in dmg-*/*.dmg; do
    cp "$f" .
  done

  # Archives, raw binaries and .dmg files (nullglob drops any absent). `kimun-*`
  # covers both apps: kimun-<v>-... and kimun-gui-<v>-...
  local files=(
    kimun-*.zip kimun-*.tar.gz kimun-*.dmg
    kimun-*-linux-x64 kimun-*-linux-arm64
    kimun-*-macos-x64 kimun-*-macos-arm64 kimun-*-windows-x64.exe
  )
  if [[ ${#files[@]} -eq 0 ]]; then
    echo "::error::No assets found to checksum"
    exit 1
  fi
  # Both `sha256sum` and the macOS/BSD `shasum -a 256` write the same two-column format.
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${files[@]}" >checksums-sha256.txt
  else
    shasum -a 256 "${files[@]}" >checksums-sha256.txt
  fi
  ls -lh kimun-* checksums-sha256.txt
}

case "${1:-}" in
  assemble) shift; assemble "$@" ;;
  package) shift; package "$@" ;;
  *) sed -n '2,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//' | head -n -1; exit 2 ;;
esac
