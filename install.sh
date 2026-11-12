#!/bin/sh
# Kimün installer — downloads the latest stable release, verifies its checksum,
# installs the binary into a user-writable directory, and records an install
# marker so the app knows it may self-update (the "script" channel).
#
#   curl -fsSL https://kimun.2co.dev/install.sh | sh                 # both apps
#   curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --tui     # the terminal app only
#   curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --gui     # the desktop app only
#
# With no option the script installs both apps, or updates the one that is already there when only
# one is installed. The desktop app is best effort then: when
# it cannot be installed the terminal app stays installed, the script prints a warning and
# exits 0. On a Linux machine with no display (no DISPLAY or WAYLAND_DISPLAY, such as a
# server) the desktop app is skipped, with a note. Name an app with --tui or --gui and
# that app must install, or the script fails. Both apps record their install dir in one
# shared marker.
#
# Options:
#   --tui               install only the terminal app (kimun)
#   --gui               install only the desktop app (kimun-gui). With both --tui and --gui
#                       both apps are installed and both must succeed
#   --uninstall         remove what this script installed: both apps, or only the one that
#                       --tui or --gui names. Your notes and configuration are not touched
#   -h, --help          show this help
#
# Env overrides:
#   KIMUN_INSTALL_DIR   install location (default: $HOME/.local/bin)
#   KIMUN_APP_DIR       where Kimun.app goes on macOS (default: $HOME/Applications)
#   KIMUN_VERSION       install this version instead of the latest stable release
#                       (a pre-release too, e.g. 0.25.0-beta.1)
#   KIMUN_DOWNLOAD_BASE where the release's files are fetched from (default: the
#                       release's GitHub download URL); for testing
#   KIMUN_MANIFEST_URL  the release manifest (default: https://kimun.2co.dev/latest.json);
#                       for testing
#   KIMUN_RELEASES_API  the GitHub releases list used when the manifest cannot be read
#                       (default: the repository's api.github.com URL); for testing
#
# The desktop app on Linux also gets its launcher entry and icons, under
# $XDG_DATA_HOME (default: $HOME/.local/share). On macOS it is the signed Kimun.app,
# taken from the release's kimun-gui-<version>-<platform>.dmg and copied to
# $HOME/Applications (open it from Launchpad or Spotlight). With --gui alone, a release with
# no .dmg gets the raw kimun-gui binary instead, like the terminal app. Without it the app
# is skipped with a warning.
#
# Unix only (Linux, macOS). Windows users: install.ps1, or the release archive.

set -eu

REPO="nico2sh/kimun"
BIN="kimun"
TAG_PREFIX="kimun-notes-v"
INSTALL_DIR="${KIMUN_INSTALL_DIR:-$HOME/.local/bin}"
# Config dir must match the app: $HOME/.config/kimun (no XDG_CONFIG_HOME lookup).
CONFIG_DIR="$HOME/.config/kimun"
MARKER="$CONFIG_DIR/install.toml"
UNINSTALL=0
WANT_TUI=0
WANT_GUI=0
GUI_RETRY="curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --gui"
# Launch Services tool: install registers the app with it, uninstall unregisters it.
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister

err() {
  printf 'error: %s\n' "$1" >&2
  exit 1
}
info() { printf '%s\n' "$1"; }
warn() { printf 'warning: %s\n' "$1" >&2; }

usage() {
  # The header comment, from the usage lines down to the platform note. Piped
  # through `sh -s`, $0 is not the script: say where to read it instead.
  if [ -r "$0" ]; then
    sed -n '/^#   curl/,/^# Unix only/p' "$0" | sed 's/^# \{0,1\}//'
  else
    info "Usage: install.sh [--tui] [--gui] [--uninstall]    (no option: both apps; see the script's header)"
    info "https://github.com/$REPO/blob/main/install.sh"
  fi
}

# --- release signature ----------------------------------------------------
# Releases carry checksums-sha256.txt.minisig, a minisign signature of their checksums file
# (see .github/RELEASING.md, "Signing"). SIGNING_PUBKEY is release-signing.pub, line 2; empty
# means signing is not live yet and nothing is checked. With a key and minisign installed the
# signature is checked and the digest comes from the file it signed. A missing signature is an
# error for the latest release (what the unsigned manifest names) and a warning for a version
# pinned with KIMUN_VERSION, which may predate signing. Without minisign the install goes on
# on the sha256 alone, and says so. install-server.sh carries the same block: keep them identical.
SIGNING_PUBKEY="RWS64g2Q9vGQfwr3tC6fK3UgWdkKkLfz5KLmcTZhewdMmCHHkkfIgNf8"
# verify_signature BASE_URL TMP PINNED: sets SIGNED_LIST to the verified checksums file, or
# "" when nothing was verified. PINNED=1 when the user chose the version.
verify_signature() {
  SIGNED_LIST=""
  [ -n "$SIGNING_PUBKEY" ] || return 0
  if ! have minisign; then
    warn "minisign is not installed: the release signature was not checked (sha256 only)"
    return 0
  fi
  if download "$1/checksums-sha256.txt" "$2/checksums-sha256.txt" 2>/dev/null \
     && download "$1/checksums-sha256.txt.minisig" "$2/checksums-sha256.txt.minisig" 2>/dev/null; then
    minisign -V -q -P "$SIGNING_PUBKEY" -m "$2/checksums-sha256.txt" -x "$2/checksums-sha256.txt.minisig" >/dev/null 2>&1 \
      || err "the signature of checksums-sha256.txt is NOT valid: refusing to install"
    SIGNED_LIST="$2/checksums-sha256.txt"
  elif [ "$3" = 1 ]; then
    warn "this release has no signature (it may predate signing): sha256 only"
  else
    err "the latest release should be signed but its signature could not be fetched: refusing to install"
  fi
}

# --- detect platform -------------------------------------------------------
detect_platform() {
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Linux)
      case "$arch" in
        x86_64 | amd64) echo "linux-x64" ;;
        aarch64 | arm64) echo "linux-arm64" ;;
        *) err "unsupported Linux architecture: $arch" ;;
      esac
      ;;
    Darwin)
      case "$arch" in
        x86_64)
          # Under Rosetta (a terminal or a shell started as Intel) uname reports
          # x86_64 even on Apple Silicon: ask the kernel, and take the native build.
          if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ] \
            || [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
            echo "macos-arm64"
          else
            echo "macos-x64"
          fi
          ;;
        arm64 | aarch64) echo "macos-arm64" ;;
        *) err "unsupported macOS architecture: $arch" ;;
      esac
      ;;
    MINGW* | MSYS* | CYGWIN*)
      err "this installer is for Linux and macOS. On Windows, run: irm https://kimun.2co.dev/install.ps1 | iex   (or download kimun-<version>-windows-x64.zip, or kimun-gui-<version>-windows-x64.zip for the desktop app, from https://github.com/$REPO/releases)"
      ;;
    *)
      err "unsupported OS: $os (this script supports Linux and macOS; Windows users should use the release archive)"
      ;;
  esac
}

# --- download helper (curl or wget) ----------------------------------------
have() { command -v "$1" >/dev/null 2>&1; }

download() { # download URL DEST
  if have curl; then
    curl -fsSL "$1" -o "$2"
  elif have wget; then
    wget -qO "$2" "$1"
  else
    err "need curl or wget to download"
  fi
}

fetch_stdout() { # fetch_stdout URL
  if have curl; then
    curl -fsSL "$1"
  elif have wget; then
    wget -qO- "$1"
  else
    err "need curl or wget to download"
  fi
}

# --- sha256 helper ---------------------------------------------------------
sha256_of() { # sha256_of FILE -> hex digest on stdout
  if have sha256sum; then
    sha256sum "$1" | awk '{print $1}'
  elif have shasum; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    err "need sha256sum or shasum to verify the download"
  fi
}

# Put the desktop app's launcher entry and icons where the desktop looks for
# them. Linux only, and only when the archive carries them (a `share/` tree that
# mirrors an install prefix). The entry's Exec is rewritten to the absolute path
# the binary was installed to, because a launcher does not see the shell's PATH.
install_desktop_files() {
  [ "$(uname -s)" = "Linux" ] || return 0
  [ -f "$tmp/share/applications/kimun.desktop" ] || return 0
  data="${XDG_DATA_HOME:-$HOME/.local/share}"
  mkdir -p "$data/applications" "$data/icons" \
    || { warn "could not create $data (is it writable?); skipping the launcher entry and icons"; return 0; }
  if [ -d "$tmp/share/icons" ]; then
    cp -R "$tmp/share/icons/." "$data/icons/" \
      || warn "could not install the icons to $data/icons"
  fi
  # Escape the characters special in a sed replacement (\ & and the | delimiter).
  exec_path="$(printf '%s' "$INSTALL_DIR/$BIN" | sed 's/[\\&|]/\\&/g')"
  sed "s|^Exec=.*|Exec=\"$exec_path\" %F|" "$tmp/share/applications/kimun.desktop" \
    > "$data/applications/kimun.desktop" \
    || { warn "could not install the launcher entry to $data/applications"; return 0; }
  # Best effort: refresh the desktop's caches when their tools are here.
  if have update-desktop-database; then
    update-desktop-database "$data/applications" >/dev/null 2>&1 || true
  fi
  # Only when a cache already exists: creating one in the user's hicolor tree would
  # be a side effect on every other icon there.
  if have gtk-update-icon-cache && [ -f "$data/icons/hicolor/icon-theme.cache" ]; then
    gtk-update-icon-cache -f -t "$data/icons/hicolor" >/dev/null 2>&1 || true
  fi
  info "Installed the launcher entry and icons under $data"
}

# --- the release manifest ---------------------------------------------------
# https://kimun.2co.dev/latest.json names the newest stable release and each of its
# files with a url and sha256 (see .github/scripts/release_manifest.py). It is
# pretty-printed, one key per line, and read here with awk: no jq needed. Anything
# unexpected about it is not an error: the installer falls back to the GitHub API.
manifest_version() { # manifest_version SECTION -> its version, or nothing
  printf '%s\n' "$MANIFEST" | awk -v s="$1" '
    $0 == "  \"" s "\": {" { on = 1; next }
    on && /^  }/ { exit }
    on && /^    "version": "/ { v = $0; sub(/^    "version": "/, "", v); sub(/".*/, "", v); print v; exit }'
}
manifest_asset() { # manifest_asset SECTION NAME FIELD -> url or sha256 of that file, or nothing
  printf '%s\n' "$MANIFEST" | awk -v s="$1" -v n="$2" -v f="$3" '
    $0 == "  \"" s "\": {" { on = 1; next }
    on && /^  }/ { exit }
    on && $0 == "      \"" n "\": {" { hit = 1; next }
    hit && /^      }/ { exit }
    hit { p = "        \"" f "\": \""; if (index($0, p) == 1) { v = substr($0, length(p) + 1); sub(/".*/, "", v); print v; exit } }'
}
# Read the manifest and take the app's version from it. On failure sets MANIFEST_WHY.
resolve_from_manifest() {
  MANIFEST="$(fetch_stdout "$MANIFEST_URL" 2>/dev/null)" || { MANIFEST_WHY="cannot reach $MANIFEST_URL"; return 1; }
  printf '%s\n' "$MANIFEST" | grep -q '^  "schema": 1,\{0,1\}$' \
    || { MANIFEST_WHY="$MANIFEST_URL is in a format this installer does not know"; return 1; }
  VERSION="$(manifest_version app)"
  case "$VERSION" in
    "" | *[!0-9A-Za-z.+-]*) VERSION=""; MANIFEST_WHY="$MANIFEST_URL names no app release"; return 1 ;;
  esac
}

# The dir the install marker records ("" when there is no marker or it names none we can read).
marker_dir() {
  if [ -f "$MARKER" ]; then
    sed -n 's/^install_dir = "\(.*\)"$/\1/p' "$MARKER" | head -n1
  fi
}

# resolve_uninstall_dir: sets UDIR (the dir to remove from) and UMARKER_DIR (the dir the marker
# records, if any), once, before the first app goes: removing the terminal app can remove the
# marker, and the desktop app's pass must still see the same dir.
resolve_uninstall_dir() {
  # The dir the marker records, if any. The marker only goes when it belongs to this dir.
  UMARKER_DIR="$(marker_dir)"
  UMARKER_DIR="${UMARKER_DIR%/}"
  UDIR="${KIMUN_INSTALL_DIR:-}"
  if [ -z "$UDIR" ]; then
    UDIR="$UMARKER_DIR"
    if [ -z "$UDIR" ] && [ -f "$MARKER" ]; then
      warn "could not read install_dir from $MARKER: using $HOME/.local/bin"
    fi
  fi
  [ -n "$UDIR" ] || UDIR="$HOME/.local/bin"
}

# Remove what this script installed for the app named in BIN (main calls it for each app).
# It only removes files the installer puts in place, never your notes or your
# configuration (config.toml and the rest of ~/.config/kimun stay). The install
# marker goes when neither app is left in its install dir. The dir is the one the
# marker records, unless KIMUN_INSTALL_DIR says otherwise.
uninstall() {
  dir="$UDIR"
  mdir="$UMARKER_DIR"
  remove() { # remove PATH...: a file or a directory this script created
    for target in "$@"; do
      if [ -e "$target" ] || [ -L "$target" ]; then
        rm -rf "$target" || err "could not remove $target"
        info "Removed $target"
        ANY_REMOVED=1
      fi
    done
  }
  remove "$dir/$BIN"
  if [ "$BIN" = "kimun-gui" ]; then
    case "$(uname -s)" in
      Darwin)
        apps="${KIMUN_APP_DIR:-$HOME/Applications}"
        # Best effort: drop the Launch Services entry before the app goes. The two hidden
        # names are what an interrupted install leaves behind.
        if [ -d "$apps/Kimun.app" ] && [ -x "$LSREGISTER" ]; then
          "$LSREGISTER" -u "$apps/Kimun.app" >/dev/null 2>&1 || true
        fi
        remove "$apps/Kimun.app" "$apps/.Kimun.app.old" "$apps/.Kimun.new"
        ;;
      Linux)
        data="${XDG_DATA_HOME:-$HOME/.local/share}"
        remove "$data/applications/kimun.desktop"
        for icon in "$data"/icons/hicolor/*/apps/kimun.png; do remove "$icon"; done
        if have update-desktop-database; then
          update-desktop-database "$data/applications" >/dev/null 2>&1 || true
        fi
        # As the install does: only when an icon cache already exists.
        if have gtk-update-icon-cache && [ -f "$data/icons/hicolor/icon-theme.cache" ]; then
          gtk-update-icon-cache -f -t "$data/icons/hicolor" >/dev/null 2>&1 || true
        fi
        ;;
    esac
  fi
  if [ -f "$MARKER" ] && [ ! -e "$dir/kimun" ] && [ ! -e "$dir/kimun-gui" ] \
    && { [ -z "$mdir" ] || [ "$mdir" = "${dir%/}" ]; }; then
    remove "$MARKER"
  fi
}

# Remove the temp dir. A mounted .dmg lives under it and must be detached first.
cleanup() {
  if [ -n "${MOUNT:-}" ]; then hdiutil detach -quiet "$MOUNT" >/dev/null 2>&1 || hdiutil detach -force -quiet "$MOUNT" >/dev/null 2>&1 || true; fi
  rm -rf "$tmp"
}

# macOS desktop app: mount the .dmg, copy Kimun.app next to the user's other apps, and
# check that its signature holds. It is copied to a hidden folder first, so a failed copy
# or a bad signature never replaces a working install. A download carries no quarantine
# flag, so Gatekeeper does not ask; the signature check is ours.
install_macos_app() {
  apps="${KIMUN_APP_DIR:-$HOME/Applications}"
  mnt="$tmp/mnt"
  mkdir -p "$mnt" || err "could not create the mount point $mnt"
  mkdir -p "$apps" || err "could not create $apps"
  # MOUNT is set only once the attach worked, so cleanup detaches only a real mount.
  hdiutil attach -nobrowse -readonly -noverify -mountpoint "$mnt" "$tmp/$ARCHIVE" >/dev/null \
    || err "could not mount $ARCHIVE"
  MOUNT="$mnt"
  [ -d "$MOUNT/Kimun.app" ] || err "Kimun.app not found in $ARCHIVE"
  # The staging folder keeps the name Kimun.app inside it, so codesign sees a real bundle name.
  stagedir="$apps/.Kimun.new"
  staged="$stagedir/Kimun.app"
  rm -rf "$stagedir"
  mkdir -p "$stagedir" || err "could not create $stagedir"
  ditto "$MOUNT/Kimun.app" "$staged" || { rm -rf "$stagedir"; err "could not copy the app to $apps (is it writable?)"; }
  # The seal must be intact AND the signature must chain to Apple as a Developer ID
  # Application certificate (not ad-hoc, not self-signed). Notarization is not checked.
  devid="anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
  if ! codesign --verify --deep --strict -R "=$devid" "$staged" >/dev/null 2>&1; then
    rm -rf "$stagedir"
    err "Kimun.app is not validly signed with an Apple Developer ID certificate: refusing to install"
  fi
  # Swap with a backup, so a failed move leaves the previous app in place.
  backup=""
  if [ -e "$apps/Kimun.app" ]; then
    backup="$apps/.Kimun.app.old"
    rm -rf "$backup"
    mv "$apps/Kimun.app" "$backup" || { rm -rf "$stagedir"; err "could not replace the existing Kimun.app in $apps (is it running or read-only?)"; }
  fi
  if ! mv "$staged" "$apps/Kimun.app"; then
    rm -rf "$stagedir"
    [ -z "$backup" ] || mv "$backup" "$apps/Kimun.app" || true
    err "could not install to $apps"
  fi
  rm -rf "$stagedir"
  [ -z "$backup" ] || rm -rf "$backup"
  # Best effort: tell Launch Services about the app so Spotlight and Launchpad find it now.
  if [ -x "$LSREGISTER" ]; then
    "$LSREGISTER" -f "$apps/Kimun.app" >/dev/null 2>&1 || true
  fi
  info "Installed Kimun.app to $apps/Kimun.app"
}

# Fail early and plainly when a tool the install needs is missing.
check_tools() {
  have curl || have wget || err "needs curl or wget to download"
  have tar || err "needs tar to unpack the download"
}

# fetch_archive NAME [quiet]: download release file NAME into $tmp. Sets ARCHIVE and EXPECTED
# (the sha256 the manifest gives, or empty). The manifest names a file's url and sha256: they
# are used unless the files come from somewhere else (KIMUN_DOWNLOAD_BASE). A file it does not
# list is looked for the usual way, so the errors in main stay the same. Needs BASE_URL, FROM_MANIFEST and tmp.
fetch_archive() {
  ARCHIVE="$1"
  EXPECTED=""
  _url="$BASE_URL/$ARCHIVE"
  if [ "$FROM_MANIFEST" = 1 ] && [ -z "${KIMUN_DOWNLOAD_BASE:-}" ]; then
    _m_url="$(manifest_asset app "$ARCHIVE" url)"
    _m_sha="$(manifest_asset app "$ARCHIVE" sha256)"
    if [ -n "$_m_url" ] && [ -n "$_m_sha" ]; then
      _url="$_m_url"
      EXPECTED="$_m_sha"
    fi
  fi
  info "Downloading $ARCHIVE..."
  if [ "${2:-}" = quiet ]; then
    download "$_url" "$tmp/$ARCHIVE" 2>/dev/null
  else
    download "$_url" "$tmp/$ARCHIVE"
  fi
}

# release_lists NAME: does the release's checksums file list NAME? Returns 0 when it does,
# 1 when the file is reachable and does not, 2 when it cannot be fetched.
release_lists() {
  # A separate file: the one in RELTMP may be the signed list the next app is checked against.
  download "$BASE_URL/checksums-sha256.txt" "$tmp/listing.txt" 2>/dev/null || return 2
  awk -v f="$1" '$2 == f { found = 1 } END { exit !found }' "$tmp/listing.txt"
}

# Install one app (BIN is kimun or kimun-gui) from the release main resolved. main runs it
# in a subshell, so an err exits only this install and the temp dir and mount are cleaned
# up here. install_one NAME [soft]: with soft, the release having no macOS .dmg is an error
# (no raw-binary fallback), and main reports a failed install as a warning.
install_one() {
  set -e
  BIN="$1"
  SOFT_APP="${2:-}"
  tmp="$(mktemp -d)"
  # cleanup runs on every exit. INT and TERM must exit, or the script would keep running
  # after cleanup removed the temp dir and detached the .dmg.
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM

  MAC_APP=0
  if [ "$BIN" = "kimun-gui" ] && [ "$(uname -s)" = "Darwin" ]; then
    if fetch_archive "${BIN}-${VERSION}-${PLATFORM}.dmg" quiet; then
      MAC_APP=1
    else
      # Fall back to the raw binary only when the release is confirmed to have no .dmg.
      # Any other failure (network, a 5xx, a rate limit) must not change the install type.
      rc=0; release_lists "$ARCHIVE" || rc=$?
      case "$rc" in
        1)
          # Only a named --gui falls back to the raw binary: in a default run it would be a
          # half-installed app, with no icon and no bundle.
          [ "$SOFT_APP" != soft ] || err "release $VERSION has no macOS app (.dmg) for $PLATFORM"
          warn "release $VERSION has no macOS app (.dmg): installing the raw binary instead" ;;
        0) err "failed to download $ARCHIVE (it is listed in release $VERSION). Try again" ;;
        *) err "failed to download $ARCHIVE, and could not check whether release $VERSION has one. Try again" ;;
      esac
    fi
  fi

  if [ "$MAC_APP" = 0 ] && ! fetch_archive "${BIN}-${VERSION}-${PLATFORM}.tar.gz"; then
    # When the release is there (its checksums are) but this app's build for this system
    # is not, say so, rather than a bare failed download.
    rc=0; release_lists "$ARCHIVE" || rc=$?
    if [ "$rc" = 1 ]; then
      if [ "$BIN" = "kimun-gui" ]; then
        err "release $VERSION has no desktop app build for $PLATFORM. Not every release has one yet: see https://github.com/$REPO/releases, or pin a release that does with KIMUN_VERSION"
      fi
      err "release $VERSION has no $BIN build for $PLATFORM (see https://github.com/$REPO/releases)"
    fi
    err "failed to download $ARCHIVE (is $VERSION a release?)"
  fi

  # Verify the checksum: the signed list when minisign can check it, else the manifest's,
  # else the release's published checksums file. main fetched and checked that file once.
  info "Verifying checksum..."
  expected="$EXPECTED"
  if [ -n "$SIGNED_LIST" ]; then
    expected=""
  fi
  SUMS="$RELTMP/checksums-sha256.txt"
  if [ -z "$expected" ] && [ ! -s "$SUMS" ]; then
    download "$BASE_URL/checksums-sha256.txt" "$SUMS" \
      || err "failed to download checksums-sha256.txt"
  fi
  if [ -z "$expected" ]; then
    # Exact filename match (field 2), not a regex — the archive name contains '.'
    # which would otherwise act as a wildcard.
    expected="$(awk -v f="$ARCHIVE" '$2 == f { print $1 }' "$SUMS")"
  fi
  [ -n "$expected" ] || err "no checksum found for $ARCHIVE"
  actual="$(sha256_of "$tmp/$ARCHIVE")"
  [ "$expected" = "$actual" ] \
    || err "checksum mismatch for $ARCHIVE (expected $expected, got $actual)"

  if [ "$MAC_APP" = 1 ]; then
    install_macos_app
    # No install marker: a binary inside Kimun.app is the Bundle channel, found from its
    # path. A marker would also overwrite the terminal app's.
    # An older install put a raw kimun-gui in $INSTALL_DIR (and wrote the marker). Left in
    # place it would be a second, diverging copy that still updates itself.
    if [ -e "$INSTALL_DIR/kimun-gui" ] || [ -L "$INSTALL_DIR/kimun-gui" ]; then
      rm -f "$INSTALL_DIR/kimun-gui" && info "Removed the old binary $INSTALL_DIR/kimun-gui: Kimun.app replaces it"
      # The marker goes with it unless the terminal app is still there or it names another dir.
      old_mdir="$(marker_dir)"; old_mdir="${old_mdir%/}"
      if [ -f "$MARKER" ] && [ ! -e "$INSTALL_DIR/kimun" ] \
        && { [ -z "$old_mdir" ] || [ "$old_mdir" = "${INSTALL_DIR%/}" ]; }; then
        rm -f "$MARKER" && info "Removed the old install marker $MARKER"
      fi
    fi
    info "Open Kimun from Launchpad or Spotlight, or run: open -a Kimun"
    return 0
  fi

  # Extract the binary.
  info "Extracting..."
  tar -xzf "$tmp/$ARCHIVE" -C "$tmp" || err "failed to extract $ARCHIVE"
  [ -f "$tmp/$BIN" ] || err "binary '$BIN' not found in archive"
  chmod +x "$tmp/$BIN"

  # Install.
  mkdir -p "$INSTALL_DIR" || err "could not create install dir: $INSTALL_DIR"
  mv -f "$tmp/$BIN" "$INSTALL_DIR/$BIN" \
    || err "could not install to $INSTALL_DIR (is it writable?)"
  info "Installed $BIN to $INSTALL_DIR/$BIN"
  install_desktop_files

  # Write the install marker (the "script" channel contract read by the app).
  mkdir -p "$CONFIG_DIR" || err "could not create config dir: $CONFIG_DIR"
  cat > "$MARKER" <<EOF
# Kimün install marker — written by install.sh. Safe to delete; the app will
# then fall back to detecting the install channel from the binary path.
channel = "script"
install_dir = "$INSTALL_DIR"
version = "$VERSION"
EOF
  info "Wrote install marker: $MARKER"

  if [ "$BIN" = "kimun-gui" ]; then
    if [ "$(uname -s)" = "Darwin" ]; then
      info "This release has no macOS app, so this is the raw binary: run it from a terminal."
    else
      info "Open Kimün from your application menu, or run: $BIN"
    fi
  else
    info "Run '$BIN' to start."
  fi
}

main() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --tui) WANT_TUI=1 ;;
      --gui) WANT_GUI=1 ;;
      --uninstall) UNINSTALL=1 ;;
      -h | --help) usage; exit 0 ;;
      *) err "unknown option: $1 (try --help)" ;;
    esac
    shift
  done

  # No option: both apps on a new install. When only one app is already installed, only that
  # one is updated: a re-run of the update command must not add an app nobody asked for.
  # Naming an app makes it required.
  GUI_NAMED=$WANT_GUI
  if [ "$WANT_TUI" = 0 ] && [ "$WANT_GUI" = 0 ] && [ "$UNINSTALL" = 0 ]; then
    have_tui=0; have_gui=0
    [ -e "$INSTALL_DIR/kimun" ] && have_tui=1
    if [ -e "$INSTALL_DIR/kimun-gui" ] || [ -d "${KIMUN_APP_DIR:-$HOME/Applications}/Kimun.app" ]; then have_gui=1; fi
    if [ "$have_tui" = 1 ] && [ "$have_gui" = 0 ]; then
      WANT_TUI=1
      info "Updating the terminal app only: the desktop app is not installed."
      ADD_HINT="==> The desktop app (kimun-gui) is NOT installed. To add it, run:
    $GUI_RETRY"
      info "$ADD_HINT"
    elif [ "$have_gui" = 1 ] && [ "$have_tui" = 0 ]; then
      WANT_GUI=1
      GUI_NAMED=1
      info "Updating the desktop app only: the terminal app is not installed."
      ADD_HINT="==> The terminal app (kimun) is NOT installed. To add it, run:
    ${GUI_RETRY%--gui}--tui"
      info "$ADD_HINT"
    fi
  fi
  if [ "$WANT_TUI" = 0 ] && [ "$WANT_GUI" = 0 ]; then
    WANT_TUI=1
    WANT_GUI=1
  fi

  if [ "$UNINSTALL" = 1 ]; then
    ANY_REMOVED=0
    resolve_uninstall_dir
    if [ "$WANT_TUI" = 1 ]; then BIN=kimun; uninstall; fi
    if [ "$WANT_GUI" = 1 ]; then BIN=kimun-gui; uninstall; fi
    if [ "$ANY_REMOVED" = 0 ]; then
      info "Nothing to remove: no app found in $UDIR."
      # The app install leaves no marker, so a custom KIMUN_APP_DIR is not remembered.
      if [ "$WANT_GUI" = 1 ] && [ "$(uname -s)" = Darwin ]; then
        info "Kimun.app is not in ${KIMUN_APP_DIR:-$HOME/Applications}. If you installed it with KIMUN_APP_DIR, run the uninstall with the same KIMUN_APP_DIR."
      fi
    else
      info "Done. Your notes and configuration in $CONFIG_DIR were not touched."
    fi
    exit 0
  fi

  # A default run skips the desktop app where there is no desktop to open it: a Linux
  # machine with neither an X nor a Wayland display (a server, an SSH session).
  if [ "$GUI_NAMED" = 0 ] && [ "$WANT_GUI" = 1 ] && [ "$(uname -s)" = Linux ] \
    && [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
    WANT_GUI=0
    info "No display found (DISPLAY and WAYLAND_DISPLAY are empty): skipping the desktop app."
    info "To install it anyway: $GUI_RETRY"
  fi

  check_tools
  PLATFORM="$(detect_platform)"
  info "Detected platform: $PLATFORM"

  # Resolve the latest *stable* app release: from the manifest, else from the GitHub
  # API. The API's /releases/latest cannot be used: this repo publishes several tag
  # series (kimun-notes-v* for the app, kimun_core-v*, kimun_server-v*), and "latest"
  # is the newest of any, often a core release with no binaries. So that list
  # (newest-first, and only the newest 100 across all series) is scanned for the first
  # kimun-notes-v* tag whose version has no pre-release suffix (a hyphen), matching the
  # app's stable-only update policy.
  MANIFEST_URL="${KIMUN_MANIFEST_URL:-https://kimun.2co.dev/latest.json}"
  RELEASES_API="${KIMUN_RELEASES_API:-https://api.github.com/repos/$REPO/releases?per_page=100}"
  FROM_MANIFEST=0
  VERSION="${KIMUN_VERSION:-}"
  if [ -n "$VERSION" ]; then
    info "Version (pinned): $VERSION"
  else
    info "Resolving latest release..."
    if resolve_from_manifest; then
      FROM_MANIFEST=1
    else
      warn "$MANIFEST_WHY; using the GitHub API"
      api_json="$(fetch_stdout "$RELEASES_API")" \
        || err "could not reach the GitHub releases API"
      # grep -o extracts each "tag_name":"..." token individually, so this works
      # even if the API returns the JSON minified onto a single line (a plain
      # line-based grep+greedy-sed would then capture the wrong tag).
      VERSION="$(printf '%s' "$api_json" \
        | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' \
        | sed -E 's/.*"([^"]*)"[[:space:]]*$/\1/' \
        | sed -n "s/^${TAG_PREFIX}//p" \
        | grep -v '[-]' \
        | head -n1)"
      [ -n "$VERSION" ] || err "could not find a stable ${TAG_PREFIX}* release"
    fi
    info "Latest version: $VERSION"
  fi
  TAG="${TAG_PREFIX}${VERSION}"

  # Each app has its own archive: kimun-<v>-<platform> and kimun-gui-<v>-<platform>.
  # On macOS the desktop app is the .dmg when the release has one.
  BASE_URL="${KIMUN_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download/$TAG}"


  # The release's checksums file, fetched and checked once for both apps. RELTMP holds it.
  RELTMP="$(mktemp -d)"
  trap 'rm -rf "$RELTMP"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  verify_signature "$BASE_URL" "$RELTMP" "$([ -n "${KIMUN_VERSION:-}" ] && echo 1 || echo 0)"

  # The terminal app first, and it must install. The desktop app is best effort in a
  # default run: a failure is a warning and the terminal app stays. --gui names it: then
  # it must install.
  GUI_FAILED=0
  if [ "$WANT_TUI" = 1 ]; then
    ( install_one kimun )
  fi
  if [ "$WANT_GUI" = 1 ]; then
    if [ "$GUI_NAMED" = 1 ]; then
      ( install_one kimun-gui )
    else
      # No -e around it: a failed subshell must not stop the script. Any failure counts,
      # also one that printed nothing (a signal, a missing tool).
      rc=0
      ( install_one kimun-gui soft ) || rc=$?
      if [ "$rc" != 0 ]; then
        GUI_FAILED=1
        warn "the desktop app was not installed (see the message above)."
        info "To try it again: $GUI_RETRY"
      fi
    fi
  fi

  # PATH check, once, when a binary went into the install dir.
  if [ -e "$INSTALL_DIR/kimun" ] || [ -e "$INSTALL_DIR/kimun-gui" ]; then
    case ":$PATH:" in
      *":$INSTALL_DIR:"*) : ;;
      *)
        info ""
        info "NOTE: $INSTALL_DIR is not on your PATH. Add it, e.g.:"
        info "  export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
    esac
  fi

  # Said again at the end, where the eye lands, so a re-run that added nothing is not missed.
  if [ -n "${ADD_HINT:-}" ]; then
    info ""
    info "$ADD_HINT"
  fi

  info ""
  if [ "$GUI_FAILED" = 1 ]; then
    info "Done, without the desktop app."
  else
    info "Done."
  fi
}

main "$@"
