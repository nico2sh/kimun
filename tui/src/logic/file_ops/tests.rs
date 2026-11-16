use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use tokio::sync::mpsc;

use super::*;
use crate::logic::message::AppMessage;

fn p(s: &str) -> VaultPath {
    VaultPath::new(s)
}

async fn vault(prefix: &str) -> Arc<NoteVault> {
    crate::test_support::temp_vault(prefix).await
}

async fn note(vault: &NoteVault, path: &str, text: &str) {
    vault.create_note(&p(path), text).await.unwrap();
}

async fn dir(vault: &NoteVault, path: &str) {
    vault.create_directory(&p(path)).await.unwrap();
}

async fn attachment(vault: &NoteVault, path: &str) {
    vault.save_attachment(&p(path), b"\x89PNG").await.unwrap();
}

// ---- target builders -------------------------------------------------------

#[test]
fn rename_candidate_keeps_the_directory_and_adds_the_note_extension() {
    assert_eq!(rename_candidate(&p("a/b/old.md"), "New"), p("a/b/new.md"));
    assert_eq!(
        rename_candidate(&p("a/b/old.md"), "new.md"),
        p("a/b/new.md")
    );
    // A directory or an attachment takes the name as typed.
    assert_eq!(rename_candidate(&p("a/sub"), "other"), p("a/other"));
    assert_eq!(rename_candidate(&p("a/img.png"), "pic.jpg"), p("a/pic.jpg"));
}

#[test]
fn rename_target_accepts_one_plain_name() {
    assert_eq!(rename_target(&p("a/old.md"), "new"), Ok(p("a/new.md")));
    assert_eq!(rename_target(&p("old.md"), "new"), Ok(p("new.md")));
    assert_eq!(rename_target(&p("/a/sub"), "other"), Ok(p("/a/other")));
}

#[test]
fn rename_target_refuses_empty_dot_and_multi_component_input() {
    for input in ["", "   "] {
        assert_eq!(
            rename_target(&p("a/old.md"), input),
            Err("Name cannot be empty".to_string()),
            "{input:?}"
        );
    }
    // A multi-component input, spelled the way a vault path renders.
    let nested = p("x/y").to_string();
    for (from, input) in [
        ("a/sub", "."),
        ("a/sub", ".."),
        ("sub", ".."),
        ("a/old.md", nested.as_str()),
        ("a/sub", nested.as_str()),
    ] {
        assert_eq!(
            rename_target(&p(from), input),
            Err("Enter a name, not a path".to_string()),
            "{from} <- {input:?}"
        );
    }
}

#[test]
fn move_candidate_keeps_the_name_under_the_destination() {
    assert_eq!(move_candidate(&p("a/n.md"), &p("b/c")), p("b/c/n.md"));
    assert_eq!(move_candidate(&p("a/n.md"), &VaultPath::root()), p("/n.md"));
    assert_eq!(move_candidate(&p("a/sub"), &p("b")), p("b/sub"));
    assert_eq!(move_candidate(&p("a/img.png"), &p("b")), p("b/img.png"));
}

#[test]
fn move_target_refuses_the_entry_itself_and_its_descendants() {
    let err = Err("Cannot move a directory into itself".to_string());
    assert_eq!(move_target(&p("a"), &p("a")), err);
    assert_eq!(move_target(&p("a"), &p("/A/b")), err);
    assert_eq!(move_target(&p("a"), &p("ab")), Ok(p("ab/a")));
    assert_eq!(move_target(&p("a/n.md"), &p("b")), Ok(p("b/n.md")));
}

#[test]
fn relocated_follows_the_entry_and_what_a_directory_holds() {
    // The entry itself.
    assert_eq!(
        relocated(&p("a/n.md"), &p("a/n.md"), &p("b/m.md")),
        Some(p("b/m.md"))
    );
    // Under a renamed / moved directory, at the same relative place.
    assert_eq!(
        relocated(&p("a/b/c/n.md"), &p("a/b"), &p("x/y")),
        Some(p("x/y/c/n.md"))
    );
    assert_eq!(relocated(&p("a/n.md"), &p("a"), &p("z")), Some(p("z/n.md")));
    // Case and absolute / relative spellings do not matter.
    assert_eq!(
        relocated(&p("/A/n.md"), &p("a"), &p("z")),
        Some(p("z/n.md"))
    );
    // Untouched: a sibling, a look-alike prefix, an ancestor of `from`.
    assert_eq!(relocated(&p("ab/n.md"), &p("a"), &p("z")), None);
    assert_eq!(relocated(&p("a/m.md"), &p("a/n.md"), &p("b.md")), None);
    assert_eq!(relocated(&p("a"), &p("a/b"), &p("z")), None);
}

// ---- availability ------------------------------------------------------------

#[tokio::test]
async fn availability_reports_taken_free_and_invalid() {
    let v = vault("fileops_avail").await;
    note(&v, "a/taken.md", "x").await;
    attachment(&v, "a/img.png").await;
    assert_eq!(
        availability(&v, &p("a/taken.md")).await,
        Availability::Taken
    );
    // Case-insensitive, as every vault path.
    assert_eq!(
        availability(&v, &p("A/Taken.md")).await,
        Availability::Taken
    );
    assert_eq!(availability(&v, &p("a")).await, Availability::Taken);
    assert_eq!(availability(&v, &p("a/img.png")).await, Availability::Taken);
    assert_eq!(availability(&v, &p("a/free.md")).await, Availability::Free);
    assert_eq!(
        availability(&v, &VaultPath::root()).await,
        Availability::Invalid
    );
    assert_eq!(
        availability(&v, &VaultPath::empty()).await,
        Availability::Invalid
    );
}

#[test]
fn availability_maps_onto_the_dialog_state() {
    assert_eq!(
        ValidationState::from(Availability::Free),
        ValidationState::Available
    );
    assert_eq!(
        ValidationState::from(Availability::Taken),
        ValidationState::Taken
    );
    assert_eq!(
        ValidationState::from(Availability::Invalid),
        ValidationState::Taken
    );
}

// ---- delete ------------------------------------------------------------------

#[tokio::test]
async fn delete_removes_a_note_a_directory_and_an_attachment() {
    let v = vault("fileops_delete").await;
    note(&v, "n.md", "x").await;
    note(&v, "d/inner.md", "x").await;
    attachment(&v, "img.png").await;
    for path in ["n.md", "d", "img.png"] {
        let op = delete(&v, &p(path)).await.unwrap();
        assert!(
            matches!(op, FileOp::Deleted(ref d) if *d == p(path)),
            "{path}"
        );
        assert!(!v.exists(&p(path)).await, "{path}");
    }
    assert!(!v.exists(&p("d/inner.md")).await);
}

#[tokio::test]
async fn delete_of_a_missing_entry_is_an_error_message() {
    let v = vault("fileops_delete_missing").await;
    assert!(delete(&v, &p("ghost.md")).await.is_err());
}

// ---- rename ------------------------------------------------------------------

#[tokio::test]
async fn rename_moves_a_note_a_directory_and_an_attachment() {
    let v = vault("fileops_rename").await;
    note(&v, "n.md", "x").await;
    note(&v, "d/inner.md", "y").await;
    attachment(&v, "img.png").await;
    for (from, input) in [("n.md", "m"), ("d", "e"), ("img.png", "pic.png")] {
        let to = rename_target(&p(from), input).unwrap();
        let op = rename(&v, &p(from), &to).await.unwrap();
        assert!(
            matches!(op, FileOp::Renamed { from: ref f, to: ref t } if *f == p(from) && *t == to),
            "{from}"
        );
        assert!(!v.exists(&p(from)).await, "{from}");
        assert!(v.exists(&to).await, "{from}");
    }
    assert!(v.exists(&p("e/inner.md")).await);
}

#[tokio::test]
async fn rename_onto_a_taken_name_is_taken_and_fails() {
    let v = vault("fileops_rename_taken").await;
    note(&v, "a.md", "a").await;
    note(&v, "b.md", "b").await;
    let to = rename_target(&p("a.md"), "b").unwrap();
    assert_eq!(availability(&v, &to).await, Availability::Taken);
    assert!(rename(&v, &p("a.md"), &to).await.is_err());
    assert_eq!(v.get_note_text(&p("b.md")).await.unwrap(), "b");
}

// ---- move --------------------------------------------------------------------

#[tokio::test]
async fn move_entry_moves_a_note_a_directory_and_an_attachment() {
    let v = vault("fileops_move").await;
    dir(&v, "dest").await;
    note(&v, "n.md", "x").await;
    note(&v, "d/inner.md", "y").await;
    attachment(&v, "img.png").await;
    for from in ["n.md", "d", "img.png"] {
        let to = move_target(&p(from), &p("dest")).unwrap();
        assert_eq!(availability(&v, &to).await, Availability::Free, "{from}");
        let op = move_entry(&v, &p(from), &to).await.unwrap();
        assert!(
            matches!(op, FileOp::Moved { from: ref f, to: ref t } if *f == p(from) && *t == to),
            "{from}"
        );
        assert!(!v.exists(&p(from)).await, "{from}");
        assert!(v.exists(&to).await, "{from}");
    }
    assert!(v.exists(&p("dest/d/inner.md")).await);
}

#[tokio::test]
async fn move_into_a_directory_holding_the_name_is_taken() {
    let v = vault("fileops_move_taken").await;
    note(&v, "n.md", "top").await;
    note(&v, "dest/n.md", "inner").await;
    let to = move_target(&p("n.md"), &p("dest")).unwrap();
    assert_eq!(availability(&v, &to).await, Availability::Taken);
}

// ---- directories -------------------------------------------------------------

#[tokio::test]
async fn list_directories_is_the_root_then_every_directory_sorted() {
    let v = vault("fileops_dirs").await;
    note(&v, "b/x.md", "x").await;
    note(&v, "a/deep/y.md", "y").await;
    note(&v, "top.md", "z").await;
    let dirs = list_directories(&v).unwrap();
    let names: Vec<String> = dirs.iter().map(|d| d.to_string()).collect();
    let mut expected: Vec<VaultPath> = vec![VaultPath::root(), p("/a"), p("/a/deep"), p("/b")];
    expected.sort();
    assert_eq!(dirs, expected, "{names:?}");
    assert!(dirs.contains(&VaultPath::root()));
}

#[test]
fn filter_directories_ranks_matches_and_keeps_all_for_an_empty_query() {
    let dirs = vec![
        VaultPath::root(),
        p("/archive"),
        p("/projects"),
        p("/projects/kimun"),
    ];
    assert_eq!(filter_directories(&dirs, ""), dirs);
    let hits = filter_directories(&dirs, "kim");
    assert_eq!(hits, vec![p("/projects/kimun")]);
    let hits = filter_directories(&dirs, "PROJ");
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|h| h.to_string().contains("projects")));
    assert!(filter_directories(&dirs, "zzz").is_empty());
}

// ---- create / quick note / journal -------------------------------------------

#[tokio::test]
async fn create_note_without_content_creates_then_loads() {
    let v = vault("fileops_create").await;
    let out = create_note(&v, &p("fresh.md"), None).await.unwrap();
    assert_eq!(
        out,
        NoteOutcome {
            path: p("fresh.md"),
            created: true
        }
    );
    assert!(v.exists(&p("fresh.md")).await);
    let again = create_note(&v, &p("fresh.md"), None).await.unwrap();
    assert!(!again.created);
    assert_eq!(again.path, p("fresh.md"));
}

#[tokio::test]
async fn create_note_with_content_never_lands_in_a_taken_note() {
    let v = vault("fileops_create_content").await;
    note(&v, "q.md", "original").await;
    let out = create_note(&v, &p("q.md"), Some("answer".to_string()))
        .await
        .unwrap();
    assert!(out.created);
    assert_ne!(out.path, p("q.md"));
    assert_eq!(v.get_note_text(&p("q.md")).await.unwrap(), "original");
    assert_eq!(v.get_note_text(&out.path).await.unwrap(), "answer");
}

#[tokio::test]
async fn quick_note_lands_in_the_inbox_with_the_text() {
    let v = vault("fileops_quick").await;
    let path = quick_note(&v, "remember milk").await.unwrap();
    assert!(v.inbox_path().is_ancestor_of(&path), "{path}");
    assert_eq!(v.get_note_text(&path).await.unwrap(), "remember milk");
}

#[tokio::test]
async fn journal_is_created_once_then_loaded() {
    let v = vault("fileops_journal").await;
    let first = journal(&v).await.unwrap();
    assert!(first.created);
    assert!(v.journal_path().is_ancestor_of(&first.path));
    let second = journal(&v).await.unwrap();
    assert!(!second.created);
    assert_eq!(second.path, first.path);
}

#[tokio::test]
async fn open_journal_announces_only_a_fresh_entry_then_opens_it() {
    let v = vault("fileops_open_journal").await;
    let (tx, mut rx) = mpsc::unbounded_channel::<AppMessage>();

    open_journal(&v, &tx).await;
    let created = rx.try_recv().unwrap();
    let AppMessage::FileOp(FileOp::Created(path)) = created else {
        panic!("expected Created, got {created:?}");
    };
    assert!(
        matches!(rx.try_recv().unwrap(), AppMessage::OpenPath { path: ref o, .. } if *o == path)
    );
    assert!(rx.try_recv().is_err());

    open_journal(&v, &tx).await;
    assert!(
        matches!(rx.try_recv().unwrap(), AppMessage::OpenPath { path: ref o, .. } if *o == path)
    );
    assert!(rx.try_recv().is_err());
}

// ---- new note / new directory ---------------------------------------------

#[test]
fn new_entry_base_prefers_browser_then_note_dir_then_root() {
    assert_eq!(
        new_entry_base(Some(&p("/a/b")), Some(&p("/x/note.md"))),
        p("/a/b")
    );
    assert_eq!(new_entry_base(None, Some(&p("/x/y/note.md"))), p("/x/y"));
    assert_eq!(new_entry_base(None, None), VaultPath::root());
}

#[test]
fn new_entry_base_is_absolute_even_for_relative_paths() {
    let note = VaultPath::new("d/e/note.md");
    assert!(!note.is_absolute(), "precondition");
    assert_eq!(new_entry_base(None, Some(&note)), p("/d/e"));
    assert!(new_entry_base(Some(&VaultPath::new("rel")), None).is_absolute());
}

#[test]
fn new_note_target_resolves_against_the_base_and_explains_refusals() {
    assert_eq!(
        new_note_target(&p("/a"), "Sub/Idea"),
        Ok(p("/a/sub/idea.md"))
    );
    assert!(new_note_target(&p("/a"), "").is_err());
    assert!(new_note_target(&p("/a"), "bad?").unwrap_err().contains('?'));
}

#[test]
fn new_directory_target_resolves_against_the_base() {
    assert_eq!(new_directory_target(&p("/a"), "Sub/"), Ok(p("/a/sub")));
    assert!(new_directory_target(&p("/a"), "x.md").is_err());
}

#[tokio::test]
async fn create_new_note_makes_missing_directories() {
    let vault = vault("new-note-nested").await;
    let path = new_note_target(&VaultPath::root(), "projects/rust/plan").unwrap();
    let outcome = create_new_note(&vault, &path, Some("# Plan".to_string()))
        .await
        .unwrap();
    assert_eq!(
        outcome,
        NoteOutcome {
            path: path.clone(),
            created: true
        }
    );
    assert!(vault.exists(&path).await);
}

#[tokio::test]
async fn create_new_note_refuses_an_existing_note_and_leaves_it_alone() {
    let vault = vault("new-note-exists").await;
    note(&vault, "idea.md", "original").await;
    let err = create_new_note(&vault, &p("idea.md"), Some("other".to_string()))
        .await
        .unwrap_err();
    assert_eq!(err, "A note with that name already exists");
    let text = vault.load_note(&p("idea.md")).await.unwrap().raw_text;
    assert_eq!(text, "original");
}

#[tokio::test]
async fn create_directory_reports_created_and_refuses_a_duplicate() {
    let vault = vault("new-dir").await;
    let path = new_directory_target(&VaultPath::root(), "a/b").unwrap();
    assert!(matches!(
        create_directory(&vault, &path).await,
        Ok(FileOp::Created(created)) if created == path
    ));
    assert!(matches!(
        create_directory(&vault, &path).await,
        Err(e) if e == "A directory with that name already exists"
    ));
}

#[tokio::test]
async fn create_new_note_under_a_file_is_an_error_not_a_panic() {
    let vault = vault("new-note-under-file").await;
    attachment(&vault, "pic.png").await;
    let path = new_note_target(&VaultPath::root(), "pic.png/idea").unwrap();
    let err = create_new_note(&vault, &path, None).await.unwrap_err();
    assert_eq!(
        err,
        "A file with the same name as one of the parent directories already exists"
    );
    assert!(!vault.exists(&path).await);
}

#[tokio::test]
async fn new_entry_kind_resolves_and_creates_each_kind() {
    let vault = vault("new-entry-kind").await;
    let base = VaultPath::root();

    let note_path = NewEntryKind::Note.target(&base, "Ideas/One").unwrap();
    assert_eq!(note_path, p("/ideas/one.md"));
    let made = NewEntryKind::Note
        .create(&vault, &note_path, Some("# One".to_string()))
        .await
        .unwrap();
    assert!(matches!(&made, NewEntry::Note(o) if o.path == note_path && o.created));

    let dir_path = NewEntryKind::Directory.target(&base, "Archive").unwrap();
    assert_eq!(dir_path, p("/archive"));
    // Content never lands in a directory.
    let made = NewEntryKind::Directory
        .create(&vault, &dir_path, Some("ignored".to_string()))
        .await
        .unwrap();
    assert_eq!(made, NewEntry::Directory(dir_path.clone()));
    assert!(vault.exists(&dir_path).await);

    assert!(NewEntryKind::Note.target(&base, "x/").is_err());
    assert_eq!(NewEntryKind::Note.title(), "New note");
    assert_eq!(NewEntryKind::Directory.field_label(), "DIRECTORY NAME");
}

#[tokio::test]
async fn announcing_a_directory_reports_created_without_opening() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    NewEntry::Directory(p("/archive")).announce(&tx);
    assert!(matches!(
        rx.try_recv(),
        Ok(AppMessage::FileOp(FileOp::Created(path))) if path == p("/archive")
    ));
    assert!(rx.try_recv().is_err());
}

#[test]
fn new_note_initial_text_is_relative_to_the_browsed_directory() {
    assert_eq!(
        new_note_initial_text(&p("/a"), &p("/a/sub/idea.md")),
        "sub/idea"
    );
    assert_eq!(
        new_note_initial_text(&VaultPath::root(), &p("/idea.md")),
        "idea"
    );
    // Outside the browsed directory: from the vault root.
    assert_eq!(new_note_initial_text(&p("/a"), &p("/b/idea.md")), "/b/idea");
    // And it resolves back to the note the row offered.
    let base = p("/a");
    let path = p("/a/sub/idea.md");
    let text = new_note_initial_text(&base, &path);
    assert_eq!(new_note_target(&base, &text), Ok(path));
}
