//! Inline image embeds: the lines of a note that embed a vault image on a
//! line of their own (`![alt](path)` or `![[name]]`), and the vault file each
//! one points at (see `CONTEXT.md`, **Inline image** and **Closest-file
//! rule**). Pure functions over note text and path lists: the caller brings
//! the attachment list ([`NoteVault::list_attachments`]).
//!
//! [`NoteVault::list_attachments`]: crate::NoteVault::list_attachments

use url::Url;

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

/// The longest address, in characters, that counts as a remote embed.
const MAX_REMOTE_URL_LEN: usize = 2048;

/// A line of a note that is nothing but a remote image embed
/// (`![alt](https://...)`). The address needs no image extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteImageEmbed {
    /// Zero-based line of the note, counted as an editor counts lines.
    pub row: usize,
    /// The alt text of `![alt](url)`.
    pub alt: String,
    /// The title after the address, without its quotes.
    pub title: Option<String>,
    /// The address as written, without angle brackets or title. It keeps
    /// the query.
    pub url: String,
    /// The host in lower case, in ASCII form, without the port.
    pub host: String,
    /// Whether the address uses `https`.
    pub secure: bool,
}

/// An embed found on a line of its own, vault or remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoundEmbed {
    Vault(ImageEmbed),
    Remote(RemoteImageEmbed),
}

/// The vault embeds and the remote embeds that fill a whole line of `text`,
/// in row order. The same zone rules as [`image_embeds`] apply.
pub fn all_image_embeds(text: &str) -> Vec<FoundEmbed> {
    let zones = ExclusionZones::from_text(text);
    let mut found = Vec::new();
    let mut line_start = 0;
    for (row, raw) in text.split('\n').enumerate() {
        let start = line_start;
        line_start += raw.len() + 1;
        let line = raw.trim();
        if !line.starts_with('!') || zones.contains_code_link_or_frontmatter(start) {
            continue;
        }
        // Remote first: the vault finder cannot tell an address with a
        // space (`<https://x.com/a b.png>`) from a vault path.
        if let Some(embed) = whole_line_remote_embed(row, line) {
            found.push(FoundEmbed::Remote(embed));
        } else if let Some(embed) = whole_line_embed(row, line) {
            found.push(FoundEmbed::Vault(embed));
        }
    }
    found
}

/// The remote embed `line` (trimmed, one line) is, when it is nothing else.
fn whole_line_remote_embed(row: usize, line: &str) -> Option<RemoteImageEmbed> {
    let rest = line.strip_prefix("![")?;
    let alt_end = rest.find("](")?;
    let alt = rest[..alt_end].trim().to_string();
    let rest = &rest[alt_end + 2..];
    let (url, rest) = if let Some(bracketed) = rest.strip_prefix('<') {
        let end = bracketed.find('>')?;
        (&bracketed[..end], &bracketed[end + 1..])
    } else {
        let mut depth = 0usize;
        let mut end = rest.len();
        for (i, c) in rest.char_indices() {
            match c {
                c if c.is_whitespace() => {
                    end = i;
                    break;
                }
                '(' => depth += 1,
                ')' if depth == 0 => {
                    end = i;
                    break;
                }
                ')' => depth -= 1,
                _ => {}
            }
        }
        (&rest[..end], &rest[end..])
    };
    let rest = rest.trim_start();
    let title = if rest == ")" {
        None
    } else {
        let quoted = rest.strip_prefix('"')?.strip_suffix(')')?.trim_end();
        Some(quoted.strip_suffix('"')?.to_string())
    };
    if url.chars().count() > MAX_REMOTE_URL_LEN {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_string();
    let secure = match parsed.scheme() {
        "https" => true,
        "http" => false,
        _ => return None,
    };
    Some(RemoteImageEmbed {
        row,
        alt,
        title,
        url: url.to_string(),
        host,
        secure,
    })
}

/// The cache key text of a remote address: the fragment is removed, and the
/// scheme and host are in lower case. `None` for an address that is not
/// `http` or `https`, or that has no host.
pub fn normalize_image_url(url: &str) -> Option<String> {
    let mut parsed = Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    parsed.host_str()?;
    parsed.set_fragment(None);
    Some(parsed.to_string())
}

/// The host of `url`, in lower case and ASCII form, without the port.
pub fn host_of(url: &str) -> Option<String> {
    Url::parse(url.trim()).ok()?.host_str().map(str::to_string)
}

/// Whether `host` is the host `allowed` names. The match is exact: a parent
/// domain does not match a subdomain. Case, port and the form of the name
/// (Unicode or ASCII) are ignored.
pub fn host_matches(allowed: &str, host: &str) -> bool {
    let allowed = allowed.trim();
    if allowed.is_empty() {
        return false;
    }
    match (
        host_of(&format!("https://{allowed}")),
        host_of(&format!("https://{}", host.trim())),
    ) {
        (Some(a), Some(h)) => a == h,
        _ => false,
    }
}

/// The hosts a reader allowed remote images from: plain names in lower case
/// and ASCII form, without a port, no duplicates, in the order added. One
/// rule for the settings, the store of remote images and a redirect:
/// [`host_matches`]. The settings keep the names as a plain list in their
/// file, so the list operations also work on a bare `Vec<String>`
/// ([`allow_host`], [`forget_host`], [`host_listed`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowedHosts(Vec<String>);

/// The stored name of `host` as written (a name, with or without a port,
/// any case): `None` for a blank or invalid one.
pub fn normalize_host(host: &str) -> Option<String> {
    host_of(&format!("https://{}", host.trim()))
}

/// Whether `host` is on `list`.
pub fn host_listed(list: &[String], host: &str) -> bool {
    list.iter().any(|allowed| host_matches(allowed, host))
}

/// Add `host`, normalized, to `list`. Returns true when the list changed:
/// a blank or invalid host, or one already there, changes nothing.
pub fn allow_host(list: &mut Vec<String>, host: &str) -> bool {
    let Some(name) = normalize_host(host) else {
        return false;
    };
    if list.contains(&name) {
        return false;
    }
    list.push(name);
    true
}

/// Remove `host` from `list`. Returns true when the list changed.
pub fn forget_host(list: &mut Vec<String>, host: &str) -> bool {
    let before = list.len();
    list.retain(|allowed| !host_matches(allowed, host));
    list.len() != before
}

impl AllowedHosts {
    /// Whether `host` is on the list.
    pub fn matches(&self, host: &str) -> bool {
        host_listed(&self.0, host)
    }

    /// Add `host`; see [`allow_host`].
    pub fn allow(&mut self, host: &str) -> bool {
        allow_host(&mut self.0, host)
    }

    /// Remove `host`; see [`forget_host`].
    pub fn forget(&mut self, host: &str) -> bool {
        forget_host(&mut self.0, host)
    }

    /// The names, in the order added.
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

impl std::ops::Deref for AllowedHosts {
    type Target = [String];

    fn deref(&self) -> &[String] {
        &self.0
    }
}

/// The names as given, kept as they are: for hosts already normalized.
impl FromIterator<String> for AllowedHosts {
    fn from_iter<I: IntoIterator<Item = String>>(names: I) -> Self {
        Self(names.into_iter().collect())
    }
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

    fn remotes(text: &str) -> Vec<RemoteImageEmbed> {
        all_image_embeds(text)
            .into_iter()
            .filter_map(|e| match e {
                FoundEmbed::Remote(r) => Some(r),
                FoundEmbed::Vault(_) => None,
            })
            .collect()
    }

    #[test]
    fn a_remote_markdown_image_line_is_a_remote_embed() {
        let r = remotes("![a](https://x.com/p.png)");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].alt, "a");
        assert_eq!(r[0].host, "x.com");
        assert!(r[0].secure);
        assert_eq!(r[0].row, 0);
        assert_eq!(r[0].title, None);
    }

    #[test]
    fn a_remote_embed_needs_no_image_extension() {
        assert_eq!(remotes("![](https://x.com/img?id=3)").len(), 1);
        assert_eq!(remotes("![](https://x.com/avatar/123)").len(), 1);
    }

    #[test]
    fn a_query_string_and_a_port_stay_in_the_url_and_leave_the_host() {
        let r = remotes("![a](https://X.com:8080/p?w=800&q=80)");
        assert_eq!(r[0].host, "x.com");
        assert!(r[0].url.contains("?w=800&q=80"));
    }

    #[test]
    fn balanced_parentheses_are_part_of_the_address() {
        let r = remotes("![a](https://x.com/Mars_(planet).png)");
        assert_eq!(r.len(), 1);
        assert!(r[0].url.ends_with(".png"));
        assert_eq!(r[0].url, "https://x.com/Mars_(planet).png");
    }

    #[test]
    fn an_address_in_angle_brackets_may_hold_a_space() {
        let r = remotes("![a](<https://x.com/a b.png>)");
        assert_eq!(r[0].url, "https://x.com/a b.png");
    }

    #[test]
    fn a_title_after_the_address_is_kept_apart() {
        let r = remotes("![a](https://x.com/a.png \"the title\")");
        assert_eq!(r[0].title.as_deref(), Some("the title"));
        assert_eq!(r[0].url, "https://x.com/a.png");
    }

    #[test]
    fn an_http_address_is_found_and_not_secure() {
        let r = remotes("![a](http://x.com/a.png)");
        assert_eq!(r.len(), 1);
        assert!(!r[0].secure);
    }

    #[test]
    fn a_wikilink_with_an_address_is_not_an_embed() {
        assert!(remotes("![[https://x.com/a.png]]").is_empty());
    }

    #[test]
    fn an_address_over_2048_characters_is_not_an_embed() {
        let long = format!("![a](https://x.com/{})", "a".repeat(2048));
        assert!(remotes(&long).is_empty());
    }

    #[test]
    fn remote_embeds_follow_the_zone_and_line_rules() {
        assert!(remotes("```\n![a](https://x.com/a.png)\n```").is_empty());
        assert!(remotes("---\ncover: x\n![a](https://x.com/a.png)\n---\nbody").is_empty());
        assert!(remotes("see ![a](https://x.com/a.png) here").is_empty());
        assert!(remotes("![a](https://x.com/a.png) here").is_empty());
        assert!(remotes("![a](https://x.com/a.png)![b](https://x.com/b.png)").is_empty());
        let rows: Vec<usize> =
            remotes("title\n\n  ![a](https://x.com/a.png)  \r\n![](https://x.com/b)")
                .iter()
                .map(|r| r.row)
                .collect();
        assert_eq!(rows, vec![2, 3]);
    }

    #[test]
    fn all_image_embeds_merges_in_row_order() {
        let found = all_image_embeds("![a](one.png)\n![b](https://x.com/b.png)\n![[two.png]]");
        assert_eq!(found.len(), 3);
        assert!(matches!(&found[0], FoundEmbed::Vault(e) if e.row == 0));
        assert!(matches!(&found[1], FoundEmbed::Remote(e) if e.row == 1));
        assert!(matches!(&found[2], FoundEmbed::Vault(e) if e.row == 2));
    }

    #[test]
    fn image_embeds_still_skips_remote_addresses() {
        assert!(image_embeds("![a](https://x.com/img?id=3)").is_empty());
    }

    #[test]
    fn normalize_image_url_drops_the_fragment_and_lowers_scheme_and_host_only() {
        assert_eq!(
            normalize_image_url("HTTPS://X.com/Path#top").as_deref(),
            Some("https://x.com/Path")
        );
    }

    #[test]
    fn normalize_image_url_rejects_other_schemes() {
        assert_eq!(normalize_image_url("ftp://x.com/a.png"), None);
        assert_eq!(normalize_image_url("file:///etc/a.png"), None);
        assert_eq!(normalize_image_url("javascript:alert(1)"), None);
        assert_eq!(normalize_image_url("https://"), None);
        assert_eq!(normalize_image_url("a.png"), None);
    }

    #[test]
    fn host_of_ignores_case_and_port_and_uses_the_ascii_form() {
        assert_eq!(
            host_of("https://Bücher.example:8443/a").as_deref(),
            Some("xn--bcher-kva.example")
        );
        assert_eq!(host_of("https://X.com:80/a").as_deref(), Some("x.com"));
        assert_eq!(host_of("nonsense"), None);
    }

    #[test]
    fn allowed_hosts_keeps_a_plain_normalized_list() {
        let mut hosts = AllowedHosts::default();
        assert!(hosts.allow("Example.COM:8080"));
        assert!(!hosts.allow("example.com"));
        assert!(!hosts.allow("  "));
        assert!(!hosts.allow("not a host"));
        assert_eq!(hosts.as_slice(), ["example.com"]);
        assert!(hosts.matches("EXAMPLE.com:443"));
        assert!(!hosts.matches("cdn.example.com"));
        assert!(!hosts.forget(""));
        assert!(hosts.forget("EXAMPLE.com"));
        assert!(hosts.as_slice().is_empty());
        assert_eq!(normalize_host(" X.com:1 "), Some("x.com".to_string()));
    }

    #[test]
    fn host_matches_is_exact_and_ignores_case_and_port() {
        assert!(host_matches("X.com", "x.com"));
        assert!(host_matches("x.com:8080", "x.com"));
        assert!(host_matches("bücher.example", "xn--bcher-kva.example"));
        assert!(!host_matches("x.com", "img.x.com"));
        assert!(!host_matches("img.x.com", "x.com"));
        assert!(!host_matches("", "x.com"));
    }
}
