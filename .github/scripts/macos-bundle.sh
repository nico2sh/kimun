#!/usr/bin/env bash
# Assemble the desktop app's macOS bundle: Kimun.app, from the release binary.
#
#   macos-bundle.sh BINARY VERSION OUT_DIR      -> OUT_DIR/Kimun.app
#
# The bundle holds the binary, the icon (assets/packaging/macos/kimun.icns) and an
# Info.plist made from assets/packaging/macos/Info.plist.in. It is unsigned: signing,
# notarizing and the .dmg are macos-sign.sh's. Plain shell, so it runs (and is tested)
# anywhere.
set -euo pipefail

binary="${1:?usage: macos-bundle.sh BINARY VERSION OUT_DIR}"
version="${2:?usage: macos-bundle.sh BINARY VERSION OUT_DIR}"
out="${3:?usage: macos-bundle.sh BINARY VERSION OUT_DIR}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
pkg="$root/assets/packaging/macos"

[ -f "$binary" ] || { echo "no binary at $binary" >&2; exit 1; }

# CFBundleVersion is three integers; a pre-release suffix (0.25.0-beta.1) lives only in
# the short version string.
build="${version%%-*}"
case "$build" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "version '$version' does not start with X.Y.Z" >&2; exit 1 ;;
esac

app="$out/Kimun.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/kimun-gui"
chmod +x "$app/Contents/MacOS/kimun-gui"
cp "$pkg/kimun.icns" "$app/Contents/Resources/kimun.icns"
sed -e "s|@VERSION@|$version|g" -e "s|@BUILD@|$build|g" "$pkg/Info.plist.in" >"$app/Contents/Info.plist"
printf 'APPL????' >"$app/Contents/PkgInfo"
echo "assembled $app (version $version)"
