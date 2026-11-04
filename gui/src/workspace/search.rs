//! The **search browser** (M7b G4): the `SearchNotes` chord (Ctrl+K /
//! Cmd+K), leader `f g` / `f r`, the palette, and a followed `#tag` open the
//! one note picker ([`super::picker`]) over the vault's query search, with a
//! preview of the selected note beside the results (below them in a narrow
//! window).
//!
//! **Rows.** The logic's search source (`resolving_search_source_with_create`):
//! an empty query lists the recent notes, anything else runs core's query
//! search, `{note}` and the bare `<` / `>` resolving against the open note; a
//! typed query also leads with the finder's `Create:` row. Loads run off the
//! UI thread and the newest query wins (the list core's generation guard).
//! The selection starts on the first match, not on `Create:` (Enter opens
//! the best result; Up reaches `Create:`).
//!
//! **Preview** ([`SearchPreview`]). The selected note's text, the query's
//! needles in the match colour. A selection held for [`PREVIEW_DEBOUNCE`]
//! is read on tokio, capped at [`PREVIEW_BYTES`], split into lines and
//! matched there too; the UI thread only draws the lines on screen. A newer
//! selection cancels the wait or the read and its result is never shown;
//! until the new one lands the old preview stays, dimmed. Nothing is shown
//! for `Create:` or no selection. The preview is a cached view: typing or
//! moving the selection does not rebuild it until its own state changes.
//!
//! **Opening.** Enter (or a double-click) on a note opens it with the
//! query's needles emphasised in the editor (cleared by the first edit).
//! With unsaved edits in the open note the browser stays up while they are
//! written; a write that fails keeps the browser open and says why (the
//! note stays, edits kept). `Create:` makes and opens the note as the
//! finder does.
//!
//! **Query syntax.** `F1` or the `?` button shows the reference card
//! (`logic::query::SYNTAX`) in a kit dialog over the browser: the one
//! stacked modal. Escape closes it and the browser has the keyboard again.
//!
//! The browser only reads; every write (the flush, `Create:`) is core's.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, FontWeight, HighlightStyle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledText, Task, UniformListScrollHandle,
    Window, div, rems, uniform_list,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::query::SYNTAX;
use kimun_notes::logic::session::FlushResult;
use kimun_notes::logic::text_match::match_ranges;
use kimun_notes::settings::themes::Theme;
use tokio::task::JoinError;

use super::Workspace;
use super::attachment::display_line;
use super::files::ellipsis;
use super::modal::{self, ModalKind};
use super::note::{Closing, flush_problem};
use super::route::NO_WORKSPACE;
use crate::theme;

/// Element ids, for tests.
pub(crate) const PREVIEW_ID: &str = "search-preview";
pub(crate) const PREVIEW_TRUNCATED_ID: &str = "search-preview-truncated";
pub(crate) const QUERY_HELP_ID: &str = "query-help";

/// How long a selection must hold before its note is read for the preview:
/// arrowing through the results reads nothing until it stops.
pub(crate) const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(120);
/// The most of a note the preview reads into lines; the rest is cut and
/// said so (a huge note would cost memory and time for lines nobody scrolls
/// to).
pub(crate) const PREVIEW_BYTES: usize = 256 * 1024;
/// One preview line's height, in rems.
const LINE_REMS: f32 = 1.375;

// ---- What a preview load delivers --------------------------------------------

/// One line as the preview draws it, with where the needles matched in it.
struct PreviewLine {
    text: SharedString,
    hits: Vec<Range<usize>>,
}

/// A note read for the preview: its lines, made off the UI thread.
pub(crate) struct PreviewText {
    path: VaultPath,
    name: SharedString,
    lines: Vec<PreviewLine>,
    /// How many needle matches the lines hold.
    matches: usize,
    /// The first line with a match, scrolled to when the preview lands.
    first_hit: Option<usize>,
    /// The note was longer than [`PREVIEW_BYTES`].
    truncated: bool,
}

#[cfg(test)]
impl PreviewText {
    /// Each line's text and its marked pieces.
    pub(crate) fn lines_and_hits(&self) -> Vec<(String, Vec<String>)> {
        self.lines
            .iter()
            .map(|l| {
                let hits = l.hits.iter().map(|r| l.text[r.clone()].to_string());
                (l.text.to_string(), hits.collect())
            })
            .collect()
    }

    /// The match count, the first matching line, whether it was cut.
    pub(crate) fn summary(&self) -> (usize, Option<usize>, bool) {
        (self.matches, self.first_hit, self.truncated)
    }
}

/// `text` cut to at most `max` bytes on a character boundary, and whether
/// it was cut.
pub(crate) fn cap_text(mut text: String, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    (text, true)
}

/// Lay out `text` (the note at `path`) for the preview: capped, split into
/// display lines, each matched against `needles`. Pure; runs on tokio.
pub(crate) fn preview_text(path: VaultPath, text: String, needles: &[String]) -> PreviewText {
    let (text, truncated) = cap_text(text, PREVIEW_BYTES);
    let mut matches = 0;
    let mut first_hit = None;
    let lines = text
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let text = display_line(line);
            let hits: Vec<Range<usize>> = if needles.is_empty() {
                Vec::new()
            } else {
                match_ranges(&text, needles)
                    .into_iter()
                    .map(|(start, end)| start..end)
                    .collect()
            };
            if !hits.is_empty() {
                matches += hits.len();
                first_hit.get_or_insert(i);
            }
            PreviewLine { text, hits }
        })
        .collect();
    PreviewText {
        name: path.get_parent_path().1.into(),
        path,
        lines,
        matches,
        first_hit,
        truncated,
    }
}

/// Read and lay out the note at `path` for the preview (on tokio).
async fn load_preview(
    vault: Arc<NoteVault>,
    path: VaultPath,
    needles: Arc<[String]>,
) -> Result<PreviewText, String> {
    let text = vault
        .get_note_text(&path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(preview_text(path, text, &needles))
}

/// Aborts a tokio task when dropped: a superseded preview read stops with
/// the GPUI task that awaits it.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// ---- The preview -------------------------------------------------------------

/// What the preview should show: a note and the needles to mark in it.
type Wanted = (VaultPath, Arc<[String]>);

/// The search browser's preview column. See the module docs.
pub(crate) struct SearchPreview {
    vault: Arc<NoteVault>,
    theme: Theme,
    /// The selection's note and needles, `None` for no note.
    wanted: Option<Wanted>,
    /// What is on screen (possibly an older selection's, while `pending`).
    shown: Option<PreviewText>,
    /// Why the wanted note could not be read.
    error: Option<SharedString>,
    /// The debounce, then the read, for `wanted`; replaced by a newer one.
    load: Option<Task<()>>,
    scroll: UniformListScrollHandle,
    /// Reads started (tests: the debounce reads nothing while arrowing).
    #[cfg(test)]
    pub(crate) reads: usize,
}

impl SearchPreview {
    pub(crate) fn new(vault: Arc<NoteVault>, theme: Theme) -> Self {
        Self {
            vault,
            theme,
            wanted: None,
            shown: None,
            error: None,
            load: None,
            scroll: UniformListScrollHandle::new(),
            #[cfg(test)]
            reads: 0,
        }
    }

    /// The path of the note shown, and whether it is the wanted one (tests).
    #[cfg(test)]
    pub(crate) fn shown(&self) -> Option<(VaultPath, bool)> {
        self.shown
            .as_ref()
            .map(|s| (s.path.clone(), !self.is_pending()))
    }

    /// The shown lines' text and their highlighted pieces (tests).
    #[cfg(test)]
    pub(crate) fn shown_lines(&self) -> Vec<(String, Vec<String>)> {
        self.shown
            .as_ref()
            .map(PreviewText::lines_and_hits)
            .unwrap_or_default()
    }

    /// Whether the note on screen is not (yet) the wanted one (tests).
    #[cfg(test)]
    pub(crate) fn is_pending(&self) -> bool {
        match (&self.wanted, &self.shown) {
            (None, _) => false,
            (Some((path, _)), Some(shown)) => self.load.is_some() || &shown.path != path,
            (Some(_), None) => self.error.is_none(),
        }
    }

    /// Show `path` (the selected note; `None`: nothing) with `needles`
    /// marked. The same note and needles again change nothing; anything else
    /// waits [`PREVIEW_DEBOUNCE`], then reads off the UI thread, replacing
    /// any wait or read still under way.
    pub(crate) fn show(
        &mut self,
        path: Option<VaultPath>,
        needles: Arc<[String]>,
        cx: &mut Context<Self>,
    ) {
        let wanted = path.map(|p| (p, needles));
        if wanted == self.wanted {
            return;
        }
        self.wanted = wanted.clone();
        self.error = None;
        let Some((path, needles)) = wanted else {
            self.load = None;
            self.shown = None;
            cx.notify();
            return;
        };
        let vault = self.vault.clone();
        self.load = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PREVIEW_DEBOUNCE).await;
            #[cfg(test)]
            this.update(cx, |preview, _| preview.reads += 1).ok();
            let mut read = AbortOnDrop(tokio::spawn(load_preview(
                vault,
                path.clone(),
                needles.clone(),
            )));
            let loaded = (&mut read.0).await;
            this.update(cx, |preview, cx| {
                preview.loaded((path, needles), loaded, cx)
            })
            .ok();
        }));
        cx.notify();
    }

    /// A read finished: show it if it is still the wanted note's.
    fn loaded(
        &mut self,
        wanted: Wanted,
        loaded: Result<Result<PreviewText, String>, JoinError>,
        cx: &mut Context<Self>,
    ) {
        if self.wanted.as_ref() != Some(&wanted) {
            return;
        }
        self.load = None;
        match loaded.map_err(|e| e.to_string()).and_then(|r| r) {
            Ok(text) => {
                let to = text.first_hit.unwrap_or(0);
                self.scroll.scroll_to_item(to, ScrollStrategy::Center);
                self.shown = Some(text);
            }
            Err(e) => {
                self.shown = None;
                self.error = Some(format!("Couldn't read {}: {e}", wanted.0).into());
            }
        }
        cx.notify();
    }

    fn render_header(&self) -> AnyElement {
        let t = &self.theme;
        let (name, count): (SharedString, Option<SharedString>) = match &self.shown {
            Some(shown) => {
                let count = (!self.wanted.as_ref().is_some_and(|w| w.1.is_empty())).then(|| {
                    match shown.matches {
                        1 => "1 match".into(),
                        n => format!("{n} matches").into(),
                    }
                });
                (shown.name.clone(), count)
            }
            None => ("Preview".into(), None),
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_3()
            .py_1p5()
            .text_sm()
            .border_b_1()
            .border_color(theme::divider(t))
            .child(
                ellipsis(name)
                    .flex_1()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::bright(t)),
            )
            .when_some(count, |row, count| {
                row.child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(theme::muted(t))
                        .child(count),
                )
            })
            .into_any_element()
    }
}

impl Render for SearchPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let (family, size) = {
            let kit = cx.theme();
            (kit.mono_font_family.clone(), kit.mono_font_size)
        };
        let body = match (&self.shown, &self.error) {
            (_, Some(error)) => div()
                .px_3()
                .py_2()
                .text_sm()
                .text_color(theme::error(t))
                .child(error.clone())
                .into_any_element(),
            (Some(shown), None) => {
                let mark = HighlightStyle {
                    color: Some(theme::search_match(t).into()),
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                };
                uniform_list(
                    "search-preview-lines",
                    shown.lines.len(),
                    cx.processor(move |this, range: Range<usize>, _, _| {
                        let Some(shown) = this.shown.as_ref() else {
                            return Vec::new();
                        };
                        range
                            .filter_map(|i| shown.lines.get(i))
                            .map(|line| {
                                let text = StyledText::new(line.text.clone()).with_highlights(
                                    line.hits.iter().map(|hit| (hit.clone(), mark)),
                                );
                                div()
                                    .h(rems(LINE_REMS))
                                    .flex()
                                    .items_center()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(text)
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .size_full()
                .into_any_element()
            }
            (None, None) => div().into_any_element(),
        };
        // Another note's text while the selected one is read: dimmed. The
        // same note being re-marked for a new query stays as it is.
        let stale = match (&self.wanted, &self.shown) {
            (Some((path, _)), Some(shown)) => &shown.path != path,
            _ => false,
        };
        div()
            .id(PREVIEW_ID)
            .test_support()
            .role(Role::Document)
            .aria_label("Preview")
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.render_header())
            .when(self.shown.as_ref().is_some_and(|s| s.truncated), |pane| {
                pane.child(
                    div()
                        .id(PREVIEW_TRUNCATED_ID)
                        .test_support()
                        .role(Role::Note)
                        .flex_none()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme::muted(t))
                        .child("Preview cut short. Open the note to see all of it."),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .py_1()
                    .font_family(family)
                    .text_size(size)
                    .text_color(theme::foreground(t))
                    .when(stale, |body| body.opacity(0.5))
                    .child(body),
            )
    }
}

// ---- The query-syntax reference ----------------------------------------------

/// The reference card's body: each section of [`SYNTAX`], its keys beside
/// what they do.
fn query_help_body(t: &Theme) -> AnyElement {
    div()
        .id(QUERY_HELP_ID)
        .test_support()
        .role(Role::Document)
        .aria_label("Query syntax")
        .flex()
        .flex_col()
        .gap_3()
        .text_sm()
        .children(SYNTAX.iter().map(|(heading, rows)| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::muted(t))
                        .child(*heading),
                )
                .children(rows.iter().map(|(keys, meaning)| {
                    div()
                        .flex()
                        .gap_3()
                        .child(
                            div()
                                .w(rems(9.))
                                .flex_none()
                                .text_color(theme::key_cap(t))
                                .child(*keys),
                        )
                        .child(div().text_color(theme::foreground(t)).child(*meaning))
                }))
        }))
        .into_any_element()
}

// ---- The workspace's side ----------------------------------------------------

/// The query-syntax dialog's width, in rems.
const HELP_REMS: f32 = 30.;

impl Workspace {
    /// The `SearchNotes` chord: open the search browser, or close it when it
    /// is the open modal (its help too). Refused under another modal.
    pub(crate) fn toggle_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.open_modal(window, cx) {
            Some(ModalKind::Search) => {
                if !self.picker_busy(cx) {
                    self.close_modal(window, cx);
                }
            }
            None if !modal::dialog_open(window, cx) => self.open_search(None, window, cx),
            _ => {}
        }
    }

    /// Open the search browser on `query` (empty: the recent notes).
    /// Refused while a modal is open; says so without a workspace.
    pub(crate) fn open_search(
        &mut self,
        query: Option<String>,
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
        let open = self.notes.as_ref().and_then(|h| h.note.clone());
        let recents = self
            .settings
            .as_ref()
            .map(|s| {
                s.read()
                    .unwrap_or_else(|e| e.into_inner())
                    .current_last_paths()
            })
            .unwrap_or_default();
        let source = Arc::new(
            kimun_notes::logic::pickers::resolving_search_source_with_create(
                vault.clone(),
                recents,
                open.clone(),
                self.open_note_dir(),
            ),
        );
        let theme = self.theme.clone();
        let preview = cx.new(|_| SearchPreview::new(vault, theme));
        self.open_search_picker(source, query.unwrap_or_default(), open, preview, window, cx);
    }

    /// A followed label (`#tag`): the search browser on that label.
    pub(crate) fn search_label(&mut self, name: &str, cx: &mut Context<Self>) {
        let query = format!("#{}", kimun_core::quote_query_term(name));
        self.with_window(cx, move |workspace, window, cx| {
            workspace.open_search(Some(query), window, cx)
        });
    }

    /// `F1` or the browser's `?` button: the query syntax over the browser.
    pub(crate) fn open_query_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_modal(window, cx) != Some(ModalKind::Search) || self.query_help {
            return;
        }
        self.query_help = true;
        let workspace = cx.entity().downgrade();
        let theme = self.theme.clone();
        let width = window.rem_size() * HELP_REMS;
        window.open_dialog(cx, move |dialog, _, _| {
            let workspace = workspace.clone();
            dialog
                .title("Query syntax")
                .w(width)
                .child(query_help_body(&theme))
                .on_close(move |_, _, cx| {
                    workspace
                        .update(cx, |workspace, _| workspace.query_help = false)
                        .ok();
                })
        });
        cx.notify();
    }

    /// Enter on a note in the search browser: open it with `emphasis`. With
    /// unsaved edits in the open note, they are written first while the
    /// browser waits (true: the browser is busy until then); a failed write
    /// keeps the browser open with the reason.
    pub(crate) fn search_open(
        &mut self,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let quiet = self.search_flush.is_some()
            || self.note_held()
            || self.closing != Closing::Open
            || self
                .notes
                .as_ref()
                .is_none_or(|h| h.is_switching() || h.is_open_wanted());
        if !path.is_note() || quiet || !self.can_save() || !self.editor_dirty(cx) {
            // Nothing to write first (or an open already under way, which
            // writes it): the open note's switch does the rest.
            self.close_modal(window, cx);
            self.focus_panel(Panel::Editor, window, cx);
            if path.is_note() {
                self.open_note_with(path, emphasis, cx);
            } else {
                self.files_show_dir(path, cx);
            }
            return false;
        }
        let Some(host) = self.notes.as_mut() else {
            return false;
        };
        // No tick from here: the flush is the only write.
        host.session.stop_autosave();
        let flush = match host
            .session
            .prepare_flush(Some(&self.editor.read(cx).editor))
        {
            Ok(flush) => flush,
            Err(_) => {
                self.restart_tick();
                self.close_modal(window, cx);
                self.focus_panel(Panel::Editor, window, cx);
                self.open_note_with(path, emphasis, cx);
                return false;
            }
        };
        self.block_input(true, cx);
        self.search_flush = Some(cx.spawn_in(window, async move |this, cx| {
            let flushed = tokio::spawn(flush.run()).await;
            this.update_in(cx, |workspace, window, cx| {
                workspace.search_flushed(path, emphasis, flushed, window, cx)
            })
            .ok();
        }));
        true
    }

    /// The write before a search open ended: open the note, or keep the
    /// browser with the reason.
    fn search_flushed(
        &mut self,
        path: VaultPath,
        emphasis: Option<Vec<String>>,
        flushed: Result<FlushResult, JoinError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_flush = None;
        let outcome = self.apply_flush(flushed, cx);
        self.restart_tick();
        self.block_input(false, cx);
        let picker = self.picker.as_ref().and_then(|p| p.upgrade());
        match flush_problem(&outcome) {
            Some(problem) => {
                let note = self
                    .notes
                    .as_ref()
                    .and_then(|h| h.note.clone())
                    .unwrap_or_else(VaultPath::root);
                let reason = format!("Couldn't save {note}: {problem}. Nothing was opened.");
                match picker {
                    Some(picker) => {
                        picker.update(cx, |picker, cx| picker.set_busy(false, Some(reason), cx))
                    }
                    None => self.flash_error(reason, cx),
                }
            }
            None => {
                if self.open_modal(window, cx) == Some(ModalKind::Search) {
                    self.close_modal(window, cx);
                }
                self.focus_panel(Panel::Editor, window, cx);
                self.open_note_with(path, emphasis, cx);
            }
        }
        self.file_op_ended(cx);
    }
}
