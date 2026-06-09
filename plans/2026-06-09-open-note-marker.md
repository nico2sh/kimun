# Open-Note Marker + Live Row Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the editor's FILES drawer, mark the row of the note currently open in the editor, and keep that row's title/name in step as the note is saved or renamed — without reloading the whole listing.

**Architecture:** The sidebar tracks the open note by `VaultPath` (`open_note: Option<VaultPath>`), matched against rows by `is_like`. An `is_open` flag rides on `FileListEntry::Note` and recolors its glyph; the sidebar re-stamps it after every load. A new generic `SearchList::update_rows` seam mutates a single row in place (title on save, path/filename on rename, the marker) without re-querying the directory (see `adr/0010`). Save converges both editor save paths on one `note_saved` helper keyed by the saved path; note rename updates the row in place and, for the open note, retargets the editor to the new path.

**Tech Stack:** Rust, ratatui TUI, `kimun_core` (NoteVault/NoteIndex), tokio.

**Reference docs:** `CONTEXT.md` (Open-note marker, SearchList, Row source), `adr/0010-searchlist-in-place-row-mutation.md`.

**Test commands:** `cargo test -p kimun-notes --bins`, `cargo test -p kimun_core`, `cargo clippy --workspace`. Run the TUI bin tests with `--bins` (the `--lib` target skips app_screen tests).

---

## File Structure

- `tui/src/components/search_list/mod.rs` — add generic `update_rows` seam.
- `tui/src/components/file_list.rs` — add `is_open` to `FileListEntry::Note`, `display_title` helper, glyph recolor.
- Note providers that build `FileListEntry::Note` (must set `is_open: false`): `tui/src/components/note_browser/file_finder_provider.rs`, `link_results_provider.rs`, `search_provider.rs`, and the test source in `note_browser/mod.rs`.
- `tui/src/components/sidebar.rs` — `open_note` field, marker stamping, `update_note_row`, `rename_note_row`, render re-stamp.
- `tui/src/components/events.rs` — add `title` to `AutosaveCompleted`.
- `tui/src/app_screen/editor.rs` — `note_saved` helper, wire `open_path`/save paths, split `EntryRenamed` into `on_note_renamed` with retarget-in-place.

---

## Task 1: `SearchList::update_rows` mutation seam

**Files:**
- Modify: `tui/src/components/search_list/mod.rs` (add method near `reload`, ~line 369; tests in the existing `mod tests`)

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block in `tui/src/components/search_list/mod.rs`:

```rust
    #[tokio::test]
    async fn update_rows_mutates_in_place_and_recomputes() {
        let source = VecSource {
            rows: vec![TestRow::new("alpha"), TestRow::new("beta")],
            reload: false,
        };
        let mut list = SearchList::builder(source, noop_redraw()).build();
        list.poll_until_idle().await;

        // Mutate the row named "alpha".
        let changed = list.update_rows(|r| {
            if r.name == "alpha" {
                r.name = "renamed".to_string();
                true
            } else {
                false
            }
        });
        assert!(changed, "a row was changed");
        assert!(
            list.rows().iter().any(|r| r.name == "renamed"),
            "the mutation is visible in rows()"
        );

        // A no-op mutation reports no change and does not panic.
        let changed_again = list.update_rows(|_| false);
        assert!(!changed_again, "no row changed");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kimun-notes --bins update_rows_mutates_in_place_and_recomputes`
Expected: FAIL — `no method named update_rows found`.

- [ ] **Step 3: Add the `update_rows` method**

Insert immediately after the `reload` method (the one ending `self.loader.start(self.source.clone(), self.query.clone());`) in `tui/src/components/search_list/mod.rs`:

```rust
    /// Mutate rows in place. `mutate` is called for each row and returns `true`
    /// for each row it changed; if any did, the display order is recomputed
    /// (re-filter, no re-sort) so an active filter stays correct. Returns
    /// whether anything changed.
    ///
    /// This is the one seam that touches rows outside the [`RowSource`]; every
    /// other change rebuilds from the source. Structural changes (add/remove/
    /// reorder) must still reload. See `adr/0010`.
    pub fn update_rows(&mut self, mut mutate: impl FnMut(&mut R) -> bool) -> bool {
        let mut changed = false;
        for row in &mut self.rows {
            if mutate(row) {
                changed = true;
            }
        }
        if changed {
            self.recompute_display();
        }
        changed
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p kimun-notes --bins update_rows_mutates_in_place_and_recomputes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tui/src/components/search_list/mod.rs
git commit -m "feat(search_list): add update_rows in-place mutation seam"
```

---

## Task 2: `is_open` flag, `display_title` helper, and glyph recolor on `FileListEntry`

**Files:**
- Modify: `tui/src/components/file_list.rs` (enum, `from_result`, `to_list_item`)
- Modify: `tui/src/components/note_browser/file_finder_provider.rs:60`, `link_results_provider.rs:25` and `:49`, `search_provider.rs:46`, `note_browser/mod.rs:662` (add `is_open: false`)

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block in `tui/src/components/file_list.rs` (create one if absent, mirroring the style of nearby modules):

```rust
#[cfg(test)]
mod open_marker_tests {
    use super::*;

    #[test]
    fn display_title_substitutes_placeholder_for_empty() {
        assert_eq!(FileListEntry::display_title("   ".to_string()), "<no title>");
        assert_eq!(FileListEntry::display_title("Real".to_string()), "Real");
    }

    #[test]
    fn open_note_renders_without_panicking() {
        use crate::settings::icons::Icons;
        use crate::settings::themes::Theme;
        let theme = Theme::default();
        let icons = Icons::default();
        let note = FileListEntry::Note {
            path: kimun_core::nfs::VaultPath::note_path_from("a.md"),
            title: "A".to_string(),
            filename: "a.md".to_string(),
            journal_date: None,
            is_open: true,
        };
        // Accent glyph path must build a ListItem without panicking.
        let _ = note.to_list_item(&theme, &icons);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kimun-notes --bins open_marker_tests`
Expected: FAIL — `display_title` not found and `Note { … }` missing field `is_open`.

- [ ] **Step 3: Add `is_open` to the `Note` variant**

In `tui/src/components/file_list.rs`, change the `Note` variant (lines 104-109):

```rust
    Note {
        path: VaultPath,
        title: String,
        filename: String,
        journal_date: Option<String>,
        /// `true` when this is the note currently open in the editor. Drives the
        /// open-note marker (accent glyph). Stamped by the sidebar after each
        /// load; always `false` from the row source and on non-sidebar surfaces.
        is_open: bool,
    },
```

- [ ] **Step 4: Add `display_title` and set `is_open: false` in `from_result`**

Replace the `ResultType::Note(data)` arm inside `from_result` (lines 128-140) with:

```rust
            ResultType::Note(data) => Self::Note {
                path: result.path,
                title: Self::display_title(data.title),
                filename,
                journal_date,
                is_open: false,
            },
```

Add the helper to the `impl FileListEntry` block (e.g. right after `from_result`):

```rust
    /// Map a raw note title to its display form, substituting a placeholder
    /// for an empty/whitespace title. Shared by listing construction and the
    /// sidebar's live title updates so they never diverge.
    pub fn display_title(raw: String) -> String {
        if raw.trim().is_empty() {
            "<no title>".to_string()
        } else {
            raw
        }
    }
```

- [ ] **Step 5: Recolor the glyph when open**

Replace the `Self::Note { … }` arm in `to_list_item` (lines 198-217) with:

```rust
            Self::Note {
                title,
                filename,
                journal_date,
                is_open,
                ..
            } => {
                let glyph = if journal_date.is_some() {
                    icons.journal
                } else {
                    icons.note
                };
                let mut row = RichRow::new(glyph, title.clone()).filename(filename.clone());
                if *is_open {
                    // Open-note marker: accent the type glyph (see CONTEXT.md).
                    row = row.glyph_style(Style::default().fg(theme.accent.to_ratatui()));
                }
                if let Some(date) = journal_date {
                    row = row.secondary(
                        date.clone(),
                        Some(Style::default().fg(theme.color_journal_date.to_ratatui())),
                    );
                }
                row.into_list_item(theme)
            }
```

- [ ] **Step 6: Add `is_open: false` to the other `Note` constructors**

In each of the following, add `is_open: false,` to the `FileListEntry::Note { … }` literal:
- `tui/src/components/note_browser/file_finder_provider.rs:60`
- `tui/src/components/note_browser/link_results_provider.rs:25`
- `tui/src/components/note_browser/link_results_provider.rs:49`
- `tui/src/components/note_browser/search_provider.rs:46`
- `tui/src/components/note_browser/mod.rs:662` (the `OneNoteSource` test row)

Example shape (apply the same single-line addition to each):

```rust
        FileListEntry::Note {
            path,
            title,
            filename,
            journal_date,
            is_open: false,
        }
```

- [ ] **Step 7: Run the tests + build**

Run: `cargo test -p kimun-notes --bins open_marker_tests`
Expected: PASS.
Run: `cargo build`
Expected: compiles (confirms every `Note` constructor was updated).

- [ ] **Step 8: Commit**

```bash
git add tui/src/components/file_list.rs tui/src/components/note_browser/
git commit -m "feat(file_list): is_open flag, display_title helper, accent glyph for open note"
```

---

## Task 3: Sidebar open-note tracking, marker stamping, and targeted row updates

**Files:**
- Modify: `tui/src/components/sidebar.rs` (struct field + `new`, new methods, `render` re-stamp; tests in `mod tests`)

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `tui/src/components/sidebar.rs` (the helpers `sidebar_with_notes` and `navigate_to_root` already exist):

```rust
    fn note_row_is_open(sb: &SidebarComponent, name: &str) -> bool {
        sb.list
            .as_ref()
            .unwrap()
            .rows()
            .iter()
            .find_map(|r| match r {
                FileListEntry::Note { path, is_open, .. } if path.get_name() == name => {
                    Some(*is_open)
                }
                _ => None,
            })
            .unwrap_or(false)
    }

    fn note_row_title(sb: &SidebarComponent, name: &str) -> Option<String> {
        sb.list.as_ref().unwrap().rows().iter().find_map(|r| match r {
            FileListEntry::Note { path, title, .. } if path.get_name() == name => {
                Some(title.clone())
            }
            _ => None,
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn set_open_note_stamps_matching_row() {
        let mut sb = sidebar_with_notes("sb-open", &["alpha", "beta"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sb, &tx).await;

        sb.set_open_note(Some(VaultPath::note_path_from("alpha")));
        assert!(note_row_is_open(&sb, "alpha.md"), "open note is marked");
        assert!(!note_row_is_open(&sb, "beta.md"), "other note is not marked");

        // Switching the open note moves the marker.
        sb.set_open_note(Some(VaultPath::note_path_from("beta")));
        assert!(!note_row_is_open(&sb, "alpha.md"));
        assert!(note_row_is_open(&sb, "beta.md"));

        // Clearing removes it.
        sb.set_open_note(None);
        assert!(!note_row_is_open(&sb, "beta.md"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_note_row_changes_title_in_place() {
        let mut sb = sidebar_with_notes("sb-title", &["alpha"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sb, &tx).await;

        sb.update_note_row(&VaultPath::note_path_from("alpha"), "Fresh Title");
        assert_eq!(note_row_title(&sb, "alpha.md").as_deref(), Some("Fresh Title"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rename_note_row_updates_path_and_filename() {
        let mut sb = sidebar_with_notes("sb-rename", &["alpha"]).await;
        let (tx, _rx) = unbounded_channel();
        navigate_to_root(&mut sb, &tx).await;

        sb.rename_note_row(
            &VaultPath::note_path_from("alpha"),
            &VaultPath::note_path_from("gamma"),
        );
        assert!(note_row_title(&sb, "gamma.md").is_some(), "row now at new name");
        assert!(note_row_title(&sb, "alpha.md").is_none(), "old name gone");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p kimun-notes --bins -- set_open_note_stamps_matching_row update_note_row_changes_title_in_place rename_note_row_updates_path_and_filename`
Expected: FAIL — `open_note` field/methods not found.

- [ ] **Step 3: Add the `open_note` field**

In `tui/src/components/sidebar.rs`, add to the `SidebarComponent` struct (after `current_dir`, line 132):

```rust
    /// The note currently open in the editor, if any — drives the open-note
    /// marker. `None` on the Browse screen (it never opens notes). Matched
    /// against `FileListEntry::Note` rows by `is_like`.
    open_note: Option<VaultPath>,
```

In `new` (the `Self { … }` literal around line 175), add:

```rust
            open_note: None,
```

- [ ] **Step 4: Add the marker + update methods**

Add to the `impl SidebarComponent` block (e.g. after `refresh_if_showing`):

```rust
    /// Set (or clear) the note the editor currently has open, then re-stamp the
    /// marker on the live rows. The editor calls this on every open and on an
    /// open-note rename.
    pub fn set_open_note(&mut self, path: Option<VaultPath>) {
        self.open_note = path;
        self.stamp_open_marker();
    }

    /// Re-apply `is_open` to the rows so exactly the open note's row is marked.
    /// Idempotent: a full reload rebuilds rows without the flag, so this runs
    /// again after each load (see `render`).
    fn stamp_open_marker(&mut self) {
        let open = self.open_note.clone();
        if let Some(list) = &mut self.list {
            list.update_rows(|row| {
                if let FileListEntry::Note { path, is_open, .. } = row {
                    let want = open.as_ref().is_some_and(|o| path.is_like(o));
                    if *is_open != want {
                        *is_open = want;
                        return true;
                    }
                }
                false
            });
        }
    }

    /// Update the title of the row whose note path matches `path`, if it is in
    /// the current listing. Called when a note is saved and its title (first
    /// body line) may have changed. Position is left unchanged (no re-sort).
    pub fn update_note_row(&mut self, path: &VaultPath, new_title: &str) {
        if let Some(list) = &mut self.list {
            list.update_rows(|row| {
                if let FileListEntry::Note {
                    path: row_path,
                    title,
                    ..
                } = row
                {
                    if row_path.is_like(path) && title != new_title {
                        *title = new_title.to_string();
                        return true;
                    }
                }
                false
            });
        }
    }

    /// Move the row at `from` to `to` (path + filename) in place, for a
    /// same-directory note rename. Position is left unchanged (no re-sort).
    pub fn rename_note_row(&mut self, from: &VaultPath, to: &VaultPath) {
        let new_filename = to.get_parent_path().1;
        if let Some(list) = &mut self.list {
            list.update_rows(|row| {
                if let FileListEntry::Note {
                    path, filename, ..
                } = row
                {
                    if path.is_like(from) {
                        *path = to.clone();
                        *filename = new_filename.clone();
                        return true;
                    }
                }
                false
            });
        }
    }
```

- [ ] **Step 5: Re-stamp the marker on render (after the load lands)**

In `render` (around lines 512-521), replace the `if let Some(list) = &mut self.list { … }` block with:

```rust
        // Poll the engine so a just-completed load's rows are applied, then
        // re-stamp the open-note marker (the reload rebuilt rows without it)
        // before the list renders.
        if let Some(list) = &mut self.list {
            list.poll();
        }
        self.stamp_open_marker();
        if let Some(list) = &mut self.list {
            list.render_query(f, search_inner, theme, focused);
            list.render(f, list_inner, theme, focused);
            // Record the rendered-items rect (the block's inner area) for mouse
            // hit-testing: the engine maps a click to `row - rect.y`, row 0 being
            // the first item. The panel rect makes the wheel scroll from anywhere
            // within the sidebar.
            list.set_list_rect(list_inner);
            list.set_panel_rect(rect);
        }
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p kimun-notes --bins -- set_open_note_stamps_matching_row update_note_row_changes_title_in_place rename_note_row_updates_path_and_filename`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add tui/src/components/sidebar.rs
git commit -m "feat(sidebar): track open note, stamp marker, targeted title/rename row updates"
```

---

## Task 4: Converge both save paths on `note_saved`; wire `open_note` on open

**Files:**
- Modify: `tui/src/components/events.rs` (add `title` to `AutosaveCompleted`, lines 48-51)
- Modify: `tui/src/app_screen/editor.rs` (import, `open_path`, `try_save`, `spawn_autosave`, `AutosaveCompleted` handler, new `note_saved` helper)

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block in `tui/src/app_screen/editor.rs` (mirror the setup of the existing `open_journal_dismisses_open_overlay` test):

```rust
    /// Opening a note marks its sidebar row; saving it (AutosaveCompleted with a
    /// new title) updates that row's title in place.
    #[tokio::test(flavor = "multi_thread")]
    async fn open_then_save_marks_and_retitles_sidebar_row() {
        let vault = crate::test_support::temp_vault("editor-marksave").await;
        vault.validate_and_init().await.unwrap();
        vault
            .create_note(&VaultPath::note_path_from("alpha"), "# Alpha\n\nbody")
            .await
            .unwrap();
        let settings = std::sync::Arc::new(std::sync::RwLock::new(
            crate::settings::AppSettings::default(),
        ));
        let path = VaultPath::note_path_from("alpha");
        let mut screen = EditorScreen::new(vault.clone(), path.clone(), settings);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        // on_enter → open_path loads the note, navigates the sidebar to its dir,
        // and sets the open-note marker.
        screen.on_enter(&tx).await;
        // Drain the sidebar's streamed load.
        for _ in 0..50 {
            screen.panels.sidebar_mut().poll_for_test();
            if !screen.panels.sidebar().is_loading_for_test() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        assert!(
            screen.panels.sidebar().note_row_is_open_for_test("alpha.md"),
            "the open note's row is marked"
        );

        // Simulate a completed autosave that recomputed the title.
        screen
            .handle_app_message(
                AppEvent::AutosaveCompleted {
                    path: path.clone(),
                    saved_revision: None,
                    title: Some("New First Line".to_string()),
                },
                &tx,
            )
            .await;

        assert_eq!(
            screen.panels.sidebar().note_row_title_for_test("alpha.md"),
            Some("New First Line".to_string()),
            "the saved note's row title updated in place"
        );
    }
```

This test needs three small read-only test accessors on `SidebarComponent`. Add them under a `#[cfg(test)]` block in `tui/src/components/sidebar.rs` (outside `mod tests`, so other modules' tests can call them):

```rust
#[cfg(test)]
impl SidebarComponent {
    pub(crate) fn poll_for_test(&mut self) {
        if let Some(list) = &mut self.list {
            list.poll();
        }
        self.stamp_open_marker();
    }

    pub(crate) fn is_loading_for_test(&self) -> bool {
        self.list.as_ref().is_some_and(|l| l.is_loading())
    }

    pub(crate) fn note_row_is_open_for_test(&self, name: &str) -> bool {
        self.list.as_ref().is_some_and(|l| {
            l.rows().iter().any(|r| {
                matches!(r, FileListEntry::Note { path, is_open, .. }
                    if path.get_name() == name && *is_open)
            })
        })
    }

    pub(crate) fn note_row_title_for_test(&self, name: &str) -> Option<String> {
        self.list.as_ref().and_then(|l| {
            l.rows().iter().find_map(|r| match r {
                FileListEntry::Note { path, title, .. } if path.get_name() == name => {
                    Some(title.clone())
                }
                _ => None,
            })
        })
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kimun-notes --bins open_then_save_marks_and_retitles_sidebar_row`
Expected: FAIL — `AutosaveCompleted` has no field `title` (and `open_note` is never set on open).

- [ ] **Step 3: Add `title` to `AutosaveCompleted`**

In `tui/src/components/events.rs`, change the variant (lines 48-51):

```rust
    AutosaveCompleted {
        path: VaultPath,
        saved_revision: Option<NonZeroU64>,
        /// The note's recomputed title (first body line) from the save, so the
        /// sidebar row can be retitled. `None` when the save failed.
        title: Option<String>,
    },
```

- [ ] **Step 4: Add the `FileListEntry` import + `note_saved` helper to the editor**

In `tui/src/app_screen/editor.rs`, add near the other `use crate::components::…` imports:

```rust
use crate::components::file_list::FileListEntry;
```

Add to an `impl EditorScreen` block (next to `refresh_sidebar_if_showing`):

```rust
    /// A note at `path` was just saved with raw title `raw_title`; update its
    /// sidebar row in place. Keyed by the saved path, not the open note, so a
    /// just-saved-then-deselected note's row updates too.
    fn note_saved(&mut self, path: &VaultPath, raw_title: String) {
        let title = FileListEntry::display_title(raw_title);
        self.panels.sidebar_mut().update_note_row(path, &title);
    }
```

- [ ] **Step 5: Set the open-note marker in `open_path`**

In `open_path`, immediately after `self.path = path.clone();` (line 296), add:

```rust
        // Mark this note's row in the sidebar (clears the previous one).
        self.panels
            .sidebar_mut()
            .set_open_note(Some(self.path.clone()));
```

- [ ] **Step 6: Capture the title in `try_save`**

Replace the dirty-branch of `try_save` (lines 394-403) with:

```rust
        if self.panels.editor().is_dirty() {
            let text = self.panels.editor().get_text();
            // Same cap on our own save so quit cannot hang on a stuck
            // disk. A timeout returns Err(_); we skip mark_saved so the
            // editor stays dirty for any subsequent retry.
            let save = self.vault.save_note(&self.path, &text);
            if let Ok(Ok((_, content))) = tokio::time::timeout(SAVE_TIMEOUT, save).await {
                self.panels.editor_mut().mark_saved(text);
                let path = self.path.clone();
                self.note_saved(&path, content.title);
            }
        }
```

- [ ] **Step 7: Carry the title from `spawn_autosave`**

Replace the spawned closure in `spawn_autosave` (lines 428-434) with:

```rust
        self.autosave_task.spawn(async move {
            let (saved_revision, title) = match vault.save_note(&path, &text).await {
                Ok((_, content)) => (Some(revision), Some(content.title)),
                Err(_) => (None, None),
            };
            let _ = tx.send(AppEvent::AutosaveCompleted {
                path,
                saved_revision,
                title,
            });
        });
```

- [ ] **Step 8: Update the `AutosaveCompleted` handler**

Replace the handler (lines 1692-1711) with:

```rust
            AppEvent::AutosaveCompleted {
                path,
                saved_revision,
                title,
            } => {
                if path == self.path
                    && let Some(rev) = saved_revision
                {
                    self.panels.editor_mut().mark_saved_at_revision(rev);
                }
                if let Some(raw_title) = title {
                    self.note_saved(&path, raw_title);
                }
                // The write changed the working tree — refresh the git
                // segment (throttled).
                self.doc_meta.refresh_git(tx);
            }
```

- [ ] **Step 9: Run the test + full bins suite**

Run: `cargo test -p kimun-notes --bins open_then_save_marks_and_retitles_sidebar_row`
Expected: PASS.
Run: `cargo test -p kimun-notes --bins`
Expected: all pass (confirms no other `AutosaveCompleted` construction was missed).

- [ ] **Step 10: Commit**

```bash
git add tui/src/components/events.rs tui/src/app_screen/editor.rs tui/src/components/sidebar.rs
git commit -m "feat(editor): mark open note on open, retitle sidebar row on save"
```

---

## Task 5: Note rename — targeted row update + retarget-in-place for the open note

**Files:**
- Modify: `tui/src/app_screen/editor.rs` (`EntryRenamed` handler ~line 1756; new `on_note_renamed` method)

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` block in `tui/src/app_screen/editor.rs`:

```rust
    /// Renaming the open note keeps it open under the new path (retarget in
    /// place) instead of navigating away, and reloads the buffer clean.
    #[tokio::test(flavor = "multi_thread")]
    async fn renaming_open_note_retargets_in_place() {
        let vault = crate::test_support::temp_vault("editor-rename").await;
        vault.validate_and_init().await.unwrap();
        let from = VaultPath::note_path_from("old");
        vault.create_note(&from, "# Old\n\nbody").await.unwrap();
        let settings = std::sync::Arc::new(std::sync::RwLock::new(
            crate::settings::AppSettings::default(),
        ));
        let mut screen = EditorScreen::new(vault.clone(), from.clone(), settings);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        screen.on_enter(&tx).await;

        // Rename on disk (as the rename dialog would), then deliver the event.
        let to = VaultPath::note_path_from("new");
        vault.rename_note(&from, &to).await.unwrap();
        screen
            .handle_app_message(
                AppEvent::EntryRenamed {
                    from: from.clone(),
                    to: to.clone(),
                },
                &tx,
            )
            .await;

        assert_eq!(screen.path, to, "editor retargets to the new path");
        assert!(
            !screen.panels.editor().is_dirty(),
            "reloaded buffer is clean (won't clobber the renamed file)"
        );
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p kimun-notes --bins renaming_open_note_retargets_in_place`
Expected: FAIL — the current `EntryRenamed` handler routes through `on_entry_op`, which navigates to Browse, so `screen.path` is unchanged (still `old`).

- [ ] **Step 3: Split the `EntryRenamed` handler**

In `tui/src/app_screen/editor.rs`, replace the `EntryRenamed` arm (lines 1756-1758) with:

```rust
            AppEvent::EntryRenamed { from, to } => {
                // Note rename → targeted row update (and retarget the editor if
                // it is the open note). Directory rename keeps the full reload.
                if from.is_note() {
                    self.on_note_renamed(from, to, tx).await;
                } else {
                    self.on_entry_op(from, tx).await;
                }
            }
```

- [ ] **Step 4: Add the `on_note_renamed` method**

Add to the same `impl EditorScreen` block that holds `on_entry_op` (around line 465):

```rust
    /// A note was renamed. Update its sidebar row in place; if it is the note
    /// currently open, retarget the editor to the new path and reload the body
    /// from disk so any self-link rewrites land in the buffer (the in-memory
    /// text still holds the pre-rename self-links). We deliberately do NOT
    /// `try_save` — the old path no longer exists on disk.
    async fn on_note_renamed(&mut self, from: VaultPath, to: VaultPath, tx: &AppTx) {
        self.dismiss_overlay();
        self.panels.sidebar_mut().rename_note_row(&from, &to);
        if from == self.path {
            self.path = to.clone();
            if let Ok(text) = self.vault.get_note_text(&self.path).await {
                self.panels.editor_mut().set_text(text.clone());
                self.panels.editor_mut().mark_saved(text);
            }
            self.panels
                .sidebar_mut()
                .set_open_note(Some(self.path.clone()));
            self.doc_meta.note_opened(&self.path, tx);
        }
    }
```

- [ ] **Step 5: Run the test**

Run: `cargo test -p kimun-notes --bins renaming_open_note_retargets_in_place`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add tui/src/app_screen/editor.rs
git commit -m "feat(editor): retarget-in-place on open-note rename, targeted sidebar row update"
```

---

## Task 6: Full verification

- [ ] **Step 1: Run the whole suite + clippy**

Run: `cargo test -p kimun_core && cargo test -p kimun-notes --bins && cargo clippy --workspace`
Expected: all tests pass, clippy clean.

- [ ] **Step 2: Manual smoke (optional, via the app)**

Open a note in the editor with the FILES drawer showing its directory; confirm the open note's glyph is accent-colored. Edit the first line, wait for autosave; confirm the row title updates without the list reordering. Rename the open note; confirm you stay in the editor and the row shows the new name. Rename a different note in the same dir; confirm its row updates.

- [ ] **Step 3: Final commit (if any fixups)**

```bash
git add -A
git commit -m "chore: open-note marker verification fixups"
```

---

## Self-Review notes

- **Spec coverage:** marker (Task 2 glyph + Task 3 stamp), keep-reference/title-update on save (Task 1 seam + Task 3 `update_note_row` + Task 4 convergence), rename updates list (Task 5 `rename_note_row` + retarget). Browse out of scope (Q1). `adr/0010` + `CONTEXT.md` term written during grilling.
- **Type consistency:** `is_open: bool` on `Note`; `display_title(String) -> String`; `update_rows(impl FnMut(&mut R) -> bool) -> bool`; `set_open_note(Option<VaultPath>)`, `update_note_row(&VaultPath, &str)`, `rename_note_row(&VaultPath, &VaultPath)`; `AutosaveCompleted { path, saved_revision, title }`; `note_saved(&VaultPath, String)`; `on_note_renamed(VaultPath, VaultPath, &AppTx)`. Names match across tasks.
- **Known limitation (documented):** retarget-in-place reloads the buffer from disk, dropping any edits made in the small window between the last autosave and the rename. Accepted given autosave frequency; surfaced in `on_note_renamed`'s doc comment.
- **No re-sort on targeted updates** (per `adr/0010`): a retitled/renamed row keeps its position until the next full reload. Intentional — avoids rows jumping while typing.
