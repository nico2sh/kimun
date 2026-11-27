// tui/src/cli/commands/wiki.rs
//
// `kimun wiki`: set up, check, list and stamp a wiki. The work is core's.
// This file parses the folder flags and prints the reports.

use clap::Subcommand;
use color_eyre::eyre::Result;
use kimun_core::nfs::VaultPath;
use kimun_core::{
    Catalogue, NoteVault, Severity, StampReport, WikiInitOptions, WikiInitReport, WikiReport,
};

use crate::cli::CheckFailed;
use crate::cli::output::OutputFormat;

/// The widest a summary gets in the text listing, in characters.
const SUMMARY_WIDTH: usize = 100;

#[derive(Subcommand)]
pub enum WikiSubcommand {
    /// Set up a wiki: the declaration, the two folders, the schema and log notes
    Init {
        /// Folder for the raw source notes (default: sources)
        #[arg(long)]
        sources: Option<String>,
        /// Folder for the wiki pages (default: wiki)
        #[arg(long)]
        wiki: Option<String>,
    },
    /// Check the vault against the wiki rules. Exits 1 when it finds a problem
    Check {
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// List the wiki pages with their title, type and summary
    Catalogue {
        /// Most pages to list
        #[arg(long, default_value_t = 200)]
        limit: usize,
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
    /// Give every source without a content hash its hash and ingest date.
    /// If a source cannot be stamped, the command skips it, prints it, and
    /// exits with code 1.
    Stamp,
}

pub async fn run(subcommand: WikiSubcommand, vault: &NoteVault) -> Result<()> {
    match subcommand {
        WikiSubcommand::Init { sources, wiki } => {
            let options = WikiInitOptions {
                sources: sources.as_deref().map(VaultPath::new),
                wiki: wiki.as_deref().map(VaultPath::new),
            };
            let report = vault.wiki_init(options).await?;
            print!("{}", init_text(&report));
            Ok(())
        }
        WikiSubcommand::Check { format } => {
            let report = vault.wiki_check().await?;
            match format {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
                OutputFormat::Text | OutputFormat::Paths => print!("{}", check_text(&report)),
            }
            if report.problems() > 0 {
                return Err(CheckFailed.into());
            }
            Ok(())
        }
        WikiSubcommand::Catalogue { limit, format } => {
            let catalogue = vault.wiki_catalogue(limit).await?;
            match format {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&catalogue)?),
                OutputFormat::Text => print!("{}", catalogue_text(&catalogue)),
                OutputFormat::Paths => {
                    for e in &catalogue.entries {
                        println!("{}", e.path.to_bare_string());
                    }
                }
            }
            Ok(())
        }
        WikiSubcommand::Stamp => {
            let report = vault.wiki_stamp().await?;
            print!("{}", stamp_text(&report));
            // A skipped source needs a person to fix it, so it fails the run.
            if !report.skipped.is_empty() {
                return Err(CheckFailed.into());
            }
            Ok(())
        }
    }
}

fn init_text(report: &WikiInitReport) -> String {
    report
        .entries
        .iter()
        .map(|e| {
            format!(
                "{}  {}\n",
                if e.created { "created" } else { "kept" },
                e.what
            )
        })
        .collect()
}

fn check_text(report: &WikiReport) -> String {
    if report.findings.is_empty() {
        return format!(
            "The wiki is clean. {} checked.\n",
            count(report.notes_checked, "note", "notes")
        );
    }
    let mut out = String::new();
    let mut problems = 0;
    let mut infos = 0;
    for f in &report.findings {
        let label = match f.severity {
            Severity::Problem => {
                problems += 1;
                "problem"
            }
            Severity::Info => {
                infos += 1;
                "info"
            }
        };
        let rule = serde_json::to_value(f.rule)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        out.push_str(&format!(
            "{label}  {rule}  {}  {}\n",
            f.path.to_bare_string(),
            f.message
        ));
    }
    out.push_str(&format!(
        "{}, {infos} info, {} checked.\n",
        count(problems, "problem", "problems"),
        count(report.notes_checked, "note", "notes")
    ));
    out
}

/// `n` with the singular or plural noun, such as `1 note` or `2 notes`.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A summary on one line: whitespace runs, newlines included, become single
/// spaces. A long one is cut at `SUMMARY_WIDTH` characters with `...`.
fn one_line(summary: &str) -> String {
    let flat = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= SUMMARY_WIDTH {
        return flat;
    }
    let cut: String = flat.chars().take(SUMMARY_WIDTH - 3).collect();
    format!("{}...", cut.trim_end())
}

fn catalogue_text(catalogue: &Catalogue) -> String {
    let mut out = String::new();
    for e in &catalogue.entries {
        let summary = e.summary.as_deref().map(one_line).unwrap_or_default();
        let page_type = e.page_type.as_deref().map(one_line).unwrap_or_default();
        // An empty type keeps its column so summaries line up. The line
        // itself never ends in spaces.
        let line = format!(
            "{}  {}  {}  {}",
            e.path.to_bare_string(),
            one_line(&e.title),
            page_type,
            summary
        );
        out.push_str(line.trim_end());
        out.push('\n');
    }
    let more = catalogue.total.saturating_sub(catalogue.entries.len());
    if more > 0 {
        let noun = if more == 1 { "page" } else { "pages" };
        out.push_str(&format!("{more} more {noun}\n"));
    }
    out
}

fn stamp_text(report: &StampReport) -> String {
    if report.stamped.is_empty() && report.skipped.is_empty() {
        return "Nothing to stamp.\n".to_string();
    }
    let stamped = report
        .stamped
        .iter()
        .map(|p| format!("{}\n", p.to_bare_string()));
    let skipped = report
        .skipped
        .iter()
        .map(|s| format!("skipped  {}  {}\n", s.path.to_bare_string(), s.reason));
    stamped.chain(skipped).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::{CatalogueEntry, WikiFinding, WikiRule};

    fn finding(severity: Severity) -> WikiFinding {
        WikiFinding {
            rule: WikiRule::Orphan,
            severity,
            path: VaultPath::new("/wiki/a.md"),
            message: "m".into(),
        }
    }

    fn report(findings: Vec<WikiFinding>, notes_checked: usize) -> WikiReport {
        WikiReport {
            declared: true,
            notes_checked,
            findings,
        }
    }

    #[test]
    fn check_summary_uses_the_singular_for_one() {
        let one = check_text(&report(vec![finding(Severity::Problem)], 1));
        assert!(
            one.ends_with("1 problem, 0 info, 1 note checked.\n"),
            "{one}"
        );
        let two = check_text(&report(
            vec![finding(Severity::Problem), finding(Severity::Problem)],
            2,
        ));
        assert!(
            two.ends_with("2 problems, 0 info, 2 notes checked.\n"),
            "{two}"
        );
        assert_eq!(
            check_text(&report(vec![], 1)),
            "The wiki is clean. 1 note checked.\n"
        );
    }

    #[test]
    fn catalogue_more_line_uses_the_singular_for_one() {
        let c = Catalogue {
            entries: vec![],
            total: 1,
        };
        assert_eq!(catalogue_text(&c), "1 more page\n");
    }

    #[test]
    fn catalogue_text_shows_the_type_column() {
        let c = Catalogue {
            entries: vec![
                CatalogueEntry {
                    path: VaultPath::new("/wiki/a.md"),
                    title: "A".into(),
                    summary: Some("first".into()),
                    page_type: Some("concept".into()),
                },
                CatalogueEntry {
                    path: VaultPath::new("/wiki/b.md"),
                    title: "B".into(),
                    summary: Some("second".into()),
                    page_type: None,
                },
            ],
            total: 2,
        };
        assert_eq!(
            catalogue_text(&c),
            "/wiki/a  A  concept  first\n/wiki/b  B    second\n"
        );
    }

    #[test]
    fn one_line_collapses_newlines_and_runs_of_spaces() {
        assert_eq!(one_line("a  b\n\n  c\td\n"), "a b c d");
    }

    #[test]
    fn one_line_truncates_a_long_summary_with_a_marker() {
        let long = "word ".repeat(60);
        let out = one_line(&long);
        assert!(out.chars().count() <= SUMMARY_WIDTH);
        assert!(out.ends_with("..."));
        assert_eq!(one_line("short"), "short");
    }

    #[test]
    fn catalogue_text_keeps_each_page_on_one_line() {
        let c = Catalogue {
            entries: vec![CatalogueEntry {
                path: VaultPath::new("/wiki/a.md"),
                title: "A".into(),
                summary: Some("first\nsecond".into()),
                page_type: None,
            }],
            total: 1,
        };
        assert_eq!(catalogue_text(&c), "/wiki/a  A    first second\n");
    }

    fn entry(summary: Option<&str>, page_type: Option<&str>) -> CatalogueEntry {
        CatalogueEntry {
            path: VaultPath::new("/wiki/a.md"),
            title: "A".into(),
            summary: summary.map(str::to_string),
            page_type: page_type.map(str::to_string),
        }
    }

    #[test]
    fn catalogue_text_has_no_trailing_whitespace() {
        for (summary, page_type) in [
            (None, None),
            (None, Some("concept")),
            (Some("s"), None),
            (Some("  "), None),
        ] {
            let c = Catalogue {
                entries: vec![entry(summary, page_type)],
                total: 1,
            };
            let text = catalogue_text(&c);
            for line in text.lines() {
                assert_eq!(line, line.trim_end(), "{summary:?} {page_type:?}");
            }
        }
        let c = Catalogue {
            entries: vec![entry(None, Some("concept"))],
            total: 1,
        };
        assert_eq!(catalogue_text(&c), "/wiki/a  A  concept\n");
    }

    #[test]
    fn one_line_truncates_at_a_multi_byte_character_without_panicking() {
        // Three bytes per character.
        let three = "\u{20ac}".repeat(200);
        let out = one_line(&three);
        assert_eq!(out.chars().count(), SUMMARY_WIDTH);
        assert!(out.ends_with("..."));
        // A 4-byte emoji sits exactly at the cut.
        let cut = SUMMARY_WIDTH - 3;
        let emoji = format!("{}\u{1F600}{}", "a".repeat(cut - 1), "b".repeat(50));
        let out = one_line(&emoji);
        assert!(out.contains('\u{1F600}'));
        assert!(out.ends_with("..."));
        // An emoji just past the cut is dropped whole.
        let past = format!("{}\u{1F600}{}", "a".repeat(cut), "b".repeat(50));
        let out = one_line(&past);
        assert!(!out.contains('\u{1F600}'));
        assert!(out.ends_with("..."));
    }
}
