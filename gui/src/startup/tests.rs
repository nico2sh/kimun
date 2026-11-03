//! The startup decision table, the command-line rule and the indexing /
//! restore sequence, headless (no GPUI).
#![cfg(test)]

use std::ffi::OsStr;
use std::time::Duration;

use kimun_core::nfs::VaultPath;
use kimun_core::system::SystemPath;
use kimun_notes::logic::message::AppMessage;
use kimun_notes::logic::startup::VaultOpen;

use super::*;
use crate::test_vault::{TestVault, test_vault};

fn arg(s: &str) -> Option<&OsStr> {
    Some(OsStr::new(s))
}

fn assert_like(got: &VaultPath, want: &str) {
    assert!(got.is_like(&VaultPath::new(want)), "{got} is not {want}");
}

async fn vault() -> TestVault {
    test_vault(&[("projects/todo.md", "todo")], &[]).await
}

// ── decide ───────────────────────────────────────────────────────────────

#[test]
fn no_workspace_starts_on_the_empty_state_whatever_the_argument() {
    assert!(matches!(
        decide(VaultOpen::NoWorkspace, None, None, None),
        Start::NoWorkspace
    ));
    assert!(matches!(
        decide(VaultOpen::NoWorkspace, None, arg("todo"), None),
        Start::NoWorkspace
    ));
}

#[test]
fn a_failed_open_keeps_its_reason() {
    let start = decide(
        VaultOpen::Failed("index locked".into()),
        Some("notes".into()),
        arg("todo"),
        None,
    );
    match start {
        Start::OpenFailed(reason) => assert_eq!(reason, "index locked"),
        _ => panic!("expected the failure empty state"),
    }
}

#[tokio::test]
async fn an_open_vault_restores_without_an_argument() {
    let v = vault().await;
    let Start::Vault(start) = decide(
        VaultOpen::Opened(v.vault.clone()),
        Some("notes".into()),
        None,
        None,
    ) else {
        panic!("expected the vault");
    };
    assert_eq!(start.name, "notes");
    assert_eq!(start.first, FirstNote::Restore);
}

#[tokio::test]
async fn an_open_vault_opens_the_argument_or_refuses_it() {
    let v = vault().await;
    let first = |a: &str| match decide(VaultOpen::Opened(v.vault.clone()), None, arg(a), None) {
        Start::Vault(start) => start.first,
        _ => panic!("expected the vault"),
    };
    match first("projects/todo") {
        FirstNote::Note(path) => assert_like(&path, "projects/todo.md"),
        other => panic!("expected the note, got {other:?}"),
    }
    assert!(matches!(first("bad?name"), FirstNote::Rejected(_)));
}

#[tokio::test]
async fn the_window_is_titled_after_the_workspace() {
    let v = vault().await;
    let opened = |name: Option<&str>| {
        decide(
            VaultOpen::Opened(v.vault.clone()),
            name.map(String::from),
            None,
            None,
        )
    };
    assert_eq!(window_title(&opened(Some("notes"))), "notes - kimün");
    assert_eq!(window_title(&opened(None)), "kimün");
    assert_eq!(window_title(&Start::NoWorkspace), "kimün");
    assert_eq!(window_title(&Start::OpenFailed("x".into())), "kimün");
}

#[tokio::test]
async fn only_a_missing_vault_gets_an_empty_state() {
    let none = empty_state(&Start::NoWorkspace).unwrap();
    assert_eq!(none.title, "No workspace configured");
    assert!(none.detail.contains("kimun"), "{}", none.detail);
    assert!(!none.failed);

    let failed = empty_state(&Start::OpenFailed("/notes: locked".into())).unwrap();
    assert_eq!(failed.detail, "/notes: locked");
    assert!(failed.failed);

    let v = vault().await;
    let opened = decide(VaultOpen::Opened(v.vault.clone()), None, None, None);
    assert_eq!(empty_state(&opened), None);
}

// ── the command-line argument ────────────────────────────────────────────

#[tokio::test]
async fn an_os_path_inside_the_workspace_is_its_note() {
    let v = vault().await;
    let os = v.workspace.join("projects").join("todo.md");
    let path = note_from_arg(os.as_path().as_os_str(), &v.workspace, None).unwrap();
    assert_like(&path, "projects/todo.md");
    // It need not exist yet: opening it says so.
    let os = v.workspace.join("new.md");
    let path = note_from_arg(os.as_path().as_os_str(), &v.workspace, None).unwrap();
    assert_like(&path, "new.md");
}

#[tokio::test]
async fn an_os_path_outside_the_workspace_is_refused() {
    let v = vault().await;
    let elsewhere = tempfile::TempDir::new().unwrap();
    let os = SystemPath::try_absolute(elsewhere.path())
        .unwrap()
        .join("note.md");
    let err = note_from_arg(os.as_path().as_os_str(), &v.workspace, None).unwrap_err();
    assert!(err.contains("not in the workspace"), "{err}");
}

#[tokio::test]
async fn a_directory_or_an_attachment_is_not_a_note() {
    let v = vault().await;
    let dir = v.workspace.join("projects");
    let err = note_from_arg(dir.as_path().as_os_str(), &v.workspace, None).unwrap_err();
    assert!(err.contains("not a note"), "{err}");
    let err = note_from_arg(OsStr::new("pic.png"), &v.workspace, None);
    // A vault path gets the note extension added, so only an OS path can
    // name an attachment.
    assert!(err.is_ok_and(|p| p.is_note()));
    let png = v.workspace.join("pic.png");
    let err = note_from_arg(png.as_path().as_os_str(), &v.workspace, None).unwrap_err();
    assert!(err.contains("not a note"), "{err}");
}

#[tokio::test]
async fn a_vault_relative_path_gets_the_note_extension() {
    let v = vault().await;
    let path = note_from_arg(OsStr::new("projects/todo"), &v.workspace, None).unwrap();
    assert_like(&path, "projects/todo.md");
    let path = note_from_arg(OsStr::new("Projects/Todo.md"), &v.workspace, None).unwrap();
    assert_like(&path, "projects/todo.md");
}

#[tokio::test]
async fn a_relative_path_names_the_file_next_to_the_cwd_when_inside_the_workspace() {
    let v = vault().await;
    let cwd = v.workspace.join("projects");
    let path = note_from_arg(OsStr::new("todo.md"), &v.workspace, Some(&cwd)).unwrap();
    assert_like(&path, "projects/todo.md");
    // Not there: read as a vault path instead.
    let path = note_from_arg(OsStr::new("inbox.md"), &v.workspace, Some(&cwd)).unwrap();
    assert_like(&path, "inbox.md");
}

#[tokio::test]
async fn a_path_a_vault_cannot_hold_is_refused() {
    let v = vault().await;
    let err = note_from_arg(OsStr::new("bad?name"), &v.workspace, None).unwrap_err();
    assert!(err.contains("bad?name"), "{err}");
}

// ── indexing, then the restore ───────────────────────────────────────────

/// Every message `index_and_restore` sends, in order.
async fn run(v: &TestVault, restore: bool) -> Vec<AppMessage> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::time::timeout(
        Duration::from_secs(20),
        index_and_restore(v.vault.clone(), v.settings.clone(), restore, tx),
    )
    .await
    .expect("indexing finishes");
    let mut sent = Vec::new();
    while let Ok(message) = rx.try_recv() {
        sent.push(message);
    }
    sent
}

#[tokio::test]
async fn indexing_then_the_restore_lands_on_the_newest_live_note() {
    let v = test_vault(&[("still.md", "still")], &["gone.md", "still.md"]).await;
    let sent = run(&v, true).await;
    assert!(
        matches!(sent[0], AppMessage::IndexingDone(Ok(_))),
        "{sent:?}"
    );
    match &sent[1] {
        AppMessage::FlashMessage(text) => assert!(text.contains("gone.md"), "{text}"),
        other => panic!("expected the gone flash, got {other:?}"),
    }
    match &sent[2] {
        AppMessage::OpenPath { path, .. } => assert_like(path, "still.md"),
        other => panic!("expected the open, got {other:?}"),
    }
    assert_eq!(sent.len(), 3);
    // The gone note left the history.
    assert_eq!(v.history(), vec![VaultPath::new("still.md")]);
}

#[tokio::test]
async fn with_no_history_the_restore_lands_on_the_root() {
    let v = test_vault(&[], &[]).await;
    let sent = run(&v, true).await;
    match sent.last() {
        Some(AppMessage::OpenPath { path, .. }) => assert_eq!(*path, VaultPath::root()),
        other => panic!("expected the open, got {other:?}"),
    }
}

/// A note named on the command line opens on its own: indexing restores
/// nothing.
#[tokio::test]
async fn without_a_restore_only_indexing_reports() {
    let v = test_vault(&[("a.md", "a")], &["a.md"]).await;
    let sent = run(&v, false).await;
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(matches!(sent[0], AppMessage::IndexingDone(Ok(_))));
}

/// Names differing only by case: reported as a conflict, and the restore
/// does not trust the vault (no pruning, newest entry unchecked). Needs a
/// case-sensitive filesystem to hold both names.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_case_conflict_is_reported_and_the_history_kept() {
    let v = test_vault(&[], &["gone.md"]).await;
    let ws = v.workspace.as_path();
    std::fs::write(ws.join("note.md"), "a").unwrap();
    std::fs::write(ws.join("Note.md"), "b").unwrap();
    let sent = run(&v, true).await;
    match &sent[0] {
        AppMessage::VaultConflict(details) => assert!(details.contains("note.md"), "{details}"),
        other => panic!("expected the conflict, got {other:?}"),
    }
    match &sent[1] {
        AppMessage::OpenPath { path, .. } => assert_like(path, "gone.md"),
        other => panic!("expected the open, got {other:?}"),
    }
    assert_eq!(sent.len(), 2);
    assert_eq!(v.history(), vec![VaultPath::new("gone.md")]);
}
