//! Semantic search surface (P4): the drawer view over the logic's server-backed
//! [`SemanticSource`], which queries the RAG server for similar chunks and
//! lists the matching notes. Only usable when a server is configured and
//! reachable. This file keeps the panel (render + keys).

use std::sync::Arc;

use kimun_core::NoteVault;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::components::event_state::EventState;
use crate::components::events::InputEvent;
use crate::components::file_list::FileListEntry;
use crate::components::query_list_panel::{ListPanelSpec, QueryListPanel};
use crate::logic::message::{AppMessage, AppTx, FileOp};
pub use crate::logic::views::SemanticSource;
#[cfg(test)]
use crate::logic::views::chunks_to_entries;
#[cfg(test)]
use crate::server_client::ChunkResult;
use crate::settings::SharedSettings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

/// Spec for the semantic drawer view: a query input over server-ranked note
/// results; Enter/click opens the note, right-click opens the file-ops menu.
pub struct SemanticSpec;

impl ListPanelSpec for SemanticSpec {
    type Row = FileListEntry;
    const TITLE: &'static str = "Semantic";
    const HAS_FILTER: bool = true;
    // Server-backed query: draw a bordered search box separated from the results
    // and a "Searching…" indicator while the request is in flight.
    const BORDERED_INPUT: bool = true;
    // The server already ranks/filters by the query; don't re-filter its
    // semantic results locally by the literal query text (that discards
    // conceptually-relevant notes whose titles lack the words).
    const LOCAL_FILTER: bool = false;

    fn submit(row: &FileListEntry, tx: &AppTx) {
        if let FileListEntry::Note { path, .. } = row {
            tx.send(AppMessage::open(path.clone())).ok();
        }
    }

    fn context_event(row: &FileListEntry) -> Option<AppMessage> {
        match row {
            FileListEntry::Note { path, .. } => {
                Some(AppMessage::FileOp(FileOp::ShowMenu(path.clone())))
            }
            _ => None,
        }
    }

    fn hints() -> Vec<(String, String)> {
        vec![("Enter".into(), "Open".into())]
    }
}

/// The SEMANTIC drawer view: type a query, see the notes the RAG server ranks
/// most similar. Only meaningful when a server is configured + reachable.
pub struct SemanticPanel {
    vault: Arc<NoteVault>,
    settings: SharedSettings,
    body: QueryListPanel<SemanticSpec>,
    source_installed: bool,
}

impl SemanticPanel {
    pub fn new(
        vault: Arc<NoteVault>,
        settings: SharedSettings,
        icons: Icons,
        yank_combos: Vec<crate::keys::key_combo::KeyCombo>,
    ) -> Self {
        Self {
            vault,
            settings,
            body: QueryListPanel::new(icons, yank_combos),
            source_installed: false,
        }
    }

    /// Installs the server-backed source the first time the view is opened.
    pub fn ensure_source(&mut self, tx: &AppTx) {
        if !self.source_installed {
            self.body.set_source(
                SemanticSource::new(self.vault.clone(), self.settings.clone()),
                tx,
            );
            self.source_installed = true;
        }
    }

    pub fn hint_shortcuts(&self) -> Vec<(String, String)> {
        self.body.hint_shortcuts()
    }

    pub fn handle_input(&mut self, event: &InputEvent, tx: &AppTx) -> EventState {
        self.body.handle_input(event, tx)
    }

    pub fn render(&mut self, f: &mut Frame, rect: Rect, theme: &Theme, focused: bool) {
        self.body.render(f, rect, theme, focused);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_core::VaultConfig;
    use tempfile::TempDir;

    fn chunk(path: &str, title: &str, score: f64) -> ChunkResult {
        ChunkResult {
            path: path.to_string(),
            title: title.to_string(),
            date: None,
            content: String::new(),
            hash: String::new(),
            similarity_score: score,
            ordinal: 0,
        }
    }

    #[tokio::test]
    async fn dedups_by_note_keeping_first_and_preserves_order() {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(crate::test_support::sys(dir.path())))
            .await
            .unwrap();

        // Two chunks of "/a.md" (best first) then one of "/b.md".
        let chunks = vec![
            chunk("/a.md", "A > Intro", 0.9),
            chunk("/a.md", "A > Details", 0.7),
            chunk("/b.md", "B", 0.5),
        ];
        let entries = chunks_to_entries(chunks, &vault);
        assert_eq!(entries.len(), 2);
        match (&entries[0], &entries[1]) {
            (
                FileListEntry::Note {
                    path: pa,
                    title: ta,
                    ..
                },
                FileListEntry::Note { path: pb, .. },
            ) => {
                assert_eq!(pa.to_string(), "/a.md");
                assert_eq!(ta, "A > Intro"); // the higher-ranked chunk won
                assert_eq!(pb.to_string(), "/b.md");
            }
            _ => panic!("expected note entries"),
        }
    }
}
