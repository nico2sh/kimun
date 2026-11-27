//! Wiki mode on [`NoteVault`]. This file reads the declaration in
//! `.kimun/wiki.toml`. It also holds the four wiki operations: init, stamp,
//! catalogue and check. The public types and pure helpers are in `wiki.rs`.

use std::collections::HashSet;

use crate::error::{FSError, VaultError};
use crate::nfs::wiki_declaration;
use crate::nfs::{self, VaultPath};
use crate::note::properties::NoteProperties;
use crate::note::{
    FrontmatterFormat, LinkType, NoteDetails, NoteTarget, PropertyEntry, PropertyValue,
};
use crate::wiki::{body_hash, is_stale, log_template, schema_template};
use crate::{
    Catalogue, CatalogueEntry, LinkResolution, NoteVault, ResultType, SkippedSource, StampReport,
    VaultBrowseOptionsBuilder, WikiDeclaration, WikiFinding, WikiInitEntry, WikiInitOptions,
    WikiInitReport, WikiReport, WikiRule,
};

const CONTENT_HASH: &str = "content_hash";
const DECLARATION_LABEL: &str = ".kimun/wiki.toml";
const INGESTED_AT: &str = "ingested_at";
const SOURCES: &str = "sources";
const UPDATED: &str = "updated";

impl NoteVault {
    /// The vault's wiki declaration, or `None` when the vault is not a wiki.
    /// A file that does not parse, or breaks a declaration rule, gives
    /// `InvalidWikiDeclaration`.
    pub async fn wiki_declaration(&self) -> Result<Option<WikiDeclaration>, VaultError> {
        let declaration = match wiki_declaration::read_wiki_declaration(self.workspace_path()).await
        {
            Ok(d) => d,
            Err(FSError::SerializationError(message)) => {
                return Err(invalid(message));
            }
            Err(e) => return Err(e.into()),
        };
        if let Some(d) = &declaration {
            d.validate().map_err(invalid)?;
        }
        Ok(declaration)
    }

    /// Sets up a wiki. It never overwrites. When a declaration exists, it
    /// keeps it, ignores `options`, and creates only the folders and notes the
    /// declaration names that are missing. Otherwise it checks the new
    /// declaration, writes it, and creates the two folders, the schema note
    /// and the log note.
    pub async fn wiki_init(&self, options: WikiInitOptions) -> Result<WikiInitReport, VaultError> {
        let mut report = WikiInitReport::default();
        let declaration = match self.wiki_declaration().await? {
            Some(d) => {
                report.entries.push(WikiInitEntry {
                    what: DECLARATION_LABEL.to_string(),
                    created: false,
                });
                d
            }
            None => {
                let defaults = WikiDeclaration::defaults();
                let d = WikiDeclaration::with_folders(
                    &options.sources.unwrap_or_else(|| defaults.sources_path()),
                    &options.wiki.unwrap_or_else(|| defaults.wiki_path()),
                );
                d.validate().map_err(invalid)?;
                wiki_declaration::write_wiki_declaration(self.workspace_path(), &d).await?;
                report.entries.push(WikiInitEntry {
                    what: DECLARATION_LABEL.to_string(),
                    created: true,
                });
                d
            }
        };
        for folder in [declaration.sources_path(), declaration.wiki_path()] {
            let created = match self.create_directory(&folder).await {
                Ok(_) => true,
                Err(VaultError::DirectoryExists { .. }) => false,
                Err(e) => return Err(e),
            };
            report.entries.push(WikiInitEntry {
                what: format!("folder {folder}"),
                created,
            });
        }
        let notes = [
            (declaration.schema_path(), schema_template(&declaration)),
            (declaration.log_path(), log_template()),
        ];
        for (path, text) in notes {
            let created = match self.create_note(&path, text).await {
                Ok(_) => true,
                Err(VaultError::NoteExists { .. }) => false,
                Err(e) => return Err(e),
            };
            report.entries.push(WikiInitEntry {
                what: format!("note {path}"),
                created,
            });
        }
        Ok(report)
    }

    /// Stamps every source that has no `content_hash`: sets `content_hash` to
    /// the hash of its body and, when `ingested_at` is absent, sets it to the
    /// local day. It writes past source protection and backs up first. Sources
    /// that already have a `content_hash` are left alone. A source whose
    /// frontmatter does not parse is left alone and listed in `skipped`.
    pub async fn wiki_stamp(&self) -> Result<StampReport, VaultError> {
        // The local day, not the UTC day. An agent writes `updated` with the
        // local date. A UTC date can be one day ahead in the evening west of
        // UTC, and then every page that cites the source looks stale.
        self.wiki_stamp_on(chrono::Local::now().date_naive()).await
    }

    /// [`Self::wiki_stamp`] with `today` as the ingest date.
    pub(crate) async fn wiki_stamp_on(
        &self,
        today: chrono::NaiveDate,
    ) -> Result<StampReport, VaultError> {
        let declaration = self
            .wiki_declaration()
            .await?
            .ok_or(VaultError::WikiNotDeclared)?;
        let mut report = StampReport::default();
        for path in self.notes_under(&declaration.sources_path()).await? {
            match self.stamp_note(&path, today).await? {
                Stamp::Written => report.stamped.push(path),
                Stamp::AlreadyStamped => {}
                Stamp::BrokenFrontmatter => report.skipped.push(SkippedSource {
                    path,
                    reason: "The frontmatter does not parse. Fix it by hand, then stamp again."
                        .to_string(),
                }),
                Stamp::Uneditable(key) => report.skipped.push(SkippedSource {
                    path,
                    reason: format!(
                        "The frontmatter cannot be edited to add {key}. Rewrite it as plain key: value lines, then stamp again."
                    ),
                }),
            }
        }
        Ok(report)
    }

    /// The wiki pages, sorted by path, with the title, `summary` and `type` of
    /// each. The log and schema notes are left out. `total` counts every page
    /// and `entries` keeps the first `limit`.
    pub async fn wiki_catalogue(&self, limit: usize) -> Result<Catalogue, VaultError> {
        let declaration = self
            .wiki_declaration()
            .await?
            .ok_or(VaultError::WikiNotDeclared)?;
        let pages = self.wiki_pages(&declaration).await?;
        let total = pages.len();
        let mut entries = Vec::new();
        for path in pages.into_iter().take(limit) {
            let text = self.get_note_text(&path).await?;
            let properties = NoteDetails::properties_of(text.clone());
            let text_of = |key: &str| match value_of(&properties, key) {
                Some(PropertyValue::Text(t)) => Some(t.clone()),
                _ => None,
            };
            entries.push(CatalogueEntry {
                title: NoteDetails::get_title_from_text(&text),
                summary: text_of("summary"),
                page_type: text_of("type"),
                path,
            });
        }
        Ok(Catalogue { entries, total })
    }

    /// Checks the vault against the wiki rules. It makes no change.
    ///
    /// With a declaration, the wiki pages get the orphan, link, required
    /// property, `sources` and stale page rules, and the sources get the
    /// `content_hash` rules. Without one, every note outside the journal and
    /// inbox folders gets the orphan and link rules. Findings sort by rule,
    /// then by path.
    pub async fn wiki_check(&self) -> Result<WikiReport, VaultError> {
        let declaration = self.wiki_declaration().await?;
        let mut findings = Vec::new();
        let mut notes_checked = 0;
        match &declaration {
            Some(d) => {
                for path in self.wiki_pages(d).await? {
                    let page = self.load_note(&path).await?;
                    self.check_links(&page, &mut findings).await?;
                    self.check_page(d, &page, &mut findings).await?;
                    notes_checked += 1;
                }
                for path in self.notes_under(&d.sources_path()).await? {
                    self.check_source(&path, &mut findings).await?;
                    notes_checked += 1;
                }
            }
            None => {
                for path in self.notes_under(&VaultPath::root()).await? {
                    if self.journal_path.is_ancestor_of(&path)
                        || self.inbox_path.is_ancestor_of(&path)
                    {
                        continue;
                    }
                    let note = self.load_note(&path).await?;
                    self.check_links(&note, &mut findings).await?;
                    notes_checked += 1;
                }
            }
        }
        findings.sort_by(|a, b| (a.rule, &a.path).cmp(&(b.rule, &b.path)));
        Ok(WikiReport {
            declared: declaration.is_some(),
            notes_checked,
            findings,
        })
    }

    /// The generic rules: orphan, unresolved link and ambiguous link.
    async fn check_links(
        &self,
        note: &NoteDetails,
        findings: &mut Vec<WikiFinding>,
    ) -> Result<(), VaultError> {
        let path = &note.path;
        let linked_from_another = self
            .get_backlinks(path)
            .await?
            .iter()
            .any(|(entry, _)| !entry.path.is_like(path));
        if !linked_from_another {
            findings.push(finding(
                WikiRule::Orphan,
                path,
                "No other note links to this note.".to_string(),
            ));
        }
        let (_, links) = note.get_markdown_and_links();
        let mut seen = HashSet::new();
        for link in links {
            let LinkType::Note(target) = link.ltype else {
                continue;
            };
            if !seen.insert(link.raw_link.clone()) {
                continue;
            }
            let raw = &link.raw_link;
            match self.resolve_note_target(&target).await? {
                LinkResolution::Note(..) => {}
                LinkResolution::Missing(_) => findings.push(finding(
                    WikiRule::UnresolvedLink,
                    path,
                    format!("The link \"{raw}\" points to no note."),
                )),
                LinkResolution::Ambiguous(hits) => findings.push(finding(
                    WikiRule::AmbiguousLink,
                    path,
                    format!("The link \"{raw}\" matches {} notes.", hits.len()),
                )),
            }
        }
        Ok(())
    }

    /// The page rules: missing property, missing source and stale page.
    async fn check_page(
        &self,
        d: &WikiDeclaration,
        page: &NoteDetails,
        findings: &mut Vec<WikiFinding>,
    ) -> Result<(), VaultError> {
        let path = &page.path;
        let properties = page.get_properties();
        for name in &d.required_properties {
            if value_of(&properties, name).is_none() {
                findings.push(finding(
                    WikiRule::MissingProperty,
                    path,
                    format!("The page has no \"{name}\" property."),
                ));
            }
        }
        let items = match value_of(&properties, SOURCES) {
            Some(PropertyValue::List(items)) => items.clone(),
            Some(PropertyValue::Text(item)) => vec![item.clone()],
            _ => Vec::new(),
        };
        if NoteProperties::new(&page.raw_text, FrontmatterFormat::Yaml)
            .has_non_scalar_items(SOURCES)
        {
            findings.push(finding(
                WikiRule::MissingSource,
                path,
                "The sources list has an item that is not quoted text. Write it as \"[[name]]\"."
                    .to_string(),
            ));
        }
        let updated = value_of(&properties, UPDATED);
        let mut seen = HashSet::new();
        for item in items {
            if !seen.insert(item.clone()) {
                continue;
            }
            let Some(target) = NoteTarget::from_wikilink(link_text(&item), path) else {
                findings.push(finding(
                    WikiRule::MissingSource,
                    path,
                    format!("The source \"{item}\" is not a note link."),
                ));
                continue;
            };
            let source = match self.resolve_note_target(&target).await? {
                LinkResolution::Note(entry, _) => entry.path,
                LinkResolution::Missing(_) => {
                    findings.push(finding(
                        WikiRule::MissingSource,
                        path,
                        format!("The source \"{item}\" names no note."),
                    ));
                    continue;
                }
                LinkResolution::Ambiguous(hits) => {
                    findings.push(finding(
                        WikiRule::AmbiguousLink,
                        path,
                        format!("The source \"{item}\" matches {} notes.", hits.len()),
                    ));
                    continue;
                }
            };
            let Some(updated) = updated else {
                continue;
            };
            let source_properties = self.get_properties(&source).await?;
            if let Some(ingested) = value_of(&source_properties, INGESTED_AT) {
                if is_stale(updated, ingested) {
                    findings.push(finding(
                        WikiRule::StalePage,
                        path,
                        format!(
                            "The source {source} was ingested after the page was last updated."
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// The source rules: source modified and unstamped.
    async fn check_source(
        &self,
        path: &VaultPath,
        findings: &mut Vec<WikiFinding>,
    ) -> Result<(), VaultError> {
        let text = self.get_note_text(path).await?;
        if !NoteProperties::new(&text, FrontmatterFormat::Yaml).parses() {
            findings.push(finding(
                WikiRule::SourceModified,
                path,
                "The source frontmatter does not parse, so its content_hash cannot be read."
                    .to_string(),
            ));
            return Ok(());
        }
        let properties = NoteDetails::properties_of(&text);
        match value_of(&properties, CONTENT_HASH) {
            Some(PropertyValue::Text(hash)) => {
                if *hash != body_hash(&text) {
                    findings.push(finding(
                        WikiRule::SourceModified,
                        path,
                        "The source text no longer matches its content_hash.".to_string(),
                    ));
                }
            }
            _ => findings.push(finding(
                WikiRule::Unstamped,
                path,
                "The source has no content_hash, so a change to it cannot be found.".to_string(),
            )),
        }
        Ok(())
    }

    /// The notes under the wiki folder, sorted by path, without the declared
    /// log and schema notes. A missing wiki folder gives an empty list.
    async fn wiki_pages(&self, d: &WikiDeclaration) -> Result<Vec<VaultPath>, VaultError> {
        let (log, schema) = (d.log_path(), d.schema_path());
        let mut pages = self.notes_under(&d.wiki_path()).await?;
        pages.retain(|p| !p.is_like(&log) && !p.is_like(&schema));
        Ok(pages)
    }

    /// Every note under `folder`, at any depth, sorted by path. A missing
    /// folder gives an empty list.
    async fn notes_under(&self, folder: &VaultPath) -> Result<Vec<VaultPath>, VaultError> {
        if !self.exists(folder).await {
            return Ok(Vec::new());
        }
        let options = VaultBrowseOptionsBuilder::new(folder)
            .recursive(true)
            .build();
        let mut notes: Vec<VaultPath> = self
            .browse_vault(options)
            .await?
            .into_iter()
            .filter(|e| matches!(e.rtype, ResultType::Note(_)))
            .map(|e| e.path)
            .collect();
        notes.sort();
        Ok(notes)
    }

    /// Stamps one note. A note whose frontmatter does not parse, or parses but
    /// refuses the property edit, is left alone. A read or write error is
    /// still an error.
    async fn stamp_note(
        &self,
        path: &VaultPath,
        today: chrono::NaiveDate,
    ) -> Result<Stamp, VaultError> {
        let _guard = self.lock_note(path).await;
        let text = nfs::to_lf(&self.get_note_text(path).await?);
        if !NoteProperties::new(&text, FrontmatterFormat::Yaml).parses() {
            return Ok(Stamp::BrokenFrontmatter);
        }
        let properties = NoteDetails::properties_of(text.clone());
        if value_of(&properties, CONTENT_HASH).is_some() {
            return Ok(Stamp::AlreadyStamped);
        }
        let hash = PropertyValue::Text(body_hash(&text));
        // Both edits happen in memory. A refusal returns before anything is
        // written, so the file stays as it was.
        let set = |text: &str, key: &str, value: &PropertyValue| {
            NoteProperties::new(text, FrontmatterFormat::Yaml).set(key, value)
        };
        let refused = |key: &str, e| match Self::frontmatter_error(path, key, e) {
            VaultError::InvalidProperty { .. } => Ok(Stamp::Uneditable(key.to_string())),
            other => Err(other),
        };
        let mut updated = match set(&text, CONTENT_HASH, &hash) {
            Ok(t) => t,
            Err(e) => return refused(CONTENT_HASH, e),
        };
        if value_of(&properties, INGESTED_AT).is_none() {
            updated = match set(&updated, INGESTED_AT, &PropertyValue::Date(today)) {
                Ok(t) => t,
                Err(e) => return refused(INGESTED_AT, e),
            };
        }
        self.save_note_unchecked(path, updated).await?;
        Ok(Stamp::Written)
    }
}

/// What [`NoteVault::stamp_note`] did with one note.
enum Stamp {
    Written,
    AlreadyStamped,
    BrokenFrontmatter,
    /// The frontmatter parses but refused an edit of this property.
    Uneditable(String),
}

fn finding(rule: WikiRule, path: &VaultPath, message: String) -> WikiFinding {
    WikiFinding {
        rule,
        severity: rule.severity(),
        path: path.clone(),
        message,
    }
}

/// The value of property `key`, matched case-insensitively. A key with no
/// value counts as absent.
fn value_of<'a>(properties: &'a [PropertyEntry], key: &str) -> Option<&'a PropertyValue> {
    properties
        .iter()
        .find(|(k, _)| crate::note::property_keys_match(k, key))
        .and_then(|(_, v)| v.as_ref())
}

/// The link target in a `sources` item: the text inside `[[` and `]]`, with
/// any `|alias` cut off. An item without the brackets stays as it is.
fn link_text(item: &str) -> &str {
    let item = item.trim();
    let inner = item
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .unwrap_or(item);
    inner.split('|').next().unwrap_or(inner)
}

fn invalid(detail: String) -> VaultError {
    VaultError::InvalidWikiDeclaration {
        message: format!("Invalid .kimun/wiki.toml: {detail}"),
    }
}
