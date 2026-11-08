//! The GUI's own keymap (ADR-0050 §6): chords for the shared action
//! vocabulary ([`ActionShortcuts`]), the `[gui.keys]` overrides, and the vim
//! rule. Pure data, headless-testable; the only platform knowledge is the
//! [`Platform`] value passed in, chosen at the edge ([`Platform::current`]),
//! so either platform's table can be tested anywhere.
//!
//! **Chords.** A chord is the toolkit-neutral [`KeyInput`] the editor already
//! receives (`gui/src/input.rs`): the physical key, letters lower-case, with
//! Shift as a modifier (`Ctrl+Shift+F` is `Char('f')` + Ctrl + Shift) and
//! Shift+Tab as `BackTab`. Written chords join modifiers and one key with
//! `+`: `primary+shift+f`, `alt+left`, `ctrl+tab`. Modifiers are `primary`
//! (Cmd on macOS, Ctrl elsewhere), `cmd`, `ctrl`, `alt`, `shift`.
//!
//! **The vim rule.** While the editor is focused and runs vim, the Ctrl+letter
//! chords the vim engine reads (R, C, X, V) are vim's on Linux and Windows,
//! whatever the keymap says. Other Ctrl+letters stay the app's, vim or not
//! (Ctrl+S saves, Ctrl+E opens FILES). Every default bound to a bare
//! Ctrl+letter also gets a Ctrl+Shift twin, which vim never reads, unless
//! another default already holds that Ctrl+Shift chord (Ctrl+Shift+T is the
//! right drawer's toggle, so Ctrl+T has no twin; vim reads no Ctrl+T).
//! On macOS `primary` is Cmd, which vim does not use, so nothing collides.
//! In the plain editor, and with any other panel focused, app chords win.

use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use gpui_kit::component::kbd::Kbd;
use gpui_kit::{App, Context, Global, Subscription};
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::input::{Key, KeyInput, Mods};
use kimun_notes::logic::layout::{Dir, Panel};

/// Which platform's conventions the keymap follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// `primary` is Cmd; Cmd+Option+Arrow moves focus.
    MacOs,
    /// `primary` is Ctrl; Alt+Arrow moves focus.
    LinuxWindows,
}

impl Platform {
    /// The platform this binary was built for.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::LinuxWindows
        }
    }

    fn primary(self) -> Mods {
        match self {
            Platform::MacOs => Mods::SUPER,
            Platform::LinuxWindows => Mods::CTRL,
        }
    }
}

/// Where a key arrives: the focused panel, whether the editor runs vim, and
/// whether the keyboard is where a search query is typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyCtx {
    pub panel: Panel,
    /// The editor's keys go to the vim engine (vim backend, and no find bar
    /// open: the bar is a plain field whatever the backend).
    pub vim: bool,
    /// A query is typed here: FIND has the keyboard, or the search browser
    /// is the open modal. [`Scope::Query`] chords are live only then.
    pub query: bool,
}

/// Where an action's chord is live. Two actions may share a chord only when
/// their scopes are disjoint, or when one is [`Scope::Query`] and the other
/// [`Scope::Everywhere`]: the query chord then shadows the other one where a
/// query is typed (F1 is the query syntax there, the keyboard reference
/// everywhere else).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Whatever panel is focused.
    Everywhere,
    /// Only while the editor is focused (the find bar is the editor's).
    Editor,
    /// Only while the rail or a slot is focused: arranging views and slots,
    /// which acts on the focused slot (or the rail's cell). The editor keeps
    /// these chords for itself.
    Shell,
    /// Only where a query is typed ([`KeyCtx::query`]: FIND with the
    /// keyboard, the search browser), ahead of an `Everywhere` chord.
    Query,
}

impl Scope {
    fn of(action: &ActionShortcuts) -> Scope {
        match action {
            ActionShortcuts::FindInBuffer
            | ActionShortcuts::ReplaceInBuffer
            | ActionShortcuts::FollowLink
            | ActionShortcuts::Text(_) => Scope::Editor,
            ActionShortcuts::MoveViewToNeighbour(_)
            | ActionShortcuts::SplitSlotRight
            | ActionShortcuts::SplitSlotDown
            | ActionShortcuts::CloseSlot
            | ActionShortcuts::YankRow
            | ActionShortcuts::SaveCurrentQuery
            | ActionShortcuts::OpenSortDialog => Scope::Shell,
            ActionShortcuts::QuerySyntax => Scope::Query,
            _ => Scope::Everywhere,
        }
    }

    fn overlaps(self, other: Scope) -> bool {
        match (self, other) {
            // The query chord shadows, see the type's docs.
            (Scope::Query, Scope::Everywhere) | (Scope::Everywhere, Scope::Query) => false,
            (Scope::Everywhere, _) | (_, Scope::Everywhere) => true,
            // A query is typed in a drawer (FIND) or over any panel (the
            // browser): it meets both of the panel scopes.
            (Scope::Query, _) | (_, Scope::Query) => true,
            (a, b) => a == b,
        }
    }

    fn applies(self, ctx: KeyCtx) -> bool {
        match self {
            Scope::Everywhere => true,
            Scope::Editor => ctx.panel == Panel::Editor,
            Scope::Shell => ctx.panel != Panel::Editor,
            Scope::Query => ctx.query,
        }
    }
}

/// The default chords for `action`, written with `primary`; `None` marks an
/// action the GUI deliberately ships unbound (its surface is reached another
/// way: the leader, the palette, the rail). The match has no wildcard: a
/// new shared action does not compile until it is given chords or marked.
fn default_chords(action: &ActionShortcuts, platform: Platform) -> Option<&'static [&'static str]> {
    use ActionShortcuts as A;
    let mac = platform == Platform::MacOs;
    Some(match action {
        A::FindInBuffer => &["primary+f"],
        // Cmd+H hides the app on macOS; Cmd+Option+F is the platform's
        // find-and-replace. Elsewhere Ctrl+H stays where M4 put it.
        A::ReplaceInBuffer if mac => &["cmd+alt+f"],
        A::ReplaceInBuffer => &["ctrl+h"],
        // The TUI's drawer toggle, kept for muscle memory.
        A::ToggleSidebar => &["primary+t"],
        // Directional focus (ADR-0050 §4). Not Ctrl+Arrow (word motion) and
        // not Ctrl+Alt+Arrow (GNOME's workspace switch).
        A::FocusRegion(dir) => match (mac, dir) {
            (true, Dir::Left) => &["cmd+alt+left"],
            (true, Dir::Right) => &["cmd+alt+right"],
            (true, Dir::Up) => &["cmd+alt+up"],
            (true, Dir::Down) => &["cmd+alt+down"],
            (false, Dir::Left) => &["alt+left"],
            (false, Dir::Right) => &["alt+right"],
            (false, Dir::Up) => &["alt+up"],
            (false, Dir::Down) => &["alt+down"],
        },
        // The wrapping panel cycle (the TUI's Tab / Shift+Tab, whose Tab the
        // editor keeps for indent). Literal Ctrl on every platform: Cmd+Tab
        // is macOS's app switcher.
        A::FocusEditor => &["ctrl+tab"],
        A::FocusSidebar => &["ctrl+shift+tab"],
        // Write the open note now (the TUI's leader `w`). Everywhere: the
        // note is the window's, whichever panel has the keyboard.
        A::SaveNote => &["primary+s"],
        // Show FILES on the open note's folder and focus it (the TUI's
        // Ctrl+E). Everywhere, as in the TUI.
        A::OpenFileBrowser => &["primary+e"],
        // Follow the link under the caret: Ctrl+Enter (the TUI's kitty chord,
        // which a window always receives). Editor only; the vim engine reads
        // no Ctrl+Enter. The TUI's Ctrl+N is not a GUI default (a user can
        // still bind it under `[gui.keys]`). Never Shift+Enter: in the editor
        // that is a plain newline.
        A::FollowLink => &["primary+enter"],
        // The right side's toggle: the left toggle plus Shift. It takes
        // the Ctrl+Shift+T that would be Ctrl+T's vim twin (no twin is made
        // for a chord another default holds); Ctrl+T needs none, as vim
        // reads no Ctrl+T.
        A::ToggleRightDrawer => &["primary+shift+t"],
        // Arranging the layout tree (ADR-0052), only from the rail or a
        // slot, so the editor keeps these chords (Option+Shift+Arrow and
        // Alt+Shift+letter are text keys there). The directional-focus
        // chord plus Shift carries the focused view to the neighbouring
        // slot that way. Splitting and closing are the same modifiers with
        // a letter: R splits right, D splits down, W closes (the desktop's
        // close). Not Ctrl+letter, so the vim rule never meets them.
        A::MoveViewToNeighbour(dir) => match (mac, dir) {
            (true, Dir::Left) => &["cmd+alt+shift+left"],
            (true, Dir::Right) => &["cmd+alt+shift+right"],
            (true, Dir::Up) => &["cmd+alt+shift+up"],
            (true, Dir::Down) => &["cmd+alt+shift+down"],
            (false, Dir::Left) => &["alt+shift+left"],
            (false, Dir::Right) => &["alt+shift+right"],
            (false, Dir::Up) => &["alt+shift+up"],
            (false, Dir::Down) => &["alt+shift+down"],
        },
        A::SplitSlotRight if mac => &["cmd+alt+shift+r"],
        A::SplitSlotRight => &["alt+shift+r"],
        A::SplitSlotDown if mac => &["cmd+alt+shift+d"],
        A::SplitSlotDown => &["alt+shift+d"],
        A::CloseSlot if mac => &["cmd+alt+shift+w"],
        A::CloseSlot => &["alt+shift+w"],
        // Zoom (ADR-0053): the focused view or editor group takes over
        // everything right of the rail, and the same chord restores; Esc
        // never does. M for maximize. Everywhere, so the editor can zoom
        // too: Ctrl+Shift+letter is no vim chord (no twin needed) and
        // Cmd+Shift+M is no macOS default.
        A::ToggleZoom => &["primary+shift+m"],
        // The command palette (M6): VS Code's chord. Primary+P stays free for
        // a GUI quick-open convention; under vim Ctrl+Shift+P is no vim chord.
        A::OpenCommandPalette => &["primary+shift+p"],
        // The leader gateway (M6): the TUI's Ctrl+B plus Shift, because
        // Cmd/Ctrl+B is "Bold" in desktop editors. Not a bare Ctrl+letter, so
        // no twin; vim reads no Ctrl+Shift+B. Under vim the editor's Normal
        // mode also arms it with a bare Space (the workspace's, not a chord).
        A::Leader => &["primary+shift+b"],
        // Close the window through its close guard (the open note is flushed
        // first). Cmd+Q is the platform's quit on macOS; vim reads no Ctrl+Q.
        A::Quit => &["primary+q"],
        // The note menu (rename, move, delete) for the open note: the TUI's
        // F2. Everywhere, as the note is the window's.
        A::FileOperations => &["f2"],
        // The note finder (M6 G4): the TUI's Ctrl+O, the desktop's "open".
        // Everywhere; vim reads no Ctrl+O here, and it gets its twin.
        A::OpenNote => &["primary+o"],
        // Today's journal entry (M6 G4): the TUI's Ctrl+J. Everywhere; vim
        // reads no Ctrl+J, and it gets its twin.
        A::NewJournal => &["primary+j"],
        // The quick note (M6 G4): the TUI's Ctrl+W plus Shift, because
        // Cmd/Ctrl+W closes a tab or window on the desktop. Not a bare
        // Ctrl+letter, so no twin; vim reads no Ctrl+Shift+W.
        A::QuickNote => &["primary+shift+w"],
        // Copy the selected drawer row (a note's path, a heading's text;
        // M7a G1): the desktop's copy, only from the rail or a drawer, so
        // the editor keeps it. Where no list row is selected (the FILES
        // filter) the chord passes on and copies the field's text.
        A::YankRow => &["primary+c"],
        // The search browser (M7b G4): the TUI's Ctrl+K. Everywhere; vim
        // reads no Ctrl+K (it is not one of R, C, X, V), and it gets its
        // twin.
        A::SearchNotes => &["primary+k"],
        // The saved searches (M7b G6): the TUI's F3. Everywhere.
        A::OpenSavedSearches => &["f3"],
        // Save the query (M7b G6): the TUI's Ctrl+D, only from the rail or
        // a drawer (FIND), so the editor keeps it whatever it means there
        // (vim's half-page down); the search browser takes it too while it
        // is the open modal (the workspace checks [`Keymap::is_bound`]).
        A::SaveCurrentQuery => &["primary+d"],
        // The sort dialog (M7b G7): the TUI's Ctrl+R, only from the rail or
        // a drawer (FILES, FIND), never the editor, where Ctrl+R is vim's
        // redo; the search browser takes it too while it is the open modal
        // (the workspace checks [`Keymap::is_bound`]).
        A::OpenSortDialog => &["primary+r"],
        // The Ask Thread, its composer focused (M7c G10a): the TUI's F6.
        // Everywhere, as in the TUI; no editor or vim key is F6.
        A::OpenAsk => &["f6"],
        // The workspace switcher (M8d): the TUI's F5. Everywhere, as in the
        // TUI; no editor or vim key is F5.
        A::SwitchWorkspace => &["f5"],
        // Preferences (M9b): the TUI's F4 and Ctrl+, (Cmd+, on macOS, the
        // platform's settings chord). Everywhere; no editor or vim key is F4,
        // and vim reads no Ctrl+, (no letter: no twin).
        A::OpenPreferences => &["f4", "primary+,"],
        // The keyboard reference (M10d): the TUI's F1 help, the desktop's
        // help key. Everywhere, on both platforms; no editor or vim key is
        // F1. Where a query is typed F1 is the query syntax instead
        // (`QuerySyntax`, whose scope shadows this chord there).
        A::KeyboardHelp => &["f1"],
        // The query syntax, where a query is typed (FIND with the keyboard,
        // the search browser): F1 there, as in the TUI's search views.
        A::QuerySyntax => &["f1"],
        // The TUI's FIND toggle: here FIND is a drawer view, shown from the
        // rail, the leader (`o q`) and the palette. Nothing acts on it.
        A::ToggleQueryPanel => return None,
        // Formatting lives on the palette and leader, as in the TUI.
        A::Text(_) => return None,
    })
}

/// Why a written chord was not understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordError {
    Empty,
    UnknownModifier(String),
    UnknownKey(String),
    /// A bare character (or Shift+character) types text; it cannot be a
    /// command.
    NeedsModifier,
}

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChordError::Empty => write!(f, "empty chord"),
            ChordError::UnknownModifier(m) => write!(
                f,
                "unknown modifier `{m}` (use primary, cmd, ctrl, alt, shift)"
            ),
            ChordError::UnknownKey(k) => write!(f, "unknown key `{k}`"),
            ChordError::NeedsModifier => {
                write!(f, "a character key needs Ctrl, Alt, Cmd or primary")
            }
        }
    }
}

/// An override the loader rejected. Reported, never fatal: the rest of the
/// keymap loads (ADR-0050 §6). Logged at startup; the F1 reference shows the
/// keymap that did load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeymapError {
    /// `[gui.keys]` names no action.
    UnknownAction { name: String },
    /// A chord did not parse; the action's other chords still load.
    InvalidChord {
        action: ActionShortcuts,
        chord: String,
        reason: ChordError,
    },
    /// The chord already belongs to another action in a context both are
    /// live in; the override's chord is dropped.
    Collision {
        action: ActionShortcuts,
        chord: String,
        taken_by: ActionShortcuts,
    },
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeymapError::UnknownAction { name } => write!(f, "[gui.keys] unknown action `{name}`"),
            KeymapError::InvalidChord {
                action,
                chord,
                reason,
            } => write!(f, "[gui.keys] {action} = \"{chord}\": {reason}"),
            KeymapError::Collision {
                action,
                chord,
                taken_by,
            } => write!(
                f,
                "[gui.keys] {action} = \"{chord}\": already bound to {taken_by}"
            ),
        }
    }
}

/// Parse a written chord for `platform` (see the module docs for the form).
pub fn parse_chord(written: &str, platform: Platform) -> Result<KeyInput, ChordError> {
    let lower = written.trim().to_lowercase();
    if lower.is_empty() {
        return Err(ChordError::Empty);
    }
    let mut parts: Vec<&str> = lower.split('+').map(str::trim).collect();
    let key_name = parts
        .pop()
        .filter(|k| !k.is_empty())
        .ok_or(ChordError::Empty)?;
    let mut mods = Mods::NONE;
    for m in parts {
        let add = match m {
            "primary" => platform.primary(),
            "cmd" => Mods::SUPER,
            "ctrl" => Mods::CTRL,
            "alt" => Mods::ALT,
            "shift" => Mods::SHIFT,
            other => return Err(ChordError::UnknownModifier(other.to_string())),
        };
        mods.ctrl |= add.ctrl;
        mods.alt |= add.alt;
        mods.shift |= add.shift;
        mods.sup |= add.sup;
    }
    let key = match key_name {
        "enter" => Key::Enter,
        "escape" | "esc" => Key::Esc,
        // As the input edge reports it: Shift+Tab arrives as BackTab.
        "tab" if mods.shift => Key::BackTab,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "insert" => Key::Insert,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "space" => Key::Char(' '),
        name => {
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Char(c),
                _ => match name.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
                    Some(n) if (1..=35).contains(&n) => Key::F(n),
                    _ => return Err(ChordError::UnknownKey(name.to_string())),
                },
            }
        }
    };
    if matches!(key, Key::Char(_)) && !(mods.ctrl || mods.alt || mods.sup) {
        return Err(ChordError::NeedsModifier);
    }
    Ok(KeyInput::new(key, mods))
}

/// One chord bound to one action.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding {
    chord: KeyInput,
    action: ActionShortcuts,
}

/// The GUI's chord table for one platform.
#[derive(Debug, Clone, PartialEq)]
pub struct Keymap {
    platform: Platform,
    bindings: Vec<Binding>,
}

impl Keymap {
    /// The default table for `platform` (tests; the app loads with its
    /// overrides through [`Keymap::load`]).
    #[cfg(test)]
    pub fn defaults(platform: Platform) -> Self {
        let (keymap, errors) = Self::load(platform, &BTreeMap::new());
        debug_assert!(errors.is_empty(), "default keymap: {errors:?}");
        keymap
    }

    /// The defaults with the `[gui.keys]` overrides applied, plus what was
    /// rejected.
    ///
    /// An override replaces all of that action's default chords (an empty
    /// list unbinds it) and is taken literally: no vim twin is added. Overrides
    /// apply in action order; a chord already held by another action in an
    /// overlapping context is rejected, so a default is never displaced by a
    /// clash — rebind the holder too.
    pub fn load(
        platform: Platform,
        overrides: &BTreeMap<String, Vec<String>>,
    ) -> (Self, Vec<KeymapError>) {
        let mut errors = Vec::new();
        let mut overridden: BTreeMap<ActionShortcuts, Vec<String>> = BTreeMap::new();
        for (name, chords) in overrides {
            match ActionShortcuts::try_from(name.clone()) {
                Ok(action) => overridden
                    .entry(action)
                    .or_default()
                    .extend(chords.iter().cloned()),
                Err(_) => errors.push(KeymapError::UnknownAction { name: name.clone() }),
            }
        }
        let mut keymap = Keymap {
            platform,
            bindings: Vec::new(),
        };
        // Every default chord in effect (overridden actions' are gone), so a
        // vim twin can yield to a default that holds it.
        let defaults: Vec<(ActionShortcuts, &str, KeyInput)> = ActionShortcuts::ALL
            .into_iter()
            .filter(|action| !overridden.contains_key(action))
            .flat_map(|action| {
                default_chords(&action, platform)
                    .unwrap_or_default()
                    .iter()
                    .map(move |written| {
                        let chord = parse_chord(written, platform)
                            .unwrap_or_else(|e| panic!("default chord {written:?}: {e}"));
                        (action.clone(), *written, chord)
                    })
            })
            .collect();
        for (action, written, chord) in &defaults {
            keymap.bind(action, written, *chord, &mut errors);
            // The vim rule's twin: Ctrl+Shift for a bare Ctrl+letter, unless
            // another action's default holds that chord.
            if platform == Platform::LinuxWindows && is_bare_ctrl_letter(*chord) {
                let twin = KeyInput::new(
                    chord.key,
                    Mods {
                        shift: true,
                        ..chord.mods
                    },
                );
                let scope = Scope::of(action);
                let taken = defaults.iter().any(|(other, _, c)| {
                    *c == twin && other != action && Scope::of(other).overlaps(scope)
                });
                if !taken {
                    keymap.bind(action, written, twin, &mut errors);
                }
            }
        }
        for (action, chords) in overridden {
            for written in chords {
                match parse_chord(&written, platform) {
                    Ok(chord) => keymap.bind(&action, &written, chord, &mut errors),
                    Err(reason) => errors.push(KeymapError::InvalidChord {
                        action: action.clone(),
                        chord: written,
                        reason,
                    }),
                }
            }
        }
        (keymap, errors)
    }

    /// Bind `chord` to `action` unless another action holds it in an
    /// overlapping context (reported). The same pair twice is one binding.
    fn bind(
        &mut self,
        action: &ActionShortcuts,
        written: &str,
        chord: KeyInput,
        errors: &mut Vec<KeymapError>,
    ) {
        let scope = Scope::of(action);
        if let Some(holder) = self
            .bindings
            .iter()
            .find(|b| b.chord == chord && Scope::of(&b.action).overlaps(scope))
        {
            if holder.action != *action {
                errors.push(KeymapError::Collision {
                    action: action.clone(),
                    chord: written.to_string(),
                    taken_by: holder.action.clone(),
                });
            }
            return;
        }
        self.bindings.push(Binding {
            chord,
            action: action.clone(),
        });
    }

    /// The chord `action` is bound to first (its written default, before any
    /// vim twin), for a hint next to the command. `None` when unbound.
    pub fn chord_for(&self, action: &ActionShortcuts) -> Option<KeyInput> {
        self.bindings
            .iter()
            .find(|b| b.action == *action)
            .map(|b| b.chord)
    }

    /// Every chord `action` is bound to, in binding order (its first is
    /// [`Self::chord_for`]'s). Empty when unbound.
    pub fn chords_for(&self, action: &ActionShortcuts) -> Vec<KeyInput> {
        self.bindings
            .iter()
            .filter(|b| b.action == *action)
            .map(|b| b.chord)
            .collect()
    }

    /// Whether `chord` is one of `action`'s, wherever it is live: for a
    /// modal that takes a scoped chord whatever panel opened it.
    pub fn is_bound(&self, chord: KeyInput, action: &ActionShortcuts) -> bool {
        self.bindings
            .iter()
            .any(|b| b.chord == chord && b.action == *action)
    }

    /// Whether the desktop app ships `action` with a chord on this
    /// platform: an action it could bind, so one without chords was unbound
    /// by a `[gui.keys]` override.
    pub fn ships_bound(&self, action: &ActionShortcuts) -> bool {
        default_chords(action, self.platform).is_some()
    }

    /// Whether the vim rule takes `chord` from `action` while the vim editor
    /// has the keyboard: a Ctrl+letter the engine reads, on Linux and
    /// Windows, for an action live in the editor. Elsewhere the chord stays
    /// the action's (a keyboard reference says so next to it).
    pub fn vim_shadows(&self, action: &ActionShortcuts, chord: KeyInput) -> bool {
        self.platform == Platform::LinuxWindows
            && is_vim_ctrl_chord(chord)
            && Scope::of(action).applies(KeyCtx {
                panel: Panel::Editor,
                vim: true,
                query: false,
            })
    }

    /// The action `chord` triggers in `ctx`, if any: a [`Scope::Query`]
    /// binding first where a query is typed. `None` also when the vim rule
    /// gives the chord to the editor.
    pub fn resolve(&self, chord: KeyInput, ctx: KeyCtx) -> Option<&ActionShortcuts> {
        if self.platform == Platform::LinuxWindows
            && ctx.vim
            && ctx.panel == Panel::Editor
            && is_vim_ctrl_chord(chord)
        {
            return None;
        }
        let live = |b: &&Binding| b.chord == chord && Scope::of(&b.action).applies(ctx);
        self.bindings
            .iter()
            .filter(live)
            .find(|b| Scope::of(&b.action) == Scope::Query)
            .or_else(|| self.bindings.iter().find(live))
            .map(|b| &b.action)
    }
}

/// Ctrl and a letter, nothing else held.
fn is_bare_ctrl_letter(chord: KeyInput) -> bool {
    chord.mods == Mods::CTRL && matches!(chord.key, Key::Char(c) if c.is_ascii_alphabetic())
}

/// The Ctrl+letter chords the vim engine reads: redo and the clipboard
/// chords. Every other Ctrl+letter means nothing to the engine, so the app
/// keeps it (Ctrl+S saves, Ctrl+E opens FILES) even in vim mode.
fn is_vim_ctrl_chord(chord: KeyInput) -> bool {
    chord.mods == Mods::CTRL && matches!(chord.key, Key::Char('r' | 'c' | 'x' | 'v'))
}

/// The keymap in effect, shared by every view that resolves keys.
struct ActiveKeymap(Rc<Keymap>);

impl Global for ActiveKeymap {}

/// Make `keymap` the one in effect. Call once at startup, after
/// `gpui_kit::init`; again to replace it.
pub fn init(cx: &mut App, keymap: Keymap) {
    cx.set_global(ActiveKeymap(Rc::new(keymap)));
}

/// The keymap in effect, the one [`init`] installed.
///
/// # Panics
/// When [`init`] was never called. A silent fallback would resolve every key
/// against a fresh default table and drop the user's `[gui.keys]`; a view
/// built outside the app (a test) installs one explicitly.
pub fn active(cx: &App) -> Rc<Keymap> {
    cx.try_global::<ActiveKeymap>()
        .map(|active| active.0.clone())
        .expect("keymap::init must run before any view resolves a key")
}

/// `key` written the platform's way (`Ctrl+Shift+B`, `⌘⇧B`).
pub(crate) fn format_chord(key: KeyInput) -> String {
    Kbd::format(&crate::input::keystroke(key))
}

/// The chord the active keymap binds `action` to, written the platform's
/// way (`Ctrl+Shift+B`, `⌘⇧B`), for a hint. `None` when unbound.
pub(crate) fn chord_label(cx: &App, action: &ActionShortcuts) -> Option<String> {
    active(cx).chord_for(action).map(format_chord)
}

/// Every chord the active keymap binds `action` to, each written the
/// platform's way, for a summary (the CFG view). Empty when unbound.
pub(crate) fn chord_labels(cx: &App, action: &ActionShortcuts) -> Vec<String> {
    active(cx)
        .chords_for(action)
        .into_iter()
        .map(format_chord)
        .collect()
}

/// Call `f` on `view` whenever [`init`] replaces the active keymap, so a
/// view can rebuild what it derived from the chords.
pub(crate) fn observe<T: 'static>(
    cx: &mut Context<T>,
    mut f: impl FnMut(&mut T, &mut Context<T>) + 'static,
) -> Subscription {
    cx.observe_global::<ActiveKeymap>(move |view, cx| f(view, cx))
}

#[cfg(test)]
mod tests;
