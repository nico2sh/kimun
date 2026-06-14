#!/bin/sh
# Kimün installer — downloads the latest stable release, verifies its checksum,
# installs the binary into a user-writable directory, and records an install
# marker so the app knows it may self-update (the "script" channel).
#
#   curl -fsSL https://kimun.2co.dev/install.sh | sh
#
# Env overrides:
#   KIMUN_INSTALL_DIR   install location (default: $HOME/.local/bin)
#
# Unix only (Linux, macOS). Windows users: use the release archive directly.

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

# --- detect platform -------------------------------------------------------
detect_platform() {
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Linux)
      case "$arch" in
        x86_64 | amd64) echo "linux-x64" ;;
        *) err "unsupported Linux architecture: $arch (only x86_64 is published)" ;;
      esac
      ;;
    Darwin)
      case "$arch" in
        x86_64) echo "macos-x64" ;;
        arm64 | aarch64) echo "macos-arm64" ;;
        *) err "unsupported macOS architecture: $arch" ;;
      esac
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

main() {
  PLATFORM="$(detect_platform)"
  info "Detected platform: $PLATFORM"

  # Resolve the latest *stable* app release. NOTE: /releases/latest cannot be
  # used — this repo publishes two tag series (kimun-notes-v* for the app and
  # kimun_core-v* for the core library), and "latest" is the newest of *either*,
  # often a core release with no binaries. Instead list releases (newest-first)
  # and take the first kimun-notes-v* tag whose version has no pre-release
  # suffix (a hyphen), matching the app's stable-only update policy.
  info "Resolving latest release..."
  api_json="$(fetch_stdout "https://api.github.com/repos/$REPO/releases?per_page=100")" \
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
  TAG="${TAG_PREFIX}${VERSION}"
  info "Latest version: $VERSION"

  ARCHIVE="kimun-${VERSION}-${PLATFORM}.tar.gz"
  BASE_URL="https://github.com/$REPO/releases/download/$TAG"

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT INT TERM

  info "Downloading $ARCHIVE..."
  download "$BASE_URL/$ARCHIVE" "$tmp/$ARCHIVE" \
    || err "failed to download $ARCHIVE"

  # Verify checksum against the published checksums file.
  info "Verifying checksum..."
  download "$BASE_URL/checksums-sha256.txt" "$tmp/checksums-sha256.txt" \
    || err "failed to download checksums-sha256.txt"
  # Exact filename match (field 2), not a regex — the archive name contains '.'
  # which would otherwise act as a wildcard.
  expected="$(awk -v f="$ARCHIVE" '$2 == f { print $1 }' "$tmp/checksums-sha256.txt")"
  [ -n "$expected" ] || err "no checksum found for $ARCHIVE"
  actual="$(sha256_of "$tmp/$ARCHIVE")"
  [ "$expected" = "$actual" ] \
    || err "checksum mismatch for $ARCHIVE (expected $expected, got $actual)"

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
}

main "$@"
