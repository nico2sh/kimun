//! The note a link points at, by its *form*: a wikilink names a note (any
//! folder; a path part is a suffix that disambiguates), a markdown link is a
//! path (exact). Every reader that asks "does link L point at note P" asks
//! [`NoteTarget::matches`]; the index's SQL twin is `link_points_at_sql`
//! (`index/mod.rs`), and a test runs one case table through both.

use crate::nfs::VaultPath;

use super::content_extractor::{
    is_remote_url, link_target_filename, split_link_fragment, target_looks_like_image, LinkSpan,
    LinkSpanKind,
};

/// Extensions that make a wikilink target a file other than a note
/// (`![[pic.png]]`, `[[doc.pdf]]`), lowercase. Images come from
/// [`target_looks_like_image`]. Anything else with a dot is read as a note
/// name (`[[v1.2]]` is `v1.2.md`).
const ATTACHMENT_EXTENSIONS: &[&str] = &[
    "pdf", "txt", "csv", "tsv", "json", "xml", "html", "htm", "doc", "docx", "odt", "rtf", "xls",
    "xlsx", "ods", "ppt", "pptx", "odp", "epub", "zip", "tar", "gz", "tgz", "bz2", "xz", "7z",
    "rar", "mp3", "wav", "ogg", "flac", "m4a", "aac", "opus", "mp4", "m4v", "mov", "avi", "mkv",
    "webm", "wmv",
];

/// Whether a wikilink target (its fragment cut) names a non-note file by
/// its extension: an image or a known attachment type. Case-insensitive.
fn names_a_non_note_file(target: &str) -> bool {
    if target_looks_like_image(target) {
        return true;
    }
    match link_target_filename(target).rsplit_once('.') {
        Some((_, ext)) => ATTACHMENT_EXTENSIONS
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known)),
        None => false,
    }
}

/// The note a link points at, and how it matches.
///
/// The bracket style only matters while parsing: `[[/work/dup]]` and
/// `[[../x/dup]]` behave exactly like markdown paths, so what every reader
/// sees is the match kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NoteTarget {
    /// A wikilink by name: a relative, flattened [`VaultPath`] of one or more
    /// components with the note extension applied (`dup.md`,
    /// `work/dup.md`). Matches every note whose path ends with these
    /// components ([`VaultPath::ends_with`]); several matches are an
    /// ambiguous link.
    Name(VaultPath),
    /// An exact note path, canonical (flattened, vault-absolute): markdown
    /// links, relative ones resolved against the linking note's folder, and
    /// wikilinks written from the root (`[[/work/dup]]`) or the note's
    /// folder (`[[./dup]]`, `[[../x/dup]]`).
    Path(VaultPath),
}

impl NoteTarget {
    /// Reads a wikilink target written in `from_note`. A `#section` /
    /// `^block` is cut (`split_link_fragment`, idempotent on a target
    /// already cut). A leading separator makes it an exact vault path, a
    /// leading `.`/`..` an exact path relative to `from_note`'s folder;
    /// anything else is a [`NoteTarget::Name`]. `None` for a target that is
    /// no note link: empty (`[[#tag]]`), not a valid vault path, or naming a
    /// non-note file (`![[pic.png]]`, `[[doc.pdf]]`).
    ///
    /// ```
    /// use kimun_core::{nfs::VaultPath, note::NoteTarget};
    /// let at = VaultPath::new("/journal/today.md");
    /// assert_eq!(
    ///     NoteTarget::from_wikilink("Work/Dup#Goals", &at),
    ///     Some(NoteTarget::Name(VaultPath::new("work/dup.md")))
    /// );
    /// assert_eq!(
    ///     NoteTarget::from_wikilink("../work/dup", &at),
    ///     Some(NoteTarget::Path(VaultPath::new("/work/dup.md")))
    /// );
    /// assert_eq!(NoteTarget::from_wikilink("pic.png", &at), None);
    /// ```
    pub fn from_wikilink(target: &str, from_note: &VaultPath) -> Option<Self> {
        let (note, _) = split_link_fragment(target);
        if note.is_empty() || !VaultPath::is_valid(note) || names_a_non_note_file(note) {
            return None;
        }
        let path = VaultPath::note_path_from(note);
        if path.is_root_or_empty() {
            return None;
        }
        if path.is_absolute() {
            Some(Self::Path(path.flatten()))
        } else if path.is_dot_relative() {
            Some(Self::Path(resolve_in_folder_of(&path, from_note)))
        } else {
            let name = path.flatten();
            (!name.is_root_or_empty()).then_some(Self::Name(name))
        }
    }

    /// Reads a markdown link destination written in `from_note`: always an
    /// exact path, a relative one (a bare `dup.md` included) resolved
    /// against `from_note`'s folder, the result canonical. A `#section` is
    /// cut, a `<…>` wrapper unwrapped. `None` for a URL, a destination that is not a valid vault path,
    /// or one that is not a note (an attachment or image: `doc.pdf`).
    ///
    /// ```
    /// use kimun_core::{nfs::VaultPath, note::NoteTarget};
    /// let at = VaultPath::new("/journal/today.md");
    /// assert_eq!(
    ///     NoteTarget::from_markdown("dup.md#s", &at),
    ///     Some(NoteTarget::Path(VaultPath::new("/journal/dup.md")))
    /// );
    /// assert_eq!(NoteTarget::from_markdown("doc.pdf", &at), None);
    /// ```
    pub fn from_markdown(dest: &str, from_note: &VaultPath) -> Option<Self> {
        let path = markdown_path(dest)?;
        path.is_note()
            .then(|| Self::Path(resolve_in_folder_of(&path, from_note)))
    }

    /// Reads a link span the editor found (`link_char_spans`) in
    /// `from_note`: a wikilink as [`Self::from_wikilink`], a markdown link
    /// as [`Self::from_markdown`]; `None` for an image.
    pub fn from_span(span: &LinkSpan, from_note: &VaultPath) -> Option<Self> {
        match span.kind {
            LinkSpanKind::WikiLink => Self::from_wikilink(&span.target, from_note),
            LinkSpanKind::Markdown => Self::from_markdown(&span.target, from_note),
            LinkSpanKind::Image => None,
        }
    }

    /// Whether this link points at the note at `note` (any form of its
    /// path): a name when `note`'s components end with the name's, a path
    /// when it is `note`'s canonical path. THE predicate; the index's SQL
    /// twin is `link_points_at_sql`.
    ///
    /// ```
    /// use kimun_core::{nfs::VaultPath, note::NoteTarget};
    /// let name = NoteTarget::Name(VaultPath::new("work/dup.md"));
    /// assert!(name.matches(&VaultPath::new("/a/work/dup.md")));
    /// assert!(!name.matches(&VaultPath::new("/notwork/dup.md")));
    /// ```
    pub fn matches(&self, note: &VaultPath) -> bool {
        match self {
            Self::Name(suffix) => note.canonical().ends_with(suffix),
            Self::Path(path) => note.canonical() == path.canonical(),
        }
    }

    /// The last component of the target — the note's file name, extension
    /// included — which every note it can match carries.
    pub fn name(&self) -> String {
        self.path().get_name()
    }

    /// The name suffix ([`NoteTarget::Name`], relative) or the canonical
    /// path ([`NoteTarget::Path`], absolute).
    pub fn path(&self) -> &VaultPath {
        match self {
            Self::Name(path) | Self::Path(path) => path,
        }
    }

    /// Whether this is a [`NoteTarget::Name`].
    pub fn is_name(&self) -> bool {
        matches!(self, Self::Name(_))
    }
}

/// The shortest wikilink target naming `note` and no note in `others`: the
/// fewest trailing components of `note`, at least `min_parts` (a rename keeps
/// the number the link was written with; all of them when it has fewer),
/// without the note extension
/// (`dup`, `work/dup`). When even the whole relative path is a suffix of
/// another note (`work/dup` of `/a/work/dup.md`), the vault-absolute path
/// (`/work/dup`). `others` is every other note that could share a suffix
/// with `note` — the notes named like it — and `note` itself in it is
/// ignored. Pure: the caller brings the index facts.
pub(crate) fn unique_wikilink(note: &VaultPath, min_parts: usize, others: &[VaultPath]) -> String {
    let note = note.canonical();
    let others: Vec<VaultPath> = others
        .iter()
        .map(VaultPath::canonical)
        .filter(|other| *other != note)
        .collect();
    let parts = note.get_slices().len();
    (min_parts.clamp(1, parts.max(1))..=parts)
        .map(|n| note.tail(n))
        .find(|suffix| !others.iter().any(|other| other.ends_with(suffix)))
        .map_or_else(|| note.to_bare_string(), |suffix| suffix.to_bare_string())
}

/// The vault file a markdown link destination written in `from_note` points
/// at when it is no note (an attachment, an image): resolved against
/// `from_note`'s folder like [`NoteTarget::from_markdown`], canonical. A
/// `#fragment` is cut, a `<…>` wrapper unwrapped. `None` for a URL, a
/// destination that is not a valid vault path, or a note (that is a
/// [`NoteTarget`]).
///
/// ```
/// use kimun_core::{nfs::VaultPath, note::markdown_attachment};
/// let at = VaultPath::new("/journal/today.md");
/// assert_eq!(
///     markdown_attachment("../files/doc.pdf", &at),
///     Some(VaultPath::new("/files/doc.pdf"))
/// );
/// assert_eq!(markdown_attachment("dup.md", &at), None);
/// ```
pub fn markdown_attachment(dest: &str, from_note: &VaultPath) -> Option<VaultPath> {
    let path = markdown_path(dest)?;
    (!path.is_note()).then(|| resolve_in_folder_of(&path, from_note))
}

/// A markdown destination as a vault path, as written: `<…>` unwrapped, the
/// `#fragment` cut. `None` for a URL, an empty or invalid path.
fn markdown_path(dest: &str) -> Option<VaultPath> {
    let dest = dest.trim();
    let dest = dest
        .strip_prefix('<')
        .and_then(|d| d.strip_suffix('>'))
        .unwrap_or(dest);
    if is_remote_url(dest) {
        return None;
    }
    let (written, _) = split_link_fragment(dest);
    (!written.is_empty() && VaultPath::is_valid(written)).then(|| VaultPath::new(written))
}

/// `path` written in the note at `from_note`, resolved against that note's
/// folder (an absolute `path` as is), canonical.
fn resolve_in_folder_of(path: &VaultPath, from_note: &VaultPath) -> VaultPath {
    let (folder, _) = from_note.canonical().get_parent_path();
    folder.append(path).canonical()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::NoteTarget;
    use crate::nfs::VaultPath;
    use crate::note::scan::{LinkSpan, LinkSpanKind};

    fn name(p: &str) -> Option<NoteTarget> {
        Some(NoteTarget::Name(VaultPath::new(p)))
    }

    fn path(p: &str) -> Option<NoteTarget> {
        Some(NoteTarget::Path(VaultPath::new(p)))
    }

    #[test]
    fn a_wikilink_reads_as_a_name_or_an_exact_path() {
        let at = VaultPath::new("/journal/today.md");
        for (written, expected) in [
            ("dup", name("dup.md")),
            ("Dup.md", name("dup.md")),
            ("work/dup", name("work/dup.md")),
            ("WORK/DUP", name("work/dup.md")),
            ("/work/dup", path("/work/dup.md")),
            ("./dup", path("/journal/dup.md")),
            ("../x/dup", path("/x/dup.md")),
            ("dup#Sec", name("dup.md")),
            ("dup^blk", name("dup.md")),
            (" spaced ", name("spaced.md")),
            ("v1.2", name("v1.2.md")),
            ("#tag", None),
            ("", None),
            ("pic.png", None),
            ("sub/Pic.PNG", None),
            ("doc.pdf", None),
            ("doc.pdf#page", None),
            ("bad?name", None),
            ("a|b", None),
        ] {
            assert_eq!(
                NoteTarget::from_wikilink(written, &at),
                expected,
                "{written:?}"
            );
        }
    }

    #[test]
    fn a_unique_wikilink_is_the_shortest_unshared_suffix() {
        let p = VaultPath::new;
        let note = p("/a/work/dup.md");
        for (min, others, expected) in [
            (1, vec![], "dup"),
            (2, vec![], "work/dup"),
            (9, vec![], "a/work/dup"),
            (1, vec![p("/dup.md")], "work/dup"),
            (1, vec![p("/dup.md"), p("/work/dup.md")], "a/work/dup"),
            (1, vec![p("/notwork/dup.md")], "work/dup"),
            (1, vec![p("/x/a/work/dup.md")], "/a/work/dup"),
            // The note itself among the others is ignored.
            (1, vec![p("a/work/dup.md")], "dup"),
            (1, vec![p("/other.md")], "dup"),
        ] {
            assert_eq!(
                super::unique_wikilink(&note, min, &others),
                expected,
                "{min} {others:?}"
            );
        }
    }

    #[test]
    fn a_dot_relative_wikilink_resolves_against_a_relative_note_canonically() {
        let at = VaultPath::new("journal/today.md");
        assert_eq!(
            NoteTarget::from_wikilink("./dup", &at),
            path("/journal/dup.md")
        );
    }

    #[test]
    fn a_markdown_destination_is_an_exact_canonical_path() {
        let at = VaultPath::new("/journal/today.md");
        for (written, expected) in [
            ("work/dup.md", path("/journal/work/dup.md")),
            ("../work/dup.md", path("/work/dup.md")),
            ("../work/dup.md#s", path("/work/dup.md")),
            ("/work/dup.md", path("/work/dup.md")),
            ("dup.md", path("/journal/dup.md")),
            ("./dup.md", path("/journal/dup.md")),
            ("Dup.MD", path("/journal/dup.md")),
            ("https://x.y/a.md", None),
            ("doc.pdf", None),
            ("pic.png", None),
            ("dir", None),
            ("a|b.md", None),
            ("", None),
        ] {
            assert_eq!(
                NoteTarget::from_markdown(written, &at),
                expected,
                "{written:?}"
            );
        }
    }

    #[test]
    fn a_markdown_destination_to_a_non_note_is_an_attachment_path() {
        let at = VaultPath::new("/journal/today.md");
        let file = |p: &str| Some(VaultPath::new(p));
        for (written, expected) in [
            ("doc.pdf", file("/journal/doc.pdf")),
            ("../files/doc.pdf#page=2", file("/files/doc.pdf")),
            ("/files/doc.pdf", file("/files/doc.pdf")),
            ("<my doc.pdf>", file("/journal/my doc.pdf")),
            ("img/pic.png", file("/journal/img/pic.png")),
            ("dup.md", None),
            ("https://x.y/a.pdf", None),
            ("a|b.pdf", None),
            ("", None),
        ] {
            assert_eq!(
                super::markdown_attachment(written, &at),
                expected,
                "{written:?}"
            );
        }
    }

    #[test]
    fn a_wrapped_markdown_destination_is_unwrapped() {
        let at = VaultPath::new("/journal/today.md");
        assert_eq!(
            NoteTarget::from_markdown("<my dup.md>", &at),
            path("/journal/my dup.md")
        );
    }

    #[test]
    fn a_markdown_destination_in_a_relative_note_is_canonical() {
        for at in ["journal/today.md", "/journal/./today.md"] {
            assert_eq!(
                NoteTarget::from_markdown("dup.md", &VaultPath::new(at)),
                path("/journal/dup.md"),
                "{at}"
            );
        }
    }

    #[test]
    fn a_span_reads_by_its_kind() {
        let at = VaultPath::new("/journal/today.md");
        let span = |kind, target: &str| LinkSpan {
            start: 0,
            end: 0,
            kind,
            target: target.to_string(),
        };
        assert_eq!(
            NoteTarget::from_span(&span(LinkSpanKind::WikiLink, "dup"), &at),
            name("dup.md")
        );
        assert_eq!(
            NoteTarget::from_span(&span(LinkSpanKind::Markdown, "dup.md"), &at),
            path("/journal/dup.md")
        );
        assert_eq!(
            NoteTarget::from_span(&span(LinkSpanKind::Markdown, "<my dup.md>"), &at),
            path("/journal/my dup.md")
        );
        assert_eq!(
            NoteTarget::from_span(&span(LinkSpanKind::Image, "dup.md"), &at),
            None
        );
    }

    /// The case table [`NoteTarget::matches`] and the index's SQL predicate
    /// both answer (see `index::tests::link_predicate_agrees_with_note_target`).
    pub(crate) const MATCH_CASES: &[(&str, &str, bool)] = &[
        // (target as `name:` or `path:`, note, expected)
        ("name:dup.md", "/dup.md", true),
        ("name:dup.md", "/work/dup.md", true),
        ("name:dup.md", "/a/work/dup.md", true),
        ("name:dup.md", "/notwork/dup.md", true),
        ("name:dup.md", "/xdup.md", false),
        ("name:work/dup.md", "/dup.md", false),
        ("name:work/dup.md", "/work/dup.md", true),
        ("name:work/dup.md", "/a/work/dup.md", true),
        ("name:work/dup.md", "/notwork/dup.md", false),
        ("name:a/work/dup.md", "/work/dup.md", false),
        ("name:a/work/dup.md", "/a/work/dup.md", true),
        ("path:/work/dup.md", "/work/dup.md", true),
        ("path:/work/dup.md", "/a/work/dup.md", false),
        ("path:/work/dup.md", "/dup.md", false),
        ("path:/dup.md", "/dup.md", true),
        ("path:/journal/dup.md", "/dup.md", false),
    ];

    pub(crate) fn case_target(spec: &str) -> NoteTarget {
        match spec.split_once(':') {
            Some(("name", p)) => NoteTarget::Name(VaultPath::new(p)),
            Some(("path", p)) => NoteTarget::Path(VaultPath::new(p)),
            _ => panic!("bad case {spec}"),
        }
    }

    #[test]
    fn matches_answers_the_case_table() {
        for (target, note, expected) in MATCH_CASES {
            assert_eq!(
                case_target(target).matches(&VaultPath::new(note)),
                *expected,
                "{target} vs {note}"
            );
        }
    }

    #[test]
    fn matches_is_case_insensitive_and_form_agnostic_on_the_note() {
        let t = NoteTarget::Name(VaultPath::new("work/dup.md"));
        assert!(t.matches(&VaultPath::new("/Work/DUP.md")));
        assert!(t.matches(&VaultPath::new("work/dup.md")));
        let p = NoteTarget::Path(VaultPath::new("/work/dup.md"));
        assert!(p.matches(&VaultPath::new("work/dup.md")));
    }

    #[test]
    fn name_is_the_last_component() {
        assert_eq!(name("work/dup.md").unwrap().name(), "dup.md");
        assert_eq!(path("/a/b.md").unwrap().name(), "b.md");
    }
}
