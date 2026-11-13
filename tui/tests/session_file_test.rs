use kimun_core::nfs::VaultPath;
use kimun_notes::settings::AppSettings;
use kimun_notes::settings::history::HistoryFile;
use kimun_notes::settings::session::{
    AxisRecord, GroupTreeRecord, SESSION_VERSION, SessionFile, SessionRecord, TabRecord,
};

mod common;
use common::sys;

fn session_in(dir: &tempfile::TempDir) -> SessionFile {
    SessionFile::in_dir(&sys(dir.path()), "ws")
}

fn leaf(note: Option<&str>) -> GroupTreeRecord {
    GroupTreeRecord::Leaf {
        note: note.map(VaultPath::new),
    }
}

fn two_tab_record() -> SessionRecord {
    SessionRecord {
        version: SESSION_VERSION,
        tabs: vec![
            TabRecord {
                tree: leaf(Some("notes/a.md")),
                active: 0,
            },
            TabRecord {
                tree: GroupTreeRecord::Split {
                    axis: AxisRecord::Horizontal,
                    weights: vec![1.0, 2.0],
                    children: vec![
                        leaf(Some("b.md")),
                        GroupTreeRecord::Split {
                            axis: AxisRecord::Vertical,
                            weights: vec![1.0, 1.0],
                            children: vec![leaf(None), leaf(Some("deep/c.md"))],
                        },
                    ],
                },
                active: 2,
            },
        ],
        shown: 1,
    }
}

#[test]
fn round_trips_a_two_tab_record() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file = session_in(&tmp);
    let record = two_tab_record();

    file.save(&record).unwrap();

    assert!(file.exists());
    assert_eq!(file.load(), Some(record));
}

#[test]
fn missing_file_loads_none() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file = session_in(&tmp);
    assert!(!file.exists());
    assert_eq!(file.load(), None);
}

#[test]
fn half_written_file_loads_none_without_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file = session_in(&tmp);
    file.save(&two_tab_record()).unwrap();
    let full = std::fs::read(file.path().as_path()).unwrap();
    std::fs::write(file.path().as_path(), &full[..full.len() / 2]).unwrap();

    assert_eq!(file.load(), None);

    std::fs::write(file.path().as_path(), b"").unwrap();
    assert_eq!(file.load(), None);
    std::fs::write(file.path().as_path(), b"not json at all").unwrap();
    assert_eq!(file.load(), None);
}

#[test]
fn session_and_history_files_share_a_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = sys(tmp.path());
    let history = HistoryFile::in_dir(&dir, "ws");
    let session = SessionFile::in_dir(&dir, "ws");

    assert_eq!(
        history.path().as_path().parent(),
        session.path().as_path().parent()
    );
    assert_ne!(history.path(), session.path());
    assert_eq!(SessionFile::beside(&history), session);
}

/// Renaming a workspace moves nothing on disk: its files keep the name they
/// were created under (`file_key`). The session must follow the same rule as
/// the history, so after a rename the new name still finds the saved session.
#[test]
fn workspace_rename_keeps_the_session_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config_dir = tmp.path().canonicalize().unwrap();
    let mut settings = AppSettings::load_from_file(config_dir.join("config.toml")).unwrap();
    let mut wc = kimun_notes::settings::workspace_config::WorkspaceConfig::new_empty();
    wc.add_workspace("work".to_string(), tmp.path().join("vault"))
        .unwrap();
    wc.global.current_workspace = "work".to_string();
    settings.workspace_config = Some(wc);

    let before = settings.session_for("work");
    before.save(&two_tab_record()).unwrap();
    let history_before = settings.history_for("work");

    assert!(
        kimun_notes::logic::preferences::rename_workspace(&mut settings, "work", "job").unwrap()
    );

    let after = settings.session_for("job");
    assert_eq!(after.load(), Some(two_tab_record()));
    assert_eq!(after.path(), before.path());
    assert_eq!(
        SessionFile::beside(&settings.history_for("job")),
        after,
        "the session stays beside the history"
    );
    assert_eq!(settings.history_for("job"), history_before);
}

/// The type itself can also be moved, like `HistoryFile::move_to`.
#[test]
fn move_to_relocates_and_refuses_an_occupied_destination() {
    let tmp = tempfile::TempDir::new().unwrap();
    let from = SessionFile::in_dir(&sys(tmp.path()), "a");
    let to = SessionFile::in_dir(&sys(tmp.path()), "b");
    from.save(&two_tab_record()).unwrap();

    from.move_to(&to).unwrap();
    assert!(!from.exists());
    assert_eq!(to.load(), Some(two_tab_record()));

    from.save(&two_tab_record()).unwrap();
    assert!(from.move_to(&to).is_err());
    assert!(from.exists());
}

#[test]
fn workspace_remove_deletes_the_session_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = sys(tmp.path());
    let index = kimun_core::IndexFile::in_dir(&dir, "gone");
    let history = HistoryFile::in_dir(&dir, "gone");
    let session = SessionFile::in_dir(&dir, "gone");
    history.write(&[VaultPath::new("a.md")]).unwrap();
    session.save(&two_tab_record()).unwrap();

    let leftovers = kimun_notes::settings::delete_artifacts(&index, &history);

    assert!(leftovers.is_empty(), "got {leftovers:?}");
    assert!(!session.exists());
    assert!(!history.exists());
    // A workspace with no session still removes cleanly.
    assert!(kimun_notes::settings::delete_artifacts(&index, &history).is_empty());
}

#[test]
fn out_of_range_shown_or_active_loads_none() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file = session_in(&tmp);

    let mut bad_shown = two_tab_record();
    bad_shown.shown = 2;
    file.save(&bad_shown).unwrap();
    assert_eq!(file.load(), None);

    let mut bad_active = two_tab_record();
    bad_active.tabs[0].active = 1; // a one-group tab
    file.save(&bad_active).unwrap();
    assert_eq!(file.load(), None);

    let mut zero_tabs = two_tab_record();
    zero_tabs.tabs.clear();
    zero_tabs.shown = 0;
    file.save(&zero_tabs).unwrap();
    assert_eq!(file.load(), None);

    let mut empty_split = two_tab_record();
    empty_split.tabs[0].tree = GroupTreeRecord::Split {
        axis: AxisRecord::Vertical,
        weights: vec![],
        children: vec![],
    };
    file.save(&empty_split).unwrap();
    assert_eq!(file.load(), None);

    // The boundary values are fine.
    let mut last = two_tab_record();
    last.tabs[1].active = 2;
    last.shown = 1;
    file.save(&last).unwrap();
    assert!(file.load().is_some());
}

#[test]
fn a_leaf_with_no_note_round_trips_as_an_empty_group() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file = session_in(&tmp);
    let record = SessionRecord {
        version: SESSION_VERSION,
        tabs: vec![TabRecord {
            tree: leaf(None),
            active: 0,
        }],
        shown: 0,
    };
    file.save(&record).unwrap();
    let loaded = file.load().unwrap();
    assert_eq!(loaded, record);
    assert_eq!(loaded.tabs[0].tree, GroupTreeRecord::Leaf { note: None });
}

#[cfg(unix)]
#[test]
fn a_failed_write_leaves_the_previous_file_intact() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::TempDir::new().unwrap();
    let file = session_in(&tmp);
    file.save(&two_tab_record()).unwrap();

    // A read-only directory refuses the temp file the atomic write needs.
    let dir = tmp.path();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let mut other = two_tab_record();
    other.shown = 0;
    let result = file.save(&other);
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    if result.is_ok() {
        // Running with privileges that ignore directory permissions (root).
        return;
    }
    assert_eq!(file.load(), Some(two_tab_record()));
}

/// A split weight that is negative or zero cannot be laid out: the record
/// is unreadable, like any other inconsistent one. (JSON has no NaN or
/// infinity, so a non-finite weight cannot be written; the check refuses
/// one all the same.)
#[test]
fn a_negative_or_zero_weight_loads_none() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file = session_in(&tmp);
    for bad in [-1.0, 0.0] {
        let mut record = two_tab_record();
        let GroupTreeRecord::Split { weights, .. } = &mut record.tabs[1].tree else {
            unreachable!("the second tab is a split");
        };
        weights[0] = bad;
        file.save(&record).unwrap();
        assert_eq!(file.load(), None, "weight {bad} is refused");
    }
    // A nested split is checked too.
    let mut nested = two_tab_record();
    let GroupTreeRecord::Split { children, .. } = &mut nested.tabs[1].tree else {
        unreachable!("the second tab is a split");
    };
    let GroupTreeRecord::Split { weights, .. } = &mut children[1] else {
        unreachable!("its second child is a split");
    };
    weights[1] = 0.0;
    file.save(&nested).unwrap();
    assert_eq!(file.load(), None, "a nested zero weight is refused");
    // Positive weights of any scale are fine.
    let mut scaled = two_tab_record();
    let GroupTreeRecord::Split { weights, .. } = &mut scaled.tabs[1].tree else {
        unreachable!("the second tab is a split");
    };
    *weights = vec![0.001, 1000.0];
    file.save(&scaled).unwrap();
    assert!(file.load().is_some());
}

/// Every record is written with this build's format version, and a file of
/// another version (a newer build's, with fields this one does not know)
/// or with none does not load: the caller keeps such a file untouched.
#[test]
fn a_record_of_another_version_loads_none() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file = session_in(&tmp);
    file.save(&two_tab_record()).unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(file.path().as_path()).unwrap()).unwrap();
    assert_eq!(json["version"], serde_json::json!(SESSION_VERSION));
    assert_eq!(SESSION_VERSION, 1);

    std::fs::write(file.path().as_path(), br#"{"version":2,"future":true}"#).unwrap();
    assert_eq!(file.load(), None);

    // A whole record, only its version newer.
    let mut newer = json.clone();
    newer["version"] = serde_json::json!(SESSION_VERSION + 1);
    std::fs::write(file.path().as_path(), serde_json::to_vec(&newer).unwrap()).unwrap();
    assert_eq!(file.load(), None);

    let mut unversioned = json;
    unversioned.as_object_mut().unwrap().remove("version");
    std::fs::write(
        file.path().as_path(),
        serde_json::to_vec(&unversioned).unwrap(),
    )
    .unwrap();
    assert_eq!(file.load(), None);
}
