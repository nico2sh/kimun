#!/usr/bin/env bash
# Assemble the desktop app's macOS bundle: Kimun.app, from the release binary.
#
#   macos-bundle.sh BINARY VERSION OUT_DIR      -> OUT_DIR/Kimun.app
#
# The bundle holds the binary, the icon and an Info.plist made from
# assets/packaging/macos/Info.plist.in. The icon is two files: kimun.icns, which every macOS
# reads, and Assets.car compiled by `actool` (Xcode 26) from the Icon Composer file kimun.icon,
# which macOS 26 (Tahoe) draws as its own glass icon. Without the .car, Tahoe puts the .icns in
# a gray frame. Where actool is missing (no Xcode 26) the .car is left out, with a warning. It is unsigned: signing,
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
if [ -d "$pkg/kimun.icon" ] && command -v xcrun >/dev/null 2>&1 && xcrun --find actool >/dev/null 2>&1; then
  actool_out="$(mktemp -d)"
  trap 'rm -rf "$actool_out"' EXIT
  # The compiled icon goes to Resources (Assets.car); the partial plist is not used: the
  # keys it would hold (CFBundleIconName) are in Info.plist.in already.
  xcrun actool "$pkg/kimun.icon" --compile "$app/Contents/Resources" \
    --output-format human-readable-text --notices --warnings --errors \
    --output-partial-info-plist "$actool_out/partial.plist" \
    --app-icon kimun --include-all-app-icons --enable-on-demand-resources NO \
    --development-region en --target-device mac --minimum-deployment-target 11.0 \
    --platform macosx >"$actool_out/log.txt" 2>&1 \
    || { cat "$actool_out/log.txt" >&2; echo "actool failed on $pkg/kimun.icon" >&2; exit 1; }
  [ -f "$app/Contents/Resources/Assets.car" ] \
    || { cat "$actool_out/log.txt" >&2; echo "actool wrote no Assets.car" >&2; exit 1; }
elif [ -d "$pkg/kimun.icon" ]; then
  echo "warning: actool (Xcode 26) not found: no Assets.car, macOS 26 will frame the icon" >&2
fi
sed -e "s|@VERSION@|$version|g" -e "s|@BUILD@|$build|g" "$pkg/Info.plist.in" >"$app/Contents/Info.plist"
printf 'APPL????' >"$app/Contents/PkgInfo"
echo "assembled $app (version $version)"
