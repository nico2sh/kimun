//! The plain texts of a collection view.
//!
//! It owns the words of the datelines and of the collection line: the
//! folder of a note, the piece and note counts, and the report of a
//! refresh. It draws nothing and holds no state.

use chrono::NaiveTime;
use kimun_core::nfs::VaultPath;

/// The folder a dateline names for the note at `path`: its parent folder,
/// written from the vault root without a leading separator
/// (`journal/2026`). `None` for a note at the vault root.
pub(crate) fn note_folder(path: &VaultPath) -> Option<String> {
    let (mut parent, _) = path.flatten().get_parent_path();
    if parent.is_root_or_empty() {
        return None;
    }
    parent.to_relative();
    Some(parent.to_string())
}

/// `1 piece`, `2 pieces`.
pub(crate) fn piece_count(pieces: usize) -> String {
    match pieces {
        1 => "1 piece".into(),
        n => format!("{n} pieces"),
    }
}

/// A dateline's meta text: the note's folder ([`note_folder`]) and its
/// drawn pieces ([`piece_count`]), joined by ` · `; the count alone at the
/// vault root.
pub(crate) fn dateline_meta(path: &VaultPath, pieces: usize) -> String {
    let count = piece_count(pieces);
    match note_folder(path) {
        Some(folder) => format!("{folder} \u{b7} {count}"),
        None => count,
    }
}

/// The local wall-clock time.
pub(super) fn local_now() -> NaiveTime {
    chrono::Local::now().time()
}

/// `1 note`, `2 notes`.
pub(super) fn note_count(notes: usize) -> String {
    match notes {
        1 => "1 note".into(),
        n => format!("{n} notes"),
    }
}

/// What the collection line says for a while after a refresh: the notes
/// `added` and `removed` against the list before, and the picks `dropped`
/// because no note holds them any more. The first count names the notes.
pub(crate) fn refresh_text(added: usize, removed: usize, dropped: usize) -> String {
    let mut changes = Vec::new();
    if added > 0 {
        changes.push(format!("{} added", note_count(added)));
    }
    if removed > 0 {
        changes.push(match changes.is_empty() {
            true => format!("{} removed", note_count(removed)),
            false => format!("{removed} removed"),
        });
    }
    let mut text = match changes.is_empty() {
        true => "Refreshed: no changes".to_string(),
        false => format!("Refreshed: {}", changes.join(", ")),
    };
    if dropped > 0 {
        let picks = if dropped == 1 { "pick" } else { "picks" };
        text.push_str(&format!(" \u{b7} {dropped} {picks} dropped"));
    }
    text
}
