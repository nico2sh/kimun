#!/usr/bin/env bash
# Self-test for install.sh against a fake release served from a local directory
# (file:// URLs, a pinned version): the terminal app, the desktop app and its
# Linux launcher files, the shared marker, a bad checksum, an archive with no
# launcher files, the options. The macOS desktop app is covered with stubbed hdiutil, ditto and codesign,
# so the real tools are not exercised here (see rehearse-release.sh for that).
# Run: bash .github/scripts/test-install.sh
# shellcheck disable=SC2015  # `[ A ] && [ B ] || fail ...` is intended here: fail when either test is false
set -euo pipefail
cd "$(dirname "$0")/../.."
if [ "$(uname -s)" != "Linux" ]; then echo "skipped: not Linux"; exit 0; fi
case "$(uname -m)" in
  x86_64 | amd64) platform=linux-x64 ;;
  aarch64 | arm64) platform=linux-arm64 ;;
  *) echo "skipped: unsupported architecture"; exit 0 ;;
esac

v=9.9.9
# A default install takes the desktop app when there is a display: tests that want that say so, and
# the headless ones unset these.
export DISPLAY="${DISPLAY:-:0}"
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
install_in "$h" "$rel" --tui >/dev/null
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
# shellcheck disable=SC2015  # the first test is a plain grep: the failure message only runs when a check fails
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
out="$(install_in "$h" "$ng" --tui 2>&1)" || fail "the terminal app should still install from that release"
ok "a release without a desktop build: a plain message, the terminal app unaffected"
h="$work/h7b"; mkdir -p "$h"
if out="$(HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$work/nowhere" sh install.sh 2>&1)"; then fail "a missing release installed"; fi
echo "$out" | grep -q "failed to download" || fail "missing release error: $out"
ok "a missing release: a failed-download message"

# 8. Platform detection with a fake uname / sysctl on PATH: Rosetta, Intel Mac, Windows shells.
fakebin() { # DIR OS ARCH ARM64 TRANSLATED
  mkdir -p "$1"
  # shellcheck disable=SC2016  # the single quotes are on purpose: this is the text of a stub script
  printf '#!/bin/sh\ncase "$1" in -s) echo %s;; -m) echo %s;; *) echo %s;; esac\n' "$2" "$3" "$2" >"$1/uname"
  # shellcheck disable=SC2016  # the single quotes are on purpose: this is the text of a stub script
  printf '#!/bin/sh\ncase "$2" in hw.optional.arm64) echo %s;; sysctl.proc_translated) echo %s;; *) exit 1;; esac\n' "$4" "$5" >"$1/sysctl"
  chmod +x "$1/uname" "$1/sysctl"
}
detected() { # DIR -> the platform install.sh picked
  local h; h="$(mktemp -d -p "$work")"
  PATH="$1:$PATH" HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$rel" \
    sh install.sh --tui 2>&1 | sed -n 's/^Detected platform: //p'
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

# 8b. The macOS desktop app: --gui on a (fake) Mac takes the .dmg, mounts it, copies Kimun.app. The
# stubs: uname/sysctl say Darwin; hdiutil "mounts" a .dmg that is a tar of its contents; ditto is cp;
# codesign passes unless CODESIGN_FAIL is set, and, like the real one, also fails when the bundle
# path does not end in .app or when no -R requirement is given.
macbin="$work/macbin"; fakebin "$macbin" Darwin arm64 1 0
cat >"$macbin/hdiutil" <<'STUB'
#!/bin/sh
case "$1" in
  attach) for last; do :; done; while [ $# -gt 0 ]; do [ "$1" = -mountpoint ] && mp="$2"; shift; done
          tar -xf "$last" -C "$mp" || exit 1; echo "$mp" >"${HDIUTIL_LOG:-/dev/null}" ;;
  detach) echo "detach $*" >>"${HDIUTIL_LOG:-/dev/null}" ;;
esac
STUB
# shellcheck disable=SC2016  # the single quotes are on purpose: this is the text of a stub script
printf '#!/bin/sh\ncp -R "$1" "$2"\n' >"$macbin/ditto"
cat >"$macbin/codesign" <<'STUB'
#!/bin/sh
[ -z "${CODESIGN_FAIL:-}" ] || exit 1
req=0; for last; do [ "$prev" = -R ] && req=1; prev="$last"; done
[ "$req" = 1 ] || exit 1
case "$last" in *.app) exit 0 ;; *) exit 1 ;; esac
STUB
chmod +x "$macbin/hdiutil" "$macbin/ditto" "$macbin/codesign"
macrel="$work/macrel"; make_release "$macrel"
stage="$(mktemp -d)"; mkdir -p "$stage/Kimun.app/Contents/MacOS"; printf '#!/bin/sh\necho app\n' >"$stage/Kimun.app/Contents/MacOS/kimun-gui"; chmod +x "$stage/Kimun.app/Contents/MacOS/kimun-gui"
ln -s /Applications "$stage/Applications"
tar -cf "$macrel/kimun-gui-$v-macos-arm64.dmg" -C "$stage" .; rm -rf "$stage"
(cd "$macrel" && sha256sum kimun-*.tar.gz kimun-*.dmg >checksums-sha256.txt)
mac_install() { # HOME [args]
  local h="$1"; shift
  PATH="$macbin:$PATH" HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$macrel" \
    HDIUTIL_LOG="$h/hdiutil.log" sh install.sh --gui "$@"
}
h="$work/m1"; mkdir -p "$h"
mac_install "$h" >"$work/m1.out" 2>&1 || fail "macOS app install failed: $(cat "$work/m1.out")"
[ -x "$h/Applications/Kimun.app/Contents/MacOS/kimun-gui" ] || fail "Kimun.app not in ~/Applications: $(cat "$work/m1.out"; find "$h" -maxdepth 3)"
[ ! -e "$h/Applications/.Kimun.new" ] || fail "staging copy left behind"
[ ! -e "$h/bin/kimun-gui" ] || fail "the macOS app install also put a raw binary in place"
[ ! -e "$h/.config/kimun/install.toml" ] || fail "the macOS app install wrote a marker"
grep -q '^detach' "$h/hdiutil.log" || fail "the .dmg was not detached"
ok "macOS --gui: Kimun.app copied to ~/Applications, .dmg detached, no marker, no raw binary"
# A bad signature: refused, and the app already there is untouched.
h="$work/m2"; mkdir -p "$h/Applications/Kimun.app"; echo old >"$h/Applications/Kimun.app/old"
if out="$(CODESIGN_FAIL=1 mac_install "$h" 2>&1)"; then fail "a bad signature was accepted"; fi
echo "$out" | grep -q "not validly signed with an Apple Developer ID" || fail "bad signature said: $out"
[ -f "$h/Applications/Kimun.app/old" ] || fail "a bad signature replaced the installed app"
[ ! -e "$h/Applications/.Kimun.new" ] || fail "staging copy left after a bad signature"
ok "macOS --gui: a bad signature is refused and the installed app stays"
# A re-install replaces the app, and KIMUN_APP_DIR moves it.
h="$work/m3"; mkdir -p "$h/Applications/Kimun.app"; echo old >"$h/Applications/Kimun.app/old"
mac_install "$h" >/dev/null 2>&1 || fail "re-install failed"
[ ! -e "$h/Applications/Kimun.app/old" ] || fail "re-install kept stale files"
KIMUN_APP_DIR="$h/elsewhere" mac_install "$h" >/dev/null 2>&1 || fail "KIMUN_APP_DIR install failed"
[ -d "$h/elsewhere/Kimun.app" ] || fail "KIMUN_APP_DIR ignored"
ok "macOS --gui: a re-install replaces the app; KIMUN_APP_DIR is honored"
# An older raw kimun-gui (and its marker) is replaced by Kimun.app, not left as a second copy.
h="$work/mraw1"; mkdir -p "$h/bin" "$h/.config/kimun"; echo old >"$h/bin/kimun-gui"
printf 'channel = "script"\ninstall_dir = "%s/bin"\n' "$h" >"$h/.config/kimun/install.toml"
mac_install "$h" >"$work/mraw1.out" 2>&1 || fail "app install over a raw binary failed: $(cat "$work/mraw1.out")"
[ -x "$h/Applications/Kimun.app/Contents/MacOS/kimun-gui" ] || fail "Kimun.app missing after an upgrade from the raw binary"
[ ! -e "$h/bin/kimun-gui" ] || fail "the old raw kimun-gui was left next to Kimun.app"
[ ! -e "$h/.config/kimun/install.toml" ] || fail "the old marker was left after the raw binary went"
ok "macOS --gui: the old raw binary and its marker are removed when Kimun.app is installed"
# ... but the terminal app and its marker stay.
h="$work/mraw2"; mkdir -p "$h/bin" "$h/.config/kimun"; echo old >"$h/bin/kimun-gui"; echo t >"$h/bin/kimun"
printf 'channel = "script"\ninstall_dir = "%s/bin"\n' "$h" >"$h/.config/kimun/install.toml"
mac_install "$h" >/dev/null 2>&1 || fail "app install beside the terminal app failed"
[ ! -e "$h/bin/kimun-gui" ] || fail "the old raw kimun-gui was left"
[ -f "$h/bin/kimun" ] && [ -f "$h/.config/kimun/install.toml" ] || fail "the terminal app or its marker was removed"
ok "macOS --gui: the terminal app and its marker are kept"
# A corrupted .dmg: checksum mismatch, nothing installed.
cp -R "$macrel" "$work/macbad"; echo x >>"$work/macbad/kimun-gui-$v-macos-arm64.dmg"
h="$work/m4"; mkdir -p "$h"
if PATH="$macbin:$PATH" HOME="$h" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$work/macbad" sh install.sh --gui >/dev/null 2>&1; then fail "a corrupt .dmg installed"; fi
[ ! -e "$h/Applications/Kimun.app" ] || fail "a corrupt .dmg still installed"
ok "macOS --gui: a corrupt .dmg fails the checksum"
# A release with no .dmg: the raw binary, with a note.
rm "$macrel/kimun-gui-$v-macos-arm64.dmg"; (cd "$macrel" && sha256sum kimun-*.tar.gz >checksums-sha256.txt)
h="$work/m5"; mkdir -p "$h"
out="$(mac_install "$h" 2>&1)" || fail "no-.dmg release failed: $out"
[ -x "$h/bin/kimun-gui" ] || fail "no-.dmg release: raw binary missing"
echo "$out" | grep -q "no macOS app" || fail "no-.dmg release did not say so: $out"
ok "macOS --gui: a release with no .dmg falls back to the raw binary"
# macOS default run: the app and the terminal app; with no .dmg, no raw binary, a warning.
h="$work/m7"; mkdir -p "$h"
PATH="$macbin:$PATH" HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$macrel" \
  HDIUTIL_LOG="$h/hdiutil.log" sh install.sh >"$work/m7.out" 2>&1 || fail "macOS: a default install failed with no .dmg: $(cat "$work/m7.out")"
[ -x "$h/bin/kimun" ] || fail "macOS default, no .dmg: the terminal app is missing"
[ ! -e "$h/bin/kimun-gui" ] || fail "macOS default, no .dmg: a raw binary was installed"
grep -q "^warning: the desktop app was not installed" "$work/m7.out" || fail "macOS default, no .dmg: no warning: $(cat "$work/m7.out")"
ok "macOS default with no .dmg: terminal app, a warning, no raw binary"
# The terminal app on a Mac is not affected.
h="$work/m6"; mkdir -p "$h"
PATH="$macbin:$PATH" HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$macrel" sh install.sh --tui >/dev/null 2>&1 || fail "terminal install on a Mac failed"
[ -x "$h/bin/kimun" ] && [ ! -e "$h/Applications" ] || fail "terminal install on a Mac changed"
ok "macOS: the terminal app install is unchanged"

# 8c. --uninstall: removes what the installer put in place, keeps the user's data, keeps the marker
# while the other app is there. It finds the install dir from the marker, with no KIMUN_INSTALL_DIR.
uninstall_in() { # HOME [args...]
  local h="$1"; shift
  HOME="$h" sh install.sh --uninstall "$@"
}
h="$work/u1"; mkdir -p "$h"
install_in "$h" "$rel" >/dev/null
echo 'keep = "me"' >"$h/.config/kimun/config.toml"
out="$(uninstall_in "$h" --tui 2>&1)" || fail "terminal uninstall failed: $out"
[ ! -e "$h/bin/kimun" ] || fail "uninstall left the terminal binary"
[ -x "$h/bin/kimun-gui" ] || fail "the terminal uninstall removed the desktop app"
[ -f "$h/.config/kimun/install.toml" ] || fail "the marker went while the desktop app is still installed"
ok "--uninstall --tui: removes the terminal app only; the marker stays for the desktop app"
out="$(uninstall_in "$h" --gui 2>&1)" || fail "desktop uninstall failed: $out"
[ ! -e "$h/bin/kimun-gui" ] || fail "uninstall left the desktop binary"
if [ "$(uname -s)" = Linux ]; then # the launcher files are Linux's
  [ ! -e "$h/.local/share/applications/kimun.desktop" ] || fail "uninstall left the launcher entry"
  [ -z "$(find "$h/.local/share/icons" -name 'kimun.png' 2>/dev/null)" ] || fail "uninstall left icons"
fi
[ ! -e "$h/.config/kimun/install.toml" ] || fail "the marker stayed after both apps were removed"
grep -q 'keep = "me"' "$h/.config/kimun/config.toml" || fail "uninstall touched the user's configuration"
ok "--uninstall --gui: removes binary, launcher entry and icons, then the marker; config.toml is kept"
out="$(uninstall_in "$h" 2>&1)" || fail "a second uninstall failed: $out"
echo "$out" | grep -q "Nothing to remove" || fail "a second uninstall said: $out"
ok "--uninstall with nothing installed: a plain message, exit 0"
# Other launcher icons and entries are left alone; a custom dir is found from the marker.
h="$work/u2"; mkdir -p "$h/.local/share/applications"; echo x >"$h/.local/share/applications/other.desktop"
HOME="$h" KIMUN_INSTALL_DIR="$h/custom" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$rel" sh install.sh --tui >/dev/null
[ -x "$h/custom/kimun" ] || fail "custom-dir install failed"
uninstall_in "$h" >/dev/null || fail "uninstall from a marker's dir failed"
[ ! -e "$h/custom/kimun" ] || fail "uninstall did not find the dir in the marker"
[ -f "$h/.local/share/applications/other.desktop" ] || fail "uninstall removed another app's launcher entry"
ok "--uninstall: finds a custom install dir from the marker and leaves other files alone"
# macOS: the app is removed from ~/Applications; no network needed.
h="$work/u3"; mkdir -p "$h/Applications/Kimun.app"
out="$(PATH="$macbin:$PATH" HOME="$h" sh install.sh --uninstall --gui 2>&1)" || fail "macOS uninstall failed: $out"
[ ! -e "$h/Applications/Kimun.app" ] || fail "macOS uninstall left Kimun.app"
ok "macOS --uninstall --gui: removes Kimun.app"
# Nothing in the default dir: the message points at KIMUN_APP_DIR (no marker remembers it).
h="$work/u3a"; mkdir -p "$h"
out="$(PATH="$macbin:$PATH" HOME="$h" sh install.sh --uninstall --gui 2>&1)" || fail "macOS uninstall with no app failed: $out"
case "$out" in *"Nothing to remove"*"KIMUN_APP_DIR"*) ;; *) fail "the macOS hint about KIMUN_APP_DIR is missing: $out" ;; esac
ok "macOS --uninstall --gui: with no app found, the message names KIMUN_APP_DIR"
# What an interrupted upgrade leaves (the backup and the staging folder) goes too, even with no app.
h="$work/u3b"; mkdir -p "$h/Applications/.Kimun.app.old" "$h/Applications/.Kimun.new/Kimun.app"
out="$(PATH="$macbin:$PATH" HOME="$h" sh install.sh --uninstall --gui 2>&1)" || fail "macOS uninstall of leftovers failed: $out"
[ ! -e "$h/Applications/.Kimun.app.old" ] && [ ! -e "$h/Applications/.Kimun.new" ] || fail "uninstall left the upgrade leftovers"
ok "macOS --uninstall --gui: also removes the leftovers of an interrupted upgrade"
# Launch Services is told to forget the app (best effort), before it is removed.
h="$work/u3c"; mkdir -p "$h/Applications/Kimun.app" "$work/fakels"
printf '#!/bin/sh\necho "$*" >"%s/ls.log"\n' "$work/fakels" >"$work/fakels/lsregister"; chmod +x "$work/fakels/lsregister"
sed "s|^LSREGISTER=.*|LSREGISTER=$work/fakels/lsregister|" install.sh >"$work/install-ls.sh"
PATH="$macbin:$PATH" HOME="$h" sh "$work/install-ls.sh" --uninstall --gui >/dev/null 2>&1 || fail "uninstall with lsregister failed"
grep -q -- "-u $h/Applications/Kimun.app" "$work/fakels/ls.log" || fail "lsregister -u was not called: $(cat "$work/fakels/ls.log" 2>&1)"
ok "macOS --uninstall --gui: unregisters the app from Launch Services"
# The marker belongs to the install it names: uninstalling another dir must not remove it.
h="$work/u5"; mkdir -p "$h"
install_in "$h" "$rel" >/dev/null
HOME="$h" KIMUN_INSTALL_DIR="$h/custom" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$rel" sh install.sh --gui >/dev/null
grep -q "install_dir = \"$h/custom\"" "$h/.config/kimun/install.toml" || fail "setup: the marker does not name the custom dir"
HOME="$h" KIMUN_INSTALL_DIR="$h/bin" sh install.sh --uninstall >/dev/null 2>&1 || fail "uninstall of the other dir failed"
[ ! -e "$h/bin/kimun" ] || fail "uninstall left the terminal binary"
[ -f "$h/.config/kimun/install.toml" ] || fail "uninstalling another dir removed the marker of the desktop app"
[ -x "$h/custom/kimun-gui" ] || fail "uninstalling another dir touched the desktop app"
ok "--uninstall: the marker of an install in another dir stays"
# A marker that cannot be read: said aloud, the default dir is used.
h="$work/u6"; mkdir -p "$h/.config/kimun"; printf "install_dir = 'x'\n" >"$h/.config/kimun/install.toml"
out="$(HOME="$h" sh install.sh --uninstall 2>&1)" || fail "uninstall with an unreadable marker failed: $out"
echo "$out" | grep -q "could not read install_dir" || fail "an unreadable marker was not reported: $out"
ok "--uninstall: an unreadable marker is reported"
# The icon cache is refreshed after the icons go, when one exists (Linux).
if [ "$(uname -s)" = Linux ]; then
  h="$work/u7"; mkdir -p "$h" "$work/gtkbin"
  install_in "$h" "$rel" --gui >/dev/null
  echo cache >"$h/.local/share/icons/hicolor/icon-theme.cache"
  printf '#!/bin/sh\necho "$*" >"%s/gtk.log"\n' "$work/gtkbin" >"$work/gtkbin/gtk-update-icon-cache"; chmod +x "$work/gtkbin/gtk-update-icon-cache"
  PATH="$work/gtkbin:$PATH" HOME="$h" sh install.sh --uninstall --gui >/dev/null 2>&1 || fail "uninstall with an icon cache failed"
  [ -s "$work/gtkbin/gtk.log" ] || fail "the icon cache was not refreshed after the icons were removed"
  ok "--uninstall --gui: refreshes an existing icon cache"
fi
# No network and no curl needed to uninstall.
nt2="$work/nocurl"; mkdir -p "$nt2"
for tool in sh uname rm sed head dirname; do ln -sf "$(command -v $tool)" "$nt2/$tool"; done
h="$work/u4"; mkdir -p "$h"
PATH="$nt2" HOME="$h" sh install.sh --uninstall >/dev/null 2>&1 || fail "uninstall needed a tool the install needs"
ok "--uninstall needs no downloader"

# 8d. The default run installs both apps; the desktop app is best effort, and --tui / --gui select.
h="$work/d1"; mkdir -p "$h"
out="$(install_in "$h" "$rel" 2>&1)" || fail "a default install failed: $out"
[ -x "$h/bin/kimun" ] && [ -x "$h/bin/kimun-gui" ] || fail "a default install did not install both apps"
[ -f "$h/.local/share/applications/kimun.desktop" ] || fail "a default install did not add the launcher entry"
ok "default: both apps installed, one marker, launcher entry"
h="$work/d2"; mkdir -p "$h"
headless_in() { # HOME RELEASE_DIR [args...]: install on a machine with no display
  local home="$1" relv="$2"; shift 2
  env -u DISPLAY -u WAYLAND_DISPLAY HOME="$home" KIMUN_INSTALL_DIR="$home/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$relv" sh install.sh "$@"
}
out="$(headless_in "$h" "$rel" 2>&1)" \
  || fail "a headless default install failed: $out"
[ -x "$h/bin/kimun" ] || fail "headless: the terminal app was not installed"
[ ! -e "$h/bin/kimun-gui" ] || fail "headless: the desktop app was installed"
echo "$out" | grep -q "No display found" || fail "headless: no note: $out"
echo "$out" | grep -qi "^warning" && fail "headless: the skip was a warning: $out"
ok "default on a Linux machine with no display: terminal app only, a plain note"
h="$work/d3"; mkdir -p "$h"
if ! headless_in "$h" "$rel" --gui >/dev/null 2>&1; then
  fail "--gui with no display must still install the desktop app"
fi
[ -x "$h/bin/kimun-gui" ] || fail "--gui with no display: nothing installed"
ok "--gui with no display installs the desktop app anyway"
h="$work/d4"; mkdir -p "$h"
install_in "$h" "$rel" --tui >/dev/null
[ -x "$h/bin/kimun" ] && [ ! -e "$h/bin/kimun-gui" ] || fail "--tui installed the wrong app"
ok "--tui: the terminal app only, with a display"
# Best effort: no desktop build in the release -> the terminal app stays, a warning, exit 0.
h="$work/d5"; mkdir -p "$h"
out="$(install_in "$h" "$ng" 2>&1)" || fail "a default install failed when the desktop app has no build: $out"
[ -x "$h/bin/kimun" ] || fail "best effort: the terminal app was not installed"
[ ! -e "$h/bin/kimun-gui" ] || fail "best effort: a desktop binary appeared"
echo "$out" | grep -q "^warning: the desktop app was not installed" || fail "best effort: no warning: $out"
echo "$out" | grep -q -- "--gui" || fail "best effort: the warning does not say how to retry: $out"
ok "default with no desktop build: terminal app installed, a warning, exit 0"
# Strict: named apps must install.
h="$work/d6"; mkdir -p "$h"
if install_in "$h" "$ng" --tui --gui >/dev/null 2>&1; then fail "--tui --gui succeeded without a desktop build"; fi
install_in "$h" "$rel" --tui --gui >/dev/null || fail "--tui --gui failed on a full release"
[ -x "$h/bin/kimun" ] && [ -x "$h/bin/kimun-gui" ] || fail "--tui --gui did not install both"
ok "--tui --gui: both, and both must succeed"
# A failing terminal install stops the run before the desktop app.
h="$work/d7"; mkdir -p "$h"
bt="$work/bt"; make_release "$bt"; printf 'tampered' >>"$bt/kimun-$v-$platform.tar.gz"
if install_in "$h" "$bt" >/dev/null 2>&1; then fail "a tampered terminal archive installed"; fi
[ ! -e "$h/bin/kimun-gui" ] || fail "the desktop app installed after the terminal app failed"
ok "a failed terminal install stops the run"
# Bare --uninstall removes both.
h="$work/d8"; mkdir -p "$h"
install_in "$h" "$rel" >/dev/null
out="$(uninstall_in "$h" 2>&1)" || fail "a bare uninstall failed: $out"
[ ! -e "$h/bin/kimun" ] && [ ! -e "$h/bin/kimun-gui" ] || fail "a bare uninstall left an app"
[ ! -e "$h/.config/kimun/install.toml" ] || fail "a bare uninstall left the marker"
ok "--uninstall with no option removes both apps"
# A bare --uninstall of an install in a custom dir, found from the marker: both apps go.
h="$work/d8b"; mkdir -p "$h"
HOME="$h" KIMUN_INSTALL_DIR="$h/custom" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$rel" sh install.sh >/dev/null
[ -x "$h/custom/kimun" ] && [ -x "$h/custom/kimun-gui" ] || fail "custom dir: both apps should be installed"
out="$(HOME="$h" sh install.sh --uninstall 2>&1)" || fail "a bare uninstall from the marker failed: $out"
[ ! -e "$h/custom/kimun" ] && [ ! -e "$h/custom/kimun-gui" ] || fail "a bare uninstall left an app in the custom dir"
ok "--uninstall with no option: both apps go from the dir the marker names"
# A default run updates only the app that is installed: the update command adds nothing new.
h="$work/d9"; mkdir -p "$h"
install_in "$h" "$rel" --tui >/dev/null
out="$(install_in "$h" "$rel" 2>&1)" || fail "a default re-run failed: $out"
[ ! -e "$h/bin/kimun-gui" ] || fail "a default re-run added the desktop app to a terminal-only install"
echo "$out" | grep -q -- "--gui" || fail "the default re-run does not say how to add the desktop app: $out"
install_in "$h" "$rel" --gui >/dev/null
out="$(install_in "$h" "$rel" 2>&1)" || fail "a default re-run of both apps failed: $out"
echo "$out" | grep -q "Installed kimun-gui" || fail "a default re-run did not update both installed apps: $out"
ok "default re-run: updates the installed apps, adds no new one"
# The desktop app fails its integrity check in a default run: the error shows, the run says so at the end.
h="$work/d10"; mkdir -p "$h"
bg="$work/bg"; make_release "$bg"; printf 'tampered' >>"$bg/kimun-gui-$v-$platform.tar.gz"
out="$(install_in "$h" "$bg" 2>&1)" || fail "a default install with a tampered desktop archive failed: $out"
echo "$out" | grep -q "^error: checksum mismatch" || fail "the checksum error is not shown: $out"
echo "$out" | grep -q "^warning: the desktop app was not installed" || fail "no warning: $out"
echo "$out" | grep -q "Done, without the desktop app" || fail "the last line hides the failure: $out"
[ -x "$h/bin/kimun" ] && [ ! -e "$h/bin/kimun-gui" ] || fail "a tampered desktop archive installed"
ok "default: a desktop app failure shows its error, a warning and a final note"

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
# shellcheck disable=SC2002  # the pipe is the point: this is the `curl | sh` form
cat install.sh | sh -s -- --help | grep -qi "gui" || fail "--help when piped"
ok "options"

# 11. The release manifest (latest.json, built by release_manifest.py) and the GitHub API fallback.
# No network: the manifest and the fake API are files, and the real generator writes the manifest.
if ! command -v python3 >/dev/null; then
  echo "skipped: the manifest tests need python3"
else
  python3 - "$rel" "$work/releases.json" <<'PY'
import json, os, sys
rel, out = sys.argv[1], sys.argv[2]
assets = [{"name": n, "browser_download_url": f"file://{rel}/{n}"} for n in sorted(os.listdir(rel))]
json.dump([[{"tag_name": "kimun-notes-v9.9.9", "draft": False, "published_at": "2026-01-01T00:00:00Z",
             "html_url": "https://example.test/9.9.9", "assets": assets}]], open(out, "w"))
PY
  python3 -I .github/scripts/release_manifest.py "$work/releases.json" "$work/latest.json" >/dev/null 2>&1
  # Unpinned, the files not redirected: everything must come from the manifest's own urls.
  install_unpinned() { # HOME MANIFEST_URL [args...]  (extra env from the caller)
    local home="$1" manifest="$2"; shift 2
    HOME="$home" KIMUN_INSTALL_DIR="$home/bin" KIMUN_MANIFEST_URL="$manifest" \
      KIMUN_RELEASES_API="file://$work/no-such-api.json" sh install.sh --tui "$@"
  }

  h="$work/m1"; mkdir -p "$h"
  out="$(install_unpinned "$h" "file://$work/latest.json" 2>&1)" || fail "install from the manifest failed: $out"
  [ -x "$h/bin/kimun" ] || fail "manifest install: no binary"
  grep -q "version = \"$v\"" "$h/.config/kimun/install.toml" || fail "manifest install: marker version"
  echo "$out" | grep -qi "warning" && fail "manifest install warned: $out"
  h="$work/m1g"; mkdir -p "$h"
  install_unpinned "$h" "file://$work/latest.json" --gui >/dev/null 2>&1 || fail "desktop install from the manifest failed"
  [ -x "$h/bin/kimun-gui" ] || fail "manifest install: no desktop binary"
  ok "manifest: the version, the urls and the checksum come from it, no API call"

  # The manifest's sha256 is the one checked: change it and the install is refused.
  sed "s/\"sha256\": \"[0-9a-f]*\"/\"sha256\": \"$(printf '0%.0s' $(seq 64))\"/" "$work/latest.json" >"$work/bad.json"
  h="$work/m2"; mkdir -p "$h"
  out="$(install_unpinned "$h" "file://$work/bad.json" 2>&1)" && fail "a wrong manifest checksum was accepted"
  echo "$out" | grep -q "checksum mismatch" || fail "wrong manifest checksum said: $out"
  [ ! -e "$h/bin/kimun" ] || fail "a wrong manifest checksum still installed"
  ok "manifest: its checksum is enforced"

  # Fallback: unreachable, another schema, no app in it. Each one warns and uses the API (a file here).
  fallback() { # NAME MANIFEST_URL WHY-PATTERN
    h="$work/fb-$1"; mkdir -p "$h"
    out="$(HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_MANIFEST_URL="$2" KIMUN_RELEASES_API="file://$work/api.json" \
      KIMUN_DOWNLOAD_BASE="file://$rel" sh install.sh 2>&1)" || fail "fallback ($1) failed: $out"
    echo "$out" | grep -q "warning: .*$3.*using the GitHub API" || fail "fallback ($1) did not say why: $out"
    [ -x "$h/bin/kimun" ] || fail "fallback ($1): no binary"
    grep -q "version = \"$v\"" "$h/.config/kimun/install.toml" || fail "fallback ($1): marker version"
  }
  printf '[{"tag_name":"kimun_core-v9.0.0"},{"tag_name":"kimun-notes-v9.9.9-beta.1"},{"tag_name":"kimun-notes-v%s"}]\n' "$v" >"$work/api.json"
  fallback unreachable "file://$work/no-such-manifest.json" "cannot reach"
  sed 's/"schema": 1/"schema": 2/' "$work/latest.json" >"$work/schema2.json"
  fallback schema "file://$work/schema2.json" "format"
  printf '{\n  "schema": 1\n}\n' >"$work/empty.json"
  fallback empty "file://$work/empty.json" "names no app release"
  ok "manifest unreachable, unknown or empty: a warning, then the GitHub API"

  # With nothing to fall back to either, the failure says so.
  h="$work/m3"; mkdir -p "$h"
  out="$(install_unpinned "$h" "file://$work/no-such-manifest.json" 2>&1)" && fail "installed with no manifest and no API"
  echo "$out" | grep -q "could not reach the GitHub releases API" || fail "no manifest, no API said: $out"
  ok "no manifest and no API: a plain error"
fi

# 12. The release signature (minisign), against a throwaway key written into a copy of the script.
if ! command -v minisign >/dev/null; then
  echo "skipped: the signature tests need minisign"
else
  minisign -G -W -f -p "$work/s.pub" -s "$work/s.key" >/dev/null
  pub="$(sed -n 2p "$work/s.pub")"
  signed_rel() { # DIR: a signed copy of the release
    rm -rf "$1"; cp -r "$rel" "$1"
    minisign -S -s "$work/s.key" -m "$1/checksums-sha256.txt" -x "$1/checksums-sha256.txt.minisig" >/dev/null
  }
  patched() { # NAME: a copy of the script that trusts the throwaway key
    sed -e "s|^SIGNING_PUBKEY=.*|SIGNING_PUBKEY=\"$pub\"|" install.sh >"$work/$1.sh"
  }
  run_signed() { # NAME REL HOME [PATH]: install into a fresh home; prints the output
    mkdir -p "$3"
    PATH="${4:-$PATH}" HOME="$3" KIMUN_INSTALL_DIR="$3/bin" KIMUN_VERSION="$v" KIMUN_DOWNLOAD_BASE="file://$2" \
      "$(command -v sh)" "$work/$1.sh" --tui 2>&1
  }

  # a. Signed, and the key is right: installs; the digest is the signed list's.
  signed_rel "$work/srel"; patched sig-ok
  out="$(run_signed sig-ok "$work/srel" "$work/sg-a")" || fail "a validly signed release was refused: $out"
  [ -x "$work/sg-a/bin/kimun" ] || fail "signed install: no binary"
  echo "$out" | grep -qi "warning" && fail "a validly signed release warned: $out"
  ok "signed release: installs, no warning"

  # b. The list was edited after signing (archive and list changed together): refused, nothing installed.
  signed_rel "$work/trel"
  tmpd="$(mktemp -d)"; printf '#!/bin/sh\necho evil\n' >"$tmpd/kimun"; chmod +x "$tmpd/kimun"
  arch="$work/trel/kimun-$v-$platform.tar.gz"
  tar -czf "$arch" -C "$tmpd" .
  (cd "$work/trel" && grep -v "$(basename "$arch")" checksums-sha256.txt >c.new; sha256sum "$(basename "$arch")" >>c.new; mv c.new checksums-sha256.txt)
  out="$(run_signed sig-ok "$work/trel" "$work/sg-b")" && fail "an edited checksums list was accepted"
  echo "$out" | grep -q "NOT valid" || fail "an edited list said: $out"
  [ ! -e "$work/sg-b/bin/kimun" ] || fail "an edited list still installed"
  ok "a signed list that was changed is refused"

  # c. A pinned version with no signature (it may predate signing): installs, with a warning.
  out="$(run_signed sig-ok "$rel" "$work/sg-c")" || fail "a pinned unsigned release was refused: $out"
  echo "$out" | grep -q "no signature" || fail "a pinned unsigned release did not warn: $out"
  ok "a pinned version without a signature installs on its sha256, with a warning"

  # d. The latest release (not pinned) with no signature: refused, nothing installed.
  h="$work/sg-d"; mkdir -p "$h"
  out="$(HOME="$h" KIMUN_INSTALL_DIR="$h/bin" KIMUN_MANIFEST_URL="file://$work/latest.json" KIMUN_RELEASES_API="file://$work/no-such-api.json" \
    KIMUN_DOWNLOAD_BASE="file://$rel" sh "$work/sig-ok.sh" --tui 2>&1)" && fail "an unsigned latest release was accepted although a key is pinned"
  echo "$out" | grep -q "should be signed" || fail "unsigned latest said: $out"
  [ ! -e "$h/bin/kimun" ] || fail "an unsigned latest release still installed"
  ok "the latest release without a signature is refused"

  # d2. No key pinned (signing not live): nothing is checked, nothing said.
  cp install.sh "$work/sig-off.sh"
  out="$(run_signed sig-off "$rel" "$work/sg-d2")" || fail "signing off: an unsigned release was refused: $out"
  echo "$out" | grep -qi "warning" && fail "signing off: an unsigned release warned: $out"
  ok "with no key pinned, nothing is checked"

  # e. No minisign on the machine: a release installs on its sha256 and says it was not checked.
  nm="$work/nominisign"; mkdir -p "$nm"
  for tool in sh uname curl cat mktemp rm awk sed grep tar gzip sha256sum mkdir mv chmod head dirname tr cut sort id ls cp date ln; do
    command -v "$tool" >/dev/null && ln -sf "$(command -v "$tool")" "$nm/$tool"
  done
  out="$(run_signed sig-ok "$work/srel" "$work/sg-e" "$nm")" || fail "no minisign: install failed: $out"
  # shellcheck disable=SC2015  # the first test is a plain grep: the failure message only runs when a check fails
  echo "$out" | grep -q "minisign is not installed" && echo "$out" | grep -q "not checked" || fail "no minisign: no warning: $out"
  ok "without minisign: installs on the sha256 alone, with a warning"
fi

echo "install.sh behaves"
