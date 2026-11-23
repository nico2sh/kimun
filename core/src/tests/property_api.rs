// Gated with the inner attribute, not only by `#[cfg(test)] mod tests;` in
// lib.rs: `.github/scripts/check-host-fs.py` exempts a file that carries
// `#![cfg(test)]`, and these TempDir fixtures call `std::fs` directly.
#![cfg(test)]

use crate::nfs::VaultPath;
use crate::note::{FrontmatterFormat, PropertyInput, PropertyKind, PropertyValue};
use crate::*;

async fn new_vault() -> (tempfile::TempDir, NoteVault) {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = VaultConfig::new(crate::system::sys(tmp.path()));
    let vault = NoteVault::new(cfg).await.unwrap();
    vault.validate_and_init().await.unwrap();
    (tmp, vault)
}

fn p(s: &str) -> VaultPath {
    VaultPath::note_path_from(s)
}

fn text(s: &str) -> PropertyValue {
    PropertyValue::Text(s.into())
}

/// Paths of the notes `query` finds, sorted.
async fn hits(vault: &NoteVault, query: &str) -> Vec<String> {
    let mut paths: Vec<String> = vault
        .search_notes(query)
        .await
        .unwrap()
        .into_iter()
        .map(|(e, _)| e.path.to_string())
        .collect();
    paths.sort();
    paths
}

fn input(values: &[&str]) -> PropertyInput {
    PropertyInput::new(values.iter().map(|v| v.to_string()).collect())
}

#[tokio::test]
async fn property_keys_lists_distinct_search_form_keys_sorted() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(
            &p("/a.md"),
            "+++\nStatus = \"x\"\ndue = 2024-01-01\n+++\nbody",
        )
        .await
        .unwrap();
    vault
        .create_note(&p("/b.md"), "---\nstatus: y\nRésumé: z\nempty:\n---\nbody")
        .await
        .unwrap();
    assert_eq!(
        vault.property_keys().await.unwrap(),
        vec!["due", "empty", "resume", "status"]
    );
}

#[tokio::test]
async fn input_is_inferred_for_a_new_key() {
    let (_tmp, vault) = new_vault().await;
    vault.create_note(&p("/n.md"), "body").await.unwrap();
    let set = |key: &'static str, values: PropertyInput| {
        let vault = &vault;
        async move {
            vault
                .set_property_from_input(&p("/n.md"), key, &values, FrontmatterFormat::Toml)
                .await
                .unwrap()
        }
    };
    assert_eq!(set("n", input(&["5"])).await, PropertyValue::Number(5.0));
    assert_eq!(set("zip", input(&["02134"])).await, text("02134"));
    assert_eq!(
        set("tags", input(&["a", "b"])).await,
        PropertyValue::List(vec!["a".into(), "b".into()])
    );
    assert_eq!(
        vault.get_property(&p("/n.md"), "zip").await.unwrap(),
        Some(Some(text("02134"))),
        "the typed text is stored unchanged"
    );
}

#[tokio::test]
async fn input_must_fit_the_vault_type_unless_typed_explicitly() {
    let (_tmp, vault) = new_vault().await;
    for (path, body) in [
        ("/a.md", "+++\npriority = 1\ntags = [\"x\"]\n+++\n"),
        ("/b.md", "+++\npriority = 2\n+++\n"),
        ("/c.md", "body"),
    ] {
        vault.create_note(&p(path), body).await.unwrap();
    }
    let set = |values: &[&str], kind: Option<PropertyKind>| {
        let values = input(values).forced(kind);
        let vault = &vault;
        async move {
            vault
                .set_property_from_input(&p("/c.md"), "priority", &values, FrontmatterFormat::Toml)
                .await
        }
    };
    let err = set(&["high"], None).await.unwrap_err();
    assert!(matches!(err, VaultError::InvalidProperty { .. }), "{err:?}");
    assert!(
        err.user_message().unwrap().contains("holds number values"),
        "{err:?}"
    );
    assert_eq!(
        vault.get_note_text(&p("/c.md")).await.unwrap(),
        "body",
        "refused, untouched"
    );

    assert_eq!(set(&["3"], None).await.unwrap(), PropertyValue::Number(3.0));
    assert_eq!(
        set(&["high"], Some(PropertyKind::Text)).await.unwrap(),
        text("high")
    );
    assert_eq!(
        vault.get_property(&p("/a.md"), "priority").await.unwrap(),
        Some(Some(PropertyValue::Number(1.0))),
        "an explicit type changes only the note written"
    );

    let tags = vault
        .set_property_from_input(
            &p("/c.md"),
            "tags",
            &input(&["garden"]),
            FrontmatterFormat::Toml,
        )
        .await
        .unwrap();
    assert_eq!(tags, PropertyValue::List(vec!["garden".into()]));
}

#[tokio::test]
async fn a_notes_own_value_does_not_lock_its_type() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&p("/n.md"), "+++\nstatus = 1\n+++\n")
        .await
        .unwrap();
    let value = vault
        .set_property_from_input(
            &p("/n.md"),
            "status",
            &input(&["open"]),
            FrontmatterFormat::Toml,
        )
        .await
        .unwrap();
    assert_eq!(value, text("open"));
}

#[tokio::test]
async fn set_then_get_round_trips_and_creates_toml_by_default() {
    let (_tmp, vault) = new_vault().await;
    vault.create_note(&p("/n.md"), "# T\nbody").await.unwrap();
    vault
        .set_property(
            &p("/n.md"),
            "Status",
            text("Done"),
            FrontmatterFormat::default(),
        )
        .await
        .unwrap();
    vault
        .set_property(
            &p("/n.md"),
            "n",
            PropertyValue::Number(2.0),
            FrontmatterFormat::Yaml,
        )
        .await
        .unwrap();

    let raw = vault.get_note_text(&p("/n.md")).await.unwrap();
    assert!(
        raw.starts_with("+++\nStatus = \"Done\"\nn = 2\n+++\n# T"),
        "{raw}"
    );
    assert_eq!(
        vault.get_properties(&p("/n.md")).await.unwrap(),
        vec![
            ("Status".to_string(), Some(text("Done"))),
            ("n".to_string(), Some(PropertyValue::Number(2.0)))
        ]
    );
    assert_eq!(
        vault.get_property(&p("/n.md"), "STATUS").await.unwrap(),
        Some(Some(text("Done")))
    );
    assert_eq!(vault.get_property(&p("/n.md"), "nope").await.unwrap(), None);
    // The index saw the write.
    let hits = vault.search_notes("prop:status=done").await.unwrap();
    assert_eq!(hits.len(), 1);
}

#[tokio::test]
async fn set_creates_yaml_block_on_request() {
    let (_tmp, vault) = new_vault().await;
    vault.create_note(&p("/n.md"), "body").await.unwrap();
    vault
        .set_property(&p("/n.md"), "a", text("x"), FrontmatterFormat::Yaml)
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/n.md")).await.unwrap(),
        "---\na: x\n---\nbody"
    );
}

#[tokio::test]
async fn existing_block_format_is_preserved() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&p("/y.md"), "---\na: 1\n---\nbody")
        .await
        .unwrap();
    vault
        .set_property(&p("/y.md"), "b", text("x"), FrontmatterFormat::Toml)
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/y.md")).await.unwrap(),
        "---\na: 1\nb: x\n---\nbody"
    );

    vault
        .create_note(&p("/t.md"), "+++\na = 1\n+++\nbody")
        .await
        .unwrap();
    vault
        .set_property(&p("/t.md"), "b", text("x"), FrontmatterFormat::Yaml)
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/t.md")).await.unwrap(),
        "+++\na = 1\nb = \"x\"\n+++\nbody"
    );
}

#[tokio::test]
async fn remove_property_only_touches_its_key() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&p("/n.md"), "+++\na = 1\nb = 2\n+++\nbody")
        .await
        .unwrap();
    assert!(vault.remove_property(&p("/n.md"), "A").await.unwrap());
    assert!(!vault.remove_property(&p("/n.md"), "a").await.unwrap());
    assert_eq!(
        vault.get_properties(&p("/n.md")).await.unwrap(),
        vec![("b".to_string(), Some(PropertyValue::Number(2.0)))]
    );
    assert!(vault.search_notes("prop:a=1").await.unwrap().is_empty());
}

#[tokio::test]
async fn a_refused_edit_of_a_sound_block_is_an_invalid_property() {
    let (_tmp, vault) = new_vault().await;
    let original = "+++\nmeta = { x = 1 }\n+++\nbody";
    vault.create_note(&p("/n.md"), original).await.unwrap();
    for err in [
        vault
            .set_property(&p("/n.md"), "meta", text("x"), FrontmatterFormat::Toml)
            .await
            .unwrap_err(),
        vault
            .set_property(&p("/n.md"), "d", text("a\n+++\nb"), FrontmatterFormat::Toml)
            .await
            .unwrap_err(),
    ] {
        assert!(matches!(err, VaultError::InvalidProperty { .. }), "{err:?}");
    }
    assert_eq!(vault.get_note_text(&p("/n.md")).await.unwrap(), original);
}

// A lone `\r` (an old-Mac line break) is a line break to YAML; edits must
// see it as one too, or a neighbouring entry on the same "line" is lost.
#[tokio::test]
async fn a_lone_carriage_return_does_not_cost_a_neighbouring_entry() {
    let (tmp, vault) = new_vault().await;
    tokio::fs::write(
        tmp.path().join("n.md"),
        "---\ntitle: Hello\nstatus: draft\rdue: 2024-01-01\n---\nbody\n",
    )
    .await
    .unwrap();
    vault
        .set_property(&p("/n.md"), "status", text("done"), FrontmatterFormat::Toml)
        .await
        .unwrap();
    let keys: Vec<String> = vault
        .get_properties(&p("/n.md"))
        .await
        .unwrap()
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(keys, ["title", "status", "due"]);
}

#[tokio::test]
async fn unparseable_frontmatter_is_refused_on_write() {
    let (_tmp, vault) = new_vault().await;
    let original = "+++\nnot = = toml\n+++\nbody";
    vault.create_note(&p("/n.md"), original).await.unwrap();
    let err = vault
        .set_property(&p("/n.md"), "a", text("x"), FrontmatterFormat::Toml)
        .await
        .unwrap_err();
    assert!(
        matches!(err, VaultError::FSError(FSError::InvalidFrontmatter { .. })),
        "{err:?}"
    );
    let err = vault.remove_property(&p("/n.md"), "not").await.unwrap_err();
    assert!(matches!(
        err,
        VaultError::FSError(FSError::InvalidFrontmatter { .. })
    ));
    assert_eq!(
        vault.get_note_text(&p("/n.md")).await.unwrap(),
        original,
        "file untouched"
    );
    // Reads stay lenient.
    assert!(vault.get_properties(&p("/n.md")).await.unwrap().is_empty());
}

#[tokio::test]
async fn invalid_key_and_value_are_rejected() {
    let (_tmp, vault) = new_vault().await;
    vault.create_note(&p("/n.md"), "body").await.unwrap();
    let err = vault
        .set_property(&p("/n.md"), "  ", text("x"), FrontmatterFormat::Toml)
        .await
        .unwrap_err();
    assert!(matches!(err, VaultError::InvalidProperty { .. }));
    let err = vault
        .set_property(
            &p("/n.md"),
            "n",
            PropertyValue::Number(f64::NAN),
            FrontmatterFormat::Toml,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, VaultError::InvalidProperty { .. }));
}

#[tokio::test]
async fn missing_note_is_not_found() {
    let (_tmp, vault) = new_vault().await;
    assert!(vault
        .get_properties(&p("/nope.md"))
        .await
        .unwrap_err()
        .is_not_found());
    assert!(vault
        .get_property(&p("/nope.md"), "a")
        .await
        .unwrap_err()
        .is_not_found());
    assert!(vault
        .set_property(&p("/nope.md"), "a", text("x"), FrontmatterFormat::Toml)
        .await
        .unwrap_err()
        .is_not_found());
}

#[tokio::test]
async fn get_tags_unions_inline_and_frontmatter() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(
            &p("/n.md"),
            "---\ntags: [Fm, shared]\n---\nbody #shared #inline",
        )
        .await
        .unwrap();
    assert_eq!(
        vault.get_tags(&p("/n.md")).await.unwrap(),
        vec!["fm", "inline", "shared"]
    );
}

/// A CRLF note keeps its CRLF endings and its single frontmatter block.
#[tokio::test]
async fn crlf_note_keeps_endings_and_block() {
    let (tmp, vault) = new_vault().await;
    // Authored on Windows, outside the vault (same seeding as `crlf_index_tests`).
    tokio::fs::write(tmp.path().join("n.md"), "---\r\na: 1\r\n---\r\nbody\r\n")
        .await
        .unwrap();
    vault
        .set_property(&p("/n.md"), "b", text("x"), FrontmatterFormat::Toml)
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/n.md")).await.unwrap(),
        "---\r\na: 1\r\nb: x\r\n---\r\nbody\r\n"
    );
}

/// Keys are matched case-insensitively and spelled as written: in the
/// file, in what reads return, and still found by any casing.
#[tokio::test]
async fn key_casing_is_kept_but_never_matters() {
    let (_tmp, vault) = new_vault().await;
    vault.create_note(&p("/n.md"), "body").await.unwrap();
    vault
        .set_property(&p("/n.md"), " dueDate ", text("x"), FrontmatterFormat::Yaml)
        .await
        .unwrap();
    vault
        .set_property(
            &p("/n.md"),
            "Aliases",
            PropertyValue::List(vec!["a".into()]),
            FrontmatterFormat::Toml,
        )
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/n.md")).await.unwrap(),
        "---\ndueDate: x\nAliases:\n  - a\n---\nbody"
    );
    // Another casing edits the same entry, keeping its spelling.
    vault
        .set_property(&p("/n.md"), "DUEDATE", text("y"), FrontmatterFormat::Yaml)
        .await
        .unwrap();
    assert_eq!(
        vault.get_properties(&p("/n.md")).await.unwrap(),
        vec![
            ("dueDate".to_string(), Some(text("y"))),
            (
                "Aliases".to_string(),
                Some(PropertyValue::List(vec!["a".into()]))
            )
        ]
    );
    assert_eq!(
        vault.get_property(&p("/n.md"), "duedate").await.unwrap(),
        Some(Some(text("y")))
    );
    // The index matches by any casing.
    for q in ["%duedate=y", "prop:DUEDATE=Y", "%dueDate", "%ALIASES=a"] {
        assert_eq!(vault.search_notes(q).await.unwrap().len(), 1, "{q}");
    }
    assert_eq!(
        vault
            .search_notes("%duedate or:prop:DueDate")
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(vault.remove_property(&p("/n.md"), "DUEdate").await.unwrap());
    assert_eq!(
        vault.get_note_text(&p("/n.md")).await.unwrap(),
        "---\nAliases:\n  - a\n---\nbody"
    );
}

/// `Status`/`status` and `Done`/`done` are the same in every query form.
#[tokio::test]
async fn property_searches_ignore_case_in_keys_and_values() {
    let (_tmp, vault) = new_vault().await;
    let notes = [
        (
            "/a.md",
            "---\nStatus: Done\nOwner: Ánna\nlabels: [Red, Blue]\nflag: True\n---\n",
        ),
        (
            "/b.md",
            "+++\nSTATUS = \"In Progress\"\nowner = \"bob\"\nlabels = [\"GREEN\"]\n+++\n",
        ),
        ("/c.md", "+++\nstatus = \"done\"\n+++\n"),
    ];
    for (path, body) in notes {
        vault.create_note(&p(path), body).await.unwrap();
    }
    let done = vec!["/a.md".to_string(), "/c.md".to_string()];
    for q in [
        "prop:status=done",
        "prop:STATUS=DONE",
        "%Status=Done",
        "%sTaTuS=dOnE",
        "%status='DONE'",
        "-%status!=done",
    ] {
        assert_eq!(hits(&vault, q).await, done, "{q}");
    }
    assert_eq!(
        hits(&vault, "%status=\"IN progress\"").await,
        ["/b.md"],
        "quoted"
    );
    assert_eq!(hits(&vault, "%status!=DONE").await, ["/b.md"], "!=");
    assert_eq!(
        hits(&vault, "-%status=DONE %status").await,
        ["/b.md"],
        "exclusion"
    );
    assert_eq!(hits(&vault, "%labels=RED").await, ["/a.md"], "list item");
    assert_eq!(hits(&vault, "%labels!=red").await, ["/b.md"], "list !=");
    assert_eq!(hits(&vault, "%owner=ÁNNA").await, ["/a.md"], "non-ASCII");
    assert_eq!(hits(&vault, "%owner=BOB").await, ["/b.md"], "TOML text");
    assert_eq!(hits(&vault, "%flag=TRUE").await, ["/a.md"], "bool");
    assert_eq!(
        hits(&vault, "%status>=DONE %status<=Done").await,
        done,
        "ordering"
    );
    // Sorting ignores case: "done" < "done" < "in progress".
    let sorted: Vec<String> = vault
        .search_notes("%STATUS or:prop:Status")
        .await
        .unwrap()
        .into_iter()
        .map(|(e, _)| e.path.to_string())
        .collect();
    assert_eq!(sorted.last().unwrap(), "/b.md", "{sorted:?}");
}

/// Multibyte text (accents, CJK, emoji, non-Latin cased scripts) in keys,
/// values and comments: matched without regard to case, edited without
/// slicing a character, and untouched where it isn't the target.
#[tokio::test]
async fn multibyte_keys_values_and_comments() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(
            &p("/a.md"),
            "---\nÉtat: Terminé # 完了 ✅\n名前: 田中\nпроект: Ёлка\nstatus: \"日本語 # 😀\" # ok\n---\nbody",
        )
        .await
        .unwrap();
    vault
        .create_note(
            &p("/b.md"),
            "+++\n\"ÉTAT\" = \"TERMINÉ\" # ünï\n\"емоджи\" = \"😀\"\n+++\n",
        )
        .await
        .unwrap();
    let both = ["/a.md", "/b.md"];
    assert_eq!(hits(&vault, "%état=terminé").await, both);
    assert_eq!(hits(&vault, "%ÉTAT=TERMINÉ").await, both);
    assert_eq!(hits(&vault, "%名前=田中").await, ["/a.md"]);
    assert_eq!(hits(&vault, "%ПРОЕКТ=ёлка").await, ["/a.md"]);
    assert_eq!(hits(&vault, "%емоджи=😀").await, ["/b.md"]);
    assert_eq!(hits(&vault, "%status=\"日本語 # 😀\"").await, ["/a.md"]);

    // Edits next to multibyte text keep every character, and the
    // trailing comment of the replaced entry.
    vault
        .set_property(&p("/a.md"), "état", text("Fini ✔"), FrontmatterFormat::Yaml)
        .await
        .unwrap();
    vault
        .set_property(&p("/b.md"), "état", text("Ça va"), FrontmatterFormat::Toml)
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/a.md")).await.unwrap(),
        "---\nÉtat: Fini ✔ # 完了 ✅\n名前: 田中\nпроект: Ёлка\nstatus: \"日本語 # 😀\" # ok\n---\nbody"
    );
    assert_eq!(
        vault.get_note_text(&p("/b.md")).await.unwrap(),
        "+++\n\"ÉTAT\" = \"Ça va\" # ünï\n\"емоджи\" = \"😀\"\n+++\n"
    );
    assert_eq!(
        hits(&vault, "%état=FINI ✔").await.len(),
        0,
        "space needs quotes"
    );
    assert_eq!(hits(&vault, "%état=\"fini ✔\"").await, ["/a.md"]);
    assert!(vault.remove_property(&p("/a.md"), "НАЗВАНИЕ").await.is_ok());
    assert!(vault.remove_property(&p("/a.md"), "ПРОЕКТ").await.unwrap());
}

/// Searching ignores accents too (`e` finds `é`, also in decomposed form),
/// but editing keeps `résumé` and `resume` as two properties.
#[tokio::test]
async fn property_searches_ignore_accents() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(
            &p("/a.md"),
            "---\nÉtat: Terminé\nlabels: [Café, naïve]\nrésumé: x\n---\n",
        )
        .await
        .unwrap();
    for q in [
        "%etat=termine",
        "%ETAT=TERMINE",
        "%état=terminé",
        "%e\u{301}tat=terminé",
        "%état=termine\u{301}",
        "%labels=cafe",
        "%labels=NAIVE",
        "%labels=cafe\u{301}",
        "%resume=x",
        "%étât>=termine %état<=termine",
    ] {
        assert_eq!(hits(&vault, q).await.len(), 1, "{q}");
    }
    assert!(hits(&vault, "%etat=terminer").await.is_empty());
    vault
        .set_property(&p("/a.md"), "resume", text("y"), FrontmatterFormat::Yaml)
        .await
        .unwrap();
    let raw = vault.get_note_text(&p("/a.md")).await.unwrap();
    assert!(
        raw.contains("résumé: x") && raw.contains("resume: y"),
        "{raw}"
    );
}

/// Keys that fold together in the index (`résumé`, `resume`) stay separate
/// in the note; the index keeps the first and never mixes their values.
/// A `tâgs` key is not a tags key.
#[tokio::test]
async fn keys_that_fold_together_do_not_mix_in_the_index() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(
            &p("/a.md"),
            "---\nrésumé: [one]\nresume: [x, y, z]\ntâgs: [nope]\n---\n",
        )
        .await
        .unwrap();
    assert_eq!(vault.get_properties(&p("/a.md")).await.unwrap().len(), 3);
    assert_eq!(hits(&vault, "%resume=one").await, ["/a.md"]);
    assert!(hits(&vault, "%resume=y").await.is_empty());
    assert!(vault.get_tags(&p("/a.md")).await.unwrap().is_empty());
    let value = vault
        .set_property_from_input(
            &p("/a.md"),
            "tâgs",
            &input(&["solo"]),
            FrontmatterFormat::Yaml,
        )
        .await
        .unwrap();
    assert_eq!(value, text("solo"), "only `tags` / `tag` are forced lists");
}

/// A key spelled differently in two notes is still one vault-wide key.
#[tokio::test]
async fn vault_type_check_ignores_key_casing() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&p("/a.md"), "+++\nPriority = 1\n+++\n")
        .await
        .unwrap();
    vault
        .create_note(&p("/b.md"), "+++\npriority = 2\n+++\n")
        .await
        .unwrap();
    vault.create_note(&p("/c.md"), "body").await.unwrap();
    let err = vault
        .set_property_from_input(
            &p("/c.md"),
            "PRIORITY",
            &input(&["high"]),
            FrontmatterFormat::Toml,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, VaultError::InvalidProperty { .. }), "{err:?}");
}

/// A key with no value (Obsidian's unset `due:`) is listed, as search's
/// `%due` finds it, and gets as present without a value.
#[tokio::test]
async fn a_key_without_a_value_is_listed_with_none() {
    let (tmp, vault) = new_vault().await;
    tokio::fs::write(tmp.path().join("n.md"), "---\ndue:\nstatus: ok\n---\nbody")
        .await
        .unwrap();
    assert_eq!(
        vault.get_properties(&p("/n.md")).await.unwrap(),
        vec![
            ("due".to_string(), None),
            ("status".to_string(), Some(text("ok")))
        ]
    );
    assert_eq!(
        vault.get_property(&p("/n.md"), "due").await.unwrap(),
        Some(None),
        "present, without a value"
    );
}

/// A byte-order mark (Windows editors) must not hide frontmatter, nor end
/// up after a block written above it.
#[tokio::test]
async fn bom_note_keeps_its_block_and_its_bom() {
    let (tmp, vault) = new_vault().await;
    tokio::fs::write(tmp.path().join("y.md"), "\u{feff}---\ntitle: x\n---\nbody")
        .await
        .unwrap();
    tokio::fs::write(tmp.path().join("plain.md"), "\u{feff}body")
        .await
        .unwrap();
    assert_eq!(
        vault.get_properties(&p("/y.md")).await.unwrap(),
        vec![("title".to_string(), Some(text("x")))]
    );
    vault
        .set_property(&p("/y.md"), "a", text("1"), FrontmatterFormat::Toml)
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/y.md")).await.unwrap(),
        "\u{feff}---\ntitle: x\na: \"1\"\n---\nbody"
    );
    vault
        .set_property(&p("/plain.md"), "a", text("1"), FrontmatterFormat::Toml)
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/plain.md")).await.unwrap(),
        "\u{feff}+++\na = \"1\"\n+++\nbody"
    );
}

#[tokio::test]
async fn concurrent_sets_on_one_note_both_land() {
    let (_tmp, vault) = new_vault().await;
    let vault = std::sync::Arc::new(vault);
    vault.create_note(&p("/n.md"), "body").await.unwrap();
    let tasks: Vec<_> = (0..8)
        .map(|i| {
            let v = vault.clone();
            tokio::spawn(async move {
                v.set_property(
                    &p("/n.md"),
                    &format!("k{i}"),
                    PropertyValue::Number(i as f64),
                    FrontmatterFormat::Toml,
                )
                .await
            })
        })
        .collect();
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    assert_eq!(vault.get_properties(&p("/n.md")).await.unwrap().len(), 8);
}

#[tokio::test]
async fn full_reindex_populates_properties() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&p("/n.md"), "+++\nstatus = \"open\"\n+++\n")
        .await
        .unwrap();
    vault.recreate_index().await.unwrap();
    assert_eq!(vault.search_notes("%status=open").await.unwrap().len(), 1);
}

#[tokio::test]
async fn rename_keeps_position_value_and_comments_toml() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(
            &p("/t.md"),
            "+++\na = 1\n# about b\nb = \"x\" # keep\nc = 3\n+++\nbody",
        )
        .await
        .unwrap();
    vault
        .rename_property(&p("/t.md"), "B", "beta")
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/t.md")).await.unwrap(),
        "+++\na = 1\n# about b\nbeta = \"x\" # keep\nc = 3\n+++\nbody"
    );
}

#[tokio::test]
async fn rename_keeps_position_value_and_comments_yaml() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(
            &p("/y.md"),
            "---\na: 1\ntags: # keep\n  - x\n  - y\nc: 3\nbare:\n---\nbody",
        )
        .await
        .unwrap();
    vault
        .rename_property(&p("/y.md"), "tags", "labels")
        .await
        .unwrap();
    vault
        .rename_property(&p("/y.md"), "bare", "due date")
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/y.md")).await.unwrap(),
        "---\na: 1\nlabels: # keep\n  - x\n  - y\nc: 3\ndue date:\n---\nbody"
    );
}

#[tokio::test]
async fn rename_case_only_respells_in_place() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&p("/n.md"), "---\nStatus: x\nb: 2\n---\nbody")
        .await
        .unwrap();
    vault
        .rename_property(&p("/n.md"), "Status", "status")
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/n.md")).await.unwrap(),
        "---\nstatus: x\nb: 2\n---\nbody"
    );
}

#[tokio::test]
async fn rename_refuses_existing_target_and_missing_source() {
    let (_tmp, vault) = new_vault().await;
    let original = "+++\na = 1\nb = 2\n+++\nbody";
    vault.create_note(&p("/n.md"), original).await.unwrap();
    let taken = vault
        .rename_property(&p("/n.md"), "a", "B")
        .await
        .unwrap_err();
    assert!(
        matches!(taken, VaultError::InvalidProperty { .. }),
        "{taken:?}"
    );
    let missing = vault
        .rename_property(&p("/n.md"), "zzz", "q")
        .await
        .unwrap_err();
    assert!(
        matches!(missing, VaultError::InvalidProperty { .. }),
        "{missing:?}"
    );
    assert_eq!(vault.get_note_text(&p("/n.md")).await.unwrap(), original);
}

#[tokio::test]
async fn rename_updates_the_index() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&p("/n.md"), "+++\nstatus = \"done\"\n+++\nbody")
        .await
        .unwrap();
    vault
        .rename_property(&p("/n.md"), "status", "state")
        .await
        .unwrap();
    assert!(vault
        .search_notes("prop:status=done")
        .await
        .unwrap()
        .is_empty());
    assert_eq!(hits(&vault, "prop:state=done").await, vec!["/n.md"]);
}

#[tokio::test]
async fn rename_onto_a_toml_table_name_is_refused() {
    for (i, (block, new)) in [
        ("a = 1\n\n[meta]\nx = 1\n", "meta"),
        ("a = 1\n\n[meta]\nx = 1\n", "Meta"),
        ("meta = {x = 1}\na = 1\n", "meta"),
        ("a = 1\nmeta.x = 1\n", "meta"),
        ("a = 1\n\n[[meta]]\nx = 1\n", "meta"),
    ]
    .into_iter()
    .enumerate()
    {
        let (_tmp, vault) = new_vault().await;
        let original = format!("+++\n{block}+++\nbody");
        let path = p(&format!("/t{i}.md"));
        vault.create_note(&path, &original).await.unwrap();
        let err = vault.rename_property(&path, "a", new).await.unwrap_err();
        assert!(
            matches!(err, VaultError::InvalidProperty { .. }),
            "{block:?}: {err:?}"
        );
        assert_eq!(
            vault.get_note_text(&path).await.unwrap(),
            original,
            "{block:?}"
        );
    }
}

#[tokio::test]
async fn rename_onto_a_yaml_mapping_key_is_refused() {
    let (_tmp, vault) = new_vault().await;
    let original = "---\na: 1\nmeta:\n  x: 1\n---\nbody";
    vault.create_note(&p("/y.md"), original).await.unwrap();
    let err = vault
        .rename_property(&p("/y.md"), "a", "meta")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, VaultError::InvalidProperty { message, .. } if message.contains("already exists")),
        "{err:?}"
    );
    assert_eq!(vault.get_note_text(&p("/y.md")).await.unwrap(), original);
}

#[tokio::test]
async fn rename_a_quoted_yaml_key() {
    let (_tmp, vault) = new_vault().await;
    vault
        .create_note(&p("/q.md"), "---\n\"my key\" : v # c\nb: 2\n---\nbody")
        .await
        .unwrap();
    vault
        .rename_property(&p("/q.md"), "my key", "other")
        .await
        .unwrap();
    assert_eq!(
        vault.get_note_text(&p("/q.md")).await.unwrap(),
        "---\nother: v # c\nb: 2\n---\nbody"
    );
}

/// `property_sort_values` maps each note with the key to its first value;
/// ordering those values (missing last) gives the `or:prop:` order.
#[tokio::test]
async fn property_sort_values_match_the_query_sort() {
    let (_tmp, vault) = new_vault().await;
    let notes = [
        ("/n1.md", "---\nRank: 10\n---\nbody"),
        ("/n2.md", "---\nrank: 2\n---\nbody"),
        ("/d1.md", "---\nrank: 2024-01-15\n---\nbody"),
        ("/d2.md", "---\nrank: 2023-06-01T10:00:00Z\n---\nbody"),
        ("/t1.md", "---\nrank: Beta\n---\nbody"),
        ("/t2.md", "---\nrank: alpha\n---\nbody"),
        ("/l.md", "---\nrank: [zulu, aaa]\n---\nbody"),
        ("/none.md", "---\nother: 1\n---\nbody"),
    ];
    for (path, body) in notes {
        vault.create_note(&p(path), body).await.unwrap();
    }
    let values = vault.property_sort_values("RANK").await.unwrap();
    assert_eq!(values.len(), 7, "{values:?}");
    assert!(!values.contains_key(&p("/none.md")));
    assert!(values[&p("/n2.md")] < values[&p("/n1.md")], "numbers");
    assert!(
        values[&p("/n1.md")] < values[&p("/d2.md")],
        "numbers before dates"
    );
    assert!(
        values[&p("/d2.md")] < values[&p("/d1.md")],
        "dates by instant"
    );
    assert!(
        values[&p("/d1.md")] < values[&p("/t2.md")],
        "dates before text"
    );
    assert!(
        values[&p("/t2.md")] < values[&p("/t1.md")],
        "text ignores case"
    );
    assert!(
        values[&p("/t1.md")] < values[&p("/l.md")],
        "a list's first item"
    );
    assert!(vault.property_sort_values(" ").await.unwrap().is_empty());

    for asc in [true, false] {
        let dir = if asc { "" } else { "-" };
        let query: Vec<VaultPath> = vault
            .search_notes(format!("body {dir}or:prop:rank"))
            .await
            .unwrap()
            .into_iter()
            .map(|(e, _)| e.path)
            .collect();
        let mut ours = query.clone();
        ours.sort_by(|a, b| match (values.get(a), values.get(b)) {
            (Some(x), Some(y)) if asc => x.cmp(y),
            (Some(x), Some(y)) => y.cmp(x),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        assert_eq!(ours, query, "asc={asc}");
        assert_eq!(
            query.last(),
            Some(&p("/none.md")),
            "missing last, asc={asc}"
        );
    }
}
