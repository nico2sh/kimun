//! Frontmatter properties: the typed key/value pairs in a note's leading
//! `+++` (TOML) or `---` (YAML) block.
//!
//! Composition, one contract: [`NoteProperties`] is the only entry point. It
//! locates the block, picks the [`PropertyFormatter`] for its syntax, and
//! delegates every read and edit to it; formatters only ever see the block's
//! contents. Reading is lenient (a malformed block yields no properties);
//! editing is strict (a malformed block refuses the edit).

mod toml_formatter;
mod yaml_formatter;

use std::collections::HashSet;
use std::ops::Range;

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};

use super::content_extractor::frontmatter_delimiter;
use toml_formatter::TomlFormatter;
use yaml_formatter::YamlFormatter;

/// A typed frontmatter property value — the neutral model every
/// `PropertyFormatter` parses into and writes from. Mirrors Obsidian's
/// property types.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum PropertyValue {
    /// Free text.
    Text(String),
    /// Any number. Integers beyond 2^53 lose exactness (as in Obsidian).
    Number(f64),
    /// `true` / `false`.
    Bool(bool),
    /// A calendar date with no time component.
    Date(NaiveDate),
    /// A date and time; offset-less sources are read as UTC.
    DateTime(DateTime<Utc>),
    /// A list of text items (Obsidian's List type); non-text items are
    /// stringified.
    List(Vec<String>),
}

/// The syntax of a frontmatter block, chosen by its delimiter: `+++` is TOML,
/// `---` is YAML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrontmatterFormat {
    /// `+++`-delimited TOML — the format new blocks use unless asked otherwise.
    #[default]
    Toml,
    /// `---`-delimited YAML, as written by Obsidian.
    Yaml,
}

impl FrontmatterFormat {
    /// The formatter that reads and edits this syntax.
    pub(crate) fn formatter(self) -> &'static dyn PropertyFormatter {
        match self {
            FrontmatterFormat::Toml => &TomlFormatter,
            FrontmatterFormat::Yaml => &YamlFormatter,
        }
    }

    /// The block's opening and closing line.
    pub(crate) fn delimiter(self) -> &'static str {
        match self {
            FrontmatterFormat::Toml => "+++",
            FrontmatterFormat::Yaml => "---",
        }
    }
}

/// Why a frontmatter read or edit was refused; the message is user-facing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrontmatterError(pub(crate) String);

/// One top-level frontmatter entry: its key (lowercased) and its value, or
/// `None` when the note has the key without a usable value (YAML `key:`, a
/// non-finite number, a TOML local time).
pub(crate) type PropertyEntry = (String, Option<PropertyValue>);

/// What a note's frontmatter declares: its property entries in file order,
/// the first of any case-duplicate keys winning. The one shape the index and
/// the API read properties through — keys (for "has property"), typed values
/// and the `tags` labels all come from here.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PropertySet {
    entries: Vec<PropertyEntry>,
}

impl PropertySet {
    fn from_entries(entries: Vec<PropertyEntry>) -> Self {
        let mut seen = HashSet::new();
        Self {
            entries: entries
                .into_iter()
                .filter(|(k, _)| seen.insert(k.clone()))
                .collect(),
        }
    }

    /// Every key the note has, valued or not.
    pub(crate) fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    /// The properties that have a usable value.
    pub(crate) fn values(&self) -> impl Iterator<Item = (&str, &PropertyValue)> {
        self.entries
            .iter()
            .filter_map(|(k, v)| Some((k.as_str(), v.as_ref()?)))
    }

    /// [`Self::values`], owned.
    pub(crate) fn into_values(self) -> Vec<(String, PropertyValue)> {
        self.entries
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect()
    }

    /// Label names from the `tags` property (and Obsidian's legacy singular
    /// `tag`): a list gives one tag per item, a bare string is one tag (no
    /// comma splitting, as in Obsidian). Trimmed, a leading `#` dropped,
    /// lowercased, empties removed.
    pub(crate) fn tags(&self) -> Vec<String> {
        self.values()
            .filter(|(k, _)| TAG_KEYS.contains(k))
            .flat_map(|(_, v)| match v {
                PropertyValue::List(items) => items.iter().map(String::as_str).collect(),
                PropertyValue::Text(s) => vec![s.as_str()],
                _ => Vec::new(),
            })
            .map(|t| t.trim().trim_start_matches('#').to_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    }
}

/// The single contract for one frontmatter syntax. A formatter sees only the
/// block's contents (between the delimiter lines, LF endings, empty or ending
/// in `\n`); locating the block, choosing the formatter, leniency and splicing
/// are [`NoteProperties`]'s job. Every implementation is held to the
/// `contract_*` tests in this module:
///
/// - `parse` is strict: a malformed block or a non-mapping root is `Err`; an
///   empty block is `Ok(vec![])`. Entries come back with keys lowercased, in
///   file order. A key whose value doesn't map onto [`PropertyValue`] (null,
///   non-finite number, TOML local time) is still an entry, with no value — the
///   note visibly has that property. A nested table/mapping is not a property
///   and yields no entry.
/// - `set` / `remove` match keys case-insensitively, keep the spelling and
///   position of the first match, collapse case-duplicates, and leave every
///   other line of the block (comments, order, untouched entries) as it was.
///   The comment lines directly above an entry belong to it: removing the
///   entry (or collapsing it as a duplicate) removes them too. A comment
///   separated from the entry by a blank line does not, and stays.
///   They refuse a malformed block and a key holding a nested table/mapping.
/// - `set` returns a block that is empty or ends in `\n`; given a finite
///   number (callers validate that), whatever it writes `parse` reads back
///   equal. One exception: YAML has no date tag, so `Text` that is exactly an
///   ISO date / date-time reads back as `Date` / `DateTime` (Obsidian behaves
///   the same).
/// - `remove` returns `Ok(None)` when the key is absent; removing the last
///   entry yields an empty block.
pub(crate) trait PropertyFormatter: Send + Sync {
    /// Every property entry in `block`.
    fn parse(&self, block: &str) -> Result<Vec<PropertyEntry>, FrontmatterError>;
    /// `block` with `key` set to `value`.
    fn set(
        &self,
        block: &str,
        key: &str,
        value: &PropertyValue,
    ) -> Result<String, FrontmatterError>;
    /// `block` without `key`, or `None` when it had no such key.
    fn remove(&self, block: &str, key: &str) -> Result<Option<String>, FrontmatterError>;
}

/// Where a note's frontmatter block sits in its text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FrontmatterSpan {
    format: FrontmatterFormat,
    /// Byte range of the block's contents, between the delimiter lines.
    inner: Range<usize>,
}

/// Finds the leading frontmatter block, with the same rules as the indexer's
/// `remove_frontmatter`: the first line is exactly `---` or `+++` and a later
/// line is exactly the same delimiter (CRLF tolerated).
fn locate_frontmatter(text: &str) -> Option<FrontmatterSpan> {
    let (delimiter, start) = frontmatter_delimiter(text)?;
    let format = if delimiter == FrontmatterFormat::Toml.delimiter() {
        FrontmatterFormat::Toml
    } else {
        FrontmatterFormat::Yaml
    };
    let mut offset = start;
    for line in text[start..].split_inclusive('\n') {
        if line.trim_end_matches('\n').trim_end_matches('\r') == delimiter {
            return Some(FrontmatterSpan {
                format,
                inner: start..offset,
            });
        }
        offset += line.len();
    }
    None
}

/// A note's properties — the one door for reading and editing them, whatever
/// syntax the note uses. Composes the note's text with the
/// [`PropertyFormatter`] for its block: the existing block's (by delimiter),
/// or `new_block_format`'s when the note has none yet. Edits take LF text and
/// return the whole new note text.
pub(crate) struct NoteProperties<'t> {
    text: &'t str,
    span: Option<FrontmatterSpan>,
    format: FrontmatterFormat,
    formatter: &'static dyn PropertyFormatter,
}

impl<'t> NoteProperties<'t> {
    /// Properties of `text`. `new_block_format` only matters if `text` has no
    /// block yet and an edit creates one; an existing block is never
    /// converted.
    pub(crate) fn new(text: &'t str, new_block_format: FrontmatterFormat) -> Self {
        let span = locate_frontmatter(text);
        let format = span.as_ref().map_or(new_block_format, |s| s.format);
        Self {
            text,
            span,
            format,
            formatter: format.formatter(),
        }
    }

    #[cfg(test)]
    /// The syntax reads and edits go through.
    pub(crate) fn format(&self) -> FrontmatterFormat {
        self.format
    }

    fn block(&self) -> &'t str {
        self.span
            .as_ref()
            .map_or("", |s| &self.text[s.inner.clone()])
    }

    /// Everything the block declares; see [`PropertySet`]. Lenient: a
    /// malformed block yields an empty set.
    pub(crate) fn entries(&self) -> PropertySet {
        if self.span.is_none() {
            return PropertySet::default();
        }
        PropertySet::from_entries(self.formatter.parse(self.block()).unwrap_or_default())
    }

    /// The note's text with `key` (already normalized) set to `value`.
    /// Strict: a malformed existing block is an error, never guessed at.
    pub(crate) fn set(&self, key: &str, value: &PropertyValue) -> Result<String, FrontmatterError> {
        let block = self.formatter.set(self.block(), key, value)?;
        Ok(self.with_block(&block))
    }

    /// The note's text without `key`, or `None` when there is nothing to
    /// remove (no block, or no such key).
    pub(crate) fn remove(&self, key: &str) -> Result<Option<String>, FrontmatterError> {
        if self.span.is_none() {
            return Ok(None);
        }
        Ok(self
            .formatter
            .remove(self.block(), key)?
            .map(|block| self.with_block(&block)))
    }

    /// The note's text with its block contents replaced by `block`, or with a
    /// new block prepended when it had none.
    fn with_block(&self, block: &str) -> String {
        match &self.span {
            Some(s) => format!(
                "{}{}{}",
                &self.text[..s.inner.start],
                block,
                &self.text[s.inner.end..]
            ),
            None => {
                let d = self.format.delimiter();
                format!("{d}\n{block}{d}\n{}", self.text)
            }
        }
    }
}

/// A property key as stored: trimmed and lowercased. `None` when empty or
/// containing control characters (a line break would corrupt the block).
pub(crate) fn normalize_key(key: &str) -> Option<String> {
    let key = key.trim();
    (!key.is_empty() && !key.chars().any(char::is_control)).then(|| key.to_lowercase())
}

/// The frontmatter keys whose items are unified into the label index:
/// `tags`, and the singular `tag` older Obsidian notes use.
const TAG_KEYS: [&str; 2] = ["tags", "tag"];

// ---- Helpers shared by the formatters ------------------------------------

/// Largest integer an `f64` represents exactly (2^53).
const MAX_EXACT_INT: f64 = 9_007_199_254_740_992.0;

fn is_integral(n: f64) -> bool {
    n.fract() == 0.0 && n.abs() < MAX_EXACT_INT
}

fn finite(n: f64) -> Option<f64> {
    n.is_finite().then_some(n)
}

/// Canonical text of a number: integral values without a fraction (`5`, not
/// `5.0`).
pub(crate) fn format_number(n: f64) -> String {
    if is_integral(n) {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Canonical RFC3339 text of a date-time, in UTC with a `Z` suffix.
pub(crate) fn format_datetime(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// Text that is an RFC3339 date-time, or an offset-less
/// `YYYY-MM-DDTHH:MM[:SS[.f]]` (read as UTC — Obsidian's Date & time property
/// omits seconds and offset), as a UTC date-time. The `T` and `Z` are
/// case-insensitive.
pub(crate) fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    let s = s.to_uppercase();
    if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
        return Some(dt.with_timezone(&Utc));
    }
    ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"]
        .iter()
        .find_map(|f| chrono::NaiveDateTime::parse_from_str(&s, f).ok())
        .map(|n| n.and_utc())
}

fn nested_key_error(key: &str) -> FrontmatterError {
    FrontmatterError(format!(
        "'{key}' holds a nested table, which properties cannot edit"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    pub(super) fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn list(text: &str) -> Vec<(String, PropertyValue)> {
        set_of(text).into_values()
    }

    fn set_of(text: &str) -> PropertySet {
        NoteProperties::new(text, FrontmatterFormat::default()).entries()
    }

    /// The valued entries of a formatter's `parse`.
    fn values(entries: Vec<PropertyEntry>) -> Vec<(String, PropertyValue)> {
        entries
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect()
    }

    // ---- NoteProperties: block location, formatter choice, splicing ----

    #[test]
    fn formatter_follows_the_delimiter() {
        assert_eq!(
            NoteProperties::new("+++\na = 1\n+++\n", FrontmatterFormat::Yaml).format(),
            FrontmatterFormat::Toml
        );
        assert_eq!(
            NoteProperties::new("---\na: 1\n---\n", FrontmatterFormat::Toml).format(),
            FrontmatterFormat::Yaml
        );
        assert_eq!(
            NoteProperties::new("body", FrontmatterFormat::Yaml).format(),
            FrontmatterFormat::Yaml
        );
        assert_eq!(
            NoteProperties::new("body", FrontmatterFormat::default()).format(),
            FrontmatterFormat::Toml
        );
    }

    #[test]
    fn locates_blocks_with_crlf_and_rejects_unclosed() {
        let y = "---\r\na: 1\r\n---\r\nbody";
        let span = locate_frontmatter(y).unwrap();
        assert_eq!(span.format, FrontmatterFormat::Yaml);
        assert_eq!(&y[span.inner.clone()], "a: 1\r\n");
        assert!(locate_frontmatter("---\nunclosed: 1\n").is_none());
        assert!(locate_frontmatter("# no block\n").is_none());
        assert!(locate_frontmatter("+++\n+++\n").unwrap().inner.is_empty());
    }

    #[test]
    fn set_without_block_prepends_one_in_the_requested_format() {
        let v = PropertyValue::Text("done".into());
        assert_eq!(
            NoteProperties::new("# T\nbody\n", FrontmatterFormat::Toml)
                .set("status", &v)
                .unwrap(),
            "+++\nstatus = \"done\"\n+++\n# T\nbody\n"
        );
        assert_eq!(
            NoteProperties::new("body", FrontmatterFormat::Yaml)
                .set("status", &v)
                .unwrap(),
            "---\nstatus: done\n---\nbody"
        );
    }

    #[test]
    fn set_with_block_keeps_its_format_and_the_body() {
        let v = PropertyValue::Bool(true);
        assert_eq!(
            NoteProperties::new("---\na: 1\n---\nbody", FrontmatterFormat::Toml)
                .set("b", &v)
                .unwrap(),
            "---\na: 1\nb: true\n---\nbody"
        );
        assert_eq!(
            NoteProperties::new("+++\na = 1\n+++\nbody", FrontmatterFormat::Yaml)
                .set("b", &v)
                .unwrap(),
            "+++\na = 1\nb = true\n+++\nbody"
        );
    }

    #[test]
    fn remove_without_block_or_key_is_a_no_op() {
        assert_eq!(
            NoteProperties::new("body", FrontmatterFormat::Toml)
                .remove("a")
                .unwrap(),
            None
        );
        assert_eq!(
            NoteProperties::new("+++\nb = 1\n+++\n", FrontmatterFormat::Toml)
                .remove("a")
                .unwrap(),
            None
        );
    }

    #[test]
    fn removing_the_last_key_keeps_an_empty_block() {
        assert_eq!(
            NoteProperties::new("+++\na = 1\n+++\nb", FrontmatterFormat::Toml)
                .remove("a")
                .unwrap(),
            Some("+++\n+++\nb".to_string())
        );
    }

    #[test]
    fn list_is_lenient_and_dedupes_case_variants() {
        assert!(list("+++\nnot = = toml\n+++\nbody").is_empty());
        assert!(list("no frontmatter").is_empty());
        assert_eq!(
            list("+++\nStatus = \"a\"\nstatus = \"b\"\n+++\n"),
            vec![("status".to_string(), PropertyValue::Text("a".into()))]
        );
    }

    #[test]
    fn tags_from_list_or_bare_string() {
        let l = set_of("+++\ntags = [\"Rust\", \"#notes\", \"\"]\n+++\n");
        assert_eq!(l.tags(), vec!["rust", "notes"]);
        let bare = set_of("+++\ntags = \"Big Project, misc\"\n+++\n");
        assert_eq!(bare.tags(), vec!["big project, misc"]);
        assert!(set_of("+++\ntags = 5\n+++\n").tags().is_empty());
        assert_eq!(
            set_of("---\ntags: [a]\ntag: b\n---\n").tags(),
            vec!["a", "b"],
            "legacy singular `tag` counts too"
        );
    }

    #[test]
    fn yaml_list_reads_through_the_struct() {
        let p = set_of("---\ntags: [Rust, '#notes', '']\n---\n");
        assert_eq!(p.tags(), vec!["rust", "notes"]);
        assert!(list("---\nkey: [unclosed\n---\nbody").is_empty());
    }

    #[test]
    fn normalize_key_rules() {
        assert_eq!(normalize_key("  Status "), Some("status".into()));
        assert_eq!(normalize_key("due date"), Some("due date".into()));
        assert_eq!(normalize_key("  "), None);
        assert_eq!(normalize_key("a\nb"), None);
    }

    #[test]
    fn number_and_datetime_formatting() {
        assert_eq!(format_number(5.0), "5");
        assert_eq!(format_number(-2.0), "-2");
        assert_eq!(format_number(4.5), "4.5");
        let dt = Utc.with_ymd_and_hms(2024, 1, 31, 10, 0, 0).unwrap();
        assert_eq!(format_datetime(&dt), "2024-01-31T10:00:00Z");
    }

    // ---- The PropertyFormatter contract, run against every formatter ----

    /// Every formatter the contract must hold for.
    const FORMATS: &[FrontmatterFormat] = &[FrontmatterFormat::Toml, FrontmatterFormat::Yaml];

    /// The per-format spelling of the same fixture.
    fn fixture(format: FrontmatterFormat, toml: &'static str, yaml: &'static str) -> &'static str {
        match format {
            FrontmatterFormat::Toml => toml,
            FrontmatterFormat::Yaml => yaml,
        }
    }

    fn sample_values() -> Vec<(String, PropertyValue)> {
        let at = Utc.with_ymd_and_hms(2024, 1, 31, 10, 0, 0).unwrap();
        [
            ("t", PropertyValue::Text("Hello world".into())),
            ("n", PropertyValue::Number(4.5)),
            ("i", PropertyValue::Number(7.0)),
            ("b", PropertyValue::Bool(true)),
            ("d", PropertyValue::Date(d(2024, 1, 31))),
            ("dt", PropertyValue::DateTime(at)),
            ("l", PropertyValue::List(vec!["x".into(), "y z".into()])),
            ("e", PropertyValue::List(vec![])),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
    }

    #[test]
    fn contract_empty_block_parses_to_nothing() {
        for &format in FORMATS {
            assert_eq!(
                values(format.formatter().parse("").unwrap()),
                vec![],
                "{format:?}"
            );
        }
    }

    #[test]
    fn contract_set_then_parse_round_trips_every_type() {
        for &format in FORMATS {
            let f = format.formatter();
            let mut block = String::new();
            for (k, v) in sample_values() {
                block = f.set(&block, &k, &v).unwrap();
                assert!(block.ends_with('\n'), "{format:?}: {block:?}");
            }
            assert_eq!(
                values(f.parse(&block).unwrap()),
                sample_values(),
                "{format:?}:\n{block}"
            );
        }
    }

    #[test]
    fn contract_set_replaces_in_place_keeping_spelling_and_order() {
        for &format in FORMATS {
            let f = format.formatter();
            let mut block = String::new();
            for k in ["a", "Status", "c"] {
                block = f
                    .set(&block, k, &PropertyValue::Text("old".into()))
                    .unwrap();
            }
            block = f
                .set(&block, "status", &PropertyValue::Text("new".into()))
                .unwrap();
            let keys: Vec<String> = values(f.parse(&block).unwrap())
                .into_iter()
                .map(|(k, _)| k)
                .collect();
            assert_eq!(keys, ["a", "status", "c"], "{format:?}:\n{block}");
            assert!(
                block.contains("Status"),
                "existing spelling kept ({format:?}):\n{block}"
            );
            assert_eq!(
                block.matches("tatus").count(),
                1,
                "no duplicate ({format:?}):\n{block}"
            );
        }
    }

    #[test]
    fn contract_set_collapses_case_duplicates() {
        for &format in FORMATS {
            let block = fixture(
                format,
                "Status = \"a\"\nstatus = \"b\"\n",
                "Status: a\nstatus: b\n",
            );
            let out = format
                .formatter()
                .set(block, "status", &PropertyValue::Text("c".into()))
                .unwrap();
            assert_eq!(
                values(format.formatter().parse(&out).unwrap()),
                vec![("status".to_string(), PropertyValue::Text("c".into()))],
                "{format:?}:\n{out}"
            );
        }
    }

    #[test]
    fn contract_preserves_comments_and_untouched_entries() {
        for &format in FORMATS {
            let block = fixture(format, "# keep\na = 1\nb = 2\n", "# keep\na: 1\nb: 2\n");
            let out = format
                .formatter()
                .set(block, "b", &PropertyValue::Number(3.0))
                .unwrap();
            assert_eq!(
                out,
                fixture(format, "# keep\na = 1\nb = 3\n", "# keep\na: 1\nb: 3\n")
            );
        }
    }

    #[test]
    fn contract_remove_drops_only_the_target() {
        for &format in FORMATS {
            let f = format.formatter();
            let block = fixture(
                format,
                "# about a\na = 1\n# about b\nb = 2\n",
                "# about a\na: 1\n# about b\nb: 2\n",
            );
            let out = f.remove(block, "a").unwrap().unwrap();
            assert!(!out.contains("about a"), "{format:?}:\n{out}");
            assert!(out.contains("# about b"), "{format:?}:\n{out}");
            assert_eq!(
                values(f.parse(&out).unwrap()),
                vec![("b".to_string(), PropertyValue::Number(2.0))]
            );
            assert_eq!(f.remove(&out, "a").unwrap(), None, "{format:?}");
            assert_eq!(
                f.remove(&out, "b").unwrap(),
                Some(String::new()),
                "{format:?}"
            );
        }
    }

    #[test]
    fn contract_remove_keeps_comment_separated_by_a_blank_line() {
        for &format in FORMATS {
            let f = format.formatter();
            let block = fixture(
                format,
                "# header\n\n# about a\na = 1\nb = 2\n",
                "# header\n\n# about a\na: 1\nb: 2\n",
            );
            let out = f.remove(block, "a").unwrap().unwrap();
            assert_eq!(
                out,
                fixture(format, "# header\n\nb = 2\n", "# header\n\nb: 2\n"),
                "{format:?}"
            );
            // The same when the removed key is the last one.
            let last = fixture(
                format,
                "# header\n\n# about a\na = 1\n",
                "# header\n\n# about a\na: 1\n",
            );
            let out = f.remove(last, "a").unwrap().unwrap();
            assert!(out.contains("# header"), "{format:?}:\n{out}");
            assert!(!out.contains("about a"), "{format:?}:\n{out}");
            // Collapsing a case-duplicate keeps it too.
            let dup = fixture(
                format,
                "a = 1\n# header\n\n# about A\nA = 2\n",
                "a: 1\n# header\n\n# about A\nA: 2\n",
            );
            let out = f.set(dup, "a", &PropertyValue::Number(3.0)).unwrap();
            assert!(out.contains("# header"), "{format:?}:\n{out}");
            assert!(!out.contains("about A"), "{format:?}:\n{out}");
        }
    }

    #[test]
    fn contract_malformed_block_is_refused() {
        for &format in FORMATS {
            let f = format.formatter();
            let bad = fixture(format, "not = = toml\n", "- a\n- b\n");
            assert!(f.parse(bad).is_err(), "{format:?}");
            assert!(
                f.set(bad, "a", &PropertyValue::Bool(true)).is_err(),
                "{format:?}"
            );
            assert!(f.remove(bad, "a").is_err(), "{format:?}");
        }
    }

    #[test]
    fn contract_valueless_key_is_an_entry_without_a_value() {
        for &format in FORMATS {
            let block = fixture(format, "blank = nan\nok = 1\n", "blank:\nok: 1\n");
            assert_eq!(
                format.formatter().parse(block).unwrap(),
                vec![
                    ("blank".to_string(), None),
                    ("ok".to_string(), Some(PropertyValue::Number(1.0))),
                ],
                "{format:?}"
            );
        }
    }

    #[test]
    fn property_set_separates_keys_from_values() {
        let set = set_of("---\nStatus:\nstatus: x\ntags: [a]\ndue: 2024-01-01\n---\n");
        assert_eq!(set.keys().collect::<Vec<_>>(), ["status", "tags", "due"]);
        assert_eq!(
            set.values().map(|(k, _)| k).collect::<Vec<_>>(),
            ["tags", "due"],
            "the first of the case-duplicates wins, and it has no value"
        );
        assert_eq!(set.tags(), vec!["a"]);
        assert!(set_of("---\nkey: [unclosed\n---\n").keys().next().is_none());
    }

    #[test]
    fn contract_nested_table_is_skipped_on_read_and_refused_on_write() {
        for &format in FORMATS {
            let f = format.formatter();
            let block = fixture(format, "ok = 1\n[meta]\nx = 1\n", "ok: 1\nmeta:\n  x: 1\n");
            assert_eq!(
                f.parse(block).unwrap(),
                vec![("ok".to_string(), Some(PropertyValue::Number(1.0)))],
                "a nested table is no entry at all ({format:?})"
            );
            assert!(
                f.set(block, "meta", &PropertyValue::Bool(true)).is_err(),
                "{format:?}"
            );
            assert!(f.remove(block, "meta").is_err(), "{format:?}");
        }
    }
}
