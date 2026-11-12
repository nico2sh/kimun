//! The note reader under a list host's preview: a [`PreviewPane`] plus the
//! async load of the note it shows, anchored on one matched section. SEM
//! (semantic search) and the Ask Sources drawer share it, so the load
//! lifecycle — stale drop by path, same-note section moves without a
//! refetch, the loading and failed placeholders — lives once.
//!
//! The load lands through the app channel: the host's `handle_data` hands
//! the [`AppMessage`] payload back to [`NoteReader::deliver`].

use std::ops::Range;
use std::sync::Arc;

use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

use crate::components::preview_pane::{Highlight, PreviewPane};
use crate::logic::message::{AppMessage, AppTx};
use crate::settings::themes::Theme;

/// What the reader shows: a note, and the matched section in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteAnchor {
    pub path: VaultPath,
    /// The section's citation ordinal (its identity within the note).
    pub ordinal: usize,
    pub heading: String,
    pub chunk: String,
}

/// Find the anchor's section in the note's text: the byte range the
/// preview highlights and scrolls to, or `None` (the note from the top).
pub type Locate = fn(&str, &NoteAnchor) -> Option<Range<usize>>;

/// Wrap a landed load as the host's app message.
pub type Deliver = fn(VaultPath, Option<String>) -> AppMessage;

/// Where the anchored note's text stands.
enum Content {
    /// The note load is in flight.
    Loading,
    /// The note text, and its matched section's byte range, if found.
    Loaded {
        text: String,
        highlight: Option<Range<usize>>,
    },
    /// The note load failed.
    Failed,
}

struct LoadedNote {
    anchor: NoteAnchor,
    content: Content,
}

pub struct NoteReader {
    /// The preview surface (expand state, scroll, render).
    pub(crate) preview: PreviewPane,
    /// The note behind the preview. `None` until a preview first opens.
    loaded: Option<LoadedNote>,
    vault: Arc<NoteVault>,
    locate: Locate,
    deliver: Deliver,
}

impl NoteReader {
    pub fn new(vault: Arc<NoteVault>, locate: Locate, deliver: Deliver) -> Self {
        Self {
            preview: PreviewPane::new(),
            loaded: None,
            vault,
            locate,
            deliver,
        }
    }

    /// Forget the note, so the next open loads it fresh.
    pub fn drop_text(&mut self) {
        self.loaded = None;
    }

    /// Back the preview with `anchor`'s note. The same anchor does nothing;
    /// another section of the loaded note moves the highlight and
    /// re-anchors without a refetch; another note starts a load, whose
    /// result comes back through `tx` (an earlier note's late result is
    /// then dropped on arrival).
    pub fn ensure(&mut self, anchor: NoteAnchor, tx: &AppTx) {
        if let Some(l) = &mut self.loaded
            && l.anchor.path == anchor.path
        {
            if l.anchor != anchor {
                if let Content::Loaded { text, highlight } = &mut l.content {
                    *highlight = (self.locate)(text, &anchor);
                }
                l.anchor = anchor;
                self.preview.re_anchor();
            }
            return;
        }
        let path = anchor.path.clone();
        self.loaded = Some(LoadedNote {
            anchor,
            content: Content::Loading,
        });
        let vault = self.vault.clone();
        let deliver = self.deliver;
        let tx = tx.clone();
        tokio::spawn(async move {
            let text = vault.get_note_text(&path).await.ok();
            tx.send(deliver(path, text)).ok();
        });
    }

    /// A landed load: kept only when it is the anchored note's.
    pub fn deliver(&mut self, path: VaultPath, text: Option<String>) {
        let Some(l) = &mut self.loaded else {
            return;
        };
        if l.anchor.path != path {
            return;
        }
        l.content = match text {
            Some(text) => {
                let highlight = (self.locate)(&text, &l.anchor);
                Content::Loaded { text, highlight }
            }
            None => Content::Failed,
        };
    }

    /// Draw the preview (Full with `title`/`filename` in its header, or
    /// Context) into `area`, or the load's placeholder. Only `selected`'s
    /// text is painted: another note's shows as still loading.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        f: &mut Frame,
        area: Rect,
        full: bool,
        selected: Option<&VaultPath>,
        title: &str,
        filename: &str,
        theme: &Theme,
    ) {
        let content = self
            .loaded
            .as_ref()
            .filter(|l| selected == Some(&l.anchor.path))
            .map(|l| &l.content);
        match content {
            Some(Content::Loaded { text, highlight }) => {
                let highlight = Highlight::Range(highlight.as_ref());
                if full {
                    self.preview
                        .render_full(f, area, title, filename, text, highlight, theme);
                } else {
                    self.preview.render_context(f, area, text, highlight, theme);
                }
            }
            Some(Content::Failed) => {
                let red = Style::default().fg(theme.red.to_ratatui());
                f.render_widget(Paragraph::new("failed to load note").style(red), area);
            }
            None | Some(Content::Loading) => {
                let dim = Style::default().fg(theme.gray.to_ratatui());
                f.render_widget(Paragraph::new("loading\u{2026}").style(dim), area);
            }
        }
    }
}

#[cfg(test)]
impl NoteReader {
    pub(crate) fn has_note(&self) -> bool {
        self.loaded.is_some()
    }

    pub(crate) fn anchor(&self) -> Option<&NoteAnchor> {
        self.loaded.as_ref().map(|l| &l.anchor)
    }

    pub(crate) fn text(&self) -> Option<&str> {
        match self.loaded.as_ref().map(|l| &l.content) {
            Some(Content::Loaded { text, .. }) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn highlight(&self) -> Option<Range<usize>> {
        match self.loaded.as_ref().map(|l| &l.content) {
            Some(Content::Loaded { highlight, .. }) => highlight.clone(),
            _ => None,
        }
    }

    pub(crate) fn is_loading(&self) -> bool {
        matches!(
            self.loaded.as_ref().map(|l| &l.content),
            Some(Content::Loading)
        )
    }

    pub(crate) fn is_failed(&self) -> bool {
        matches!(
            self.loaded.as_ref().map(|l| &l.content),
            Some(Content::Failed)
        )
    }

    /// Anchor on `anchor` with its load in flight, without spawning it.
    pub(crate) fn set_loading(&mut self, anchor: NoteAnchor) {
        self.loaded = Some(LoadedNote {
            anchor,
            content: Content::Loading,
        });
    }

    /// Anchor on `path` with `text` already loaded and `highlight` set.
    pub(crate) fn set_loaded(
        &mut self,
        path: VaultPath,
        text: String,
        highlight: Option<Range<usize>>,
    ) {
        self.loaded = Some(LoadedNote {
            anchor: NoteAnchor {
                path,
                ordinal: 0,
                heading: String::new(),
                chunk: String::new(),
            },
            content: Content::Loaded { text, highlight },
        });
    }

    /// Anchor on `path` with a failed load.
    pub(crate) fn set_failed(&mut self, path: VaultPath) {
        self.set_loading(NoteAnchor {
            path,
            ordinal: 0,
            heading: String::new(),
            chunk: String::new(),
        });
        self.deliver(self.anchor().unwrap().path.clone(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locate(text: &str, a: &NoteAnchor) -> Option<Range<usize>> {
        text.find(&a.chunk).map(|s| s..s + a.chunk.len())
    }

    fn deliver(path: VaultPath, text: Option<String>) -> AppMessage {
        AppMessage::Semantic(crate::logic::message::SemanticData::PreviewNote { path, text })
    }

    fn anchor(path: &str, ordinal: usize, chunk: &str) -> NoteAnchor {
        NoteAnchor {
            path: VaultPath::new(path),
            ordinal,
            heading: String::new(),
            chunk: chunk.into(),
        }
    }

    async fn reader() -> NoteReader {
        let vault = crate::test_support::temp_vault("note-reader").await;
        NoteReader::new(vault, locate, deliver)
    }

    #[tokio::test]
    async fn a_load_for_another_path_is_dropped() {
        let mut r = reader().await;
        r.set_loading(anchor("a.md", 1, "x"));
        r.deliver(VaultPath::new("b.md"), Some("x".into()));
        assert!(r.is_loading());
        r.deliver(VaultPath::new("a.md"), Some("hi x".into()));
        assert_eq!(r.text(), Some("hi x"));
        assert_eq!(r.highlight(), Some(3..4));
    }

    #[tokio::test]
    async fn another_section_of_the_loaded_note_moves_the_highlight() {
        let mut r = reader().await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        r.set_loading(anchor("a.md", 1, "one"));
        r.deliver(VaultPath::new("a.md"), Some("one two".into()));
        r.ensure(anchor("a.md", 2, "two"), &tx);
        assert_eq!(r.highlight(), Some(4..7), "no reload, highlight moved");
        assert_eq!(r.anchor().map(|a| a.ordinal), Some(2));
        assert!(rx.try_recv().is_err(), "no load was started");
    }

    #[tokio::test]
    async fn a_failed_load_is_recorded() {
        let mut r = reader().await;
        r.set_loading(anchor("a.md", 1, "x"));
        r.deliver(VaultPath::new("a.md"), None);
        assert!(r.is_failed());
    }
}
