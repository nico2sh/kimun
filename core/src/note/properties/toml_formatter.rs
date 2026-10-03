//! TOML (`+++`) frontmatter, read and edited through `toml_edit`'s document
//! model, which keeps comments, key order and formatting of everything an
//! edit does not touch.

use chrono::{FixedOffset, NaiveDate, NaiveTime, TimeZone};
use toml_edit::{DocumentMut, Item, Value};

use super::{
    finite, format_number, is_integral, keys_match, nested_key_error, FrontmatterError,
    PropertyDateTime, PropertyEntry, PropertyFormatter, PropertyValue,
};
use crate::dates::format_iso_date;

/// The [`PropertyFormatter`] for `+++` blocks.
pub(super) struct TomlFormatter;

impl PropertyFormatter for TomlFormatter {
    fn parse(&self, block: &str) -> Result<Vec<PropertyEntry>, FrontmatterError> {
        let doc = document(block)?;
        // A `[table]` / `[[array]]` item is not a value, so not a property.
        Ok(doc
            .iter()
            .filter_map(|(k, item)| Some((k.to_string(), read_value(item.as_value()?))))
            .collect())
    }

    fn set(
        &self,
        block: &str,
        key: &str,
        value: &PropertyValue,
    ) -> Result<String, FrontmatterError> {
        let mut doc = document(block)?;
        let matches = matching_keys(&doc, key)?;
        let target = matches.first().cloned().unwrap_or_else(|| key.to_string());
        for k in matches.iter().skip(1) {
            remove_entry(&mut doc, k);
        }
        let mut new = write_value(value)?;
        // A `# comment` trailing the old value stays on its line.
        if let Some(suffix) = doc
            .get(target.as_str())
            .and_then(Item::as_value)
            .and_then(|old| old.decor().suffix())
            .filter(|s| s.as_str().is_some_and(|s| s.contains('#')))
        {
            new.decor_mut().set_suffix(suffix.clone());
        }
        doc[target.as_str()] = Item::Value(new);
        Ok(doc.to_string())
    }

    fn remove(&self, block: &str, key: &str) -> Result<Option<String>, FrontmatterError> {
        let mut doc = document(block)?;
        let matches = matching_keys(&doc, key)?;
        if matches.is_empty() {
            return Ok(None);
        }
        for k in &matches {
            remove_entry(&mut doc, k);
        }
        Ok(Some(doc.to_string()))
    }
}

/// Removes root key `key`. toml_edit keeps every comment above a key in its
/// decor prefix; the contract says only the lines *directly* above go with it,
/// so the part of the prefix up to its last blank line moves onto whatever
/// follows (the next entry, or the document's trailing text).
fn remove_entry(doc: &mut DocumentMut, key: &str) {
    let keep = doc
        .as_table()
        .key(key)
        .and_then(|k| k.leaf_decor().prefix())
        .and_then(|p| p.as_str())
        .map(through_last_blank_line)
        .unwrap_or_default()
        .to_string();
    let next = doc
        .as_table()
        .iter()
        .map(|(k, _)| k.to_string())
        .skip_while(|k| k != key)
        .nth(1);
    doc.remove(key);
    if keep.is_empty() {
        return;
    }
    let Some(next) = next else {
        let trailing = format!("{keep}{}", doc.trailing().as_str().unwrap_or_default());
        doc.set_trailing(trailing);
        return;
    };
    let Some((mut k, item)) = doc.as_table_mut().get_key_value_mut(&next) else {
        return;
    };
    let decor = match item {
        Item::Table(t) => t.decor_mut(),
        _ => k.leaf_decor_mut(),
    };
    let prefix = format!(
        "{keep}{}",
        decor.prefix().and_then(|p| p.as_str()).unwrap_or_default()
    );
    decor.set_prefix(prefix);
}

/// `text` up to and including its last blank (whitespace-only) line.
fn through_last_blank_line(text: &str) -> &str {
    let mut end = 0;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        at += line.len();
        if line.ends_with('\n') && line.trim().is_empty() {
            end = at;
        }
    }
    &text[..end]
}

fn document(block: &str) -> Result<DocumentMut, FrontmatterError> {
    block
        .parse::<DocumentMut>()
        .map_err(|e| FrontmatterError(e.to_string()))
}

/// Root keys equal to `key` case-insensitively, as written. A match holding a
/// table (not a value) refuses the edit.
fn matching_keys(doc: &DocumentMut, key: &str) -> Result<Vec<String>, FrontmatterError> {
    let mut keys = Vec::new();
    for (k, item) in doc.iter() {
        if keys_match(k, key) {
            if !item.is_value() {
                return Err(nested_key_error(key));
            }
            keys.push(k.to_string());
        }
    }
    Ok(keys)
}

fn read_value(v: &Value) -> Option<PropertyValue> {
    match v {
        Value::String(s) => Some(PropertyValue::Text(s.value().clone())),
        Value::Integer(i) => Some(PropertyValue::Number(*i.value() as f64)),
        Value::Float(f) => finite(*f.value()).map(PropertyValue::Number),
        Value::Boolean(b) => Some(PropertyValue::Bool(*b.value())),
        Value::Datetime(dt) => read_datetime(dt.value()),
        Value::Array(items) => Some(PropertyValue::List(
            items.iter().filter_map(read_list_item).collect(),
        )),
        Value::InlineTable(_) => None,
    }
}

fn read_list_item(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.value().clone()),
        Value::Integer(i) => Some(i.value().to_string()),
        Value::Float(f) => Some(format_number(*f.value())),
        Value::Boolean(b) => Some(b.value().to_string()),
        Value::Datetime(dt) => Some(dt.value().to_string()),
        Value::Array(_) | Value::InlineTable(_) => None,
    }
}

/// TOML's four datetime kinds: local date → `Date`; offset and local
/// date-time → `DateTime`, keeping its offset or lack of one; local time →
/// unsupported.
fn read_datetime(dt: &toml_edit::Datetime) -> Option<PropertyValue> {
    let date = dt.date?;
    let day = NaiveDate::from_ymd_opt(date.year.into(), date.month.into(), date.day.into())?;
    let Some(time) = dt.time else {
        return Some(PropertyValue::Date(day));
    };
    let clock = NaiveTime::from_hms_nano_opt(
        time.hour.into(),
        time.minute.into(),
        time.second.unwrap_or(0).into(),
        time.nanosecond.unwrap_or(0),
    )?;
    let naive = day.and_time(clock);
    let offset_minutes = match dt.offset {
        None => return Some(PropertyValue::DateTime(PropertyDateTime::local(naive))),
        Some(toml_edit::Offset::Z) => 0,
        Some(toml_edit::Offset::Custom { minutes }) => minutes,
    };
    let offset = FixedOffset::east_opt(i32::from(offset_minutes) * 60)?;
    let with_offset = offset.from_local_datetime(&naive).single()?;
    Some(PropertyValue::DateTime(PropertyDateTime::with_offset(
        with_offset,
    )))
}

fn write_value(value: &PropertyValue) -> Result<Value, FrontmatterError> {
    let datetime = |s: String| {
        s.parse::<toml_edit::Datetime>()
            .map(Value::from)
            .map_err(|e| FrontmatterError(e.to_string()))
    };
    Ok(match value {
        PropertyValue::Text(s) => s.as_str().into(),
        PropertyValue::Number(n) if is_integral(*n) => (*n as i64).into(),
        PropertyValue::Number(n) => (*n).into(),
        PropertyValue::Bool(b) => (*b).into(),
        PropertyValue::Date(d) => datetime(format_iso_date(*d))?,
        PropertyValue::DateTime(dt) => datetime(dt.to_string_with_seconds())?,
        PropertyValue::List(items) => Value::Array(items.iter().map(String::as_str).collect()),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::d;
    use super::*;

    fn parse(block: &str) -> Vec<(String, PropertyValue)> {
        TomlFormatter
            .parse(block)
            .unwrap()
            .into_iter()
            .filter_map(|(k, v)| Some((k, v?)))
            .collect()
    }

    #[test]
    fn toml_types_map_to_property_values() {
        let p = parse(
            "title = \"Hello\"\ncount = 3\nratio = 0.5\ndone = true\ndue = 2024-01-31\n\
             at = 2024-01-31T10:00:00+02:00\nlocal = 2024-01-31T10:00:00\ntags = [\"a\", 2, true]\n",
        );
        assert_eq!(
            p,
            vec![
                ("title".into(), PropertyValue::Text("Hello".into())),
                ("count".into(), PropertyValue::Number(3.0)),
                ("ratio".into(), PropertyValue::Number(0.5)),
                ("done".into(), PropertyValue::Bool(true)),
                ("due".into(), PropertyValue::Date(d(2024, 1, 31))),
                (
                    "at".into(),
                    PropertyValue::DateTime(
                        PropertyDateTime::parse("2024-01-31T10:00:00+02:00").unwrap()
                    )
                ),
                (
                    "local".into(),
                    PropertyValue::DateTime(PropertyDateTime::parse("2024-01-31T10:00").unwrap())
                ),
                (
                    "tags".into(),
                    PropertyValue::List(vec!["a".into(), "2".into(), "true".into()])
                ),
            ]
        );
    }

    #[test]
    fn unsupported_values_are_skipped_individually() {
        assert_eq!(
            parse("bad = nan\nworse = inf\nt = 10:00:00\nok = 1\n"),
            vec![("ok".to_string(), PropertyValue::Number(1.0))]
        );
    }

    #[test]
    fn dates_are_written_as_native_toml() {
        let out = TomlFormatter
            .set("", "due", &PropertyValue::Date(d(2024, 1, 31)))
            .unwrap();
        assert_eq!(out, "due = 2024-01-31\n");
    }

    #[test]
    fn new_keys_land_before_sub_tables() {
        let out = TomlFormatter
            .set("a = 1\n[meta]\nx = 1\n", "b", &PropertyValue::Bool(true))
            .unwrap();
        assert_eq!(parse(&out).len(), 2, "{out}");
        assert!(
            out.find("b = true").unwrap() < out.find("[meta]").unwrap(),
            "{out}"
        );
    }

    #[test]
    fn keeps_the_inline_comment_of_a_replaced_value() {
        let out = TomlFormatter
            .set(
                "a = \"x\" # why\nb = 1 # also\n",
                "A",
                &PropertyValue::Text("y".into()),
            )
            .unwrap();
        assert_eq!(out, "a = \"y\" # why\nb = 1 # also\n");
        let out = TomlFormatter
            .set(
                "tags = [\"a\"] # t\n",
                "tags",
                &PropertyValue::List(vec!["b".into(), "c".into()]),
            )
            .unwrap();
        assert_eq!(out, "tags = [\"b\", \"c\"] # t\n");
        // No comment, no stray whitespace.
        let out = TomlFormatter
            .set("a = 1\n", "a", &PropertyValue::Bool(true))
            .unwrap();
        assert_eq!(out, "a = true\n");
    }
}
