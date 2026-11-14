#!/bin/sh
# Kimün server installer — downloads the latest stable kimun-server release,
# verifies its checksum, and installs the binary into a user-writable directory.
# Re-running the script updates an existing install (and restarts the service,
# if one was set up). It also writes ~/.config/kimun/server-install.toml, the marker
# that tells the Kimün apps they may update this server (ADR-0059).
#
#   curl -fsSL https://kimun.2co.dev/install-server.sh | sh
#   curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service
#
# Options:
#   --service           also set up a user service (systemd on Linux, launchd
#                       on macOS) so the server starts on login and restarts
#                       on failure
#   --uninstall         stop and remove the user service, then remove the server
#                       binary. Your configuration and data are not touched
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
# The install marker: tells the Kimün app that this script installed the server here, so the
# app may update it (ADR-0059). It records the install folder and the kind of service.
MARKER="$CONFIG_DIR/server-install.toml"
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
UNINSTALL=0
for arg in "$@"; do
  case "$arg" in
    --service) SETUP_SERVICE=1 ;;
    --uninstall) UNINSTALL=1 ;;
    *) err "unknown option: $arg (supported: --service, --uninstall)" ;;
  esac
done

# --- release signature ----------------------------------------------------
# Releases carry checksums-sha256.txt.minisig, a minisign signature of their checksums file
# (see .github/RELEASING.md, "Signing"). SIGNING_PUBKEY is release-signing.pub, line 2; empty
# means signing is not live yet and nothing is checked. With a key and minisign installed the
# signature is checked and the digest comes from the file it signed. A missing signature is an
# error: this installer only takes the latest release, which the unsigned manifest names, so a
# release without one is not skipped silently. Without minisign the install goes on on the
# sha256 alone, and says so. install.sh carries the same block, except it only warns for a
# version pinned with KIMUN_VERSION (which may predate signing).
SIGNING_PUBKEY="RWS64g2Q9vGQfwr3tC6fK3UgWdkKkLfz5KLmcTZhewdMmCHHkkfIgNf8"
# verify_signature BASE_URL TMP: sets SIGNED_LIST to the verified checksums file, or "" when
# nothing was verified.
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
      old_unit=""
      [ -f "$SYSTEMD_UNIT" ] && old_unit="$(cat "$SYSTEMD_UNIT")"
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
      # `enable --now` does not restart a unit that already runs, so a new binary (or a changed
      # unit) would sit on the disk while the old process kept serving. A running unit is restarted
      # when either changed, and left alone when neither did.
      was_active=0
      if systemctl --user is-active --quiet kimun-server.service >/dev/null 2>&1; then
        was_active=1
      fi
      changed=0
      [ "$UPDATED" -eq 1 ] && changed=1
      [ "$old_unit" = "$(cat "$SYSTEMD_UNIT")" ] || changed=1
      systemctl --user daemon-reload
      systemctl --user enable --now kimun-server.service
      if [ "$was_active" -eq 1 ] && [ "$changed" -eq 1 ]; then
        systemctl --user restart kimun-server.service \
          || err "could not restart kimun-server.service: run systemctl --user restart kimun-server.service"
        info "Service restarted (systemctl --user status kimun-server)"
      elif [ "$was_active" -eq 1 ]; then
        info "Service already running the current server (systemctl --user status kimun-server)"
      else
        info "Service enabled and started (systemctl --user status kimun-server)"
      fi
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

# --- install marker -----------------------------------------------------------
# Written at the end of every install run, also when the binary was already current, so a
# server that was installed before the marker existed gets one by running this script once.
# The service kind is what exists now: a service set up by an earlier run counts.
write_marker() {
  kind="none"
  case "$(uname -s)" in
    Linux) [ -f "$SYSTEMD_UNIT" ] && kind="systemd" ;;
    Darwin) [ -f "$LAUNCHD_PLIST" ] && kind="launchd" ;;
  esac
  # TOML needs `\` and `"` escaped. INSTALL_DIR is absolute (see main) and has no control characters.
  esc_dir="$(printf '%s' "$INSTALL_DIR" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')"
  mkdir -p "$CONFIG_DIR" || err "could not create $CONFIG_DIR"
  cat > "$MARKER.new" <<EOF
# Written by install-server.sh. The Kimün app reads it to know that it may update this server.
installer = "install-server.sh"
install_dir = "$esc_dir"
service = "$kind"
EOF
  mv -f "$MARKER.new" "$MARKER" || err "could not write $MARKER"
}

# --- uninstall ----------------------------------------------------------------
# Stop and remove the user service if there is one, then the binary. The configuration
# (server.toml) and the data (the vector store and the downloaded embedding model under
# $STATE_DIR) stay: they are the user's, and the model is a few hundred MB to fetch again.
uninstall() {
  removed=0
  remove() { # remove PATH: a file this script created
    if [ -e "$1" ] || [ -L "$1" ]; then
      rm -f "$1" || err "could not remove $1"
      info "Removed $1"
      removed=1
    fi
  }
  # A failed stop is said aloud: the binary goes next, and a server that keeps running from
  # a deleted binary cannot be managed by the service any more.
  case "$(uname -s)" in
    Linux)
      if have systemctl; then
        # Also stop a unit that is running but whose file is gone (or lives elsewhere).
        if [ -f "$SYSTEMD_UNIT" ] || systemctl --user is-active --quiet kimun-server.service >/dev/null 2>&1; then
          systemctl --user stop kimun-server.service >/dev/null 2>&1 \
            || warn "could not stop kimun-server.service (no user session?). If the server is still running, stop it yourself: systemctl --user stop kimun-server.service"
        fi
        if [ -f "$SYSTEMD_UNIT" ]; then
          systemctl --user disable kimun-server.service >/dev/null 2>&1 \
            || warn "could not disable kimun-server.service: run systemctl --user disable kimun-server.service"
        fi
      elif [ -f "$SYSTEMD_UNIT" ]; then
        warn "systemctl not found: the service was not stopped. If the server is still running, end the $BIN process yourself"
      fi
      if [ -f "$SYSTEMD_UNIT" ]; then
        remove "$SYSTEMD_UNIT"
        if have systemctl; then systemctl --user daemon-reload >/dev/null 2>&1 || true; fi
      fi
      ;;
    Darwin)
      if [ -f "$LAUNCHD_PLIST" ]; then
        # launchctl also fails when the job is simply not loaded, so this is a warning.
        launchctl unload "$LAUNCHD_PLIST" >/dev/null 2>&1 \
          || warn "launchctl could not unload $LAUNCHD_PLIST (it may not be loaded). If the server is still running, end the $BIN process yourself"
        remove "$LAUNCHD_PLIST"
      fi
      ;;
  esac
  remove "$INSTALL_DIR/$BIN"
  remove "$MARKER"
  if [ "$removed" = 0 ]; then
    info "Nothing to remove: no $BIN found in $INSTALL_DIR."
  else
    info "Done. Your configuration ($CONFIG_FILE) and data ($STATE_DIR) were not touched."
  fi
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
  if [ "$UNINSTALL" -eq 1 ]; then
    [ "$SETUP_SERVICE" -eq 0 ] || err "--uninstall cannot be combined with --service"
    uninstall
    exit 0
  fi
  PLATFORM="$(detect_platform)"
  info "Detected platform: $PLATFORM"

  # An absolute install folder: the service file and the marker both need one. Nothing is created
  # here: a run that fails early leaves nothing behind.
  case "$INSTALL_DIR" in
    /*) ;;
    *) INSTALL_DIR="$(pwd)/$INSTALL_DIR" ;;
  esac
  # A folder name with a control character (a newline, a tab) cannot go into the marker or the
  # service file.
  case "$INSTALL_DIR" in
    *[![:print:]]*) err "the install folder has a control character in its name: choose another with KIMUN_SERVER_INSTALL_DIR" ;;
  esac

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
    verify_signature "$BASE_URL" "$tmp"
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
  write_marker

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
    info "Done. The server is installed but NOT set up to start automatically:"
    info "it will not run after a restart or login."
    info ""
    info "To start it once, run '$BIN --default-config' (working local defaults)."
    info "To start it on every login, re-run the installer with --service:"
    info "  curl -fsSL https://kimun.2co.dev/install-server.sh | sh -s -- --service"
  fi
  info "First run downloads the embedding model (a few hundred MB)."
}

main "$@"
