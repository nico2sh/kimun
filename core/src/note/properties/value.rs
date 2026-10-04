//! The property value model: [`PropertyValue`], its [`PropertyKind`], and the
//! rules for turning user-typed text into a value — inferred, or forced to a
//! kind. Shared by every caller that takes property values as text (the CLI,
//! the MCP server), so they all type input the same way.

use std::fmt;
use std::str::FromStr;

use chrono::NaiveDate;

use super::{format_number, is_list_key, is_tag_key, split_tags, PropertyDateTime};
use crate::dates::{format_iso_date, parse_iso_date};

/// A typed frontmatter property value — the neutral model every
/// `PropertyFormatter` parses into and writes from. Mirrors Obsidian's
/// property types. Serializes as the plain JSON value (`"done"`, `2`, `true`,
/// `"2024-03-01"`, `["a", "b"]`).
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    /// Free text.
    Text(String),
    /// Any number. Integers beyond 2^53 lose exactness (as in Obsidian).
    Number(f64),
    /// `true` / `false`.
    Bool(bool),
    /// A calendar date with no time component.
    Date(NaiveDate),
    /// A date and time, local or with an offset, as written.
    DateTime(PropertyDateTime),
    /// A list of text items (Obsidian's List type); non-text items are
    /// stringified.
    List(Vec<String>),
}

/// The type of a [`PropertyValue`], without the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyKind {
    /// [`PropertyValue::Text`]
    Text,
    /// [`PropertyValue::Number`]
    Number,
    /// [`PropertyValue::Bool`]
    Bool,
    /// [`PropertyValue::Date`]
    Date,
    /// [`PropertyValue::DateTime`]
    DateTime,
    /// [`PropertyValue::List`]
    List,
}

impl PropertyKind {
    /// Every kind, in declaration order.
    pub const ALL: [PropertyKind; 6] = [
        PropertyKind::Text,
        PropertyKind::Number,
        PropertyKind::Bool,
        PropertyKind::Date,
        PropertyKind::DateTime,
        PropertyKind::List,
    ];

    /// The kind's name as written by users and stored in the index:
    /// `text`, `number`, `bool`, `date`, `datetime`, `list`.
    pub fn as_str(self) -> &'static str {
        match self {
            PropertyKind::Text => "text",
            PropertyKind::Number => "number",
            PropertyKind::Bool => "bool",
            PropertyKind::Date => "date",
            PropertyKind::DateTime => "datetime",
            PropertyKind::List => "list",
        }
    }

    /// A human description of values of this kind, for messages
    /// ("holds number values").
    fn describe(self) -> &'static str {
        match self {
            PropertyKind::Text => "text",
            PropertyKind::Number => "number",
            PropertyKind::Bool => "true/false",
            PropertyKind::Date => "date",
            PropertyKind::DateTime => "date-time",
            PropertyKind::List => "list",
        }
    }
}

impl fmt::Display for PropertyKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PropertyKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| {
                let names: Vec<&str> = Self::ALL.iter().map(|k| k.as_str()).collect();
                format!(
                    "unknown property type '{s}' (expected one of: {})",
                    names.join(", ")
                )
            })
    }
}

impl PropertyValue {
    /// This value's kind.
    pub fn kind(&self) -> PropertyKind {
        match self {
            PropertyValue::Text(_) => PropertyKind::Text,
            PropertyValue::Number(_) => PropertyKind::Number,
            PropertyValue::Bool(_) => PropertyKind::Bool,
            PropertyValue::Date(_) => PropertyKind::Date,
            PropertyValue::DateTime(_) => PropertyKind::DateTime,
            PropertyValue::List(_) => PropertyKind::List,
        }
    }

    /// Types one user-typed value by its look, never changing what it says:
    /// a number only when writing it back reproduces the input exactly (`5`,
    /// `4.5` — not `02134`, `1.10` or `1e3`), `true`/`false` as a bool, an exact
    /// `YYYY-MM-DD` as a date, an RFC3339 or `YYYY-MM-DDTHH:MM[:SS]` date-time
    /// (its offset, or lack of one, kept), and anything else as text.
    pub fn infer(raw: &str) -> PropertyValue {
        [
            PropertyKind::Number,
            PropertyKind::Bool,
            PropertyKind::Date,
            PropertyKind::DateTime,
        ]
        .into_iter()
        .find_map(|kind| Self::parse_one_exact(kind, raw))
        .unwrap_or_else(|| PropertyValue::Text(raw.to_string()))
    }

    /// `values` as a value of `kind`, read strictly: only input that a value
    /// of that kind writes back unchanged (see [`Self::infer`]). A list takes
    /// every value, any other kind exactly one.
    fn parse_exact(kind: PropertyKind, values: &[String]) -> Option<PropertyValue> {
        match (kind, values) {
            (PropertyKind::List, _) => Some(PropertyValue::List(values.to_vec())),
            (_, [raw]) => Self::parse_one_exact(kind, raw),
            _ => None,
        }
    }

    fn parse_one_exact(kind: PropertyKind, raw: &str) -> Option<PropertyValue> {
        match kind {
            PropertyKind::Text => Some(PropertyValue::Text(raw.to_string())),
            PropertyKind::Number => raw
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && format_number(*n) == raw)
                .map(PropertyValue::Number),
            PropertyKind::Bool => match raw {
                "true" => Some(PropertyValue::Bool(true)),
                "false" => Some(PropertyValue::Bool(false)),
                _ => None,
            },
            PropertyKind::Date => parse_iso_date(raw)
                .filter(|d| format_iso_date(*d) == raw)
                .map(PropertyValue::Date),
            PropertyKind::DateTime => PropertyDateTime::parse(raw).map(PropertyValue::DateTime),
            PropertyKind::List => Some(PropertyValue::List(vec![raw.to_string()])),
        }
    }

    /// `values` as a value of `kind`, read leniently — for a type the user
    /// forced, where `02134` as a number means 2134. A list takes every value
    /// (none makes an empty list), any other kind exactly one. `Err` is a
    /// user-facing reason (not a number, not a date, …).
    pub fn parse_as(kind: PropertyKind, values: &[String]) -> Result<PropertyValue, String> {
        if kind == PropertyKind::List {
            return Ok(PropertyValue::List(values.to_vec()));
        }
        let [raw] = values else {
            return Err(format!(
                "a {} property takes exactly one value, got {}",
                kind.describe(),
                values.len()
            ));
        };
        let raw = raw.trim();
        let invalid = || format!("\"{raw}\" is not a {}", kind.describe());
        match kind {
            PropertyKind::Number => raw
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .map(PropertyValue::Number)
                .ok_or_else(invalid),
            PropertyKind::Bool => {
                Self::parse_one_exact(kind, &raw.to_lowercase()).ok_or_else(invalid)
            }
            // Text keeps the input untrimmed; dates parse exactly.
            PropertyKind::Text => Ok(PropertyValue::Text(values[0].clone())),
            _ => Self::parse_one_exact(kind, raw).ok_or_else(invalid),
        }
    }
}

/// Property values as a user typed them, plus what is known of their type:
/// *forced* (the user said so — always wins) or *implied* by the input's own
/// shape (a JSON number, boolean or array — checked against the vault like a
/// guess). The one shape every text front end (CLI, MCP) hands to core.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropertyInput {
    values: Vec<String>,
    forced: Option<PropertyKind>,
    implied: Option<PropertyKind>,
}

impl PropertyInput {
    /// Input of `values`, its type to be decided.
    pub fn new(values: Vec<String>) -> Self {
        Self {
            values,
            ..Self::default()
        }
    }

    /// The user's explicit type, if any: it always wins.
    pub fn forced(mut self, kind: Option<PropertyKind>) -> Self {
        self.forced = kind;
        self
    }

    /// The type the input's own shape carries (e.g. a JSON number).
    pub fn implied(mut self, kind: PropertyKind) -> Self {
        self.implied = Some(kind);
        self
    }

    /// The type the user forced, if any.
    pub fn forced_kind(&self) -> Option<PropertyKind> {
        self.forced
    }

    /// The value to store for `key` (normalized), whose values in other notes
    /// are mostly of `vault_kind`:
    /// - a forced type always wins (read leniently);
    /// - `tags` / `tag` are always a list — a single comma-separated value
    ///   split into items, several values kept as given; `aliases` and `cssclasses` are always a list, values as given;
    /// - otherwise the input must fit the vault's type: exactly a value of it
    ///   (a single value for a list key is a one-item list), or — when its
    ///   implied type *is* the vault's — any value of that type; an array
    ///   never fits a single-value key. `Err` explains a mismatch;
    /// - with no vault type, an implied type is used, else one value is
    ///   [inferred](PropertyValue::infer) and several make a list.
    pub fn resolve(
        &self,
        key: &str,
        vault_kind: Option<PropertyKind>,
    ) -> Result<PropertyValue, String> {
        if let Some(kind) = self.forced {
            return PropertyValue::parse_as(kind, &self.values);
        }
        if is_tag_key(key) {
            // One `"work, q1"` is two tags, as it reads back from a bare
            // string; an explicit list (several values, or an array) keeps its
            // items as a list item does — trimmed, empties dropped.
            let tags = match (self.implied, self.values.as_slice()) {
                (implied, [one]) if implied != Some(PropertyKind::List) => {
                    split_tags(one).map(str::to_string).collect()
                }
                (_, items) => items
                    .iter()
                    .map(|t| t.trim())
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .collect(),
            };
            return Ok(PropertyValue::List(tags));
        }
        if is_list_key(key) {
            return Ok(PropertyValue::List(self.values.clone()));
        }
        if let Some(vault_kind) = vault_kind {
            let fits = match self.implied {
                // The input's own type is the vault's: trust it (`4.0` is a
                // JSON number, not text that only looks numeric).
                Some(implied) if implied == vault_kind => {
                    PropertyValue::parse_as(implied, &self.values).ok()
                }
                // An array never fits a key that holds single values.
                Some(PropertyKind::List) => None,
                _ => PropertyValue::parse_exact(vault_kind, &self.values),
            };
            return fits.ok_or_else(|| {
                let given = match self.values.as_slice() {
                    [] => "no value was given".to_string(),
                    [raw] => format!("\"{raw}\" is not one"),
                    values => format!("{} values make a list", values.len()),
                };
                format!(
                    "it holds {} values in this vault and {given}; set the type explicitly to store it anyway",
                    vault_kind.describe()
                )
            });
        }
        match (self.implied, self.values.as_slice()) {
            (Some(kind), _) => PropertyValue::parse_as(kind, &self.values),
            (None, []) => Err("no value given".to_string()),
            (None, [raw]) => Ok(PropertyValue::infer(raw)),
            (None, values) => Ok(PropertyValue::List(values.to_vec())),
        }
    }
}

/// The value as plain text: canonical numbers/dates, list items joined with
/// `, ` (an empty list as `[]`).
impl fmt::Display for PropertyValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyValue::Text(s) => f.write_str(s),
            PropertyValue::Number(n) => f.write_str(&format_number(*n)),
            PropertyValue::Bool(b) => write!(f, "{b}"),
            PropertyValue::Date(d) => f.write_str(&format_iso_date(*d)),
            PropertyValue::DateTime(dt) => write!(f, "{dt}"),
            PropertyValue::List(items) if items.is_empty() => f.write_str("[]"),
            PropertyValue::List(items) => f.write_str(&items.join(", ")),
        }
    }
}

impl serde::Serialize for PropertyValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PropertyValue::Text(s) => serializer.serialize_str(s),
            PropertyValue::Number(n) if super::is_integral(*n) => {
                serializer.serialize_i64(*n as i64)
            }
            PropertyValue::Number(n) => serializer.serialize_f64(*n),
            PropertyValue::Bool(b) => serializer.serialize_bool(*b),
            PropertyValue::Date(_) | PropertyValue::DateTime(_) => {
                serializer.serialize_str(&self.to_string())
            }
            PropertyValue::List(items) => items.serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    fn dt(s: &str) -> PropertyValue {
        PropertyValue::DateTime(PropertyDateTime::parse(s).unwrap())
    }

    #[test]
    fn infer_types_by_look() {
        assert_eq!(PropertyValue::infer("5"), PropertyValue::Number(5.0));
        assert_eq!(PropertyValue::infer("-4.5"), PropertyValue::Number(-4.5));
        assert_eq!(PropertyValue::infer("true"), PropertyValue::Bool(true));
        assert_eq!(
            PropertyValue::infer("2024-03-01"),
            PropertyValue::Date(d(2024, 3, 1))
        );
        assert_eq!(
            PropertyValue::infer("2024-03-01T14:30"),
            dt("2024-03-01T14:30")
        );
        assert_eq!(
            PropertyValue::infer("done"),
            PropertyValue::Text("done".into())
        );
    }

    #[test]
    fn infer_never_changes_what_was_typed() {
        for raw in [
            "02134",
            "1.10",
            "1e3",
            "+5",
            "5.0",
            " 5",
            "True",
            "2024-3-1",
            "0612345678x",
        ] {
            assert_eq!(
                PropertyValue::infer(raw),
                PropertyValue::Text(raw.into()),
                "{raw:?} must stay text"
            );
        }
        assert_eq!(
            PropertyValue::infer("2024-03-01T14:30").to_string(),
            "2024-03-01T14:30",
            "a local date-time stays local"
        );
        assert_eq!(
            PropertyValue::infer("2024-03-01T14:30:00+02:00").to_string(),
            "2024-03-01T14:30:00+02:00",
            "an offset is kept"
        );
    }

    #[test]
    fn parse_as_forces_a_kind_leniently() {
        let one = |s: &str| strings(&[s]);
        assert_eq!(
            PropertyValue::parse_as(PropertyKind::Text, &one("2024")),
            Ok(PropertyValue::Text("2024".into()))
        );
        assert_eq!(
            PropertyValue::parse_as(PropertyKind::Number, &one("02134")),
            Ok(PropertyValue::Number(2134.0))
        );
        assert_eq!(
            PropertyValue::parse_as(PropertyKind::Bool, &one("TRUE")),
            Ok(PropertyValue::Bool(true))
        );
        assert_eq!(
            PropertyValue::parse_as(PropertyKind::List, &[]),
            Ok(PropertyValue::List(vec![])),
            "no values make an empty list"
        );
        assert!(PropertyValue::parse_as(PropertyKind::Number, &one("high"))
            .unwrap_err()
            .contains("not a number"));
        assert!(PropertyValue::parse_as(PropertyKind::Date, &one("2024-02-30")).is_err());
        assert!(PropertyValue::parse_as(PropertyKind::Text, &strings(&["a", "b"])).is_err());
    }

    fn input(values: &[&str]) -> PropertyInput {
        PropertyInput::new(strings(values))
    }

    #[test]
    fn resolve_follows_the_vault_kind_exactly() {
        let num = Some(PropertyKind::Number);
        assert_eq!(
            input(&["5"]).resolve("p", num),
            Ok(PropertyValue::Number(5.0))
        );
        let err = input(&["high"]).resolve("priority", num).unwrap_err();
        assert!(err.starts_with("it holds number values"), "{err}");
        assert!(
            input(&["02134"]).resolve("id", num).is_err(),
            "a numeric key never rewrites text that only looks numeric"
        );
        assert_eq!(
            input(&["high"])
                .forced(Some(PropertyKind::Text))
                .resolve("p", num),
            Ok(PropertyValue::Text("high".into())),
            "a forced kind wins"
        );
        assert_eq!(
            input(&["2024"]).resolve("v", Some(PropertyKind::Text)),
            Ok(PropertyValue::Text("2024".into()))
        );
        assert_eq!(
            input(&["garden"]).resolve("k", Some(PropertyKind::List)),
            Ok(PropertyValue::List(vec!["garden".into()]))
        );
        assert!(input(&["1", "2"])
            .resolve("p", num)
            .unwrap_err()
            .contains("2 values make a list"));
    }

    #[test]
    fn resolve_checks_implied_kinds_against_the_vault() {
        let as_number = |raw: &str| input(&[raw]).implied(PropertyKind::Number);
        assert_eq!(
            as_number("2134").resolve("zip", Some(PropertyKind::Text)),
            Ok(PropertyValue::Text("2134".into())),
            "a JSON number for a text key is stored as that key's text"
        );
        assert!(input(&["true"])
            .implied(PropertyKind::Bool)
            .resolve("n", Some(PropertyKind::Number))
            .is_err());
        assert_eq!(
            as_number("7").resolve("new", None),
            Ok(PropertyValue::Number(7.0)),
            "with no vault kind the implied one is used"
        );
    }

    #[test]
    fn an_implied_kind_matching_the_vault_is_trusted() {
        let num = Some(PropertyKind::Number);
        for raw in ["4.0", "1e3", "-0.0"] {
            assert!(
                matches!(
                    input(&[raw])
                        .implied(PropertyKind::Number)
                        .resolve("rating", num),
                    Ok(PropertyValue::Number(_))
                ),
                "a JSON number {raw} for a number key"
            );
        }
        let err = input(&["5"])
            .implied(PropertyKind::List)
            .resolve("rating", num)
            .unwrap_err();
        assert!(
            err.contains("holds number values"),
            "an array for a scalar key: {err}"
        );
        assert_eq!(
            input(&["5"])
                .implied(PropertyKind::Number)
                .resolve("k", Some(PropertyKind::List)),
            Ok(PropertyValue::List(vec!["5".into()])),
            "a scalar for a list key is still a one-item list"
        );
    }

    #[test]
    fn comma_separated_tags_input_is_split_into_items() {
        assert_eq!(
            input(&["work, q1"]).resolve("tags", None),
            Ok(PropertyValue::List(vec!["work".into(), "q1".into()]))
        );
        assert_eq!(
            input(&["a,b", "big project"]).resolve("tags", None),
            Ok(PropertyValue::List(vec![
                "a,b".into(),
                "big project".into()
            ])),
            "an explicit list keeps its items, as a list item reads back"
        );
        assert_eq!(
            input(&["a, b"]).resolve("other", Some(PropertyKind::Text)),
            Ok(PropertyValue::Text("a, b".into())),
            "only tags are split"
        );
    }

    #[test]
    fn an_explicit_tags_list_is_cleaned_but_never_split() {
        assert_eq!(
            input(&["work", " q1 ", ""]).resolve("tags", None),
            Ok(PropertyValue::List(vec!["work".into(), "q1".into()]))
        );
        assert_eq!(
            input(&["a, b"])
                .implied(PropertyKind::List)
                .resolve("tags", None),
            Ok(PropertyValue::List(vec!["a, b".into()])),
            "a one-item array is a list, not a comma-separated string"
        );
    }

    #[test]
    fn obsidian_list_keys_are_always_lists() {
        for key in ["aliases", "Aliases", "cssclasses", "alias", "cssclass"] {
            assert_eq!(
                input(&["Smith, John"]).resolve(key, Some(PropertyKind::Text)),
                Ok(PropertyValue::List(vec!["Smith, John".into()])),
                "{key}: a list, and only tags split on commas"
            );
        }
        assert_eq!(
            input(&["Bob", "Rob"]).resolve("aliases", None),
            Ok(PropertyValue::List(vec!["Bob".into(), "Rob".into()]))
        );
    }

    #[test]
    fn tags_are_always_a_list_and_lists_may_be_empty() {
        assert_eq!(
            input(&["garden"]).resolve("tags", None),
            Ok(PropertyValue::List(vec!["garden".into()]))
        );
        assert_eq!(
            input(&["x", "y"]).resolve("tag", Some(PropertyKind::Text)),
            Ok(PropertyValue::List(vec!["x".into(), "y".into()]))
        );
        assert_eq!(
            input(&[]).resolve("tags", None),
            Ok(PropertyValue::List(vec![]))
        );
        assert_eq!(
            input(&[]).implied(PropertyKind::List).resolve("k", None),
            Ok(PropertyValue::List(vec![]))
        );
        assert!(input(&[]).resolve("k", None).is_err());
    }

    #[test]
    fn kind_names_round_trip() {
        for kind in PropertyKind::ALL {
            assert_eq!(kind.as_str().parse::<PropertyKind>(), Ok(kind));
        }
        assert!("Number".parse::<PropertyKind>().is_ok());
        assert!("float"
            .parse::<PropertyKind>()
            .unwrap_err()
            .contains("expected one of"));
    }

    #[test]
    fn display_and_json_are_plain() {
        let utc =
            PropertyValue::DateTime(Utc.with_ymd_and_hms(2024, 3, 1, 14, 30, 0).unwrap().into());
        let cases = [
            (PropertyValue::Text("done".into()), "done", r#""done""#),
            (PropertyValue::Number(2.0), "2", "2"),
            (PropertyValue::Number(4.5), "4.5", "4.5"),
            (PropertyValue::Bool(true), "true", "true"),
            (
                PropertyValue::Date(d(2024, 3, 1)),
                "2024-03-01",
                r#""2024-03-01""#,
            ),
            (utc, "2024-03-01T14:30:00Z", r#""2024-03-01T14:30:00Z""#),
            (
                dt("2024-03-01T14:30"),
                "2024-03-01T14:30",
                r#""2024-03-01T14:30""#,
            ),
            (
                PropertyValue::List(vec!["a".into(), "b c".into()]),
                "a, b c",
                r#"["a","b c"]"#,
            ),
            (PropertyValue::List(vec![]), "[]", "[]"),
        ];
        for (value, text, json) in cases {
            assert_eq!(value.to_string(), text);
            assert_eq!(serde_json::to_string(&value).unwrap(), json);
        }
    }
}
