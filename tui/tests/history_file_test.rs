use kimun_core::nfs::VaultPath;
use kimun_notes::settings::history::{load_history, push_history};

#[test]
fn missing_file_returns_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("does_not_exist.txt");
    assert!(load_history(&path).is_empty());
}

#[test]
fn push_creates_parent_dir_and_writes_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("nested/dir/hist.txt");
    let p = VaultPath::new("notes/a.md");
    push_history(&path, &p).unwrap();

    assert!(path.exists());
    let loaded = load_history(&path);
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0], p);
}

#[test]
fn push_dedupes_existing_entry_and_moves_to_front() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("hist.txt");
    push_history(&path, &VaultPath::new("a.md")).unwrap();
    push_history(&path, &VaultPath::new("b.md")).unwrap();
    push_history(&path, &VaultPath::new("a.md")).unwrap();

    let loaded = load_history(&path);
    assert_eq!(
        loaded.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        vec!["a.md".to_string(), "b.md".to_string()]
    );
}

#[test]
fn push_truncates_to_50() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("hist.txt");
    for i in 0..60 {
        push_history(&path, &VaultPath::new(format!("note{i}.md"))).unwrap();
    }
    let loaded = load_history(&path);
    assert_eq!(loaded.len(), 50);
    // newest first
    assert_eq!(loaded[0].to_string(), "note59.md");
    assert_eq!(loaded[49].to_string(), "note10.md");
}

#[test]
fn load_skips_blank_and_invalid_lines() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("hist.txt");
    std::fs::write(&path, "good.md\n\n  \nalso_good.md\n").unwrap();
    let loaded = load_history(&path);
    assert_eq!(loaded.len(), 2);
}

#[test]
fn atomic_write_leaves_no_tmp_on_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("hist.txt");
    push_history(&path, &VaultPath::new("a.md")).unwrap();
    let entries: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);
    let entry_path = entries[0].as_ref().unwrap().path();
    assert_eq!(entry_path.extension().and_then(|s| s.to_str()), Some("txt"));
}
