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
    self, LayoutStep, LeaderJob, LeaderRoute, NoteDialog, RouteFacts, jobs,
};
use kimun_notes::logic::nav::{DrawerView, LinksTab};
use kimun_notes::logic::overlay_open::OverlayOpen;
use kimun_notes::logic::query::is_default_query;

use super::Workspace;
use super::file_dialogs::FileOpKind;
use crate::shell_layout;

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
/// Copying a row needs a drawer list (LINKS, OUTLINE, FIND, TAGS) to have
/// the keyboard.
pub(crate) const NEEDS_LIST_FOCUS: &str = "Needs a drawer list focused";
/// Saving a query needs FIND to have the keyboard (the search browser
/// saves its own with the chord).
pub(crate) const NEEDS_FIND_FOCUS: &str = "Needs FIND focused";
/// Resizing a drawer with none shown.
pub(crate) const NO_DRAWER: &str = "No drawer shown";

/// How far one grow or shrink step moves a drawer's edge, in rems (the TUI
/// moves it four columns).
const DRAWER_STEP_REMS: f32 = 2.;

impl Workspace {
    /// The facts a route reads: the open note (the vault root when none is
    /// open) and whether the Thread holds the editor Panel (then no note is
    /// "this note").
    pub(crate) fn route_facts(&self) -> RouteFacts {
        let note = self
            .notes
            .as_ref()
            .and_then(|h| h.note.clone())
            .unwrap_or_else(VaultPath::root);
        RouteFacts::new(note, self.ask_shown())
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
            // SEM only while the server can search (the TUI's words why
            // not); the rail hides its cell then too.
            LeaderRoute::Drawer(DrawerView::Semantic) => self.semantic_block(),
            // ASK (the Sources view beside the Thread) only while the server
            // can answer (the TUI gates every route into it so).
            LeaderRoute::Drawer(DrawerView::Ask) => self.ask_drawer_block(),
            LeaderRoute::Drawer(view) if shell_layout::hosts(*view) => None,
            LeaderRoute::Drawer(_) => Some(NOT_YET),
            // `+ask`: the Thread ([`super::ask`]).
            LeaderRoute::Ask(step) => self.ask_block(*step),
            // LINKS, on that tab.
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
            // Leader `p`: the palette.
            LeaderRoute::Overlay(OverlayOpen::CommandPalette) => None,
            // Leader `f f`, `f g` / `f r`, `f s`, `f p` and `n d`: the
            // finder, the search browser, the saved searches, the pinned
            // notes, today's journal.
            LeaderRoute::Overlay(
                OverlayOpen::FileFinder
                | OverlayOpen::SearchBrowser
                | OverlayOpen::SavedSearches
                | OverlayOpen::PinnedNotes,
            )
            | LeaderRoute::OpenJournal
                if has_vault =>
            {
                None
            }
            LeaderRoute::Overlay(
                OverlayOpen::FileFinder
                | OverlayOpen::SearchBrowser
                | OverlayOpen::SavedSearches
                | OverlayOpen::PinnedNotes,
            )
            | LeaderRoute::OpenJournal => Some(NO_WORKSPACE),
            // Leader `f b`: the open note's backlinks (LINKS' Backlinks tab); any other
            // query opens FIND on it.
            LeaderRoute::FindWithQuery(_) if has_vault => None,
            LeaderRoute::FindWithQuery(_) => Some(NO_WORKSPACE),
            // `+note` rename / move / delete / properties of the open note
            // (the route is `NoNoteOpen` when there is none).
            LeaderRoute::NoteDialog(
                NoteDialog::Rename | NoteDialog::Move | NoteDialog::Delete | NoteDialog::Properties,
                _,
            ) if has_vault => None,
            // `app.onboarding`: the setup assistant, vault or not.
            LeaderRoute::OpenOnboarding => None,
            LeaderRoute::Overlay(_)
            | LeaderRoute::OpenPreferences
            | LeaderRoute::NoteDialog(..)
            | LeaderRoute::Job(LeaderJob::CheckUpdates)
            | LeaderRoute::GitStatus => Some(NOT_YET),
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
            // Gated again: a leaf sent on the channel skips `route_block`.
            LeaderRoute::Drawer(DrawerView::Semantic) => self.open_semantic(window, cx),
            LeaderRoute::Drawer(DrawerView::Ask) => self.open_ask_drawer(window, cx),
            LeaderRoute::Drawer(view) if shell_layout::hosts(view) => {
                self.show_view(view, window, cx)
            }
            LeaderRoute::LinksTab(tab) => self.show_links_tab(tab, window, cx),
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
            LeaderRoute::Overlay(OverlayOpen::FileFinder) => self.open_finder(window, cx),
            LeaderRoute::Overlay(OverlayOpen::SearchBrowser) => self.open_search(None, window, cx),
            LeaderRoute::Overlay(OverlayOpen::SavedSearches) => {
                self.open_saved_searches(window, cx)
            }
            LeaderRoute::Overlay(OverlayOpen::PinnedNotes) => self.open_pinned_notes(window, cx),
            // Leader `f b` is the open note's backlinks, which LINKS owns (its
            // Backlinks tab): show that, not FIND on a query.
            LeaderRoute::FindWithQuery(query) if is_default_query(&query) => {
                self.show_links_tab(LinksTab::Backlinks, window, cx)
            }
            LeaderRoute::FindWithQuery(query) => self.open_find_with_query(query, window, cx),
            LeaderRoute::OpenJournal => self.open_journal(window, cx),
            LeaderRoute::OpenOnboarding => self.open_onboarding(window, cx),
            // Gated again inside: a leaf sent on the channel skips
            // `route_block`.
            LeaderRoute::Ask(step) => self.ask_step(step, window, cx),
            LeaderRoute::NoteDialog(dialog, path) => {
                let kind = match dialog {
                    NoteDialog::Rename => FileOpKind::Rename,
                    NoteDialog::Move => FileOpKind::Move,
                    NoteDialog::Delete => FileOpKind::Delete,
                    NoteDialog::Properties => FileOpKind::Properties,
                };
                self.open_file_dialog(kind, path, window, cx);
            }
            // Blocked here (see `route_block`).
            LeaderRoute::Drawer(_)
            | LeaderRoute::NewNote
            | LeaderRoute::Overlay(_)
            | LeaderRoute::OpenPreferences
            | LeaderRoute::GitStatus
            | LeaderRoute::Soon(_) => {}
        }
        cx.notify();
    }

    /// Show `view` where it is placed (a view placed nowhere: in the left
    /// drawer's focused slot) and give it the keyboard. Unlike a rail click,
    /// a shown view stays shown: the leader opens, it never toggles.
    /// Persisted.
    pub(super) fn show_view(
        &mut self,
        view: DrawerView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.place_rail_cursor(Some(view));
        let (side, index) = self.layout.open_view(view);
        self.layout.focus_region(Region::Slot { side, index });
        self.ensure_files_listing(cx);
        self.apply_focus(window, cx);
        self.save_layout(cx);
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
        self.save_layout(cx);
    }

    /// Spawn an asynchronous leaf (`leader_route::jobs`) on the window's
    /// runtime; it reports on the app channel.
    fn run_leader_job(&mut self, job: LeaderJob, cx: &mut Context<Self>) {
        let (Some(tx), Some(vault)) = (self.tx.clone(), self.vault()) else {
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
