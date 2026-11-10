#!/usr/bin/env bash
# Compile the Icon Composer file kimun.icon into Assets.car, the icon macOS 26 (Tahoe) draws as
# glass. Run it on a Mac with Xcode 26 whenever kimun.icon changes, and commit the two files it
# writes:
#
#   assets/packaging/macos/make-icon.sh          -> Assets.car and Assets.car.source-sha256
#   assets/packaging/macos/make-icon.sh hash     -> print the hash of kimun.icon's files
#
# The release copies the committed Assets.car into the app (macos-bundle.sh), so what ships is
# the file that was tried. Assets.car.source-sha256 records which kimun.icon it was built from:
# test-macos-packaging.sh compares it with the current files, on any machine, so an edit of
# kimun.icon without a rebuild fails CI.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
icon="$here/kimun.icon"

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }

# One hash over every file of the icon: names and contents, in a fixed order.
source_hash() {
  (cd "$icon" && find . -type f ! -name .DS_Store | LC_ALL=C sort | while read -r f; do sha256 "$f"; done) | sha256 | awk '{print $1}'
}

if [ "${1:-}" = hash ]; then
  source_hash
  exit 0
fi

command -v xcrun >/dev/null 2>&1 && xcrun --find actool >/dev/null 2>&1 \
  || { echo "needs actool: run this on a Mac with Xcode 26 installed" >&2; exit 1; }

out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT
xcrun actool "$icon" --compile "$out" \
  --output-format human-readable-text --notices --warnings --errors \
  --output-partial-info-plist "$out/partial.plist" \
  --app-icon kimun --include-all-app-icons --enable-on-demand-resources NO \
  --development-region en --target-device mac --minimum-deployment-target 11.0 \
  --platform macosx >"$out/log.txt" 2>&1 \
  || { cat "$out/log.txt" >&2; echo "actool failed on $icon" >&2; exit 1; }
[ -f "$out/Assets.car" ] || { cat "$out/log.txt" >&2; echo "actool wrote no Assets.car" >&2; exit 1; }

cp "$out/Assets.car" "$here/Assets.car"
source_hash >"$here/Assets.car.source-sha256"
echo "wrote $here/Assets.car (source $(cat "$here/Assets.car.source-sha256"))"
echo "commit Assets.car and Assets.car.source-sha256 together with the kimun.icon change"
