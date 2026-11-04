//! **Saved searches** (M7b G6): named queries kept with the workspace
//! (core's `save_search` store), listed in the Saved Searches and run in
//! FIND, and the save-search dialog that names one.
//!
//! - **Saved Searches** (`OpenSavedSearches` chord, leader `f s`, the
//!   palette): the one picker ([`super::picker`]) over the logic's
//!   [`SavedSearchSource`]: the virtual "Backlinks (current note)" entry on
//!   top, then the saved searches, the first nine numbered. Typing filters
//!   by name and by number (the TUI's ranking: an exact number first), and
//!   the best match is the one Enter runs, so `2` Enter runs search 2.
//!   Enter (or a double-click) runs it in FIND, which is named after it.
//!   Delete (the key or `Delete…`) asks first in a kit `AlertDialog` over
//!   the list; the Backlinks entry is not a saved search and cannot go.
//! - **Save search** (`SaveCurrentQuery` in FIND and in the search browser,
//!   FIND's `Save…`, the palette): the query, and a name — offered from the
//!   saved search the query came from. A line under the name says what
//!   saving does, from the logic's `save_hint`: create a search, update the
//!   one the query came from, or overwrite another (in the caution colour,
//!   and the button says `Overwrite`). Saving is never silent: until the
//!   saved names are known, and with a blank name, the button waits. The
//!   write runs in the background; a failure stays in the dialog (and is
//!   flashed), a success closes it. Saved from FIND under a name that is
//!   not the query itself, FIND is named for it (no longer edited); saved
//!   from the browser, FIND is left alone.
//! - **The browser.** The one modal slot holds one dialog: saving the
//!   browser's query closes the browser, and when the save dialog closes
//!   (saved or not) the browser opens again on that query.
//!
//! **Where every read and write runs.** The list, the delete, the names the
//! dialog checks against and the save are core calls on tokio (the list in
//! the list core's load task, the rest in `tokio::spawn`); the UI thread
//! only applies their results. Nothing here touches a file.

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState};
use gpui_kit::component::{Icon, WindowExt as _};
use gpui_kit::{
    AppContext as _, Context, Div, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Rgba, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div,
};
use kimun_notes::logic::list::{Filter, RowSource};
use kimun_notes::logic::saved_search::{
    SaveHint, SavedSearchSource, SearchItem, effective_save_name, rank_to_indices, save_hint,
};
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::file_dialogs::{OpStatus, error_row, footer};
use super::files::{ellipsis, row_frame};
use super::modal::{self, ModalKind};
use super::picker::{PickerKind, PickerRow};
use super::route::NO_WORKSPACE;
use crate::theme;

/// Element ids, for tests.
pub(crate) const SAVE_SEARCH_ID: &str = "save-search";
pub(crate) const SAVE_SEARCH_INPUT_ID: &str = "save-search-name";
pub(crate) const SAVE_SEARCH_HINT_ID: &str = "save-search-hint";
pub(crate) const BACKLINKS_ROW_ID: &str = "saved-search-backlinks";

/// A saved search's row id, by its name (names are unique, ignoring case).
pub(crate) fn saved_row_id(name: &str) -> SharedString {
    format!("saved-search-row:{name}").into()
}

/// A row's height, in rems: the name over its query (FILES' note rows).
const ROW_REMS: f32 = 2.75;
/// The save dialog's width, in rems.
const DIALOG_REMS: f32 = 30.;

/// The save dialog while it is open: its view, its progress, and the
/// search browser's query to go back to when it closes.
pub(crate) struct SaveSearchOpen {
    /// The view (tests read it).
    #[cfg(test)]
    pub(crate) dialog: WeakEntity<SaveSearchDialog>,
    status: Entity<OpStatus>,
    browser_query: Option<String>,
}

impl PickerRow for SearchItem {
    fn element_id(&self) -> SharedString {
        if self.is_virtual {
            BACKLINKS_ROW_ID.into()
        } else {
            saved_row_id(&self.name)
        }
    }

    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        let name = SharedString::from(self.name.clone());
        // The number lane: 1-9 quick select, the Backlinks entry's glyph.
        let lane = match self.index {
            _ if self.is_virtual => Some(
                Icon::new(IconName::Link)
                    .size_4()
                    .text_color(theme::accent(t))
                    .into_any_element(),
            ),
            Some(n) => Some(
                div()
                    .text_xs()
                    .text_color(theme::muted(t))
                    .child(n.to_string())
                    .into_any_element(),
            ),
            None => None,
        };
        let name_color = if self.is_virtual {
            theme::accent(t)
        } else {
            theme::foreground(t)
        };
        row_frame(self.element_id(), name.clone(), selected, ROW_REMS, t)
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_center()
                    .w_5()
                    .children(lane),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(ellipsis(name).text_color(name_color))
                    .child(
                        ellipsis(self.query.clone())
                            .text_xs()
                            .text_color(theme::secondary(t)),
                    ),
            )
    }

    fn filter() -> Filter<Self> {
        Filter::Rank(Arc::new(rank_to_indices))
    }

    fn deletable_name(&self) -> Option<&str> {
        (!self.is_virtual).then_some(self.name.as_str())
    }

    fn choose(
        &self,
        _: PickerKind,
        _: Option<Vec<String>>,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> bool {
        workspace.apply_saved_search(self.query.clone(), self.name.clone(), window, cx);
        false
    }
}

// ---- The save dialog ------------------------------------------------------------

/// Where the query being saved came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveSource {
    /// FIND: a save under another name than the query names FIND.
    Find,
    /// The search browser: it opens again on the query afterwards, and FIND
    /// is left alone.
    Browser,
}

/// What saving under the typed name would do, as the dialog says it.
struct Verdict {
    /// The line under the name.
    text: SharedString,
    /// Shown in the caution colour (an overwrite).
    caution: bool,
    /// The confirm button's label.
    verb: &'static str,
    /// Whether the confirm runs (a name, and the saved names known).
    ready: bool,
}

/// The save-search dialog: the query, its name, what saving does.
pub(crate) struct SaveSearchDialog {
    pub(crate) input: Entity<InputState>,
    query: String,
    /// The saved search the query came from (FIND's name for it).
    provenance: Option<String>,
    source: SaveSource,
    /// The workspace's saved-search names; `None` until they are read.
    existing: Option<Vec<String>>,
    workspace: WeakEntity<Workspace>,
    status: Entity<OpStatus>,
    theme: Theme,
    _subscriptions: Vec<Subscription>,
}

impl SaveSearchDialog {
    #[allow(clippy::too_many_arguments)]
    fn new(
        query: String,
        provenance: Option<String>,
        source: SaveSource,
        workspace: WeakEntity<Workspace>,
        status: Entity<OpStatus>,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = provenance.clone().unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Name the search")
                .default_value(name)
        });
        let subscriptions = vec![
            // Typing clears a shown error and re-reads the hint.
            cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.status.update(cx, |status, cx| {
                        if status.error.take().is_some() {
                            cx.notify();
                        }
                    });
                    cx.notify();
                }
            }),
            cx.observe(&status, |_, _, cx| cx.notify()),
        ];
        Self {
            input,
            query,
            provenance,
            source,
            existing: None,
            workspace,
            status,
            theme,
            _subscriptions: subscriptions,
        }
    }

    /// The saved names arrived (`Err`: they could not be read, and the
    /// dialog says so; saving then waits).
    fn names_loaded(&mut self, names: Result<Vec<String>, String>, cx: &mut Context<Self>) {
        match names {
            Ok(names) => self.existing = Some(names),
            Err(error) => self.status.update(cx, |status, cx| {
                status.error = Some(format!("Couldn't read the saved searches: {error}").into());
                cx.notify();
            }),
        }
        cx.notify();
    }

    /// What saving would do now, from the logic's `save_hint`.
    fn verdict(&self, cx: &gpui_kit::App) -> Verdict {
        let typed = self.input.read(cx).value();
        if typed.trim().is_empty() {
            return Verdict {
                text: "Name the search to save it".into(),
                caution: false,
                verb: "Save",
                ready: false,
            };
        }
        let hint = save_hint(
            &typed,
            &self.query,
            self.provenance.as_deref(),
            self.existing.as_deref(),
        );
        match hint {
            SaveHint::Update(name) => Verdict {
                text: format!("Updates \"{name}\", the search this query came from").into(),
                caution: false,
                verb: "Update",
                ready: true,
            },
            SaveHint::Overwrite(name) => Verdict {
                text: format!("Overwrites the saved search \"{name}\"").into(),
                caution: true,
                verb: "Overwrite",
                ready: true,
            },
            SaveHint::SaveNew | SaveHint::SaveNewAsQuery(_) => Verdict {
                text: "Creates a new saved search".into(),
                caution: false,
                verb: "Save",
                ready: true,
            },
            SaveHint::Pending => Verdict {
                text: "Checking the saved names…".into(),
                caution: false,
                verb: "Save",
                ready: false,
            },
        }
    }

    /// The confirm's label and whether it runs (tests).
    #[cfg(test)]
    pub(crate) fn confirm(&self, cx: &gpui_kit::App) -> (&'static str, bool) {
        let verdict = self.verdict(cx);
        (verdict.verb, verdict.ready)
    }

    /// The error the dialog shows (tests).
    #[cfg(test)]
    pub(crate) fn error(&self, cx: &gpui_kit::App) -> Option<String> {
        self.status.read(cx).error.as_ref().map(|e| e.to_string())
    }

    /// The hint line (tests).
    #[cfg(test)]
    pub(crate) fn hint_text(&self, cx: &gpui_kit::App) -> String {
        self.verdict(cx).text.to_string()
    }

    /// What a save would write: (name, query, source), when it may run.
    fn submission(&self, cx: &gpui_kit::App) -> Option<(String, String, SaveSource)> {
        if !self.verdict(cx).ready || self.status.read(cx).busy {
            return None;
        }
        let typed = self.input.read(cx).value();
        let name = effective_save_name(&typed, &self.query).to_string();
        Some((name, self.query.clone(), self.source))
    }

    /// Enter or the confirm.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(submission) = self.submission(cx) else {
            return;
        };
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.save_search_confirmed(submission, window, cx)
            })
            .ok();
    }

    fn label(text: &'static str, t: &Theme) -> impl IntoElement {
        div().text_xs().text_color(theme::muted(t)).child(text)
    }
}

impl Render for SaveSearchDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let verdict = self.verdict(cx);
        let status = self.status.read(cx);
        let busy = status.busy;
        let hint_color: Rgba = if verdict.caution {
            theme::caution(t)
        } else {
            theme::muted(t)
        };
        let confirm = cx.entity().downgrade();
        let cancel = self.workspace.clone();
        div()
            .id(SAVE_SEARCH_ID)
            .test_support()
            .flex()
            .flex_col()
            .gap_3()
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                this.submit(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                this.workspace
                    .update(cx, |workspace, cx| {
                        workspace.dismiss_save_search(window, cx)
                    })
                    .ok();
                cx.stop_propagation();
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::label("Query", t))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::foreground(t))
                            .truncate()
                            .child(self.query.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::label("Name", t))
                    .child(
                        Input::new(&self.input)
                            .id(SAVE_SEARCH_INPUT_ID)
                            .aria_label("Name"),
                    )
                    .child(
                        div()
                            .id(SAVE_SEARCH_HINT_ID)
                            .test_support()
                            .role(Role::Status)
                            .aria_label(verdict.text.clone())
                            .text_xs()
                            .text_color(hint_color)
                            .child(verdict.text.clone()),
                    ),
            )
            .children(error_row(status, t))
            .child(footer(
                verdict.verb,
                verdict.ready,
                busy,
                move |_, window, cx| {
                    confirm
                        .update(cx, |dialog, cx| dialog.submit(window, cx))
                        .ok();
                },
                move |_, window, cx| {
                    cancel
                        .update(cx, |workspace, cx| {
                            workspace.dismiss_save_search(window, cx)
                        })
                        .ok();
                },
            ))
    }
}

// ---- The workspace's side ----------------------------------------------------------

impl Workspace {
    /// The `OpenSavedSearches` chord: open the Saved Searches, or close them
    /// (and a delete confirmation over them) when they are the open modal.
    /// Refused while another modal is open.
    pub(crate) fn toggle_saved_searches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.open_modal(window, cx) {
            Some(ModalKind::SavedSearches) => self.close_modal(window, cx),
            None if !modal::dialog_open(window, cx) => self.open_saved_searches(window, cx),
            _ => {}
        }
    }

    /// Open the Saved Searches, the filter focused; the searches load off
    /// the UI thread. Refused while a modal is open; says so without a
    /// workspace.
    pub(crate) fn open_saved_searches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if modal::dialog_open(window, cx) {
            return;
        }
        let Some(vault) = self.notes.as_ref().map(|h| h.session.vault().clone()) else {
            self.flash(NO_WORKSPACE, cx);
            return;
        };
        // Deletes run on their own (their errors are shown), never through
        // the load: the slot stays empty.
        let source: Arc<dyn RowSource<SearchItem>> =
            Arc::new(SavedSearchSource::new(vault, Arc::new(Mutex::new(None))));
        let picker = self.open_picker(
            PickerKind::SavedSearches,
            source,
            Some("Saved searches".to_string()),
            None,
            String::new(),
            window,
            cx,
        );
        self.saved_searches = picker.map(|p| p.downgrade());
    }

    /// A saved search chosen: the Saved Searches close and FIND runs it,
    /// named after it (the TUI's `apply_saved_search`; the Backlinks
    /// entry's FIND still reads "Backlinks").
    pub(crate) fn apply_saved_search(
        &mut self,
        query: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_modal(window, cx);
        self.open_find_named(query, Some(name), window, cx);
    }

    /// Delete in the Saved Searches on the search `name`: ask in an
    /// `AlertDialog` over them.
    pub(crate) fn ask_delete_saved_search(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        let title = format!("Delete \"{name}\"?");
        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let name = name.clone();
            alert
                .confirm()
                .title(title.clone())
                .description("Only the saved search goes; no note is changed.")
                .ok_text("Delete")
                .ok_variant(ButtonVariant::Danger)
                .cancel_text("Cancel")
                .on_ok(move |_, window, cx| {
                    let name = name.clone();
                    workspace
                        .update(cx, |workspace, cx| {
                            workspace.delete_saved_search(name, window, cx)
                        })
                        .ok();
                    true
                })
        });
        cx.notify();
    }

    /// The delete was confirmed: core removes `name` (off the UI thread),
    /// then the list loads again; a failure shows in the list.
    fn delete_saved_search(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(vault) = self.notes.as_ref().map(|h| h.session.vault().clone()) else {
            return;
        };
        let deleting = {
            let name = name.clone();
            tokio::spawn(async move { vault.delete_saved_search(&name).await })
        };
        cx.spawn_in(window, async move |this, cx| {
            let deleted = match deleting.await {
                Ok(result) => result.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            this.update_in(cx, |workspace, _, cx| {
                let Some(picker) = workspace.saved_searches.as_ref().and_then(|p| p.upgrade())
                else {
                    if let Err(error) = &deleted {
                        workspace.flash_error(format!("Couldn't delete \"{name}\": {error}"), cx);
                    }
                    return;
                };
                picker.update(cx, |picker, cx| match deleted {
                    Ok(()) => picker.reload(cx),
                    Err(error) => picker.set_busy(
                        false,
                        Some(format!("Couldn't delete \"{name}\": {error}")),
                        cx,
                    ),
                });
            })
            .ok();
        })
        .detach();
    }

    /// `SaveCurrentQuery` in FIND (and FIND's `Save…`): save FIND's query.
    pub(crate) fn save_find_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(find) = self.find.clone() else {
            self.flash(NO_WORKSPACE, cx);
            return;
        };
        let (query, name) = {
            let view = find.read(cx);
            (view.query(cx), view.saved_name())
        };
        self.open_save_search(query, name, SaveSource::Find, window, cx);
    }

    /// `SaveCurrentQuery` in the search browser: the browser gives way to
    /// the save dialog (one modal at a time) and comes back on its query.
    pub(crate) fn save_browser_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.as_ref().and_then(|p| p.upgrade()) else {
            return;
        };
        if picker.read(cx).is_busy() {
            return;
        }
        let query = picker.read(cx).input.read(cx).value().to_string();
        if query.trim().is_empty() {
            self.flash("No query to save", cx);
            return;
        }
        self.close_modal(window, cx);
        self.open_save_search(query, None, SaveSource::Browser, window, cx);
    }

    /// Open the save dialog for `query`, its name offered from `provenance`
    /// (the saved search it came from). Refused while a modal is open, and
    /// for a blank query.
    fn open_save_search(
        &mut self,
        query: String,
        provenance: Option<String>,
        source: SaveSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) {
            return;
        }
        let Some(vault) = self.notes.as_ref().map(|h| h.session.vault().clone()) else {
            self.flash(NO_WORKSPACE, cx);
            return;
        };
        if query.trim().is_empty() {
            self.flash("No query to save", cx);
            return;
        }
        // A modal takes every key: no sequence survives under it.
        self.cancel_leader(cx);
        let status = cx.new(|_| OpStatus::default());
        let workspace = cx.entity().downgrade();
        let theme = self.theme.clone();
        let browser_query = source == SaveSource::Browser;
        let query_back = query.clone();
        let view = cx.new(|cx| {
            SaveSearchDialog::new(
                query,
                provenance,
                source,
                workspace.clone(),
                status.clone(),
                theme,
                window,
                cx,
            )
        });
        // The names an overwrite is told from, read off the UI thread.
        let names = tokio::spawn(async move {
            vault
                .list_saved_searches()
                .await
                .map(|all| all.into_iter().map(|s| s.name).collect::<Vec<_>>())
                .map_err(|e| e.to_string())
        });
        let loading = view.downgrade();
        cx.spawn(async move |_, cx| {
            let names = names.await.unwrap_or_else(|e| Err(e.to_string()));
            loading
                .update(cx, |dialog, cx| dialog.names_loaded(names, cx))
                .ok();
        })
        .detach();
        let width = window.rem_size() * DIALOG_REMS;
        let first = Cell::new(true);
        self.modal = Some(ModalKind::SaveSearch);
        self.save_search = Some(SaveSearchOpen {
            #[cfg(test)]
            dialog: view.downgrade(),
            status,
            browser_query: browser_query.then(|| query_back.clone()),
        });
        window.open_dialog(cx, move |dialog, window, cx| {
            if first.replace(false) {
                // The offered name selected: typing replaces it.
                let input = view.read(cx).input.clone();
                input.update(cx, |input, cx| {
                    input.focus(window, cx);
                    input.select_all(window, cx);
                });
            }
            let cancel = workspace.clone();
            dialog
                .title("Save search")
                .w(width)
                .close_button(false)
                // Escape, the backdrop: the workspace closes it (and brings
                // the browser back), unless a save runs.
                .on_cancel(move |_, window, cx| {
                    cancel
                        .update(cx, |workspace, cx| {
                            workspace.dismiss_save_search(window, cx)
                        })
                        .ok();
                    false
                })
                .child(view.clone())
        });
        cx.notify();
    }

    /// Save (Enter, the confirm): core writes it in the background; the
    /// dialog waits.
    fn save_search_confirmed(
        &mut self,
        (name, query, source): (String, String, SaveSource),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(vault) = self.notes.as_ref().map(|h| h.session.vault().clone()) else {
            return;
        };
        if self.save_task.is_some() {
            return;
        }
        self.set_save_status(true, None, cx);
        let saving = {
            let (name, query) = (name.clone(), query.clone());
            tokio::spawn(async move { vault.save_search(&name, &query).await })
        };
        self.save_task = Some(cx.spawn_in(window, async move |this, cx| {
            let saved = match saving.await {
                Ok(result) => result.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            this.update_in(cx, |workspace, window, cx| {
                workspace.search_saved(name, query, source, saved, window, cx)
            })
            .ok();
        }));
    }

    /// The save ended. Saved: FIND is named for it when the query was
    /// FIND's and the name is not the query itself (the TUI's `Persisted`
    /// rule); the dialog closes; the browser comes back. Failed: said in the
    /// dialog, which stays, and flashed.
    fn search_saved(
        &mut self,
        name: String,
        query: String,
        source: SaveSource,
        saved: Result<(), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_task = None;
        match saved {
            Ok(()) => {
                if source == SaveSource::Find
                    && name.trim() != query.trim()
                    && let Some(find) = self.find.clone()
                {
                    find.update(cx, |view, cx| view.repin(name.clone(), &query, cx));
                }
                let open = self.open_modal(window, cx) == Some(ModalKind::SaveSearch);
                let back = self
                    .save_search
                    .as_mut()
                    .and_then(|s| s.browser_query.take());
                if open {
                    self.close_modal(window, cx);
                }
                self.flash(format!("Saved \"{name}\""), cx);
                if let (true, Some(query)) = (open, back) {
                    self.open_search(Some(query), window, cx);
                }
            }
            Err(error) => {
                self.set_save_status(false, Some(error), cx);
                self.flash_error(format!("Couldn't save the search \"{name}\""), cx);
            }
        }
    }

    /// Cancel, Escape or the backdrop in the save dialog: close it (unless
    /// a save runs); a query from the browser goes back to the browser.
    pub(crate) fn dismiss_save_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.save_task.is_some() {
            return;
        }
        let back = self
            .save_search
            .as_mut()
            .and_then(|s| s.browser_query.take());
        self.close_modal(window, cx);
        if let Some(query) = back {
            self.open_search(Some(query), window, cx);
        }
    }

    fn set_save_status(&self, busy: bool, error: Option<String>, cx: &mut Context<Self>) {
        if let Some(open) = self.save_search.as_ref() {
            open.status.update(cx, |status, cx| {
                status.busy = busy;
                status.error = error.map(Into::into);
                cx.notify();
            });
        }
    }
}
