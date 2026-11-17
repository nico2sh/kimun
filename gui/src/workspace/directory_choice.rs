//! The **directory choice**: a filter field over the vault's folders, fuzzy
//! ranked, one row highlighted — what the new note and directory dialogs
//! show under their `Change…` button. It owns the loaded folders, the
//! filter, the highlight and the list; its host owns where it sits and which
//! keys reach it (the host's dialog captures Enter, Escape and the arrows
//! and forwards them: [`DirectoryChoice::step`], [`DirectoryChoice::choose`]).
//!
//! The folders come from walking the disk, so they load off the UI thread
//! (the same walk the move dialog does, `file_ops::list_directories`). A
//! walk that fails is said in the list, which then offers nothing.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, EventEmitter, InteractiveElement,
    IntoElement, ParentElement, Render, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle, Window, div,
    rems, uniform_list,
};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::file_ops;
use std::sync::Arc;

use crate::theme;

/// Element ids, for tests.
pub(crate) const CHOICE_FILTER_ID: &str = "directory-choice-filter";
pub(crate) const CHOICE_LIST_ID: &str = "directory-choice-list";

/// A row of the list, by its folder.
pub(crate) fn choice_row_id(dir: &VaultPath) -> SharedString {
    format!("directory-choice-row:{dir}").into()
}

/// What the choice tells its host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DirectoryChoiceEvent {
    /// A folder was chosen (Enter on the highlighted row, a click).
    Chosen(VaultPath),
}

/// A folder as the list shows it.
pub(crate) fn directory_label(dir: &VaultPath) -> SharedString {
    file_ops::directory_name(dir).into()
}

pub(crate) struct DirectoryChoice {
    pub(crate) filter: Entity<InputState>,
    /// Every folder, the root first; `None` while loading.
    dirs: Option<Vec<VaultPath>>,
    /// Why the folders could not be listed.
    load_error: Option<String>,
    /// The folders the filter keeps, best first.
    visible: Vec<VaultPath>,
    selected: Option<usize>,
    /// Highlighted once the folders are there.
    current: VaultPath,
    scroll: UniformListScrollHandle,
    _load: Task<()>,
    _filter_events: Subscription,
}

impl EventEmitter<DirectoryChoiceEvent> for DirectoryChoice {}

impl DirectoryChoice {
    pub(crate) fn new(
        vault: Arc<NoteVault>,
        current: VaultPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter folders"));
        let events = cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refilter(None, cx);
            }
        });
        // The walk blocks: off the UI thread, and off the runtime's own
        // thread too. The outer task wakes this one from the runtime.
        let load = cx.spawn(async move |this, cx| {
            let listed = tokio::spawn(async move {
                tokio::task::spawn_blocking(move || file_ops::list_directories(&vault)).await
            })
            .await;
            let listed = match listed {
                Ok(Ok(Ok(dirs))) => Ok(dirs),
                Ok(Ok(Err(e))) => Err(e),
                Ok(Err(e)) | Err(e) => Err(e.to_string()),
            };
            this.update(cx, |choice, cx| choice.loaded(listed, cx)).ok();
        });
        Self {
            filter,
            dirs: None,
            load_error: None,
            visible: Vec::new(),
            selected: None,
            current,
            scroll: UniformListScrollHandle::new(),
            _load: load,
            _filter_events: events,
        }
    }

    /// Put the keyboard in the filter.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter
            .update(cx, |filter, cx| filter.focus(window, cx));
    }

    #[cfg(test)]
    pub(crate) fn highlighted(&self) -> Option<VaultPath> {
        self.selected.and_then(|ix| self.visible.get(ix).cloned())
    }

    #[cfg(test)]
    pub(crate) fn is_loading(&self) -> bool {
        self.dirs.is_none() && self.load_error.is_none()
    }

    fn loaded(&mut self, listed: Result<Vec<VaultPath>, String>, cx: &mut Context<Self>) {
        match listed {
            Ok(dirs) => self.dirs = Some(dirs),
            Err(reason) => {
                self.dirs = Some(Vec::new());
                self.load_error = Some(reason);
            }
        }
        // The row the user already moved to stays; otherwise the folder the
        // dialog opened on.
        let keep = self
            .highlighted_now()
            .unwrap_or_else(|| self.current.clone());
        self.refilter(Some(keep), cx);
    }

    fn highlighted_now(&self) -> Option<VaultPath> {
        self.selected.and_then(|ix| self.visible.get(ix).cloned())
    }

    /// Rank the folders against the filter; highlight `keep` when it is
    /// still shown, else the best match.
    fn refilter(&mut self, keep: Option<VaultPath>, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).value().to_string();
        self.visible = self
            .dirs
            .as_deref()
            .map(|dirs| file_ops::filter_directories(dirs, &query))
            .unwrap_or_default();
        let at = keep
            .and_then(|k| self.visible.iter().position(|d| d.is_like(&k)))
            .or((!self.visible.is_empty()).then_some(0));
        self.select(at, cx);
    }

    fn select(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        self.selected = ix;
        if let Some(ix) = ix {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    /// The arrows: one row down or up, stopping at the ends.
    pub(crate) fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        let ix = match (self.selected, down) {
            (None, _) => 0,
            (Some(ix), true) => (ix + 1).min(last),
            (Some(ix), false) => ix.saturating_sub(1),
        };
        if Some(ix) != self.selected {
            self.select(Some(ix), cx);
        }
    }

    /// Enter: choose the highlighted folder. `false` when there is none
    /// (still loading, nothing matches, the walk failed): nothing was chosen.
    pub(crate) fn choose(&mut self, cx: &mut Context<Self>) -> bool {
        match self.highlighted_now() {
            Some(dir) => {
                cx.emit(DirectoryChoiceEvent::Chosen(dir));
                true
            }
            None => false,
        }
    }

    fn render_row(&self, ix: usize, dir: &VaultPath, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let selected = self.selected == Some(ix);
        let hover = theme::hover_background(t);
        let chosen = dir.clone();
        div()
            .id(choice_row_id(dir))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(directory_label(dir))
            .aria_selected(selected)
            .flex()
            .items_center()
            .w_full()
            .h(rems(1.75))
            .px_2()
            .rounded_md()
            .cursor_pointer()
            .text_sm()
            .text_color(theme::directory(t))
            .when(selected, |row| row.bg(theme::selected_background(t)))
            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
            .child(div().truncate().child(directory_label(dir)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.select(Some(ix), cx);
                cx.emit(DirectoryChoiceEvent::Chosen(chosen.clone()));
            }))
            .into_any_element()
    }
}

impl Render for DirectoryChoice {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let count = self.visible.len();
        let list = if count > 0 {
            uniform_list(
                CHOICE_LIST_ID,
                count,
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    let rows: Vec<(usize, VaultPath)> = range
                        .filter_map(|ix| this.visible.get(ix).map(|d| (ix, d.clone())))
                        .collect();
                    rows.iter()
                        .map(|(ix, dir)| this.render_row(*ix, dir, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full()
            .into_any_element()
        } else {
            let note = match (&self.load_error, &self.dirs) {
                (Some(reason), _) => format!("Couldn't list the folders: {reason}"),
                (None, None) => "(loading...)".to_string(),
                (None, Some(_)) => "(no matches)".to_string(),
            };
            div()
                .px_2()
                .text_sm()
                .text_color(theme::muted(t))
                .child(note)
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                Input::new(&self.filter)
                    .id(CHOICE_FILTER_ID)
                    .aria_label("Filter folders"),
            )
            .child(
                div()
                    .id("directory-choice-rows")
                    .role(Role::ListBox)
                    .aria_label("Folders")
                    .h(rems(9.))
                    .child(list),
            )
    }
}
