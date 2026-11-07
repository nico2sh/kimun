//! The **CFG** drawer view (M9c): a read-only summary of the settings that
//! shape the window, and the two doors to change them. The TUI's
//! `ConfigPanel`, in the desktop app's words.
//!
//! - **Rows.** Theme, Leader key, Preferences key, Which-key delay and
//!   Config file, read from the shared settings as the view renders, so a
//!   theme pick or a Preferences save shows at once, without a reopen. The
//!   key rows are the desktop app's own chords (`keymap`), every chord of the
//!   action (`F4 / Ctrl+,`), or "not set" when none is bound. A path too long
//!   for the drawer ends in an ellipsis; its tooltip holds all of it.
//! - **Theme name.** The configured name resolves against the theme list,
//!   which reads the themes folder: that runs on tokio's blocking pool when
//!   the configured name changes, and the configured name (or the default
//!   theme's) shows until it lands. Nothing else here reads a file.
//! - **Launchers.** `Theme…` (Enter or `t` with the view focused) opens the
//!   theme picker, `Preferences…` (`p`) opens Preferences, `Keyboard
//!   shortcuts…` (`k`) the keyboard reference — through the same leader
//!   leaves (`vault.theme`, `vault.preferences`, `help`) and the same
//!   refusals as the leader, so none opens over a modal or the setup
//!   assistant. All are kit Buttons, mouse and keyboard, after the view in
//!   the Tab order.
//! - **No vault needed.** The view only reads the settings: it works in
//!   every window state, a workspace open or not.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, rems,
};
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::input::{Key, KeyInput};
use kimun_notes::logic::preferences::config_path_label;
use kimun_notes::settings::SharedSettings;
use kimun_notes::settings::themes::Theme;

use super::files::ellipsis;
use crate::input;
use crate::keymap;
use crate::tasks::blocking;
use crate::theme;

/// Element ids, for tests.
pub(crate) const CONFIG_ID: &str = "config";
pub(crate) const CONFIG_THEME_ID: &str = "config-theme";
pub(crate) const CONFIG_PREFERENCES_ID: &str = "config-preferences";
pub(crate) const CONFIG_KEYS_ID: &str = "config-keys";
pub(crate) const CONFIG_PATH_ID: &str = "config-path";

/// What a key row says when its action has no chord.
pub(crate) const NOT_SET: &str = "not set";
/// What the view says before the window has its settings (a window built
/// without them, in the tests).
pub(crate) const NO_SETTINGS_TEXT: &str = "No settings loaded";

/// The label column's width, in rems: the longest label fits.
const LABEL_REMS: f32 = 7.5;

/// A launcher asked for: the leader leaf it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigEvent {
    Launch(LeaderAction),
}

/// One summary row: what it shows, as the view draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigRow {
    pub(crate) label: &'static str,
    pub(crate) value: String,
}

/// Every chord bound to `action`, joined, or [`NOT_SET`].
fn keys_text(cx: &gpui_kit::App, action: ActionShortcuts) -> String {
    let chords = keymap::chord_labels(cx, &action);
    if chords.is_empty() {
        NOT_SET.to_string()
    } else {
        chords.join(" / ")
    }
}

/// The theme name shown while `configured` is not resolved: the configured
/// name, the default theme's when none is configured.
fn fallback_theme_name(configured: &str) -> String {
    if configured.is_empty() {
        Theme::default().name
    } else {
        configured.to_string()
    }
}

/// The configured theme name the resolved name is for, and that name.
struct Resolved {
    configured: String,
    name: String,
}

/// The CFG view. See the module docs.
pub(crate) struct ConfigView {
    /// The window's shared settings, once it has them.
    settings: Option<SharedSettings>,
    /// The resolved theme name, for the configured name it was resolved
    /// from.
    resolved: Option<Resolved>,
    /// The resolution under way, for the configured name it resolves.
    resolving: Option<(String, Task<()>)>,
    focus: FocusHandle,
    theme_focus: FocusHandle,
    preferences_focus: FocusHandle,
    keys_focus: FocusHandle,
}

impl EventEmitter<ConfigEvent> for ConfigView {}

impl Focusable for ConfigView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ConfigView {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            settings: None,
            resolved: None,
            resolving: None,
            focus: cx.focus_handle().tab_stop(true),
            theme_focus: cx.focus_handle(),
            preferences_focus: cx.focus_handle(),
            keys_focus: cx.focus_handle(),
        }
    }

    /// The window's settings (set once, when the window starts).
    pub(crate) fn set_settings(&mut self, settings: SharedSettings, cx: &mut Context<Self>) {
        self.settings = Some(settings);
        cx.notify();
    }

    /// The view's root and its three launchers, in Tab order (tests).
    #[cfg(test)]
    pub(crate) fn handles(&self) -> [FocusHandle; 4] {
        [
            self.focus.clone(),
            self.theme_focus.clone(),
            self.preferences_focus.clone(),
            self.keys_focus.clone(),
        ]
    }

    /// The rows as the view draws them now; `None` without settings.
    pub(crate) fn rows(&mut self, cx: &mut Context<Self>) -> Option<Vec<ConfigRow>> {
        let settings = self.settings.clone()?;
        let s = settings.read().unwrap_or_else(|e| e.into_inner());
        let theme_name = self.theme_name(&s.theme, cx);
        let keys = |action: ActionShortcuts| keys_text(cx, action);
        Some(vec![
            ConfigRow {
                label: "Theme",
                value: theme_name,
            },
            ConfigRow {
                label: "Leader key",
                value: keys(ActionShortcuts::Leader),
            },
            ConfigRow {
                label: "Preferences",
                value: keys(ActionShortcuts::OpenPreferences),
            },
            ConfigRow {
                label: "Which-key delay",
                value: format!("{} ms", s.leader_timeout_ms),
            },
            ConfigRow {
                label: "Config file",
                value: config_path_label(&s),
            },
        ])
    }

    /// The theme `configured` names, as far as it is known: the resolved
    /// name once its resolution landed, else the configured name (the
    /// default theme's when none is configured) while one is started.
    fn theme_name(&mut self, configured: &str, cx: &mut Context<Self>) -> String {
        if let Some(resolved) = self
            .resolved
            .as_ref()
            .filter(|r| r.configured == configured)
        {
            return resolved.name.clone();
        }
        self.resolve(configured.to_string(), cx);
        fallback_theme_name(configured)
    }

    /// Resolve `configured` against the theme list on the blocking pool,
    /// unless that is under way already. Without a tokio runtime (a window
    /// built in a test without one) nothing starts: the configured name
    /// stays.
    fn resolve(&mut self, configured: String, cx: &mut Context<Self>) {
        if self
            .resolving
            .as_ref()
            .is_some_and(|(name, _)| *name == configured)
            || tokio::runtime::Handle::try_current().is_err()
        {
            return;
        }
        let Some(settings) = self.settings.clone() else {
            return;
        };
        let job = blocking(move || {
            settings
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .get_theme()
                .name
        });
        let key = configured.clone();
        let task = cx.spawn(async move |this, cx| {
            // A failed job (a panic, a runtime going down) settles on the
            // fallback too: with nothing recorded, the next render would
            // start the job again, and its failure would render again.
            let name = job.await.unwrap_or_else(|_| fallback_theme_name(&key));
            this.update(cx, |view, cx| {
                view.resolving = None;
                view.resolved = Some(Resolved {
                    configured: key,
                    name,
                });
                cx.notify();
            })
            .ok();
        });
        self.resolving = Some((configured, task));
    }

    fn launch(&mut self, action: LeaderAction, cx: &mut Context<Self>) {
        cx.emit(ConfigEvent::Launch(action));
    }

    /// Enter and `t` open the theme picker, `p` Preferences, `k` the
    /// keyboard reference. Enter only
    /// on the view itself: on a launcher it is the launcher's.
    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(KeyInput { key, mods }) = input::key_down_input(event) else {
            return;
        };
        if mods.ctrl || mods.alt || mods.sup {
            return;
        }
        match key {
            Key::Enter if self.focus.is_focused(window) => {
                self.launch(LeaderAction::VaultTheme, cx)
            }
            Key::Char('t') if !mods.shift => self.launch(LeaderAction::VaultTheme, cx),
            Key::Char('p') if !mods.shift => self.launch(LeaderAction::VaultPreferences, cx),
            Key::Char('k') if !mods.shift => self.launch(LeaderAction::Help, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_row(&self, row: ConfigRow, t: &Theme) -> AnyElement {
        let value = ellipsis(row.value.clone()).text_color(theme::foreground(t));
        let value = if row.label == "Config file" {
            let full = SharedString::from(row.value);
            div()
                .id(CONFIG_PATH_ID)
                .test_support()
                .flex_1()
                .min_w_0()
                .child(value)
                .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                .into_any_element()
        } else {
            div().flex_1().min_w_0().child(value).into_any_element()
        };
        div()
            .flex()
            .gap_2()
            .min_w_0()
            .text_sm()
            .child(
                div()
                    .flex_none()
                    .w(rems(LABEL_REMS))
                    .text_color(theme::muted(t))
                    .child(row.label),
            )
            .child(value)
            .into_any_element()
    }

    fn render_launchers(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new(CONFIG_THEME_ID)
                    .small()
                    .label("Theme…")
                    .tooltip("Pick a theme (t or Enter)")
                    .track_focus(&self.theme_focus)
                    .on_click(
                        cx.listener(|this, _, _, cx| this.launch(LeaderAction::VaultTheme, cx)),
                    ),
            )
            .child(
                Button::new(CONFIG_PREFERENCES_ID)
                    .small()
                    .label("Preferences…")
                    .tooltip("Open Preferences (p)")
                    .track_focus(&self.preferences_focus)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.launch(LeaderAction::VaultPreferences, cx)
                    })),
            )
            .child(
                Button::new(CONFIG_KEYS_ID)
                    .small()
                    .label("Keyboard shortcuts…")
                    .tooltip("Every shortcut and leader key (k)")
                    .track_focus(&self.keys_focus)
                    .on_click(cx.listener(|this, _, _, cx| this.launch(LeaderAction::Help, cx))),
            )
    }
}

impl Render for ConfigView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let rows = self.rows(cx);
        let summary = div()
            .id("config-summary")
            .role(Role::Group)
            .aria_label("Settings")
            .flex()
            .flex_col()
            .gap_1();
        let summary = match rows {
            Some(rows) => summary.children(rows.into_iter().map(|row| self.render_row(row, t))),
            None => summary.child(
                div()
                    .text_sm()
                    .text_color(theme::muted(t))
                    .child(NO_SETTINGS_TEXT),
            ),
        };
        div()
            .id(CONFIG_ID)
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_3()
            .px_3()
            .py_2()
            .child(summary)
            .child(self.render_launchers(cx))
    }
}
