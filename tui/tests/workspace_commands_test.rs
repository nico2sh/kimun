// tui/tests/workspace_commands_test.rs
//
// Integration tests for workspace management CLI commands.
// These tests follow the TDD approach: written before implementation.

use kimun_notes::cli::commands::WorkspaceSubcommand;
use kimun_notes::cli::{CliCommand, run_cli};
use kimun_notes::settings::AppSettings;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Where a workspace's index and history actually live.
///
/// A test has to ask rather than assume: files are named after an opaque key
/// minted at creation, precisely so that nothing — not even a test — derives a
/// filename from the workspace's name.
fn artifacts(config_path: &Path, workspace: &str) -> (PathBuf, PathBuf) {
    let settings =
        AppSettings::load_from_file(config_path.to_path_buf()).expect("settings should load");
    (
        settings.index_for(workspace).path().as_path().to_path_buf(),
        settings
            .history_for(workspace)
            .path()
            .as_path()
            .to_path_buf(),
    )
}

/// A sibling of `index`, e.g. its `-wal`.
fn sidecar(index: &Path, suffix: &str) -> PathBuf {
    let mut name = index.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

// ---------------------------------------------------------------------------
// workspace init tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_init_creates_new_workspace() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();

    // Start with empty config
    std::fs::write(&config_path, "# empty config\n").unwrap();

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("myworkspace".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_ok(),
        "workspace init should succeed: {:?}",
        result
    );

    // Verify the workspace was added to the config
    let settings = AppSettings::load_from_file(config_path).expect("settings should load");
    let ws_config = settings
        .workspace_config
        .as_ref()
        .expect("workspace_config should be set after init");

    assert!(
        ws_config.workspaces.contains_key("myworkspace"),
        "workspace 'myworkspace' should be in config; workspaces: {:?}",
        ws_config.workspaces.keys().collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_workspace_init_first_workspace_defaults_to_default_name() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();

    // Empty config, no workspaces yet
    std::fs::write(&config_path, "# empty config\n").unwrap();

    // Init without a name — should use "default" since no workspaces exist
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: None,
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_ok(),
        "workspace init without name (first workspace) should succeed: {:?}",
        result
    );

    let settings = AppSettings::load_from_file(config_path).unwrap();
    let ws_config = settings.workspace_config.as_ref().unwrap();
    assert!(
        ws_config.workspaces.contains_key("default"),
        "should default to 'default' name for first workspace"
    );
}

#[tokio::test]
async fn test_workspace_init_duplicate_name_fails() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir1 = TempDir::new().unwrap();
    let workspace_dir2 = TempDir::new().unwrap();

    // Add first workspace
    std::fs::write(&config_path, "# empty config\n").unwrap();
    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("myworkspace".to_string()),
                path: workspace_dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("first init should succeed");

    // Try to add another workspace with the same name
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("myworkspace".to_string()),
                path: workspace_dir2.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_err(),
        "workspace init with duplicate name should fail"
    );
}

/// An invalid name must be rejected before it is used to build a path.
///
/// The name becomes `<cache_dir>/<name>.kimuncache`, so a `..` or a separator
/// in it aims that file — and its `-wal`/`-shm` sidecars, and any parent
/// directories — outside the cache directory. Validating only when the entry
/// reaches the config (as `add_workspace` does) is too late: the database has
/// been created and indexed by then, and the command aborts leaving it behind.
///
/// The config lives one level down so an escaping name lands inside the
/// tempdir, where the assertions can see it, rather than in the system temp
/// directory.
#[tokio::test]
async fn test_workspace_init_rejects_names_that_escape_the_cache_dir() {
    for name in ["../escape", "sub/escape", "con"] {
        let config_dir = TempDir::new().unwrap();
        let cache_dir = config_dir.path().join("nested");
        std::fs::create_dir(&cache_dir).unwrap();
        let config_path = cache_dir.join("config.toml");
        std::fs::write(&config_path, "# empty config\n").unwrap();
        let workspace_dir = TempDir::new().unwrap();

        let result = run_cli(
            CliCommand::Workspace {
                subcommand: WorkspaceSubcommand::Init {
                    name: Some(name.to_string()),
                    path: workspace_dir.path().to_path_buf(),
                },
            },
            Some(config_path.clone()),
        )
        .await;

        assert!(result.is_err(), "init with name '{name}' should fail");

        let stray: Vec<_> = std::fs::read_dir(config_dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "nested")
            .collect();
        assert!(
            stray.is_empty(),
            "init with name '{name}' wrote outside the cache dir: {stray:?}"
        );
        let inside: Vec<_> = std::fs::read_dir(&cache_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "config.toml")
            .collect();
        assert!(
            inside.is_empty(),
            "init with name '{name}' left files in the cache dir: {inside:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// workspace list tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_list_empty() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    std::fs::write(&config_path, "# empty config\n").unwrap();

    // list should succeed even when no workspaces configured
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::List,
        },
        Some(config_path),
    )
    .await;

    assert!(
        result.is_ok(),
        "workspace list on empty config should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_workspace_list_shows_workspaces() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    // Add a workspace first
    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("work".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::List,
        },
        Some(config_path),
    )
    .await;

    assert!(
        result.is_ok(),
        "workspace list should succeed: {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// workspace use tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_use_switches_current() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    // Create two workspaces
    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws1".to_string()),
                path: dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init ws1 should succeed");

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws2".to_string()),
                path: dir2.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init ws2 should succeed");

    // Switch to ws2
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Use {
                name: "ws2".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(result.is_ok(), "workspace use should succeed: {:?}", result);

    let settings = AppSettings::load_from_file(config_path).unwrap();
    let ws_config = settings.workspace_config.as_ref().unwrap();
    assert_eq!(
        ws_config.global.current_workspace, "ws2",
        "current workspace should be 'ws2'"
    );
}

#[tokio::test]
async fn test_workspace_use_nonexistent_fails() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let dir1 = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws1".to_string()),
                path: dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Use {
                name: "nonexistent".to_string(),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(
        result.is_err(),
        "workspace use with nonexistent name should fail"
    );
}

// ---------------------------------------------------------------------------
// workspace rename tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_rename_succeeds() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let dir1 = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("oldname".to_string()),
                path: dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Rename {
                old_name: "oldname".to_string(),
                new_name: "newname".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_ok(),
        "workspace rename should succeed: {:?}",
        result
    );

    let settings = AppSettings::load_from_file(config_path).unwrap();
    let ws_config = settings.workspace_config.as_ref().unwrap();
    assert!(
        ws_config.workspaces.contains_key("newname"),
        "renamed workspace should exist under new name"
    );
    assert!(
        !ws_config.workspaces.contains_key("oldname"),
        "old workspace name should no longer exist"
    );
}

/// A rename does not touch the index at all — not the `.kimuncache`, not
/// SQLite's `-wal`/`-shm` siblings.
///
/// The files are named after the entry's `file_key`, which a rename does not
/// change. Renaming used to move all three, which meant moving an open SQLite
/// database: Windows refuses while any handle is on it, and that cost a
/// workspace its whole index on roughly one CI run in three. The move is gone
/// rather than retried.
#[tokio::test]
async fn test_workspace_rename_leaves_the_index_and_sidecars_alone() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("oldname".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let (index, _) = artifacts(&config_path, "oldname");
    // Stand in for an index left open by a crashed process.
    let wal = sidecar(&index, "-wal");
    let shm = sidecar(&index, "-shm");
    std::fs::write(&wal, b"wal").unwrap();
    std::fs::write(&shm, b"shm").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Rename {
                old_name: "oldname".to_string(),
                new_name: "newname".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("rename should succeed");

    assert!(index.exists(), "the index stays exactly where it was");
    assert_eq!(std::fs::read(&wal).unwrap(), b"wal");
    assert_eq!(std::fs::read(&shm).unwrap(), b"shm");

    // Nothing new appeared: the cache directory holds the same three files.
    let cache_dir = config_dir.path().canonicalize().unwrap();
    let mut caches: Vec<_> = std::fs::read_dir(&cache_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("kimuncache"))
        .collect();
    caches.sort();
    let stem = index.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        caches,
        [stem.clone(), format!("{stem}-shm"), format!("{stem}-wal")]
    );

    // And the renamed workspace still resolves to that index — no reindex, no
    // orphan.
    let (after, _) = artifacts(&config_path, "newname");
    assert_eq!(after, index);
}

/// A config written before `file_key` existed keeps its index.
///
/// Those workspaces' files are named after the workspace, and there is no
/// migration to rename them — a migration would have to move an open SQLite
/// database, which is the failure this whole design removes. So the absent
/// field has to keep meaning "my name", including across a rename, which pins
/// it before the name can change.
#[tokio::test]
async fn test_a_config_without_file_keys_keeps_its_index() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    let history_dir = config_dir.path().join("history");
    std::fs::create_dir_all(&history_dir).unwrap();

    // As an older kimün wrote it: no `file_key` anywhere.
    std::fs::write(
        &config_path,
        format!(
            r#"
config_version = 6
cache_dir = "."
history_dir = "history"

[global]
current_workspace = "work"

[workspaces.work]
path = '{}'
created = "2026-01-01T00:00:00Z"
"#,
            workspace_dir.path().display()
        ),
    )
    .unwrap();
    let legacy_index = config_dir
        .path()
        .canonicalize()
        .unwrap()
        .join("work.kimuncache");
    std::fs::write(&legacy_index, b"pre-existing index").unwrap();
    std::fs::write(history_dir.join("work.txt"), "/notes/a.md\n").unwrap();

    let (index, history) = artifacts(&config_path, "work");
    assert_eq!(index, legacy_index, "an absent file_key means the name");
    assert!(history.ends_with("work.txt"));

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Rename {
                old_name: "work".to_string(),
                new_name: "job".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("renaming a legacy workspace should succeed");

    let (after_index, after_history) = artifacts(&config_path, "job");
    assert_eq!(after_index, index, "the rename must not orphan the index");
    assert_eq!(after_history, history);
    assert_eq!(
        std::fs::read(&legacy_index).unwrap(),
        b"pre-existing index",
        "and must not have touched it"
    );
}

/// Renaming twice must not drift the key, or the second rename would point
/// the workspace at a file that was never created.
#[tokio::test]
async fn test_renaming_twice_keeps_the_file_key() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("first".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let (index, history) = artifacts(&config_path, "first");

    for (old, new) in [("first", "second"), ("second", "third")] {
        run_cli(
            CliCommand::Workspace {
                subcommand: WorkspaceSubcommand::Rename {
                    old_name: old.to_string(),
                    new_name: new.to_string(),
                },
            },
            Some(config_path.clone()),
        )
        .await
        .unwrap_or_else(|e| panic!("rename {old} -> {new} should succeed: {e:?}"));
    }

    let (after_index, after_history) = artifacts(&config_path, "third");
    assert_eq!(
        after_index, index,
        "the key must not follow an intermediate name"
    );
    assert_eq!(after_history, history);
    assert!(index.exists(), "and the file it names is still there");
    // No name, at any point in the chain, ever became a filename.
    let cache_dir = config_dir.path().canonicalize().unwrap();
    for name in ["first", "second", "third"] {
        assert!(
            !cache_dir.join(format!("{name}.kimuncache")).exists(),
            "a workspace name must never name a file"
        );
    }
}

/// Leftovers under the new name no longer block a rename.
///
/// They used to: the rename moved both artifacts, so a stale `newname.txt` —
/// left by a `remove` whose best-effort delete failed — aborted the command,
/// and for a while aborted it *after* the index had already moved. Now nothing
/// moves, so a leftover is simply irrelevant, and it must be left untouched
/// rather than adopted.
#[tokio::test]
async fn test_workspace_rename_ignores_leftovers_under_the_new_name() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("oldname".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let (_, history) = artifacts(&config_path, "oldname");
    let history_dir = history.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&history_dir).unwrap();
    std::fs::write(&history, "notes/a.md\n").unwrap();
    // A stranger sitting under the name the workspace is about to take.
    let leftover = history_dir.join("newname.txt");
    std::fs::write(&leftover, "someone else\n").unwrap();

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Rename {
                old_name: "oldname".to_string(),
                new_name: "newname".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_ok(),
        "a leftover under the new name is irrelevant now: {result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&leftover).unwrap(),
        "someone else\n",
        "the leftover must be neither overwritten nor adopted"
    );

    let settings = AppSettings::load_from_file(config_path.clone()).unwrap();
    let ws_config = settings.workspace_config.as_ref().unwrap();
    assert!(ws_config.workspaces.contains_key("newname"));
    assert!(!ws_config.workspaces.contains_key("oldname"));
    // The renamed workspace reads its own history, not the stranger's.
    let (_, after) = artifacts(&config_path, "newname");
    assert_eq!(after, history);
    assert_eq!(settings.history_for("newname").load().len(), 1);
}

/// A renamed workspace keeps its recently-opened notes, and a removed one
/// takes its history with it.
///
/// The history file stays where it was written — like the index, its name
/// comes from the entry's `file_key`, not from the workspace's name — so the
/// rename costs nothing and `remove` still has to find it under that key.
#[tokio::test]
async fn test_workspace_rename_and_remove_carry_the_history_file() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();
    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("oldname".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let (_, history) = artifacts(&config_path, "oldname");
    std::fs::create_dir_all(history.parent().unwrap()).unwrap();
    std::fs::write(&history, "notes/a.md\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Rename {
                old_name: "oldname".to_string(),
                new_name: "newname".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("rename should succeed");

    assert_eq!(
        std::fs::read_to_string(&history).unwrap(),
        "notes/a.md\n",
        "the history stays where it was written"
    );
    assert_eq!(
        AppSettings::load_from_file(config_path.clone())
            .unwrap()
            .history_for("newname")
            .load()
            .len(),
        1,
        "the renamed workspace still reads its history"
    );

    // A second workspace, so the first is not the current one when removed.
    let other_dir = TempDir::new().unwrap();
    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("other".to_string()),
                path: other_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("second init should succeed");
    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Use {
                name: "other".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("use should succeed");

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Remove {
                name: "newname".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("remove should succeed");

    assert!(
        !history.exists(),
        "a removed workspace must not leave its history behind, whatever it is called"
    );
}

// ---------------------------------------------------------------------------
// workspace remove tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_remove_succeeds() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    // Create two workspaces (ws1 is current, ws2 is the extra)
    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws1".to_string()),
                path: dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init ws1 should succeed");

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws2".to_string()),
                path: dir2.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init ws2 should succeed");

    // Remove ws2 (not current)
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Remove {
                name: "ws2".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(
        result.is_ok(),
        "workspace remove should succeed: {:?}",
        result
    );

    let settings = AppSettings::load_from_file(config_path).unwrap();
    let ws_config = settings.workspace_config.as_ref().unwrap();
    assert!(
        !ws_config.workspaces.contains_key("ws2"),
        "removed workspace should no longer exist"
    );
}

/// Removing a workspace has to take the whole index with it, sidecars
/// included — otherwise the cache directory accumulates `-wal`/`-shm` files
/// belonging to workspaces that no longer exist.
#[tokio::test]
async fn test_workspace_remove_deletes_the_index_sidecars() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let keep_dir = TempDir::new().unwrap();
    let doomed_dir = TempDir::new().unwrap();
    std::fs::write(&config_path, "# empty config\n").unwrap();

    for (name, dir) in [("keep", &keep_dir), ("doomed", &doomed_dir)] {
        run_cli(
            CliCommand::Workspace {
                subcommand: WorkspaceSubcommand::Init {
                    name: Some(name.to_string()),
                    path: dir.path().to_path_buf(),
                },
            },
            Some(config_path.clone()),
        )
        .await
        .unwrap_or_else(|e| panic!("init of '{name}' should succeed: {e:?}"));
    }

    let (doomed_index, _) = artifacts(&config_path, "doomed");
    let (kept_index, _) = artifacts(&config_path, "keep");
    let wal = sidecar(&doomed_index, "-wal");
    let shm = sidecar(&doomed_index, "-shm");
    std::fs::write(&wal, b"wal").unwrap();
    std::fs::write(&shm, b"shm").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Remove {
                name: "doomed".to_string(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("remove should succeed");

    assert!(!doomed_index.exists());
    assert!(!wal.exists(), "orphaned WAL left behind");
    assert!(!shm.exists(), "orphaned shm left behind");
    assert!(
        kept_index.exists(),
        "the other workspace's index must be untouched"
    );
}

#[tokio::test]
async fn test_workspace_remove_current_fails() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let dir1 = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("ws1".to_string()),
                path: dir1.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    // Try to remove the current workspace
    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Remove {
                name: "ws1".to_string(),
            },
        },
        Some(config_path),
    )
    .await;

    assert!(
        result.is_err(),
        "removing the current workspace should fail with helpful error"
    );
}

// ---------------------------------------------------------------------------
// workspace reindex tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_workspace_reindex_succeeds() {
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let workspace_dir = TempDir::new().unwrap();

    std::fs::write(&config_path, "# empty config\n").unwrap();

    run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Init {
                name: Some("myws".to_string()),
                path: workspace_dir.path().to_path_buf(),
            },
        },
        Some(config_path.clone()),
    )
    .await
    .expect("init should succeed");

    let result = run_cli(
        CliCommand::Workspace {
            subcommand: WorkspaceSubcommand::Reindex {
                name: None, // use current workspace
            },
        },
        Some(config_path),
    )
    .await;

    assert!(
        result.is_ok(),
        "workspace reindex should succeed: {:?}",
        result
    );
}
