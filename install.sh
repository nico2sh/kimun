#!/bin/sh
# Kimün installer — downloads the latest stable release, verifies its checksum,
# installs the binary into a user-writable directory, and records an install
# marker so the app knows it may self-update (the "script" channel).
#
#   curl -fsSL https://kimun.2co.dev/install.sh | sh                 # the terminal app
#   curl -fsSL https://kimun.2co.dev/install.sh | sh -s -- --gui     # the desktop app
#
# Run it once for each app you want; both record their install dir in one shared marker.
#
# Options:
#   --gui               install the desktop app (kimun-gui) instead of the terminal app
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
# $HOME/Applications (open it from Launchpad or Spotlight). A release with no .dmg
# gets the raw kimun-gui binary instead, like the terminal app.
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

err() { printf 'error: %s\n' "$1" >&2; exit 1; }
info() { printf '%s\n' "$1"; }
warn() { printf 'warning: %s\n' "$1" >&2; }

usage() {
  # The header comment, from the usage lines down to the platform note. Piped
  # through `sh -s`, $0 is not the script: say where to read it instead.
  if [ -r "$0" ]; then
    sed -n '/^#   curl/,/^# Unix only/p' "$0" | sed 's/^# \{0,1\}//'
  else
    info "Usage: install.sh [--gui]    (--gui installs the desktop app; see the script's header)"
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
SIGNING_PUBKEY=""
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
  lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
  [ -x "$lsregister" ] && "$lsregister" -f "$apps/Kimun.app" >/dev/null 2>&1 || true
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
  download "$BASE_URL/checksums-sha256.txt" "$tmp/checksums-sha256.txt" 2>/dev/null || return 2
  awk -v f="$1" '$2 == f { found = 1 } END { exit !found }' "$tmp/checksums-sha256.txt"
}

main() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --gui) BIN="kimun-gui" ;;
      -h | --help) usage; exit 0 ;;
      *) err "unknown option: $1 (try --help)" ;;
    esac
    shift
  done

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
        1) warn "release $VERSION has no macOS app (.dmg): installing the raw binary instead" ;;
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
  # else the release's published checksums file.
  info "Verifying checksum..."
  expected="$EXPECTED"
  verify_signature "$BASE_URL" "$tmp" "$([ -n "${KIMUN_VERSION:-}" ] && echo 1 || echo 0)"
  if [ -n "$SIGNED_LIST" ]; then
    expected=""
    SUMS="$SIGNED_LIST"
  else
    SUMS="$tmp/checksums-sha256.txt"
    if [ -z "$expected" ] && [ ! -s "$SUMS" ]; then
      download "$BASE_URL/checksums-sha256.txt" "$SUMS" \
        || err "failed to download checksums-sha256.txt"
    fi
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
    info ""
    info "Done. Open Kimun from Launchpad or Spotlight, or run: open -a Kimun"
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

  # PATH check.
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) : ;;
    *)
      info ""
      info "NOTE: $INSTALL_DIR is not on your PATH. Add it, e.g.:"
      info "  export PATH=\"$INSTALL_DIR:\$PATH\""
      ;;
  esac

  info ""
  info "Done. Run '$BIN' to start."
  if [ "$BIN" = "kimun-gui" ] && [ "$(uname -s)" = "Darwin" ]; then
    info "(This release has no macOS app, so this is the raw binary: run it from a terminal.)"
  fi
}

main "$@"
