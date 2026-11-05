//! The **Pinned notes dialog**'s rules, shared by both frontends: reading
//! the pins with their existence flags, the reorder and unpin writes with
//! the flash each outcome gives, which pin a digit or Enter opens, and the
//! optimistic swap a reorder makes before its write lands. Core keeps the
//! pins ([`NoteVault::list_pinned_notes`], capped at
//! [`kimun_core::PINNED_NOTES_CAP`]); this module only calls it.

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

use super::message::PinnedRow;

/// Read the pins, in the user's order, each with whether its note is on
/// disk (a pin is kept, and shown as missing, when it is not). `Err` carries
/// the flash text.
pub async fn load(vault: &NoteVault) -> Result<Vec<PinnedRow>, String> {
    let paths = vault
        .list_pinned_notes()
        .await
        .map_err(|e| format!("could not read pinned notes: {e}"))?;
    let mut rows = Vec::with_capacity(paths.len());
    for path in paths {
        let missing = !vault.exists(&path).await;
        rows.push(PinnedRow { path, missing });
    }
    Ok(rows)
}

/// How a reorder or unpin write ended, as the user is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Written; the reload shows it.
    Done,
    /// Nothing written: the list changed underneath (the pin moved away or
    /// is gone). Carries the flash.
    Stale(String),
    /// The write failed. Carries the flash.
    Failed(String),
}

impl Outcome {
    /// What to flash, if anything.
    pub fn flash(&self) -> Option<&str> {
        match self {
            Outcome::Done => None,
            Outcome::Stale(text) | Outcome::Failed(text) => Some(text),
        }
    }
}

/// Move the pin for `path` by `delta` slots (negative raises it). Anchored
/// on the path, never an index, so a list changed underneath moves nothing
/// else.
pub async fn move_by(vault: &NoteVault, path: &VaultPath, delta: isize) -> Outcome {
    match vault.move_pinned_note(path, delta).await {
        Ok(true) => Outcome::Done,
        Ok(false) => Outcome::Stale(format!("pinned notes changed — could not move {path}")),
        Err(e) => Outcome::Failed(format!("could not reorder pinned notes: {e}")),
    }
}

/// Unpin `path`.
pub async fn unpin(vault: &NoteVault, path: &VaultPath) -> Outcome {
    match vault.unpin_note(path).await {
        Ok(true) => Outcome::Done,
        Ok(false) => Outcome::Stale(format!("not pinned: {path}")),
        Err(e) => Outcome::Failed(format!("could not unpin {path}: {e}")),
    }
}

/// The note row `index` (0-based) opens, or the flash when there is no
/// such pin or its note is missing.
pub fn open_target(rows: &[PinnedRow], index: usize) -> Result<VaultPath, String> {
    let Some(row) = rows.get(index) else {
        return Err(format!("no pinned note {}", index + 1));
    };
    if row.missing {
        return Err(format!("pinned note not found: {}", row.path));
    }
    Ok(row.path.clone())
}

/// The rows a reorder of `selected` by `delta` swaps, `(from, to)` — the
/// cursor follows to `to` — or `None` when the move would leave the list
/// (or it is empty).
pub fn swap_selection(len: usize, selected: usize, delta: isize) -> Option<(usize, usize)> {
    if selected >= len {
        return None;
    }
    let to = selected.checked_add_signed(delta)?;
    (to < len).then_some((selected, to))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn row(path: &str, missing: bool) -> PinnedRow {
        PinnedRow {
            path: VaultPath::new(path),
            missing,
        }
    }

    /// A vault where `names` exist and are pinned, in order, and its
    /// directory.
    async fn pinned(names: &[&str]) -> (Arc<NoteVault>, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let vault = NoteVault::new(kimun_core::VaultConfig::new(crate::test_support::sys(
            dir.path(),
        )))
        .await
        .unwrap();
        vault.validate_and_init().await.unwrap();
        for n in names {
            let path = VaultPath::new(n);
            vault.create_note(&path, "hi").await.unwrap();
            vault.toggle_pinned_note(&path).await.unwrap();
        }
        (Arc::new(vault), dir)
    }

    fn paths(rows: &[PinnedRow]) -> Vec<VaultPath> {
        rows.iter().map(|r| r.path.clone()).collect()
    }

    #[tokio::test]
    async fn load_keeps_the_order_and_flags_missing_notes() {
        let (vault, dir) = pinned(&["b.md", "a.md", "c.md"]).await;
        // Gone behind core's back (a delete through core unpins it).
        std::fs::remove_file(dir.path().join("a.md")).unwrap();
        let rows = load(&vault).await.unwrap();
        assert_eq!(
            paths(&rows),
            [
                VaultPath::new("/b.md"),
                VaultPath::new("/a.md"),
                VaultPath::new("/c.md")
            ]
        );
        assert_eq!(
            rows.iter().map(|r| r.missing).collect::<Vec<_>>(),
            [false, true, false]
        );
    }

    #[tokio::test]
    async fn load_of_a_vault_with_no_pins_is_empty() {
        let (vault, _dir) = pinned(&[]).await;
        assert!(load(&vault).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn load_is_capped() {
        let names: Vec<String> = (0..kimun_core::PINNED_NOTES_CAP)
            .map(|i| format!("n{i}.md"))
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (vault, _dir) = pinned(&refs).await;
        let rows = load(&vault).await.unwrap();
        assert_eq!(rows.len(), kimun_core::PINNED_NOTES_CAP);
    }

    #[tokio::test]
    async fn move_by_writes_and_reports() {
        let (vault, _dir) = pinned(&["a.md", "b.md"]).await;
        let a = VaultPath::new("a.md");
        assert_eq!(move_by(&vault, &a, 1).await, Outcome::Done);
        assert_eq!(
            vault.list_pinned_notes().await.unwrap(),
            [VaultPath::new("/b.md"), VaultPath::new("/a.md")]
        );
        // Off the end: nothing written.
        assert_eq!(
            move_by(&vault, &a, 1).await,
            Outcome::Stale("pinned notes changed — could not move a.md".into())
        );
        // Not pinned at all.
        let gone = VaultPath::new("gone.md");
        let outcome = move_by(&vault, &gone, -1).await;
        assert_eq!(
            outcome.flash(),
            Some("pinned notes changed — could not move gone.md")
        );
    }

    #[tokio::test]
    async fn unpin_writes_and_reports() {
        let (vault, _dir) = pinned(&["a.md", "b.md"]).await;
        let a = VaultPath::new("a.md");
        assert_eq!(unpin(&vault, &a).await, Outcome::Done);
        assert_eq!(Outcome::Done.flash(), None);
        assert_eq!(
            vault.list_pinned_notes().await.unwrap(),
            [VaultPath::new("/b.md")]
        );
        assert_eq!(
            unpin(&vault, &a).await,
            Outcome::Stale("not pinned: a.md".into())
        );
    }

    #[test]
    fn open_target_refuses_an_empty_slot_and_a_missing_note() {
        let rows = [row("a.md", false), row("gone.md", true)];
        assert_eq!(open_target(&rows, 0), Ok(VaultPath::new("a.md")));
        assert_eq!(
            open_target(&rows, 1),
            Err("pinned note not found: gone.md".to_string())
        );
        assert_eq!(open_target(&rows, 4), Err("no pinned note 5".to_string()));
        assert_eq!(open_target(&[], 0), Err("no pinned note 1".to_string()));
    }

    #[test]
    fn swap_selection_stays_in_the_list() {
        assert_eq!(swap_selection(3, 0, 1), Some((0, 1)));
        assert_eq!(swap_selection(3, 2, -1), Some((2, 1)));
        assert_eq!(swap_selection(3, 0, -1), None, "first row cannot rise");
        assert_eq!(swap_selection(3, 2, 1), None, "last row cannot sink");
        assert_eq!(swap_selection(0, 0, 1), None, "empty list");
        assert_eq!(swap_selection(0, 0, -1), None);
        assert_eq!(swap_selection(2, 5, -1), None, "cursor past the end");
    }
}
