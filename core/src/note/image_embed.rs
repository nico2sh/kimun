//! Inline image embeds: the lines of a note that embed a vault image on a
//! line of their own (`![alt](path)` or `![[name]]`), and the vault file each
//! one points at (see `CONTEXT.md`, **Inline image** and **Closest-file
//! rule**). Pure functions over note text and path lists: the caller brings
//! the attachment list ([`NoteVault::list_attachments`]).
//!
//! [`NoteVault::list_attachments`]: crate::NoteVault::list_attachments

use crate::nfs::{VaultPath, PATH_SEPARATOR};

use super::content_extractor::{
    is_remote_url, md_link_matches, split_link_fragment, target_looks_like_image, wikilink_matches,
    ExclusionZones,
};
use super::link_target::{markdown_path, resolve_in_folder_of};

/// A line of a note that is nothing but an image embed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageEmbed {
    /// Zero-based line of the note, counted as an editor counts lines.
    pub row: usize,
    /// The alt text of `![alt](path)`. Empty for `![[name]]`.
    pub alt: String,
    /// The destination as written: the markdown destination, or the
    /// wikilink target without its `|size` part.
    pub target: String,
    /// Whether `target` is an exact path. `![alt](path)` and a wikilink with
    /// a folder part (`![[dir/name.png]]`) are. A bare `![[name.png]]` is
    /// not, and takes the closest-file rule.
    pub exact: bool,
}

/// The embeds that fill a whole line of `text`, in order. An embed inside
/// prose, inside code or inside frontmatter is not one, and neither is a
/// remote URL or a target that is not an image.
pub fn image_embeds(text: &str) -> Vec<ImageEmbed> {
    let zones = ExclusionZones::from_text(text);
    let mut embeds = Vec::new();
    let mut line_start = 0;
    for (row, raw) in text.split('\n').enumerate() {
        let start = line_start;
        line_start += raw.len() + 1;
        let line = raw.trim();
        if !line.starts_with('!') || zones.contains_code_link_or_frontmatter(start) {
            continue;
        }
        if let Some(embed) = whole_line_embed(row, line) {
            embeds.push(embed);
        }
    }
    embeds
}

/// The embed `line` (trimmed, one line) is, when it is nothing else.
fn whole_line_embed(row: usize, line: &str) -> Option<ImageEmbed> {
    if let Some(m) = md_link_matches(line).next() {
        if !m.image || m.range != (0..line.len()) {
            return None;
        }
        let target = m.target.trim_start_matches('<').trim_end_matches('>');
        if is_remote_url(target) || !target_looks_like_image(split_link_fragment(target).0) {
            return None;
        }
        return Some(ImageEmbed {
            row,
            alt: line[m.label].trim().to_string(),
            target: m.target.to_string(),
            exact: true,
        });
    }
    let inner = line.strip_prefix('!')?;
    let span = wikilink_matches(inner).next()?;
    if span != (0..inner.len()) {
        return None;
    }
    let written = inner[2..inner.len() - 2].split('|').next()?.trim();
    let name = split_link_fragment(written).0;
    if is_remote_url(name) || !target_looks_like_image(name) {
        return None;
    }
    Some(ImageEmbed {
        row,
        alt: String::new(),
        target: written.to_string(),
        exact: name.contains(PATH_SEPARATOR),
    })
}

/// The vault file `embed`, written in the note at `note`, points at, when it
/// is in `attachments`. An exact embed is resolved against the note's
/// folder. A bare name takes the closest file of that name: the note's own
/// folder, then the nearest parent folder, then the nearest subfolder. A file
/// elsewhere in the vault (a sibling folder, a root `assets` folder) comes
/// last, and path order breaks every remaining tie, so the choice is stable.
pub fn resolve_image_embed(
    embed: &ImageEmbed,
    note: &VaultPath,
    attachments: &[VaultPath],
) -> Option<VaultPath> {
    // Filtered by name first: the text changes on each keystroke, and only
    // the few files of that name are worth a canonical copy.
    let written = markdown_path(&embed.target)?;
    if embed.exact {
        let path = resolve_in_folder_of(&written, note);
        let name = path.get_name();
        return attachments
            .iter()
            .filter(|candidate| candidate.name_is(&name))
            .map(VaultPath::canonical)
            .find(|candidate| *candidate == path);
    }
    let name = written.get_name();
    let (folder, _) = note.canonical().get_parent_path();
    let folder = folder.get_slices();
    attachments
        .iter()
        .filter(|candidate| candidate.name_is(&name))
        .map(VaultPath::canonical)
        .min_by_key(|candidate| {
            let (parent, _) = candidate.get_parent_path();
            (closeness(&folder, &parent.get_slices()), candidate.clone())
        })
}

/// How far a file in `parent` is from a note in `folder`, as a sort key:
/// the note's own folder, then each parent folder going up, then each
/// subfolder going down, then any other folder.
fn closeness(folder: &[String], parent: &[String]) -> (u8, usize) {
    if parent == folder {
        (0, 0)
    } else if folder.starts_with(parent) {
        (1, folder.len() - parent.len())
    } else if parent.starts_with(folder) {
        (2, parent.len() - folder.len())
    } else {
        (3, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(p: &str) -> VaultPath {
        VaultPath::new(p)
    }

    fn bare(name: &str) -> ImageEmbed {
        ImageEmbed {
            row: 0,
            alt: String::new(),
            target: name.to_string(),
            exact: false,
        }
    }

    #[test]
    fn whole_line_markdown_image_is_an_embed() {
        let embeds = image_embeds("![a](pic.png)");
        assert_eq!(embeds.len(), 1);
        assert_eq!(embeds[0].alt, "a");
        assert_eq!(embeds[0].target, "pic.png");
        assert!(embeds[0].exact);
    }

    #[test]
    fn whole_line_wikilink_image_is_an_embed() {
        let embeds = image_embeds("![[pic.png]]");
        assert_eq!(embeds.len(), 1);
        assert_eq!(embeds[0].alt, "");
        assert_eq!(embeds[0].target, "pic.png");
        assert!(!embeds[0].exact);
    }

    #[test]
    fn a_wikilink_with_a_folder_is_exact_and_a_size_is_dropped() {
        let embeds = image_embeds("![[assets/pic.png|200]]");
        assert_eq!(embeds[0].target, "assets/pic.png");
        assert!(embeds[0].exact);
    }

    #[test]
    fn rows_count_lines_and_indentation_is_allowed() {
        let embeds = image_embeds("title\n\n  ![[a.png]]  \r\n![](b.png)");
        let rows: Vec<usize> = embeds.iter().map(|e| e.row).collect();
        assert_eq!(rows, vec![2, 3]);
    }

    #[test]
    fn embed_inside_prose_is_not_an_embed() {
        assert!(image_embeds("see ![a](pic.png) here").is_empty());
        assert!(image_embeds("![a](pic.png) here").is_empty());
        assert!(image_embeds("![[pic.png]] here").is_empty());
        assert!(image_embeds("![a](pic.png)![b](pic.png)").is_empty());
    }

    #[test]
    fn remote_url_is_not_an_embed() {
        assert!(image_embeds("![a](https://x.com/p.png)").is_empty());
        assert!(image_embeds("![[https://x.com/p.png]]").is_empty());
    }

    #[test]
    fn a_non_image_or_a_plain_link_is_not_an_embed() {
        assert!(image_embeds("![a](doc.pdf)").is_empty());
        assert!(image_embeds("[a](pic.png)").is_empty());
        assert!(image_embeds("![[note]]").is_empty());
    }

    #[test]
    fn code_and_frontmatter_hold_no_embeds() {
        assert!(image_embeds("```\n![a](pic.png)\n```").is_empty());
        assert!(image_embeds("---\ncover: x\n![a](pic.png)\n---\nbody").is_empty());
    }

    #[test]
    fn closest_file_prefers_the_notes_own_folder() {
        let files = [vp("/a/pic.png"), vp("/a/b/pic.png"), vp("/pic.png")];
        assert_eq!(
            resolve_image_embed(&bare("pic.png"), &vp("/a/b/n.md"), &files),
            Some(vp("/a/b/pic.png"))
        );
    }

    #[test]
    fn closest_file_then_nearest_parent() {
        let files = [vp("/pic.png"), vp("/a/pic.png"), vp("/a/b/c/d/pic.png")];
        assert_eq!(
            resolve_image_embed(&bare("pic.png"), &vp("/a/b/c/n.md"), &files),
            Some(vp("/a/pic.png"))
        );
    }

    #[test]
    fn closest_file_then_nearest_subfolder() {
        let files = [vp("/a/x/y/pic.png"), vp("/a/x/pic.png")];
        assert_eq!(
            resolve_image_embed(&bare("pic.png"), &vp("/a/n.md"), &files),
            Some(vp("/a/x/pic.png"))
        );
    }

    #[test]
    fn a_file_in_a_sibling_folder_still_resolves_last() {
        let files = [vp("/assets/pic.png")];
        assert_eq!(
            resolve_image_embed(&bare("pic.png"), &vp("/journal/n.md"), &files),
            Some(vp("/assets/pic.png"))
        );
    }

    #[test]
    fn ties_break_by_path_order() {
        let files = [vp("/a/z/pic.png"), vp("/a/y/pic.png")];
        assert_eq!(
            resolve_image_embed(&bare("pic.png"), &vp("/a/n.md"), &files),
            Some(vp("/a/y/pic.png"))
        );
    }

    #[test]
    fn exact_path_is_resolved_against_the_note_folder() {
        let files = [vp("/a/pic.png"), vp("/a/img/pic.png"), vp("/pic.png")];
        let note = vp("/a/n.md");
        let rel = image_embeds("![](img/pic.png)").remove(0);
        assert_eq!(
            resolve_image_embed(&rel, &note, &files),
            Some(vp("/a/img/pic.png"))
        );
        let up = image_embeds("![](../pic.png)").remove(0);
        assert_eq!(
            resolve_image_embed(&up, &note, &files),
            Some(vp("/pic.png"))
        );
        let abs = image_embeds("![[/a/pic.png]]").remove(0);
        assert_eq!(
            resolve_image_embed(&abs, &note, &files),
            Some(vp("/a/pic.png"))
        );
        // An exact path never falls back to another file of the same name.
        let wrong = image_embeds("![](other/pic.png)").remove(0);
        assert_eq!(resolve_image_embed(&wrong, &note, &files), None);
    }

    #[test]
    fn missing_file_resolves_to_none() {
        let files = [vp("/a/other.png")];
        assert_eq!(
            resolve_image_embed(&bare("pic.png"), &vp("/a/n.md"), &files),
            None
        );
        assert_eq!(
            resolve_image_embed(&bare("pic.png"), &vp("/a/n.md"), &[]),
            None
        );
    }

    #[test]
    fn names_match_case_insensitively() {
        let files = [vp("/a/pic.png")];
        assert_eq!(
            resolve_image_embed(&bare("PIC.PNG"), &vp("/a/n.md"), &files),
            Some(vp("/a/pic.png"))
        );
    }
}
