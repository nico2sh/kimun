//! The GUI's performer of a **Leader route** (`logic::leader_route`): what a
//! leader leaf does in this window, and whether it can do it yet. The route
//! is decided in logic, once for every door onto a leaf (the command
//! palette, the leader sequence); this file only maps each route onto the
//! window's surfaces — the same methods the chords and the rail use.
//!
//! A route the window has no surface for yet is **blocked** with a reason
//! ([`Workspace::route_block`]); the palette shows such rows disabled, saying
//! why, the leader flashes the reason, and [`Workspace::perform_route`] never
//! runs them.

use gpui_kit::{App, ClipboardItem, Context, Window};
use kimun_core::nfs::VaultPath;
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::layout::{Panel, Region, Side};
use kimun_notes::logic::leader_route::{
    self, LayoutStep, LeaderJob, LeaderRoute, RouteFacts, jobs,
};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::overlay_open::OverlayOpen;

use super::Workspace;
use crate::shell_layout::{self, RAIL_VIEWS};

/// Why a command cannot run here yet: no surface in the desktop app.
pub(crate) const NOT_YET: &str = "Not in the desktop app yet";
/// A leaf whose surface has not been built in either frontend.
pub(crate) const SOON: &str = "Coming soon";
/// The command acts on the open note and there is none.
pub(crate) const NO_NOTE: &str = "No note open";
/// The command needs a workspace (a vault) and the window has none.
pub(crate) const NO_WORKSPACE: &str = "No workspace open";
/// The command edits or searches the note and the note did not have the
/// keyboard when the palette opened.
pub(crate) const NEEDS_NOTE_FOCUS: &str = "Needs the note focused";
/// The command arranges the focused drawer view (or the rail's).
pub(crate) const NEEDS_DRAWER_FOCUS: &str = "Needs a drawer focused";
/// Resizing a drawer with none shown.
pub(crate) const NO_DRAWER: &str = "No drawer shown";

/// How far one grow or shrink step moves a drawer's edge, in rems (the TUI
/// moves it four columns).
const DRAWER_STEP_REMS: f32 = 2.;

impl Workspace {
    /// The facts a route reads: the open note (the vault root when none is
    /// open). The GUI has no Ask workspace yet.
    pub(crate) fn route_facts(&self) -> RouteFacts {
        let note = self
            .notes
            .as_ref()
            .and_then(|h| h.note.clone())
            .unwrap_or_else(VaultPath::root);
        RouteFacts::new(note, false)
    }

    /// The route of `action` in this window.
    pub(crate) fn route_of(&self, action: LeaderAction) -> LeaderRoute {
        leader_route::route(action, &self.route_facts())
    }

    /// Whether the editor is on screen and had the keyboard with its find
    /// bar closed — where a text action or a find may run. `opener` is the
    /// panel that had the keyboard before a modal took it.
    pub(crate) fn note_has_keyboard(&self, opener: Panel, cx: &App) -> bool {
        opener == Panel::Editor
            && self.empty.is_none()
            && self.note_empty_state().is_none()
            && !self.editor.read(cx).editor.find_open()
    }

    /// Why `route` cannot run in this window now, or `None` when it can.
    /// `opener` is the panel that had the keyboard when the command was
    /// asked for.
    pub(crate) fn route_block(
        &self,
        route: &LeaderRoute,
        opener: Panel,
        cx: &App,
    ) -> Option<&'static str> {
        let has_vault = self.notes.is_some();
        match route {
            LeaderRoute::Drawer(view) if shell_layout::is_on_rail(*view) => None,
            LeaderRoute::Drawer(_) => Some(NOT_YET),
            // LINKS shows its placeholder, as the rail does.
            LeaderRoute::LinksTab(_) => None,
            LeaderRoute::NewNote if self.files.is_some() => None,
            LeaderRoute::NewNote => Some(NO_WORKSPACE),
            LeaderRoute::Quit => None,
            LeaderRoute::NoNoteOpen => Some(NO_NOTE),
            LeaderRoute::Yank { .. } => None,
            LeaderRoute::Text(_) if self.note_has_keyboard(opener, cx) => None,
            LeaderRoute::Text(_) => Some(NEEDS_NOTE_FOCUS),
            LeaderRoute::Save => match self.notes.as_ref().and_then(|h| h.note.as_ref()) {
                Some(_) => None,
                None => Some(NO_NOTE),
            },
            LeaderRoute::Layout(LayoutStep::Zen) => None,
            LeaderRoute::Layout(_) if self.resized_drawer().is_some() => None,
            LeaderRoute::Layout(_) => Some(NO_DRAWER),
            LeaderRoute::Job(LeaderJob::TogglePin(_) | LeaderJob::JumpToPinned(_))
            | LeaderRoute::Job(LeaderJob::Reindex)
                if has_vault =>
            {
                None
            }
            LeaderRoute::Job(LeaderJob::TogglePin(_) | LeaderJob::JumpToPinned(_))
            | LeaderRoute::Job(LeaderJob::Reindex) => Some(NO_WORKSPACE),
            LeaderRoute::Soon(_) => Some(SOON),
            // Leader `p`: the palette is the window's one modal so far.
            LeaderRoute::Overlay(OverlayOpen::CommandPalette) => None,
            LeaderRoute::Overlay(_)
            | LeaderRoute::FindWithQuery(_)
            | LeaderRoute::OpenJournal
            | LeaderRoute::OpenPreferences
            | LeaderRoute::OpenOnboarding
            | LeaderRoute::NoteDialog(..)
            | LeaderRoute::Job(LeaderJob::CheckUpdates)
            | LeaderRoute::GitStatus
            | LeaderRoute::Ask(_) => Some(NOT_YET),
        }
    }

    /// The drawer a grow or shrink step resizes: the focused one, else the
    /// left one when shown, else the right one when shown.
    fn resized_drawer(&self) -> Option<Side> {
        if let Panel::Drawer(side) = self.layout.focused() {
            return Some(side);
        }
        [Side::Left, Side::Right]
            .into_iter()
            .find(|side| self.layout.is_visible(Panel::Drawer(*side)))
    }

    /// Run `route` with the window's surfaces. A blocked route does nothing:
    /// its callers only offer the ones [`Self::route_block`] allows.
    pub(crate) fn perform_route(
        &mut self,
        route: LeaderRoute,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match route {
            LeaderRoute::Drawer(DrawerView::Files) => self.open_file_browser(window, cx),
            LeaderRoute::Drawer(view) if shell_layout::is_on_rail(view) => {
                self.show_view(view, window, cx)
            }
            LeaderRoute::LinksTab(_) => self.show_view(DrawerView::Links, window, cx),
            LeaderRoute::NewNote if self.files.is_some() => {
                // The FILES filter doubles as the create field: typing a
                // name offers `Create:`.
                self.open_file_browser(window, cx);
                self.flash("Type a name — Enter creates", cx);
            }
            LeaderRoute::Quit => self.request_close(window, cx),
            LeaderRoute::Yank { text, label } => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.flash(label, cx);
            }
            LeaderRoute::Text(action) => {
                self.editor.update(cx, |editor, cx| {
                    editor.apply_text_action(action, window, cx)
                });
            }
            LeaderRoute::Save => self.save_note(cx),
            LeaderRoute::Layout(step) => self.layout_step(step, window, cx),
            LeaderRoute::Job(job) => self.run_leader_job(job, cx),
            LeaderRoute::NoNoteOpen => self.flash("No note open", cx),
            LeaderRoute::Overlay(OverlayOpen::CommandPalette) => self.open_palette(window, cx),
            // Blocked here (see `route_block`).
            LeaderRoute::Drawer(_)
            | LeaderRoute::NewNote
            | LeaderRoute::Overlay(_)
            | LeaderRoute::FindWithQuery(_)
            | LeaderRoute::OpenJournal
            | LeaderRoute::OpenPreferences
            | LeaderRoute::OpenOnboarding
            | LeaderRoute::NoteDialog(..)
            | LeaderRoute::GitStatus
            | LeaderRoute::Ask(_)
            | LeaderRoute::Soon(_) => {}
        }
        cx.notify();
    }

    /// Show `view` where it is placed (a view placed nowhere: in the left
    /// drawer's focused slot) and give it the keyboard. Unlike a rail click,
    /// a shown view stays shown: the leader opens, it never toggles.
    /// Persisted.
    fn show_view(&mut self, view: DrawerView, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = RAIL_VIEWS.iter().position(|v| *v == view) {
            self.rail_cursor = i;
        }
        let (side, index) = self.layout.open_view(view);
        self.layout.focus_region(Region::Slot { side, index });
        self.ensure_files_listing(cx);
        self.apply_focus(window, cx);
        self.store.save(&self.layout);
    }

    /// `+window`: zen hides both drawers and gives the note the keyboard;
    /// grow and shrink move the resized drawer's edge by one step.
    /// Persisted.
    fn layout_step(&mut self, step: LayoutStep, window: &mut Window, cx: &mut Context<Self>) {
        match step {
            LayoutStep::Zen => {
                for side in [Side::Left, Side::Right] {
                    self.layout.hide(Panel::Drawer(side));
                }
                self.layout.focus(Panel::Editor);
                self.apply_focus(window, cx);
            }
            LayoutStep::GrowDrawer | LayoutStep::ShrinkDrawer => {
                let Some(side) = self.resized_drawer() else {
                    return;
                };
                let limits = Self::limits(window);
                let delta = f64::from(DRAWER_STEP_REMS * f32::from(window.rem_size()));
                let delta = if step == LayoutStep::GrowDrawer {
                    delta
                } else {
                    -delta
                };
                let width = shell_layout::shown_width(&self.layout, side, limits);
                self.layout.set_drawer_width(side, width + delta, limits);
            }
        }
        self.store.save(&self.layout);
    }

    /// Spawn an asynchronous leaf (`leader_route::jobs`) on the window's
    /// runtime; it reports on the app channel.
    fn run_leader_job(&mut self, job: LeaderJob, cx: &mut Context<Self>) {
        let (Some(tx), Some(vault)) = (
            self.tx.clone(),
            self.notes.as_ref().map(|h| h.session.vault().clone()),
        ) else {
            return;
        };
        match job {
            LeaderJob::TogglePin(path) => {
                tokio::spawn(jobs::toggle_pin(vault, path, tx));
            }
            LeaderJob::JumpToPinned(n) => {
                tokio::spawn(jobs::jump_to_pinned(vault, n, tx));
            }
            LeaderJob::Reindex => {
                self.flash("Reindexing…", cx);
                tokio::spawn(jobs::reindex(vault, tx));
            }
            // Blocked (no update dialog until M10).
            LeaderJob::CheckUpdates => {}
        }
    }
}
