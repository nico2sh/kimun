//! Frontmatter properties: the typed key/value pairs in a note's leading
//! `+++` (TOML) or `---` (YAML) block.
//!
//! Composition, one contract: [`NoteProperties`] is the only entry point. It
//! locates the block, picks the [`PropertyFormatter`] for its syntax, and
//! delegates every read and edit to it; formatters only ever see the block's
//! contents. Reading is lenient (a malformed block yields no properties);
//! editing is strict (a malformed block refuses the edit).

mod datetime;
mod toml_formatter;
mod value;
mod yaml_formatter;

pub use datetime::PropertyDateTime;
pub use value::{PropertyInput, PropertyKind, PropertyValue};

use std::collections::HashSet;
use std::ops::Range;

use chrono::{DateTime, SecondsFormat, Utc};

use super::content_extractor::{frontmatter_bounds, split_bom};
use toml_formatter::TomlFormatter;
use yaml_formatter::YamlFormatter;

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

/// Parses `toml` / `yaml` (case-insensitive).
impl std::str::FromStr for FrontmatterFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "toml" => Ok(FrontmatterFormat::Toml),
            "yaml" => Ok(FrontmatterFormat::Yaml),
            other => Err(format!(
                "unknown frontmatter format '{other}' (expected toml or yaml)"
            )),
        }
    }
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
pub(crate) enum FrontmatterError {
    /// The existing block doesn't parse, or isn't a key/value mapping.
    Malformed(String),
    /// The block is fine, but this edit can't be made safely (a nested table,
    /// a value that would end the block, an edit the YAML splice can't place).
    Refused(String),
}

impl FrontmatterError {
    /// The user-facing reason.
    pub(crate) fn message(&self) -> &str {
        match self {
            FrontmatterError::Malformed(m) | FrontmatterError::Refused(m) => m,
        }
    }
}

/// One top-level frontmatter entry: its key (as written) and its value, or
/// `None` when the note has the key without a usable value (YAML `key:`, a
/// non-finite number, a TOML local time).
pub type PropertyEntry = (String, Option<PropertyValue>);

/// What a note's frontmatter declares: its property entries in file order,
/// keys spelled as written, the first of any case-duplicate keys winning. The
/// one shape the index and the API read properties through — keys (for "has
/// property"), typed values and the `tags` labels all come from here.
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
                .filter(|(k, _)| seen.insert(match_key(k)))
                .collect(),
        }
    }

    /// Every key the note has, with its value when it has a usable one.
    pub(crate) fn into_entries(self) -> Vec<PropertyEntry> {
        self.entries
    }

    /// The properties that have a usable value.
    pub(crate) fn values(&self) -> impl Iterator<Item = (&str, &PropertyValue)> {
        self.entries
            .iter()
            .filter_map(|(k, v)| Some((k.as_str(), v.as_ref()?)))
    }

    /// Label names from the `tags` property (and Obsidian's legacy singular
    /// `tag`): a list gives one tag per item, a bare string one tag per
    /// comma-separated part (`tags: project, urgent`, as older Obsidian notes
    /// write it). Trimmed, a leading `#` dropped, lowercased, empties removed.
    pub(crate) fn tags(&self) -> Vec<String> {
        self.values()
            .filter(|(k, _)| is_tag_key(k))
            .flat_map(|(_, v)| match v {
                PropertyValue::List(items) => items.iter().map(String::as_str).collect(),
                PropertyValue::Text(s) => split_tags(s).collect(),
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
///   empty block is `Ok(vec![])`. Entries come back with keys as written, in
///   file order. A key whose value doesn't map onto [`PropertyValue`] (null,
///   non-finite number, TOML local time) is still an entry, with no value — the
///   note visibly has that property. A nested table/mapping is not a property
///   and yields no entry.
/// - `set` / `remove` match keys case-insensitively, keep the spelling and
///   position of the first match, collapse case-duplicates, and leave every
///   other line of the block (comments, order, untouched entries) as it was.
///   A new key is written as given (casing kept); an inline `# comment`
///   trailing the replaced entry's first line stays with it.
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
    /// `block` with the entry for `old` keyed `new` instead, in place: same
    /// position, value, inline comment and comment lines. `None` when `old`
    /// is absent. Callers have checked that `new` names no *other* entry.
    fn rename(&self, block: &str, old: &str, new: &str)
        -> Result<Option<String>, FrontmatterError>;
}

/// Where a note's frontmatter block sits in its text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FrontmatterSpan {
    format: FrontmatterFormat,
    /// Byte range of the block's contents, between the delimiter lines.
    inner: Range<usize>,
}

/// Finds the leading frontmatter block, with the indexer's rules
/// (`frontmatter_bounds`): the first line is exactly `---` or `+++` and a
/// later line is exactly the same delimiter (CRLF tolerated).
fn locate_frontmatter(text: &str) -> Option<FrontmatterSpan> {
    let bounds = frontmatter_bounds(text)?;
    let format = if bounds.delimiter == FrontmatterFormat::Toml.delimiter() {
        FrontmatterFormat::Toml
    } else {
        FrontmatterFormat::Yaml
    };
    Some(FrontmatterSpan {
        format,
        inner: bounds.inner,
    })
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
    /// malformed block yields an empty set. Line endings are read as LF, so a
    /// multi-line value never carries a `\r` (edits already get LF text).
    pub(crate) fn property_set(&self) -> PropertySet {
        if self.span.is_none() {
            return PropertySet::default();
        }
        let block = crate::nfs::to_lf(self.block());
        PropertySet::from_entries(self.formatter.parse(&block).unwrap_or_default())
    }

    /// The note's text with `key` (already normalized) set to `value`.
    /// Strict: a malformed existing block is an error, never guessed at.
    pub(crate) fn set(&self, key: &str, value: &PropertyValue) -> Result<String, FrontmatterError> {
        let block = self.formatter.set(self.block(), key, value)?;
        self.checked(&block)
    }

    /// The note's text without `key`, or `None` when there is nothing to
    /// remove (no block, or no such key).
    pub(crate) fn remove(&self, key: &str) -> Result<Option<String>, FrontmatterError> {
        if self.span.is_none() {
            return Ok(None);
        }
        self.formatter
            .remove(self.block(), key)?
            .map(|block| self.checked(&block))
            .transpose()
    }

    /// The note's text with `old` renamed to `new` (both normalized), or
    /// `None` when there is no such key. Refuses a `new` that names another
    /// existing entry.
    pub(crate) fn rename(&self, old: &str, new: &str) -> Result<Option<String>, FrontmatterError> {
        if self.span.is_none() {
            return Ok(None);
        }
        let block = crate::nfs::to_lf(self.block());
        let entries = self.formatter.parse(&block)?;
        if !keys_match(old, new) && entries.iter().any(|(k, _)| keys_match(k, new)) {
            return Err(FrontmatterError::Refused(format!("'{new}' already exists")));
        }
        self.formatter
            .rename(&block, old, new)?
            .map(|block| self.checked(&block))
            .transpose()
    }

    /// The note's text with `block` in place — provided the result still
    /// holds exactly that block. A value with a line that reads as the
    /// delimiter (a TOML multi-line string containing `+++`) would end the
    /// block early and turn the rest into body text; that edit is refused and
    /// the note is left as it was.
    fn checked(&self, block: &str) -> Result<String, FrontmatterError> {
        let text = self.with_block(block);
        match locate_frontmatter(&text) {
            Some(span) if &text[span.inner.clone()] == block => Ok(text),
            _ => Err(FrontmatterError::Refused(format!(
                "the value has a line that reads as the frontmatter delimiter `{}`, which would end the block early; store it without that line",
                self.format.delimiter()
            ))),
        }
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
                // A byte-order mark must stay the first character of the file.
                let (bom, body) = split_bom(self.text);
                format!("{bom}{d}\n{block}{d}\n{body}")
            }
        }
    }
}

// Key rule: a key is *spelled* as the user wrote it ([`clean_key`]: in the
// note, and in what reads return), *identified* by case alone ([`match_key`] /
// [`keys_match`]: edits, lookups, tag detection), and *searched* in
// [`search_form`] (case and accents: the index and queries).

/// A property key as spelled: trimmed, casing kept. `None` when empty or
/// containing control characters (a line break would corrupt the block).
pub(crate) fn clean_key(key: &str) -> Option<String> {
    let key = key.trim();
    (!key.is_empty() && !key.chars().any(char::is_control)).then(|| key.to_string())
}

/// A key's identity: lowercased. Two keys with the same identity are one
/// property (`Status` and `status`), whatever their accents.
pub(crate) fn match_key(key: &str) -> String {
    key.to_lowercase()
}

/// Whether two keys name the same property.
pub(crate) fn keys_match(a: &str, b: &str) -> bool {
    match_key(a) == match_key(b)
}

/// The form property text is searched in: case-folded and accent-stripped
/// (`É` → `e`), so `done` finds `Done` and `e` finds `é`. Used for every
/// indexed value, every query value and every indexed or queried key.
///
/// Searching is looser than editing: an edit identifies a key by
/// [`match_key`], so `résumé` and `resume` stay two properties when written,
/// and the index keeps only the first of a note's keys that fold together.
pub(crate) fn search_form(text: &str) -> String {
    super::diacritics::remove_diacritics(text).to_lowercase()
}

/// A property key as it is searched: [`clean_key`] in [`search_form`].
pub(crate) fn search_key(key: &str) -> Option<String> {
    clean_key(key).map(|k| search_form(&k))
}

/// A property key in the form the index stores, filters and sorts it by
/// (trimmed, case-folded, accents stripped), so `Rank` and `rank` are one
/// key. `None` for a key that can't be a property (blank, multi-line).
pub fn property_search_key(key: &str) -> Option<String> {
    search_key(key)
}

/// The date or date-time `s` spells exactly: `YYYY-MM-DD` as a date, RFC3339
/// or an offset-less `YYYY-MM-DDTHH:MM[:SS[.f]]` (a local time — Obsidian's
/// Date & time property omits seconds and offset) as a date-time. How YAML
/// strings are read, and how the index compares text that is a date (a TOML
/// string, Hugo's quoted dates).
pub(crate) fn date_in_text(s: &str) -> Option<PropertyValue> {
    if s.len() == 10 {
        if let Some(date) = crate::dates::parse_iso_date(s) {
            return Some(PropertyValue::Date(date));
        }
    }
    PropertyDateTime::parse(s).map(PropertyValue::DateTime)
}

/// The frontmatter keys whose items are unified into the label index:
/// `tags`, and the singular `tag` older Obsidian notes use.
const TAG_KEYS: [&str; 2] = ["tags", "tag"];

/// The tags in one comma-separated string (`project, urgent`), trimmed,
/// empties dropped; spaces inside a tag (`big project`) are kept.
pub(crate) fn split_tags(s: &str) -> impl Iterator<Item = &str> {
    s.split(',').map(str::trim).filter(|t| !t.is_empty())
}

/// `key` holds a note's tags (whatever its casing): always a list of labels.
pub(crate) fn is_tag_key(key: &str) -> bool {
    let key = match_key(key);
    TAG_KEYS.contains(&key.as_str())
}

/// The other keys Obsidian always treats as lists (plus their legacy
/// singular spellings): note aliases and CSS classes. Not labels, and core
/// never splits their values on commas — an alias may contain one. (The TUI
/// form does split its single comma-separated field into items for these
/// keys; that is the form's input syntax, not a core rule.)
const OTHER_LIST_KEYS: [&str; 4] = ["aliases", "alias", "cssclasses", "cssclass"];

/// `key` always holds a list (whatever its casing): the tag keys, `aliases`,
/// `cssclasses`.
pub(crate) fn is_list_key(key: &str) -> bool {
    is_tag_key(key) || OTHER_LIST_KEYS.contains(&match_key(key).as_str())
}

/// Whether `key` (in any casing) is a property that always holds a list:
/// the tag keys (`tags`, `tag`), `aliases`, `cssclasses` and their legacy
/// singular spellings. Lets a UI split comma-separated input into list items
/// for these keys without keeping its own copy of the key names.
pub fn is_list_property_key(key: &str) -> bool {
    is_list_key(key)
}

/// Whether two keys name the same property in a note (`Status` and
/// `status` do; `résumé` and `resume` don't): the identity core uses for
/// every edit and lookup. Lets a UI spot a key the note already has
/// without keeping its own copy of the rule.
pub fn property_keys_match(a: &str, b: &str) -> bool {
    keys_match(a, b)
}

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

/// A block that repeats `key`, written again at byte `at` of `block` —
/// invalid in TOML and in YAML, which both parsers report in their own terms
/// (byte offsets, node dumps). The line is the note's: the block starts on
/// line 2, below its opening delimiter.
fn duplicate_key_error(block: &str, key: &str, at: usize) -> FrontmatterError {
    let line = block
        .get(..at)
        .map_or(0, |before| before.matches('\n').count())
        + 2;
    FrontmatterError::Malformed(format!(
        "'{key}' appears more than once (again on line {line}); keep only one of them"
    ))
}

fn nested_key_error(key: &str) -> FrontmatterError {
    FrontmatterError::Refused(format!(
        "'{key}' holds a nested table, which properties cannot edit"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};

    #[test]
    fn is_list_property_key_matches_core_list_keys() {
        for key in ["tags", "Tag", "aliases", "cssclasses"] {
            assert!(is_list_property_key(key), "{key}");
        }
        assert!(!is_list_property_key("status"));
    }

    #[test]
    fn property_keys_match_follows_core_identity() {
        assert!(property_keys_match("Status", "status"));
        assert!(property_keys_match("TAGS", "Tags"));
        assert!(!property_keys_match("résumé", "resume"));
    }

    pub(super) fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn list(text: &str) -> Vec<(String, PropertyValue)> {
        set_of(text)
            .values()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn set_of(text: &str) -> PropertySet {
        NoteProperties::new(text, FrontmatterFormat::default()).property_set()
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
    fn a_byte_order_mark_is_skipped_when_locating_and_kept_on_top() {
        let y = "\u{feff}---\na: 1\n---\nbody";
        let span = locate_frontmatter(y).unwrap();
        assert_eq!(&y[span.inner.clone()], "a: 1\n");
        assert_eq!(
            NoteProperties::new(y, FrontmatterFormat::Toml)
                .set("b", &PropertyValue::Bool(true))
                .unwrap(),
            "\u{feff}---\na: 1\nb: true\n---\nbody"
        );
        assert_eq!(
            NoteProperties::new("\u{feff}body", FrontmatterFormat::Toml)
                .set("b", &PropertyValue::Bool(true))
                .unwrap(),
            "\u{feff}+++\nb = true\n+++\nbody"
        );
    }

    #[test]
    fn keys_keep_their_casing_when_written_and_read() {
        for (fmt, expect) in [
            (FrontmatterFormat::Toml, "+++\ndueDate = 1\n+++\n"),
            (FrontmatterFormat::Yaml, "---\ndueDate: 1\n---\n"),
        ] {
            let out = NoteProperties::new("", fmt)
                .set("dueDate", &PropertyValue::Number(1.0))
                .unwrap();
            assert_eq!(out, expect);
            assert_eq!(list(&out)[0].0, "dueDate");
        }
        assert_eq!(match_key("dueDate"), "duedate");
        assert_eq!(clean_key(" dueDate "), Some("dueDate".into()));
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
            vec![("Status".to_string(), PropertyValue::Text("a".into()))],
            "keys read back as written; the first case variant wins"
        );
    }

    #[test]
    fn windows_line_endings_never_reach_a_value() {
        let toml = "+++\r\ndesc = \"\"\"\r\na\r\nb\"\"\"\r\n+++\r\nbody\r\n";
        let yaml = "---\r\ndesc: |\r\n  a\r\n  b\r\n---\r\nbody\r\n";
        // YAML's `|` keeps one trailing line break; neither keeps a `\r`.
        for (text, value) in [(toml, "a\nb"), (yaml, "a\nb\n")] {
            assert_eq!(
                crate::note::NoteDetails::properties_of(text),
                vec![("desc".to_string(), Some(PropertyValue::Text(value.into())))],
                "{text:?}"
            );
        }
    }

    #[test]
    fn tags_from_list_or_bare_string() {
        let l = set_of("+++\ntags = [\"Rust\", \"#notes\", \"\"]\n+++\n");
        assert_eq!(l.tags(), vec!["rust", "notes"]);
        let bare = set_of("+++\ntags = \"Big Project, misc\"\n+++\n");
        assert_eq!(
            bare.tags(),
            vec!["big project", "misc"],
            "a comma-separated string is a list; spaces stay inside a tag"
        );
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
    fn key_rules() {
        assert_eq!(clean_key("  Status "), Some("Status".into()));
        assert_eq!(clean_key("due date"), Some("due date".into()));
        assert_eq!(clean_key("  "), None);
        assert_eq!(clean_key("a\nb"), None);
        assert!(keys_match("Status", "STATUS"));
        assert!(
            !keys_match("résumé", "resume"),
            "identity ignores case only"
        );
        assert_eq!(search_key(" RÉSUMÉ "), Some("resume".into()));
        assert_eq!(search_form("Ça va"), "ca va");
        assert!(is_tag_key("Tags") && is_tag_key("TAG") && !is_tag_key("tâgs"));
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
            ("dt", PropertyValue::DateTime(at.into())),
            (
                "local_dt",
                PropertyValue::DateTime(PropertyDateTime::parse("2024-01-31T10:00").unwrap()),
            ),
            (
                "offset_dt",
                PropertyValue::DateTime(
                    PropertyDateTime::parse("2024-01-31T10:00:00+02:00").unwrap(),
                ),
            ),
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
    fn contract_a_repeated_key_is_named_in_the_error() {
        use FrontmatterFormat::{Toml, Yaml};
        for (format, block, key, line) in [
            (Toml, "status = 'a'\nother = 1\nstatus = 'b'\n", "status", 4),
            (Toml, "\"my key\" = 1\n\"my key\" = 2\n", "my key", 3),
            (
                Toml,
                "'say \"hi\"' = 1\n'say \"hi\"' = 2\n",
                "say \"hi\"",
                3,
            ),
            (Toml, "\"caf\\u00e9\" = 1\n\"caf\\u00e9\" = 2\n", "café", 3),
            (Toml, "a = 1\n[meta]\nx = 1\nx = 2\n", "x", 5),
            (Yaml, "status: a\nother: 1\nstatus: b\n", "status", 4),
            // The nested repeat is the error; the top-level one comes later.
            (Yaml, "a: 1\nb:\n  x: 1\n  x: 2\na: 2\n", "x", 5),
            // A repeated key holding a block value.
            (Yaml, "a:\n  x: 1\nb: 2\na:\n  y: 2\n", "a", 5),
            (Yaml, "tags:\n  - a\ntags:\n  - b\n", "tags", 4),
            (Yaml, "a: |\n  t\nb: 1\na: |\n  u\n", "a", 5),
            (Yaml, "a: {x: 1}\na: {y: 2}\n", "a", 3),
            (Yaml, "a: 1\na: 'multi\n  line'\nb: 2\n", "a", 3),
            (Yaml, "1: a\n1: b\n", "1", 3),
            // `"1"` and `1` differ in YAML: only `k` repeats.
            (Yaml, "\"1\": a\n1: b\nm:\n  k: 1\n  k: 2\n", "k", 6),
        ] {
            let f = format.formatter();
            let expected = format!(
                "'{key}' appears more than once (again on line {line}); keep only one of them"
            );
            for result in [
                f.parse(block).map(|_| ()),
                f.set(block, "x", &PropertyValue::Number(1.0)).map(|_| ()),
                f.remove(block, "x").map(|_| ()),
            ] {
                assert_eq!(
                    result,
                    Err(FrontmatterError::Malformed(expected.clone())),
                    "{format:?}: {block:?}"
                );
            }
        }
    }

    #[test]
    fn contract_other_parse_errors_keep_the_parsers_message() {
        for (format, block) in [
            (FrontmatterFormat::Toml, "a = \n"),
            (FrontmatterFormat::Toml, "a = [1\n"),
            (FrontmatterFormat::Yaml, "a: [1\n"),
            (FrontmatterFormat::Yaml, "a: 1\n  b: 2\n"),
        ] {
            match format.formatter().parse(block) {
                Err(FrontmatterError::Malformed(m)) => {
                    assert!(!m.contains("appears more than once"), "{block:?}: {m}")
                }
                other => panic!("{format:?} {block:?}: {other:?}"),
            }
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
            assert_eq!(keys, ["a", "Status", "c"], "{format:?}:\n{block}");
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
                vec![("Status".to_string(), PropertyValue::Text("c".into()))],
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
        assert_eq!(
            set.clone()
                .into_entries()
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<_>>(),
            ["Status", "tags", "due"]
        );
        assert_eq!(
            set.values().map(|(k, _)| k).collect::<Vec<_>>(),
            ["tags", "due"],
            "the first of the case-duplicates wins, and it has no value"
        );
        assert_eq!(set.tags(), vec!["a"]);
        assert!(set_of("---\nkey: [unclosed\n---\n")
            .into_entries()
            .is_empty());
    }

    #[test]
    fn contract_nested_table_is_skipped_on_read_and_refused_on_write() {
        for &format in FORMATS {
            let f = format.formatter();
            let blocks = [
                fixture(format, "ok = 1\n[meta]\nx = 1\n", "ok: 1\nmeta:\n  x: 1\n"),
                fixture(
                    format,
                    "ok = 1\nmeta = { x = 1 }\n",
                    "ok: 1\nmeta: {x: 1}\n",
                ),
                fixture(
                    format,
                    "ok = 1\nmeta = [{ name = \"a\" }]\n",
                    "ok: 1\nmeta:\n  - name: a\n",
                ),
            ];
            for block in blocks {
                assert_eq!(
                    f.parse(block).unwrap(),
                    vec![("ok".to_string(), Some(PropertyValue::Number(1.0)))],
                    "a nested table is no entry at all ({format:?}): {block:?}"
                );
                assert!(
                    f.set(block, "meta", &PropertyValue::Bool(true)).is_err(),
                    "{format:?}: {block:?}"
                );
                assert!(f.remove(block, "meta").is_err(), "{format:?}: {block:?}");
            }
        }
    }

    #[test]
    fn an_edit_that_would_break_the_block_is_refused() {
        let fence = PropertyValue::Text("before\n+++\nafter".into());
        let listed = PropertyValue::List(vec!["a\n+++\nb".into()]);
        for text in ["+++\na = 1\n+++\n# Title\n", "# Title\n"] {
            let props = NoteProperties::new(text, FrontmatterFormat::Toml);
            for value in [&fence, &listed] {
                let err = props.set("desc", value).unwrap_err();
                assert!(
                    matches!(&err, FrontmatterError::Refused(m) if m.contains("delimiter")),
                    "{err:?}"
                );
            }
        }
        // YAML escapes line breaks, so the same text is stored safely.
        let yaml = NoteProperties::new("---\na: 1\n---\nbody", FrontmatterFormat::Yaml);
        let out = yaml
            .set("desc", &PropertyValue::Text("x\n---\ny".into()))
            .unwrap();
        assert_eq!(
            NoteProperties::new(&out, FrontmatterFormat::Yaml)
                .property_set()
                .values()
                .count(),
            2
        );
    }
}
