#!/bin/sh
# Seed /data/server.toml on first start, then run the server against it.
#
# Always start with --config rather than --default-config: --default-config
# means "run with built-in defaults", it never READS the file it leaves
# behind — config edits (web UI or manual) would be silently ignored on
# restart. Extra arguments are passed through to kimun-server.
set -eu

CONFIG=/data/server.toml

if [ ! -f "$CONFIG" ]; then
  cp /usr/local/share/kimun/server.toml.default "$CONFIG"
  echo "Seeded default config at $CONFIG (SQLite + fastembed, semantic-only)"
fi

exec /usr/local/bin/kimun-server --config "$CONFIG" "$@"
