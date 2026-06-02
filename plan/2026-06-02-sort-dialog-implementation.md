# Sort Dialog Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the sidebar/query-panel's two sort keybindings with a single shortcut that opens a sort dialog (sort field, order, and a sidebar-only "group directories" toggle).

**Architecture:** A new `SortDialog` overlay (modelled on `HelpDialog`) is opened centrally from `editor.rs` for whichever panel is focused. As the user toggles rows, the dialog emits `AppEvent::SortChanged`; `editor.rs` routes it to the focused panel, which applies it live (sidebar mutates its shared sort + group flag and reloads; query panel rewrites the `or:` directive in its query string). A `SortSaveDefault` event persists the sidebar's choice to settings. Directory grouping is done in the sidebar's `DirListingSource::load`.

**Tech Stack:** Rust, ratatui, tokio, the existing `kimun_core` query DSL (`core/src/db/search_terms.rs`).

---

## File Structure

- `core/src/db/search_terms.rs` — add `OrderField` enum + `with_order_directive()` (DSL serialization stays in core).
- `tui/src/components/file_list.rs` — add reverse `From` conversions (`SortField → SortFieldSetting`, `SortOrder → SortOrderSetting`); add `Debug` to the two enums.
- `tui/src/settings/mod.rs` — add `group_directories: bool` setting + default; bind `OpenSortDialog` to Ctrl+N; drop the two old bindings.
- `tui/src/keys/action_shortcuts.rs` — replace `CycleSortField`/`SortReverseOrder` with `OpenSortDialog` (+ legacy parse aliases).
- `tui/src/components/events.rs` — add `SortTarget` enum + `SortChanged`/`SortSaveDefault` events.
- `tui/src/components/dialogs/sort_dialog.rs` — **new** dialog component.
- `tui/src/components/dialogs/mod.rs` — register `ActiveDialog::Sort` variant + constructor + routing.
- `tui/src/components/sidebar.rs` — shared `group_dirs` flag, grouping in load, `apply_sort`/`current_sort`/`group_dirs` accessors; remove old cycle/reverse + intercept.
- `tui/src/components/backlinks_panel.rs` — `apply_sort`/`current_order`; remove the separate sort override; title indicator derives from the query.
- `tui/src/app_screen/editor.rs` — dispatch `OpenSortDialog`; route `SortChanged`/`SortSaveDefault`.

---

## Task 1: Core — order-directive rewrite

**Files:**
- Modify: `core/src/db/search_terms.rs` (constants at lines 5-6; `OrderBy` at 137-154; append public fn + tests)

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `core/src/db/search_terms.rs` (after the existing tests, before the closing `}`):

```rust
    #[test]
    fn with_order_inserts_into_plain_query() {
        use super::{with_order_directive, OrderField};
        assert_eq!(
            with_order_directive("hello world", OrderField::Title, true),
            "hello world or:title"
        );
        assert_eq!(
            with_order_directive("hello", OrderField::FileName, false),
            "hello -or:file"
        );
    }

    #[test]
    fn with_order_replaces_existing_directive() {
        use super::{with_order_directive, OrderField};
        // Long form replaced.
        assert_eq!(
            with_order_directive("foo or:title bar", OrderField::FileName, true),
            "foo bar or:file"
        );
        // Descending long form replaced.
        assert_eq!(
            with_order_directive("-or:file foo", OrderField::Title, true),
            "foo or:title"
        );
        // Short forms (`^` / `-^`) replaced too.
        assert_eq!(
            with_order_directive("foo ^title", OrderField::Title, false),
            "foo -or:title"
        );
        assert_eq!(
            with_order_directive("-^file foo", OrderField::FileName, true),
            "foo or:file"
        );
    }

    #[test]
    fn with_order_empty_query_yields_bare_directive() {
        use super::{with_order_directive, OrderField};
        assert_eq!(
            with_order_directive("", OrderField::Title, true),
            "or:title"
        );
    }

    #[test]
    fn with_order_roundtrips_through_parser() {
        use super::{with_order_directive, OrderField, OrderBy, SearchTerms};
        let q = with_order_directive("note text", OrderField::Title, false);
        let st = SearchTerms::from_query_string(&q);
        assert!(matches!(
            st.order_by.first(),
            Some(OrderBy::Title { asc: false })
        ));
        // The free-text term survives the rewrite.
        assert!(st.terms.iter().any(|t| t == "note"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kimun_core with_order`
Expected: FAIL — `cannot find function with_order_directive` / `cannot find type OrderField`.

- [ ] **Step 3: Implement `OrderField` + `with_order_directive`**

In `core/src/db/search_terms.rs`, immediately after the `impl OrderBy { ... }` block (ends at line 154), add:

```rust
/// The field a query can be ordered by. The asc/desc choice is carried
/// separately by callers; this names only the column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderField {
    Title,
    FileName,
}

/// True if `token` is an order directive in any of its four forms:
/// `or:<x>`, `-or:<x>`, `^<x>`, `-^<x>`.
fn is_order_token(token: &str) -> bool {
    let order_prefix = format!("{}:", ORDER_LETTER);
    let desc_order_prefix = format!("-{}:", ORDER_LETTER);
    let desc_order_char = format!("-{}", ORDER_CHAR);
    token.starts_with(&desc_order_prefix)
        || token.starts_with(&order_prefix)
        || token.starts_with(&desc_order_char)
        || token.starts_with(ORDER_CHAR)
}

/// Return `query` with its order directive replaced by `field`/`asc`.
///
/// Any existing order directive (`or:`/`-or:`/`^`/`-^`, in any position) is
/// stripped, then the canonical `or:<field>` (ascending) / `-or:<field>`
/// (descending) directive is appended. Other tokens are preserved in order
/// (whitespace is normalised to single spaces). The DSL knowledge lives here in
/// core so the TUI never hardcodes the directive syntax.
pub fn with_order_directive(query: &str, field: OrderField, asc: bool) -> String {
    let kept: Vec<&str> = query
        .split_whitespace()
        .filter(|t| !is_order_token(t))
        .collect();
    let field_term = match field {
        OrderField::Title => "title",
        OrderField::FileName => "file",
    };
    let prefix = if asc { ORDER_LETTER } else { "-or" }; // "or" / "-or"
    let directive = format!("{}:{}", prefix, field_term);
    if kept.is_empty() {
        directive
    } else {
        format!("{} {}", kept.join(" "), directive)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kimun_core with_order`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add core/src/db/search_terms.rs
git commit -m "feat(core): add with_order_directive query rewrite helper"
```

---

## Task 2: TUI — reverse sort-setting conversions + Debug

**Files:**
- Modify: `tui/src/components/file_list.rs:15-43` (enum derives + `From` impls)

- [ ] **Step 1: Write the failing test**

Add a test module at the end of `tui/src/components/file_list.rs` (the file currently has no `#[cfg(test)]` block — add one):

```rust
#[cfg(test)]
mod tests {
    use super::{SortField, SortOrder};
    use crate::settings::{SortFieldSetting, SortOrderSetting};

    #[test]
    fn sort_field_setting_roundtrip() {
        assert_eq!(SortFieldSetting::from(SortField::Name), SortFieldSetting::Name);
        assert_eq!(SortFieldSetting::from(SortField::Title), SortFieldSetting::Title);
        assert_eq!(SortField::from(SortFieldSetting::Title), SortField::Title);
    }

    #[test]
    fn sort_order_setting_roundtrip() {
        assert_eq!(
            SortOrderSetting::from(SortOrder::Ascending),
            SortOrderSetting::Ascending
        );
        assert_eq!(
            SortOrderSetting::from(SortOrder::Descending),
            SortOrderSetting::Descending
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kimun-tui sort_field_setting_roundtrip`
Expected: FAIL — `the trait From<SortField> is not implemented for SortFieldSetting`.

(Note: the test crate name may be `kimun_tui`/`tui`; if `-p kimun-tui` errors with "package not found", run `cargo test -p $(sed -n 's/^name = "\(.*\)"/\1/p' tui/Cargo.toml | head -1) sort_field_setting_roundtrip`.)

- [ ] **Step 3: Add `Debug` derive + reverse `From` impls**

In `tui/src/components/file_list.rs`, change the two enum derives (lines 15 and 21):

```rust
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SortField {
    Name,
    Title,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SortOrder {
    Ascending,
    Descending,
}
```

Then, immediately after the existing `impl From<SortOrderSetting> for SortOrder { ... }` block (ends at line 43), add the reverse conversions:

```rust
impl From<SortField> for SortFieldSetting {
    fn from(s: SortField) -> Self {
        match s {
            SortField::Name => Self::Name,
            SortField::Title => Self::Title,
        }
    }
}

impl From<SortOrder> for SortOrderSetting {
    fn from(s: SortOrder) -> Self {
        match s {
            SortOrder::Ascending => Self::Ascending,
            SortOrder::Descending => Self::Descending,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kimun-tui sort_field_setting_roundtrip sort_order_setting_roundtrip`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/file_list.rs
git commit -m "feat(tui): add reverse SortField/SortOrder setting conversions"
```

---

## Task 3: Settings — `group_directories` flag + keybinding swap

**Files:**
- Modify: `tui/src/settings/mod.rs` (struct field ~137, default fn ~245, `Default` impl ~268, keybindings ~181-183)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `tui/src/settings/mod.rs` (if no test module exists in this file, add one at the end):

```rust
#[cfg(test)]
mod sort_settings_tests {
    use super::*;

    #[test]
    fn group_directories_defaults_off() {
        let s = AppSettings::default();
        assert!(!s.group_directories);
    }

    #[test]
    fn open_sort_dialog_is_bound_by_default() {
        let s = AppSettings::default();
        let map = s.key_bindings.to_hashmap();
        assert!(
            map.contains_key(&ActionShortcuts::OpenSortDialog),
            "OpenSortDialog must have a default binding"
        );
        // The two old actions must no longer be bound.
        assert!(!map.contains_key(&ActionShortcuts::OpenSettings) == false); // sanity: map non-empty
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kimun-tui group_directories_defaults_off`
Expected: FAIL — `no field group_directories on type AppSettings` (and `no variant OpenSortDialog`, which Task 4 adds; this task and Task 4 compile together — if running Task 3 alone fails to compile on `OpenSortDialog`, do Task 4 first then return; recommended order is 4 before 3. See note below).

> **Ordering note:** Task 4 introduces `ActionShortcuts::OpenSortDialog`. Implement Task 4 before this step's `cargo test` so the enum variant exists. The two tasks are committed separately but compiled together.

- [ ] **Step 3: Add the `group_directories` field**

In `tui/src/settings/mod.rs`, add the field to `AppSettings` right after `journal_sort_order` (line 137):

```rust
    #[serde(default = "default_journal_sort_order")]
    pub journal_sort_order: SortOrderSetting,
    #[serde(default)]
    pub group_directories: bool,
```

Add it to the `Default` impl right after `journal_sort_order: default_journal_sort_order(),` (line 268):

```rust
            journal_sort_order: default_journal_sort_order(),
            group_directories: false,
```

(`bool`'s `Default` is `false`, so `#[serde(default)]` needs no helper fn.)

- [ ] **Step 4: Swap the keybindings**

In `default_keybindings()`, replace lines 181 and 183:

```rust
        .add(KeyStrike::KeyN, ActionShortcuts::CycleSortField)
```
becomes
```rust
        .add(KeyStrike::KeyN, ActionShortcuts::OpenSortDialog)
```

and delete the `SortReverseOrder` binding line entirely:
```rust
        .add(KeyStrike::KeyR, ActionShortcuts::SortReverseOrder)
```
(remove this line; Ctrl+R becomes unbound).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p kimun-tui group_directories_defaults_off open_sort_dialog_is_bound_by_default`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add tui/src/settings/mod.rs
git commit -m "feat(tui): add group_directories setting; bind OpenSortDialog to Ctrl+N"
```

---

## Task 4: Action enum — replace two sort actions with `OpenSortDialog`

**Files:**
- Modify: `tui/src/keys/action_shortcuts.rs` (enum 40-41; category 65-66; label 99-100; Display 136-137; TryFrom 165-166; tests 231-237, 315-319)

- [ ] **Step 1: Update the tests first**

In `tui/src/keys/action_shortcuts.rs`, replace the two `CycleSortField`/`SortReverseOrder` category assertions (lines 230-237) with a single `OpenSortDialog` assertion:

```rust
        assert_eq!(
            ActionShortcuts::OpenSortDialog.category(),
            ShortcutCategory::Navigation
        );
```

Replace the two label assertions (lines 315-319) with:

```rust
        assert_eq!(ActionShortcuts::OpenSortDialog.label(), "Sort options");
```

Add a new roundtrip + legacy-alias test inside the `tests` module:

```rust
    #[test]
    fn open_sort_dialog_roundtrip_and_legacy_alias() {
        assert_eq!(
            ActionShortcuts::OpenSortDialog.to_string(),
            "OpenSortDialog"
        );
        assert_eq!(
            ActionShortcuts::try_from("OpenSortDialog".to_string()),
            Ok(ActionShortcuts::OpenSortDialog)
        );
        // Legacy action names migrate to the new single action.
        assert_eq!(
            ActionShortcuts::try_from("CycleSortField".to_string()),
            Ok(ActionShortcuts::OpenSortDialog)
        );
        assert_eq!(
            ActionShortcuts::try_from("SortReverseOrder".to_string()),
            Ok(ActionShortcuts::OpenSortDialog)
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kimun-tui open_sort_dialog_roundtrip_and_legacy_alias`
Expected: FAIL — `no variant named OpenSortDialog`.

- [ ] **Step 3: Replace the enum variants and all match arms**

In the `ActionShortcuts` enum (lines 40-41), replace:
```rust
    CycleSortField,
    SortReverseOrder,
```
with:
```rust
    OpenSortDialog,
```

In `category()` (lines 65-66), replace:
```rust
            | ActionShortcuts::CycleSortField
            | ActionShortcuts::SortReverseOrder
```
with:
```rust
            | ActionShortcuts::OpenSortDialog
```

In `label()` (lines 99-100), replace both arms with:
```rust
            ActionShortcuts::OpenSortDialog => "Sort options".into(),
```

In `Display` (lines 136-137), replace both arms with:
```rust
            ActionShortcuts::OpenSortDialog => "OpenSortDialog".to_string(),
```

In `TryFrom<String>` (lines 165-166), replace both arms with the new name plus the two legacy aliases:
```rust
            "OpenSortDialog" => ActionShortcuts::OpenSortDialog,
            // Legacy action names → the single sort dialog action.
            "CycleSortField" => ActionShortcuts::OpenSortDialog,
            "SortReverseOrder" => ActionShortcuts::OpenSortDialog,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kimun-tui -- action_shortcuts open_sort_dialog`
Expected: PASS (existing `action_shortcuts_categories`, `action_shortcuts_labels`, and the new test all green).

- [ ] **Step 5: Commit**

```bash
git add tui/src/keys/action_shortcuts.rs
git commit -m "feat(tui): replace sort field/order actions with OpenSortDialog"
```

---

## Task 5: Events — `SortTarget` + sort events

**Files:**
- Modify: `tui/src/components/events.rs` (imports near top; `AppEvent` enum 12-127)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `tui/src/components/events.rs` (extend `_assert_new_variants_exist` and add a constructor test):

```rust
    #[test]
    fn sort_events_construct() {
        use crate::components::file_list::{SortField, SortOrder};
        let _ = AppEvent::SortChanged {
            target: SortTarget::Sidebar,
            field: SortField::Name,
            order: SortOrder::Ascending,
            group_directories: true,
        };
        let _ = AppEvent::SortSaveDefault {
            target: SortTarget::Query,
            field: SortField::Title,
            order: SortOrder::Descending,
            group_directories: false,
        };
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kimun-tui sort_events_construct`
Expected: FAIL — `no variant SortChanged` / `cannot find type SortTarget`.

- [ ] **Step 3: Add `SortTarget` and the two events**

In `tui/src/components/events.rs`, add the import near the other `use` lines (after line 8):

```rust
use crate::components::file_list::{SortField, SortOrder};
```

Add the `SortTarget` enum just above `pub enum AppEvent` (before line 12):

```rust
/// Which panel a sort selection applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortTarget {
    Sidebar,
    Query,
}
```

Add the two variants inside `AppEvent` (e.g. right before the closing `}` at line 127, after `SavedSearchSelected`):

```rust
    /// Sort selection changed in the sort dialog — apply live to `target`.
    SortChanged {
        target: SortTarget,
        field: SortField,
        order: SortOrder,
        /// Sidebar only; ignored by the query panel.
        group_directories: bool,
    },
    /// Persist the current sort selection as the default for `target`.
    SortSaveDefault {
        target: SortTarget,
        field: SortField,
        order: SortOrder,
        group_directories: bool,
    },
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kimun-tui sort_events_construct`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/events.rs
git commit -m "feat(tui): add SortTarget and sort dialog events"
```

---

## Task 6: Sort dialog component

**Files:**
- Create: `tui/src/components/dialogs/sort_dialog.rs`

- [ ] **Step 1: Write the failing tests (logic only)**

Create `tui/src/components/dialogs/sort_dialog.rs` with the full implementation below in Step 3, but author the tests first so they drive the API. The tests (place in the `#[cfg(test)] mod tests` at the bottom of the new file):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::events::SortTarget;
    use crate::components::file_list::{SortField, SortOrder};
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use tokio::sync::mpsc::unbounded_channel;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn sidebar_dialog() -> SortDialog {
        SortDialog::new(SortTarget::Sidebar, SortField::Name, SortOrder::Ascending, false)
    }

    #[test]
    fn space_toggles_field_and_emits_change() {
        let mut d = sidebar_dialog();
        let (tx, mut rx) = unbounded_channel();
        // Row 0 is "Sort by"; Space flips Name -> Title.
        d.handle_key(key(KeyCode::Char(' ')), &tx);
        assert_eq!(d.field, SortField::Title);
        let evt = rx.try_recv().expect("a SortChanged event");
        match evt {
            AppEvent::SortChanged { target, field, order, group_directories } => {
                assert_eq!(target, SortTarget::Sidebar);
                assert_eq!(field, SortField::Title);
                assert_eq!(order, SortOrder::Ascending);
                assert!(!group_directories);
            }
            other => panic!("expected SortChanged, got {other:?}"),
        }
    }

    #[test]
    fn down_then_space_toggles_order() {
        let mut d = sidebar_dialog();
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Down), &tx); // move to "Order" row
        assert!(rx.try_recv().is_err(), "navigation alone emits nothing");
        d.handle_key(key(KeyCode::Char(' ')), &tx);
        assert_eq!(d.order, SortOrder::Descending);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::SortChanged { .. })));
    }

    #[test]
    fn group_row_present_only_for_sidebar() {
        let sidebar = sidebar_dialog();
        assert_eq!(sidebar.row_count(), 3); // field, order, group
        let query = SortDialog::new(
            SortTarget::Query,
            SortField::Name,
            SortOrder::Ascending,
            false,
        );
        assert_eq!(query.row_count(), 2); // field, order — no group row
    }

    #[test]
    fn s_saves_default_for_sidebar_only() {
        let mut d = sidebar_dialog();
        let (tx, mut rx) = unbounded_channel();
        d.handle_key(key(KeyCode::Char('s')), &tx);
        assert!(matches!(rx.try_recv(), Ok(AppEvent::SortSaveDefault { .. })));

        let mut q = SortDialog::new(
            SortTarget::Query,
            SortField::Name,
            SortOrder::Ascending,
            false,
        );
        let (tx2, mut rx2) = unbounded_channel();
        q.handle_key(key(KeyCode::Char('s')), &tx2);
        assert!(rx2.try_recv().is_err(), "query target has no save-default");
    }

    #[test]
    fn enter_and_esc_close_overlay() {
        for code in [KeyCode::Enter, KeyCode::Esc] {
            let mut d = sidebar_dialog();
            let (tx, mut rx) = unbounded_channel();
            d.handle_key(key(code), &tx);
            assert!(matches!(rx.try_recv(), Ok(AppEvent::CloseOverlay)));
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kimun-tui sort_dialog`
Expected: FAIL — file/module not declared, `SortDialog` undefined.

- [ ] **Step 3: Implement the dialog**

Write the full `tui/src/components/dialogs/sort_dialog.rs`:

```rust
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::components::event_state::EventState;
use crate::components::events::{AppEvent, AppTx, SortTarget};
use crate::components::file_list::{SortField, SortOrder};
use crate::settings::themes::Theme;

/// The selectable rows, in display order.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Field,
    Order,
    GroupDirs,
}

/// Modal that edits sort field / order (+ a sidebar-only "group directories"
/// toggle). Changes apply live: each toggle emits `AppEvent::SortChanged`.
/// `s` (sidebar only) emits `SortSaveDefault`; Enter/Esc emit `CloseOverlay`.
pub struct SortDialog {
    target: SortTarget,
    pub(crate) field: SortField,
    pub(crate) order: SortOrder,
    group_dirs: bool,
    /// Rows shown for this target (the group row is sidebar-only).
    rows: Vec<Row>,
    selected: usize,
}

impl SortDialog {
    pub fn new(
        target: SortTarget,
        field: SortField,
        order: SortOrder,
        group_dirs: bool,
    ) -> Self {
        let mut rows = vec![Row::Field, Row::Order];
        if target == SortTarget::Sidebar {
            rows.push(Row::GroupDirs);
        }
        Self {
            target,
            field,
            order,
            group_dirs,
            rows,
            selected: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn emit_change(&self, tx: &AppTx) {
        tx.send(AppEvent::SortChanged {
            target: self.target,
            field: self.field,
            order: self.order,
            group_directories: self.group_dirs,
        })
        .ok();
    }

    /// Toggle the value on the selected row, then emit the live change.
    fn toggle_selected(&mut self, tx: &AppTx) {
        match self.rows[self.selected] {
            Row::Field => self.field = self.field.cycle(),
            Row::Order => self.order = self.order.toggle(),
            Row::GroupDirs => self.group_dirs = !self.group_dirs,
        }
        self.emit_change(tx);
    }

    pub fn handle_key(&mut self, key: KeyEvent, tx: &AppTx) -> EventState {
        match key.code {
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.rows.len() - 1);
            }
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right => {
                self.toggle_selected(tx);
            }
            KeyCode::Char('s') if self.target == SortTarget::Sidebar => {
                tx.send(AppEvent::SortSaveDefault {
                    target: self.target,
                    field: self.field,
                    order: self.order,
                    group_directories: self.group_dirs,
                })
                .ok();
            }
            KeyCode::Enter | KeyCode::Esc => {
                tx.send(AppEvent::CloseOverlay).ok();
            }
            _ => {}
        }
        EventState::Consumed
    }

    fn row_label(&self, row: Row) -> (String, String) {
        match row {
            Row::Field => (
                "Sort by".to_string(),
                match self.field {
                    SortField::Name => "Name".to_string(),
                    SortField::Title => "Title".to_string(),
                },
            ),
            Row::Order => (
                "Order".to_string(),
                match self.order {
                    SortOrder::Ascending => "Ascending \u{2191}".to_string(),
                    SortOrder::Descending => "Descending \u{2193}".to_string(),
                },
            ),
            Row::GroupDirs => (
                "Group directories".to_string(),
                if self.group_dirs { "On" } else { "Off" }.to_string(),
            ),
        }
    }
}

const OUTER_WIDTH: u16 = 44;

impl crate::components::Component for SortDialog {
    fn handle_input(
        &mut self,
        event: &crate::components::events::InputEvent,
        tx: &AppTx,
    ) -> EventState {
        if let crate::components::events::InputEvent::Key(key) = event {
            self.handle_key(*key, tx)
        } else {
            EventState::NotConsumed
        }
    }

    fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, _focused: bool) {
        // borders(2) + rows + footer(1) + a blank line above the footer(1).
        let outer_height = self.rows.len() as u16 + 4;
        let popup = super::fixed_centered_rect(OUTER_WIDTH, outer_height, rect);
        f.render_widget(Clear, popup);

        let block = Block::default()
            .title(" Sort ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.fg.to_ratatui()))
            .style(theme.panel_style());
        let inner = block.inner(popup);
        f.render_widget(block, popup);
        if inner.height < 2 {
            return;
        }

        let bg = theme.bg_panel.to_ratatui();
        let fg = theme.fg.to_ratatui();
        let fg_muted = theme.fg_muted.to_ratatui();
        let fg_sel = theme.fg_selected.to_ratatui();
        let bg_sel = theme.bg_selected.to_ratatui();

        // One line per row, then a blank line, then the footer hint.
        for (i, &row) in self.rows.iter().enumerate() {
            let y = inner.y + i as u16;
            if y >= inner.y + inner.height {
                break;
            }
            let (label, value) = self.row_label(row);
            let selected = i == self.selected;
            let style = if selected {
                Style::default().fg(fg_sel).bg(bg_sel).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(fg).bg(bg)
            };
            let marker = if selected { ">" } else { " " };
            f.render_widget(
                Paragraph::new(format!(" {marker} {label:<20}{value}")).style(style),
                Rect { x: inner.x, y, width: inner.width, height: 1 },
            );
        }

        let footer_y = inner.y + inner.height.saturating_sub(1);
        let footer = if self.target == SortTarget::Sidebar {
            "  [↑↓] Move  [Space] Toggle  [s] Save default  [Enter/Esc] Close"
        } else {
            "  [↑↓] Move  [Space] Toggle  [Enter/Esc] Close"
        };
        f.render_widget(
            Paragraph::new(footer).style(Style::default().fg(fg_muted).bg(bg)),
            Rect { x: inner.x, y: footer_y, width: inner.width, height: 1 },
        );
    }
}
```

Then declare the module: in `tui/src/components/dialogs/mod.rs`, add to the `pub mod` list (after line 49) `pub mod sort_dialog;` and to the `pub use` block (after line 8) `pub use sort_dialog::SortDialog;`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p kimun-tui sort_dialog`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/dialogs/sort_dialog.rs tui/src/components/dialogs/mod.rs
git commit -m "feat(tui): add SortDialog overlay component"
```

---

## Task 7: Register `ActiveDialog::Sort`

**Files:**
- Modify: `tui/src/components/dialogs/mod.rs` (enum 52-62; `set_error` 65-77; constructors 79-115; `Component::handle_input` 198-209; `Component::render` 211-223)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block at the bottom of `tui/src/components/dialogs/mod.rs`:

```rust
    #[test]
    fn active_dialog_sort_variant_compiles() {
        use crate::components::events::SortTarget;
        use crate::components::file_list::{SortField, SortOrder};
        let _active: ActiveDialog = ActiveDialog::sort(
            SortTarget::Sidebar,
            SortField::Name,
            SortOrder::Ascending,
            false,
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kimun-tui active_dialog_sort_variant_compiles`
Expected: FAIL — `no variant or associated item named sort` / `no variant Sort`.

- [ ] **Step 3: Wire the variant through `ActiveDialog`**

In `tui/src/components/dialogs/mod.rs`:

Add the variant to the `ActiveDialog` enum (after `SaveSearch(SaveSearchDialog),` line 61):
```rust
    Sort(SortDialog),
```

Add to `set_error` (in the match, after the `SaveSearch` arm line 75):
```rust
            ActiveDialog::Sort(_) => {} // no error state
```

Add the constructor (in the `impl ActiveDialog` block, after `save_search` ~line 98). Note the imports `SortField`, `SortOrder`, `SortTarget` are needed — add `use crate::components::events::SortTarget;` and `use crate::components::file_list::{SortField, SortOrder};` to the file's imports (top of file):
```rust
    pub fn sort(
        target: SortTarget,
        field: SortField,
        order: SortOrder,
        group_directories: bool,
    ) -> Self {
        ActiveDialog::Sort(SortDialog::new(target, field, order, group_directories))
    }
```

Add to `Component::handle_input` match (after `SaveSearch` arm line 207):
```rust
            ActiveDialog::Sort(d) => d.handle_input(event, tx),
```

Add to `Component::render` match (after `SaveSearch` arm line 221):
```rust
            ActiveDialog::Sort(d) => d.render(f, rect, theme, focused),
```

(The `SortDialog` `handle_input`/`render` come from its `Component` impl written in Task 6.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kimun-tui active_dialog_sort_variant_compiles`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/dialogs/mod.rs
git commit -m "feat(tui): register ActiveDialog::Sort variant"
```

---

## Task 8: Sidebar — grouping + live apply + accessors

**Files:**
- Modify: `tui/src/components/sidebar.rs` (`DirListingSource` 31-117; `SidebarComponent` 119-238; key handling 327-334; tests)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `tui/src/components/sidebar.rs`. First add a directory-aware helper and a grouping test:

```rust
    /// Build a sidebar over a vault with both notes and a subdirectory.
    async fn sidebar_with_notes_and_dir(prefix: &str) -> SidebarComponent {
        let vault = temp_vault(prefix).await;
        vault.validate_and_init().await.unwrap();
        // Note that sorts before the directory by name ("a" < "z-dir").
        vault
            .create_note(&VaultPath::note_path_from("alpha"), "body")
            .await
            .unwrap();
        // A note inside a subdirectory creates the directory entry "z-dir".
        vault
            .create_note(&VaultPath::note_path_from("z-dir/inner"), "body")
            .await
            .unwrap();
        let settings = AppSettings::default();
        SidebarComponent::new(
            settings.key_bindings.clone(),
            vault,
            settings.icons(),
            &settings,
        )
    }

    /// Kinds of the visible rows, in display order (excluding the Up row).
    fn row_kinds(sidebar: &SidebarComponent) -> Vec<&'static str> {
        sidebar
            .list
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .filter_map(|e| match e {
                FileListEntry::Note { .. } => Some("note"),
                FileListEntry::Directory { .. } => Some("dir"),
                _ => None,
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn group_dirs_puts_directories_first() {
        let mut sidebar = sidebar_with_notes_and_dir("sidebar-group").await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;

        // Ungrouped (default): name sort interleaves — "alpha" note before "z-dir".
        assert_eq!(row_kinds(&sidebar), vec!["note", "dir"]);

        // Turn grouping on via the same path the dialog drives.
        sidebar.apply_sort(SortField::Name, SortOrder::Ascending, true);
        poll_to_idle(&mut sidebar).await;
        assert_eq!(
            row_kinds(&sidebar),
            vec!["dir", "note"],
            "grouping must cluster directories first"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_sort_updates_shared_state() {
        let mut sidebar = sidebar_with_notes("sidebar-apply", &["alpha", "bravo"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;

        sidebar.apply_sort(SortField::Title, SortOrder::Descending, false);
        poll_to_idle(&mut sidebar).await;
        assert_eq!(sidebar.current_sort(), (SortField::Title, SortOrder::Descending));
        assert!(!sidebar.group_dirs());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kimun-tui group_dirs_puts_directories_first apply_sort_updates_shared_state`
Expected: FAIL — `no method apply_sort` / `current_sort` / `group_dirs`.

- [ ] **Step 3: Add the shared group flag to `DirListingSource` and group in `load`**

In `tui/src/components/sidebar.rs`, add a field to `DirListingSource` (after `sort` at line 37):

```rust
    /// Shared "group directories first" flag, read by `load`.
    group_dirs: Arc<Mutex<bool>>,
```

Replace the sort block in `load` (lines 63-84) — read the group flag and partition when on:

```rust
        // Read the active sort + grouping out of the locks, then drop the
        // guards before the await on the blocking task.
        let (field, order) = *self.sort.lock().unwrap();
        let group_dirs = *self.group_dirs.lock().unwrap();
        let drain = tokio::task::spawn_blocking(move || {
            let mut entries: Vec<FileListEntry> = Vec::new();
            while let Ok(result) = rx.recv() {
                if matches!(result.rtype, ResultType::Directory) && result.path == dir {
                    continue;
                }
                let journal_date = vault.journal_date(&result.path).map(format_journal_date);
                entries.push(FileListEntry::from_result(result, journal_date));
            }
            let cmp = |a: &FileListEntry, b: &FileListEntry| {
                let ka = a.sort_key(field);
                let kb = b.sort_key(field);
                match order {
                    SortOrder::Ascending => ka.cmp(&kb),
                    SortOrder::Descending => kb.cmp(&ka),
                }
            };
            if group_dirs {
                // Directories first, then everything else; each group sorted by
                // field/order. `Up`/`CreateNote` never appear here (Up is pushed
                // separately above; CreateNote is a leading row).
                let (mut dirs, mut rest): (Vec<_>, Vec<_>) = entries
                    .into_iter()
                    .partition(|e| matches!(e, FileListEntry::Directory { .. }));
                dirs.sort_by(&cmp);
                rest.sort_by(&cmp);
                dirs.extend(rest);
                dirs
            } else {
                entries.sort_by(&cmp);
                entries
            }
        });
```

- [ ] **Step 4: Update `SidebarComponent` — fields, accessors, `apply_sort`; remove old combos/methods**

Replace the sort-combo fields in `SidebarComponent` (lines 132-134):
```rust
    /// Combos the engine intercepts: cycle sort field / reverse sort order.
    sort_cycle_combos: Vec<KeyCombo>,
    sort_reverse_combos: Vec<KeyCombo>,
```
with:
```rust
    /// Shared "group directories first" flag. `DirListingSource::load` reads it;
    /// the sort dialog mutates it via `apply_sort`, then the listing reloads.
    group_dirs: Arc<Mutex<bool>>,
```

In `new()` (lines 145-169): delete the two `sort_*_combos` locals (152-153), read the group default from settings, and update the struct literal. Replace lines 145-169 body accordingly:

```rust
        let default_sort_field = SortField::from(settings.default_sort_field);
        let default_sort_order = SortOrder::from(settings.default_sort_order);
        Self {
            current_dir: VaultPath::root(),
            list: None,
            vault,
            icons,
            default_sort_field,
            default_sort_order,
            journal_sort_field: SortField::from(settings.journal_sort_field),
            journal_sort_order: SortOrder::from(settings.journal_sort_order),
            sort: Arc::new(Mutex::new((default_sort_field, default_sort_order))),
            group_dirs: Arc::new(Mutex::new(settings.group_directories)),
            rendered_rect: Rect::default(),
        }
```
(The `let combos = |action| ...;` closure at 145-151 is now unused — delete it. `KeyBindings`/`KeyCombo`/`ActionShortcuts` imports may become unused; remove any the compiler flags.)

In `navigate()` (lines 194-216): pass the group flag into the source and drop the combo interception. Replace the body:

```rust
    pub fn navigate(&mut self, dir: VaultPath, tx: &AppTx) {
        self.current_dir = dir.clone();
        let (sort_field, sort_order) = self.sort_for(&dir);
        self.sort = Arc::new(Mutex::new((sort_field, sort_order)));
        let source = DirListingSource {
            vault: self.vault.clone(),
            dir,
            sort: self.sort.clone(),
            group_dirs: self.group_dirs.clone(),
        };
        self.list = Some(
            SearchList::builder(source, redraw_callback(tx.clone()))
                .filter(Filter::Fuzzy)
                .icons(self.icons.clone())
                .build(),
        );
    }
```
(Note: `group_dirs` is shared across navigations — re-created `sort` per-dir keeps per-dir sort defaults, but grouping is a single global toggle, so we clone the existing `Arc` rather than re-create it.)

Replace `cycle_sort` and `reverse_sort` (lines 218-238) with the accessors + `apply_sort`:

```rust
    /// Current sort field/order for the active listing.
    pub fn current_sort(&self) -> (SortField, SortOrder) {
        *self.sort.lock().unwrap()
    }

    /// Current "group directories first" flag.
    pub fn group_dirs(&self) -> bool {
        *self.group_dirs.lock().unwrap()
    }

    /// Apply a sort selection from the sort dialog and reload so the source
    /// re-orders the listing.
    pub fn apply_sort(&mut self, field: SortField, order: SortOrder, group_dirs: bool) {
        *self.sort.lock().unwrap() = (field, order);
        *self.group_dirs.lock().unwrap() = group_dirs;
        if let Some(list) = &mut self.list {
            list.reload();
        }
    }
```

In `handle_input` (lines 327-334): remove the two `Intercepted` arms for the sort combos. The `match reaction` becomes:

```rust
            match reaction {
                KeyReaction::Submit => {
                    self.activate_selected_entry(tx);
                    EventState::Consumed
                }
                KeyReaction::Consumed | KeyReaction::Cancel => EventState::Consumed,
                KeyReaction::Intercepted(_) | KeyReaction::Unhandled => EventState::NotConsumed,
            }
```

- [ ] **Step 5: Update the existing sidebar tests that drove the removed methods**

The existing tests `reverse_sort_flips_listing_order` (lines 658-678) and `cycle_sort_field_reorders_and_advances_field` (lines 682-698) call the now-removed `reverse_sort()` / `cycle_sort()`. Rewrite them to drive `apply_sort` instead:

```rust
    #[tokio::test(flavor = "multi_thread")]
    async fn apply_sort_reverse_flips_listing_order() {
        let mut sidebar = sidebar_with_notes("sidebar-sort", &["alpha", "bravo", "charlie"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;

        let before = note_names(&sidebar);
        assert_eq!(before.len(), 3, "expected three notes, got {before:?}");

        sidebar.apply_sort(SortField::Name, SortOrder::Descending, false);
        poll_to_idle(&mut sidebar).await;

        let after = note_names(&sidebar);
        assert_eq!(
            after,
            before.iter().rev().cloned().collect::<Vec<_>>(),
            "descending order should reverse the listing"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_sort_changes_field() {
        let mut sidebar = sidebar_with_notes("sidebar-cycle", &["alpha", "bravo"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sidebar, &tx).await;

        sidebar.apply_sort(SortField::Title, SortOrder::Ascending, false);
        poll_to_idle(&mut sidebar).await;

        assert_eq!(sidebar.current_sort().0, SortField::Title);
        assert_eq!(note_names(&sidebar).len(), 2, "notes survive the resort");
    }
```

Delete the two old test fns they replace.

- [ ] **Step 6: Run the sidebar tests**

Run: `cargo test -p kimun-tui --lib components::sidebar`
Expected: PASS (grouping, apply_sort, reverse, field, plus the pre-existing mouse/nav tests).

- [ ] **Step 7: Commit**

```bash
git add tui/src/components/sidebar.rs
git commit -m "feat(tui): sidebar directory grouping + apply_sort via dialog"
```

---

## Task 9: Query panel — apply sort via query rewrite; drop the sort override

**Files:**
- Modify: `tui/src/components/backlinks_panel.rs` (`BacklinkSource` 97-122; `sort_entries` 124-136; `QueryPanel` fields 153-186; `new` 188-253; `cycle_sort`/`reverse_sort` 345-363; key handling 367-416; `hint_shortcuts` 451-466; render title 479-487)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `tui/src/components/backlinks_panel.rs`:

```rust
    #[test]
    fn apply_sort_rewrites_query_order_directive() {
        let vault = crate::test_support::temp_vault_blocking_unavailable();
        // No vault I/O needed: apply_sort only rewrites the query string.
        let mut panel = make_panel(vault);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        panel.set_active_query("widget".to_string());

        panel.apply_sort(SortField::Title, SortOrder::Ascending, &tx);
        assert_eq!(panel.active_query(), "widget or:title");

        // Re-applying replaces, never stacks, the directive.
        panel.apply_sort(SortField::Name, SortOrder::Descending, &tx);
        assert_eq!(panel.active_query(), "widget -or:file");
    }

    #[test]
    fn current_order_reads_query_directive() {
        let vault = crate::test_support::temp_vault_blocking_unavailable();
        let mut panel = make_panel(vault);
        panel.set_active_query("widget -or:title".to_string());
        assert_eq!(
            panel.current_order(),
            (SortField::Title, SortOrder::Descending)
        );
        // No directive → default (Name, Ascending).
        panel.set_active_query("widget".to_string());
        assert_eq!(
            panel.current_order(),
            (SortField::Name, SortOrder::Ascending)
        );
    }
```

> If `temp_vault_blocking_unavailable` does not exist in `test_support`, use the existing async pattern instead: make both tests `#[tokio::test]` and build the vault via `crate::test_support::temp_vault("qp-sort").await; vault.validate_and_init().await.unwrap();`. Do NOT add a new helper.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kimun-tui apply_sort_rewrites_query_order_directive current_order_reads_query_directive`
Expected: FAIL — `no method apply_sort` / `current_order`.

- [ ] **Step 3: Remove the sort override from `BacklinkSource`**

In `tui/src/components/backlinks_panel.rs`:

Delete the `sort` field from `BacklinkSource` (line 100) and the sort lines in `load` (lines 118-119). The `load` body's tail becomes:

```rust
        let q = resolve_query(query, Some(&note));
        let entries = load_query(&self.vault, &q).await;
        emit.replace(entries);
```
(Results are already ordered by core's query engine when the query carries an `or:` directive.)

Delete the `sort_entries` fn entirely (lines 124-136).

- [ ] **Step 4: Update `QueryPanel` fields + `new`**

Remove the `sort` field (line 165) and the three `*_combos` sort fields are partially kept: `follow_link_combos` stays; remove `sort_cycle_combos` and `sort_reverse_combos` (lines 183-184).

In `new` (188-253): delete `let sort = ...` (192), remove `sort` from the `BacklinkSource` literal (208), delete `sort_cycle_combos`/`sort_reverse_combos` locals (217-218), and remove them from the `intercept` extension (222-223) and the struct literal (249-250). Keep `follow_link_combos`. The `intercept` block becomes:

```rust
        let follow_link_combos = combos(&ActionShortcuts::FollowLink);

        let mut intercept = Vec::new();
        intercept.extend(follow_link_combos.iter().cloned());
```

- [ ] **Step 5: Replace `cycle_sort`/`reverse_sort` with `apply_sort` + `current_order`**

Add the import at the top of the file (it already imports `SortField, SortOrder` from `file_list` at line 16; add the core helper):
```rust
use kimun_core::db::search_terms::{with_order_directive, OrderField};
```
> Verify the path: the function is `pub` in `core/src/db/search_terms.rs`. If `db` is not re-exported at `kimun_core::db`, check `core/src/lib.rs` for the actual public path (e.g. `kimun_core::SearchTerms` is re-exported at the crate root per `backlinks_panel.rs:889`). If `with_order_directive`/`OrderField` are not reachable, add `pub use db::search_terms::{with_order_directive, OrderField};` (or extend the existing `SearchTerms` re-export) in `core/src/lib.rs`, then import from `kimun_core::{with_order_directive, OrderField}`.

Replace `cycle_sort` and `reverse_sort` (lines 345-363) with:

```rust
    /// Current sort field/order, derived from the active query's order
    /// directive. Defaults to (Name, Ascending) when the query has none.
    pub fn current_order(&self) -> (SortField, SortOrder) {
        let st = kimun_core::SearchTerms::from_query_string(self.list.query());
        match st.order_by.first() {
            Some(kimun_core::db::search_terms::OrderBy::Title { asc }) => (
                SortField::Title,
                if *asc { SortOrder::Ascending } else { SortOrder::Descending },
            ),
            Some(kimun_core::db::search_terms::OrderBy::FileName { asc }) => (
                SortField::Name,
                if *asc { SortOrder::Ascending } else { SortOrder::Descending },
            ),
            None => (SortField::Name, SortOrder::Ascending),
        }
    }

    /// Apply a sort selection from the sort dialog: rewrite the query's order
    /// directive (the query string is the single source of truth) and reload.
    pub fn apply_sort(&mut self, field: SortField, order: SortOrder, tx: &AppTx) {
        self.ensure_redraw_tx(tx);
        let order_field = match field {
            SortField::Name => OrderField::FileName,
            SortField::Title => OrderField::Title,
        };
        let asc = matches!(order, SortOrder::Ascending);
        let rewritten = with_order_directive(self.list.query(), order_field, asc);
        self.list.set_query(rewritten);
        self.reset_expand();
    }
```
> If `OrderBy` is not reachable at `kimun_core::db::search_terms::OrderBy`, mirror whatever public path the re-export in Step 5's note established (e.g. `kimun_core::OrderBy`).

- [ ] **Step 6: Remove sort intercepts from `handle_key`; fix title + hints**

In `handle_key` (367-416), delete the two `Intercepted` arms for `sort_cycle_combos` and `sort_reverse_combos` (381-388). Keep the `follow_link_combos` arm.

In `hint_shortcuts` (451-466), replace the `(ActionShortcuts::CycleSortField, "sort")` entry (line 457) with:
```rust
            (ActionShortcuts::OpenSortDialog, "sort"),
```

In `render`, the title indicator currently reads `self.sort` (lines 479-480). Replace with `current_order()`:
```rust
        let count = self.list.visible_rows().len();
        let (sort_field, sort_order) = self.current_order();
        let sort_indicator = format!("{}{}", sort_field.label(), sort_order.label());
```

- [ ] **Step 7: Run the query-panel tests**

Run: `cargo test -p kimun-tui --lib components::backlinks_panel`
Expected: PASS (new sort tests + existing navigation/load tests).

- [ ] **Step 8: Commit**

```bash
git add tui/src/components/backlinks_panel.rs core/src/lib.rs
git commit -m "feat(tui): query panel sort via query rewrite; drop sort override"
```

---

## Task 10: Editor — dispatch `OpenSortDialog` + route sort events

**Files:**
- Modify: `tui/src/app_screen/editor.rs` (imports 12-34; action match ~670; app-message match ~935)

- [ ] **Step 1: Write the failing test**

Add a test to the bottom of `tui/src/app_screen/editor.rs` (if the file has no `#[cfg(test)]` block, add one). This test exercises the event-routing path directly (no full TUI):

```rust
#[cfg(test)]
mod sort_routing_tests {
    use super::*;
    use crate::components::events::SortTarget;
    use crate::components::file_list::{SortField, SortOrder};

    async fn make_editor() -> (EditorScreen, AppTx, tokio::sync::mpsc::UnboundedReceiver<AppEvent>) {
        let vault = crate::test_support::temp_vault("editor-sort").await;
        vault.validate_and_init().await.unwrap();
        let settings = std::sync::Arc::new(std::sync::RwLock::new(
            crate::settings::AppSettings::default(),
        ));
        let screen = EditorScreen::new(vault, VaultPath::root(), settings);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (screen, tx, rx)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sort_save_default_persists_to_settings() {
        let (mut screen, tx, _rx) = make_editor().await;
        // Non-journal context → writes the default (non-journal) sort settings.
        screen
            .handle_app_message(
                AppEvent::SortSaveDefault {
                    target: SortTarget::Sidebar,
                    field: SortField::Title,
                    order: SortOrder::Descending,
                    group_directories: true,
                },
                &tx,
            )
            .await;
        let s = screen.settings.read().unwrap();
        assert_eq!(s.default_sort_field, crate::settings::SortFieldSetting::Title);
        assert_eq!(
            s.default_sort_order,
            crate::settings::SortOrderSetting::Descending
        );
        assert!(s.group_directories);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kimun-tui sort_save_default_persists_to_settings`
Expected: FAIL — `no variant SortSaveDefault is matched` → the match falls through; the assertion on `default_sort_field` fails (still `Name`). (It compiles once Task 5 is in; the routing arm is what's missing.)

- [ ] **Step 3: Add the import**

In `tui/src/app_screen/editor.rs`, extend the events import (line 19):
```rust
use crate::components::events::{AppEvent, AppTx, InputEvent, ScreenEvent, SortTarget};
```
Add (near line 19):
```rust
use crate::components::file_list::{SortField, SortOrder};
```

- [ ] **Step 4: Dispatch `OpenSortDialog` in the action match**

In `handle_input`'s action `match action { ... }` (the block of `Some(ActionShortcuts::…)` arms, e.g. right after the `OpenSavedSearches` arm at line 690), add:

```rust
                Some(ActionShortcuts::OpenSortDialog) => {
                    if !self.overlays.is_open() {
                        // Target the focused panel; if neither sort-capable panel
                        // is focused, fall back to the sidebar when it is visible.
                        let target = match self.focus {
                            Focus::Backlinks => Some(SortTarget::Query),
                            Focus::Sidebar => Some(SortTarget::Sidebar),
                            _ if self.sidebar_visible => Some(SortTarget::Sidebar),
                            _ => None,
                        };
                        if let Some(target) = target {
                            let dialog = match target {
                                SortTarget::Sidebar => {
                                    let (f, o) = self.sidebar.current_sort();
                                    ActiveDialog::sort(target, f, o, self.sidebar.group_dirs())
                                }
                                SortTarget::Query => {
                                    let (f, o) = self.backlinks_panel.current_order();
                                    ActiveDialog::sort(target, f, o, false)
                                }
                            };
                            self.overlays.open(Box::new(dialog), self.opener_focus());
                            self.set_focus(Focus::Overlay);
                        }
                    }
                    return EventState::Consumed;
                }
```

- [ ] **Step 5: Route `SortChanged` / `SortSaveDefault` in `handle_app_message`**

In `handle_app_message`'s `match msg { ... }` (after the `CloseOverlay` arm ~line 978), add:

```rust
            AppEvent::SortChanged {
                target,
                field,
                order,
                group_directories,
            } => {
                match target {
                    SortTarget::Sidebar => {
                        self.sidebar.apply_sort(field, order, group_directories)
                    }
                    SortTarget::Query => {
                        self.backlinks_panel.apply_sort(field, order, tx)
                    }
                }
                None
            }
            AppEvent::SortSaveDefault {
                target,
                field,
                order,
                group_directories,
            } => {
                // Apply live first (mirrors the SortChanged path), then persist.
                match target {
                    SortTarget::Sidebar => {
                        self.sidebar.apply_sort(field, order, group_directories);
                        {
                            let mut s = self.settings.write().unwrap();
                            // Context-aware: journal dir writes the journal
                            // defaults, every other dir writes the normal defaults.
                            if self.sidebar.current_dir() == self.vault.journal_path() {
                                s.journal_sort_field = SortField::from(field).into();
                                s.journal_sort_order = SortOrder::from(order).into();
                            } else {
                                s.default_sort_field = SortField::from(field).into();
                                s.default_sort_order = SortOrder::from(order).into();
                            }
                            s.group_directories = group_directories;
                        }
                    }
                    SortTarget::Query => {
                        // The query panel has no persisted default sort (the order
                        // lives in the query string / saved searches); apply only.
                        self.backlinks_panel.apply_sort(field, order, tx);
                    }
                }
                let snapshot = self.settings.read().unwrap().clone();
                tokio::spawn(async move {
                    snapshot.save_to_disk().ok();
                });
                None
            }
```

> `SortField::from(field).into()` is a double conversion: `field` is already a `SortField`, so the inner `SortField::from(field)` is just `field`; write `SortFieldSetting::from(field)` directly instead — clearer:
> ```rust
> s.default_sort_field = crate::settings::SortFieldSetting::from(field);
> s.default_sort_order = crate::settings::SortOrderSetting::from(order);
> ```
> Use the explicit `SortFieldSetting::from(field)` / `SortOrderSetting::from(order)` form in all four assignment lines.

- [ ] **Step 6: Run the routing test + full build**

Run: `cargo test -p kimun-tui sort_save_default_persists_to_settings`
Expected: PASS.

Then a full workspace check:
Run: `cargo build` and `cargo clippy --workspace --all-targets`
Expected: builds clean; fix any "unused import" warnings flagged in sidebar/backlinks (e.g. `KeyCombo`, `ActionShortcuts` if now unused).

- [ ] **Step 7: Commit**

```bash
git add tui/src/app_screen/editor.rs
git commit -m "feat(tui): open sort dialog and route sort events from editor"
```

---

## Task 11: Full verification + docs touch-up

**Files:**
- Verify only; optionally update `docs/` if a keybinding/user doc mentions the old sort shortcuts.

- [ ] **Step 1: Full test + lint pass**

Run: `cargo test --workspace`
Expected: all green.

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo fmt --all`
Expected: no diff (or commit the formatting).

- [ ] **Step 2: Check user docs for stale sort keybindings**

Run: `grep -rniE "cycle sort|reverse sort|sort field|ctrl\+n|ctrl\+r" docs/`
If any user doc lists the old two shortcuts, update it to describe the single "Sort options" dialog (Ctrl+N) with field/order/group-directories. Keep edits to existing docs; do not create new ones.

- [ ] **Step 3: Manual smoke test (optional but recommended)**

Build and run the TUI against the example vault, focus the sidebar, press Ctrl+N, toggle each row (verify the list re-sorts live and directories cluster first when grouping is on), press `s` then reopen to confirm the default persisted, press Enter to close. Repeat with the query panel focused (no group row; query string shows the `or:` directive).

- [ ] **Step 4: Commit any doc/fmt changes**

```bash
git add -A
git commit -m "docs: document the sort options dialog; fmt"
```

---

## Self-Review Notes

- **Spec coverage:** single shortcut (Task 4/3/10), dialog with field/order/group rows (Task 6), Enter+Esc close (Task 6), live apply (Tasks 8/9/10), save-as-default context-aware (Task 10), sidebar grouping dirs-first (Task 8), query panel rewrites `or:` directive and drops the UI override (Tasks 1/9), `group_directories` setting (Task 3), legacy keybinding aliases (Task 4). All covered.
- **Open item carried from the spec:** the query panel has no persisted default sort — `SortSaveDefault` for `SortTarget::Query` only applies live (the dialog also hides the `s` key for the query target, so this path is normally unreachable; it is handled defensively).
- **Type consistency:** `apply_sort(field, order, group)` on sidebar; `apply_sort(field, order, tx)` on query panel (different last arg by design — query needs `tx` to wire redraw; sidebar's redraw is already wired at `navigate`). `current_sort()` (sidebar) vs `current_order()` (query) are intentionally distinct names because the query value is derived from the query string, not stored.
