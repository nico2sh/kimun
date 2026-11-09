#!/usr/bin/env bash
# Smoke test for the desktop app: start `kimun-gui` for a few seconds in two
# situations and fail if it exits early or panics.
#
#   first-run   an empty home: the setup assistant (Welcome) is what shows
#   workspace   a config naming a workspace with a note: the window stays up
#               (alive and panic-free is all this checks, not what it shows)
#
# Everything runs against a throw-away HOME, so it never touches real notes or
# config. It needs a display (always assumed on macOS). With none ($DISPLAY and $WAYLAND_DISPLAY unset)
# it re-runs itself under `xvfb-run`, which also needs a Vulkan driver that
# works without a GPU (Mesa's lavapipe: `mesa-vulkan-drivers`).
#
# Usage: gui-smoke.sh [path/to/kimun-gui] [seconds-per-scenario]
set -euo pipefail

# Absolute before the `cd`: $0 and a relative binary path are the caller's, and
# the re-run under xvfb-run starts from the repo root.
self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
case "${1:-}" in
  /*) bin="$1" ;;
  "") bin="" ;;
  *) bin="$(pwd)/$1" ;;
esac
seconds="${2:-6}"
cd "$(dirname "$self")/../.."
bin="${bin:-$PWD/target/debug/kimun-gui}"

# macOS draws through its own window server, so there is no $DISPLAY to look for.
if [ "$(uname -s)" != "Darwin" ] && [ -z "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
  command -v xvfb-run >/dev/null || {
    echo "no display and no xvfb-run: install xvfb, or run on a desktop" >&2
    exit 2
  }
  exec xvfb-run -a "$self" "$bin" "$seconds"
fi

if [ ! -x "$bin" ]; then
  cargo build -p kimun-gui
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

run() {
  local name="$1" home="$2"
  shift 2
  local log="$work/$name.log"
  echo "== $name"
  HOME="$home" XDG_CONFIG_HOME="$home/.config" XDG_STATE_HOME="$home/.state" \
    XDG_DATA_HOME="$home/.data" XDG_CACHE_HOME="$home/.cache" \
    "$bin" "$@" >"$log" 2>&1 &
  local pid=$!
  sleep "$seconds"
  if ! kill -0 "$pid" 2>/dev/null; then
    wait "$pid" && status=0 || status=$?
    echo "::error::$name: kimun-gui exited early (status $status)"
    cat "$log"
    return 1
  fi
  kill -TERM "$pid"
  wait "$pid" 2>/dev/null || true
  if grep -qE "panicked at|thread '.*' panicked" "$log"; then
    echo "::error::$name: kimun-gui panicked"
    cat "$log"
    return 1
  fi
  echo "ok: alive after ${seconds}s, no panic"
}

# 1. First run: an empty home.
mkdir -p "$work/first/.config"
run first-run "$work/first"

# 2. A configured workspace holding a note.
ws="$work/ws-home/notes"
# The config's own current version, so the fixture never runs a migration by
# accident when it moves.
version="$(sed -n 's/^pub const CURRENT_CONFIG_VERSION: u32 = \([0-9]*\);/\1/p' \
  tui/src/settings/config_migration.rs)"
[ -n "$version" ] || { echo "::error::CURRENT_CONFIG_VERSION not found" >&2; exit 2; }
mkdir -p "$ws" "$work/ws-home/.config/kimun"
printf '# Smoke\n\nA note with a [[link]] and a #tag.\n' >"$ws/smoke.md"
cat >"$work/ws-home/.config/kimun/config.toml" <<EOF
config_version = $version
[global]
current_workspace = "smoke"
[workspaces.smoke]
path = "$ws"
created = "2026-01-01T00:00:00Z"
EOF
run workspace "$work/ws-home"

echo "gui smoke: ok"
