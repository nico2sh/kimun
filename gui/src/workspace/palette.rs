//! The **command palette**: every leader leaf, plus the GUI's own layout
//! and editor chords, in one fuzzy-filtered list inside a kit `Dialog`
//! (`Command`, unbordered). A row runs the same thing its leader leaf or its
//! chord runs ([`Workspace::perform_route`], `workspace_command`), never a
//! second implementation.
//!
//! **Rows.** The leaves come from the configured leader tree
//! (`logic::palette::palette_rows`, labelled `+group · leaf`); the GUI-only
//! rows (`+window · toggle right side`, moving views and `move view to…`
//! (the move chooser), splitting and closing slots, directional focus,
//! find and replace, following a link) follow them,
//! then the layout preset rows (`Apply layout preset: <name>` per loaded
//! preset, `Reload layout presets`, `Reset to preset`). A
//! row the window cannot run is shown disabled with the reason in its
//! trailing slot; a runnable one shows there its leader sequence (a leaf,
//! from the gateway that works where the keyboard was: `Space` in vim
//! Normal, the `Leader` chord elsewhere) and its chord, when it has one.
//! Availability is decided when the palette opens, against the panel that
//! had the keyboard then (the opener).
//!
//! **Filtering.** Fuzzy, as in the TUI (`logic::list::fuzzy_rank`): the kit's
//! own filter (a substring match) is off, and the palette hands `Command`
//! only the matching rows, best first.
//!
//! **Pointer.** A click runs a row ([`ClickPolicy::Open`], the kit's
//! click), under the pointing hand; a disabled row is inert and keeps the
//! arrow.
//!
//! **Keys.** Up/Down, Enter and Escape are the kit's: Escape clears a typed
//! query first, then closes. The palette chord closes it too (a toggle).
//! Closing gives the keyboard back to the opener.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::command::{Command, CommandItem, CommandState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::{Disableable as _, IndexPath};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Rgba,
    SharedString, Styled as _, WeakEntity, Window, div,
};
use kimun_notes::keys::leader::LeaderAction;
use kimun_notes::logic::actions::ActionShortcuts;
use kimun_notes::logic::input::KeyInput;
use kimun_notes::logic::layout::{Dir, Panel};
use kimun_notes::logic::layout_tree::RailPosition;
use kimun_notes::logic::list::{ListRow, fuzzy_rank};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::palette::palette_rows;

use super::Workspace;
use super::files::ClickPolicy;
use super::keyboard_help::action_label;
use super::modal::{ModalKind, Slot};
use super::presets::PresetCommand;
use super::route::{
    NEEDS_FIND_FOCUS, NEEDS_LIST_FOCUS, NEEDS_NOTE_FOCUS, NEEDS_SLOT_FOCUS, NO_WORKSPACE,
};
use crate::input;
use crate::keymap;
use crate::theme;

/// The palette's width, in rems.
const WIDTH_REMS: f32 = 36.;

/// What a palette row runs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PaletteRun {
    /// A leader leaf, through its route.
    Leaf(LeaderAction),
    /// One of the GUI's own chords, as if pressed in the opener.
    Chord(ActionShortcuts),
    /// A layout preset command ([`super::presets`]).
    Preset(PresetCommand),
    /// Put the rail at this window edge (ADR-0055 §3).
    RailAt(RailPosition),
    /// "Move view to…" for the focused slot
    /// ([`Workspace::open_move_chooser`]); no chord does it.
    MoveViewTo,
}

/// The move chooser's row: after the move rows, before splitting (its
/// place in [`chord_rows`]).
const MOVE_VIEW_TO: &str = "+window · move view to…";

/// Why the row for the rail's current position does nothing.
const RAIL_ALREADY_HERE: &str = "The rail is already there";

/// One row of the palette.
#[derive(Debug, Clone)]
pub(crate) struct PaletteEntry {
    label: SharedString,
    run: PaletteRun,
    /// The chord that runs the same thing, for the trailing slot.
    hint: Option<KeyInput>,
    /// A leaf's leader sequence from the active gateway (`Space n w`), for
    /// the trailing slot.
    sequence: Option<SharedString>,
    /// Why the row cannot run now; `None` when it can.
    blocked: Option<&'static str>,
}

impl ListRow for PaletteEntry {
    fn match_text(&self) -> Option<&str> {
        Some(&self.label)
    }
}

impl PaletteEntry {
    #[cfg(test)]
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    #[cfg(test)]
    pub(crate) fn run(&self) -> &PaletteRun {
        &self.run
    }

    #[cfg(test)]
    pub(crate) fn blocked(&self) -> Option<&'static str> {
        self.blocked
    }

    #[cfg(test)]
    pub(crate) fn sequence(&self) -> Option<&str> {
        self.sequence.as_deref()
    }

    /// The kit row: the label, then the reason it is disabled, or its leader
    /// sequence (in `quiet`: the chord's key caps lead) and its chord.
    fn item(&self, quiet: Rgba) -> CommandItem {
        let label = self.label.clone();
        let blocked = self.blocked;
        let hint = self.hint;
        let sequence = self.sequence.clone();
        // A click runs a runnable row (the kit's); the pointing hand says
        // so, a disabled row keeps the arrow. The kit's row sets the
        // default arrow itself, so the hand is on the content it lays out.
        let cursor = ClickPolicy::Open.cursor(blocked.is_none());
        CommandItem::new()
            .label(self.label.clone())
            .disabled(blocked.is_some())
            .child(move |_, cx| {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .when_some(cursor, |row, cursor| row.cursor(cursor))
                    .child(div().flex_1().min_w_0().truncate().child(label.clone()))
                    .map(|row| match blocked {
                        Some(reason) => row.child(div().flex_none().text_xs().child(reason)),
                        None => row
                            .when_some(sequence.clone(), |row, keys| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_color(quiet)
                                        .child(keys),
                                )
                            })
                            .when_some(hint, |row, chord| {
                                row.child(Kbd::new(input::keystroke(chord)))
                            }),
                    })
            })
    }
}

/// The GUI's own commands the palette offers next to the leader leaves:
/// what the GUI keymap binds that no leaf reaches (ADR-0050 §6: under vim the
/// palette is the route to every app command).
fn chord_rows() -> Vec<(String, PaletteRun)> {
    use ActionShortcuts as A;
    vec![
        window_row(A::ToggleSidebar),
        window_row(A::ToggleRightDrawer),
        (
            "+window · focus left".into(),
            PaletteRun::Chord(A::FocusRegion(Dir::Left)),
        ),
        (
            "+window · focus right".into(),
            PaletteRun::Chord(A::FocusRegion(Dir::Right)),
        ),
        (
            "+window · focus up".into(),
            PaletteRun::Chord(A::FocusRegion(Dir::Up)),
        ),
        (
            "+window · focus down".into(),
            PaletteRun::Chord(A::FocusRegion(Dir::Down)),
        ),
        window_row(A::MoveViewToNeighbour(Dir::Left)),
        window_row(A::MoveViewToNeighbour(Dir::Right)),
        window_row(A::MoveViewToNeighbour(Dir::Up)),
        window_row(A::MoveViewToNeighbour(Dir::Down)),
        (MOVE_VIEW_TO.into(), PaletteRun::MoveViewTo),
        window_row(A::SplitSlotRight),
        window_row(A::SplitSlotDown),
        (
            "+window · close slot".into(),
            PaletteRun::Chord(A::CloseSlot),
        ),
        (
            "+window · toggle zoom".into(),
            PaletteRun::Chord(A::ToggleZoom),
        ),
        (
            "+window · toggle rail".into(),
            PaletteRun::Chord(A::ToggleRail),
        ),
        (
            "+text · find in note".into(),
            PaletteRun::Chord(A::FindInBuffer),
        ),
        (
            "+text · replace in note".into(),
            PaletteRun::Chord(A::ReplaceInBuffer),
        ),
        (
            "+links · follow link at caret".into(),
            PaletteRun::Chord(A::FollowLink),
        ),
        (
            "+links · copy selected row".into(),
            PaletteRun::Chord(A::YankRow),
        ),
        (
            "+note · quick note…".into(),
            PaletteRun::Chord(A::QuickNote),
        ),
        (
            "+find · save query…".into(),
            PaletteRun::Chord(A::SaveCurrentQuery),
        ),
        // The focused list (FILES or FIND); FILES when none is focused.
        ("+view · sort…".into(), PaletteRun::Chord(A::OpenSortDialog)),
    ]
}

/// Why a command on the focused slot cannot run from the palette: only
/// a slot (a drawer panel) has the keyboard for it.
fn slot_focus_block(opener: Panel) -> Option<&'static str> {
    (!matches!(opener, Panel::Drawer(_))).then_some(NEEDS_SLOT_FOCUS)
}

/// A `+window` palette row for `action`, labelled as the keyboard
/// reference labels it ([`action_label`]), so the two cannot disagree.
fn window_row(action: ActionShortcuts) -> (String, PaletteRun) {
    let label = action_label(&action);
    let mut chars = label.chars();
    let lowered = chars
        .next()
        .map(|c| c.to_lowercase().chain(chars).collect::<String>())
        .unwrap_or_default();
    (format!("+window · {lowered}"), PaletteRun::Chord(action))
}

/// The chord a leaf shares its effect with, for the hint.
fn leaf_chord(action: LeaderAction) -> Option<ActionShortcuts> {
    match action {
        LeaderAction::NoteSave => Some(ActionShortcuts::SaveNote),
        LeaderAction::AppQuit => Some(ActionShortcuts::Quit),
        LeaderAction::OpenDrawer(DrawerView::Files) => Some(ActionShortcuts::OpenFileBrowser),
        LeaderAction::FindFiles => Some(ActionShortcuts::OpenNote),
        LeaderAction::FindGrep | LeaderAction::FindRecent => Some(ActionShortcuts::SearchNotes),
        LeaderAction::FindSaved => Some(ActionShortcuts::OpenSavedSearches),
        LeaderAction::NoteDaily => Some(ActionShortcuts::NewJournal),
        LeaderAction::AskFocus => Some(ActionShortcuts::OpenAsk),
        LeaderAction::VaultSwitch => Some(ActionShortcuts::SwitchWorkspace),
        LeaderAction::VaultPreferences => Some(ActionShortcuts::OpenPreferences),
        LeaderAction::Help => Some(ActionShortcuts::KeyboardHelp),
        _ => None,
    }
}

/// The palette's view: the kit palette's state, every row, and the rows the
/// query keeps.
pub(crate) struct Palette {
    state: Entity<CommandState>,
    entries: Vec<PaletteEntry>,
    /// Indices into `entries` matching the query, best first.
    visible: Vec<usize>,
    workspace: WeakEntity<Workspace>,
    /// The leader sequences' colour, one for every row.
    quiet: Rgba,
    /// The query changed since the kit last laid the rows out: the
    /// highlight goes back to the first runnable visible row once it has.
    reselect: bool,
}

impl Palette {
    fn new(
        entries: Vec<PaletteEntry>,
        quiet: Rgba,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let visible = (0..entries.len()).collect();
        Self {
            state: cx.new(|cx| CommandState::new(window, cx)),
            entries,
            visible,
            workspace,
            quiet,
            reselect: false,
        }
    }

    /// Every row, in list order (unfiltered).
    #[cfg(test)]
    pub(crate) fn entries(&self) -> &[PaletteEntry] {
        &self.entries
    }

    /// Whether the query field has the keyboard.
    #[cfg(test)]
    pub(crate) fn query_focused(&self, window: &Window, cx: &App) -> bool {
        use gpui_kit::Focusable as _;
        self.state.read(cx).focus_handle(cx).is_focused(window)
    }

    /// The query typed so far.
    #[cfg(test)]
    pub(crate) fn query(&self, cx: &App) -> String {
        self.state.read(cx).query(cx).to_string()
    }

    /// The rows the query keeps, best first.
    pub(crate) fn visible(&self) -> impl Iterator<Item = &PaletteEntry> {
        self.visible.iter().map(|&ix| &self.entries[ix])
    }

    fn on_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.visible = fuzzy_rank(&self.entries, query);
        self.reselect = true;
        cx.notify();
    }

    /// What the visible row at `index` runs, unless it is disabled.
    fn run_at(&self, index: IndexPath) -> Option<PaletteRun> {
        let entry = &self.entries[*self.visible.get(index.row)?];
        entry.blocked.is_none().then(|| entry.run.clone())
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.reselect) {
            // The kit keeps its highlight by row number across a re-filter,
            // so Enter would run whatever row now sits where it was. Put it
            // on the first runnable row of the new list, after the kit has
            // installed that list (this frame's `Command` render).
            let first = self
                .visible
                .iter()
                .position(|&ix| self.entries[ix].blocked.is_none());
            let state = self.state.clone();
            window.defer(cx, move |window, cx| {
                state.update(cx, |state, cx| {
                    state.set_selected_index(first.map(IndexPath::new), window, cx)
                });
            });
        }
        let this = cx.entity().downgrade();
        let confirm = this.clone();
        let workspace = self.workspace.clone();
        let quiet = self.quiet;
        Command::new(&self.state)
            .bordered(false)
            .filterable(false)
            .placeholder("Type a command")
            .items(self.visible().map(|entry| entry.item(quiet)))
            .on_query(move |query, _, cx| {
                this.update(cx, |palette, cx| palette.on_query(query, cx))
                    .ok();
            })
            .on_confirm(move |index, window, cx| {
                let Some(run) = confirm
                    .read_with(cx, |palette, _| palette.run_at(index))
                    .ok()
                    .flatten()
                else {
                    return;
                };
                workspace
                    .update(cx, |workspace, cx| {
                        workspace.run_palette_row(run, window, cx)
                    })
                    .ok();
            })
    }
}

impl Workspace {
    /// The rows the palette offers now, each runnable or saying why not,
    /// against `opener`, the panel that has the keyboard.
    pub(crate) fn palette_entries(&self, opener: Panel, cx: &App) -> Vec<PaletteEntry> {
        let keymap = keymap::active(cx);
        let gateway = self.active_gateway(opener, cx);
        let leaves = palette_rows(self.leader.tree(), &gateway)
            .into_iter()
            .map(|row| {
                let route = self.route_of(row.action);
                PaletteEntry {
                    label: row.label.into(),
                    hint: leaf_chord(row.action).and_then(|a| keymap.chord_for(&a)),
                    sequence: Some(row.keys.into()),
                    blocked: self.route_block(&route, opener, cx),
                    run: PaletteRun::Leaf(row.action),
                }
            });
        let chords = chord_rows().into_iter().map(|(label, run)| {
            let (hint, blocked) = match &run {
                PaletteRun::Chord(action) => (
                    keymap.chord_for(action),
                    self.chord_block(action, opener, cx),
                ),
                _ => (None, slot_focus_block(opener)),
            };
            PaletteEntry {
                label: label.into(),
                hint,
                sequence: None,
                blocked,
                run,
            }
        });
        let presets = self
            .preset_rows()
            .into_iter()
            .map(|(command, blocked)| PaletteEntry {
                label: command.label().into(),
                hint: None,
                sequence: None,
                blocked,
                run: PaletteRun::Preset(command),
            });
        let rail_rows = RailPosition::ALL.into_iter().map(|rail| PaletteEntry {
            label: format!("Rail position: {}", rail.label()).into(),
            hint: None,
            sequence: None,
            blocked: (self.rail == rail).then_some(RAIL_ALREADY_HERE),
            run: PaletteRun::RailAt(rail),
        });
        leaves
            .chain(chords)
            .chain(presets)
            .chain(rail_rows)
            .collect()
    }

    /// Why one of the GUI's own chords cannot run from the palette now.
    fn chord_block(
        &self,
        action: &ActionShortcuts,
        opener: Panel,
        cx: &App,
    ) -> Option<&'static str> {
        use ActionShortcuts as A;
        match action {
            A::FindInBuffer | A::ReplaceInBuffer | A::FollowLink => {
                (!self.note_has_keyboard(opener, cx)).then_some(NEEDS_NOTE_FOCUS)
            }
            A::MoveViewToNeighbour(_) => (opener == Panel::Editor).then_some(NEEDS_SLOT_FOCUS),
            A::SplitSlotRight | A::SplitSlotDown | A::CloseSlot => slot_focus_block(opener),
            A::QuickNote | A::OpenSortDialog => self.notes.is_none().then_some(NO_WORKSPACE),
            A::YankRow => (!self.lists_have_keyboard(opener)).then_some(NEEDS_LIST_FOCUS),
            A::SaveCurrentQuery => (!(matches!(opener, Panel::Drawer(_))
                && self.focused_view() == Some(DrawerView::Find)))
            .then_some(NEEDS_FIND_FOCUS),
            _ => None,
        }
    }

    /// Open the palette over the window, its query field focused. Refused
    /// while a modal is open (one modal at a time).
    pub(crate) fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if super::modal::dialog_open(window, cx) {
            return;
        }
        let entries = self.palette_entries(self.focused_panel(), cx);
        let workspace = cx.entity().downgrade();
        let quiet = theme::muted(&theme::current(cx));
        let palette = cx.new(|cx| Palette::new(entries, quiet, workspace, window, cx));
        self.palette = Some(palette.downgrade());
        let state = palette.read(cx).state.clone();
        self.show_modal(
            Slot::Own(ModalKind::Palette),
            WIDTH_REMS,
            move |window, cx| state.update(cx, |state, cx| state.focus(window, cx)),
            move |dialog, _, _| dialog.p_0().child(palette.clone()),
            window,
            cx,
        );
    }

    /// A palette row was chosen: close the palette (its opener gets the
    /// keyboard back), then run the row there.
    pub(crate) fn run_palette_row(
        &mut self,
        run: PaletteRun,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_modal(window, cx);
        match run {
            PaletteRun::Leaf(action) => {
                let route = self.route_of(action);
                self.perform_route(route, window, cx);
            }
            PaletteRun::Chord(ActionShortcuts::FindInBuffer) => {
                self.editor
                    .update(cx, |editor, cx| editor.find_command(false, window, cx));
            }
            PaletteRun::Chord(ActionShortcuts::ReplaceInBuffer) => {
                self.editor
                    .update(cx, |editor, cx| editor.find_command(true, window, cx));
            }
            PaletteRun::Chord(action) => {
                self.workspace_command(&action, None, window, cx);
            }
            PaletteRun::Preset(command) => self.run_preset_command(command, window, cx),
            PaletteRun::RailAt(rail) => self.set_rail(rail, cx),
            PaletteRun::MoveViewTo => match self.focused_slot() {
                Some(slot) => self.open_move_chooser(slot, window, cx),
                None => self.flash("Focus a slot to move its view", cx),
            },
        }
        cx.notify();
    }
}
