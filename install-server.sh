#!/bin/sh
# Kimün server installer — downloads the latest stable kimun-server release,
# verifies its checksum, and installs the binary into a user-writable directory.
# Re-running the script updates an existing install (and restarts the service,
# if one was set up).
#
#   curl -fsSL https://kimun.2co.dev/install-server.sh | sh
#   curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service
#
# Options:
#   --service           also set up a user service (systemd on Linux, launchd
#                       on macOS) so the server starts on login and restarts
#                       on failure
#
# Env overrides:
#   KIMUN_SERVER_INSTALL_DIR   install location (default: $HOME/.local/bin)
#   KIMUN_DOWNLOAD_BASE        where the release's files are fetched from, when the
#                              manifest is not used (default: the release's GitHub
#                              download URL); for testing
#   KIMUN_MANIFEST_URL         the release manifest (default:
#                              https://kimun.2co.dev/latest.json); for testing
#   KIMUN_RELEASES_API         the GitHub releases list used when the manifest cannot
#                              be read (default: the repository's api.github.com URL);
#                              for testing
#
# Unix only (Linux, macOS Apple Silicon). Windows users: use the release
# archive directly. Intel Macs: no prebuilt binary (the ONNX runtime dropped
# x86_64 macOS) — use Docker or `cargo install`.

set -eu

REPO="nico2sh/kimun"
BIN="kimun-server"
TAG_PREFIX="kimun_server-v"
INSTALL_DIR="${KIMUN_SERVER_INSTALL_DIR:-$HOME/.local/bin}"
# Config location must match the server: ~/.config/kimun/server.toml
# (no XDG_CONFIG_HOME lookup).
CONFIG_DIR="$HOME/.config/kimun"
CONFIG_FILE="$CONFIG_DIR/server.toml"
# Service working directory: the fastembed model cache is created relative to
# the process cwd, and the generated SQLite store also lives here.
STATE_DIR="$HOME/.local/share/kimun"
LAUNCHD_LABEL="dev.2co.kimun-server"
LAUNCHD_PLIST="$HOME/Library/LaunchAgents/$LAUNCHD_LABEL.plist"
SYSTEMD_UNIT="$HOME/.config/systemd/user/kimun-server.service"

err() { printf 'error: %s\n' "$1" >&2; exit 1; }
info() { printf '%s\n' "$1"; }
warn() { printf 'warning: %s\n' "$1" >&2; }

SETUP_SERVICE=0
for arg in "$@"; do
  case "$arg" in
    --service) SETUP_SERVICE=1 ;;
    *) err "unknown option: $arg (supported: --service)" ;;
  esac
done

# --- release signature ----------------------------------------------------
# Every release from FIRST_SIGNED on carries checksums-sha256.txt.minisig, a minisign
# signature of its checksums file (see .github/RELEASING.md, "Signing"). When minisign is
# installed it is checked against SIGNING_PUBKEY (release-signing.pub, line 2) and the
# digest comes from the file it signed. Without minisign the install goes on on the sha256
# alone, and says so. install-server.sh carries the same block: keep them identical.
SIGNING_PUBKEY=""
FIRST_SIGNED="9999.0.0"
# version_ge A B: is plain version A at or above B? (a pre-release suffix is ignored)
version_ge() {
  awk -v a="${1%%-*}" -v b="${2%%-*}" 'BEGIN {
    na = split(a, x, "."); nb = split(b, y, ".")
    for (i = 1; i <= 3; i++) { if (x[i] + 0 != y[i] + 0) exit !(x[i] + 0 > y[i] + 0) }
    exit 0 }'
}
# verify_signature VERSION FIRST_SIGNED BASE_URL TMP: sets SIGNED_LIST to the verified
# checksums file, or "" when nothing was verified. Errors on a bad or required-but-missing one.
verify_signature() {
  SIGNED_LIST=""
  _signed_release=0
  version_ge "$1" "$2" && _signed_release=1
  if ! have minisign || [ -z "$SIGNING_PUBKEY" ]; then
    [ "$_signed_release" = 0 ] || warn "minisign is not installed: release $1 is signed, but its signature was not checked (sha256 only)"
    return 0
  fi
  if download "$3/checksums-sha256.txt" "$4/checksums-sha256.txt" 2>/dev/null \
     && download "$3/checksums-sha256.txt.minisig" "$4/checksums-sha256.txt.minisig" 2>/dev/null; then
    minisign -V -q -P "$SIGNING_PUBKEY" -m "$4/checksums-sha256.txt" -x "$4/checksums-sha256.txt.minisig" >/dev/null 2>&1 \
      || err "the signature of checksums-sha256.txt is NOT valid: refusing to install"
    SIGNED_LIST="$4/checksums-sha256.txt"
    return 0
  fi
  [ "$_signed_release" = 0 ] || err "release $1 should be signed but its signature could not be fetched: refusing to install"
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
        arm64 | aarch64) echo "macos-arm64" ;;
        x86_64)
          err "no prebuilt kimun-server for Intel Macs (the ONNX runtime dropped x86_64 macOS). Use Docker (ghcr.io/$REPO-server) or 'cargo install --git https://github.com/$REPO kimun_server'"
          ;;
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

# --- ready-default config ---------------------------------------------------
# A plain first `kimun-server` run generates an UNCONFIGURED config (no
# embedder — indexing and search disabled), and the embedder is not editable
# from the web UI. A service must come up working, so seed the same ready
# defaults `--default-config` uses (embedded SQLite + local fastembed). Never
# touches an existing file.
seed_config() {
  [ -f "$CONFIG_FILE" ] && return 0
  mkdir -p "$CONFIG_DIR" "$STATE_DIR"
  cat > "$CONFIG_FILE" <<EOF
# Kimün RAG server configuration — seeded by install-server.sh with working
# local defaults (embedded SQLite + local fastembed embedder, semantic-only).
# All options: https://github.com/$REPO/blob/main/server/config.example.toml

[server]
host = "127.0.0.1"
port = 7573

[vector_db]
type = "sqlite"
path = "$STATE_DIR/rag_sqlite"

[embedder]
type = "fastembed"

[reranker]
enabled = true
top_k = 20
EOF
  info "Seeded default config: $CONFIG_FILE"
}

# --- service setup ----------------------------------------------------------
setup_service() {
  case "$(uname -s)" in
    Linux)
      have systemctl || err "--service requires systemd (systemctl not found)"
      mkdir -p "$(dirname "$SYSTEMD_UNIT")" "$STATE_DIR"
      cat > "$SYSTEMD_UNIT" <<EOF
[Unit]
Description=Kimün RAG server
After=network.target

[Service]
ExecStart=$INSTALL_DIR/$BIN
WorkingDirectory=$STATE_DIR
Restart=on-failure

[Install]
WantedBy=default.target
EOF
      systemctl --user daemon-reload
      systemctl --user enable --now kimun-server.service
      info "Service enabled and started (systemctl --user status kimun-server)"
      ;;
    Darwin)
      mkdir -p "$(dirname "$LAUNCHD_PLIST")" "$STATE_DIR"
      cat > "$LAUNCHD_PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LAUNCHD_LABEL</string>
  <key>ProgramArguments</key>
  <array><string>$INSTALL_DIR/$BIN</string></array>
  <key>WorkingDirectory</key><string>$STATE_DIR</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key>
  <dict><key>SuccessfulExit</key><false/></dict>
</dict>
</plist>
EOF
      launchctl unload "$LAUNCHD_PLIST" 2>/dev/null || true
      launchctl load "$LAUNCHD_PLIST"
      info "Launchd agent loaded (launchctl list | grep $LAUNCHD_LABEL)"
      ;;
  esac
}

# Restart an already-set-up service after a binary update (no-op otherwise).
restart_service_if_present() {
  case "$(uname -s)" in
    Linux)
      [ -f "$SYSTEMD_UNIT" ] || return 0
      systemctl --user try-restart kimun-server.service 2>/dev/null || true
      info "Restarted kimun-server service"
      ;;
    Darwin)
      [ -f "$LAUNCHD_PLIST" ] || return 0
      launchctl kickstart -k "gui/$(id -u)/$LAUNCHD_LABEL" 2>/dev/null || true
      info "Restarted kimun-server launchd agent"
      ;;
  esac
}

# --- the release manifest ---------------------------------------------------
# https://kimun.2co.dev/latest.json names the newest stable release of each series and
# each of its files with a url and sha256 (see .github/scripts/release_manifest.py). It
# is pretty-printed, one key per line, and read here with awk: no jq needed. Anything
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
# Read the manifest and take the server's version from it. On failure sets MANIFEST_WHY.
resolve_from_manifest() {
  MANIFEST="$(fetch_stdout "$MANIFEST_URL" 2>/dev/null)" || { MANIFEST_WHY="cannot reach $MANIFEST_URL"; return 1; }
  printf '%s\n' "$MANIFEST" | grep -q '^  "schema": 1,\{0,1\}$' \
    || { MANIFEST_WHY="$MANIFEST_URL is in a format this installer does not know"; return 1; }
  VERSION="$(manifest_version server)"
  case "$VERSION" in
    "" | *[!0-9A-Za-z.+-]*) VERSION=""; MANIFEST_WHY="$MANIFEST_URL names no server release"; return 1 ;;
  esac
}

main() {
  PLATFORM="$(detect_platform)"
  info "Detected platform: $PLATFORM"

  # Resolve the latest *stable* server release: from the manifest, else from the GitHub
  # API. The API's /releases/latest cannot be used: this repo publishes several tag
  # series and "latest" is the newest of any of them. So that list (newest-first, and
  # only the newest 100 across all series) is scanned for the first kimun_server-v* tag
  # whose version has no pre-release suffix (a hyphen).
  MANIFEST_URL="${KIMUN_MANIFEST_URL:-https://kimun.2co.dev/latest.json}"
  RELEASES_API="${KIMUN_RELEASES_API:-https://api.github.com/repos/$REPO/releases?per_page=100}"
  FROM_MANIFEST=0
  info "Resolving latest server release..."
  if resolve_from_manifest; then
    FROM_MANIFEST=1
  else
    warn "$MANIFEST_WHY; using the GitHub API"
    api_json="$(fetch_stdout "$RELEASES_API")" \
      || err "could not reach the GitHub releases API"
    # grep -o extracts each "tag_name":"..." token individually, so this works
    # even if the API returns the JSON minified onto a single line.
    VERSION="$(printf '%s' "$api_json" \
      | grep -o '"tag_name"[[:space:]]*:[[:space:]]*"[^"]*"' \
      | sed -E 's/.*"([^"]*)"[[:space:]]*$/\1/' \
      | sed -n "s/^${TAG_PREFIX}//p" \
      | grep -v '[-]' \
      | head -n1)"
    [ -n "$VERSION" ] || err "could not find a stable ${TAG_PREFIX}* release"
  fi
  TAG="${TAG_PREFIX}${VERSION}"
  info "Latest version: $VERSION"

  # Idempotent update: skip the download when the installed binary already
  # reports the latest version.
  INSTALLED=""
  if [ -x "$INSTALL_DIR/$BIN" ]; then
    INSTALLED="$("$INSTALL_DIR/$BIN" --version 2>/dev/null | awk '{print $2}')" || INSTALLED=""
  fi

  if [ "$INSTALLED" = "$VERSION" ]; then
    info "$BIN $VERSION already installed — up to date."
    UPDATED=0
  else
    ARCHIVE="kimun-server-${VERSION}-${PLATFORM}.tar.gz"
    BASE_URL="${KIMUN_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download/$TAG}"

    # The manifest names this file's url and sha256: use them, unless the files are
    # being fetched from somewhere else (KIMUN_DOWNLOAD_BASE).
    ARCHIVE_URL="$BASE_URL/$ARCHIVE"
    EXPECTED=""
    if [ "$FROM_MANIFEST" = 1 ] && [ -z "${KIMUN_DOWNLOAD_BASE:-}" ]; then
      m_url="$(manifest_asset server "$ARCHIVE" url)"
      m_sha="$(manifest_asset server "$ARCHIVE" sha256)"
      if [ -n "$m_url" ] && [ -n "$m_sha" ]; then
        ARCHIVE_URL="$m_url"
        EXPECTED="$m_sha"
      fi
    fi

    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT INT TERM

    info "Downloading $ARCHIVE..."
    download "$ARCHIVE_URL" "$tmp/$ARCHIVE" \
      || err "failed to download $ARCHIVE"

    info "Verifying checksum..."
    expected="$EXPECTED"
    verify_signature "$VERSION" "$FIRST_SIGNED" "$BASE_URL" "$tmp"
    if [ -n "$SIGNED_LIST" ]; then
      expected=""
      SUMS="$SIGNED_LIST"
    else
      SUMS="$tmp/checksums-sha256.txt"
      if [ -z "$expected" ]; then
        download "$BASE_URL/checksums-sha256.txt" "$SUMS" \
          || err "failed to download checksums-sha256.txt"
      fi
    fi
    if [ -z "$expected" ]; then
      # Exact filename match (field 2), not a regex — the archive name contains
      # '.' which would otherwise act as a wildcard.
      expected="$(awk -v f="$ARCHIVE" '$2 == f { print $1 }' "$SUMS")"
    fi
    [ -n "$expected" ] || err "no checksum found for $ARCHIVE"
    actual="$(sha256_of "$tmp/$ARCHIVE")"
    [ "$expected" = "$actual" ] \
      || err "checksum mismatch for $ARCHIVE (expected $expected, got $actual)"

    info "Extracting..."
    tar -xzf "$tmp/$ARCHIVE" -C "$tmp" || err "failed to extract $ARCHIVE"
    [ -f "$tmp/$BIN" ] || err "binary '$BIN' not found in archive"
    chmod +x "$tmp/$BIN"

    mkdir -p "$INSTALL_DIR" || err "could not create install dir: $INSTALL_DIR"
    mv -f "$tmp/$BIN" "$INSTALL_DIR/$BIN" \
      || err "could not install to $INSTALL_DIR (is it writable?)"
    info "Installed $BIN to $INSTALL_DIR/$BIN"
    UPDATED=1
  fi

  if [ "$SETUP_SERVICE" -eq 1 ]; then
    seed_config
    setup_service
  elif [ "$UPDATED" -eq 1 ]; then
    restart_service_if_present
  fi

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
  if [ "$SETUP_SERVICE" -eq 1 ]; then
    info "Done. The server is running — open http://127.0.0.1:7573/ for the web UI."
  else
    info "Done. Start the server with '$BIN --default-config' (working local"
    info "defaults), or re-run this script with --service to run it on login."
  fi
  info "First run downloads the embedding model (a few hundred MB)."
}

main "$@"
