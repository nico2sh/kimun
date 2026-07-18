//! The Ask workspace's coordination layer: Thread↔Sources sync, capability
//! refresh, async-result routing, and the show/stash transitions that keep the
//! resident thread alive across drawer view switches (adr/0030). The editor
//! screen keeps only "which panel content is showing"; everything that decides
//! *what the Ask panels should reflect* lives here, against `PanelSet`.

use std::sync::Arc;

use kimun_core::NoteVault;

use crate::app_screen::panel_set::PanelSet;
use crate::components::drawer::DrawerView;
use crate::components::events::{AppTx, AskData};
use crate::rag::RagStatus;
use crate::settings::SharedSettings;

/// Coordinates the Ask workspace's moving parts so the screen doesn't have to:
/// the resident `ThreadPanel`, the Sources drawer, and the background capability
/// probe all meet here.
#[derive(Default)]
pub struct AskCoordinator {
    /// The turn id last mirrored into the Sources drawer — `sync_sources`
    /// compares against this instead of a dirty flag on `ThreadPanel`, since
    /// turn ids are never reused (`Thread::bump` only increments), a stale
    /// value here can never falsely match a fresh selection. Reset to `None`
    /// when leaving Ask so re-entering always re-syncs.
    last_synced_turn: Option<u64>,
}

impl AskCoordinator {
    /// Select or deselect the editor-area Ask content to match a drawer view
    /// switch. Entering ASK shows the resident thread and syncs its Sources;
    /// leaving ASK returns the note editor. The thread survives either way by
    /// residency — this only flips the content selector.
    pub fn transition(&mut self, panels: &mut PanelSet, target: DrawerView, tx: &AppTx) {
        let showing = panels.is_showing_ask();
        if target == DrawerView::Ask && !showing {
            panels.show_ask();
            self.sync_sources_from_selected(panels, tx);
        } else if target != DrawerView::Ask && showing {
            panels.hide_ask();
        }
    }

    /// Deselect the Ask content so the note editor (or an attachment) can take
    /// the editor area. The resident thread panel is untouched — the
    /// conversation survives by residency.
    ///
    /// Also switches the drawer off Ask (to FILES) when it's still showing the
    /// conversation's Sources: a hidden thread's sources shouldn't linger, and
    /// leaving the drawer's active view on `Ask` would make the rail's next
    /// ASK click read as a toggle-off (drawer already "on" Ask) instead of a
    /// re-open. The switch uses `switch_drawer_view` (not `open_drawer_view`)
    /// so it never force-reveals a drawer the user explicitly hid.
    pub fn hide_if_shown(&mut self, panels: &mut PanelSet) {
        if panels.is_showing_ask() {
            panels.hide_ask();
            panels.switch_drawer_view(DrawerView::Files);
            // Force a re-sync on the next Ask entry — see the field doc.
            self.last_synced_turn = None;
        }
    }

    /// Push the Ask capability decision into the resident panel: rebuild the
    /// client from config when the server can answer questions, else clear it,
    /// so the composer-enabled state follows client presence (no
    /// forever-`Thinking` turn; carry-forward #2). `set_ask_client` is the
    /// single injection point, so this one call is enough.
    pub async fn refresh_capability(
        &self,
        panels: &mut PanelSet,
        settings: &SharedSettings,
        vault: &Arc<NoteVault>,
        rag_status: RagStatus,
    ) {
        let client = if rag_status.llm_available() {
            crate::rag::rag_client(settings, vault).await.map(Arc::new)
        } else {
            None
        };
        panels.set_ask_client(client);
    }

    /// Route an Ask async result. The answer always lands on the resident
    /// thread panel — whether or not Ask is on screen (an answer may complete
    /// while the user browses FILES). A reader note goes to the Sources
    /// drawer. When Ask *is* shown and the completed turn is the selected one,
    /// its sources are refreshed (rule 3).
    pub fn handle_data(&mut self, panels: &mut PanelSet, data: AskData, tx: &AppTx) {
        match &data {
            AskData::AnswerReady { turn_id, .. } => {
                let turn_id = *turn_id;
                panels.ask_mut().handle_data(data);
                // Refresh Sources only when Ask is shown and the completed turn
                // is the one selected.
                if panels.is_showing_ask() {
                    let panel = panels.ask_mut();
                    let selected = panel.thread().selected().map(|t| t.id);
                    if selected == Some(turn_id) {
                        let sources = panel
                            .thread()
                            .turns()
                            .iter()
                            .find(|t| t.id == turn_id)
                            .map(|t| t.sources.clone())
                            .unwrap_or_default();
                        panels.ask_sources_mut().refresh(turn_id, sources, tx);
                    }
                }
            }
            AskData::ReaderNote { .. } => {
                panels.ask_sources_mut().handle_data(data);
            }
        }
    }

    /// Mirror the live Ask panel's selected turn into the Sources drawer when
    /// it differs from `last_synced_turn`, consuming the per-input citation
    /// signal. Called after every Ask input event (keyboard and mouse) so the
    /// drawer tracks the thread.
    pub fn sync_sources(&mut self, panels: &mut PanelSet, tx: &AppTx) {
        // Only track the drawer to the thread while Ask is actually on screen.
        if !panels.is_showing_ask() {
            return;
        }
        let (selected, citation) = {
            let panel = panels.ask_mut();
            (
                panel.thread().selected().map(|t| t.id),
                panel.take_citation_target(),
            )
        };
        if selected.is_some() && selected != self.last_synced_turn {
            self.sync_sources_from_selected(panels, tx);
        }
        if let Some(ordinal) = citation {
            panels.ask_sources_mut().focus_source(ordinal);
        }
    }

    /// Point the Sources drawer at the live thread's currently-selected turn,
    /// unconditionally — the switch-into-Ask sync (rule 1), the post-answer
    /// refresh, and `sync_sources` above (once it knows the selection
    /// changed) all need the current turn's sources shown. Updates
    /// `last_synced_turn` so a following `sync_sources` call doesn't redo
    /// the same work.
    pub fn sync_sources_from_selected(&mut self, panels: &mut PanelSet, tx: &AppTx) {
        let selected = panels
            .ask_mut()
            .thread()
            .selected()
            .map(|t| (t.id, t.sources.clone()));
        if let Some((id, sources)) = selected {
            panels.ask_sources_mut().set_turn(id, sources, tx);
            self.last_synced_turn = Some(id);
        }
    }
}
