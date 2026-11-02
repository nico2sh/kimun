#!/usr/bin/env bash
# The Logic layer (CONTEXT.md § GUI) must not name a UI crate, nor reach back into
# the TUI's own layers. Comment lines are dropped first: module docs state the
# rule, and stating it means writing the names. Only real code is checked.
set -euo pipefail
cd "$(dirname "$0")/../.."
fail=0
leaks=$(grep -rn --include='*.rs' -E '\b(ratatui|crossterm|egui|eframe)\b' tui/src/logic \
          | grep -vE '^[^:]+:[0-9]+: *//' || true)
if [ -n "$leaks" ]; then
  echo "::error::tui/src/logic must not name a UI crate:"
  echo "$leaks"
  fail=1
fi
back=$(grep -rn --include='*.rs' -E 'crate::(components|app::|app_screen)' tui/src/logic \
          | grep -vE '^[^:]+:[0-9]+: *//' || true)
if [ -n "$back" ]; then
  echo "::error::tui/src/logic must not depend on the TUI layers (components, app, app_screen):"
  echo "$back"
  fail=1
fi
exit $fail
