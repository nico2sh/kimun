//! Preferences > **Keyboard**: one chord per action, edited in the draft
//! (ADR-0064). Unlike Workspaces and Remote images it acts on nothing until
//! Save: every change goes into the model's `gui_key_overrides`, so Cancel
//! and Save follow the view's draft flow.
//!
//! - **Rows.** Every bindable action ([`bindable_groups`]) under its group's
//!   heading: its label, its chords in the [`KeyDraft`] (or "none"), and
//!   where it is live when that is not everywhere. `Change` and `Clear`
//!   always (`Clear` off while the row has no chord), `Reset` once the row
//!   differs from its default.
//! - **Change** listens on the row; the listening lives here, not in the
//!   draft. The next key is checked ([`check_capture`]): a refused key says
//!   why and the row keeps listening, Esc ends it (the refusal goes, an
//!   earlier warning stays). A taken key is assigned, and it moves from
//!   every holder. Both rows then say so.
//! - **Clear** leaves the row with no chord, saved as `Action = []`.
//! - **Focus.** After a capture, Esc, Clear or Reset the keyboard goes back
//!   to the row's `Change`: one handle, given to the `Change` of the row
//!   edited last.
//! - **Notes.** A chord the plain editor edits with, a chord vim reads
//!   first, and a chord a query action keeps beside it (no steal), get a
//!   quiet line on the row.
//! - **Reset** gives the row its defaults back (and takes them from a
//!   holder, with the same warning). **Reset all** forgets every change.
//!
//! - **Export and import** of the keymap file: [`file`](mod@file).
//!
//! The lines under a row stay until that row changes again, Reset all, an
//! import, or the view closes. The `Reset all` notice and the Export
//! and Import result lines go at the next edit.

use std::collections::BTreeMap;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{
    AnyElement, ClickEvent, Context, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Role, SharedString, StatefulInteractiveElement, Styled, Window, div,
};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::input::{Key, KeyInput};
use kimun_notes::settings::themes::Theme;

use super::workspaces::{Notice, button_row, notice_line};
use super::{PreferencesView, note};
use crate::keymap::draft::{Group, KeyDraft, bindable_groups, check_capture, takes_editing_key};
use crate::keymap::{self, format_chord};
use crate::theme;

pub(crate) mod file;

/// The section's list of rows.
pub(crate) const KEYBOARD_ID: &str = "preferences-keyboard";
/// The `Reset all` button.
pub(crate) const RESET_ALL_ID: &str = "preferences-keyboard-reset-all";
/// The section's result line, under `Reset all`.
pub(crate) const NOTICE_ID: &str = "preferences-keyboard-notice";

/// What a row with no chord reads.
pub(crate) const NONE: &str = "none";
/// The note on a row whose chord the vim rule takes in the editor.
pub(crate) const VIM_NOTE: &str = "In the editor with vim mode, vim reads this key first.";
/// The note on a row whose chord the plain editor edits with
/// ([`takes_editing_key`]).
pub(crate) const EDITOR_NOTE: &str =
    "In the editor, this key then runs this shortcut and no longer edits text.";
/// What `Reset all` says it did.
pub(crate) const RESET_ALL_DONE: &str = "Every shortcut is back to its default.";

/// The element id of the heading of group `ix` of [`bindable_groups`].
pub(crate) fn group_id(ix: usize) -> SharedString {
    format!("preferences-keyboard-group-{ix}").into()
}

/// The element id of the row of `action`.
pub(crate) fn row_id(action: &ActionShortcuts) -> SharedString {
    format!("preferences-keyboard-row-{action}").into()
}

/// The element id of the `Change` button of `action`.
pub(crate) fn change_id(action: &ActionShortcuts) -> SharedString {
    format!("preferences-keyboard-change-{action}").into()
}

/// The element id of the `Clear` button of `action`.
pub(crate) fn clear_id(action: &ActionShortcuts) -> SharedString {
    format!("preferences-keyboard-clear-{action}").into()
}

/// The element id of the `Reset` button of `action`.
pub(crate) fn reset_id(action: &ActionShortcuts) -> SharedString {
    format!("preferences-keyboard-reset-{action}").into()
}

/// The element id of the lines under the row of `action`: a moved chord, a
/// refused key, the notes.
pub(crate) fn warning_id(action: &ActionShortcuts) -> SharedString {
    format!("preferences-keyboard-warning-{action}").into()
}

/// The warning on both rows when `chord` moves away from `holder`.
fn moved(chord: KeyInput, holder: &str) -> String {
    format!(
        "{} was {holder}. Saving clears it there.",
        format_chord(chord)
    )
}

/// What one line under a row is, for its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// A chord moved between two rows.
    Moved,
    /// The key pressed cannot be a shortcut.
    Refused,
    /// A chord that works, with a limit.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    kind: LineKind,
    text: SharedString,
}

/// The section's state beside the draft.
pub(crate) struct KeyboardSection {
    /// Where the keyboard rests while a row listens.
    focus: FocusHandle,
    /// The `Change` button of [`Self::last_row`] tracks it, so the keyboard
    /// can go back there after an edit.
    return_focus: FocusHandle,
    /// The row edited last.
    last_row: Option<ActionShortcuts>,
    groups: Vec<Group>,
    /// The draft of the model's `gui_key_overrides`, built when the model
    /// is read.
    draft: Option<KeyDraft>,
    /// The row that takes the next key.
    listening: Option<ActionShortcuts>,
    lines: BTreeMap<ActionShortcuts, Vec<Line>>,
    notice: Option<Notice>,
    /// Export and import.
    file: file::KeyFile,
}

impl KeyboardSection {
    pub(crate) fn new(focus: FocusHandle, return_focus: FocusHandle, file: file::KeyFile) -> Self {
        Self {
            focus,
            return_focus,
            last_row: None,
            groups: bindable_groups(),
            draft: None,
            listening: None,
            lines: BTreeMap::new(),
            notice: None,
            file,
        }
    }

    /// The row label of `action`, or its on-disk name for an action the
    /// section does not list.
    fn label_of(&self, action: &ActionShortcuts) -> String {
        self.groups
            .iter()
            .flat_map(|g| &g.rows)
            .find(|r| r.action == *action)
            .map(|r| r.label.to_string())
            .unwrap_or_else(|| action.to_string())
    }

    /// Drop the refusal under the row of `action`, keeping its other lines.
    fn drop_refusal(&mut self, action: &ActionShortcuts) {
        if let Some(lines) = self.lines.get_mut(action) {
            lines.retain(|l| l.kind != LineKind::Refused);
        }
    }

    fn push(&mut self, action: &ActionShortcuts, kind: LineKind, text: impl Into<SharedString>) {
        self.lines.entry(action.clone()).or_default().push(Line {
            kind,
            text: text.into(),
        });
    }

    /// A row stops listening; the keyboard stays where it is.
    pub(crate) fn stop_listening(&mut self) {
        self.listening = None;
    }

    /// The section is left: no row listens, the chooser goes.
    pub(crate) fn leave(&mut self) {
        self.listening = None;
        self.file.close_chooser();
    }

    /// Whether a row takes the next key.
    pub(crate) fn is_listening(&self) -> bool {
        self.listening.is_some()
    }
}

impl PreferencesView {
    // ---- reading (the tests) ----------------------------------------------

    /// The row that takes the next key.
    #[cfg(test)]
    pub(crate) fn keyboard_listening(&self) -> Option<&ActionShortcuts> {
        self.keyboard.listening.as_ref()
    }

    // ---- acting ------------------------------------------------------------

    /// The draft of the model just read, for the keymap's platform.
    pub(crate) fn keyboard_loaded(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.model.as_ref() else {
            return;
        };
        self.keyboard.draft = Some(KeyDraft::new(
            keymap::platform(cx),
            &model.gui_key_overrides,
        ));
    }

    /// `Change` on the row of `action`: it takes the next key.
    pub(crate) fn keyboard_change(
        &mut self,
        action: ActionShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        self.keyboard.last_row = Some(action.clone());
        self.keyboard.listening = Some(action);
        window.focus(&self.keyboard.focus, cx);
        cx.notify();
    }

    /// The key the listening row takes ([module docs](self)). True while a
    /// row listens: every key is the row's then.
    pub(crate) fn keyboard_capture(
        &mut self,
        key: KeyInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(action) = self.keyboard.listening.clone() else {
            return false;
        };
        // The keyboard went elsewhere (a click on a path field): the key is
        // not the row's, and the row stops listening.
        if !self.keyboard.focus.is_focused(window) {
            self.keyboard.listening = None;
            return false;
        }
        self.keyboard.drop_refusal(&action);
        if key.key == Key::Esc {
            self.keyboard.listening = None;
            self.keyboard_focus_row(action, window, cx);
            cx.notify();
            return true;
        }
        if let Err(refused) = check_capture(key) {
            self.keyboard
                .push(&action, LineKind::Refused, refused.to_string());
            cx.notify();
            return true;
        }
        self.keyboard.listening = None;
        self.keyboard.lines.remove(&action);
        self.keyboard.notice = None;
        self.keyboard.file.clear_notices();
        self.keyboard_focus_row(action.clone(), window, cx);
        let Some(draft) = self.keyboard.draft.as_mut() else {
            return true;
        };
        let holders = draft.assign(&action, key);
        let shared = draft.shared(&action, key);
        let vim = draft.vim_shadows(&action, key);
        self.keyboard_write();
        self.keyboard_moved(&action, holders.iter().map(|h| (h.clone(), key)));
        for s in shared {
            let other = self.keyboard.label_of(&s.other);
            let chord = format_chord(key);
            let text = match s.other_wins_in {
                Some(place) => format!("{place}, {chord} is {other}."),
                None => format!("Everywhere else, {chord} is {other}."),
            };
            self.keyboard.push(&action, LineKind::Note, text);
        }
        if takes_editing_key(&action, key) {
            self.keyboard.push(&action, LineKind::Note, EDITOR_NOTE);
        }
        if vim {
            self.keyboard.push(&action, LineKind::Note, VIM_NOTE);
        }
        cx.notify();
        true
    }

    /// `Reset` on the row of `action`: its defaults back, taken from any
    /// holder.
    pub(crate) fn keyboard_reset(
        &mut self,
        action: ActionShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        self.keyboard.listening = None;
        self.keyboard.notice = None;
        self.keyboard.file.clear_notices();
        self.keyboard_focus_row(action.clone(), window, cx);
        let Some(draft) = self.keyboard.draft.as_mut() else {
            return;
        };
        let before = draft.clone();
        let holders = draft.reset(&action);
        let restored = draft.chords(&action);
        // The chord each holder gave back: one it had that the row has now.
        let moves: Vec<(ActionShortcuts, KeyInput)> = holders
            .into_iter()
            .filter_map(|holder| {
                let had = before.chords(&holder);
                let chord = restored.iter().find(|c| had.contains(c)).copied()?;
                Some((holder, chord))
            })
            .collect();
        self.keyboard.lines.remove(&action);
        self.keyboard_write();
        self.keyboard_moved(&action, moves);
        cx.notify();
    }

    /// `Clear` on the row of `action`: it has no chord.
    pub(crate) fn keyboard_clear(
        &mut self,
        action: ActionShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() {
            return;
        }
        self.keyboard.listening = None;
        self.keyboard.notice = None;
        self.keyboard.file.clear_notices();
        self.keyboard_focus_row(action.clone(), window, cx);
        let Some(draft) = self.keyboard.draft.as_mut() else {
            return;
        };
        draft.clear(&action);
        self.keyboard.lines.remove(&action);
        self.keyboard_write();
        cx.notify();
    }

    /// The keyboard back on the `Change` of the row of `action`.
    fn keyboard_focus_row(
        &mut self,
        action: ActionShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.keyboard.last_row = Some(action);
        window.focus(&self.keyboard.return_focus, cx);
    }

    /// `Reset all`: every change goes.
    pub(crate) fn keyboard_reset_all(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        self.keyboard.listening = None;
        let Some(draft) = self.keyboard.draft.as_mut() else {
            return;
        };
        draft.reset_all();
        self.keyboard.lines.clear();
        self.keyboard.notice = Some(Notice::info(RESET_ALL_DONE));
        self.keyboard.file.clear_notices();
        self.keyboard_write();
        cx.notify();
    }

    /// The draft into the model, which works out whether it is dirty.
    fn keyboard_write(&mut self) {
        let Some(overrides) = self.keyboard.draft.as_ref().map(KeyDraft::overrides) else {
            return;
        };
        if let Some(model) = self.model_mut() {
            model.gui_key_overrides = overrides;
        }
    }

    /// `action` took each chord from its holder: both rows say so. Earlier
    /// lines of the action stay cleared; a holder's are replaced.
    fn keyboard_moved(
        &mut self,
        action: &ActionShortcuts,
        moves: impl IntoIterator<Item = (ActionShortcuts, KeyInput)>,
    ) {
        for (holder, chord) in moves {
            let text = moved(chord, &self.keyboard.label_of(&holder));
            self.keyboard.lines.remove(&holder);
            self.keyboard.push(&holder, LineKind::Moved, text.clone());
            self.keyboard.push(action, LineKind::Moved, text);
        }
    }

    // ---- drawing -----------------------------------------------------------

    fn render_keyboard_lines(&self, action: &ActionShortcuts, t: &Theme) -> Option<AnyElement> {
        let lines = self.keyboard.lines.get(action).filter(|l| !l.is_empty())?;
        let all = lines
            .iter()
            .map(|l| l.text.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        Some(
            div()
                .id(warning_id(action))
                .test_support()
                .role(Role::Label)
                .aria_label(SharedString::from(all))
                .flex()
                .flex_col()
                .children(lines.iter().map(|l| {
                    div()
                        .text_xs()
                        .text_color(match l.kind {
                            LineKind::Moved => theme::caution(t),
                            LineKind::Refused => theme::error(t),
                            LineKind::Note => theme::muted(t),
                        })
                        .child(l.text.clone())
                }))
                .into_any_element(),
        )
    }

    fn render_keyboard_row(
        &self,
        draft: &KeyDraft,
        action: &ActionShortcuts,
        label: &'static str,
        scope_tag: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let chords = draft.chords(action);
        let unbound = chords.is_empty();
        let chords = if unbound {
            NONE.to_string()
        } else {
            chords
                .into_iter()
                .map(format_chord)
                .collect::<Vec<_>>()
                .join(" / ")
        };
        let mut read = format!("{label}: {chords}");
        if let Some(tag) = scope_tag {
            read.push_str(&format!(" ({tag})"));
        }
        let listening = self.keyboard.listening.as_ref() == Some(action);
        let busy = self.busy();
        let change = action.clone();
        let reset = action.clone();
        let clear = action.clone();
        let reset_button = draft.is_changed(action).then(|| {
            Button::new(reset_id(action))
                .small()
                .ghost()
                .label("Reset")
                .accessibility_label(format!("Reset the shortcut of {label}"))
                .disabled(busy)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.keyboard_reset(reset.clone(), window, cx)
                }))
        });
        let mut change_button = Button::new(change_id(action))
            .small()
            .outline()
            .label("Change")
            .accessibility_label(format!("Change the shortcut of {label}"))
            .disabled(busy)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.keyboard_change(change.clone(), window, cx)
            }));
        if self.keyboard.last_row.as_ref() == Some(action) {
            change_button = change_button.track_focus(&self.keyboard.return_focus);
        }
        let clear_button = Button::new(clear_id(action))
            .small()
            .ghost()
            .label("Clear")
            .accessibility_label(format!("Clear the shortcut of {label}"))
            .disabled(busy || unbound)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.keyboard_clear(clear.clone(), window, cx)
            }));
        div()
            .id(row_id(action))
            .test_support()
            .role(Role::ListItem)
            .aria_label(SharedString::from(read))
            .flex()
            .flex_col()
            .gap_1()
            .py_1()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(gpui_kit::rems(10.))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme::foreground(t))
                                    .child(label),
                            )
                            .children(scope_tag.map(|tag| note(tag, t))),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(if listening {
                                theme::muted(t)
                            } else {
                                theme::bright(t)
                            })
                            .child(if listening {
                                "Press a shortcut…".to_string()
                            } else {
                                chords
                            }),
                    )
                    .child(
                        button_row()
                            .child(change_button)
                            .child(clear_button)
                            .children(reset_button),
                    ),
            )
            .children(self.render_keyboard_lines(action, t))
            .into_any_element()
    }

    pub(crate) fn render_keyboard(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let Some(draft) = self.keyboard.draft.as_ref() else {
            return note("Reading the shortcuts…", t).into_any_element();
        };
        let groups: Vec<AnyElement> = self
            .keyboard
            .groups
            .iter()
            .enumerate()
            .map(|(ix, group)| {
                let rows: Vec<AnyElement> = group
                    .rows
                    .iter()
                    .map(|r| self.render_keyboard_row(draft, &r.action, r.label, r.scope_tag, cx))
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .id(group_id(ix))
                            .test_support()
                            .role(Role::Heading)
                            .aria_label(group.title)
                            .pb_1()
                            .border_b_1()
                            .border_color(theme::divider(t))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::muted(t))
                            .child(group.title),
                    )
                    .children(rows)
                    .into_any_element()
            })
            .collect();
        let nothing_changed = !draft.has_changes();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(note(
                "Change a shortcut, then press the new keys. Esc cancels. If another \
                 action has the shortcut, it moves here. Nothing is kept until you save.",
                t,
            ))
            .child(
                button_row().child(
                    Button::new(RESET_ALL_ID)
                        .small()
                        .outline()
                        .label("Reset all")
                        .disabled(nothing_changed || self.busy())
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.keyboard_reset_all(cx)),
                        ),
                ),
            )
            .children(
                self.keyboard
                    .notice
                    .as_ref()
                    .map(|n| notice_line(NOTICE_ID, n, t)),
            )
            .child(self.render_keyboard_file(cx))
            .child(
                div()
                    .id(KEYBOARD_ID)
                    .test_support()
                    .role(Role::List)
                    .aria_label("Keyboard shortcuts")
                    .track_focus(&self.keyboard.focus)
                    .flex()
                    .flex_col()
                    .gap_5()
                    .children(groups),
            )
            .into_any_element()
    }
}
