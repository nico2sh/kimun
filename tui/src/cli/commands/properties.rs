// tui/src/cli/commands/properties.rs
//
// `kimun note prop …` — read and edit a note's frontmatter properties. Typing
// rules (inference, the vault-wide type check) live in core; this module only
// parses arguments and prints.

use clap::{Subcommand, ValueEnum};
use color_eyre::eyre::{Result, eyre};
use kimun_core::NoteVault;
use kimun_core::note::{FrontmatterFormat, PropertyKind, PropertyValue};

use crate::cli::helpers::resolve_note_path;
use crate::cli::json_output::JsonProperties;

/// Output format of `prop list` / `prop get`.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PropFormat {
    /// `key: value` lines (`get` prints just the value)
    #[default]
    Text,
    /// One JSON object (`get` prints just the JSON value)
    Json,
}

#[derive(Subcommand, Debug)]
pub enum PropSubcommand {
    /// List a note's frontmatter properties
    List {
        /// Note path, relative to quick_note_path or absolute from vault root
        path: String,
        #[arg(long, value_enum, default_value = "text")]
        format: PropFormat,
    },
    /// Print one property's value (fails when the note doesn't have it)
    Get {
        /// Note path, relative to quick_note_path or absolute from vault root
        path: String,
        /// Property key (case-insensitive)
        key: String,
        #[arg(long, value_enum, default_value = "text")]
        format: PropFormat,
    },
    /// Set a property. One value is typed by the type the key has elsewhere in
    /// the vault, or by its look for a new key; several values make a list.
    /// A value that doesn't fit the vault's type is refused unless --type is given.
    Set {
        /// Note path, relative to quick_note_path or absolute from vault root
        path: String,
        /// Property key (stored lowercase)
        key: String,
        /// The value, or several for a list
        #[arg(required = true, num_args = 1.., allow_negative_numbers = true)]
        values: Vec<String>,
        /// Force a type: text, number, bool, date, datetime or list. Changes only this note.
        #[arg(long = "type", value_parser = parse_kind)]
        kind: Option<PropertyKind>,
        /// Write a new frontmatter block as YAML (`---`) instead of TOML (`+++`).
        /// An existing block always keeps its format.
        #[arg(long)]
        yaml: bool,
    },
    /// Remove a property (no error when the note doesn't have it)
    Remove {
        /// Note path, relative to quick_note_path or absolute from vault root
        path: String,
        /// Property key (case-insensitive)
        key: String,
    },
}

fn parse_kind(s: &str) -> Result<PropertyKind, String> {
    s.parse()
}

pub async fn run(
    subcommand: PropSubcommand,
    vault: &NoteVault,
    quick_note_path: &str,
) -> Result<()> {
    match subcommand {
        PropSubcommand::List { path, format } => {
            let path = resolve_note_path(&path, quick_note_path)?;
            let properties = vault.get_properties(&path).await?;
            print!("{}", format_properties(&properties, format)?);
        }
        PropSubcommand::Get { path, key, format } => {
            let path = resolve_note_path(&path, quick_note_path)?;
            let value = vault
                .get_property(&path, &key)
                .await?
                .ok_or_else(|| eyre!("No property '{key}' in {path}"))?;
            println!("{}", format_value(&value, format)?);
        }
        PropSubcommand::Set {
            path,
            key,
            values,
            kind,
            yaml,
        } => {
            let path = resolve_note_path(&path, quick_note_path)?;
            let new_block_format = if yaml {
                FrontmatterFormat::Yaml
            } else {
                FrontmatterFormat::Toml
            };
            let value = vault
                .set_property_from_input(&path, &key, &values, kind, new_block_format)
                .await?;
            println!("Set {key} = {value} ({}) in {path}", value.kind());
        }
        PropSubcommand::Remove { path, key } => {
            let path = resolve_note_path(&path, quick_note_path)?;
            if vault.remove_property(&path, &key).await? {
                println!("Removed {key} from {path}");
            } else {
                println!("No property '{key}' in {path}");
            }
        }
    }
    Ok(())
}

/// A property list as `key: value` lines, or one JSON object in file order.
pub fn format_properties(
    properties: &[(String, PropertyValue)],
    format: PropFormat,
) -> Result<String> {
    Ok(match format {
        PropFormat::Text => properties
            .iter()
            .map(|(key, value)| format!("{key}: {value}\n"))
            .collect(),
        PropFormat::Json => {
            format!(
                "{}\n",
                serde_json::to_string(&JsonProperties(properties.to_vec()))?
            )
        }
    })
}

/// One value as plain text, or as its JSON value.
pub fn format_value(value: &PropertyValue, format: PropFormat) -> Result<String> {
    Ok(match format {
        PropFormat::Text => value.to_string(),
        PropFormat::Json => serde_json::to_string(value)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<(String, PropertyValue)> {
        vec![
            ("status".into(), PropertyValue::Text("done".into())),
            ("priority".into(), PropertyValue::Number(2.0)),
            (
                "tags".into(),
                PropertyValue::List(vec!["a".into(), "b c".into()]),
            ),
        ]
    }

    #[test]
    fn lists_as_lines_or_ordered_json() {
        assert_eq!(
            format_properties(&sample(), PropFormat::Text).unwrap(),
            "status: done\npriority: 2\ntags: a, b c\n"
        );
        assert_eq!(
            format_properties(&sample(), PropFormat::Json).unwrap(),
            "{\"status\":\"done\",\"priority\":2,\"tags\":[\"a\",\"b c\"]}\n",
            "keys keep the note's order"
        );
        assert_eq!(format_properties(&[], PropFormat::Json).unwrap(), "{}\n");
    }

    #[derive(clap::Parser)]
    struct Cli {
        #[command(subcommand)]
        prop: PropSubcommand,
    }

    fn parse_set(args: &[&str]) -> (Vec<String>, Option<PropertyKind>) {
        use clap::Parser;
        let argv = ["kimun", "set", "n", "k"].iter().chain(args);
        match Cli::try_parse_from(argv).unwrap().prop {
            PropSubcommand::Set { values, kind, .. } => (values, kind),
            other => panic!("parsed as {other:?}"),
        }
    }

    #[test]
    fn set_flags_after_values_are_flags_and_negatives_are_values() {
        assert_eq!(
            parse_set(&["high", "--type", "text"]),
            (vec!["high".to_string()], Some(PropertyKind::Text))
        );
        assert_eq!(parse_set(&["-5"]), (vec!["-5".to_string()], None));
        assert_eq!(
            parse_set(&["a", "b", "--type", "list"]),
            (
                vec!["a".to_string(), "b".to_string()],
                Some(PropertyKind::List)
            )
        );
    }

    #[test]
    fn formats_one_value() {
        let tags = PropertyValue::List(vec!["a".into()]);
        assert_eq!(format_value(&tags, PropFormat::Text).unwrap(), "a");
        assert_eq!(format_value(&tags, PropFormat::Json).unwrap(), "[\"a\"]");
    }
}
