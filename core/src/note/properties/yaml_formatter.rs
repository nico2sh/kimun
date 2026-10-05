//! YAML (`---`) frontmatter, as Obsidian writes it. Read with `yaml-rust2`.
//! No format-preserving YAML editor exists, so an edit splices one top-level
//! entry as lines (its key line plus indented / `- ` continuation lines) and
//! re-parses the result to prove the edit landed where intended.

use std::ops::Range;

use yaml_rust2::{yaml::Hash, ScanError, Yaml, YamlLoader};

use super::{
    date_in_text, duplicate_key_error, finite, format_number, keys_match, nested_key_error,
    FrontmatterError, PropertyEntry, PropertyFormatter, PropertyValue,
};
use crate::dates::format_iso_date;

/// The [`PropertyFormatter`] for `---` blocks.
pub(super) struct YamlFormatter;

impl PropertyFormatter for YamlFormatter {
    fn parse(&self, block: &str) -> Result<Vec<PropertyEntry>, FrontmatterError> {
        let Some(root) = root(block)? else {
            return Ok(Vec::new());
        };
        // A nested mapping is not a property; a null (`key:`) is one, unvalued.
        Ok(root
            .iter()
            .filter(|(_, v)| !is_nested(v))
            .filter_map(|(k, v)| Some((scalar_text(k)?, read_value(v))))
            .collect())
    }

    fn set(
        &self,
        block: &str,
        key: &str,
        value: &PropertyValue,
    ) -> Result<String, FrontmatterError> {
        check_root(block, key)?;
        let mut lines: Vec<String> = block.lines().map(str::to_string).collect();
        let entries = entry_ranges(&lines, key);
        let written_key = entries
            .first()
            .map_or_else(|| key.to_string(), |(_, k)| k.clone());
        let mut rendered = render_entry(&written_key, value);
        // A `# comment` trailing the old entry's first line stays on it.
        if let Some((range, _)) = entries.first() {
            if let Some(comment) = inline_comment(&lines[range.start]) {
                rendered[0].push_str(comment);
            }
        }
        // Drop later duplicates (and their comments) back to front so the
        // first range stays valid.
        for (range, _) in entries.iter().skip(1).rev() {
            lines.drain(leading_comments(&lines, range.start)..range.end);
        }
        match entries.first() {
            Some((range, _)) => {
                lines.splice(range.clone(), rendered.clone());
            }
            None => {
                let at = append_at(&lines);
                lines.splice(at..at, rendered.clone());
            }
        }
        let out = join_lines(&lines);
        verify(&out, key, true)?;
        verify_value(&out, key, &rendered)?;
        verify_others(block, &out, key)?;
        Ok(out)
    }

    fn remove(&self, block: &str, key: &str) -> Result<Option<String>, FrontmatterError> {
        check_root(block, key)?;
        let mut lines: Vec<String> = block.lines().map(str::to_string).collect();
        let entries = entry_ranges(&lines, key);
        if entries.is_empty() {
            // No entry line: the key is absent, or lives somewhere the line
            // splice cannot see (flow-style root) — which must not pass as
            // "nothing to remove".
            verify(block, key, false)?;
            return Ok(None);
        }
        for (range, _) in entries.iter().rev() {
            lines.drain(leading_comments(&lines, range.start)..range.end);
        }
        let out = join_lines(&lines);
        verify(&out, key, false)?;
        verify_others(block, &out, key)?;
        Ok(Some(out))
    }

    fn rename(
        &self,
        block: &str,
        old: &str,
        new: &str,
    ) -> Result<Option<String>, FrontmatterError> {
        check_root(block, old)?;
        let mut lines: Vec<String> = block.lines().map(str::to_string).collect();
        let entries = entry_ranges(&lines, old);
        let Some((range, _)) = entries.first() else {
            verify(block, old, false)?;
            return Ok(None);
        };
        // Every root key counts, nested mappings included.
        if !keys_match(old, new)
            && root(block)?.is_some_and(|r| {
                r.keys()
                    .any(|k| scalar_text(k).is_some_and(|k| keys_match(&k, new)))
            })
        {
            return Err(FrontmatterError::Refused(format!("'{new}' already exists")));
        }
        let line = &lines[range.start];
        let colon = key_colon(line).ok_or_else(|| {
            FrontmatterError::Refused(format!("cannot locate the key '{old}' on its line"))
        })?;
        lines[range.start] = format!("{}{}", render_key(new), &line[colon..]);
        let out = join_lines(&lines);
        verify(&out, new, true)?;
        if !keys_match(old, new) {
            verify(&out, old, false)?;
        }
        let before = self.parse(block)?;
        let after = self.parse(&out)?;
        let value_of = |es: &[PropertyEntry], k: &str| {
            es.iter()
                .find(|(e, _)| keys_match(e, k))
                .map(|(_, v)| v.clone())
        };
        if value_of(&before, old) != value_of(&after, new) {
            return Err(FrontmatterError::Refused(format!(
                "renaming '{old}' would change its value; edit the frontmatter by hand"
            )));
        }
        Ok(Some(out))
    }
}

/// The block's root mapping; `Ok(None)` for an empty block. Anything that is
/// not a mapping is an error.
fn root(block: &str) -> Result<Option<Hash>, FrontmatterError> {
    let docs = YamlLoader::load_from_str(block).map_err(|e| match duplicated_key(block, &e) {
        Some((key, at)) => duplicate_key_error(block, &key, at),
        None => FrontmatterError::Malformed(e.to_string()),
    })?;
    match docs.into_iter().next() {
        None | Some(Yaml::Null) => Ok(None),
        Some(Yaml::Hash(h)) => Ok(Some(h)),
        Some(_) => Err(FrontmatterError::Malformed(
            "frontmatter is not a key/value mapping".to_string(),
        )),
    }
}

/// Where a "duplicated key" error points: the key the parser names in it
/// and the byte its line starts at — the nearest line at or above the
/// parser's mark (which sits in or after the repeated value) that holds that
/// key, at any depth. `None` when the error is another one, or names a key no
/// such line holds, so a reworded message falls back to the parser's own.
fn duplicated_key(block: &str, error: &ScanError) -> Option<(String, usize)> {
    let named = error.info().strip_suffix(": duplicated key in mapping")?;
    // The key's debug form: `String("due")`, `Integer(1)`.
    let inner = named.split_once('(')?.1.strip_suffix(')')?;
    let key = match inner.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Some(quoted) => quoted.replace("\\\"", "\"").replace("\\\\", "\\"),
        None => inner.to_string(),
    };
    let at = error.marker().index();
    let mut start = 0;
    let mut found = None;
    for line in block.split_inclusive('\n') {
        if start > at {
            break;
        }
        let content = line.trim_start().trim_start_matches("- ").trim_end();
        if line_key(content).is_some_and(|k| k == key) {
            found = Some(start);
        }
        start += line.len();
    }
    Some((key, found?))
}

fn key_matches(k: &Yaml, key: &str) -> bool {
    scalar_text(k).is_some_and(|k| keys_match(&k, key))
}

/// The block must parse, and `key` must not hold a nested mapping.
fn check_root(block: &str, key: &str) -> Result<(), FrontmatterError> {
    let root = root(block)?;
    if root
        .iter()
        .flatten()
        .any(|(k, v)| key_matches(k, key) && is_nested(v))
    {
        return Err(nested_key_error(key));
    }
    Ok(())
}

/// A mapping, or a list holding one: a nested table, not a property.
fn is_nested(v: &Yaml) -> bool {
    match v {
        Yaml::Hash(_) => true,
        Yaml::Array(items) => items.iter().any(is_nested),
        _ => false,
    }
}

/// The edited block must still parse and hold `key` exactly once (set) or
/// not at all (remove) — otherwise the line splice missed (flow-style root,
/// unusual key quoting) and the edit is refused instead of written.
fn verify(out: &str, key: &str, expect_present: bool) -> Result<(), FrontmatterError> {
    let root = root(out).map_err(|e| {
        FrontmatterError::Refused(format!("edit would leave invalid YAML: {}", e.message()))
    })?;
    let count = root
        .iter()
        .flatten()
        .filter(|(k, _)| key_matches(k, key))
        .count();
    if count == usize::from(expect_present) {
        Ok(())
    } else {
        Err(FrontmatterError::Refused(format!(
            "could not edit '{key}' safely in the YAML block; edit it by hand"
        )))
    }
}

/// A YAML string as the date or date-time it spells ([`date_in_text`]), else
/// as text — YAML has no date type of its own.
fn text_or_date(s: &str) -> PropertyValue {
    date_in_text(s).unwrap_or_else(|| PropertyValue::Text(s.to_string()))
}

/// Every entry other than `key` must read back exactly as before the edit:
/// the line splice may only ever change the entry it targets. Anything else
/// (a misjudged entry boundary eating a neighbour's lines) refuses the edit.
fn verify_others(before: &str, after: &str, key: &str) -> Result<(), FrontmatterError> {
    // The raw YAML entries, so nested mappings (which aren't properties) are
    // protected too.
    let others = |block: &str| -> Vec<(Yaml, Yaml)> {
        root(block)
            .ok()
            .flatten()
            .map(|h| {
                h.into_iter()
                    .filter(|(k, _)| !key_matches(k, key))
                    .collect()
            })
            .unwrap_or_default()
    };
    if others(before) == others(after) {
        Ok(())
    } else {
        Err(FrontmatterError::Refused(format!(
            "could not edit '{key}' without changing another property in the YAML block; edit it by hand"
        )))
    }
}

/// The value `key` holds in `out` must be exactly what the freshly rendered
/// entry says — otherwise leftover lines of the old entry merged into it.
fn verify_value(out: &str, key: &str, rendered: &[String]) -> Result<(), FrontmatterError> {
    let value_of = |block: &str| -> Option<Option<PropertyValue>> {
        let root = root(block).ok()??;
        let (_, v) = root.iter().find(|(k, _)| key_matches(k, key))?;
        Some(read_value(v))
    };
    if value_of(out).is_some() && value_of(out) == value_of(&join_lines(rendered)) {
        Ok(())
    } else {
        Err(FrontmatterError::Refused(format!(
            "could not edit '{key}' safely in the YAML block; edit it by hand"
        )))
    }
}

fn read_value(v: &Yaml) -> Option<PropertyValue> {
    match v {
        Yaml::String(s) => Some(text_or_date(s)),
        Yaml::Integer(i) => Some(PropertyValue::Number(*i as f64)),
        Yaml::Real(_) => v.as_f64().and_then(finite).map(PropertyValue::Number),
        Yaml::Boolean(b) => Some(PropertyValue::Bool(*b)),
        Yaml::Array(items) => Some(PropertyValue::List(
            items.iter().filter_map(scalar_text).collect(),
        )),
        _ => None,
    }
}

fn scalar_text(v: &Yaml) -> Option<String> {
    match v {
        Yaml::String(s) | Yaml::Real(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

/// The unquoted key of a top-level `key: value` / `key:` line, or `None` for
/// indented, comment, list-item and non-entry lines.
fn line_key(line: &str) -> Option<String> {
    if line.is_empty() || line.starts_with([' ', '\t', '#', '-']) {
        return None;
    }
    for quote in ['"', '\''] {
        if let Some(rest) = line.strip_prefix(quote) {
            let end = rest.find(quote)?;
            return rest[end + 1..]
                .trim_start()
                .starts_with(':')
                .then(|| rest[..end].to_string());
        }
    }
    let colon = line
        .find(": ")
        .or_else(|| line.strip_suffix(':').map(str::len))?;
    Some(line[..colon].trim_end().to_string())
}

/// The trailing `# comment` of a line (with the whitespace before it), if
/// any: a `#` outside quotes that follows whitespace. A quote opens only where
/// a scalar can start (line start, after whitespace or `: [ , {`), so an
/// apostrophe inside a word is not one.
fn inline_comment(line: &str) -> Option<&str> {
    let mut chars = line.char_indices().peekable();
    let mut quote: Option<char> = None;
    let mut prev = None;
    while let Some((i, c)) = chars.next() {
        match quote {
            Some('"') if c == '\\' => {
                chars.next();
            }
            // `''` is an escaped apostrophe, not the end of the string.
            Some('\'') if c == '\'' && chars.next_if(|&(_, n)| n == '\'').is_some() => {}
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if matches!(c, '"' | '\'') && prev.is_none_or(opens_scalar) => quote = Some(c),
            None if c == '#' && prev.is_some_and(char::is_whitespace) => {
                return Some(line[line[..i].trim_end().len()..].trim_end());
            }
            None => {}
        }
        prev = Some(c);
    }
    None
}

/// `c` may directly precede the first character of a YAML scalar.
fn opens_scalar(c: char) -> bool {
    c.is_whitespace() || matches!(c, ':' | '[' | ',' | '{')
}

/// A line that continues the entry above it: indented or a `- ` list item.
/// A line that carries part of the entry above it: indented content (not an
/// indented comment, not whitespace) or a column-0 `- ` list item.
fn is_continuation(line: &str) -> bool {
    let content = line.trim_start();
    let indented = content.len() < line.len();
    (indented && !content.is_empty() && !content.starts_with('#'))
        || line.starts_with("- ")
        || line == "-"
}

/// A blank or comment line (indented or not): inside an entry only if a
/// continuation follows it.
fn is_neutral(line: &str) -> bool {
    let content = line.trim_start();
    content.is_empty() || content.starts_with('#')
}

/// Where a new entry goes: after the last entry (all of it — a block
/// scalar's indented comment-looking or blank lines are its text), before the
/// block's trailing blank and comment lines — so it never adopts a trailing
/// comment (which, sitting right above it, would go with it on removal).
fn append_at(lines: &[String]) -> usize {
    let after_content = lines
        .iter()
        .rposition(|l| !is_neutral(l))
        .map_or(0, |i| i + 1);
    let after_last_entry = lines
        .iter()
        .rposition(|l| line_key(l).is_some())
        .map_or(0, |i| entry_end(lines, i));
    after_content.max(after_last_entry)
}

/// The end (exclusive) of the entry whose key line is `start`. Its indented
/// and `- ` lines belong to it; blank and comment lines only when more of the
/// entry follows them. A block scalar (`key: |`, `key: >-`, …) owns every
/// indented or blank line below it, comment-looking ones included — they are
/// its text — trailing blank lines too (a `|+` scalar keeps them).
fn entry_end(lines: &[String], start: usize) -> usize {
    let block_scalar = lines[start]
        .split_once(':')
        .map(|(_, value)| value.trim_start())
        .is_some_and(|v| v.starts_with(['|', '>']));
    let mut end = start + 1;
    let mut probe = end;
    while probe < lines.len() {
        let line = &lines[probe];
        let owned = if block_scalar {
            line.trim().is_empty() || line.starts_with([' ', '\t'])
        } else {
            is_continuation(line)
        };
        if owned {
            probe += 1;
            if block_scalar || !line.trim().is_empty() {
                end = probe;
            }
        } else if !block_scalar && is_neutral(line) {
            probe += 1;
        } else {
            break;
        }
    }
    end
}

/// Start of the contiguous run of column-0 comment lines directly above
/// `start`; those comments belong to the entry at `start`.
fn leading_comments(lines: &[String], start: usize) -> usize {
    let mut s = start;
    while s > 0 && lines[s - 1].starts_with('#') {
        s -= 1;
    }
    s
}

/// Line ranges of every top-level entry whose key matches `key`
/// case-insensitively, with the key as written. Comment and blank lines are
/// part of an entry only when an indented / `- ` line follows them; trailing
/// ones are left out.
fn entry_ranges(lines: &[String], key: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        match line_key(&lines[i]) {
            Some(k) if keys_match(&k, key) => {
                let end = entry_end(lines, i);
                out.push((i..end, k));
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

fn join_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

/// `key` as an entry line spells it: plain when the line reader finds it
/// again as written (`-foo:` reads as a list item, `#tag:` as a comment, so
/// those are quoted), double-quoted otherwise.
fn render_key(key: &str) -> String {
    let readable = line_key(&format!("{key}: x")).as_deref() == Some(key);
    if plain_ok(key) && !key.contains(':') && readable {
        key.to_string()
    } else {
        double_quote(key)
    }
}

/// Byte offset of the `:` ending the key of top-level entry line `line`.
fn key_colon(line: &str) -> Option<usize> {
    for quote in ['"', '\''] {
        if let Some(rest) = line.strip_prefix(quote) {
            let end = rest.find(quote)? + 1; // index in `rest` past the closing quote
            let after = &rest[end..];
            let pad = after.len() - after.trim_start().len();
            return after.trim_start().starts_with(':').then_some(1 + end + pad);
        }
    }
    line.find(": ")
        .or_else(|| line.strip_suffix(':').map(str::len))
}

fn render_entry(key: &str, value: &PropertyValue) -> Vec<String> {
    let k = render_key(key);
    match value {
        PropertyValue::List(items) if items.is_empty() => vec![format!("{k}: []")],
        PropertyValue::List(items) => std::iter::once(format!("{k}:"))
            .chain(items.iter().map(|i| format!("  - {}", scalar(i))))
            .collect(),
        PropertyValue::Text(s) => vec![format!("{k}: {}", scalar(s))],
        PropertyValue::Number(n) => vec![format!("{k}: {}", format_number(*n))],
        PropertyValue::Bool(b) => vec![format!("{k}: {b}")],
        PropertyValue::Date(d) => vec![format!("{k}: {}", format_iso_date(*d))],
        PropertyValue::DateTime(dt) => vec![format!("{k}: {dt}")],
    }
}

fn scalar(s: &str) -> String {
    if plain_ok(s) {
        s.to_string()
    } else {
        double_quote(s)
    }
}

/// `s` can be written unquoted: a YAML parser reads `k: s` back as exactly
/// the string `s` (so not `true`, `12`, `null`, `[x]`, `a: b`, `x # y`, …).
fn plain_ok(s: &str) -> bool {
    if s.is_empty() || s.trim() != s || s.chars().any(char::is_control) {
        return false;
    }
    let Ok(docs) = YamlLoader::load_from_str(&format!("k: {s}")) else {
        return false;
    };
    match docs.first() {
        Some(Yaml::Hash(h)) => {
            h.len() == 1 && h.get(&Yaml::String("k".into())) == Some(&Yaml::String(s.to_string()))
        }
        _ => false,
    }
}

fn double_quote(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::super::tests::d;
    use super::super::PropertyDateTime;
    use super::*;

    fn parse(block: &str) -> Vec<(String, PropertyValue)> {
        YamlFormatter
            .parse(block)
            .unwrap()
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect()
    }

    #[test]
    fn yaml_types_map_to_property_values() {
        let p = parse(
            "Title: Hello\ncount: 3\nratio: 0.5\ndone: false\ndue: 2024-01-31\n\
             at: 2024-01-31T10:00:00Z\nobsidian_dt: 2024-01-15T14:30\ntags:\n  - a\n  - 2\nempty:\n",
        );
        assert_eq!(
            p,
            vec![
                ("Title".into(), PropertyValue::Text("Hello".into())),
                ("count".into(), PropertyValue::Number(3.0)),
                ("ratio".into(), PropertyValue::Number(0.5)),
                ("done".into(), PropertyValue::Bool(false)),
                ("due".into(), PropertyValue::Date(d(2024, 1, 31))),
                (
                    "at".into(),
                    PropertyValue::DateTime(
                        Utc.with_ymd_and_hms(2024, 1, 31, 10, 0, 0).unwrap().into()
                    )
                ),
                // Obsidian's Date & time property (no seconds, no offset).
                (
                    "obsidian_dt".into(),
                    PropertyValue::DateTime(PropertyDateTime::parse("2024-01-15T14:30").unwrap())
                ),
                (
                    "tags".into(),
                    PropertyValue::List(vec!["a".into(), "2".into()])
                ),
                // `empty:` (null) is skipped.
            ]
        );
    }

    #[test]
    fn short_date_like_text_stays_text() {
        assert_eq!(
            parse("v: 2024-1-1\n"),
            vec![("v".to_string(), PropertyValue::Text("2024-1-1".into()))]
        );
    }

    // A capitalized key with a block-list continuation is replaced in place.
    #[test]
    fn obsidian_entry_is_replaced_in_place() {
        let out = YamlFormatter
            .set(
                "title: T\nTags:\n  - a\n  - b\ncount: 1\n",
                "tags",
                &PropertyValue::List(vec!["c".into()]),
            )
            .unwrap();
        assert_eq!(out, "title: T\nTags:\n  - c\ncount: 1\n");
    }

    #[test]
    fn writes_obsidian_style_lists() {
        let out = YamlFormatter
            .set(
                "",
                "tags",
                &PropertyValue::List(vec!["a".into(), "b c".into()]),
            )
            .unwrap();
        assert_eq!(out, "tags:\n  - a\n  - b c\n");
    }

    #[test]
    fn quotes_text_that_would_not_read_back_as_itself() {
        let out = YamlFormatter
            .set("", "note", &PropertyValue::Text("x: y # z".into()))
            .unwrap();
        assert_eq!(out, "note: \"x: y # z\"\n");
        let out = YamlFormatter
            .set("", "flag", &PropertyValue::Text("true".into()))
            .unwrap();
        assert_eq!(out, "flag: \"true\"\n");
        assert_eq!(
            parse(&out),
            vec![("flag".to_string(), PropertyValue::Text("true".into()))]
        );
    }

    #[test]
    fn removes_a_multi_line_entry() {
        let out = YamlFormatter
            .remove("a: 1\nlist:\n  - x\nb: 2\n", "list")
            .unwrap()
            .unwrap();
        assert_eq!(out, "a: 1\nb: 2\n");
    }

    #[test]
    fn comment_inside_an_entry_does_not_strand_old_items() {
        let out = YamlFormatter
            .set(
                "x: 1\ntags:\n# c\n  - a\ny: 2\n",
                "tags",
                &PropertyValue::List(vec!["c".into()]),
            )
            .unwrap();
        assert_eq!(
            parse(&out)
                .into_iter()
                .find(|(k, _)| k == "tags")
                .map(|(_, v)| v),
            Some(PropertyValue::List(vec!["c".into()])),
            "{out}"
        );
    }

    #[test]
    fn flow_style_root_is_refused_for_edits() {
        let f = YamlFormatter;
        assert!(f.set("{a: 1}\n", "b", &PropertyValue::Bool(true)).is_err());
        assert!(f.remove("{a: 1}\n", "a").is_err());
    }

    #[test]
    fn keeps_the_inline_comment_of_a_replaced_entry() {
        let set = |block: &str, v: PropertyValue| YamlFormatter.set(block, "k", &v).unwrap();
        let text = |s: &str| PropertyValue::Text(s.into());
        assert_eq!(set("k: a # why\nz: 1\n", text("b")), "k: b # why\nz: 1\n");
        assert_eq!(set("k: a   # why\n", text("b")), "k: b   # why\n");
        // A `#` inside quotes or glued to a word is not a comment.
        assert_eq!(set("k: \"a # b\"\n", text("c")), "k: c\n");
        assert_eq!(set("k: a#b\n", text("c")), "k: c\n");
        assert_eq!(set("k: 'it''s' # c\n", text("d")), "k: d # c\n");
        assert_eq!(set("k: 'a''b # x' # c\n", text("d")), "k: d # c\n");
        assert_eq!(set("k: \"a\\\" # x\" # c\n", text("d")), "k: d # c\n");
        assert_eq!(set("k: it's # c\n", text("d")), "k: d # c\n");
        // The comment survives a change of shape, scalar to list and back.
        let list = set("k: a # c\n", PropertyValue::List(vec!["x".into()]));
        assert_eq!(list, "k: # c\n  - x\n");
        assert_eq!(set(&list, text("a")), "k: a # c\n");
    }

    #[test]
    fn comments_and_blank_lines_next_to_an_entry_survive_its_edit() {
        let set = |block: &str| {
            YamlFormatter
                .set(block, "title", &PropertyValue::Text("Y".into()))
                .unwrap()
        };
        assert_eq!(
            set("title: X\n# draft: true\n  \n"),
            "title: Y\n# draft: true\n  \n"
        );
        assert_eq!(
            set("title: X\n  # reviewed by Ana\nnext: 1\n"),
            "title: Y\n  # reviewed by Ana\nnext: 1\n"
        );
    }

    #[test]
    fn a_new_key_does_not_adopt_a_trailing_comment() {
        let block = "a: 1\n# trailing note\n";
        let out = YamlFormatter
            .set(block, "b", &PropertyValue::Number(2.0))
            .unwrap();
        assert_eq!(out, "a: 1\nb: 2\n# trailing note\n");
        let removed = YamlFormatter.remove(&out, "b").unwrap().unwrap();
        assert_eq!(removed, block, "removing it again keeps the comment");
    }

    #[test]
    fn a_key_the_line_reader_cant_see_is_written_quoted_and_stays_editable() {
        for key in ["-foo", "#tag", "? q"] {
            let once = YamlFormatter
                .set("", key, &PropertyValue::Number(1.0))
                .unwrap();
            let twice = YamlFormatter
                .set(&once, key, &PropertyValue::Number(2.0))
                .unwrap_or_else(|e| panic!("{key:?} must stay editable: {e:?}\n{once}"));
            assert_eq!(
                YamlFormatter.parse(&twice).unwrap(),
                vec![(key.to_string(), Some(PropertyValue::Number(2.0)))],
                "{twice}"
            );
            assert!(YamlFormatter.remove(&twice, key).unwrap().is_some());
        }
    }

    #[test]
    fn edits_never_change_a_neighbouring_block_scalar() {
        let entries = |block: &str| YamlFormatter.parse(block).unwrap();
        for block in [
            "a: 1\ndesc: |\n  text\n  # last\n",
            "desc: |+\n  text\n\n",
            "desc: >\n  folded\n\n  more\nnext: 1\n",
        ] {
            let out = YamlFormatter
                .set(block, "b", &PropertyValue::Number(2.0))
                .unwrap();
            let mut expected = entries(block);
            expected.push(("b".to_string(), Some(PropertyValue::Number(2.0))));
            assert_eq!(entries(&out), expected, "{block:?} -> {out:?}");
        }
    }

    #[test]
    fn a_block_scalar_entry_is_replaced_or_removed_whole() {
        let block = "desc: |\n  text\n  # last\nnext: 1\n";
        assert_eq!(
            YamlFormatter.remove(block, "desc").unwrap().unwrap(),
            "next: 1\n"
        );
        let out = YamlFormatter
            .set(block, "desc", &PropertyValue::Text("x".into()))
            .unwrap();
        assert_eq!(out, "desc: x\nnext: 1\n");
    }

    #[test]
    fn an_edit_never_changes_a_nested_mapping() {
        let block = "meta:\n  desc: |\n    x\n    # y\n";
        let err = YamlFormatter
            .set(block, "b", &PropertyValue::Number(2.0))
            .map(|out| (out.clone(), root(&out).unwrap() == root(block).unwrap()));
        // Either the edit keeps `meta` exactly, or it is refused.
        if let Ok((out, _)) = &err {
            let before = root(block).unwrap().unwrap();
            let after = root(out).unwrap().unwrap();
            let meta = Yaml::String("meta".into());
            assert_eq!(before.get(&meta), after.get(&meta), "{out:?}");
        }
    }
}
