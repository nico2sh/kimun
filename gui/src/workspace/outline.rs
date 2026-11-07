//! The **OUTLINE** drawer view (M7a G1): the open note's headings as an
//! indented list, read from the editor's buffer (not the file), in the
//! shared [`DrawerList`].
//!
//! **State.** The logic [`OutlineModel`] decides when the rows change and
//! maps a listed heading back to a buffer row after edits; this view keeps it
//! and lists its headings when it says they changed.
//!
//! **When it syncs** (TUI parity, decision 14): when it is revealed, when
//! the open note changes while it is shown, after each successful save of
//! the open note while it is shown, and right before a jump. Never per
//! keystroke or per frame, and never while hidden; an unchanged buffer
//! revision skips the parse.
//!
//! **Jumping.** Enter or a click syncs, resolves the heading against
//! the buffer, puts the caret on its row and gives the editor the keyboard;
//! a heading no longer in the note is flashed.

use std::num::NonZeroU64;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::{
    AppContext as _, Context, Div, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Stateful, Styled as _, Window, div,
    rems,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::message::AppTx;
use kimun_notes::logic::nav::{DrawerView, HeadingTarget};
use kimun_notes::logic::views::{Changed, OutlineEntry, OutlineModel};
use kimun_notes::settings::themes::Theme;

use super::Workspace;
use super::drawer_list::{DrawerList, DrawerListEvent, DrawerRow};
use super::files::{ellipsis, row_frame};
use crate::theme;

/// Element ids, for tests.
pub(crate) const OUTLINE_ID: &str = "outline";
pub(crate) const OUTLINE_LIST_ID: &str = "outline-list";

/// A row's height, in rems: one line of small text.
const ROW_REMS: f32 = 1.75;
/// One nesting step's indent, in rems.
const INDENT_REMS: f32 = 0.875;
/// Deeper headings indent no further: the text keeps its room.
const MAX_INDENT_STEPS: usize = 6;

/// What the view says with no note open, and for a note without headings.
pub(crate) const NO_NOTE_TEXT: &str = "No note open";
pub(crate) const NO_HEADINGS_TEXT: &str = "This note has no headings";

/// The element id of the row for `target`: its text and which of the
/// same-named headings it is, so a row keeps its id when headings above it
/// move.
pub(crate) fn heading_row_id(target: &HeadingTarget) -> SharedString {
    format!("outline-row:{}#{}", target.text, target.occurrence).into()
}

impl DrawerRow for OutlineEntry {
    fn element_id(&self) -> SharedString {
        heading_row_id(&self.target)
    }

    fn render_row(&self, selected: bool, t: &Theme) -> Stateful<Div> {
        let text = SharedString::from(self.target.text.clone());
        let steps = self.depth.saturating_sub(1).min(MAX_INDENT_STEPS);
        row_frame(self.element_id(), text.clone(), selected, ROW_REMS, t)
            .text_sm()
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .pl(rems(INDENT_REMS * steps as f32))
                    .child(ellipsis(text).text_color(if self.depth == 1 {
                        theme::foreground(t)
                    } else {
                        theme::secondary(t)
                    })),
            )
    }
}

/// The OUTLINE view. See the module docs.
pub(crate) struct OutlineView {
    pub(crate) list: Entity<DrawerList<OutlineEntry>>,
    model: OutlineModel,
    /// Whether the view was shown at the last refresh: a reveal syncs.
    shown: bool,
}

impl OutlineView {}

impl OutlineView {
    fn new(list: Entity<DrawerList<OutlineEntry>>) -> Self {
        Self {
            list,
            model: OutlineModel::new(),
            shown: false,
        }
    }

    /// The headings as last synced (tests).
    #[cfg(test)]
    pub(crate) fn model(&self) -> &OutlineModel {
        &self.model
    }

    /// Whether the view was shown at the last refresh.
    pub(crate) fn is_shown(&self) -> bool {
        self.shown
    }

    /// Whether the headings were last read from `note`'s buffer at
    /// `revision`: a sync would not parse.
    fn is_current(&self, note: &VaultPath, revision: Option<NonZeroU64>) -> bool {
        revision.is_some() && self.model.revision() == revision && self.model.note() == note
    }

    /// Bring the headings up to `note`'s buffer at `revision`; the rows are
    /// re-listed only when the headings changed.
    pub(crate) fn sync(
        &mut self,
        note: &VaultPath,
        revision: Option<NonZeroU64>,
        text: impl FnOnce() -> String,
        cx: &mut Context<Self>,
    ) {
        if self.model.sync(note, revision, text) == Changed::Rebuilt {
            let rows = self.model.entries().to_vec();
            let empty = if note.is_root_or_empty() {
                NO_NOTE_TEXT
            } else {
                NO_HEADINGS_TEXT
            };
            self.list
                .update(cx, |list, cx| list.set_rows(rows, empty, cx));
        }
    }

    /// The buffer row `target` stands for now (sync first).
    pub(crate) fn resolve(&self, target: &HeadingTarget) -> Option<usize> {
        self.model.resolve(target)
    }
}

impl Focusable for OutlineView {
    fn focus_handle(&self, cx: &gpui_kit::App) -> FocusHandle {
        self.list.read(cx).focus_handle(cx)
    }
}

impl Render for OutlineView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(OUTLINE_ID)
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(self.list.clone())
    }
}

// ---- The workspace's side ----------------------------------------------------

impl Workspace {
    /// Make the OUTLINE view (nothing is read until it is shown).
    pub(super) fn start_outline(&mut self, tx: AppTx, window: &mut Window, cx: &mut Context<Self>) {
        let list = DrawerList::spawn(OUTLINE_LIST_ID, "Outline", tx, cx);
        self._views_events
            .push(cx.subscribe_in(&list, window, Self::on_outline_event));
        self.outline = Some(cx.new(|_| OutlineView::new(list)));
    }

    /// The note OUTLINE reads and its buffer revision: the open note in
    /// the editor, or nothing (an empty path, an unknown revision).
    fn outline_note(&self, cx: &gpui_kit::App) -> (VaultPath, Option<NonZeroU64>) {
        match self.notes.as_ref().and_then(|h| h.note.clone()) {
            Some(note) => (note, Some(self.editor.read(cx).editor.revision())),
            None => (VaultPath::empty(), None),
        }
    }

    /// Bring OUTLINE up to the editor's buffer. The text is copied only
    /// when the headings must be read again (another note, a newer
    /// revision).
    fn sync_outline(&mut self, cx: &mut Context<Self>) {
        let Some(outline) = self.outline.clone() else {
            return;
        };
        let (note, revision) = self.outline_note(cx);
        if outline.read(cx).is_current(&note, revision) {
            return;
        }
        let text = if note.is_root_or_empty() {
            String::new()
        } else {
            self.editor.read(cx).editor.get_text()
        };
        outline.update(cx, |view, cx| view.sync(&note, revision, move || text, cx));
    }

    /// Keep OUTLINE in step with the layout and the open note: a reveal, or
    /// another note while shown, syncs it. Hidden, it does nothing.
    pub(super) fn refresh_outline(&mut self, cx: &mut Context<Self>) {
        let Some(outline) = self.outline.clone() else {
            return;
        };
        let shown = self.is_shown(DrawerView::Outline);
        let (note, _) = self.outline_note(cx);
        let view = outline.read(cx);
        let revealed = shown && !view.shown;
        let other_note = view.model.note() != &note;
        outline.update(cx, |view, _| view.shown = shown);
        if shown && (revealed || other_note) {
            self.sync_outline(cx);
        }
    }

    /// The open note was saved: OUTLINE follows its headings while shown.
    pub(super) fn outline_saved(&mut self, cx: &mut Context<Self>) {
        if self
            .outline
            .as_ref()
            .is_some_and(|outline| outline.read(cx).is_shown())
        {
            self.sync_outline(cx);
        }
    }

    fn on_outline_event(
        &mut self,
        _: &Entity<DrawerList<OutlineEntry>>,
        event: &DrawerListEvent<OutlineEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            DrawerListEvent::Activate(row) => self.jump_to_heading(&row.target, window, cx),
            DrawerListEvent::Verb(..) => {}
            DrawerListEvent::Dismiss => self.focus_panel(Panel::Editor, window, cx),
            // OUTLINE rows have no menu.
            DrawerListEvent::FileOp(..) => {}
        }
    }

    /// Put the caret on the heading `target` stands for and give the editor
    /// the keyboard, after bringing OUTLINE up to the buffer; or say it is
    /// gone.
    pub(crate) fn jump_to_heading(
        &mut self,
        target: &HeadingTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(outline) = self.outline.clone() else {
            return;
        };
        if self.notes.as_ref().and_then(|h| h.note.as_ref()).is_none() {
            self.flash("No note open", cx);
            return;
        }
        self.sync_outline(cx);
        let row = outline.read(cx).resolve(target);
        match row {
            Some(row) => {
                self.editor.update(cx, |view, cx| {
                    view.editor.jump_to_row(row);
                    cx.notify();
                });
                self.focus_panel(Panel::Editor, window, cx);
            }
            None => self.flash(
                format!("heading \"{}\" is no longer in the note", target.text),
                cx,
            ),
        }
    }
}
