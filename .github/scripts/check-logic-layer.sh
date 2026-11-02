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
# Brace imports (`use crate::{components::X, ...}`, possibly multi-line) dodge the
# literal-path grep above, so scan each `use crate::{ ... };` block's top-level heads.
brace=$(find tui/src/logic -name '*.rs' -print0 | xargs -0 perl -0777 -ne '
  s{^\s*//.*$}{}mg;
  while (/use\s+crate::\{(.*?)\};/sg) {
    my $b = $1;
    print "$ARGV: use crate::{...} names a TUI layer\n" if $b =~ /(?:^|,)\s*(?:components|app|app_screen)\b/;
  }' || true)
if [ -n "$brace" ]; then
  echo "::error::tui/src/logic must not depend on the TUI layers (brace import):"
  echo "$brace"
  fail=1
fi
# Crossterm-typed key conversion lives outside the Logic layer.
xt=$(grep -rn --include='*.rs' -E 'crate::keys::(crossterm_input|key_event_to_combo)|keys::\{[^}]*crossterm_input' tui/src/logic \
          | grep -vE '^[^:]+:[0-9]+: *//' || true)
if [ -n "$xt" ]; then
  echo "::error::tui/src/logic must not use crossterm-typed key conversion:"
  echo "$xt"
  fail=1
fi
exit $fail
