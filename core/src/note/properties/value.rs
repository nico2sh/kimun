//! The property value model: [`PropertyValue`], its [`PropertyKind`], and the
//! rules for turning user-typed text into a value — inferred, or forced to a
//! kind. Shared by every caller that takes property values as text (the CLI,
//! the MCP server), so they all type input the same way.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};

use super::{format_datetime, format_number, parse_datetime};
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
    /// A date and time; offset-less sources are read as UTC.
    DateTime(DateTime<Utc>),
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

    /// Types one user-typed value by its look, never changing what was typed:
    /// a number only when writing it back reproduces the input exactly (`5`,
    /// `4.5` — not `02134`, `1.10` or `1e3`), `true`/`false` as a bool, an exact
    /// `YYYY-MM-DD` as a date, an RFC3339 or `YYYY-MM-DDTHH:MM[:SS]` date-time,
    /// and anything else as text.
    pub fn infer(raw: &str) -> PropertyValue {
        if let Ok(n) = raw.parse::<f64>() {
            if n.is_finite() && format_number(n) == raw {
                return PropertyValue::Number(n);
            }
        }
        match raw {
            "true" => return PropertyValue::Bool(true),
            "false" => return PropertyValue::Bool(false),
            _ => {}
        }
        if let Some(date) = parse_iso_date(raw).filter(|d| format_iso_date(*d) == raw) {
            return PropertyValue::Date(date);
        }
        match parse_datetime(raw) {
            Some(dt) => PropertyValue::DateTime(dt),
            None => PropertyValue::Text(raw.to_string()),
        }
    }

    /// `values` as a value of `kind`: a list takes every value, any other kind
    /// exactly one. `Err` is a user-facing reason (not a number, not a date, …).
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
        let invalid = || format!("\"{raw}\" is not a {}", kind.describe());
        match kind {
            PropertyKind::Text => Ok(PropertyValue::Text(raw.clone())),
            PropertyKind::Number => raw
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .map(PropertyValue::Number)
                .ok_or_else(invalid),
            PropertyKind::Bool => match raw.trim().to_lowercase().as_str() {
                "true" => Ok(PropertyValue::Bool(true)),
                "false" => Ok(PropertyValue::Bool(false)),
                _ => Err(invalid()),
            },
            PropertyKind::Date => parse_iso_date(raw.trim())
                .map(PropertyValue::Date)
                .ok_or_else(invalid),
            PropertyKind::DateTime => parse_datetime(raw.trim())
                .map(PropertyValue::DateTime)
                .ok_or_else(invalid),
            PropertyKind::List => unreachable!("handled above"),
        }
    }

    /// Types user input for a key whose values elsewhere in the vault are
    /// mostly of `vault_kind`. An explicit `kind` always wins. Otherwise the
    /// input must fit the vault's kind (a single value for a list key becomes
    /// a one-item list); with no vault kind, one value is [inferred](Self::infer)
    /// and several make a list. `Err` explains a mismatch.
    pub fn from_input(
        values: &[String],
        kind: Option<PropertyKind>,
        vault_kind: Option<PropertyKind>,
    ) -> Result<PropertyValue, String> {
        if values.is_empty() {
            return Err("no value given".to_string());
        }
        if let Some(kind) = kind {
            return Self::parse_as(kind, values);
        }
        match (vault_kind, values) {
            (None, [raw]) => Ok(Self::infer(raw)),
            (None, _) => Ok(PropertyValue::List(values.to_vec())),
            (Some(vault_kind), _) => Self::parse_as(vault_kind, values).map_err(|_| {
                let given = match values {
                    [raw] => format!("\"{raw}\" is not one"),
                    _ => format!("{} values make a list", values.len()),
                };
                format!(
                    "it holds {} values in this vault and {given}; set the type explicitly to store it anyway",
                    vault_kind.describe()
                )
            }),
        }
    }
}

/// The value as plain text: canonical numbers/dates, list items joined with
/// `, `.
impl fmt::Display for PropertyValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyValue::Text(s) => f.write_str(s),
            PropertyValue::Number(n) => f.write_str(&format_number(*n)),
            PropertyValue::Bool(b) => write!(f, "{b}"),
            PropertyValue::Date(d) => f.write_str(&format_iso_date(*d)),
            PropertyValue::DateTime(dt) => f.write_str(&format_datetime(dt)),
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
    use chrono::TimeZone;

    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
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
            PropertyValue::DateTime(Utc.with_ymd_and_hms(2024, 3, 1, 14, 30, 0).unwrap())
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
    }

    #[test]
    fn parse_as_forces_a_kind() {
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
            PropertyValue::parse_as(PropertyKind::List, &one("a")),
            Ok(PropertyValue::List(vec!["a".into()]))
        );
        assert!(PropertyValue::parse_as(PropertyKind::Number, &one("high"))
            .unwrap_err()
            .contains("not a number"));
        assert!(PropertyValue::parse_as(PropertyKind::Date, &one("2024-02-30")).is_err());
        assert!(PropertyValue::parse_as(PropertyKind::Text, &strings(&["a", "b"])).is_err());
    }

    #[test]
    fn from_input_follows_the_vault_kind() {
        let one = |s: &str| strings(&[s]);
        let num = Some(PropertyKind::Number);
        assert_eq!(
            PropertyValue::from_input(&one("5"), None, num),
            Ok(PropertyValue::Number(5.0))
        );
        let err = PropertyValue::from_input(&one("high"), None, num).unwrap_err();
        assert!(err.starts_with("it holds number values"), "{err}");
        assert_eq!(
            PropertyValue::from_input(&one("high"), Some(PropertyKind::Text), num),
            Ok(PropertyValue::Text("high".into())),
            "an explicit kind wins"
        );
        assert_eq!(
            PropertyValue::from_input(&one("2024"), None, Some(PropertyKind::Text)),
            Ok(PropertyValue::Text("2024".into())),
            "a text key keeps number-looking input as text"
        );
        assert_eq!(
            PropertyValue::from_input(&one("garden"), None, Some(PropertyKind::List)),
            Ok(PropertyValue::List(vec!["garden".into()])),
            "one value for a list key is a one-item list"
        );
        let err = PropertyValue::from_input(&strings(&["1", "2"]), None, num).unwrap_err();
        assert!(err.contains("2 values make a list"), "{err}");
        assert_eq!(
            PropertyValue::from_input(&strings(&["a", "b"]), None, None),
            Ok(PropertyValue::List(vec!["a".into(), "b".into()]))
        );
        assert!(PropertyValue::from_input(&[], None, None).is_err());
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
        let at = Utc.with_ymd_and_hms(2024, 3, 1, 14, 30, 0).unwrap();
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
            (
                PropertyValue::DateTime(at),
                "2024-03-01T14:30:00Z",
                r#""2024-03-01T14:30:00Z""#,
            ),
            (
                PropertyValue::List(vec!["a".into(), "b c".into()]),
                "a, b c",
                r#"["a","b c"]"#,
            ),
        ];
        for (value, text, json) in cases {
            assert_eq!(value.to_string(), text);
            assert_eq!(serde_json::to_string(&value).unwrap(), json);
        }
    }
}
