//! The **leader** in the window (M6 G2): a gateway arms a key sequence that
//! walks the configured leader tree (`keys::leader::LeaderEngine`, the
//! TUI's own engine and key feed), and a fired leaf runs its **Leader
//! route** through the same performer the command palette uses
//! ([`Workspace::perform_route`]). A route the window cannot run yet is
//! refused with its reason in a flash (a toast), as the palette shows it on a
//! disabled row.
//!
//! **Gateways.** The `Leader` chord (primary+Shift+B) from any panel, and
//! a bare Space in the vim editor's Normal mode with nothing pending
//! (`Editor::space_leads`; never in Insert or Visual, the plain editor, the
//! find bar, the FILES filter or the rail). Refused while a dialog is open.
//!
//! **Keys while pending.** Every key goes to the sequence first, in the
//! workspace's keystroke interceptor, before key bindings and before the
//! editor's text input: a pending sequence's letters and digits never reach
//! the note, the IME or a field. With nothing pending the interceptor leaves
//! every key alone. Esc cancels and gives the note the keyboard; Backspace
//! steps up a level (cancelling at the root); a key the tree does not have
//! says so ("leader: no entry for 'x'") and the sequence stays pending; any
//! Ctrl or Cmd chord cancels the sequence and then does what it always does
//! (Ctrl+S saves), except the gateway chord itself, which just cancels (a
//! toggle). Other keys (arrows, function keys) are swallowed.
//!
//! **Which-key.** Typed fluently, a sequence shows nothing. Once it has not
//! moved for `leader_timeout_ms`, the which-key popup docks above the
//! Footer, over the bottom of the note: the pressed keys and the group's
//! caption, the controls hint, and the group's `key → target` rows
//! (`logic::which_key::WhichKey`). Each step of the sequence hides it and
//! restarts the wait; it goes as the sequence fires or cancels. The wait is
//! a GPUI timer (the test clock drives it), not the logic's wall-clock
//! `which_key::revealed`.

use std::time::Duration;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, KeystrokeEvent,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Task,
    Window, div, rems,
};
use kimun_notes::keys::leader::{LeaderAction, LeaderEngine, LeaderNode, LeaderOutcome};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::input::{Key, KeyInput};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::which_key::{CONTROLS, WhichKey};
use kimun_notes::settings::themes::Theme;

use super::{Workspace, modal};
use crate::input;
use crate::keymap;
use crate::theme;

/// The element id of the which-key popup.
pub(crate) const WHICH_KEY_ID: &str = "which-key";

/// The hesitation before the which-key shows when the window has no
/// settings (the shell tests): the settings' own default.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_millis(400);

/// The gateway's label when the `Leader` chord is unbound.
const UNBOUND_GATEWAY: &str = "Leader";

/// The vim Normal-mode gateway's label.
pub(crate) const SPACE_GATEWAY: &str = "Space";

/// The `Leader` chord's label (`Ctrl+Shift+B`) from the keymap's, or a
/// stand-in when the chord is unbound.
pub(crate) fn gateway_label(chord: Option<String>) -> SharedString {
    chord
        .map(SharedString::from)
        .unwrap_or_else(|| SharedString::new_static(UNBOUND_GATEWAY))
}

/// The which-key popup's text, built once as it reveals (from the logic's
/// [`WhichKey`]) and kept until the sequence moves: a frame only reads it.
pub(crate) struct WhichKeyView {
    pressed: SharedString,
    caption: SharedString,
    rows: Vec<WhichKeyRow>,
}

/// One `key → target` cell of the which-key.
struct WhichKeyRow {
    keys: SharedString,
    label: SharedString,
    is_group: bool,
}

impl WhichKeyView {
    fn new(model: WhichKey) -> Self {
        Self {
            pressed: model.pressed().to_string().into(),
            caption: model.caption().to_string().into(),
            rows: model
                .rows()
                .iter()
                .map(|row| WhichKeyRow {
                    keys: row.keys.clone().into(),
                    label: row.label.clone().into(),
                    is_group: row.is_group,
                })
                .collect(),
        }
    }

    /// The gateway and the keys pressed since (`Ctrl+Shift+B f`).
    #[cfg(test)]
    pub(crate) fn pressed(&self) -> &str {
        &self.pressed
    }

    /// How many `key → target` rows the group shows.
    #[cfg(test)]
    pub(crate) fn row_count(&self) -> usize {
        self.rows.len()
    }
}

/// A pending sequence and its which-key popup.
pub(crate) struct LeaderState {
    /// Walks the configured tree; kept across sequences, its tree replaced
    /// only when the settings are ([`Self::set_tree`]).
    engine: LeaderEngine,
    /// The gateway that armed the sequence, at the head of the pressed keys.
    gateway: SharedString,
    /// The which-key popup's text while it shows (the sequence has not
    /// moved for the timeout); `None` while hidden.
    shown: Option<WhichKeyView>,
    /// Reveals the popup when it fires; replaced on every step, dropped
    /// (cancelled) when the sequence ends.
    reveal: Option<Task<()>>,
}

impl Default for LeaderState {
    fn default() -> Self {
        Self {
            engine: LeaderEngine::new(),
            gateway: SharedString::default(),
            shown: None,
            reveal: None,
        }
    }
}

impl LeaderState {
    /// A sequence is pending.
    pub(crate) fn is_pending(&self) -> bool {
        self.engine.is_pending()
    }

    /// The which-key popup shows.
    #[cfg(test)]
    pub(crate) fn revealed(&self) -> bool {
        self.shown.is_some() && self.is_pending()
    }

    /// The popup's text while it shows.
    pub(crate) fn which_key(&self) -> Option<&WhichKeyView> {
        self.shown.as_ref().filter(|_| self.is_pending())
    }

    /// The configured leader tree (`[leader]` overrides applied), the one
    /// sequences walk and the palette lists; the default tree until
    /// [`Self::set_tree`].
    pub(crate) fn tree(&self) -> &LeaderNode {
        self.engine.tree()
    }

    /// Walk `tree` from now on (the settings' tree). Ends a pending
    /// sequence: its path may not exist in the new tree.
    pub(crate) fn set_tree(&mut self, tree: LeaderNode) {
        self.end();
        self.engine = LeaderEngine::with_tree(tree);
    }

    /// Show the popup for where the sequence is now.
    fn show(&mut self) {
        self.shown = Some(WhichKeyView::new(WhichKey::new(
            &self.engine,
            &self.gateway,
        )));
    }

    /// End the sequence without firing; the popup goes with it.
    fn end(&mut self) {
        self.engine.cancel();
        self.shown = None;
        self.reveal = None;
    }
}

/// A chord of the primary modifier (Ctrl, or Cmd on macOS): never the
/// sequence's. The engine's feed only knows Ctrl (the TUI's), so Cmd is
/// checked here.
fn is_primary_chord(key: KeyInput) -> bool {
    key.mods.ctrl || key.mods.sup
}

impl Workspace {
    /// The `Leader` chord's label (`Ctrl+Shift+B`), the gateway the palette's
    /// hints and the which-key header name. Cached with the keymap.
    pub(crate) fn leader_chord_label(&self) -> SharedString {
        self.chord_labels.leader.clone()
    }

    /// The gateway that arms a sequence where the keyboard is now: Space in
    /// the vim editor's Normal mode, the chord everywhere else.
    pub(crate) fn active_gateway(&self, opener: Panel, cx: &gpui_kit::App) -> SharedString {
        if opener == Panel::Editor && self.space_leads(cx) {
            SharedString::new_static(SPACE_GATEWAY)
        } else {
            self.leader_chord_label()
        }
    }

    /// Whether a bare Space arms the leader now: the vim editor is on screen
    /// with the keyboard, in Normal mode with nothing pending, its find bar
    /// closed and its input open.
    fn space_leads(&self, cx: &gpui_kit::App) -> bool {
        let view = self.editor.read(cx);
        self.focused_panel() == Panel::Editor
            && self.empty.is_none()
            && self.note_empty_state().is_none()
            && !view.input_blocked
            && !view.editor.find_open()
            && view.editor.space_leads()
    }

    /// The hesitation before the which-key shows (`leader_timeout_ms`).
    fn leader_timeout(&self) -> Duration {
        self.settings
            .as_ref()
            .map(|s| {
                let ms = s
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .leader_timeout_ms;
                Duration::from_millis(ms)
            })
            .unwrap_or(DEFAULT_TIMEOUT)
    }

    /// Arm a sequence over the configured tree (kept on the engine since
    /// the settings were read, [`LeaderState::set_tree`]), `gateway`
    /// heading its pressed keys. Refused while a dialog is open.
    pub(crate) fn start_leader(
        &mut self,
        gateway: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if modal::dialog_open(window, cx) {
            return;
        }
        self.leader.engine.start();
        self.leader.gateway = gateway.into();
        self.restart_reveal(cx);
        cx.notify();
    }

    /// End a pending sequence without firing.
    pub(crate) fn cancel_leader(&mut self, cx: &mut Context<Self>) {
        if self.leader.is_pending() {
            self.leader.end();
            cx.notify();
        }
    }

    /// The sequence moved (or just started): hide the which-key and wait
    /// for the next hesitation.
    fn restart_reveal(&mut self, cx: &mut Context<Self>) {
        self.leader.shown = None;
        let timeout = self.leader_timeout();
        self.leader.reveal = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(timeout).await;
            this.update(cx, |workspace, cx| {
                if workspace.leader.is_pending() {
                    workspace.leader.show();
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// A key seen by the workspace's interceptor, before any binding and
    /// before the editor's text input: the pending sequence's, or a bare
    /// Space arming one in vim Normal. True when the key was taken; false
    /// leaves it to the rest of the dispatch (a chord that cancelled the
    /// sequence, every key while nothing is pending).
    ///
    /// The gateway chord pressed again while a sequence is pending cancels
    /// it here (a toggle), and is taken: `workspace_command` never sees the
    /// `Leader` chord while a sequence is pending.
    pub(crate) fn intercept_leader_key(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.leader_wants(event) {
            return false;
        }
        let character_input = event.input_preference == gpui_kit::InputPreference::CharacterInput;
        let key = input::keystroke_input(&event.keystroke, character_input);
        if !self.leader.is_pending() {
            if key == Some(KeyInput::plain(Key::Char(' '))) && self.space_leads(cx) {
                self.start_leader(SPACE_GATEWAY, window, cx);
                return true;
            }
            return false;
        }
        let Some(key) = key else {
            // A bare modifier is no key; text with no single char (an
            // emoji sequence) is swallowed like any other non-leader key.
            return !event.keystroke.is_modifier_key();
        };
        if is_primary_chord(key) {
            let action = keymap::active(cx).resolve(key, self.key_ctx(cx)).cloned();
            self.cancel_leader(cx);
            // The gateway again is a toggle; any other chord goes on.
            return action == Some(ActionShortcuts::Leader);
        }
        match self.leader.engine.key(key) {
            LeaderOutcome::Cancelled => {
                self.leader.end();
                if key.key == Key::Esc {
                    self.set_focus_panel(Panel::Editor);
                    self.apply_focus(window, cx);
                }
            }
            LeaderOutcome::Descended | LeaderOutcome::SteppedUp => self.restart_reveal(cx),
            LeaderOutcome::Invalid => {
                if let Key::Char(c) = key.key {
                    self.flash(format!("leader: no entry for '{c}'"), cx);
                }
                self.restart_reveal(cx);
            }
            LeaderOutcome::Fired(action) => {
                self.leader.end();
                self.fire_leader(action, window, cx);
            }
            LeaderOutcome::Swallowed => {}
            // Ctrl chords are handled above.
            LeaderOutcome::Passed => {
                self.leader.end();
                return false;
            }
        }
        true
    }

    /// Whether the leader could take this key: a sequence is pending (it
    /// takes every key), or it may be the bare Space that arms one. Cheap,
    /// so every other key skips building a [`KeyInput`] and the modal check.
    pub(crate) fn leader_wants(&self, event: &KeystrokeEvent) -> bool {
        self.leader.is_pending() || could_be_space(&event.keystroke)
    }

    /// A leaf fired: run its route where the keyboard is, or say why it
    /// cannot run here.
    pub(super) fn fire_leader(
        &mut self,
        action: LeaderAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let route = self.route_of(action);
        match self.route_block(&route, self.focused_panel(), cx) {
            Some(reason) => self.flash(reason, cx),
            None => self.perform_route(route, window, cx),
        }
    }

    /// The which-key popup, docked at the bottom of the editor Panel's
    /// document area (above the Footer), while it shows.
    pub(crate) fn render_which_key(&self, cx: &gpui_kit::App) -> Option<AnyElement> {
        let model = self.leader.which_key()?;
        let mono = &cx.theme().mono_font_family;
        let theme = theme::current(cx);
        let t = &*theme;
        let header = div()
            .flex()
            .flex_row()
            .items_baseline()
            .gap_3()
            .child(
                div()
                    .flex_none()
                    .font_family(mono.clone())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::key_cap(t))
                    .child(model.pressed.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme::secondary(t))
                    .child(model.caption.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme::muted(t))
                    .child(CONTROLS),
            );
        let rows = model.rows.iter().map(|row| which_key_row(row, mono, t));
        Some(
            div()
                .id(WHICH_KEY_ID)
                .test_support()
                .role(Role::Group)
                .aria_label(model.pressed.clone())
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .flex()
                .flex_col()
                .gap_2()
                .px_4()
                .py_3()
                .text_sm()
                .bg(theme::panel_background(t))
                .border_t_1()
                .border_color(theme::divider(t))
                .child(header)
                .child(div().flex().flex_row().flex_wrap().gap_y_1().children(rows))
                .into_any_element(),
        )
    }
}

/// Whether `ks` may be a bare Space: its key, or the text it types. A
/// superset of what [`input::keystroke_input`] turns into a plain Space.
fn could_be_space(ks: &gpui_kit::Keystroke) -> bool {
    ks.key == "space" || ks.key == " " || ks.key_char.as_deref() == Some(" ")
}

/// One `key → target` cell; a group's target in the group colour.
fn which_key_row(row: &WhichKeyRow, mono: &SharedString, t: &Theme) -> AnyElement {
    let target = if row.is_group {
        theme::key_group(t)
    } else {
        theme::foreground(t)
    };
    div()
        .flex()
        .flex_row()
        .flex_none()
        .w(rems(13.))
        .gap_2()
        .pr_2()
        .child(
            div()
                .flex_none()
                .min_w(rems(1.5))
                .font_family(mono.clone())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::key_cap(t))
                .child(row.keys.clone()),
        )
        .child(div().flex_none().text_color(theme::muted(t)).child("→"))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(target)
                .child(row.label.clone()),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Keystroke, Modifiers};
    use kimun_notes::logic::input::{Key, KeyInput};

    use super::could_be_space;
    use crate::input;

    fn ks(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: key_char.map(str::to_string),
        }
    }

    /// The prefilter never drops a key `keystroke_input` reads as a plain
    /// Space, and lets ordinary keys skip the leader.
    #[test]
    fn could_be_space_covers_every_plain_space() {
        let space = Some(KeyInput::plain(Key::Char(' ')));
        let none = Modifiers::default();
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        let cases = [
            ks("space", None, none),
            ks("space", Some(" "), none),
            ks(" ", Some(" "), none),
            ks("a", Some("a"), none),
            ks("a", Some("A"), shift),
            ks("enter", None, none),
            ks("b", None, Modifiers::control()),
        ];
        for keystroke in &cases {
            for character_input in [false, true] {
                if input::keystroke_input(keystroke, character_input) == space {
                    assert!(could_be_space(keystroke), "{keystroke:?} dropped");
                }
            }
        }
        assert!(could_be_space(&cases[0]));
        assert!(!could_be_space(&cases[3]));
        assert!(!could_be_space(&cases[5]));
    }
}
