//! The Keyboard section's draft (ADR-0064): one chord per action, edited on
//! top of the [`Keymap`] the `[gui.keys]` overrides load, and the rules for
//! a captured chord. Pure data, headless-testable, like the keymap.
//!
//! **An edit takes the chord.** [`KeyDraft::assign`](crate::keymap::draft::KeyDraft::assign) takes the chord from
//! every other action live in an overlapping [`Scope`]. The holder keeps its
//! other chords; a holder left with none is saved as `Action = []`, so its
//! default does not come back at the next load and undo the steal. The
//! loader keeps its own rule (a chord stays with the action that holds it,
//! defaults first, then overrides in action order): a file is never read
//! this way.
//!
//! **Overrides are literal.** A changed action is written with exactly the
//! chords it keeps: a holder loses the vim twin of the chord it gave up, and
//! an assigned chord gets no twin. The draft keeps an override only while it
//! changes the keymap, so an action set back to its default leaves no entry.
//!
//! **Unknown actions pass through.** An override for an action this build
//! does not know (a typo, or an action of a newer build) is no row, but
//! [`KeyDraft::overrides`](crate::keymap::draft::KeyDraft::overrides) hands it back as it came: an edit, Reset all or
//! an import never deletes a line the person cannot see in the section.
//!
//! **What is bindable.** [`bindable_groups`](crate::keymap::draft::bindable_groups) lists every action the GUI acts
//! on when its chord resolves, the ones with no default chord too (the facet
//! view and the collection commands). It leaves out the actions nothing acts
//! on: the TUI's FIND toggle and its note-or-directory menu, and the text
//! formatting actions, which the palette and the leader reach.

use std::collections::BTreeMap;
use std::fmt;

use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::editor::plain_keys;
use kimun_notes::logic::input::{Key, KeyInput};
use kimun_notes::logic::layout::Dir;

use super::{Keymap, Platform, Scope, default_chords, parse_chord};

/// The Keyboard section's changes, not yet saved: overrides as written, and
/// the keymap they load to.
#[derive(Debug, Clone)]
pub(crate) struct KeyDraft {
    platform: Platform,
    /// The changed actions and their chords as written (`primary+e`). Only
    /// entries that change the keymap: [`Self::settle`] drops the rest.
    entries: BTreeMap<ActionShortcuts, Vec<String>>,
    /// The keymap with [`Self::entries`] applied.
    keymap: Keymap,
    /// The keymap with no overrides, for [`Self::is_changed`].
    defaults: Keymap,
    /// The overrides for action names this build does not know, as
    /// written: no row, handed back by [`Self::overrides`] untouched.
    unknown: BTreeMap<String, Vec<String>>,
}

impl KeyDraft {
    /// A draft of `overrides` (the `[gui.keys]` table) for `platform`.
    /// Unknown action names are kept aside (see the module docs); the
    /// loader reports them.
    pub(crate) fn new(platform: Platform, overrides: &BTreeMap<String, Vec<String>>) -> Self {
        let mut draft = Self {
            platform,
            entries: BTreeMap::new(),
            keymap: load(platform, &BTreeMap::new()),
            defaults: load(platform, &BTreeMap::new()),
            unknown: BTreeMap::new(),
        };
        draft.fill(overrides);
        draft
    }

    /// Every row as `overrides` (an imported keymap file) has it: what the
    /// draft had goes, except the unknown actions it carries. Unknown
    /// names in `overrides` are left out: the import reports them.
    ///
    /// A file line takes its chord as an interactive edit does: from the
    /// action that holds it by default or in an earlier line, in action
    /// order. Each take, as the action that took it, the chord and the
    /// holder it came from.
    pub(crate) fn replace(
        &mut self,
        overrides: &BTreeMap<String, Vec<String>>,
    ) -> Vec<(ActionShortcuts, KeyInput, ActionShortcuts)> {
        let unknown = std::mem::take(&mut self.unknown);
        self.fill(overrides);
        self.unknown = unknown;
        let mut taken = Vec::new();
        let lines: Vec<(ActionShortcuts, Vec<String>)> = self
            .entries
            .iter()
            .map(|(action, chords)| (action.clone(), chords.clone()))
            .collect();
        for (action, chords) in lines {
            for written in chords {
                let Ok(chord) = parse_chord(&written, self.platform) else {
                    continue;
                };
                for holder in self.take(&action, chord) {
                    taken.push((action.clone(), chord, holder));
                }
            }
        }
        self.settle();
        taken
    }

    fn fill(&mut self, overrides: &BTreeMap<String, Vec<String>>) {
        self.entries.clear();
        self.unknown.clear();
        for (name, chords) in overrides {
            match ActionShortcuts::try_from(name.clone()) {
                Ok(action) => self
                    .entries
                    .entry(action)
                    .or_default()
                    .extend(chords.iter().cloned()),
                Err(_) => {
                    self.unknown.insert(name.clone(), chords.clone());
                }
            }
        }
        self.settle();
    }

    /// Whether any row differs from its default (unknown actions aside).
    pub(crate) fn has_changes(&self) -> bool {
        !self.entries.is_empty()
    }

    /// How many actions the draft changes (unknown actions aside).
    pub(crate) fn changes(&self) -> usize {
        self.entries.len()
    }

    /// The chords `action` has with the draft applied, in binding order.
    pub(crate) fn chords(&self, action: &ActionShortcuts) -> Vec<KeyInput> {
        self.keymap.chords_for(action)
    }

    /// The other actions that hold `chord` where `action` would be live (an
    /// overlapping scope): the ones [`Self::assign`] takes it from. Empty
    /// when `action` holds it already, as `assign` then takes nothing.
    pub(crate) fn holders(
        &self,
        action: &ActionShortcuts,
        chord: KeyInput,
    ) -> Vec<ActionShortcuts> {
        if self.chords(action).contains(&chord) {
            return Vec::new();
        }
        // As if `action` had no chord: its own defaults do not hide a twin
        // another action holds once `action` is overridden.
        let mut unbound = self.entries.clone();
        unbound.insert(action.clone(), Vec::new());
        holders_in(&load(self.platform, &unbound), action, chord)
    }

    /// Bind `chord` to `action`, replacing all of its chords, and take it
    /// from every holder (see the module docs). The holders it was taken
    /// from; empty, and nothing changed, when `action` already holds it.
    pub(crate) fn assign(
        &mut self,
        action: &ActionShortcuts,
        chord: KeyInput,
    ) -> Vec<ActionShortcuts> {
        if self.chords(action).contains(&chord) {
            return Vec::new();
        }
        self.entries
            .insert(action.clone(), vec![write_chord(chord, self.platform)]);
        let holders = self.take(action, chord);
        self.settle();
        holders
    }

    /// Leave `action` with no chord. An action that ships with none is then
    /// unchanged.
    pub(crate) fn clear(&mut self, action: &ActionShortcuts) {
        self.entries.insert(action.clone(), Vec::new());
        self.settle();
    }

    /// Give `action` its default chords back, its vim twins included, and
    /// take them from any holder, as [`Self::assign`] does. The holders.
    pub(crate) fn reset(&mut self, action: &ActionShortcuts) -> Vec<ActionShortcuts> {
        self.entries.remove(action);
        let restored = load(self.platform, &self.entries).chords_for(action);
        let mut holders: Vec<ActionShortcuts> = Vec::new();
        for chord in restored {
            for holder in self.take(action, chord) {
                if !holders.contains(&holder) {
                    holders.push(holder);
                }
            }
        }
        self.settle();
        holders
    }

    /// Forget every change: the defaults.
    pub(crate) fn reset_all(&mut self) {
        self.entries.clear();
        self.settle();
    }

    /// Whether the draft changed `action`: it has an override, and its
    /// chords differ from its default. An action pinned to its default
    /// chords (see [`Self::take`]) is not changed. Neither is one that got
    /// its vim twin back because another action was overridden: its row
    /// was not touched, and a reset there does nothing.
    pub(crate) fn is_changed(&self, action: &ActionShortcuts) -> bool {
        self.entries.contains_key(action) && self.chords(action) != self.defaults.chords_for(action)
    }

    /// Whether the vim rule takes `chord` from `action` while the vim editor
    /// has the keyboard ([`Keymap::vim_shadows`]).
    pub(crate) fn vim_shadows(&self, action: &ActionShortcuts, chord: KeyInput) -> bool {
        self.keymap.vim_shadows(action, chord)
    }

    /// The other actions that keep `chord` beside `action`, because one of
    /// the two is live only where a query is typed and the other everywhere
    /// (see [`Scope`]): no steal, the scoped one wins there. Each once.
    pub(crate) fn shared(&self, action: &ActionShortcuts, chord: KeyInput) -> Vec<Shared> {
        let scope = Scope::of(action);
        let mut shared: Vec<Shared> = Vec::new();
        for binding in &self.keymap.bindings {
            let other = Scope::of(&binding.action);
            if binding.chord != chord
                || binding.action == *action
                || shared.iter().any(|s| s.other == binding.action)
            {
                continue;
            }
            let other_wins_in = match (scope, other) {
                (Scope::Everywhere, Scope::Query) => Some("Where a query is typed"),
                (Scope::Everywhere, Scope::Browser) => Some("In the search browser"),
                (Scope::Query | Scope::Browser, Scope::Everywhere) => None,
                _ => continue,
            };
            shared.push(Shared {
                other: binding.action.clone(),
                other_wins_in,
            });
        }
        shared
    }

    /// The draft as `[gui.keys]` overrides: only the actions that differ
    /// from their default, a holder left with no chord as an empty list,
    /// and the unknown actions as they came.
    pub(crate) fn overrides(&self) -> BTreeMap<String, Vec<String>> {
        let mut out = self.unknown.clone();
        out.extend(names(&self.entries));
        out
    }

    /// Every bindable action with the chords it holds now, written as the
    /// keymap file reads them, defaults included. An unbound action is an
    /// empty list. What the file kept for names this build does not know
    /// stays. This is what Export writes, so an old file pins every
    /// action when it is imported later.
    pub(crate) fn export_all(&self) -> BTreeMap<String, Vec<String>> {
        let mut out = self.unknown.clone();
        for group in bindable_groups() {
            for row in group.rows {
                let chords = self
                    .chords(&row.action)
                    .into_iter()
                    .map(|chord| write_chord(chord, self.platform))
                    .collect();
                out.insert(row.action.to_string(), chords);
            }
        }
        out
    }

    /// Take `chord` from every other action live where `action` is, once
    /// `action`'s own entry is in place. The holders it was taken from.
    ///
    /// A holder is written with the chords it keeps, so it loses the chord
    /// and, being literal now, the vim twin of it. Writing an override can
    /// free a twin elsewhere (the twin of `primary+f` yields to the search's
    /// `primary+shift+f` only while the search runs on defaults): the loop
    /// pins such an action to its written chords too, without the twin. It
    /// keeps its chords, so it is no holder.
    fn take(&mut self, action: &ActionShortcuts, chord: KeyInput) -> Vec<ActionShortcuts> {
        let holders = self.holders(action, chord);
        for holder in &holders {
            self.drop_chord(holder, chord);
        }
        loop {
            let keymap = load(self.platform, &self.entries);
            let others = holders_in(&keymap, action, chord);
            if others.is_empty() {
                break;
            }
            for other in &others {
                self.drop_chord(other, chord);
            }
        }
        holders
    }

    /// Write `action`'s chords (its entry, or its written defaults) without
    /// `chord`.
    fn drop_chord(&mut self, action: &ActionShortcuts, chord: KeyInput) {
        let written: Vec<String> = match self.entries.get(action) {
            Some(chords) => chords.clone(),
            None => default_chords(action, self.platform)
                .unwrap_or_default()
                .iter()
                .map(|c| c.to_string())
                .collect(),
        };
        let kept = written
            .into_iter()
            .filter(|w| parse_chord(w, self.platform) != Ok(chord))
            .collect();
        self.entries.insert(action.clone(), kept);
    }

    /// Drop every entry the keymap does not need, then rebuild the keymap.
    /// One at a time against the full keymap, so what is left loads to it.
    fn settle(&mut self) {
        let full = load(self.platform, &self.entries);
        let actions: Vec<ActionShortcuts> = self.entries.keys().cloned().collect();
        for action in actions {
            let mut without = self.entries.clone();
            without.remove(&action);
            if same_chords(&load(self.platform, &without), &full) {
                self.entries = without;
            }
        }
        self.keymap = full;
    }
}

/// Another action that has a chord too, live in a scope that meets the
/// first one's without overlapping it ([`KeyDraft::shared`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shared {
    pub(crate) other: ActionShortcuts,
    /// Where `other` wins, as a sentence opens ("Where a query is typed"),
    /// when `other` is the scoped one. `None`: the first action wins
    /// there, and `other` has the chord everywhere else.
    pub(crate) other_wins_in: Option<&'static str>,
}

/// `entries` with the on-disk action names.
fn names(entries: &BTreeMap<ActionShortcuts, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(action, chords)| (action.to_string(), chords.clone()))
        .collect()
}

/// The keymap `entries` load to. Their errors are the loader's to report.
fn load(platform: Platform, entries: &BTreeMap<ActionShortcuts, Vec<String>>) -> Keymap {
    Keymap::load(platform, &names(entries)).0
}

/// The actions other than `action` that hold `chord` in `keymap` where
/// `action` is live, each once.
fn holders_in(keymap: &Keymap, action: &ActionShortcuts, chord: KeyInput) -> Vec<ActionShortcuts> {
    let scope = Scope::of(action);
    let mut holders: Vec<ActionShortcuts> = Vec::new();
    for binding in &keymap.bindings {
        if binding.chord == chord
            && binding.action != *action
            && Scope::of(&binding.action).overlaps(scope)
            && !holders.contains(&binding.action)
        {
            holders.push(binding.action.clone());
        }
    }
    holders
}

/// Whether every action has the same chords, in the same order, in both.
fn same_chords(a: &Keymap, b: &Keymap) -> bool {
    ActionShortcuts::ALL
        .iter()
        .all(|action| a.chords_for(action) == b.chords_for(action))
}

/// Why a captured chord cannot be bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureError {
    /// A character key with no Ctrl, Alt or Cmd: it types text.
    NeedsModifier,
    /// Enter, Tab, Backspace, Esc, an arrow or another editing key with no
    /// Ctrl, Alt or Cmd (Shift too: Shift+Enter is a newline, Shift+Tab
    /// moves focus back). The editor and every list need it.
    Reserved,
    /// The `+` key, with any modifiers: `+` joins the parts of a written
    /// chord, so [`parse_chord`] cannot read it back.
    PlusKey,
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureError::NeedsModifier => {
                write!(f, "A character key types text. Add Ctrl, Alt or Cmd.")
            }
            CaptureError::Reserved => write!(
                f,
                "This key edits text or moves around. Add Ctrl, Alt or Cmd."
            ),
            CaptureError::PlusKey => write!(f, "The + key cannot be used in a shortcut."),
        }
    }
}

/// Whether `chord` on `action` takes a key the plain editor edits with. The
/// editor asks the keymap first (`editor_view/keys.rs`), so an action live
/// there (Everywhere or Editor) then runs instead of the edit. The editor's
/// key table ([`plain_keys::operation`]: word moves, select all, word
/// deletes) is asked directly. `Editor::handle_key` claims a few chords
/// before that table, and they are listed here: copy, cut, paste, undo and
/// redo on Ctrl, or on Cmd through its alias (Cmd+A too).
pub(crate) fn takes_editing_key(action: &ActionShortcuts, chord: KeyInput) -> bool {
    let m = chord.mods;
    if !matches!(Scope::of(action), Scope::Everywhere | Scope::Editor)
        || !(m.ctrl || m.alt || m.sup)
    {
        return false;
    }
    if plain_keys::operation(chord).is_some() {
        return true;
    }
    let Key::Char(c) = chord.key else {
        return false;
    };
    let one_primary = !m.alt && m.ctrl != m.sup;
    one_primary
        && matches!(
            (m.shift, c.to_ascii_lowercase()),
            (false, 'a' | 'c' | 'v' | 'x' | 'y' | 'z') | (true, 'z')
        )
}

/// Whether `chord`, pressed in the Keyboard section, can be bound: it needs
/// Ctrl, Alt or Cmd, unless it is a function key. Never the `+` key.
pub(crate) fn check_capture(chord: KeyInput) -> Result<(), CaptureError> {
    if chord.key == Key::Char('+') {
        return Err(CaptureError::PlusKey);
    }
    let mods = chord.mods;
    if mods.ctrl || mods.alt || mods.sup {
        return Ok(());
    }
    match chord.key {
        Key::F(_) => Ok(()),
        Key::Char(_) => Err(CaptureError::NeedsModifier),
        _ => Err(CaptureError::Reserved),
    }
}

/// `chord` written for `platform`, the inverse of [`parse_chord`]: the
/// platform's primary modifier (Cmd on macOS, Ctrl elsewhere) as `primary`,
/// then `cmd` or `ctrl`, `alt`, `shift`, and the key.
pub(crate) fn write_chord(chord: KeyInput, platform: Platform) -> String {
    let mods = chord.mods;
    let primary = platform.primary();
    let mut parts: Vec<String> = Vec::new();
    if (mods.sup && primary.sup) || (mods.ctrl && primary.ctrl) {
        parts.push("primary".into());
    }
    if mods.sup && !primary.sup {
        parts.push("cmd".into());
    }
    if mods.ctrl && !primary.ctrl {
        parts.push("ctrl".into());
    }
    if mods.alt {
        parts.push("alt".into());
    }
    // Shift+Tab arrives as BackTab, which is written with its Shift.
    if mods.shift || chord.key == Key::BackTab {
        parts.push("shift".into());
    }
    parts.push(match chord.key {
        Key::Char(' ') => "space".into(),
        Key::Char(c) => c.to_string(),
        Key::Enter => "enter".into(),
        Key::Esc => "escape".into(),
        Key::Tab | Key::BackTab => "tab".into(),
        Key::Backspace => "backspace".into(),
        Key::Delete => "delete".into(),
        Key::Insert => "insert".into(),
        Key::Left => "left".into(),
        Key::Right => "right".into(),
        Key::Up => "up".into(),
        Key::Down => "down".into(),
        Key::Home => "home".into(),
        Key::End => "end".into(),
        Key::PageUp => "pageup".into(),
        Key::PageDown => "pagedown".into(),
        Key::F(n) => format!("f{n}"),
    });
    parts.join("+")
}

/// One action of the Keyboard section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) action: ActionShortcuts,
    /// What the action does, short enough for "Ctrl+E was {label}."
    pub(crate) label: &'static str,
    /// Where the chord is live, when that is not everywhere.
    pub(crate) scope_tag: Option<&'static str>,
}

/// A heading of the Keyboard section and its rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Group {
    pub(crate) title: &'static str,
    pub(crate) rows: Vec<Row>,
}

/// Every bindable action, grouped for the Keyboard section, in display
/// order (see the module docs for what is left out).
pub(crate) fn bindable_groups() -> Vec<Group> {
    use ActionShortcuts as A;
    let group = |title, rows: &[(ActionShortcuts, &'static str)]| Group {
        title,
        rows: rows
            .iter()
            .map(|(action, label)| Row {
                action: action.clone(),
                label,
                scope_tag: scope_tag(Scope::of(action)),
            })
            .collect(),
    };
    vec![
        group(
            "Notes",
            &[
                (A::OpenNote, "Open note"),
                (A::NewNote, "New note"),
                (A::NewDirectory, "New directory"),
                (A::NewJournal, "Today's journal"),
                (A::QuickNote, "Quick note"),
                (A::SaveNote, "Save note"),
                (A::FileOperations, "Note menu"),
                (A::OpenFileBrowser, "Open files"),
                (A::YankRow, "Copy selected row"),
                (A::OpenSortDialog, "Sort list"),
            ],
        ),
        group(
            "Editor",
            &[
                (A::FindInBuffer, "Find in note"),
                (A::ReplaceInBuffer, "Replace in note"),
                (A::FollowLink, "Follow link"),
                (A::ToggleFacetView, "Toggle facet view"),
            ],
        ),
        group(
            "Layout and views",
            &[
                (A::ToggleSidebar, "Toggle left side"),
                (A::ToggleRightDrawer, "Toggle right side"),
                (A::ToggleRail, "Toggle rail"),
                (A::ToggleZoom, "Toggle zoom"),
                (A::FocusEditor, "Focus editor"),
                (A::FocusSidebar, "Focus sidebar"),
                (A::FocusRegion(Dir::Left), "Focus left"),
                (A::FocusRegion(Dir::Right), "Focus right"),
                (A::FocusRegion(Dir::Up), "Focus up"),
                (A::FocusRegion(Dir::Down), "Focus down"),
                (A::MoveViewToNeighbour(Dir::Left), "Move view left"),
                (A::MoveViewToNeighbour(Dir::Right), "Move view right"),
                (A::MoveViewToNeighbour(Dir::Up), "Move view up"),
                (A::MoveViewToNeighbour(Dir::Down), "Move view down"),
                (A::SplitSlotRight, "Split right"),
                (A::SplitSlotDown, "Split down"),
                (A::CloseSlot, "Close slot or pane"),
            ],
        ),
        group(
            "Tabs",
            &[
                (A::NewTab, "New tab"),
                (A::CloseTab, "Close tab"),
                (A::NextTab, "Next tab"),
                (A::PrevTab, "Previous tab"),
            ],
        ),
        group(
            "Search",
            &[
                (A::SearchNotes, "Search notes"),
                (A::OpenSavedSearches, "Saved searches"),
                (A::SaveCurrentQuery, "Save query"),
                (A::QuerySyntax, "Query syntax"),
                (A::QueryBuilder, "Query builder"),
                (A::OpenQueryInSidebar, "Show query in FIND"),
                (A::OpenQueryAsCollection, "Open results as collection"),
                (A::RefreshCollection, "Refresh collection"),
                (A::OpenAsk, "Ask"),
            ],
        ),
        group(
            "Help and system",
            &[
                (A::KeyboardHelp, "Keyboard shortcuts"),
                (A::OpenCommandPalette, "Command palette"),
                (A::Leader, "Leader"),
                (A::OpenPreferences, "Preferences"),
                (A::SwitchWorkspace, "Switch workspace"),
                (A::Quit, "Quit"),
            ],
        ),
    ]
}

/// The row's note on where a chord of `scope` is live; `None` everywhere.
fn scope_tag(scope: Scope) -> Option<&'static str> {
    match scope {
        Scope::Everywhere => None,
        Scope::Editor => Some("editor only"),
        Scope::Shell => Some("sidebar and rail only"),
        Scope::Query => Some("where a query is typed"),
        Scope::Browser => Some("search browser only"),
    }
}
