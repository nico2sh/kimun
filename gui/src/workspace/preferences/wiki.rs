//! Preferences > **Wiki**: the wiki declaration of the open vault
//! (`.kimun/wiki.toml`), as five fields and one button.
//!
//! The section acts at once, outside the draft, as Indexing does: its
//! button applies the declaration to the vault, and Save and Cancel never
//! touch it. The rules (the state, the defaults, the checks, the effect
//! note, the button label) are the logic `WikiSettings` model's. This view
//! draws it and runs [`wiki_settings::load`] and [`wiki_settings::apply`]
//! off the UI thread. The view never writes a file or makes a folder.
//!
//! The section reads the declaration when it opens, when the window names
//! another vault, and after every apply that worked. Each read and apply
//! carries the vault folder it ran on: a result for a folder that the window
//! no longer names is dropped. A good apply also tells the window to load
//! its views again, so FILES lists the new folders.

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, Focusable as _, InteractiveElement,
    IntoElement, ParentElement, Role, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, div,
};
use kimun_core::system::SystemPath;
use kimun_notes::logic::preferences as prefs;
use kimun_notes::logic::wiki_settings::{self, Loaded, WikiSettings, WikiState};

use super::workspaces::{NEEDS_WORKSPACE, Notice, button_row, notice_line};
use super::{PreferencesEvent, PreferencesView, field, note, para};
use crate::theme;

/// The section's elements.
pub(crate) const WIKI_STATE_ID: &str = "preferences-wiki-state";
pub(crate) const WIKI_SOURCES_ID: &str = "preferences-wiki-sources";
pub(crate) const WIKI_WIKI_ID: &str = "preferences-wiki-wiki";
pub(crate) const WIKI_LOG_ID: &str = "preferences-wiki-log";
pub(crate) const WIKI_SCHEMA_ID: &str = "preferences-wiki-schema";
pub(crate) const WIKI_PROPERTIES_ID: &str = "preferences-wiki-properties";
pub(crate) const WIKI_ERROR_ID: &str = "preferences-wiki-error";
pub(crate) const WIKI_EFFECT_ID: &str = "preferences-wiki-effect";
pub(crate) const WIKI_APPLY_ID: &str = "preferences-wiki-apply";
pub(crate) const WIKI_RESULT_ID: &str = "preferences-wiki-result";
/// Why the fields are off.
pub(crate) const WIKI_INERT_ID: &str = "preferences-wiki-inert";
/// What a wiki is.
pub(crate) const WIKI_INTRO_ID: &str = "preferences-wiki-intro";
/// The words of the declaration reader, in their own surface.
pub(crate) const WIKI_DETAIL_ID: &str = "preferences-wiki-detail";
/// What the button does.
pub(crate) const WIKI_BUTTON_NOTE_ID: &str = "preferences-wiki-button-note";

/// What a wiki is, at the top of the section.
const INTRO: [&str; 3] = [
    "A wiki is a set of notes that an AI agent writes and keeps up to date from your sources.",
    "You collect raw sources, such as saved web pages. The agent reads them and writes the wiki pages.",
    "Kimün saves web pages as sources, protects the sources from automated edits, and checks the wiki for mistakes.",
];

/// The note under the button for a vault that is not a wiki.
const SET_UP_NOTE: &str = "Set up wiki makes the two folders and the two notes above, and marks this vault as a wiki. It never changes a note that exists.";

/// The note under the button for a wiki.
const APPLY_NOTE: &str = "Apply saves these values. It makes any folder or note that is missing. It never moves or deletes a note.";

/// Above the words of the declaration reader.
const DETAIL_HEAD: &str = "The reader said:";

/// The state line while the declaration is read.
const READING: &str = "Reading the wiki settings.";

/// One field of the section, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WikiField {
    Sources,
    Wiki,
    Log,
    Schema,
    Properties,
}

impl WikiField {
    pub(crate) const ALL: [WikiField; 5] = [
        WikiField::Sources,
        WikiField::Wiki,
        WikiField::Log,
        WikiField::Schema,
        WikiField::Properties,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            WikiField::Sources => "Sources folder",
            WikiField::Wiki => "Wiki folder",
            WikiField::Log => "Log note",
            WikiField::Schema => "Schema note",
            WikiField::Properties => "Required properties",
        }
    }

    /// What the field is for.
    fn hint(self) -> &'static str {
        match self {
            WikiField::Sources => {
                "Raw material goes here, such as saved web pages. Kimün protects this folder from automated edits."
            }
            WikiField::Wiki => {
                "The pages that the agent writes and keeps up to date from your sources."
            }
            WikiField::Log => "A note where the agent adds one entry for each piece of work.",
            WikiField::Schema => {
                "A note with the rules for the agent: page names, properties, and how to read a source."
            }
            WikiField::Properties => {
                "Every wiki page needs these properties. The wiki check reports a page that lacks one. Separate the names with commas."
            }
        }
    }

    /// The element id of the field's hint.
    pub(crate) fn hint_id(self) -> &'static str {
        match self {
            WikiField::Sources => "preferences-wiki-hint-sources",
            WikiField::Wiki => "preferences-wiki-hint-wiki",
            WikiField::Log => "preferences-wiki-hint-log",
            WikiField::Schema => "preferences-wiki-hint-schema",
            WikiField::Properties => "preferences-wiki-hint-properties",
        }
    }

    fn id(self) -> &'static str {
        match self {
            WikiField::Sources => WIKI_SOURCES_ID,
            WikiField::Wiki => WIKI_WIKI_ID,
            WikiField::Log => WIKI_LOG_ID,
            WikiField::Schema => WIKI_SCHEMA_ID,
            WikiField::Properties => WIKI_PROPERTIES_ID,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// The draft's text for this field.
    fn text(self, settings: &WikiSettings) -> &str {
        let d = settings.draft();
        match self {
            WikiField::Sources => &d.sources,
            WikiField::Wiki => &d.wiki,
            WikiField::Log => &d.log,
            WikiField::Schema => &d.schema,
            WikiField::Properties => &d.properties,
        }
    }

    /// Put `text` into the draft through the model's setter.
    fn set(self, settings: &mut WikiSettings, text: &str) {
        match self {
            WikiField::Sources => settings.set_sources(text),
            WikiField::Wiki => settings.set_wiki(text),
            WikiField::Log => settings.set_log(text),
            WikiField::Schema => settings.set_schema(text),
            WikiField::Properties => settings.set_properties(text),
        }
    }
}

/// What the last apply gave.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// One line for each item, such as `Created folder /raw.`
    Lines(Vec<String>),
    /// The core's error. Nothing changed.
    Failed(String),
}

/// The Wiki section's state.
pub(crate) struct WikiPane {
    /// The five fields, in [`WikiField::ALL`] order.
    inputs: [Entity<InputState>; 5],
    /// The model of what the section shows.
    settings: WikiSettings,
    /// The last read, for the effect note.
    loaded: Option<Loaded>,
    /// The vault folder the state belongs to, or is being read for.
    target: Option<SystemPath>,
    /// The read under way.
    loading: Option<Task<()>>,
    /// The apply under way: the running marker. Every control waits for it.
    running: Option<Task<()>>,
    /// The result of the last apply.
    outcome: Option<Outcome>,
    _subscriptions: Vec<Subscription>,
}

impl WikiPane {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<PreferencesView>) -> Self {
        let settings = WikiSettings::new(WikiState::NoVault);
        let inputs = WikiField::ALL.map(|field| {
            let text = field.text(&settings).to_string();
            cx.new(|cx| InputState::new(window, cx).default_value(text))
        });
        let subscriptions = WikiField::ALL
            .iter()
            .map(|&field| {
                cx.subscribe_in(
                    &inputs[field.index()],
                    window,
                    move |this, _, event: &InputEvent, window, cx| {
                        if matches!(event, InputEvent::Change) {
                            this.wiki_changed(field, window, cx);
                        }
                    },
                )
            })
            .collect();
        Self {
            inputs,
            settings,
            loaded: None,
            target: None,
            loading: None,
            running: None,
            outcome: None,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn running(&self) -> bool {
        self.running.is_some()
    }
}

impl PreferencesView {
    // ---- reading (the tests) ----------------------------------------------

    /// The field `field`.
    #[cfg(test)]
    pub(crate) fn wiki_input(&self, field: WikiField) -> Entity<InputState> {
        self.wiki.inputs[field.index()].clone()
    }

    /// Whether the declaration is being read.
    #[cfg(test)]
    pub(crate) fn wiki_loading(&self) -> bool {
        self.wiki.loading.is_some()
    }

    /// Whether an apply runs.
    #[cfg(test)]
    pub(crate) fn wiki_running(&self) -> bool {
        self.wiki.running.is_some()
    }

    /// Whether the last apply failed.
    #[cfg(test)]
    pub(crate) fn wiki_failed(&self) -> bool {
        matches!(self.wiki.outcome, Some(Outcome::Failed(_)))
    }

    /// The model, for a test to change the draft behind the fields.
    #[cfg(test)]
    pub(crate) fn wiki_settings_mut(&mut self) -> &mut WikiSettings {
        &mut self.wiki.settings
    }

    // ---- the vault ----------------------------------------------------------

    /// The vault folder and index the window names now.
    fn wiki_target(&self) -> Option<(SystemPath, Option<kimun_core::IndexFile>)> {
        prefs::reindex_target(&self.settings.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// Whether the window still names the vault at `workspace`.
    fn wiki_still_on(&self, workspace: &SystemPath) -> bool {
        self.wiki_target().is_some_and(|(w, _)| &w == workspace)
    }

    /// Read the declaration of the vault the window names, off the UI
    /// thread. Without a vault the section shows the "no vault" state.
    pub(crate) fn wiki_reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((workspace, cache)) = self.wiki_target() else {
            self.wiki.target = None;
            self.wiki.loading = None;
            self.wiki.loaded = None;
            self.wiki.settings = WikiSettings::new(WikiState::NoVault);
            self.wiki_sync_inputs(window, cx);
            cx.notify();
            return;
        };
        self.wiki.target = Some(workspace.clone());
        let task = cx.spawn_in(window, async move |this, cx| {
            let ran_on = workspace.clone();
            let loaded = match tokio::spawn(wiki_settings::load(workspace, cache)).await {
                Ok(loaded) => loaded,
                Err(e) => Loaded {
                    state: WikiState::Broken(e.to_string()),
                    schema_exists: false,
                },
            };
            this.update_in(cx, |view, window, cx| {
                view.wiki_loaded(ran_on, loaded, window, cx)
            })
            .ok();
        });
        self.wiki.loading = Some(task);
        cx.notify();
    }

    /// The section opens. It reads the declaration again, so a change made
    /// outside the app shows. If the user typed something here and did not
    /// apply it, and the window names the same vault, the typed text stays.
    pub(crate) fn wiki_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let same_vault = self.wiki_target().map(|(w, _)| w) == self.wiki.target;
        if same_vault && self.wiki.settings.is_edited() {
            return;
        }
        self.wiki_reload(window, cx);
    }

    /// A read of the vault at `workspace` landed. Dropped when the window
    /// names another vault now.
    pub(crate) fn wiki_loaded(
        &mut self,
        workspace: SystemPath,
        loaded: Loaded,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.wiki_still_on(&workspace) || self.wiki.target.as_ref() != Some(&workspace) {
            return;
        }
        self.wiki.loading = None;
        self.wiki.settings = WikiSettings::new(loaded.state.clone());
        self.wiki.loaded = Some(loaded);
        self.wiki_sync_inputs(window, cx);
        cx.notify();
    }

    /// The window may name another vault than the one shown (a workspace
    /// change): read the new one. Called on every frame of the section.
    pub(crate) fn wiki_follow_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = self.wiki_target().map(|(w, _)| w);
        if now != self.wiki.target {
            self.wiki.outcome = None;
            self.wiki_reload(window, cx);
        }
    }

    // ---- editing ------------------------------------------------------------

    /// The user typed in `field`: the model takes the text, and the other
    /// fields show what the model made of it (the log and schema follow the
    /// wiki folder).
    fn wiki_changed(&mut self, field: WikiField, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.wiki.inputs[field.index()].read(cx).value().to_string();
        field.set(&mut self.wiki.settings, &text);
        self.wiki_sync_inputs(window, cx);
        cx.notify();
    }

    /// Each field shows the draft's text. A field that already does is left
    /// alone, so the one being typed in keeps its cursor.
    fn wiki_sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in WikiField::ALL {
            let text = field.text(&self.wiki.settings).to_string();
            self.wiki.inputs[field.index()].update(cx, |input, cx| {
                if input.value().as_ref() != text {
                    input.set_value(text, window, cx);
                }
            });
        }
    }

    /// Whether one of the section's fields has the keyboard.
    pub(crate) fn wiki_field_focused(&self, window: &Window, cx: &gpui_kit::App) -> bool {
        self.wiki
            .inputs
            .iter()
            .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
    }

    /// Whether the button would apply now.
    fn wiki_can_apply(&self) -> bool {
        !self.busy() && self.wiki.loading.is_none() && self.wiki.settings.can_apply()
    }

    // ---- applying -----------------------------------------------------------

    /// The button (Enter in a field): apply the draft to the vault, off the
    /// UI thread. Does nothing while anything runs, while the declaration
    /// is read, or while the draft has an error.
    pub(crate) fn wiki_apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.wiki_can_apply() {
            return;
        }
        let Some((workspace, cache)) = self.wiki_target() else {
            return;
        };
        let Ok(declaration) = self.wiki.settings.declaration() else {
            return;
        };
        self.wiki.outcome = None;
        let task = cx.spawn_in(window, async move |this, cx| {
            let ran_on = workspace.clone();
            let result =
                match tokio::spawn(wiki_settings::apply(workspace, cache, declaration)).await {
                    Ok(result) => result,
                    Err(e) => Err(e.to_string()),
                };
            this.update_in(cx, |view, window, cx| {
                view.wiki_applied(ran_on, result, window, cx)
            })
            .ok();
        });
        self.wiki.running = Some(task);
        cx.notify();
    }

    /// The apply on the vault at `workspace` ended. Dropped when the window
    /// names another vault now. A good apply reads the vault again. A failed
    /// one keeps the fields as typed.
    fn wiki_applied(
        &mut self,
        workspace: SystemPath,
        result: Result<kimun_core::WikiInitReport, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.wiki.running = None;
        if !self.wiki_still_on(&workspace) {
            cx.notify();
            return;
        }
        match result {
            Ok(report) => {
                self.wiki.outcome = Some(Outcome::Lines(wiki_settings::report_lines(&report)));
                self.wiki_reload(window, cx);
                // The apply wrote through the same index: the window's views
                // load again, so FILES lists the new folders.
                cx.emit(PreferencesEvent::Reindexed);
            }
            Err(e) => {
                tracing::warn!("could not apply the wiki declaration: {e}");
                self.wiki.outcome = Some(Outcome::Failed(e));
            }
        }
        cx.notify();
    }

    // ---- rendering ----------------------------------------------------------

    /// The Wiki section: the state line, the five fields, the field error,
    /// the effect note, the button, and the result of the last apply.
    pub(crate) fn render_wiki(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme::current(cx);
        let t = &*theme;
        let pane = &self.wiki;
        let no_vault = matches!(pane.settings.state(), WikiState::NoVault);
        let off = no_vault || self.busy() || pane.loading.is_some();
        let state_line: SharedString = if pane.loading.is_some() {
            READING.into()
        } else {
            pane.settings.state_line().into()
        };
        let fields = WikiField::ALL.map(|f| {
            let input = Input::new(&pane.inputs[f.index()])
                .id(f.id())
                .aria_label(f.label())
                .disabled(off)
                .bg(theme::background(t))
                .border_color(theme::divider(t))
                .text_color(theme::foreground(t));
            let hint = div()
                .id(f.hint_id())
                .test_support()
                .role(Role::Label)
                .aria_label(f.hint())
                .child(note(f.hint(), t));
            field(f.label(), input, [hint], t).into_any_element()
        });
        let error = (!off).then(|| pane.settings.error()).flatten().map(|e| {
            div()
                .id(WIKI_ERROR_ID)
                .test_support()
                .role(Role::Label)
                .aria_label(e.clone())
                .text_xs()
                .text_color(theme::error(t))
                .child(e)
        });
        let detail = pane
            .settings
            .detail()
            .filter(|_| pane.loading.is_none())
            .map(|text| {
                let mono = cx.theme().mono_font_family.clone();
                // One row for each line, so the pointer under the faulty
                // text of the reader stays under it in the monospace font.
                let rows = text.lines().map(|line| {
                    let line = if line.is_empty() { "\u{a0}" } else { line };
                    div().child(line.to_string())
                });
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(note(DETAIL_HEAD, t))
                    .child(
                        div()
                            .id(WIKI_DETAIL_ID)
                            .test_support()
                            .role(Role::Label)
                            .aria_label(text.to_string())
                            .flex()
                            .flex_col()
                            .p_3()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::divider(t))
                            .bg(theme::panel_background(t))
                            .font_family(mono)
                            .text_xs()
                            .text_color(theme::foreground(t))
                            .children(rows),
                    )
            });
        let button_note = (!no_vault && pane.loading.is_none()).then(|| {
            let text = match pane.settings.state() {
                WikiState::Wiki(_) => APPLY_NOTE,
                _ => SET_UP_NOTE,
            };
            div()
                .id(WIKI_BUTTON_NOTE_ID)
                .test_support()
                .role(Role::Label)
                .aria_label(text)
                .child(note(text, t))
        });
        let schema_exists = pane.loaded.as_ref().is_some_and(|l| l.schema_exists);
        let effect = pane.settings.effect_note(schema_exists);
        let effect = (!effect.is_empty() && pane.loading.is_none()).then(|| {
            div()
                .id(WIKI_EFFECT_ID)
                .test_support()
                .role(Role::Label)
                .aria_label(effect.join(" "))
                .flex()
                .flex_col()
                .gap_1()
                .children(effect.into_iter().map(|line| para(line, t)))
        });
        let outcome = pane.outcome.as_ref().map(|o| match o {
            Outcome::Failed(e) => notice_line(WIKI_RESULT_ID, &Notice::error(e.clone()), t),
            Outcome::Lines(lines) => div()
                .id(WIKI_RESULT_ID)
                .test_support()
                .role(Role::Label)
                .aria_label(lines.join(" "))
                .flex()
                .flex_col()
                .gap_1()
                .children(lines.iter().map(|line| note(line.clone(), t)))
                .into_any_element(),
        });
        let running = pane.running.is_some();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .id(WIKI_INTRO_ID)
                    .test_support()
                    .role(Role::Label)
                    .aria_label(INTRO.join(" "))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(INTRO.iter().map(|line| para(*line, t))),
            )
            .child(
                div()
                    .id(WIKI_STATE_ID)
                    .test_support()
                    .role(Role::Label)
                    .aria_label(state_line.clone())
                    .child(para(state_line, t)),
            )
            .children(detail)
            .children(no_vault.then(|| {
                div()
                    .id(WIKI_INERT_ID)
                    .test_support()
                    .role(Role::Label)
                    .aria_label(NEEDS_WORKSPACE)
                    .child(note(NEEDS_WORKSPACE, t))
            }))
            .child(div().flex().flex_col().gap_3().children(fields))
            .children(error)
            .children(effect)
            .child(
                button_row().child(
                    Button::new(WIKI_APPLY_ID)
                        .label(pane.settings.button_label())
                        .loading(running)
                        .disabled(!self.wiki_can_apply())
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.wiki_apply(window, cx)
                        })),
                ),
            )
            .children(button_note)
            .children(outcome)
            .into_any_element()
    }
}
