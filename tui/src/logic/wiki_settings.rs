//! The wiki settings model shared by every frontend: the editable draft of the
//! wiki declaration, its checks, and the notes that tell the user what an apply
//! changes. Pure and synchronous. It reads no file and makes no folder. A
//! frontend reads the saved declaration into a [`WikiState`] with [`load`],
//! shows this model, and runs the apply with [`apply`]. The async operations
//! at the end open a throwaway vault, as `fast_reindex` does, so a frontend
//! calls them off the UI thread.

use kimun_core::error::VaultError;
use kimun_core::nfs::VaultPath;
use kimun_core::system::SystemPath;
use kimun_core::{IndexFile, NoteVault, WikiDeclaration, WikiInitOptions, WikiInitReport};

use crate::logic::preferences::vault_config;

/// What the vault is, as far as the wiki goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WikiState {
    /// No vault is open.
    NoVault,
    /// The vault has no wiki declaration.
    NotAWiki,
    /// The vault has this saved declaration.
    Wiki(WikiDeclaration),
    /// The declaration file exists but cannot be used. The text says why.
    Broken(String),
}

/// The text the user is editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiDraft {
    pub sources: String,
    pub wiki: String,
    pub log: String,
    pub schema: String,
    pub properties: String,
    /// The user edited `log` or `schema`, so a new wiki folder leaves them alone.
    pub log_schema_edited: bool,
}

impl WikiDraft {
    fn from_declaration(d: &WikiDeclaration) -> Self {
        let (log, schema) = notes_in(&d.wiki_path());
        Self {
            sources: d.sources.clone(),
            wiki: d.wiki.clone(),
            log_schema_edited: d.log_path() != log || d.schema_path() != schema,
            log: d.log.clone(),
            schema: d.schema.clone(),
            properties: d.required_properties.join(", "),
        }
    }
}

/// The default log and schema notes of a wiki folder.
fn notes_in(wiki: &VaultPath) -> (VaultPath, VaultPath) {
    let d = WikiDeclaration::with_folders(&VaultPath::root(), wiki);
    (d.log_path(), d.schema_path())
}

/// A path text made flat and absolute. A text with a character that a path
/// cannot hold stays as written, so `validate` can name it.
fn clean(text: &str) -> String {
    let text = text.trim();
    if VaultPath::is_valid(text) {
        VaultPath::new(text).flatten().absolute().to_string()
    } else {
        text.to_string()
    }
}

/// The property names in a text: split on commas and spaces, empty names and
/// repeats dropped.
fn split_properties(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in text.split(|c: char| c == ',' || c.is_whitespace()) {
        if !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// The wiki settings: the state of the vault and the draft being edited.
#[derive(Debug, Clone)]
pub struct WikiSettings {
    state: WikiState,
    draft: WikiDraft,
}

impl WikiSettings {
    /// The draft starts from the saved declaration, or from the defaults when
    /// the vault is not a wiki or the declaration is broken.
    pub fn new(state: WikiState) -> Self {
        let draft = match &state {
            WikiState::Wiki(d) => WikiDraft::from_declaration(d),
            _ => WikiDraft::from_declaration(&WikiDeclaration::defaults()),
        };
        Self { state, draft }
    }

    pub fn state(&self) -> &WikiState {
        &self.state
    }

    pub fn draft(&self) -> &WikiDraft {
        &self.draft
    }

    /// True when the draft differs from the one the saved state gives. The
    /// user typed something that is not applied yet.
    pub fn is_edited(&self) -> bool {
        self.draft != Self::new(self.state.clone()).draft
    }

    pub fn set_sources(&mut self, text: &str) {
        self.draft.sources = text.to_string();
    }

    /// Set the wiki folder. The log and schema follow it, unless the user
    /// edited them. An empty text, or one that a path cannot hold, leaves
    /// them as they were: they never show a repaired copy of a bad text.
    pub fn set_wiki(&mut self, text: &str) {
        self.draft.wiki = text.to_string();
        let trimmed = text.trim();
        if !self.draft.log_schema_edited && !trimmed.is_empty() && VaultPath::is_valid(trimmed) {
            let (log, schema) = notes_in(&VaultPath::new(clean(trimmed)));
            self.draft.log = log.to_string();
            self.draft.schema = schema.to_string();
        }
    }

    pub fn set_log(&mut self, text: &str) {
        self.draft.log = text.to_string();
        self.draft.log_schema_edited = true;
    }

    pub fn set_schema(&mut self, text: &str) {
        self.draft.schema = text.to_string();
        self.draft.log_schema_edited = true;
    }

    pub fn set_properties(&mut self, text: &str) {
        self.draft.properties = text.to_string();
    }

    /// The draft as a declaration, or the first message from `validate`.
    pub fn declaration(&self) -> Result<WikiDeclaration, String> {
        let d = WikiDeclaration {
            sources: clean(&self.draft.sources),
            wiki: clean(&self.draft.wiki),
            log: clean(&self.draft.log),
            schema: clean(&self.draft.schema),
            required_properties: split_properties(&self.draft.properties),
        };
        d.validate()?;
        Ok(d)
    }

    pub fn error(&self) -> Option<String> {
        self.declaration().err()
    }

    /// True if the vault is open and the draft has no error. A draft equal to
    /// the saved declaration is allowed: it repairs a missing folder or note.
    pub fn can_apply(&self) -> bool {
        !matches!(self.state, WikiState::NoVault) && self.error().is_none()
    }

    pub fn button_label(&self) -> &'static str {
        match self.state {
            WikiState::Wiki(_) => "Apply",
            _ => "Set up wiki",
        }
    }

    /// What an apply changes, one sentence group per line. Empty when nothing
    /// changes. `schema_exists` says whether the schema note is in the vault.
    pub fn effect_note(&self, schema_exists: bool) -> Vec<String> {
        let (WikiState::Wiki(saved), Ok(new)) = (&self.state, self.declaration()) else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        let sources_changed = saved.sources_path() != new.sources_path();
        let places_changed = saved.wiki_path() != new.wiki_path()
            || saved.log_path() != new.log_path()
            || saved.schema_path() != new.schema_path();
        if sources_changed {
            lines.push(format!(
                "Notes in {} stay where they are and are no longer protected. {} is the new protected folder.",
                saved.sources_path(),
                new.sources_path()
            ));
        }
        if places_changed {
            lines.push(
                "Notes in the old wiki folder, log, and schema stay where they are.".to_string(),
            );
        }
        let properties_changed = saved.required_properties != new.required_properties;
        if schema_exists && (sources_changed || places_changed || properties_changed) {
            lines.push(
                "The schema note keeps its text. Edit it so that it names the new layout."
                    .to_string(),
            );
        }
        lines
    }

    pub fn state_line(&self) -> String {
        match &self.state {
            WikiState::Broken(message) => message.clone(),
            WikiState::Wiki(_) => "This vault is a wiki.".to_string(),
            WikiState::NoVault => "No vault is open.".to_string(),
            WikiState::NotAWiki => "This vault is not a wiki.".to_string(),
        }
    }
}

/// What [`load`] found: the wiki state, and whether the declared schema note
/// exists. `schema_exists` is true only for a `Wiki` state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub state: WikiState,
    pub schema_exists: bool,
}

/// The text to show for a vault error: the user message when core gives one.
fn message(e: &VaultError) -> String {
    e.user_message().unwrap_or_else(|| e.to_string())
}

/// Read the wiki state of the vault at `workspace`. A vault that cannot open
/// also maps to `Broken(message)`, so the section shows the message.
pub async fn load(workspace: SystemPath, cache: Option<IndexFile>) -> Loaded {
    let vault = match NoteVault::new(vault_config(workspace, cache)).await {
        Ok(vault) => vault,
        Err(e) => {
            return Loaded {
                state: WikiState::Broken(message(&e)),
                schema_exists: false,
            };
        }
    };
    let loaded = match vault.wiki_declaration().await {
        Ok(None) => Loaded {
            state: WikiState::NotAWiki,
            schema_exists: false,
        },
        Ok(Some(declaration)) => {
            let schema_exists = vault.exists(&declaration.schema_path()).await;
            Loaded {
                state: WikiState::Wiki(declaration),
                schema_exists,
            }
        }
        Err(e) => Loaded {
            state: WikiState::Broken(message(&e)),
            schema_exists: false,
        },
    };
    vault.close().await;
    loaded
}

/// Save `declaration` and make its folders and notes. Nothing is changed when
/// core refuses the declaration.
pub async fn apply(
    workspace: SystemPath,
    cache: Option<IndexFile>,
    declaration: WikiDeclaration,
) -> Result<WikiInitReport, String> {
    let vault = NoteVault::new(vault_config(workspace, cache))
        .await
        .map_err(|e| message(&e))?;
    let result = vault.wiki_configure(declaration).await;
    vault.close().await;
    result.map_err(|e| message(&e))
}

/// What the palette command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteOutcome {
    /// It made these things, named as in the report.
    Created(Vec<String>),
    /// The vault was a wiki and nothing was missing.
    AlreadyAWiki,
    /// The declaration file is broken. The text says why. Nothing changed.
    BrokenDeclaration(String),
}

/// Set up a wiki with the default folders, or repair the one that exists.
pub async fn setup_defaults(
    workspace: SystemPath,
    cache: Option<IndexFile>,
) -> Result<PaletteOutcome, String> {
    let vault = NoteVault::new(vault_config(workspace, cache))
        .await
        .map_err(|e| message(&e))?;
    let result = vault.wiki_init(WikiInitOptions::default()).await;
    vault.close().await;
    match result {
        Ok(report) => {
            let created: Vec<String> = report
                .entries
                .iter()
                .filter(|e| e.created)
                .map(|e| e.what.clone())
                .collect();
            if created.is_empty() {
                Ok(PaletteOutcome::AlreadyAWiki)
            } else {
                Ok(PaletteOutcome::Created(created))
            }
        }
        Err(e @ VaultError::InvalidWikiDeclaration { .. }) => {
            Ok(PaletteOutcome::BrokenDeclaration(message(&e)))
        }
        Err(e) => Err(message(&e)),
    }
}

/// One line per report entry: `Created folder /raw.` or `Kept note /wiki/log.md.`
pub fn report_lines(report: &WikiInitReport) -> Vec<String> {
    report
        .entries
        .iter()
        .map(|e| {
            let verb = if e.created { "Created" } else { "Kept" };
            format!("{verb} {}.", e.what)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> WikiDeclaration {
        WikiDeclaration::with_folders(&VaultPath::new("/sources"), &VaultPath::new("/wiki"))
    }

    fn wiki() -> WikiSettings {
        WikiSettings::new(WikiState::Wiki(saved()))
    }

    #[test]
    fn a_vault_that_is_not_a_wiki_starts_from_the_defaults() {
        let s = WikiSettings::new(WikiState::NotAWiki);
        assert_eq!(s.declaration().unwrap(), WikiDeclaration::defaults());
        assert_eq!(s.state_line(), "This vault is not a wiki.");
    }

    #[test]
    fn without_a_vault_the_state_line_says_no_vault_is_open() {
        let s = WikiSettings::new(WikiState::NoVault);
        assert_eq!(s.state_line(), "No vault is open.");
    }

    #[test]
    fn a_wiki_starts_from_its_declaration() {
        let mut d = saved();
        d.required_properties = vec!["owner".into()];
        let s = WikiSettings::new(WikiState::Wiki(d.clone()));
        assert_eq!(s.declaration().unwrap(), d);
        assert_eq!(s.state_line(), "This vault is a wiki.");
    }

    #[test]
    fn a_fresh_draft_is_not_edited_and_a_typed_change_is() {
        let mut s = WikiSettings::new(WikiState::Wiki(saved()));
        assert!(!s.is_edited());
        s.set_sources("raw");
        assert!(s.is_edited());
        let mut fresh = WikiSettings::new(WikiState::NotAWiki);
        assert!(!fresh.is_edited());
        fresh.set_wiki("atlas");
        assert!(fresh.is_edited());
    }

    #[test]
    fn typing_the_saved_text_back_is_not_an_edit() {
        let mut s = WikiSettings::new(WikiState::Wiki(saved()));
        let original = s.draft().sources.clone();
        s.set_sources("raw");
        s.set_sources(&original);
        assert!(!s.is_edited());
    }

    #[test]
    fn a_broken_declaration_starts_from_the_defaults_and_shows_its_message() {
        let s = WikiSettings::new(WikiState::Broken("wiki.toml is not valid".into()));
        assert_eq!(s.declaration().unwrap(), WikiDeclaration::defaults());
        assert_eq!(s.state_line(), "wiki.toml is not valid");
    }

    #[test]
    fn the_log_and_schema_follow_the_wiki_folder_until_edited() {
        let mut s = wiki();
        s.set_wiki("notes/wiki");
        let d = s.declaration().unwrap();
        assert_eq!(d.log, "/notes/wiki/log.md");
        assert_eq!(d.schema, "/notes/wiki/schema.md");
        s.set_log("/my/log.md");
        s.set_wiki("other");
        let d = s.declaration().unwrap();
        assert_eq!(d.log, "/my/log.md");
        assert_eq!(d.schema, "/notes/wiki/schema.md");
    }

    #[test]
    fn an_invalid_wiki_folder_leaves_the_log_and_schema_as_they_were() {
        let mut s = wiki();
        s.set_wiki("atlas");
        s.set_wiki("bad?name");
        assert_eq!(s.draft().log, "/atlas/log.md");
        assert_eq!(s.draft().schema, "/atlas/schema.md");
        assert!(s.error().is_some());
        s.set_wiki("   ");
        assert_eq!(s.draft().log, "/atlas/log.md");
        s.set_wiki(" notes ");
        assert_eq!(s.draft().log, "/notes/log.md");
    }

    #[test]
    fn folder_fields_are_normalized_like_the_cli() {
        let expected =
            WikiDeclaration::with_folders(&VaultPath::new("raw"), &VaultPath::new("/wiki"));
        for text in ["./raw", "raw/", "/Raw/"] {
            let mut s = WikiSettings::new(WikiState::NotAWiki);
            s.set_sources(text);
            assert_eq!(s.declaration().unwrap(), expected, "{text}");
        }
    }

    #[test]
    fn overlapping_folders_give_the_validate_message() {
        let mut s = WikiSettings::new(WikiState::NotAWiki);
        s.set_sources("/wiki/raw");
        let message = s.declaration().unwrap_err();
        let mut d = WikiDeclaration::defaults();
        d.sources = "/wiki/raw".into();
        assert_eq!(message, d.validate().unwrap_err());
        assert_eq!(s.error(), Some(message));
    }

    #[test]
    fn the_property_text_is_split_on_commas_and_spaces_and_cleaned() {
        let mut s = wiki();
        s.set_properties("summary, owner  owner,,");
        assert_eq!(
            s.declaration().unwrap().required_properties,
            vec!["summary".to_string(), "owner".to_string()]
        );
    }

    #[test]
    fn an_empty_property_text_is_an_empty_list() {
        let mut s = wiki();
        s.set_properties("  ");
        assert!(s.declaration().unwrap().required_properties.is_empty());
    }

    #[test]
    fn apply_is_not_possible_without_a_vault_or_with_an_error() {
        assert!(!WikiSettings::new(WikiState::NoVault).can_apply());
        let mut s = WikiSettings::new(WikiState::NotAWiki);
        assert!(s.can_apply());
        s.set_sources("/wiki");
        assert!(!s.can_apply());
    }

    #[test]
    fn apply_is_possible_when_the_draft_equals_the_saved_declaration() {
        let s = wiki();
        assert_eq!(s.declaration().unwrap(), saved());
        assert!(s.can_apply());
    }

    #[test]
    fn the_button_label_follows_the_state() {
        assert_eq!(
            WikiSettings::new(WikiState::NotAWiki).button_label(),
            "Set up wiki"
        );
        assert_eq!(
            WikiSettings::new(WikiState::Broken("x".into())).button_label(),
            "Set up wiki"
        );
        assert_eq!(wiki().button_label(), "Apply");
    }

    #[test]
    fn the_effect_note_names_the_protection_change_when_sources_move() {
        let mut s = wiki();
        s.set_sources("raw");
        assert_eq!(
            s.effect_note(false),
            vec![
                "Notes in /sources stay where they are and are no longer protected. /raw is the new protected folder."
                    .to_string()
            ]
        );
    }

    #[test]
    fn the_effect_note_says_old_notes_stay_when_the_wiki_folder_changes() {
        let mut s = wiki();
        s.set_wiki("/atlas");
        assert_eq!(
            s.effect_note(false),
            vec!["Notes in the old wiki folder, log, and schema stay where they are.".to_string()]
        );
        let mut s = wiki();
        s.set_log("/wiki/journal.md");
        assert_eq!(s.effect_note(false).len(), 1);
    }

    #[test]
    fn the_effect_note_adds_the_schema_line_only_when_the_schema_exists_and_something_changed() {
        let line = "The schema note keeps its text. Edit it so that it names the new layout.";
        let mut s = wiki();
        s.set_properties("summary");
        assert!(!s.effect_note(false).iter().any(|l| l == line));
        assert_eq!(s.effect_note(true), vec![line.to_string()]);
        let mut s = wiki();
        s.set_sources("raw");
        assert_eq!(s.effect_note(true).last().unwrap(), line);
    }

    #[test]
    fn the_effect_note_is_empty_when_nothing_changed() {
        assert!(wiki().effect_note(true).is_empty());
        assert!(
            WikiSettings::new(WikiState::NotAWiki)
                .effect_note(true)
                .is_empty()
        );
    }

    // ── async operations, on a real vault in a temp folder ──

    use kimun_core::WikiInitEntry;
    use tempfile::TempDir;

    fn ws(dir: &TempDir) -> SystemPath {
        SystemPath::try_absolute(dir.path()).unwrap()
    }

    fn declaration_file(dir: &TempDir) -> std::path::PathBuf {
        dir.path().join(".kimun").join("wiki.toml")
    }

    fn entry(what: &str, created: bool) -> WikiInitEntry {
        WikiInitEntry {
            what: what.to_string(),
            created,
        }
    }

    #[tokio::test]
    async fn load_maps_the_three_states() {
        let dir = TempDir::new().unwrap();
        let loaded = load(ws(&dir), None).await;
        assert_eq!(loaded.state, WikiState::NotAWiki);
        assert!(!loaded.schema_exists);

        setup_defaults(ws(&dir), None).await.unwrap();
        let loaded = load(ws(&dir), None).await;
        assert_eq!(loaded.state, WikiState::Wiki(WikiDeclaration::defaults()));
        assert!(loaded.schema_exists);

        std::fs::write(declaration_file(&dir), "not = [valid").unwrap();
        let loaded = load(ws(&dir), None).await;
        assert!(matches!(loaded.state, WikiState::Broken(_)));
        assert!(!loaded.schema_exists);
    }

    #[tokio::test]
    async fn load_says_when_the_schema_note_is_missing() {
        let dir = TempDir::new().unwrap();
        setup_defaults(ws(&dir), None).await.unwrap();
        std::fs::remove_file(dir.path().join("wiki").join("schema.md")).unwrap();
        let loaded = load(ws(&dir), None).await;
        assert!(matches!(loaded.state, WikiState::Wiki(_)));
        assert!(!loaded.schema_exists);
    }

    #[tokio::test]
    async fn load_of_a_missing_vault_is_broken_not_a_panic() {
        let dir = TempDir::new().unwrap();
        let gone = ws(&dir).join("nope");
        let loaded = load(gone, None).await;
        assert!(matches!(loaded.state, WikiState::Broken(_)));
        assert!(!loaded.schema_exists);
    }

    #[tokio::test]
    async fn apply_creates_the_layout_and_reports_it() {
        let dir = TempDir::new().unwrap();
        let d = WikiDeclaration::defaults();
        let report = apply(ws(&dir), None, d.clone()).await.unwrap();
        assert!(report.entries.iter().any(|e| e.created));
        assert!(dir.path().join("sources").is_dir());
        assert!(dir.path().join("wiki").join("log.md").is_file());
        assert!(declaration_file(&dir).is_file());
        assert_eq!(load(ws(&dir), None).await.state, WikiState::Wiki(d));
    }

    #[tokio::test]
    async fn apply_with_a_note_in_the_way_fails_and_keeps_the_file() {
        let dir = TempDir::new().unwrap();
        setup_defaults(ws(&dir), None).await.unwrap();
        let before = std::fs::read(declaration_file(&dir)).unwrap();
        std::fs::write(dir.path().join("other"), "a plain file").unwrap();
        let mut d = WikiDeclaration::defaults();
        d.sources = "/other".to_string();
        let err = apply(ws(&dir), None, d).await.unwrap_err();
        assert!(!err.is_empty());
        assert_eq!(before, std::fs::read(declaration_file(&dir)).unwrap());
    }

    #[tokio::test]
    async fn setup_defaults_creates_a_wiki() {
        let dir = TempDir::new().unwrap();
        let outcome = setup_defaults(ws(&dir), None).await.unwrap();
        let PaletteOutcome::Created(labels) = outcome else {
            panic!("expected Created, got {outcome:?}");
        };
        assert!(!labels.is_empty());
        assert!(labels.iter().any(|l| l.contains("sources")));
        assert!(declaration_file(&dir).is_file());
    }

    #[tokio::test]
    async fn setup_defaults_on_a_wiki_is_already_a_wiki() {
        let dir = TempDir::new().unwrap();
        setup_defaults(ws(&dir), None).await.unwrap();
        assert_eq!(
            setup_defaults(ws(&dir), None).await.unwrap(),
            PaletteOutcome::AlreadyAWiki
        );
    }

    #[tokio::test]
    async fn setup_defaults_on_a_broken_declaration_reports_it_and_changes_nothing() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".kimun")).unwrap();
        std::fs::write(declaration_file(&dir), "not = [valid").unwrap();
        let outcome = setup_defaults(ws(&dir), None).await.unwrap();
        let PaletteOutcome::BrokenDeclaration(message) = outcome else {
            panic!("expected BrokenDeclaration, got {outcome:?}");
        };
        assert!(message.contains("wiki.toml"));
        assert_eq!(
            std::fs::read_to_string(declaration_file(&dir)).unwrap(),
            "not = [valid"
        );
    }

    #[tokio::test]
    async fn setup_defaults_after_the_wiki_folder_was_deleted_reports_created() {
        let dir = TempDir::new().unwrap();
        setup_defaults(ws(&dir), None).await.unwrap();
        std::fs::remove_dir_all(dir.path().join("wiki")).unwrap();
        let outcome = setup_defaults(ws(&dir), None).await.unwrap();
        let PaletteOutcome::Created(labels) = outcome else {
            panic!("expected Created, got {outcome:?}");
        };
        assert!(labels.iter().any(|l| l.contains("wiki")));
        assert!(dir.path().join("wiki").join("log.md").is_file());
    }

    #[test]
    fn report_lines_use_created_and_kept() {
        let report = WikiInitReport {
            entries: vec![
                entry("folder /raw", true),
                entry("note /wiki/log.md", false),
            ],
        };
        assert_eq!(
            report_lines(&report),
            vec![
                "Created folder /raw.".to_string(),
                "Kept note /wiki/log.md.".to_string()
            ]
        );
    }
}
