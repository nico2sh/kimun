//! Preferences > Keyboard: **export and import** of the keymap file
//! ([`crate::keymap::file`], ADR-0064).
//!
//! - **Export** writes every shortcut of the draft, defaults too, to the path in its field
//!   (`kimun-keys-<system>.toml` in the home folder at first) and says where, or why
//!   not. `Choose folder…` opens the [`FolderChooser`]; the folder chosen
//!   goes before the field's file name.
//! - **Import** reads the path in its field. `Choose file…` opens the
//!   chooser in files mode (`.toml`). The file replaces every row of the
//!   draft and the model is written, so Save and Cancel follow the view's
//!   draft flow: nothing is kept until Save. The result line counts the
//!   changes, then lists what the loader skipped, one line each. A file that
//!   cannot be read, or is no keymap file, changes nothing.
//! - Both run off the UI thread; every control waits for them
//!   ([`PreferencesView::busy`]).
//! - The chooser shows in place of the two fields, with its commands and
//!   key hints, as in Workspaces. The area is the foot of the page, so the
//!   pane scrolls to the bottom when the chooser opens; an area taller than
//!   the pane shows its top instead, so the Location field and the list
//!   show. The pane scrolls to the bottom again when an Export or Import
//!   result shows (it grows the page below where the pane rests). The view
//!   hands the chooser the keys while its root has the keyboard.

use std::cell::Cell;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Bounds, ClickEvent, Context, Entity, Focusable as _, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Pixels, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, Window, div, point, px,
};
use kimun_core::system::{self, SystemPath};
use kimun_notes::settings::themes::Theme;

use super::super::super::facet::reveal_offset;
use super::super::super::folder_chooser::{self, FolderChooser, FolderChooserEvent};
use super::super::workspaces::{Notice, button_row, notice_line};
use super::super::{PreferencesView, Section, label, note};
use crate::keymap::file::{Imported, read_file, write_file};
use crate::keymap::{self, KeymapError};
use crate::tasks::blocking;
use crate::theme;

/// The id prefix of the section's chooser (`preferences-keyboard-chooser…`).
pub(crate) const CHOOSER_PREFIX: &str = "preferences-keyboard";
/// The area the chooser fills while it is up: its heading, the chooser,
/// its commands and its key hints.
pub(crate) const CHOOSER_AREA_ID: &str = "preferences-keyboard-chooser-area";
/// The Export path field.
pub(crate) const EXPORT_PATH_ID: &str = "preferences-keyboard-export-path";
/// `Choose folder…` beside the Export field.
pub(crate) const EXPORT_CHOOSE_ID: &str = "preferences-keyboard-export-choose";
/// The `Export` button.
pub(crate) const EXPORT_ID: &str = "preferences-keyboard-export";
/// The Export result line.
pub(crate) const EXPORT_NOTICE_ID: &str = "preferences-keyboard-export-notice";
/// The Import path field.
pub(crate) const IMPORT_PATH_ID: &str = "preferences-keyboard-import-path";
/// `Choose file…` beside the Import field.
pub(crate) const IMPORT_CHOOSE_ID: &str = "preferences-keyboard-import-choose";
/// The `Import` button.
pub(crate) const IMPORT_ID: &str = "preferences-keyboard-import";
/// The Import result lines.
pub(crate) const IMPORT_NOTICE_ID: &str = "preferences-keyboard-import-notice";

/// The note under the Keymap file heading.
pub(crate) const FILE_NOTE: &str =
    "Export saves your changed shortcuts to a file. Import loads a file in their place.";

/// The file name Export starts with, in the home folder.
/// The default file name: it names the system, so a file shared between
/// systems says where it came from. The import reads the label inside the
/// file, never this name.
pub(crate) fn file_name() -> String {
    format!("kimun-keys-{}.toml", crate::keymap::file::os_label())
}
/// The extension the files mode of the chooser lists.
const EXTENSION: &str = "toml";

/// Which field the chooser fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Export,
    Import,
}

/// The Import result: the count and what was skipped, or the error.
struct ImportNotice {
    error: bool,
    lines: Vec<SharedString>,
}

/// The export and import area's state.
pub(crate) struct KeyFile {
    export_input: Entity<InputState>,
    import_input: Entity<InputState>,
    chooser: Option<(Target, Entity<FolderChooser>)>,
    _chooser_events: Option<Subscription>,
    /// Set when the chooser opens: the first frame that lays it out
    /// scrolls the pane to it ([`reveal_chooser`]), then clears it.
    reveal: Rc<Cell<bool>>,
    export_notice: Option<Notice>,
    import_notice: Option<ImportNotice>,
    /// The write (Export) or read (Import) under way.
    task: Option<(Target, Task<()>)>,
}

impl KeyFile {
    /// The two fields, Export's on `kimun-keys-<system>.toml` in the home folder.
    pub(crate) fn new(window: &mut Window, cx: &mut Context<PreferencesView>) -> Self {
        let start = system::browse_root()
            .map(|home| system::path_to_string(home.join(file_name())))
            .unwrap_or_default();
        let export_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Path of the file to write")
                .default_value(start)
        });
        let import_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Path of the file to read"));
        Self {
            export_input,
            import_input,
            chooser: None,
            _chooser_events: None,
            reveal: Rc::default(),
            export_notice: None,
            import_notice: None,
            task: None,
        }
    }

    /// The Export and Import result lines go: an edit makes them stale.
    pub(crate) fn clear_notices(&mut self) {
        self.export_notice = None;
        self.import_notice = None;
    }

    /// The chooser goes, if it is up.
    pub(crate) fn close_chooser(&mut self) {
        self.chooser = None;
        self._chooser_events = None;
        self.reveal.set(false);
    }

    fn input(&self, target: Target) -> &Entity<InputState> {
        match target {
            Target::Export => &self.export_input,
            Target::Import => &self.import_input,
        }
    }
}

/// The path typed in `input`: a full path, or one that starts with `~` (the
/// home folder), else the line that says what is wrong with it. Any other
/// relative path is refused: it would depend on where the app started.
fn typed_path(input: &Entity<InputState>, cx: &gpui_kit::App) -> Result<SystemPath, String> {
    let text = input.read(cx).value().trim().to_string();
    if text.is_empty() {
        return Err("Type the path of the file.".into());
    }
    let path = PathBuf::from(text);
    let mut parts = path.components();
    if parts.next() == Some(Component::Normal("~".as_ref())) {
        let rest: PathBuf = parts.collect();
        return system::home()
            .map(|home| home.join(rest))
            .map_err(|_| "Couldn't find the home folder. Type a full path.".to_string());
    }
    SystemPath::try_absolute(path).map_err(|_| "Type a full path, from the top folder.".to_string())
}

/// `n` changes, counted.
fn loaded(n: usize) -> String {
    if n == 1 {
        "Loaded 1 change.".into()
    } else {
        format!("Loaded {n} changes.")
    }
}

/// The line for the lines left out of a file from the other family of
/// systems.
fn other_defaults_line(count: usize, system: &str) -> String {
    let (lines, verb) = if count == 1 {
        ("line", "matches")
    } else {
        ("lines", "match")
    };
    format!("Skipped {count} {lines} that {verb} the {system} defaults.")
}

/// One line for what the loader skipped, with the names as the file has
/// them.
fn skipped(error: &KeymapError) -> String {
    match error {
        KeymapError::UnknownAction { name } => format!("Skipped {name}: no such action."),
        KeymapError::InvalidChord {
            action,
            chord,
            reason,
        } => format!("Skipped {action} = \"{chord}\": {reason}."),
        KeymapError::Collision {
            action,
            chord,
            taken_by,
        } => format!("Skipped {action} = \"{chord}\": {taken_by} has it."),
    }
}

/// Write the keymap file at `path` (blocking). A folder is refused before
/// anything is written.
fn export_to(
    path: &Path,
    overrides: &std::collections::BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    if path.is_dir() {
        return Err(format!(
            "This is a folder. Add a file name, such as {}.",
            file_name()
        ));
    }
    write_file(path, overrides).map_err(|e| format!("Couldn't save the file: {e}"))
}

/// Scroll the pane (`pane`) to the chooser's area, laid out as `children`
/// (its heading first, its hints last), by the least amount that shows it
/// whole. An area taller than the pane shows its top, so the Location
/// field and the list show. The children were placed at the old offset:
/// the next frame draws the new one.
fn reveal_chooser(pane: &ScrollHandle, children: &[Bounds<Pixels>], window: &mut Window) {
    let (Some(first), Some(last)) = (children.first(), children.last()) else {
        return;
    };
    let view = pane.bounds();
    let offset = pane.offset();
    // Down the page from its top, as the offset counts.
    let page = |y: Pixels| y - view.top() - offset.y;
    let area = (page(first.top()), page(last.bottom()));
    if let Some(y) = reveal_offset(area, -offset.y, view.size.height, px(0.)) {
        pane.set_offset(point(offset.x, -y));
        window.refresh();
    }
}

impl PreferencesView {
    // ---- reading -----------------------------------------------------------

    /// The Export (`export`) or Import path field.
    #[cfg(test)]
    pub(crate) fn keyboard_path_input(&self, export: bool) -> Entity<InputState> {
        let target = if export {
            Target::Export
        } else {
            Target::Import
        };
        self.keyboard.file.input(target).clone()
    }

    /// The Keyboard section's chooser, while it is up.
    pub(crate) fn keyboard_chooser(&self) -> Option<Entity<FolderChooser>> {
        self.keyboard.file.chooser.as_ref().map(|(_, c)| c.clone())
    }

    /// Whether an export or an import runs.
    pub(crate) fn keyboard_file_working(&self) -> bool {
        self.keyboard.file.task.is_some()
    }

    /// Whether one of the section's fields has the keyboard (the filter, a
    /// path field, a chooser field).
    pub(crate) fn keyboard_field_focused(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        let file = &self.keyboard.file;
        let focused =
            |input: &Entity<InputState>| input.read(cx).focus_handle(cx).is_focused(window);
        self.keyboard_filter_focused(window, cx)
            || focused(&file.export_input)
            || focused(&file.import_input)
            || file
                .chooser
                .as_ref()
                .is_some_and(|(_, c)| c.read(cx).field_focused(window, cx))
    }

    // ---- acting ------------------------------------------------------------

    /// Enter in one of the section's own fields: Export or Import in
    /// theirs, nothing in the filter (a chooser field's Enter is the
    /// chooser's).
    pub(crate) fn keyboard_field_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let file = &self.keyboard.file;
        let on = |input: &Entity<InputState>| input.read(cx).focus_handle(cx).is_focused(window);
        if on(&file.export_input) {
            self.keyboard_export(window, cx);
        } else if on(&file.import_input) {
            self.keyboard_import(window, cx);
        }
    }

    /// Esc in one of the section's own fields: the filter's text goes, the
    /// chooser closes (from the empty filter), else (a path field, the
    /// empty filter) it cancels the view, as Esc does everywhere in it. A
    /// chooser field's Esc is the chooser's.
    pub(crate) fn keyboard_field_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.keyboard_filter_focused(window, cx) && self.keyboard_filter_escape(window, cx) {
            return;
        }
        if self.keyboard_chooser().is_some() {
            // The empty filter with the chooser up: Esc closes the chooser,
            // never the view.
            self.keyboard_close_chooser(window, cx);
            return;
        }
        self.cancel(false, window, cx);
    }

    /// `Choose folder…` (Export) or `Choose file…` (Import): the chooser,
    /// on the folder of the field's path when there is one.
    fn keyboard_choose(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        self.keyboard.stop_listening();
        let start = typed_path(self.keyboard.file.input(target), cx)
            .ok()
            .and_then(|p| p.parent())
            .filter(SystemPath::is_dir)
            .or_else(|| system::browse_root().ok());
        let Some(start) = start else {
            let notice = Notice::error("Couldn't find a folder to start browsing from.");
            match target {
                Target::Export => self.keyboard.file.export_notice = Some(notice),
                Target::Import => {
                    self.keyboard.file.import_notice = Some(ImportNotice {
                        error: true,
                        lines: vec![notice.text],
                    })
                }
            }
            self.keyboard_file_result_shown();
            cx.notify();
            return;
        };
        let chooser = cx.new(|cx| {
            let chooser = FolderChooser::new(CHOOSER_PREFIX, start, window, cx);
            match target {
                Target::Export => chooser,
                Target::Import => chooser.files(EXTENSION),
            }
        });
        self.keyboard.file._chooser_events =
            Some(cx.subscribe_in(&chooser, window, Self::on_keyboard_chooser_event));
        self.keyboard.file.chooser = Some((target, chooser));
        // The chooser replaces the area at the foot of the page and is much
        // taller: scroll to the bottom so that its commands and hints show.
        // If it is taller than the pane, the frame that lays it out then
        // scrolls back to its top (`reveal_chooser`). Closing it shrinks the
        // page, and the offset clamps there.
        self.content_scroll.scroll_to_bottom();
        self.keyboard.file.reveal.set(true);
        // The chooser's Location field has the keyboard now.
        cx.notify();
    }

    /// An Export or Import result was just set. It grows the foot of the
    /// page, where the pane may rest (after a chooser): scroll to the
    /// bottom once so that the whole result shows.
    fn keyboard_file_result_shown(&mut self) {
        // A slow task can land after the user went to another section: the
        // pane is that section's then.
        if self.section == Section::Keyboard {
            self.content_scroll.scroll_to_bottom();
        }
    }

    /// The chooser goes; the keyboard goes back to the field it was for.
    pub(crate) fn keyboard_close_chooser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((target, _)) = self.keyboard.file.chooser.take() else {
            return;
        };
        self.keyboard.file.close_chooser();
        let handle = self.keyboard.file.input(target).read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    fn on_keyboard_chooser_event(
        &mut self,
        _: &Entity<FolderChooser>,
        event: &FolderChooserEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, _)) = self.keyboard.file.chooser.as_ref() else {
            return;
        };
        let chosen = match (target, event) {
            (Target::Export, FolderChooserEvent::Chosen(dir)) => {
                // The folder goes before the file name the field has.
                let name = typed_path(&self.keyboard.file.export_input, cx)
                    .ok()
                    .and_then(|p| p.as_path().file_name().map(|n| n.to_os_string()))
                    .unwrap_or_else(|| file_name().into());
                Some(dir.join(name))
            }
            (Target::Import, FolderChooserEvent::ChosenFile(path)) => Some(path.clone()),
            _ => None,
        };
        let target = *target;
        if let Some(path) = chosen {
            let text = system::path_to_string(&path);
            self.keyboard
                .file
                .input(target)
                .clone()
                .update(cx, |i, cx| i.set_value(text, window, cx));
        }
        self.keyboard_close_chooser(window, cx);
    }

    /// `Export`: every shortcut of the draft, defaults too, to the file in
    /// the field.
    pub(crate) fn keyboard_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let Some(overrides) = self.keyboard.draft.as_ref().map(|d| d.export_all()) else {
            return;
        };
        let path = match typed_path(&self.keyboard.file.export_input, cx) {
            Ok(path) => path,
            Err(why) => {
                self.keyboard.file.export_notice = Some(Notice::error(why));
                self.keyboard_file_result_shown();
                cx.notify();
                return;
            }
        };
        self.keyboard.file.export_notice = None;
        let task = cx.spawn_in(window, async move |this, cx| {
            let shown = system::path_to_string(&path);
            let written = blocking(move || export_to(path.as_path(), &overrides))
                .await
                .and_then(|r| r);
            this.update(cx, |view, cx| {
                view.keyboard.file.task = None;
                view.keyboard.file.export_notice = Some(match written {
                    Ok(()) => Notice::info(format!("Saved to {shown}.")),
                    Err(e) => Notice::error(e),
                });
                view.keyboard_file_result_shown();
                cx.notify();
            })
            .ok();
        });
        self.keyboard.file.task = Some((Target::Export, task));
        cx.notify();
    }

    /// `Import`: the file in the field replaces the draft (module docs).
    pub(crate) fn keyboard_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() || self.keyboard.draft.is_none() {
            return;
        }
        let path = match typed_path(&self.keyboard.file.import_input, cx) {
            Ok(path) => path,
            Err(why) => {
                self.keyboard.file.import_notice = Some(ImportNotice {
                    error: true,
                    lines: vec![why.into()],
                });
                self.keyboard_file_result_shown();
                cx.notify();
                return;
            }
        };
        self.keyboard.stop_listening();
        self.keyboard.file.import_notice = None;
        let platform = keymap::platform(cx);
        let task = cx.spawn_in(window, async move |this, cx| {
            let read =
                blocking(move || read_file(path.as_path(), platform).map_err(|e| e.to_string()))
                    .await
                    .and_then(|r| r);
            this.update(cx, |view, cx| {
                view.keyboard.file.task = None;
                view.keyboard_imported(read);
                cx.notify();
            })
            .ok();
        });
        self.keyboard.file.task = Some((Target::Import, task));
        cx.notify();
    }

    fn keyboard_imported(&mut self, read: Result<Imported, String>) {
        self.keyboard_file_result_shown();
        let imported = match read {
            Ok(imported) => imported,
            Err(e) => {
                self.keyboard.file.import_notice = Some(ImportNotice {
                    error: true,
                    lines: vec![e.into()],
                });
                return;
            }
        };
        let Some(draft) = self.keyboard.draft.as_mut() else {
            return;
        };
        let taken = draft.replace(&imported.overrides);
        let mut lines: Vec<SharedString> = vec![loaded(draft.changes()).into()];
        if let Some((count, system)) = imported.other_defaults {
            lines.push(other_defaults_line(count, system).into());
        }
        lines.extend(imported.dropped.iter().map(|e| skipped(e).into()));
        self.keyboard.lines.clear();
        self.keyboard.forget_kept();
        self.keyboard.notice = None;
        for (action, chord, holder) in taken {
            self.keyboard_moved(&action, [(holder, chord)]);
        }
        self.keyboard_write();
        self.keyboard.file.import_notice = Some(ImportNotice {
            error: false,
            lines,
        });
    }

    // ---- drawing -----------------------------------------------------------

    /// A path field, its chooser button and its action button, in a row.
    fn render_file_row(
        &self,
        target: Target,
        cx: &mut Context<Self>,
        t: &Theme,
    ) -> impl IntoElement {
        let busy = self.busy();
        let (field_id, choose_id, choose, act_id, act, read) = match target {
            Target::Export => (
                EXPORT_PATH_ID,
                EXPORT_CHOOSE_ID,
                "Choose folder…",
                EXPORT_ID,
                "Export",
                "Export to",
            ),
            Target::Import => (
                IMPORT_PATH_ID,
                IMPORT_CHOOSE_ID,
                "Choose file…",
                IMPORT_ID,
                "Import",
                "Import from",
            ),
        };
        let input = Input::new(self.keyboard.file.input(target))
            .id(field_id)
            .aria_label(read)
            .disabled(busy)
            .bg(theme::background(t))
            .border_color(theme::divider(t))
            .text_color(theme::foreground(t));
        div().flex().flex_col().gap_1().child(label(read, t)).child(
            button_row()
                .child(div().flex_1().min_w(gpui_kit::rems(12.)).child(input))
                .child(
                    Button::new(choose_id)
                        .small()
                        .ghost()
                        .label(choose)
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.keyboard_choose(target, window, cx)
                        })),
                )
                .child(
                    Button::new(act_id)
                        .small()
                        .outline()
                        .label(act)
                        .loading(
                            self.keyboard
                                .file
                                .task
                                .as_ref()
                                .is_some_and(|(t, _)| *t == target),
                        )
                        .disabled(busy)
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| match target {
                                Target::Export => this.keyboard_export(window, cx),
                                Target::Import => this.keyboard_import(window, cx),
                            },
                        )),
                ),
        )
    }

    fn render_import_notice(&self, t: &Theme) -> Option<AnyElement> {
        let notice = self.keyboard.file.import_notice.as_ref()?;
        let all = notice
            .lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        Some(
            div()
                .id(IMPORT_NOTICE_ID)
                .test_support()
                .role(Role::Label)
                .aria_label(SharedString::from(all))
                .flex()
                .flex_col()
                .text_sm()
                .children(notice.lines.iter().enumerate().map(|(ix, line)| {
                    // The count reads as the result; each skip as a caution.
                    let color = if notice.error {
                        theme::error(t)
                    } else if ix == 0 {
                        theme::foreground(t)
                    } else {
                        theme::caution(t)
                    };
                    div().text_color(color).child(line.clone())
                }))
                .into_any_element(),
        )
    }

    /// The export and import area, or the chooser in its place.
    pub(crate) fn render_keyboard_file(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let heading = |text: &'static str| {
            div()
                .pb_1()
                .border_b_1()
                .border_color(theme::divider(t))
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::muted(t))
                .child(text)
        };
        if let Some((target, chooser)) = self.keyboard.file.chooser.clone() {
            let hints = chooser.read(cx).hints(cx);
            let reveal = self.keyboard.file.reveal.clone();
            let pane = self.content_scroll.clone();
            return div()
                .on_children_prepainted(move |children, window, _| {
                    if reveal.replace(false) {
                        reveal_chooser(&pane, &children, window);
                    }
                })
                .id(CHOOSER_AREA_ID)
                .test_support()
                .flex()
                .flex_col()
                .gap_3()
                .child(heading(match target {
                    Target::Export => "Choose the folder for the keymap file",
                    Target::Import => "Choose a keymap file",
                }))
                .child(chooser.clone())
                .child(button_row().children(folder_chooser::actions(&chooser, cx)))
                .child(note(hints, t))
                .into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading("Keymap file"))
            .child(note(FILE_NOTE, t))
            .child(self.render_file_row(Target::Export, cx, t))
            .children(
                self.keyboard
                    .file
                    .export_notice
                    .as_ref()
                    .map(|n| notice_line(EXPORT_NOTICE_ID, n, t)),
            )
            .child(self.render_file_row(Target::Import, cx, t))
            .children(self.render_import_notice(t))
            .into_any_element()
    }
}
