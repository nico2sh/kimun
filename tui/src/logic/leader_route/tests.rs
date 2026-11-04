#![cfg(test)]

use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

use super::*;
use crate::logic::message::AppMessage;

/// Every action the table covers: `ALL` plus `Help` (kept out of `ALL` so its
/// length stays the leaf count, see `LeaderAction::from_id`).
fn every_action() -> Vec<LeaderAction> {
    let mut all = LeaderAction::ALL.to_vec();
    all.push(LeaderAction::Help);
    all
}

/// The route of `action` with `note` open — the full table. The leaves that
/// act on "this note" are listed with the note they carry.
fn with_note(action: LeaderAction, note: &VaultPath) -> LeaderRoute {
    use LeaderAction as A;
    use LeaderRoute as R;
    match action {
        A::OpenDrawer(v) => R::Drawer(v),
        A::FindFiles => R::Overlay(OverlayOpen::FileFinder),
        A::FindGrep | A::FindRecent => R::Overlay(OverlayOpen::SearchBrowser),
        A::FindTags => R::Drawer(DrawerView::Tags),
        A::FindBacklinks => R::FindWithQuery("<{note}".into()),
        A::FindSaved => R::Overlay(OverlayOpen::SavedSearches),
        A::FindHeadings => R::Drawer(DrawerView::Outline),
        A::FindPinned => R::Overlay(OverlayOpen::PinnedNotes),
        A::NoteNew => R::NewNote,
        A::NoteDaily => R::OpenJournal,
        A::NoteFromTemplate => R::Soon("templates — coming soon"),
        A::NoteRename => R::NoteDialog(NoteDialog::Rename, note.clone()),
        A::NoteMove => R::NoteDialog(NoteDialog::Move, note.clone()),
        A::NoteDelete => R::NoteDialog(NoteDialog::Delete, note.clone()),
        A::NoteProperties => R::NoteDialog(NoteDialog::Properties, note.clone()),
        A::LinksTab(t) => R::LinksTab(t),
        A::LinksGraph => R::Soon("local graph — coming soon"),
        A::GitStatus => R::GitStatus,
        A::GitSync | A::GitLog | A::GitDiff => R::Soon("git is display-only for now"),
        A::VaultSwitch => R::Overlay(OverlayOpen::WorkspaceSwitcher),
        A::VaultReindex => R::Job(LeaderJob::Reindex),
        A::VaultConfig => R::Drawer(DrawerView::Config),
        A::VaultTheme => R::Overlay(OverlayOpen::ThemePicker),
        A::VaultPreferences => R::OpenPreferences,
        A::WindowZen => R::Layout(LayoutStep::Zen),
        A::WindowSplit => R::Soon("editor splits — coming soon"),
        A::WindowGrowDrawer => R::Layout(LayoutStep::GrowDrawer),
        A::WindowShrinkDrawer => R::Layout(LayoutStep::ShrinkDrawer),
        A::NoteToggleTodo => R::Soon("toggle todo — coming soon"),
        A::NotePreview => R::Soon("preview — lands with phase 09"),
        A::NoteCopyWikilink => R::Yank {
            text: format!("[[{}]]", note.get_clean_name()),
            label: "wikilink copied",
        },
        A::NoteExport => R::Soon("export — coming soon"),
        A::NoteYankPath => R::Yank {
            text: note.to_string(),
            label: "note path copied",
        },
        A::PinnedJump(n) => R::Job(LeaderJob::JumpToPinned(n)),
        A::NoteTogglePin => R::Job(LeaderJob::TogglePin(note.clone())),
        A::AskFocus => R::Ask(AskStep::Focus),
        A::AskNew => R::Ask(AskStep::New),
        A::AskCopy => R::Ask(AskStep::Copy),
        A::AskSave => R::Ask(AskStep::Save),
        A::AskRegenerate => R::Ask(AskStep::Regenerate),
        A::AskSource => R::Ask(AskStep::Source),
        A::TextBold => R::Text(TextAction::Bold),
        A::TextItalic => R::Text(TextAction::Italic),
        A::TextStrikethrough => R::Text(TextAction::Strikethrough),
        A::Palette => R::Overlay(OverlayOpen::CommandPalette),
        A::Help => R::Overlay(OverlayOpen::Cheatsheet),
        A::NoteSave => R::Save,
        A::AppQuit => R::Quit,
        A::AppOnboarding => R::OpenOnboarding,
        A::AppCheckUpdates => R::Job(LeaderJob::CheckUpdates),
    }
}

/// Whether `action` acts on "this note" (and so refuses with no note open).
fn needs_note(action: LeaderAction) -> bool {
    use LeaderAction as A;
    matches!(
        action,
        A::NoteRename
            | A::NoteMove
            | A::NoteDelete
            | A::NoteProperties
            | A::NoteCopyWikilink
            | A::NoteYankPath
            | A::NoteTogglePin
    )
}

#[test]
fn every_action_routes_with_a_note_open() {
    let note = VaultPath::new("plans/roadmap.md");
    let facts = RouteFacts::new(note.clone(), false);
    for action in every_action() {
        assert_eq!(
            route(action, &facts),
            with_note(action, &note),
            "{action:?}"
        );
    }
}

/// With no note — the vault root, an attachment, or the Ask workspace up —
/// the "+this note" leaves refuse and every other leaf is unchanged.
#[test]
fn every_action_routes_without_a_note() {
    let note = VaultPath::new("plans/roadmap.md");
    let cases = [
        ("vault root", RouteFacts::new(VaultPath::root(), false)),
        (
            "attachment shown",
            RouteFacts::new(VaultPath::new("assets/diagram.png"), false),
        ),
        ("ask shown", RouteFacts::new(note.clone(), true)),
    ];
    for (case, facts) in cases {
        assert_eq!(facts.open_note(), None, "{case}");
        for action in every_action() {
            let expected = if needs_note(action) {
                LeaderRoute::NoNoteOpen
            } else {
                with_note(action, &note)
            };
            assert_eq!(route(action, &facts), expected, "{case}: {action:?}");
        }
    }
}

#[test]
fn note_dialogs_map_to_their_file_op() {
    let p = VaultPath::new("a.md");
    assert!(matches!(NoteDialog::Rename.file_op(p.clone()), FileOp::ShowRename(x) if x == p));
    assert!(matches!(NoteDialog::Move.file_op(p.clone()), FileOp::ShowMove(x) if x == p));
    assert!(matches!(NoteDialog::Delete.file_op(p.clone()), FileOp::ShowDelete(x) if x == p));
    assert!(
        matches!(NoteDialog::Properties.file_op(p.clone()), FileOp::ShowProperties(x) if x == p)
    );
}

// ── jobs, on a temp vault ────────────────────────────────────────────────

async fn temp_vault() -> (Arc<NoteVault>, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().unwrap();
    let vault = NoteVault::new(kimun_core::VaultConfig::new(crate::test_support::sys(
        dir.path(),
    )))
    .await
    .unwrap();
    (Arc::new(vault), dir)
}

/// Run `job` (given the sender) to completion and return every message it sent.
async fn sent<F: std::future::Future<Output = ()>>(
    job: impl FnOnce(crate::logic::message::AppTx) -> F,
) -> Vec<AppMessage> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    job(tx).await;
    let mut out = Vec::new();
    while let Ok(m) = rx.try_recv() {
        out.push(m);
    }
    out
}

fn flashes(msgs: &[AppMessage]) -> Vec<&str> {
    msgs.iter()
        .filter_map(|m| match m {
            AppMessage::FlashMessage(s) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn toggle_pin_pins_then_unpins() {
    let (vault, _dir) = temp_vault().await;
    let note = VaultPath::new("plan.md");
    vault.create_note(&note, "hi").await.unwrap();
    let (v, n) = (vault.clone(), note.clone());
    let msgs = sent(|tx| jobs::toggle_pin(v, n, tx)).await;
    assert_eq!(flashes(&msgs), ["pinned as 1 of 9"]);
    assert_eq!(
        vault.list_pinned_notes().await.unwrap(),
        vec![VaultPath::new("/plan.md")]
    );
    let (v, n) = (vault.clone(), note.clone());
    let msgs = sent(|tx| jobs::toggle_pin(v, n, tx)).await;
    assert_eq!(flashes(&msgs), ["unpinned"]);
    assert!(vault.list_pinned_notes().await.unwrap().is_empty());
}

#[tokio::test]
async fn toggle_pin_refuses_a_tenth_pin() {
    let (vault, _dir) = temp_vault().await;
    for i in 0..kimun_core::PINNED_NOTES_CAP {
        let n = VaultPath::new(format!("n{i}.md"));
        vault.create_note(&n, "hi").await.unwrap();
        vault.toggle_pinned_note(&n).await.unwrap();
    }
    let extra = VaultPath::new("extra.md");
    vault.create_note(&extra, "hi").await.unwrap();
    let v = vault.clone();
    let msgs = sent(|tx| jobs::toggle_pin(v, extra, tx)).await;
    assert_eq!(flashes(&msgs), ["already 9 pinned — unpin one first"]);
}

#[tokio::test]
async fn toggle_pin_on_a_missing_note_flashes() {
    let (vault, _dir) = temp_vault().await;
    let v = vault.clone();
    let msgs = sent(|tx| jobs::toggle_pin(v, VaultPath::new("gone.md"), tx)).await;
    assert_eq!(flashes(&msgs), ["note not found: gone.md"]);
    assert!(vault.list_pinned_notes().await.unwrap().is_empty());
}

#[tokio::test]
async fn jump_to_pinned_opens_the_nth_pin() {
    let (vault, _dir) = temp_vault().await;
    let a = VaultPath::new("a.md");
    let b = VaultPath::new("b.md");
    vault.create_note(&a, "a").await.unwrap();
    vault.create_note(&b, "b").await.unwrap();
    vault.toggle_pinned_note(&a).await.unwrap();
    vault.toggle_pinned_note(&b).await.unwrap();
    let v = vault.clone();
    let msgs = sent(|tx| jobs::jump_to_pinned(v, 2, tx)).await;
    assert!(
        matches!(&msgs[..], [AppMessage::OpenPath { path, emphasis: None }] if path.is_like(&b)),
        "{msgs:?}"
    );
}

#[tokio::test]
async fn jump_to_an_empty_slot_flashes() {
    let (vault, _dir) = temp_vault().await;
    let msgs = sent(|tx| jobs::jump_to_pinned(vault, 4, tx)).await;
    assert_eq!(flashes(&msgs), ["no pinned note 4"]);
}

#[tokio::test]
async fn jump_to_a_missing_pin_flashes_and_keeps_it() {
    let (vault, dir) = temp_vault().await;
    let gone = VaultPath::new("gone.md");
    vault.create_note(&gone, "hi").await.unwrap();
    vault.toggle_pinned_note(&gone).await.unwrap();
    std::fs::remove_file(dir.path().join("gone.md")).unwrap();
    let v = vault.clone();
    let msgs = sent(|tx| jobs::jump_to_pinned(v, 1, tx)).await;
    assert_eq!(flashes(&msgs), ["pinned note not found: /gone.md"]);
    assert_eq!(vault.list_pinned_notes().await.unwrap().len(), 1);
}

#[tokio::test]
async fn reindex_reports_its_duration() {
    let (vault, _dir) = temp_vault().await;
    let msgs = sent(|tx| jobs::reindex(vault, tx)).await;
    let f = flashes(&msgs);
    assert_eq!(f.len(), 1);
    assert!(f[0].starts_with("reindexed in "), "{f:?}");
}
