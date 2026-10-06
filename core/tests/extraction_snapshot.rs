//! Snapshot of every whole-note extractor over a fixed corpus: the
//! `example/` vault, the bench fixtures and `tests/fixtures/walk`. Taken
//! before the single-walk change (`2026-10-06-single-note-walk-design.md`)
//! so every behaviour change shows up as a reviewed diff.
//!
//! Regenerate with `UPDATE_SNAPSHOT=1 cargo test -p kimun_core --test extraction_snapshot`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kimun_core::nfs::VaultPath;
use kimun_core::note::{self, NoteDetails, NoteLink};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// `(name, text)` for every corpus note, sorted by name. Line endings are
/// normalized so a CRLF checkout snapshots the same.
pub fn corpus() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect(&root.join("../example"), &mut files);
    collect(&root.join("benches/fixtures"), &mut files);
    collect(&root.join("tests/fixtures/walk"), &mut files);
    let mut notes: Vec<(String, String)> = files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path)
                .unwrap()
                .replace("\r\n", "\n");
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string()
                .replace('\\', "/");
            (name, text)
        })
        .collect();
    notes.sort();
    notes
}

fn link_line(link: &NoteLink) -> String {
    format!("{:?} {:?} {:?}", link.ltype, link.raw_link, link.text)
}

fn render(name: &str, text: &str) -> String {
    let path = VaultPath::new("folder/note.md");
    let details = NoteDetails::new(&path, text);
    let mut out = String::new();
    writeln!(out, "=== {name}").unwrap();
    writeln!(out, "-- title {:?}", details.get_title()).unwrap();
    writeln!(out, "-- headings").unwrap();
    for h in note::note_headings(text) {
        writeln!(out, "{} {} {:?}", h.level, h.line, h.text).unwrap();
    }
    writeln!(out, "-- chunks").unwrap();
    for c in details.get_content_chunks() {
        writeln!(out, "[{}] {:?}", c.breadcrumb, c.text).unwrap();
    }
    let (index_chunks, index_links) = details.get_chunks_and_links();
    writeln!(out, "-- index chunks").unwrap();
    for c in index_chunks {
        writeln!(out, "[{}] {:?}", c.breadcrumb, c.text).unwrap();
    }
    writeln!(out, "-- index links (sorted)").unwrap();
    let mut lines: Vec<String> = index_links.iter().map(link_line).collect();
    lines.sort();
    for l in lines {
        writeln!(out, "{l}").unwrap();
    }
    writeln!(out, "-- tags {:?}", note::note_tags(text)).unwrap();
    let (md, md_links) = details.get_markdown_and_links();
    writeln!(out, "-- markdown links").unwrap();
    for l in &md_links {
        writeln!(out, "{}", link_line(l)).unwrap();
    }
    writeln!(out, "-- markdown").unwrap();
    out.push_str(&md);
    out.push_str("\n\n");
    out
}

#[test]
fn extraction_snapshot() {
    let actual: String = corpus().iter().map(|(n, t)| render(n, t)).collect();
    let snapshot = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/extraction.txt");
    if std::env::var_os("UPDATE_SNAPSHOT").is_some() {
        std::fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        std::fs::write(&snapshot, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&snapshot)
        .expect("no snapshot yet: run with UPDATE_SNAPSHOT=1")
        .replace("\r\n", "\n");
    if expected != actual {
        let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("extraction.actual.txt");
        std::fs::write(&out, &actual).unwrap();
        panic!(
            "extraction output changed — review: diff {} {}",
            snapshot.display(),
            out.display()
        );
    }
}

#[test]
fn every_extractor_agrees_with_the_others() {
    use kimun_core::note::scan::heading_display_text;
    use kimun_core::note::LinkType;

    // A vault-root note: there the rendered markdown's note-relative wikilink
    // resolution and the index's root resolution coincide. Below the root
    // they differ for multi-segment wikilinks (a pre-existing difference the
    // spec documents in Testing section 2).
    let path = VaultPath::new("note.md");
    for (name, text) in corpus() {
        let details = NoteDetails::new(&path, &text);
        let headings = note::note_headings(&text);
        let chunks = details.get_content_chunks();
        let (index_chunks, index_links) = details.get_chunks_and_links();
        assert_eq!(chunks, index_chunks, "{name}: chunk paths disagree");

        // Every breadcrumb's last segment is a listed heading's text.
        for chunk in chunks.iter().filter(|c| c.breadcrumb != "FrontMatter") {
            if let Some(last) = chunk.breadcrumb_last() {
                assert!(
                    headings.iter().any(|h| h.text == last),
                    "{name}: breadcrumb {last:?} is no heading"
                );
            }
        }

        // An ATX heading on its own line renders the same alone (a setext
        // heading cannot: its underline is on the next row).
        let rows: Vec<&str> = text.lines().collect();
        let is_atx = |row: &str| {
            let hashes = row.len() - row.trim_start_matches('#').len();
            (1..=6).contains(&hashes)
                && matches!(row[hashes..].chars().next(), None | Some(' ' | '\t'))
        };
        for h in &headings {
            let row = rows[h.line];
            if is_atx(row) {
                assert_eq!(
                    heading_display_text(row).as_deref(),
                    Some(h.text.as_str()),
                    "{name}: row {}",
                    h.line
                );
            }
        }

        // The title is the first heading's text when the note opens with one
        // (notes with frontmatter are skipped: their first rows are the block).
        let has_frontmatter = text.starts_with("---") || text.starts_with("+++");
        if let Some(first) = headings.first() {
            let first_row = rows.iter().position(|r| !r.trim().is_empty());
            if !has_frontmatter && first_row == Some(first.line) {
                assert_eq!(details.get_title(), first.text, "{name}: title");
            }
        }

        // Tags agree across the index, the markdown rewrite and note_tags.
        let (_, md_links) = details.get_markdown_and_links();
        let tags_of = |links: &[NoteLink]| -> std::collections::BTreeSet<String> {
            links
                .iter()
                .filter(|l| matches!(l.ltype, LinkType::Hashtag))
                .map(|l| l.text.to_lowercase())
                .collect()
        };
        assert_eq!(tags_of(&index_links), tags_of(&md_links), "{name}: tags");
        assert_eq!(
            tags_of(&index_links),
            note::extract_labels(&text).into_iter().collect(),
            "{name}: extract_labels"
        );

        // Note links agree (embeds are images in the rewrite, so the rewrite
        // lists a subset of the index's notes).
        let notes_of = |links: &[NoteLink]| -> std::collections::BTreeSet<String> {
            links
                .iter()
                .filter_map(|l| match &l.ltype {
                    LinkType::Note(p) => Some(p.to_string()),
                    _ => None,
                })
                .collect()
        };
        assert!(
            notes_of(&md_links).is_subset(&notes_of(&index_links)),
            "{name}: rewrite lists a note link the index lacks"
        );
    }
}
