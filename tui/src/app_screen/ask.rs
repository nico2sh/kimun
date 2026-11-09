//! The Ask workspace's coordination layer: Thread↔Sources sync, capability
//! refresh, async-result routing, and the show/stash transitions that keep the
//! resident thread alive across drawer view switches. The rules are the
//! logic's [`AskFlow`]; this layer only executes its answers against
//! `PanelSet`.

use std::sync::Arc;

use kimun_core::NoteVault;

use crate::app_screen::panel_set::PanelSet;
use crate::logic::ask_flow::{AskFlow, Transition};
use crate::logic::message::{AppTx, AskData};
use crate::logic::nav::DrawerView;
use crate::rag::RagStatus;
use crate::settings::SharedSettings;

/// Coordinates the Ask workspace's moving parts so the screen doesn't have to:
/// the resident `ThreadPanel`, the Sources drawer, and the background capability
/// probe all meet here. Owns cheap clones of the settings and vault handles so
/// capability refreshes don't thread them through every call.
pub struct AskCoordinator {
    settings: SharedSettings,
    vault: Arc<NoteVault>,
    /// The show/sync rules. `PanelSet` stays the record of what the editor area
    /// shows; each entry point adopts it first (`AskFlow::sync_showing`).
    flow: AskFlow,
}

impl AskCoordinator {
    pub fn new(settings: SharedSettings, vault: Arc<NoteVault>) -> Self {
        Self {
            settings,
            vault,
            flow: AskFlow::new(),
        }
    }

    /// Select or deselect the editor-area Ask content to match a drawer view
    /// switch. Entering ASK shows the resident thread and syncs its Sources;
    /// leaving ASK returns the note editor. The thread survives either way by
    /// residency — this only flips the content selector.
    pub fn transition(&mut self, panels: &mut PanelSet, target: DrawerView, tx: &AppTx) {
        self.flow.sync_showing(panels.is_showing_ask());
        match self.flow.switch_to(target == DrawerView::Ask) {
            Transition::Enter => {
                panels.show_ask();
                self.sync_sources_from_selected(panels, tx);
            }
            Transition::Leave => panels.hide_ask(),
            Transition::Stay => {}
        }
    }

    /// Deselect the Ask content so the note editor (or an attachment) can take
    /// the editor area. The resident thread panel is untouched — the
    /// conversation survives by residency.
    ///
    /// Also switches the drawer off Ask (to FILES) when it's still showing the
    /// conversation's Sources (see `AskFlow::hide_if_shown`). The switch uses
    /// `switch_drawer_view` (not `open_drawer_view`) so it never force-reveals
    /// a drawer the user explicitly hid.
    pub fn hide_if_shown(&mut self, panels: &mut PanelSet) {
        self.flow.sync_showing(panels.is_showing_ask());
        if self.flow.hide_if_shown() {
            panels.hide_ask();
            panels.switch_drawer_view(DrawerView::Files);
        }
    }

    /// Push the Ask capability decision into the resident panel: rebuild the
    /// client from config when the server can answer questions, else clear it,
    /// so the composer-enabled state follows client presence (no
    /// forever-`Thinking` turn; carry-forward #2). `set_ask_client` is the
    /// single injection point, so this one call is enough.
    pub async fn refresh_capability(&self, panels: &mut PanelSet, rag_status: RagStatus) {
        let client =
            crate::logic::rag_caps::ask_client(&self.settings, &self.vault, rag_status).await;
        panels.set_ask_client(client);
    }

    /// Route an Ask async result. The answer always lands on the resident
    /// thread panel — whether or not Ask is on screen (an answer may complete
    /// while the user browses FILES). A reader note goes to the Sources
    /// drawer. When Ask *is* shown and the completed turn is the selected one,
    /// its sources are refreshed (rule 3).
    pub fn handle_data(&mut self, panels: &mut PanelSet, data: AskData, tx: &AppTx) {
        self.flow.sync_showing(panels.is_showing_ask());
        match &data {
            AskData::AnswerReady { turn_id, .. } => {
                let turn_id = *turn_id;
                panels.ask_mut().handle_data(data);
                let selected = panels.ask().thread().selected().map(|t| t.id);
                if self.flow.answer_ready(turn_id, selected) {
                    let sources = panels
                        .ask()
                        .thread()
                        .turns()
                        .iter()
                        .find(|t| t.id == turn_id)
                        .map(|t| t.sources.clone())
                        .unwrap_or_default();
                    panels.ask_sources_mut().refresh(turn_id, sources, tx);
                }
            }
            AskData::ReaderNote { .. } => {
                panels.ask_sources_mut().handle_data(data);
            }
        }
    }

    /// Mirror the live Ask panel's selected turn into the Sources drawer when
    /// it differs from the last synced turn, consuming the per-input citation
    /// signal. Called after every Ask input event (keyboard and mouse) so the
    /// drawer tracks the thread.
    pub fn sync_sources(&mut self, panels: &mut PanelSet, tx: &AppTx) {
        self.flow.sync_showing(panels.is_showing_ask());
        // Only track the drawer to the thread while Ask is actually on screen
        // (the citation signal stays untaken otherwise).
        if !self.flow.showing() {
            return;
        }
        let (selected, citation) = {
            let panel = panels.ask_mut();
            (
                panel.thread().selected().map(|t| t.id),
                panel.take_citation_target(),
            )
        };
        let step = self.flow.after_input(selected, citation);
        if step.sync_sources {
            self.sync_sources_from_selected(panels, tx);
        }
        if let Some(ordinal) = step.focus_citation {
            panels.ask_sources_mut().focus_source(ordinal);
        }
    }

    /// Point the Sources drawer at the live thread's currently-selected turn,
    /// unconditionally — the switch-into-Ask sync (rule 1), the post-answer
    /// refresh, and `sync_sources` above (once it knows the selection
    /// changed) all need the current turn's sources shown. Records the synced
    /// turn so a following `sync_sources` call doesn't redo the same work.
    pub fn sync_sources_from_selected(&mut self, panels: &mut PanelSet, tx: &AppTx) {
        let selected = panels
            .ask()
            .thread()
            .selected()
            .map(|t| (t.id, t.sources.clone()));
        if let Some((id, sources)) = selected {
            panels.ask_sources_mut().set_turn(id, sources, tx);
            self.flow.synced(id);
        }
    }
}
