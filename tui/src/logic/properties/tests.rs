use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_core::note::{PropertyKind, PropertyValue};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use super::*;
use crate::logic::message::{AppMessage, OverlayData};
use crate::test_support::temp_vault;

fn note(name: &str) -> VaultPath {
    VaultPath::note_path_from(name)
}

/// Every message sent so far, in order.
fn drain(rx: &mut UnboundedReceiver<AppMessage>) -> Vec<AppMessage> {
    let mut out = Vec::new();
    while let Ok(m) = rx.try_recv() {
        out.push(m);
    }
    out
}

/// (reloads sent, written results sent) for `path`, in order of arrival.
fn summary(msgs: &[AppMessage], path: &VaultPath) -> Vec<&'static str> {
    msgs.iter()
        .filter_map(|m| match m {
            AppMessage::NoteReloadFromDisk(p) if p == path => Some("reload"),
            AppMessage::OverlayData(OverlayData::PropertyWritten { path: p, result })
                if p == path =>
            {
                Some(if result.is_ok() { "ok" } else { "err" })
            }
            _ => None,
        })
        .collect()
}

async fn vault_with(name: &str, text: &str) -> (std::sync::Arc<NoteVault>, VaultPath) {
    let vault = temp_vault("logic_props").await;
    let path = note(name);
    vault.create_note(&path, text).await.unwrap();
    (vault, path)
}

fn form(key: &str, value: &str) -> PropertyForm {
    PropertyForm {
        key: key.into(),
        value: value.into(),
        ..PropertyForm::add()
    }
}

// ---- form model -------------------------------------------------------------

#[test]
fn edit_prefills_and_tracks_the_original() {
    let f = PropertyForm::edit("status", Some(&PropertyValue::Text("draft".into())));
    assert_eq!(f.orig_key.as_deref(), Some("status"));
    assert_eq!(f.key, "status");
    assert_eq!(f.value, "draft");
    assert_eq!(f.orig_text.as_deref(), Some("draft"));
    assert_eq!(f.current_kind, Some(PropertyKind::Text));
    assert!(!f.was_bare);

    let bare = PropertyForm::edit("due", None);
    assert!(bare.was_bare);
    assert_eq!(bare.value, "");
    assert_eq!(bare.orig_text, None);

    let empty_list = PropertyForm::edit("tags", Some(&PropertyValue::List(vec![])));
    assert_eq!(
        empty_list.value, "",
        "an empty list pre-fills empty, not []"
    );
}

#[test]
fn kind_cycles_wrapping_and_clears_feedback() {
    let mut f = PropertyForm::add();
    assert_eq!(f.kind(), None);
    assert_eq!(f.kind_label(), "auto");
    f.error = Some("x".into());
    f.store_anyway = Some(PropertyKind::Text);
    f.cycle_kind(true);
    assert_eq!(f.kind(), Some(PropertyKind::Text));
    assert_eq!((f.error.clone(), f.store_anyway), (None, None));
    f.cycle_kind(false);
    f.cycle_kind(false);
    assert_eq!(f.kind(), Some(PropertyKind::List), "auto ← wraps to list");
    f.set_kind(Some(PropertyKind::Date));
    assert_eq!(f.kind_label(), "date");
    f.set_kind(None);
    assert_eq!(f.kind(), None);
}

#[test]
fn input_values_split_only_for_lists() {
    assert_eq!(form("status", " a, b ").input_values(), vec![" a, b "]);
    let mut list = form("status", " a, ,b ");
    list.set_kind(Some(PropertyKind::List));
    assert_eq!(list.input_values(), vec!["a", "b"]);
    // auto on a key core always keeps as a list
    assert_eq!(form("tags", "x, y").input_values(), vec!["x", "y"]);
    // auto on a key that was a list
    let was_list = PropertyForm {
        value: "p, q".into(),
        ..PropertyForm::edit("things", Some(&PropertyValue::List(vec!["p".into()])))
    };
    assert_eq!(was_list.input_values(), vec!["p", "q"]);
    assert_eq!(form("tags", "x, y").inferred_kind(), PropertyKind::List);
    assert_eq!(form("n", "12").inferred_kind(), PropertyKind::Number);
}

#[test]
fn store_anyway_only_for_invalid_under_auto() {
    let invalid = PropertyWriteError::Invalid("not a number".into());
    let mut f = form("priority", "high");
    assert_eq!(f.show_write_error(&invalid), Some(PropertyKind::Text));
    assert_eq!(f.error.as_deref(), Some("not a number"));
    assert_eq!(f.store_anyway, Some(PropertyKind::Text));

    let other = PropertyWriteError::Other("io".into());
    assert_eq!(f.show_write_error(&other), None);
    assert_eq!(f.store_anyway, None);

    let mut forced = form("priority", "high");
    forced.set_kind(Some(PropertyKind::Number));
    assert_eq!(
        forced.show_write_error(&invalid),
        None,
        "a chosen type gets no offer"
    );
}

// ---- plan_write --------------------------------------------------------------

#[test]
fn plan_refusals() {
    let entries: Vec<PropertyEntry> = vec![("Status".into(), None)];
    assert_eq!(
        plan_write(&entries, &form("  ", "x"), None),
        Err(FormError::KeyRequired)
    );
    assert_eq!(
        plan_write(&entries, &form("status", "x"), None),
        Err(FormError::KeyExists("status".into())),
        "add never overwrites, keys matched like core"
    );
    assert_eq!(
        plan_write(&entries, &form("new", " "), None),
        Err(FormError::ValueRequired)
    );
    assert_eq!(
        FormError::KeyRequired.message().as_deref(),
        Some("key required")
    );
    assert_eq!(
        FormError::KeyExists("k".into()).message().as_deref(),
        Some("'k' already exists — edit it")
    );
    assert_eq!(
        FormError::ValueRequired.message().as_deref(),
        Some("value required")
    );
    assert_eq!(FormError::Unchanged.message(), None);

    let mut f = form("", "");
    f.store_anyway = Some(PropertyKind::Text);
    f.show_refusal(&FormError::KeyRequired);
    assert_eq!(
        (f.error.as_deref(), f.store_anyway),
        (Some("key required"), None)
    );
}

#[test]
fn plan_noops() {
    let entries: Vec<PropertyEntry> = vec![];
    let same = PropertyForm::edit("status", Some(&PropertyValue::Text("draft".into())));
    assert_eq!(plan_write(&entries, &same, None), Err(FormError::Unchanged));
    // forced or a chosen type does write
    assert!(plan_write(&entries, &same, Some(PropertyKind::Text)).is_ok());
    let mut typed = same.clone();
    typed.set_kind(Some(PropertyKind::Text));
    assert!(plan_write(&entries, &typed, None).is_ok());
    // a bare key left bare under its own name
    assert_eq!(
        plan_write(&entries, &PropertyForm::edit("due", None), None),
        Err(FormError::Unchanged)
    );
}

#[test]
fn plan_shapes() {
    let entries: Vec<PropertyEntry> = vec![];
    let add = plan_write(&entries, &form(" k ", "v"), None).unwrap();
    assert_eq!(add.key, "k");
    assert_eq!(add.rename_from, None);
    assert_eq!(add.rename(), None);
    assert!(add.set.is_some());

    let bare = PropertyForm {
        key: "when".into(),
        ..PropertyForm::edit("due", None)
    };
    let plan = plan_write(&entries, &bare, None).unwrap();
    assert_eq!(plan.rename(), Some(("due".into(), "when".into())));
    assert!(plan.set.is_none(), "bare key: rename only");
}

// ---- jobs on a temp vault ----------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn add_writes_and_reloads() {
    let (vault, path) = vault_with("n", "body").await;
    let (tx, mut rx) = unbounded_channel();
    let plan = plan_write(&[], &form("status", "draft"), None).unwrap();
    let r = run_write(&vault, path.clone(), plan, &tx).await;
    assert_eq!(r.unwrap(), "property saved");
    assert_eq!(
        vault.get_property(&path, "status").await.unwrap(),
        Some(Some(PropertyValue::Text("draft".into())))
    );
    assert_eq!(summary(&drain(&mut rx), &path), vec!["reload", "ok"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn unchanged_edit_writes_nothing() {
    let text = "+++\nstatus = \"draft\"\n+++\nbody";
    let (vault, path) = vault_with("n", text).await;
    let entries = vault.get_properties(&path).await.unwrap();
    let f = PropertyForm::edit(&entries[0].0, entries[0].1.as_ref());
    assert_eq!(plan_write(&entries, &f, None), Err(FormError::Unchanged));
    assert_eq!(vault.get_note_text(&path).await.unwrap(), text);
}

#[tokio::test(flavor = "multi_thread")]
async fn bare_key_rename_only() {
    let (vault, path) = vault_with("n", "---\ndue:\n---\nbody").await;
    let (tx, mut rx) = unbounded_channel();
    let entries = vault.get_properties(&path).await.unwrap();
    let f = PropertyForm {
        key: "when".into(),
        ..PropertyForm::edit("due", None)
    };
    let plan = plan_write(&entries, &f, None).unwrap();
    assert!(run_write(&vault, path.clone(), plan, &tx).await.is_ok());
    assert_eq!(vault.get_property(&path, "due").await.unwrap(), None);
    assert_eq!(vault.get_property(&path, "when").await.unwrap(), Some(None));
    assert_eq!(summary(&drain(&mut rx), &path), vec!["reload", "ok"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_then_failed_set_still_reloads() {
    let (vault, path) = vault_with("n", "+++\nstatus = \"draft\"\n+++\nbody").await;
    let (tx, mut rx) = unbounded_channel();
    let entries = vault.get_properties(&path).await.unwrap();
    let mut f = PropertyForm {
        key: "when".into(),
        value: "not a date".into(),
        ..PropertyForm::edit("status", entries[0].1.as_ref())
    };
    f.set_kind(Some(PropertyKind::Date));
    let plan = plan_write(&entries, &f, None).unwrap();
    assert_eq!(plan.rename(), Some(("status".into(), "when".into())));
    assert!(run_write(&vault, path.clone(), plan, &tx).await.is_err());
    assert_eq!(
        vault.get_note_text(&path).await.unwrap(),
        "+++\nwhen = \"draft\"\n+++\nbody",
        "the rename landed"
    );
    assert_eq!(summary(&drain(&mut rx), &path), vec!["reload", "err"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_rename_is_other_and_sends_no_reload() {
    let (vault, path) = vault_with("n", "+++\na = 1\nb = 2\n+++\nbody").await;
    let (tx, mut rx) = unbounded_channel();
    let entries = vault.get_properties(&path).await.unwrap();
    let f = PropertyForm {
        key: "b".into(),
        ..PropertyForm::edit("a", entries[0].1.as_ref())
    };
    let plan = plan_write(&entries, &f, None).unwrap();
    let r = run_write(&vault, path.clone(), plan, &tx).await;
    assert!(matches!(r, Err(PropertyWriteError::Other(_))), "{r:?}");
    assert_eq!(summary(&drain(&mut rx), &path), vec!["err"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn mismatch_offers_store_anyway_which_forces_the_kind() {
    let (vault, path) = vault_with("n", "body").await;
    for n in ["o1", "o2"] {
        vault
            .create_note(&note(n), "+++\npriority = 1\n+++\nx")
            .await
            .unwrap();
    }
    let (tx, mut rx) = unbounded_channel();
    let mut f = form("priority", "high");
    let plan = plan_write(&[], &f, None).unwrap();
    let err = run_write(&vault, path.clone(), plan, &tx)
        .await
        .unwrap_err();
    assert!(matches!(err, PropertyWriteError::Invalid(_)));
    assert_eq!(
        summary(&drain(&mut rx), &path),
        vec!["err"],
        "nothing changed"
    );
    let kind = f.show_write_error(&err).unwrap();
    assert_eq!(kind, PropertyKind::Text);
    let plan = plan_write(&[], &f, Some(kind)).unwrap();
    assert!(run_write(&vault, path.clone(), plan, &tx).await.is_ok());
    assert_eq!(
        vault.get_property(&path, "priority").await.unwrap(),
        Some(Some(PropertyValue::Text("high".into())))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_frontmatter_write_is_malformed() {
    let text = "+++\na = 1\na = 2\n+++\nbody";
    let (vault, path) = vault_with("n", text).await;
    let (tx, mut rx) = unbounded_channel();
    let plan = plan_write(&[], &form("b", "x"), None).unwrap();
    let r = run_write(&vault, path.clone(), plan, &tx).await;
    assert!(matches!(r, Err(PropertyWriteError::Malformed(_))), "{r:?}");
    assert_eq!(vault.get_note_text(&path).await.unwrap(), text);
    assert_eq!(summary(&drain(&mut rx), &path), vec!["err"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_removes_and_reloads() {
    let (vault, path) = vault_with("n", "+++\nstatus = \"draft\"\n+++\nbody").await;
    let (tx, mut rx) = unbounded_channel();
    assert_eq!(
        run_delete(&vault, path.clone(), "status", &tx)
            .await
            .unwrap(),
        "property removed"
    );
    assert_eq!(vault.get_property(&path, "status").await.unwrap(), None);
    assert_eq!(summary(&drain(&mut rx), &path), vec!["reload", "ok"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn load_and_load_keys_send_their_messages() {
    let (vault, path) = vault_with("n", "+++\nstatus = \"draft\"\n+++\nbody").await;
    let (tx, mut rx) = unbounded_channel();
    let entries = load(&vault, path.clone(), &tx).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        row_cells(&entries[0]),
        ("status".into(), "text".into(), "draft".into())
    );
    assert_eq!(
        row_cells(&("due".into(), None)),
        ("due".into(), "—".into(), String::new())
    );
    let keys = load_keys(&vault, &tx).await.unwrap();
    assert!(keys.iter().any(|k| k == "status"), "{keys:?}");
    let msgs = drain(&mut rx);
    assert!(matches!(
        &msgs[0],
        AppMessage::OverlayData(OverlayData::PropertiesLoaded { path: p, result: Ok(e) })
            if *p == path && e.len() == 1
    ));
    assert!(matches!(
        &msgs[1],
        AppMessage::OverlayData(OverlayData::PropertyKeysLoaded(k)) if *k == keys
    ));
    assert_eq!(msgs.len(), 2);

    let missing = load(&vault, note("missing"), &tx).await;
    assert!(missing.is_err());
}
