//! The **Attachment view** (M7a G3): what the editor Panel shows in place of
//! the editor while an **Attachment** is opened (CONTEXT.md). Read-only: a
//! header with the file's name, vault path, size, last change and type, an
//! `Open externally` button (and the `FollowLink` chord while the view has
//! the keyboard) that hands the file to the system's default program, and
//! below it a scrollable preview of a text file, or a line saying there is
//! none (binary files, images included: showing an image is a later
//! feature).
//!
//! **Reading.** Core reads the file (`NoteVault::get_attachment_details`,
//! capped: a large text file comes back as a prefix, flagged truncated) on
//! tokio; its lines are indexed there too, so the UI thread only ever
//! touches the lines on screen ([`Lines`], a virtualised list). A newer
//! open replaces a load still under way.
//!
//! **The open note** (the G3 file-op pipeline's rules). Showing an
//! attachment leaves the open note: it is flushed first (the tick stopped,
//! the editor taking no input), and a flush that fails refuses the
//! attachment — the note stays, with its edits, its tick and the keyboard.
//! Then there is **no open note**: `NoteHost.note` is `None`, the tick is
//! stopped and the buffer emptied, so no save or tick can write anything
//! while the attachment shows, and every note-scoped command says
//! `No note open`. The FILES marker moves to the attachment's row. Opening
//! a note (any door) takes the view down and brings the editor back.
//!
//! The view never writes: renaming, moving or deleting the attachment is a
//! FILES row's file operation, which the view follows (or leaves, deleted).

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::component::button::Button;
use gpui_kit::component::empty::{
    Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, UniformListScrollHandle, Window,
    div, point, px, rems, uniform_list,
};
use kimun_core::nfs::VaultPath;
use kimun_core::{AttachmentContent, NoteVault};
use kimun_notes::logic::attachment::{format_mtime, human_size, type_label};
use kimun_notes::logic::file_ops::relocated;
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::message::FileOp;

use super::Workspace;
use super::files::ellipsis;
use super::note::{Closing, LeavingFor, NoteLeft};
use crate::input;
use crate::startup::EmptyState;
use crate::tasks;
use crate::theme;

/// Element ids, for tests.
pub(crate) const ATTACHMENT_ID: &str = "attachment-view";
pub(crate) const OPEN_EXTERNALLY_ID: &str = "attachment-open-externally";
pub(crate) const PREVIEW_ID: &str = "attachment-preview";
pub(crate) const TRUNCATED_ID: &str = "attachment-truncated";
pub(crate) const NO_PREVIEW_ID: &str = "attachment-no-preview";

/// One preview line's height, in rems.
const LINE_REMS: f32 = 1.375;
/// The most characters of one line the preview lays out: a minified file
/// can be one multi-megabyte line, and shaping it all would stall a frame.
pub(crate) const LINE_CHARS: usize = 2000;
/// A tab, as the preview draws it.
const TAB: &str = "    ";

// ---- The preview's lines -----------------------------------------------------

/// A text preview split into lines once, off the UI thread: the text and
/// where each line starts. A line is read only when it is on screen.
pub(crate) struct Lines {
    text: Arc<str>,
    /// Byte offset of each line's start. The text is capped by core well
    /// under 4 GiB, so `u32` holds every offset at half the memory.
    starts: Vec<u32>,
}

impl Lines {
    /// Index `text`'s lines. A final newline ends the last line rather than
    /// starting an empty one; an empty text is one empty line.
    pub(crate) fn of(text: String) -> Self {
        let mut starts = vec![0u32];
        let len = text.len();
        for (at, byte) in text.bytes().enumerate() {
            if byte == b'\n' && at + 1 < len {
                match u32::try_from(at + 1) {
                    Ok(start) => starts.push(start),
                    Err(_) => break,
                }
            }
        }
        Self {
            text: text.into(),
            starts,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.starts.len()
    }

    /// Line `i` without its line break (`\n` or `\r\n`); empty past the end.
    pub(crate) fn line(&self, i: usize) -> &str {
        let Some(&start) = self.starts.get(i) else {
            return "";
        };
        let end = self
            .starts
            .get(i + 1)
            .map_or(self.text.len(), |next| *next as usize - 1);
        let line = &self.text[start as usize..end];
        let line = line.strip_suffix('\n').unwrap_or(line);
        line.strip_suffix('\r').unwrap_or(line)
    }
}

/// `line` with its tabs widened to spaces.
pub(crate) fn expand_tabs(line: &str) -> String {
    line.replace('\t', TAB)
}

/// `line` as the preview draws it: tabs widened to spaces, cut after
/// [`LINE_CHARS`] characters with an ellipsis.
pub(crate) fn display_line(line: &str) -> SharedString {
    let (shown, cut) = match line.char_indices().nth(LINE_CHARS) {
        Some((at, _)) => (&line[..at], true),
        None => (line, false),
    };
    let mut out = expand_tabs(shown);
    if cut {
        out.push('\u{2026}');
    }
    out.into()
}

// ---- What a load delivers ----------------------------------------------------

/// The preview part of a loaded attachment.
pub(crate) enum Preview {
    /// A text file's lines; `truncated` when core read only a prefix.
    Text { lines: Lines, truncated: bool },
    /// Binary (images included): metadata only.
    None,
}

/// An attachment read for the view, its labels made once.
pub(crate) struct Loaded {
    pub(crate) path: VaultPath,
    name: SharedString,
    location: SharedString,
    pub(crate) size: SharedString,
    pub(crate) modified: SharedString,
    pub(crate) kind: SharedString,
    pub(crate) preview: Preview,
}

impl Loaded {
    /// The file's name, as the header and the flashes say it.
    pub(crate) fn name(&self) -> &SharedString {
        &self.name
    }
}

/// Read the attachment at `path` for the view (run on tokio).
pub(crate) async fn load(vault: &NoteVault, path: &VaultPath) -> Result<Loaded, String> {
    let details = vault
        .get_attachment_details(path)
        .await
        .map_err(|e| e.to_string())?;
    let preview = match details.content {
        AttachmentContent::Text { text, truncated } => Preview::Text {
            lines: Lines::of(text),
            truncated,
        },
        AttachmentContent::Binary => Preview::None,
    };
    Ok(Loaded {
        name: details.path.get_parent_path().1.into(),
        location: details.path.to_string().into(),
        size: human_size(details.size).into(),
        modified: format_mtime(details.modified_secs).into(),
        kind: type_label(details.extension.as_deref()).into(),
        path: details.path,
        preview,
    })
}

// ---- The view ------------------------------------------------------------------

/// What the view asks of the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachmentEvent {
    /// Hand the file to the system's default program.
    OpenExternally,
}

/// The attachment view. See the module docs.
pub(crate) struct AttachmentView {
    pub(crate) loaded: Loaded,
    focus: FocusHandle,
    pub(crate) scroll: UniformListScrollHandle,
    /// The `FollowLink` chord, for the hint beside the button.
    open_chord: Option<KeyInput>,
}

impl EventEmitter<AttachmentEvent> for AttachmentView {}

impl AttachmentView {}

impl AttachmentView {
    pub(crate) fn new(
        loaded: Loaded,
        open_chord: Option<KeyInput>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            loaded,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            open_chord,
        }
    }

    pub(crate) fn path(&self) -> &VaultPath {
        &self.loaded.path
    }

    /// The keymap changed: the hint follows.
    pub(crate) fn set_open_chord(&mut self, chord: Option<KeyInput>, cx: &mut Context<Self>) {
        if self.open_chord != chord {
            self.open_chord = chord;
            cx.notify();
        }
    }

    /// The preview's scroll keys: a line, a page (less two lines), the top
    /// and the bottom. False for any other key (it goes on to the window).
    fn scroll_key(&self, key: KeyInput, window: &Window) -> bool {
        if key.mods != Mods::NONE || !matches!(self.loaded.preview, Preview::Text { .. }) {
            return false;
        }
        let state = self.scroll.0.borrow();
        let handle = &state.base_handle;
        let line = f32::from(window.rem_size()) * LINE_REMS;
        let page = (f32::from(handle.bounds().size.height) - 2.0 * line).max(line);
        let max = f32::from(handle.max_offset().y);
        let y = -f32::from(handle.offset().y);
        let to = match key.key {
            Key::Up => y - line,
            Key::Down => y + line,
            Key::PageUp => y - page,
            Key::PageDown => y + page,
            Key::Home => 0.0,
            Key::End => max,
            _ => return false,
        };
        let x = handle.offset().x;
        handle.set_offset(point(x, px(-to.clamp(0.0, max.max(0.0)))));
        true
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let muted = theme::muted(t);
        let fg = theme::foreground(t);
        let fact = |label: &'static str, value: SharedString| {
            div()
                .flex()
                .flex_none()
                .gap_1()
                .whitespace_nowrap()
                .child(div().text_color(muted).child(label))
                .child(div().text_color(fg).child(value))
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_1()
            .px_4()
            .pt_4()
            .pb_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        Icon::new(IconName::Paperclip)
                            .size_5()
                            .flex_none()
                            .text_color(theme::secondary(t)),
                    )
                    .child(
                        ellipsis(self.loaded.name.clone())
                            .text_lg()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(theme::bright(t)),
                    ),
            )
            .child(
                ellipsis(self.loaded.location.clone())
                    .text_sm()
                    .text_color(muted),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x_4()
                    .gap_y_1()
                    .pt_1()
                    .text_sm()
                    .child(fact("Size", self.loaded.size.clone()))
                    .child(fact("Modified", self.loaded.modified.clone()))
                    .child(fact("Type", self.loaded.kind.clone())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new(OPEN_EXTERNALLY_ID)
                            .icon(IconName::ExternalLink)
                            .label("Open externally")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(AttachmentEvent::OpenExternally)
                            })),
                    )
                    .when_some(self.open_chord, |row, chord| {
                        row.child(Kbd::new(input::keystroke(chord)))
                    }),
            )
    }

    fn render_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        match &self.loaded.preview {
            Preview::Text { lines, truncated } => {
                let (family, size) = {
                    let kit = cx.theme();
                    (kit.mono_font_family.clone(), kit.mono_font_size)
                };
                let fg = theme::foreground(t);
                let list = uniform_list(
                    "attachment-preview-lines",
                    lines.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, _| {
                        let Preview::Text { lines, .. } = &this.loaded.preview else {
                            return Vec::new();
                        };
                        range
                            .map(|i| {
                                div()
                                    .h(rems(LINE_REMS))
                                    .flex()
                                    .items_center()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(display_line(lines.line(i)))
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .size_full();
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .when(*truncated, |body| {
                        body.child(
                            div()
                                .id(TRUNCATED_ID)
                                .test_support()
                                .role(Role::Note)
                                .flex_none()
                                .px_4()
                                .py_1()
                                .text_sm()
                                .text_color(theme::muted(t))
                                .child("Preview truncated. Open externally to see the whole file."),
                        )
                    })
                    .child(
                        div()
                            .id(PREVIEW_ID)
                            .test_support()
                            .role(Role::Document)
                            .aria_label("Preview")
                            .flex_1()
                            .min_h_0()
                            .px_4()
                            .py_2()
                            .font_family(family)
                            .text_size(size)
                            .text_color(fg)
                            .child(list),
                    )
                    .into_any_element()
            }
            Preview::None => div()
                .id(NO_PREVIEW_ID)
                .test_support()
                .flex()
                .flex_1()
                .min_h_0()
                .child(
                    Empty::new()
                        .border_0()
                        .text_color(theme::foreground(t))
                        .header(
                            EmptyHeader::new()
                                .media(
                                    EmptyMedia::new()
                                        .with_variant(EmptyMediaVariant::Icon)
                                        .bg(theme::panel_background(t))
                                        .child(
                                            Icon::new(IconName::File).text_color(theme::muted(t)),
                                        ),
                                )
                                .title(EmptyTitle::new().child("No preview"))
                                .description(
                                    EmptyDescription::new()
                                        .text_color(theme::muted(t))
                                        .child("Open it externally to view it."),
                                ),
                        ),
                )
                .into_any_element(),
        }
    }
}

impl Focusable for AttachmentView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AttachmentView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(ATTACHMENT_ID)
            .test_support()
            .track_focus(&self.focus)
            .role(Role::Group)
            .aria_label(self.loaded.name.clone())
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if let Some(key) = input::key_down_input(event)
                    && this.scroll_key(key, window)
                {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .child(self.render_header(cx))
            .child(
                div()
                    .flex_none()
                    .h(px(1.))
                    .w_full()
                    .bg(theme::divider(&theme::current(cx))),
            )
            .child(self.render_preview(cx))
    }
}

// ---- The workspace's side ----------------------------------------------------

impl Workspace {
    /// The attachment the editor Panel shows, if any.
    pub(crate) fn shown_attachment(&self, cx: &App) -> Option<VaultPath> {
        self.active_state()
            .attachment
            .as_ref()
            .map(|view| view.read(cx).path().clone())
    }

    /// Show the attachment at `path` in the editor Panel (see the module
    /// docs): read it, then leave the open note (flushed first) for it. A
    /// newer request replaces a read still under way; one made while the
    /// open note is being flushed for an attachment waits for that.
    pub(crate) fn open_attachment(&mut self, path: &VaultPath, cx: &mut Context<Self>) {
        self.open_attachment_in(self.layout.active_group(), path, cx);
    }

    /// [`Self::open_attachment`] in `group`, active or not.
    fn open_attachment_in(&mut self, group: GroupId, path: &VaultPath, cx: &mut Context<Self>) {
        if self.closing != Closing::Open {
            return;
        }
        let Some(vault) = self.vault() else {
            return;
        };
        let Some(state) = self.state_mut(group) else {
            return;
        };
        if state.attachment_flushing.is_some() {
            state.attachment_wanted = Some(path.clone());
            return;
        }
        let read = {
            let path = path.clone();
            tokio::spawn(async move { load(&vault, &path).await })
        };
        let path = path.clone();
        // Leaving the note for it needs the window (a flush runs in it).
        let window = self.window;
        // The read lands in the group that asked for it.
        let task = cx.spawn(async move |this, cx| {
            let loaded = tasks::joined(read).await;
            let Some(window) = window else {
                return;
            };
            window
                .update(cx, |_, window, cx| {
                    this.update(cx, |workspace, cx| {
                        workspace.attachment_loaded(group, path, loaded, window, cx)
                    })
                })
                .ok();
        });
        if let Some(state) = self.state_mut(group) {
            state.attachment_loading = Some(task);
        }
    }

    /// A note was asked for: an attachment still being read gives way.
    pub(super) fn forget_attachment_request(&mut self, group: GroupId) {
        if let Some(state) = self.state_mut(group) {
            state.attachment_loading = None;
            state.attachment_wanted = None;
        }
    }

    /// The attachment was read: show it in the group that asked for it,
    /// leaving that group's note for it first ([`Workspace::leave_note`]:
    /// flushed, or refused and the note stays); or say why not. Dropped
    /// when the group is gone.
    fn attachment_loaded(
        &mut self,
        group: GroupId,
        path: VaultPath,
        loaded: Result<Loaded, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.state_mut(group) else {
            return;
        };
        state.attachment_loading = None;
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(e) => {
                self.flash_error(format!("Couldn't open {path}: {e}"), cx);
                return;
            }
        };
        let name = loaded.name().clone();
        self.leave_note(
            group,
            LeavingFor::Attachment,
            name,
            move |workspace, left, _, cx| match left {
                NoteLeft::Now => workspace.show_attachment(group, loaded, cx),
                NoteLeft::Flushed => {
                    workspace.show_attachment(group, loaded, cx);
                    // One asked for while the flush ran goes now.
                    if let Some(next) = workspace
                        .state_mut(group)
                        .and_then(|s| s.attachment_wanted.take())
                    {
                        workspace.open_attachment_in(group, &next, cx);
                    }
                }
                NoteLeft::Stays => {
                    if let Some(state) = workspace.state_mut(group) {
                        state.attachment_wanted = None;
                    }
                }
            },
            window,
            cx,
        );
    }

    /// Put `loaded` in `group`'s editor Panel, its note (already flushed)
    /// let go ([`Workspace::release_note_in`]), so nothing can save while
    /// the attachment shows. The Footer, FILES and the keyboard follow when
    /// `group` is the active one.
    fn show_attachment(&mut self, group: GroupId, loaded: Loaded, cx: &mut Context<Self>) {
        if self.state(group).is_none() {
            return;
        }
        self.release_note_in(group, cx);
        let path = loaded.path.clone();
        let chord = self.chord_labels.follow_link;
        let view = cx.new(|cx| AttachmentView::new(loaded, chord, cx));
        let events = cx.subscribe(&view, |this, _, event, cx| match event {
            AttachmentEvent::OpenExternally => this.open_attachment_externally(cx),
        });
        if let Some(state) = self.state_mut(group) {
            state._attachment_events = Some(events);
            state.attachment = Some(view);
        }
        if group != self.layout.active_group() {
            return;
        }
        self.status.attachment = Some(path.clone());
        if let Some(files) = self.files.as_ref() {
            files.update(cx, |pane, cx| pane.set_open_note(Some(path), cx));
        }
        // LINKS and OUTLINE say "No note open".
        self.refresh_views(cx);
        self.focus_shown_content(cx);
        cx.notify();
    }

    /// The editor Panel's content changed under the keyboard: when it has
    /// the keyboard, give it to what shows now.
    pub(super) fn focus_shown_content(&mut self, cx: &mut Context<Self>) {
        if self.focused_panel() == Panel::Editor {
            self.with_window(cx, |workspace, window, cx| {
                workspace.apply_focus(window, cx)
            });
        }
    }

    /// Let the attachment view go: the view, its event subscription and the
    /// status line's name for it, together (a field added to the view's
    /// lifetime belongs here). True when one was shown. The caller
    /// refocuses and redraws.
    pub(super) fn drop_attachment(&mut self) -> bool {
        self.active_state_mut()._attachment_events = None;
        self.status.attachment = None;
        self.active_state_mut().attachment.take().is_some()
    }

    /// A note opened: the attachment view goes (the editor is back).
    pub(super) fn clear_attachment(&mut self, cx: &mut Context<Self>) {
        if self.drop_attachment() {
            self.focus_shown_content(cx);
            cx.notify();
        }
    }

    /// `Open externally` (the button, the `FollowLink` chord): hand the
    /// shown attachment to the system's default program.
    pub(crate) fn open_attachment_externally(&mut self, cx: &mut Context<Self>) {
        let (Some(view), Some(host)) = (
            self.active_state().attachment.as_ref(),
            self.active_state().notes.as_ref(),
        ) else {
            return;
        };
        let (path, name) = {
            let view = view.read(cx);
            (view.path().clone(), view.loaded.name().clone())
        };
        let os_path = host.session.vault().path_to_pathbuf(&path);
        match (self.opener)(os_path.as_os_str()) {
            Ok(()) => self.flash(format!("Opening {name}…"), cx),
            Err(e) => self.flash_error(format!("Couldn't open {name}: {e}"), cx),
        }
    }

    /// A file operation landed: the view follows its attachment renamed or
    /// moved (read again from there), and goes when it was deleted.
    pub(super) fn attachment_file_op(&mut self, op: &FileOp, cx: &mut Context<Self>) {
        let Some(shown) = self.shown_attachment(cx) else {
            return;
        };
        match op {
            FileOp::Renamed { from, to } | FileOp::Moved { from, to } => {
                if let Some(new) = relocated(&shown, from, to) {
                    self.open_attachment(&new, cx);
                }
            }
            FileOp::Deleted(gone) if relocated(&shown, gone, gone).is_some() => {
                let name = shown.get_parent_path().1;
                self.drop_attachment();
                self.note_gone = Some(EmptyState {
                    title: format!("{name} was deleted"),
                    detail: String::new(),
                    failed: false,
                });
                if let Some(files) = self.files.as_ref() {
                    files.update(cx, |pane, cx| pane.set_open_note(None, cx));
                }
                self.focus_shown_content(cx);
                cx.notify();
            }
            _ => {}
        }
    }
}
