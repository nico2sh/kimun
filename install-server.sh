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

SETUP_SERVICE=0
for arg in "$@"; do
  case "$arg" in
    --service) SETUP_SERVICE=1 ;;
    *) err "unknown option: $arg (supported: --service)" ;;
  esac
done

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

main() {
  PLATFORM="$(detect_platform)"
  info "Detected platform: $PLATFORM"

  # Resolve the latest *stable* server release. NOTE: /releases/latest cannot
  # be used — this repo publishes several tag series and "latest" is the newest
  # of any of them. List releases (newest-first) and take the first
  # kimun_server-v* tag whose version has no pre-release suffix (a hyphen).
  info "Resolving latest server release..."
  api_json="$(fetch_stdout "https://api.github.com/repos/$REPO/releases?per_page=100")" \
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
    BASE_URL="https://github.com/$REPO/releases/download/$TAG"

    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT INT TERM

    info "Downloading $ARCHIVE..."
    download "$BASE_URL/$ARCHIVE" "$tmp/$ARCHIVE" \
      || err "failed to download $ARCHIVE"

    info "Verifying checksum..."
    download "$BASE_URL/checksums-sha256.txt" "$tmp/checksums-sha256.txt" \
      || err "failed to download checksums-sha256.txt"
    # Exact filename match (field 2), not a regex — the archive name contains
    # '.' which would otherwise act as a wildcard.
    expected="$(awk -v f="$ARCHIVE" '$2 == f { print $1 }' "$tmp/checksums-sha256.txt")"
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
