//! Wiki mode: public types and pure helpers. The vault operations live in
//! `vault_wiki.rs`.

use crate::nfs;
use crate::nfs::VaultPath;
use crate::note::{text_below_frontmatter, PropertyValue};
use crate::WikiDeclaration;

/// The hash of a note's body: the text below the frontmatter block, with LF
/// line endings, as 16 lower-case hex digits. The frontmatter is left out so
/// that writing a property such as `content_hash` does not change the hash.
pub(crate) fn body_hash(text: &str) -> String {
    let text = nfs::to_lf(text);
    format!("{:016x}", nfs::hash_text(text_below_frontmatter(&text)))
}

/// What `wiki stamp` changed.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct StampReport {
    /// The sources that got a `content_hash`, sorted by path.
    pub stamped: Vec<VaultPath>,
    /// The sources that stamp could not change, sorted by path.
    pub skipped: Vec<SkippedSource>,
}

/// A source that `wiki stamp` left alone because it could not stamp it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SkippedSource {
    /// The source.
    pub path: VaultPath,
    /// Why stamp left it alone, in plain words.
    pub reason: String,
}

/// One page in the wiki catalogue.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CatalogueEntry {
    /// The page's path.
    pub path: VaultPath,
    /// The page's title.
    pub title: String,
    /// The `summary` property, when it is text.
    pub summary: Option<String>,
    /// The `type` property, when it is text.
    pub page_type: Option<String>,
}

/// The wiki pages, sorted by path and capped.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct Catalogue {
    /// The first pages, up to the cap.
    pub entries: Vec<CatalogueEntry>,
    /// How many pages the wiki has, before the cap.
    pub total: usize,
}

/// Options for `wiki init`. A folder left as `None` takes its default.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WikiInitOptions {
    /// The folder of raw source notes.
    pub sources: Option<VaultPath>,
    /// The folder of wiki notes.
    pub wiki: Option<VaultPath>,
}

/// One thing `wiki init` looked at.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WikiInitEntry {
    /// What it was, such as `.kimun/wiki.toml`, `folder /sources` or
    /// `note /wiki/schema.md`.
    pub what: String,
    /// True when init made it. False when it was already there.
    pub created: bool,
}

/// What `wiki init` did, in the order it did it.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct WikiInitReport {
    /// One entry per file, folder or note.
    pub entries: Vec<WikiInitEntry>,
}

/// A rule of `wiki check`. Findings sort in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WikiRule {
    /// No other note links to the note.
    Orphan,
    /// A note link points to no note.
    UnresolvedLink,
    /// A note link matches several notes.
    AmbiguousLink,
    /// A wiki page lacks a required property.
    MissingProperty,
    /// An item of a page's `sources` property is no note link, or names no
    /// note.
    MissingSource,
    /// A source's text no longer matches its `content_hash`.
    SourceModified,
    /// A page's `updated` date is before the `ingested_at` date of a source
    /// it cites.
    StalePage,
    /// A source has no `content_hash`, so it has no baseline.
    Unstamped,
}

impl WikiRule {
    /// How serious a finding of this rule is.
    pub(crate) fn severity(self) -> Severity {
        match self {
            WikiRule::Unstamped => Severity::Info,
            _ => Severity::Problem,
        }
    }
}

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Something to fix.
    Problem,
    /// Something to know about. It is not a problem.
    Info,
}

/// One thing `wiki check` found.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WikiFinding {
    /// The rule that found it.
    pub rule: WikiRule,
    /// How serious it is.
    pub severity: Severity,
    /// The note at fault.
    pub path: VaultPath,
    /// What is wrong, in plain words.
    pub message: String,
}

/// What `wiki check` found.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct WikiReport {
    /// True when the vault has a wiki declaration.
    pub declared: bool,
    /// How many notes the rules looked at.
    pub notes_checked: usize,
    /// The findings, sorted by rule and then by path.
    pub findings: Vec<WikiFinding>,
}

impl WikiReport {
    /// The number of findings that are problems. Info findings do not count.
    pub fn problems(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Problem)
            .count()
    }
}

/// True when a page last updated on `page_updated` is older than a source
/// ingested on `source_ingested`. It compares the date parts of `Date` and
/// `DateTime` values. The same day is not stale. Any other value type gives
/// false.
pub(crate) fn is_stale(page_updated: &PropertyValue, source_ingested: &PropertyValue) -> bool {
    match (date_part(page_updated), date_part(source_ingested)) {
        (Some(page), Some(source)) => page < source,
        _ => false,
    }
}

fn date_part(value: &PropertyValue) -> Option<chrono::NaiveDate> {
    match value {
        PropertyValue::Date(d) => Some(*d),
        PropertyValue::DateTime(dt) => Some(dt.naive_local().date()),
        _ => None,
    }
}

/// The text of the schema note, with the declared folder names filled in.
pub fn schema_template(d: &WikiDeclaration) -> String {
    let sources = &d.sources;
    let wiki = &d.wiki;
    let log = &d.log;
    format!(
        "# Wiki schema

This note tells you how this wiki works. An AI agent and a human both follow it.
Edit it to fit your needs.

## The three layers

- Sources are in `{sources}`. They are raw notes. They are the ground truth.
- The wiki is in `{wiki}`. It holds pages that you write and keep up to date.
- This schema note tells you how to do the work.

Write only under `{wiki}`. Never edit anything under `{sources}`.

## Page format

Every wiki page starts with a frontmatter block. It has these properties:

```
---
summary: One sentence that says what the page is about.
sources: [\"[[a-source]]\", \"[[another-source]]\"]
updated: 2026-01-31
type: concept
---
```

- `summary` is one short sentence.
- `sources` lists the source notes the page draws on. Quote every wikilink in a property, as in the example.
- `updated` is the date of the last change.
- `type` says what kind of page it is, such as `concept`, `person` or `topic`.

## Naming

Use kebab-case for page names. Use lower case. Use hyphens between words.
Example: `spaced-repetition`.

## Linking

- Link each concept page from a parent page, so that no page is an orphan.
- Cite sources with wikilinks in the `sources` property.
- Link related pages to each other with wikilinks.

## Ingest

Follow these steps for each new source:

1. Read the source in full.
2. Find the wiki pages that the source touches.
3. Update those pages. Create a new page when a concept has none.
4. Add the source to the `sources` property of every page you touched.
5. Set `updated` to today on every page you touched.
6. Add an entry to the log at `{log}`.

## Answer a question

1. Read the wiki pages that are about the question.
2. Answer from the wiki. Follow links when you need more.
3. Check the sources when the wiki is unclear or seems wrong.
4. Say which pages you used.
5. When you find a gap, fix it in the wiki.

## Lint

Check the wiki for these problems:

- Pages that miss a required property.
- Pages that no other page links to.
- Links that point to notes that do not exist.
- Claims that the cited sources do not support.
- Pages that have not been updated after a source changed.

Fix what you can. Report the rest.

## Log entries

Add one entry at the end of `{log}` for each piece of work. Each entry has:

- The date.
- The source.
- The pages you touched.
- The main finding.
"
    )
}

/// The text of the log note when `wiki init` creates it.
pub fn log_template() -> String {
    "# Wiki log

One entry for each piece of work. Newest at the end. Each entry has the date, the source, the pages touched, and the main finding.
"
    .to_string()
}
