// Gated with the inner attribute, not only by `#[cfg(test)] mod tests;` in
// lib.rs: `.github/scripts/check-host-fs.py` exempts a file that carries
// `#![cfg(test)]`, and these TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::*;
use tempfile::TempDir;

async fn setup_vault_with_notes(dir: &std::path::Path) -> NoteVault {
    let vault = NoteVault::new(VaultConfig::new(crate::system::sys(dir)))
        .await
        .unwrap();
    vault.validate_and_init().await.unwrap();
    vault
}

// ---- resolve_note_target ----

/// A vault holding `/dup.md`, `/work/dup.md`, `/a/work/dup.md` and
/// `/journal/today.md` (the note links are followed from).
async fn link_forms_vault(dir: &std::path::Path) -> NoteVault {
    let vault = setup_vault_with_notes(dir).await;
    for path in [
        "/dup.md",
        "/work/dup.md",
        "/a/work/dup.md",
        "/journal/today.md",
    ] {
        vault
            .save_note(&VaultPath::new(path), "body")
            .await
            .unwrap();
    }
    vault
}

/// What `target` resolves to, ambiguous candidates sorted by path.
async fn resolved(vault: &NoteVault, target: note::NoteTarget) -> (String, Vec<VaultPath>) {
    match vault.resolve_note_target(&target).await.unwrap() {
        LinkResolution::Note(entry, _) => ("note".into(), vec![entry.path]),
        LinkResolution::Missing(path) => ("missing".into(), vec![path]),
        LinkResolution::Ambiguous(hits) => {
            let mut paths: Vec<VaultPath> = hits.into_iter().map(|(e, _)| e.path).collect();
            paths.sort();
            ("ambiguous".into(), paths)
        }
    }
}

#[tokio::test]
async fn resolve_note_target_by_name_suffix_and_exact_path() {
    let dir = TempDir::new().unwrap();
    let vault = link_forms_vault(dir.path()).await;
    let today = VaultPath::new("/journal/today.md");
    let wiki = |t: &str| note::NoteTarget::from_wikilink(t, &today).unwrap();
    let md = |t: &str| note::NoteTarget::from_markdown(t, &today).unwrap();
    let p = |s: &str| VaultPath::new(s);
    for (target, kind, paths) in [
        // Name: every note whose path ends with the written components.
        (
            wiki("dup"),
            "ambiguous",
            vec![p("/a/work/dup.md"), p("/dup.md"), p("/work/dup.md")],
        ),
        (
            wiki("Dup#Goals"),
            "ambiguous",
            vec![p("/a/work/dup.md"), p("/dup.md"), p("/work/dup.md")],
        ),
        (
            wiki("work/dup"),
            "ambiguous",
            vec![p("/a/work/dup.md"), p("/work/dup.md")],
        ),
        (
            wiki("WORK/DUP"),
            "ambiguous",
            vec![p("/a/work/dup.md"), p("/work/dup.md")],
        ),
        (wiki("a/work/dup"), "note", vec![p("/a/work/dup.md")]),
        (wiki("today"), "note", vec![p("/journal/today.md")]),
        // Name, missing: the vault root plus the written components.
        (wiki("nobody"), "missing", vec![p("/nobody.md")]),
        (wiki("work/new"), "missing", vec![p("/work/new.md")]),
        (wiki("notwork/dup"), "missing", vec![p("/notwork/dup.md")]),
        // Path: exact.
        (wiki("/work/dup"), "note", vec![p("/work/dup.md")]),
        (wiki("../work/dup"), "note", vec![p("/work/dup.md")]),
        (md("../dup.md"), "note", vec![p("/dup.md")]),
        (md("/a/work/dup.md#s"), "note", vec![p("/a/work/dup.md")]),
        (md("dup.md"), "missing", vec![p("/journal/dup.md")]),
        (
            md("work/dup.md"),
            "missing",
            vec![p("/journal/work/dup.md")],
        ),
        (wiki("./ghost"), "missing", vec![p("/journal/ghost.md")]),
    ] {
        let label = format!("{target:?}");
        assert_eq!(
            resolved(&vault, target).await,
            (kind.to_string(), paths),
            "{label}"
        );
    }
}

#[tokio::test]
async fn wikilink_target_for_is_the_shortest_unique_suffix() {
    let dir = TempDir::new().unwrap();
    let vault = link_forms_vault(dir.path()).await;
    vault
        .save_note(&VaultPath::new("/x/plan.md"), "body")
        .await
        .unwrap();
    for (path, expected) in [
        ("/x/plan.md", "plan"),
        ("/journal/today.md", "today"),
        ("/dup.md", "/dup"),
        ("/a/work/dup.md", "a/work/dup"),
        ("/work/dup.md", "/work/dup"),
        // A note not indexed yet: named against the ones that are.
        ("/b/dup.md", "b/dup"),
    ] {
        assert_eq!(
            vault
                .wikilink_target_for(&VaultPath::new(path))
                .await
                .unwrap(),
            expected,
            "{path}"
        );
    }
}

#[tokio::test]
async fn resolve_note_target_finds_a_note_saved_under_another_case() {
    let dir = TempDir::new().unwrap();
    let vault = setup_vault_with_notes(dir.path()).await;
    vault
        .save_note(&VaultPath::new("/Work/Plan.md"), "body")
        .await
        .unwrap();
    let at = VaultPath::new("/today.md");
    for target in [
        note::NoteTarget::from_wikilink("PLAN", &at).unwrap(),
        note::NoteTarget::from_wikilink("work/plan", &at).unwrap(),
        note::NoteTarget::from_markdown("WORK/plan.md", &at).unwrap(),
    ] {
        assert_eq!(
            resolved(&vault, target).await,
            ("note".to_string(), vec![VaultPath::new("/work/plan.md")])
        );
    }
}
