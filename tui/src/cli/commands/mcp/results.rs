// tui/src/cli/commands/mcp/results.rs
//
// Shared pieces of the MCP tools that return notes: the hit renderer, the
// snippet cap, and the parsing of the date arguments.

use chrono::NaiveDate;
use kimun_core::note::{ContentChunk, LinkType};
use kimun_core::{DateSource, NoteDate, NoteVault, nfs::VaultPath};

/// Hits shown when the caller gives no `limit`.
pub(super) const DEFAULT_HITS: usize = 20;
/// The most hits a caller can ask for.
pub(super) const MAX_HITS: usize = 100;
/// Longest snippet, in characters.
pub(super) const SNIPPET_CHARS: usize = 240;

/// Everything `render_hit` prints about one note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct NoteHit {
    pub path: String,
    pub title: String,
    pub dates: Vec<NoteDate>,
    pub tags: Vec<String>,
    pub links_out: usize,
    pub links_in: usize,
    pub snippet: String,
}

/// Cut `text` to at most `max` characters on a character boundary. When it
/// cuts, the result ends with `…` (the ellipsis counts toward `max`).
pub(super) fn cap_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}

/// Render one hit. The first line is `path — title`. Indented lines follow
/// for dates, tags, links and snippet, each left out when empty.
pub(super) fn render_hit(hit: &NoteHit) -> String {
    let mut out = format!("{} — {}", hit.path, hit.title);
    if !hit.dates.is_empty() {
        let dates: Vec<String> = hit
            .dates
            .iter()
            .map(|d| format!("{} {}", d.source, d.day))
            .collect();
        out.push_str(&format!("\n  dates: {}", dates.join(", ")));
    }
    if !hit.tags.is_empty() {
        out.push_str(&format!("\n  tags: {}", hit.tags.join(", ")));
    }
    if hit.links_out > 0 || hit.links_in > 0 {
        out.push_str(&format!(
            "\n  links: {} out, {} in",
            hit.links_out, hit.links_in
        ));
    }
    if !hit.snippet.is_empty() {
        out.push_str(&format!("\n  snippet: {}", hit.snippet));
    }
    out
}

/// Build the hit for a note: its dates, tags, link counts and a snippet. When
/// the file is gone (an index entry that is stale because the file was
/// deleted outside Kimun), the hit has only its path and title.
pub(super) async fn build_hit(
    vault: &NoteVault,
    path: &VaultPath,
    title: &str,
) -> Result<NoteHit, kimun_core::error::VaultError> {
    match full_hit(vault, path, title).await {
        Err(e) if e.is_not_found() => Ok(NoteHit {
            path: path.to_string(),
            title: title.to_string(),
            ..Default::default()
        }),
        other => other,
    }
}

async fn full_hit(
    vault: &NoteVault,
    path: &VaultPath,
    title: &str,
) -> Result<NoteHit, kimun_core::error::VaultError> {
    let dates = vault.note_dates(path).await?;
    let tags = vault.get_tags(path).await?;
    let links_out = vault
        .get_markdown_and_links(path)
        .await?
        .links
        .iter()
        .filter(|l| matches!(l.ltype, LinkType::Note(_)))
        .count();
    let links_in = vault.get_backlinks(path).await?.len();
    let chunks = vault.get_note_chunks(path).await?;
    let snippet = chunks
        .get(path)
        .map(|c| snippet_from(c, title))
        .unwrap_or_default();
    Ok(NoteHit {
        path: path.to_string(),
        title: title.to_string(),
        dates,
        tags,
        links_out,
        links_in,
        snippet,
    })
}

/// The start of the first chunk that has text after the title, on one line.
fn snippet_from(chunks: &[ContentChunk], title: &str) -> String {
    for chunk in chunks {
        let body = chunk.text.trim();
        let body = body
            .strip_prefix(title)
            .or_else(|| body.strip_prefix(&format!("# {title}")))
            .unwrap_or(body);
        let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
        if !flat.is_empty() {
            return cap_chars(&flat, SNIPPET_CHARS);
        }
    }
    String::new()
}

/// Map a `date_key` argument to a date source. `"journal"` is the journal
/// date, `"property:<key>"` is a property (the way to name a property that is
/// literally called journal), and anything else is a property key.
pub(super) fn parse_date_key(key: &str) -> DateSource {
    let key = key.trim();
    if key.eq_ignore_ascii_case("journal") {
        return DateSource::Journal;
    }
    if let Some(rest) = key.strip_prefix("property:") {
        return DateSource::Property(rest.to_string());
    }
    DateSource::Property(key.to_string())
}

/// Parse a `YYYY-MM-DD` argument, with an error that names the format.
pub(super) fn parse_day(name: &str, text: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d").map_err(|_| {
        format!(
            "Invalid date '{text}' for {name}: use the format YYYY-MM-DD, for example 2026-11-25"
        )
    })
}

/// A parsed date range: the date key as the caller wrote it, and both ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DateRange {
    pub key: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

/// Why a range could not be built.
pub(super) enum RangeError {
    /// A message for the caller, to return as a tool error.
    Tool(String),
    /// The vault failed while listing the date keys.
    Vault(kimun_core::error::VaultError),
}

/// The argument with its spaces trimmed, or `None` when it is blank. Some
/// agent apps fill an optional field with `""`, which means not set.
pub(super) fn present(arg: Option<&str>) -> Option<&str> {
    arg.map(str::trim).filter(|a| !a.is_empty())
}

/// Parse the `from`, `to` and `date_key` arguments shared by the search tools.
/// A blank argument counts as not set. Returns `None` when none of them is
/// set. `date_key` is required when `from` or `to` is set, and it must be a
/// date source of the vault. Both errors list the keys the vault has. An open
/// end becomes `0001-01-01` or `9999-12-31`.
pub(super) async fn resolve_range(
    vault: &NoteVault,
    from: Option<&str>,
    to: Option<&str>,
    date_key: Option<&str>,
) -> Result<Option<DateRange>, RangeError> {
    let (from, to, date_key) = (present(from), present(to), present(date_key));
    if from.is_none() && to.is_none() && date_key.is_none() {
        return Ok(None);
    }
    let Some(key) = date_key else {
        let keys = vault.date_keys().await.map_err(RangeError::Vault)?;
        let known = if keys.is_empty() {
            "none".to_string()
        } else {
            keys.join(", ")
        };
        return Err(RangeError::Tool(format!(
            "date_key is required with from or to. Date keys in this vault: {known}"
        )));
    };
    let from = match from.map(|t| parse_day("from", t)) {
        Some(Ok(d)) => d,
        Some(Err(m)) => return Err(RangeError::Tool(m)),
        None => NaiveDate::from_ymd_opt(1, 1, 1).expect("valid date"),
    };
    let to = match to.map(|t| parse_day("to", t)) {
        Some(Ok(d)) => d,
        Some(Err(m)) => return Err(RangeError::Tool(m)),
        None => NaiveDate::from_ymd_opt(9999, 12, 31).expect("valid date"),
    };
    if let Err(e) = vault.check_date_source(&parse_date_key(key)).await {
        return Err(match e.user_message() {
            Some(m) => RangeError::Tool(m),
            None => RangeError::Vault(e),
        });
    }
    Ok(Some(DateRange {
        key: key.to_string(),
        from,
        to,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_hit_first_line_keeps_path_dash_title() {
        let hit = NoteHit {
            path: "/a/b.md".into(),
            title: "Title".into(),
            dates: vec![
                NoteDate {
                    source: "journal".into(),
                    day: NaiveDate::from_ymd_opt(2026, 11, 20).unwrap(),
                },
                NoteDate {
                    source: "due".into(),
                    day: NaiveDate::from_ymd_opt(2026, 11, 25).unwrap(),
                },
            ],
            tags: vec!["a".into(), "b".into()],
            links_out: 3,
            links_in: 2,
            snippet: "hello".into(),
        };
        let text = render_hit(&hit);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "/a/b.md — Title");
        assert_eq!(lines[1], "  dates: journal 2026-11-20, due 2026-11-25");
        assert_eq!(lines[2], "  tags: a, b");
        assert_eq!(lines[3], "  links: 3 out, 2 in");
        assert_eq!(lines[4], "  snippet: hello");
    }

    #[test]
    fn render_hit_omits_empty_lines() {
        let hit = NoteHit {
            path: "/a.md".into(),
            title: "T".into(),
            ..Default::default()
        };
        assert_eq!(render_hit(&hit), "/a.md — T");
    }

    #[test]
    fn cap_chars_cuts_on_char_boundary() {
        assert_eq!(cap_chars("short", 10), "short");
        assert_eq!(cap_chars("éééééé", 4), "ééé…");
        assert_eq!(cap_chars("日本語のテキスト", 4), "日本語…");
        assert_eq!(cap_chars("😀😀😀😀😀", 3), "😀😀…");
        assert_eq!(cap_chars("abc", 3), "abc");
    }

    #[test]
    fn date_key_forms() {
        assert_eq!(parse_date_key("journal"), DateSource::Journal);
        assert_eq!(
            parse_date_key("property:journal"),
            DateSource::Property("journal".into())
        );
        assert_eq!(parse_date_key("due"), DateSource::Property("due".into()));
    }

    #[test]
    fn parse_day_names_the_format() {
        assert!(parse_day("from", "2026-11-25").is_ok());
        let err = parse_day("from", "25/11/2026").unwrap_err();
        assert!(err.contains("YYYY-MM-DD"), "{err}");
    }
}
