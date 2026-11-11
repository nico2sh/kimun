#!/usr/bin/env bash
# Rehearse the release on this machine, end to end, for this machine's platform:
#
#   rehearse-release.sh [VERSION]       (default 0.0.0-rehearsal)
#
#   1. build both apps in release mode (SKIP_BUILD=1 reuses target/release);
#   2. lay them out and package them with the release workflow's own scripts
#      (release-dist.sh), then run the asset verifier on the result;
#   3. install both apps from those archives with install.sh into a scratch home
#      (KIMUN_DOWNLOAD_BASE=file://...), checking the installed files;
#   4. start the installed desktop app for a few seconds (gui-smoke.sh; needs a display
#      or xvfb-run, skipped with a note otherwise) and run the installed terminal app.
#
# Nothing leaves the machine, and your real config and notes are not touched. What it cannot
# show: the other platforms, GitHub's runners, the release page, Apple's signing.
set -euo pipefail
cd "$(dirname "$0")/../.."

version="${1:-0.0.0-rehearsal}"
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) platform=linux-x64 ;;
  Linux-aarch64 | Linux-arm64) platform=linux-arm64 ;;
  Darwin-arm64) platform=macos-arm64 ;;
  Darwin-x86_64) platform=macos-x64 ;;
  *) echo "no rehearsal for $(uname -s)-$(uname -m)" >&2; exit 2 ;;
esac

if [ "${SKIP_BUILD:-0}" != 1 ]; then
  echo "== building both apps (release)"
  cargo build --release --bin kimun --bin kimun-gui
fi
bin_dir="target/release"
[ -x "$bin_dir/kimun" ] && [ -x "$bin_dir/kimun-gui" ] || { echo "no release binaries in $bin_dir" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
art="$work/release-artifacts"
mkdir -p "$art"

echo "== assembling and packaging ($platform, version $version)"
bash .github/scripts/release-dist.sh assemble "$platform" "$bin_dir" "$work/out" >/dev/null
mv "$work/out/dist" "$art/dist-$platform"
mv "$work/out/dist-gui" "$art/distgui-$platform"
bash .github/scripts/release-dist.sh package "$art" "$version" >/dev/null
# shellcheck disable=SC2012  # only reads our own release file names
(cd "$art" && ls -lh kimun-* | awk '{print "   " $5 "  " $9}')
PLATFORMS="$platform" EXPECT_DMG=0 bash .github/scripts/verify-release-assets.sh "$art" "$version"

echo "== installing from the archives (scratch home)"
home="$work/home"
mkdir -p "$home"
install_app() { # --tui | --gui: one app, so a headless runner cannot skip the desktop app
  HOME="$home" KIMUN_INSTALL_DIR="$home/bin" KIMUN_VERSION="$version" KIMUN_DOWNLOAD_BASE="file://$art" \
    sh install.sh "$@" >"$work/install.log" 2>&1 || { cat "$work/install.log"; echo "install failed" >&2; exit 1; }
}
install_app --tui
install_app --gui
[ -x "$home/bin/kimun" ] && [ -x "$home/bin/kimun-gui" ] || { echo "the installed binaries are missing" >&2; exit 1; }
cmp -s "$home/bin/kimun" "$bin_dir/kimun" || { echo "the installed terminal app differs from the build" >&2; exit 1; }
cmp -s "$home/bin/kimun-gui" "$bin_dir/kimun-gui" || { echo "the installed desktop app differs from the build" >&2; exit 1; }
grep -q 'channel = "script"' "$home/.config/kimun/install.toml" || { echo "no install marker" >&2; exit 1; }
if [ "$(uname -s)" = Linux ]; then
  entry="$home/.local/share/applications/kimun.desktop"
  [ -f "$entry" ] || { echo "no launcher entry" >&2; exit 1; }
  grep -qx "Exec=\"$home/bin/kimun-gui\" %F" "$entry" || { echo "launcher Exec not absolute" >&2; exit 1; }
  [ -f "$home/.local/share/icons/hicolor/512x512/apps/kimun.png" ] || { echo "no icon" >&2; exit 1; }
  echo "   launcher entry and icons installed"
fi
echo "   installed byte-for-byte what was built; marker written"

echo "== running what was installed"
echo "   terminal app: $(HOME="$home" "$home/bin/kimun" --version 2>&1 | head -1)"
# macOS has its own window server: no $DISPLAY involved (needs a logged-in desktop session).
if [ "$(uname -s)" = "Darwin" ] || [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] || command -v xvfb-run >/dev/null; then
  bash .github/scripts/gui-smoke.sh "$home/bin/kimun-gui" "${SMOKE_SECONDS:-5}"
else
  echo "   desktop app: skipped (no display and no xvfb-run; on macOS run from a desktop session, not over ssh)"
fi

echo "rehearsal ok: $platform, $version"
