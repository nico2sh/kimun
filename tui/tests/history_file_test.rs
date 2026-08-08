use kimun_core::nfs::VaultPath;
use kimun_notes::settings::history::HistoryFile;

mod common;
use common::sys;

/// A history file in `dir` for a workspace called `ws`.
fn history_in(dir: &tempfile::TempDir) -> HistoryFile {
    HistoryFile::in_dir(&sys(dir.path()), "ws")
}

#[test]
fn in_dir_names_the_file_after_the_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    assert_eq!(
        history_in(&tmp).path().as_path().file_name().unwrap(),
        "ws.txt"
    );
}

#[test]
fn missing_file_loads_as_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let history = history_in(&tmp);
    assert!(!history.exists());
    assert!(history.load().is_empty());
}

#[test]
fn push_creates_parent_dir_and_writes_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let history = HistoryFile::in_dir(&sys(tmp.path()).join("nested/dir"), "ws");
    let path = VaultPath::new("notes/a.md");

    history.push(&path).unwrap();

    assert!(history.exists());
    assert_eq!(history.load(), vec![path]);
}

#[test]
fn push_dedupes_existing_entry_and_moves_to_front() {
    let tmp = tempfile::TempDir::new().unwrap();
    let history = history_in(&tmp);

    history.push(&VaultPath::new("a.md")).unwrap();
    history.push(&VaultPath::new("b.md")).unwrap();
    history.push(&VaultPath::new("a.md")).unwrap();

    assert_eq!(
        history
            .load()
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>(),
        vec!["a.md".to_string(), "b.md".to_string()]
    );
}

#[test]
fn push_truncates_to_50() {
    let tmp = tempfile::TempDir::new().unwrap();
    let history = history_in(&tmp);
    for i in 0..60 {
        history
            .push(&VaultPath::new(format!("note{i}.md")))
            .unwrap();
    }

    let loaded = history.load();
    assert_eq!(loaded.len(), 50);
    // newest first
    assert_eq!(loaded[0].to_string(), "note59.md");
    assert_eq!(loaded[49].to_string(), "note10.md");
}

#[test]
fn load_skips_blank_and_invalid_lines() {
    let tmp = tempfile::TempDir::new().unwrap();
    let history = history_in(&tmp);
    std::fs::write(history.path().as_path(), "good.md\n\n  \nalso_good.md\n").unwrap();

    assert_eq!(history.load().len(), 2);
}

#[test]
fn atomic_write_leaves_no_tmp_on_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let history = history_in(&tmp);

    history.push(&VaultPath::new("a.md")).unwrap();

    let entries: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entries, ["ws.txt"], "temp file left behind");
}

/// Renaming a workspace moves its history with it — and never over an
/// existing one, which would hand the renamed workspace somebody else's
/// recently-opened notes.
#[test]
fn move_to_relocates_the_history_and_refuses_an_occupied_destination() {
    let tmp = tempfile::TempDir::new().unwrap();
    let from = HistoryFile::in_dir(&sys(tmp.path()), "old");
    let to = HistoryFile::in_dir(&sys(tmp.path()), "new");
    from.push(&VaultPath::new("a.md")).unwrap();

    from.move_to(&to).unwrap();

    assert!(!from.exists());
    assert_eq!(to.load(), vec![VaultPath::new("a.md")]);

    let occupied = HistoryFile::in_dir(&sys(tmp.path()), "occupied");
    occupied.push(&VaultPath::new("b.md")).unwrap();
    assert!(to.move_to(&occupied).is_err());
    assert_eq!(
        occupied.load(),
        vec![VaultPath::new("b.md")],
        "the destination must be left alone"
    );
}

#[test]
fn move_to_of_a_missing_history_is_a_no_op() {
    let tmp = tempfile::TempDir::new().unwrap();
    let from = HistoryFile::in_dir(&sys(tmp.path()), "absent");
    let to = HistoryFile::in_dir(&sys(tmp.path()), "new");

    from.move_to(&to).unwrap();

    assert!(!to.exists());
}

#[test]
fn remove_deletes_it_and_tolerates_a_missing_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let history = history_in(&tmp);
    history.push(&VaultPath::new("a.md")).unwrap();

    history.remove().unwrap();
    assert!(!history.exists());

    // Already gone is the desired state, not an error.
    history.remove().unwrap();
}
