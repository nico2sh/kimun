//! Semantic search surface (P4): the drawer view over the logic's server-backed
//! [`SemanticSource`], which queries the RAG server for similar chunks and
//! lists the matching notes. Only usable when a server is configured and
//! reachable. This file keeps the panel (render + keys).

use std::sync::Arc;

use kimun_core::NoteVault;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::ListItem;

use crate::components::event_state::EventState;
use crate::components::events::InputEvent;
use crate::components::file_list::FileListEntry;
use crate::components::query_list_panel::{ListPanelSpec, QueryListPanel};
use crate::components::rich_row::RichRow;
use crate::components::search_list::SearchRow;
use crate::logic::message::{AppMessage, AppTx, FileOp};
#[cfg(test)]
use crate::logic::views::chunks_to_entries;
pub use crate::logic::views::{SemanticHit, SemanticSource};
#[cfg(test)]
use crate::server_client::ChunkResult;
use crate::settings::SharedSettings;
use crate::settings::icons::Icons;
use crate::settings::themes::Theme;

impl SearchRow for SemanticHit {
    /// The note's row (glyph, open marker, journal date, file name) with the
    /// matched section's title and the score in place of its title: `Title ·
    /// 87%`. A blank title shows the file name.
    fn to_list_item(&self, theme: &Theme, icons: &Icons, _selected: bool) -> ListItem<'static> {
        let FileListEntry::Note {
            filename,
            journal_date,
            is_open,
            ..
        } = &self.entry
        else {
            return self.entry.to_list_item(theme, icons);
        };
        let title = if self.title.trim().is_empty() {
            filename.as_str()
        } else {
            self.title.as_str()
        };
        let percent = (self.score * 100.0).round().clamp(0.0, 100.0) as u32;
        let glyph = if journal_date.is_some() {
            icons.journal
        } else {
            icons.note
        };
        let mut row =
            RichRow::new(glyph, format!("{title} \u{b7} {percent}%")).filename(filename.clone());
        if *is_open {
            row = row.glyph_style(Style::default().fg(theme.accent.to_ratatui()));
        }
        if let Some(date) = journal_date {
            row = row.secondary(
                date.clone(),
                Some(Style::default().fg(theme.color_journal_date.to_ratatui())),
            );
        }
        row.into_list_item(theme)
    }

    fn visual_height(&self) -> u16 {
        self.entry.visual_height()
    }
}

/// Spec for the semantic drawer view: a query input over server-ranked note
/// results; Enter/click opens the note, right-click opens the file-ops menu.
pub struct SemanticSpec;

impl ListPanelSpec for SemanticSpec {
    type Row = SemanticHit;
    const TITLE: &'static str = "Semantic";
    const HAS_FILTER: bool = true;
    // Server-backed query: draw a bordered search box separated from the results
    // and a "Searching…" indicator while the request is in flight.
    const BORDERED_INPUT: bool = true;
    // The server already ranks/filters by the query; don't re-filter its
    // semantic results locally by the literal query text (that discards
    // conceptually-relevant notes whose titles lack the words).
    const LOCAL_FILTER: bool = false;

    fn submit(row: &SemanticHit, tx: &AppTx) {
        if let FileListEntry::Note { path, .. } = &row.entry {
            tx.send(AppMessage::open(path.clone())).ok();
        }
    }

    fn context_event(row: &SemanticHit) -> Option<AppMessage> {
        match &row.entry {
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

    fn hit(title: &str, journal_date: Option<&str>, score: f64) -> SemanticHit {
        SemanticHit {
            entry: FileListEntry::Note {
                path: kimun_core::nfs::VaultPath::note_path_from("a.md"),
                title: if title.is_empty() {
                    "<no title>".into()
                } else {
                    title.into()
                },
                filename: "a.md".into(),
                journal_date: journal_date.map(str::to_string),
                is_open: false,
            },
            title: title.into(),
            heading: title.into(),
            chunk: "the matched chunk".into(),
            ordinal: 1,
            score,
        }
    }

    fn render_text(row: &SemanticHit) -> String {
        use crate::components::search_list::SearchRow;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::widgets::List;
        let item = row.to_list_item(&Theme::default(), &Icons::new(false), false);
        let mut term = Terminal::new(TestBackend::new(40, 4)).unwrap();
        term.draw(|f| f.render_widget(List::new(vec![item]), f.area()))
            .unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_row_shows_the_title_the_score_and_the_filename() {
        let text = render_text(&hit("Intro", None, 0.874));
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].ends_with("Intro \u{b7} 87%"), "{text}");
        assert_eq!(lines[1], "  a.md");
    }

    #[test]
    fn a_blank_title_shows_the_file_name() {
        let text = render_text(&hit("", None, 0.5));
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].ends_with("a.md \u{b7} 50%"), "{text}");
        assert!(!text.contains("<no title>"), "{text}");
    }

    #[test]
    fn a_journal_hit_shows_the_stripped_title_and_its_date_line() {
        let text = render_text(&hit("Afternoon", Some("Wednesday, April 8, 2026"), 0.9));
        assert!(
            text.lines()
                .next()
                .unwrap()
                .ends_with("Afternoon \u{b7} 90%"),
            "{text}"
        );
        assert!(text.contains("Wednesday, April 8, 2026"), "{text}");
        assert!(!text.contains("2026-04-08Afternoon"), "{text}");
    }

    #[test]
    fn a_hit_row_is_as_tall_as_its_note_row() {
        use crate::components::search_list::SearchRow;
        assert_eq!(hit("A", None, 0.5).visual_height(), 2);
        assert_eq!(hit("A", Some("d"), 0.5).visual_height(), 3);
    }

    #[test]
    fn enter_opens_the_note_and_right_click_opens_its_file_menu() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let row = hit("Intro", None, 0.8);
        SemanticSpec::submit(&row, &tx);
        match rx.try_recv().unwrap() {
            AppMessage::OpenPath { path, .. } => {
                assert_eq!(path, kimun_core::nfs::VaultPath::note_path_from("a.md"))
            }
            other => panic!("unexpected {other:?}"),
        }
        match SemanticSpec::context_event(&row) {
            Some(AppMessage::FileOp(FileOp::ShowMenu(path))) => {
                assert_eq!(path, kimun_core::nfs::VaultPath::note_path_from("a.md"))
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn yank_and_match_text_are_the_note_rows() {
        use crate::logic::list::ListRow;
        let row = hit("Intro", None, 0.8);
        assert_eq!(row.match_text(), Some("a.md"));
        assert!(row.yank_target().is_some());
    }
}
