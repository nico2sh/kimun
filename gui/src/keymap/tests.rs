use super::*;
use kimun_notes::logic::layout::Side;

use ActionShortcuts as A;
use Platform::{LinuxWindows, MacOs};

const PLATFORMS: [Platform; 2] = [LinuxWindows, MacOs];
const PANELS: [Panel; 4] = [
    Panel::Rail,
    Panel::Drawer(Side::Left),
    Panel::Editor,
    Panel::Drawer(Side::Right),
];

/// A chord written literally (`ctrl`, `cmd`: no `primary`), for tables that
/// must read the same whichever platform runs the test.
fn chord(written: &str) -> KeyInput {
    parse_chord(written, LinuxWindows).unwrap_or_else(|e| panic!("{written}: {e}"))
}

/// A chord back in the written form, modifiers in a fixed order.
fn describe(chord: KeyInput) -> String {
    let mut parts = Vec::new();
    if chord.mods.sup {
        parts.push("cmd".to_string());
    }
    if chord.mods.ctrl {
        parts.push("ctrl".to_string());
    }
    if chord.mods.alt {
        parts.push("alt".to_string());
    }
    if chord.mods.shift {
        parts.push("shift".to_string());
    }
    parts.push(match chord.key {
        Key::Char(c) => c.to_string(),
        Key::Tab | Key::BackTab => "tab".to_string(),
        other => format!("{other:?}").to_lowercase(),
    });
    parts.join("+")
}

fn ctx(panel: Panel, vim: bool) -> KeyCtx {
    KeyCtx {
        panel,
        vim,
        query: false,
        browser: false,
        vim_pending: false,
        vim_insert: false,
    }
}

/// Where a query is typed (FIND, the search browser) over `panel`.
fn query_ctx(panel: Panel) -> KeyCtx {
    KeyCtx {
        query: true,
        ..ctx(panel, false)
    }
}

/// The search browser open over `panel`: a query is typed, in the browser.
fn browser_ctx(panel: Panel) -> KeyCtx {
    KeyCtx {
        browser: true,
        ..query_ctx(panel)
    }
}

fn table(keymap: &Keymap) -> Vec<(String, ActionShortcuts)> {
    keymap
        .bindings
        .iter()
        .map(|b| (describe(b.chord), b.action.clone()))
        .collect()
}

fn overrides(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    pairs
        .iter()
        .map(|(a, cs)| (a.to_string(), cs.iter().map(|c| c.to_string()).collect()))
        .collect()
}

// ---------------------------------------------------------------- parsing

#[test]
fn chords_parse_with_primary_per_platform() {
    let f = |w, p| parse_chord(w, p).unwrap();
    assert_eq!(f("primary+f", LinuxWindows), KeyInput::ctrl('f'));
    assert_eq!(
        f("primary+f", MacOs),
        KeyInput::new(Key::Char('f'), Mods::SUPER)
    );
    assert_eq!(
        f(" Primary + Shift + F ", LinuxWindows),
        KeyInput::new(
            Key::Char('f'),
            Mods {
                ctrl: true,
                shift: true,
                ..Mods::NONE
            }
        )
    );
    assert_eq!(
        f("cmd+alt+left", LinuxWindows),
        KeyInput::new(
            Key::Left,
            Mods {
                sup: true,
                alt: true,
                ..Mods::NONE
            }
        )
    );
    assert_eq!(f("alt+up", MacOs), KeyInput::new(Key::Up, Mods::ALT));
    assert_eq!(f("f5", MacOs), KeyInput::plain(Key::F(5)));
    assert_eq!(f("ctrl+space", MacOs), KeyInput::ctrl(' '));
    assert_eq!(f("ctrl+tab", MacOs), KeyInput::new(Key::Tab, Mods::CTRL));
    // As the input edge reports it.
    assert_eq!(
        f("ctrl+shift+tab", MacOs),
        KeyInput::new(
            Key::BackTab,
            Mods {
                ctrl: true,
                shift: true,
                ..Mods::NONE
            }
        )
    );
    assert_eq!(f("alt+esc", MacOs), KeyInput::new(Key::Esc, Mods::ALT));
    assert_eq!(f("ctrl+1", MacOs), KeyInput::ctrl('1'));
}

#[test]
fn bad_chords_say_why() {
    let e = |w| parse_chord(w, LinuxWindows).unwrap_err();
    assert_eq!(e(""), ChordError::Empty);
    assert_eq!(e("ctrl+"), ChordError::Empty);
    assert_eq!(e("hyper+f"), ChordError::UnknownModifier("hyper".into()));
    assert_eq!(e("ctrl+banana"), ChordError::UnknownKey("banana".into()));
    assert_eq!(e("ctrl+f99"), ChordError::UnknownKey("f99".into()));
    // Would type text instead of running a command.
    assert_eq!(e("f"), ChordError::NeedsModifier);
    assert_eq!(e("shift+f"), ChordError::NeedsModifier);
    assert!(parse_chord("shift+f5", LinuxWindows).is_ok());
}

// ---------------------------------------------------------- the defaults

/// The whole default table, per platform, in binding order. This is the
/// table in plans/m5b1/task-1-report.md.
#[test]
fn default_tables() {
    let lw = |c: &str, a: ActionShortcuts| (c.to_string(), a);
    assert_eq!(
        table(&Keymap::defaults(LinuxWindows)),
        vec![
            lw("ctrl+q", A::Quit),
            lw("ctrl+shift+q", A::Quit),
            lw("f(4)", A::OpenPreferences),
            lw("ctrl+,", A::OpenPreferences),
            lw("ctrl+k", A::SearchNotes),
            lw("ctrl+shift+k", A::SearchNotes),
            lw("ctrl+shift+f", A::SearchNotes),
            lw("ctrl+o", A::OpenNote),
            lw("ctrl+shift+o", A::OpenNote),
            lw("ctrl+j", A::NewJournal),
            lw("ctrl+shift+j", A::NewJournal),
            lw("alt+l", A::ToggleSidebar),
            lw("ctrl+e", A::OpenFileBrowser),
            lw("ctrl+shift+e", A::OpenFileBrowser),
            lw("alt+e", A::FocusEditor),
            lw("alt+f", A::FocusSidebar),
            lw("ctrl+r", A::OpenSortDialog),
            lw("ctrl+shift+r", A::OpenSortDialog),
            lw("f(2)", A::FileOperations),
            lw("ctrl+enter", A::FollowLink),
            lw("ctrl+shift+w", A::QuickNote),
            lw("ctrl+c", A::YankRow),
            lw("ctrl+shift+c", A::YankRow),
            lw("f(3)", A::OpenSavedSearches),
            lw("ctrl+d", A::SaveCurrentQuery),
            lw("ctrl+shift+d", A::SaveCurrentQuery),
            lw("f(6)", A::OpenAsk),
            lw("f(5)", A::SwitchWorkspace),
            lw("ctrl+f", A::FindInBuffer),
            lw("ctrl+h", A::ReplaceInBuffer),
            lw("ctrl+shift+h", A::ReplaceInBuffer),
            lw("ctrl+shift+b", A::Leader),
            lw("ctrl+shift+p", A::OpenCommandPalette),
            lw("alt+left", A::FocusRegion(Dir::Left)),
            lw("alt+right", A::FocusRegion(Dir::Right)),
            lw("alt+up", A::FocusRegion(Dir::Up)),
            lw("alt+down", A::FocusRegion(Dir::Down)),
            lw("alt+r", A::ToggleRightDrawer),
            lw("ctrl+s", A::SaveNote),
            lw("ctrl+shift+s", A::SaveNote),
            lw("f(1)", A::KeyboardHelp),
            lw("f(1)", A::QuerySyntax),
            lw("alt+shift+r", A::SplitSlotRight),
            lw("alt+shift+d", A::SplitSlotDown),
            lw("alt+shift+w", A::CloseSlot),
            lw("alt+shift+left", A::MoveViewToNeighbour(Dir::Left)),
            lw("alt+shift+right", A::MoveViewToNeighbour(Dir::Right)),
            lw("alt+shift+up", A::MoveViewToNeighbour(Dir::Up)),
            lw("alt+shift+down", A::MoveViewToNeighbour(Dir::Down)),
            lw("alt+m", A::ToggleZoom),
            lw("ctrl+shift+enter", A::OpenQueryInSidebar),
            lw("alt+a", A::ToggleRail),
            lw("ctrl+t", A::NewTab),
            lw("ctrl+w", A::CloseTab),
            lw("ctrl+tab", A::NextTab),
            lw("ctrl+shift+tab", A::PrevTab),
            lw("ctrl+n", A::NewNote),
            lw("ctrl+shift+n", A::NewDirectory),
            lw("f(7)", A::QueryBuilder),
        ]
    );
    assert_eq!(
        table(&Keymap::defaults(MacOs)),
        vec![
            lw("cmd+q", A::Quit),
            lw("f(4)", A::OpenPreferences),
            lw("cmd+,", A::OpenPreferences),
            lw("cmd+k", A::SearchNotes),
            lw("cmd+shift+f", A::SearchNotes),
            lw("cmd+o", A::OpenNote),
            lw("cmd+j", A::NewJournal),
            lw("cmd+alt+l", A::ToggleSidebar),
            lw("cmd+e", A::OpenFileBrowser),
            lw("cmd+alt+e", A::FocusEditor),
            lw("cmd+alt+s", A::FocusSidebar),
            lw("cmd+r", A::OpenSortDialog),
            lw("f(2)", A::FileOperations),
            lw("cmd+enter", A::FollowLink),
            lw("cmd+shift+w", A::QuickNote),
            lw("cmd+c", A::YankRow),
            lw("f(3)", A::OpenSavedSearches),
            lw("cmd+d", A::SaveCurrentQuery),
            lw("f(6)", A::OpenAsk),
            lw("f(5)", A::SwitchWorkspace),
            lw("cmd+f", A::FindInBuffer),
            lw("cmd+alt+f", A::ReplaceInBuffer),
            lw("cmd+shift+b", A::Leader),
            lw("cmd+shift+p", A::OpenCommandPalette),
            lw("cmd+alt+left", A::FocusRegion(Dir::Left)),
            lw("cmd+alt+right", A::FocusRegion(Dir::Right)),
            lw("cmd+alt+up", A::FocusRegion(Dir::Up)),
            lw("cmd+alt+down", A::FocusRegion(Dir::Down)),
            lw("cmd+alt+r", A::ToggleRightDrawer),
            lw("cmd+s", A::SaveNote),
            lw("f(1)", A::KeyboardHelp),
            lw("f(1)", A::QuerySyntax),
            lw("cmd+alt+shift+r", A::SplitSlotRight),
            lw("cmd+alt+shift+d", A::SplitSlotDown),
            lw("cmd+alt+shift+w", A::CloseSlot),
            lw("cmd+alt+shift+left", A::MoveViewToNeighbour(Dir::Left)),
            lw("cmd+alt+shift+right", A::MoveViewToNeighbour(Dir::Right)),
            lw("cmd+alt+shift+up", A::MoveViewToNeighbour(Dir::Up)),
            lw("cmd+alt+shift+down", A::MoveViewToNeighbour(Dir::Down)),
            lw("cmd+alt+m", A::ToggleZoom),
            lw("cmd+shift+enter", A::OpenQueryInSidebar),
            lw("cmd+alt+a", A::ToggleRail),
            lw("cmd+t", A::NewTab),
            lw("cmd+w", A::CloseTab),
            lw("ctrl+tab", A::NextTab),
            lw("ctrl+shift+tab", A::PrevTab),
            lw("cmd+n", A::NewNote),
            lw("cmd+shift+n", A::NewDirectory),
            lw("f(7)", A::QueryBuilder),
        ]
    );
}

/// Preferences (M9b) are the TUI's F4 and primary+comma from every panel,
/// under vim too (no letter: no twin).
#[test]
fn preferences_are_f4_and_primary_comma_from_every_panel() {
    let lw = Keymap::defaults(LinuxWindows);
    for panel in PANELS {
        for vim in [false, true] {
            assert_eq!(
                lw.resolve(KeyInput::plain(Key::F(4)), ctx(panel, vim)),
                Some(&A::OpenPreferences)
            );
            assert_eq!(
                lw.resolve(chord("ctrl+,"), ctx(panel, vim)),
                Some(&A::OpenPreferences)
            );
        }
    }
    let mac = Keymap::defaults(MacOs);
    assert_eq!(
        mac.resolve(
            KeyInput::new(Key::Char(','), Mods::SUPER),
            ctx(Panel::Editor, false)
        ),
        Some(&A::OpenPreferences)
    );
}

/// The keyboard reference (M10d) is a bare F1 on both platforms, from every
/// panel, under vim too (no letter: no twin, nothing for the vim rule),
/// wherever no query is typed; the query syntax holds F1 only where one is.
/// An override moves it like any other action.
#[test]
fn keyboard_help_is_f1_from_every_panel_on_both_platforms() {
    let f1 = KeyInput::plain(Key::F(1));
    for platform in [LinuxWindows, MacOs] {
        let keymap = Keymap::defaults(platform);
        assert_eq!(keymap.chords_for(&A::KeyboardHelp), [f1]);
        for panel in PANELS {
            for vim in [false, true] {
                assert_eq!(
                    keymap.resolve(f1, ctx(panel, vim)),
                    Some(&A::KeyboardHelp),
                    "{platform:?} {panel:?} vim={vim}"
                );
            }
        }
        let holders: Vec<_> = table(&keymap)
            .into_iter()
            .filter(|(c, _)| c == "f(1)")
            .map(|(_, a)| a)
            .collect();
        assert_eq!(
            holders,
            [A::KeyboardHelp, A::QuerySyntax],
            "{platform:?}: F1 is the reference's, and the query syntax's where a query is typed"
        );
    }
    let (moved, errors) = Keymap::load(
        LinuxWindows,
        &overrides(&[("KeyboardHelp", &["ctrl+shift+slash", "f12"])]),
    );
    assert_eq!(
        errors,
        [KeymapError::InvalidChord {
            action: A::KeyboardHelp,
            chord: "ctrl+shift+slash".into(),
            reason: ChordError::UnknownKey("slash".into()),
        }]
    );
    assert_eq!(
        moved.chords_for(&A::KeyboardHelp),
        [KeyInput::plain(Key::F(12))]
    );
    assert_eq!(moved.resolve(f1, ctx(Panel::Editor, false)), None);
}

/// The query syntax is F1 where a query is typed (FIND, the search browser,
/// over any panel), shadowing the keyboard reference only there. Rebound
/// under `[gui.keys]` it moves: F1 is the reference everywhere again. It
/// still collides with a panel-scoped chord, and with another query chord.
#[test]
fn query_syntax_is_f1_only_where_a_query_is_typed() {
    let f1 = KeyInput::plain(Key::F(1));
    for platform in [LinuxWindows, MacOs] {
        let keymap = Keymap::defaults(platform);
        assert_eq!(keymap.chords_for(&A::QuerySyntax), [f1]);
        for panel in PANELS {
            assert_eq!(
                keymap.resolve(f1, query_ctx(panel)),
                Some(&A::QuerySyntax),
                "{platform:?} {panel:?}"
            );
            assert_eq!(
                keymap.resolve(f1, ctx(panel, false)),
                Some(&A::KeyboardHelp)
            );
        }
        // Other chords resolve as usual where a query is typed.
        assert_eq!(
            keymap.resolve(
                KeyInput::plain(Key::F(4)),
                query_ctx(Panel::Drawer(Side::Left))
            ),
            Some(&A::OpenPreferences)
        );
    }

    let (moved, errors) = Keymap::load(LinuxWindows, &overrides(&[("QuerySyntax", &["f9"])]));
    assert!(errors.is_empty(), "{errors:?}");
    let f9 = KeyInput::plain(Key::F(9));
    assert_eq!(moved.chords_for(&A::QuerySyntax), [f9]);
    assert_eq!(
        moved.resolve(f9, query_ctx(Panel::Editor)),
        Some(&A::QuerySyntax)
    );
    assert_eq!(moved.resolve(f9, ctx(Panel::Editor, false)), None);
    assert_eq!(
        moved.resolve(f1, query_ctx(Panel::Drawer(Side::Left))),
        Some(&A::KeyboardHelp),
        "F1 is the reference's again where a query is typed"
    );

    // A slot-scoped chord (the sort dialog's) is not free for it.
    let (_, errors) = Keymap::load(LinuxWindows, &overrides(&[("QuerySyntax", &["ctrl+r"])]));
    assert_eq!(
        errors,
        [KeymapError::Collision {
            action: A::QuerySyntax,
            chord: "ctrl+r".into(),
            taken_by: A::OpenSortDialog,
        }]
    );
}

/// The vim rule's shadow, for a keyboard reference: only a Ctrl+letter the
/// engine reads, only for an action live in the editor, only on Linux and
/// Windows.
#[test]
fn vim_shadows_only_editor_chords_the_engine_reads() {
    let lw = Keymap::defaults(LinuxWindows);
    // Ctrl+R sorts from the slots only: never the editor's to lose.
    assert!(!lw.vim_shadows(&A::OpenSortDialog, chord("ctrl+r")));
    // An everywhere action bound to Ctrl+V loses it in the vim editor.
    assert!(lw.vim_shadows(&A::OpenNote, chord("ctrl+v")));
    assert!(!lw.vim_shadows(&A::OpenNote, chord("ctrl+o")));
    assert!(!lw.vim_shadows(&A::OpenNote, chord("ctrl+shift+v")));
    let mac = Keymap::defaults(MacOs);
    assert!(!mac.vim_shadows(&A::OpenNote, chord("ctrl+v")));
}

/// Every chord of an action, in order, for the CFG summary: Preferences
/// has two, the leader one, and an unbound action none.
#[test]
fn chords_for_lists_every_binding_in_order() {
    let lw = Keymap::defaults(LinuxWindows);
    assert_eq!(
        lw.chords_for(&A::OpenPreferences),
        [KeyInput::plain(Key::F(4)), chord("ctrl+,")]
    );
    assert_eq!(lw.chords_for(&A::Leader), [chord("ctrl+shift+b")]);
    assert_eq!(
        lw.chords_for(&A::OpenPreferences).first().copied(),
        lw.chord_for(&A::OpenPreferences)
    );
    let mut overrides = BTreeMap::new();
    overrides.insert("Leader".to_string(), Vec::new());
    let (unbound, errors) = Keymap::load(LinuxWindows, &overrides);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(unbound.chords_for(&A::Leader).is_empty());
}

/// Save (M5b-2 G2) is primary+S from every panel, under vim too: the vim
/// engine reads no Ctrl+S.
#[test]
fn save_is_primary_s_from_every_panel() {
    let lw = Keymap::defaults(LinuxWindows);
    for panel in PANELS {
        assert_eq!(
            lw.resolve(chord("ctrl+s"), ctx(panel, false)),
            Some(&A::SaveNote)
        );
        assert_eq!(
            lw.resolve(chord("ctrl+s"), ctx(panel, true)),
            Some(&A::SaveNote)
        );
    }
    let mac = Keymap::defaults(MacOs);
    assert_eq!(
        mac.resolve(chord("cmd+s"), ctx(Panel::Editor, true)),
        Some(&A::SaveNote)
    );
}

/// FILES (M5b-2 G3) is primary+E from every panel, as the TUI's Ctrl+E, under
/// vim too: the vim engine reads no Ctrl+E.
#[test]
fn open_file_browser_is_primary_e_from_every_panel() {
    let lw = Keymap::defaults(LinuxWindows);
    for panel in PANELS {
        assert_eq!(
            lw.resolve(chord("ctrl+e"), ctx(panel, false)),
            Some(&A::OpenFileBrowser)
        );
        assert_eq!(
            lw.resolve(chord("ctrl+e"), ctx(panel, true)),
            Some(&A::OpenFileBrowser)
        );
    }
    let mac = Keymap::defaults(MacOs);
    for panel in PANELS {
        assert_eq!(
            mac.resolve(chord("cmd+e"), ctx(panel, true)),
            Some(&A::OpenFileBrowser)
        );
    }
    assert_eq!(
        mac.resolve(chord("ctrl+e"), ctx(Panel::Editor, false)),
        None
    );
}

/// Follow link is primary+Enter in the editor, under vim too (the vim engine
/// reads no Ctrl+Enter), and nothing elsewhere. The TUI's Ctrl+N (new note) is
/// the GUI's `primary+n` too, never a follow. Shift+Enter stays the editor's plain newline.
#[test]
fn follow_link_is_primary_enter_in_the_editor() {
    let lw = Keymap::defaults(LinuxWindows);
    for written in ["ctrl+enter"] {
        for vim in [false, true] {
            assert_eq!(
                lw.resolve(chord(written), ctx(Panel::Editor, vim)),
                Some(&A::FollowLink),
                "{written} vim={vim}"
            );
        }
        for panel in PANELS.into_iter().filter(|p| *p != Panel::Editor) {
            assert_eq!(lw.resolve(chord(written), ctx(panel, false)), None);
        }
    }
    let mac = Keymap::defaults(MacOs);
    for written in ["cmd+enter"] {
        for vim in [false, true] {
            assert_eq!(
                mac.resolve(chord(written), ctx(Panel::Editor, vim)),
                Some(&A::FollowLink),
                "{written} vim={vim}"
            );
        }
    }
    for keymap in [&lw, &mac] {
        for written in ["shift+enter", "enter", "alt+enter"] {
            assert_eq!(
                keymap.resolve(chord(written), ctx(Panel::Editor, false)),
                None,
                "{written} stays the editor's"
            );
        }
    }
}

/// The command palette (M6) is primary+Shift+P and quitting primary+Q, from
/// every panel and under vim too: the vim engine reads neither. Bare Ctrl+P
/// stays unbound (a GUI quick-open convention), and the palette's chord has
/// no twin of its own (it is not a bare Ctrl+letter); Quit's is Ctrl+Shift+Q.
#[test]
fn palette_and_quit_chords_from_every_panel() {
    let lw = Keymap::defaults(LinuxWindows);
    let mac = Keymap::defaults(MacOs);
    for panel in PANELS {
        for vim in [false, true] {
            let at = ctx(panel, vim);
            assert_eq!(
                lw.resolve(chord("ctrl+shift+p"), at),
                Some(&A::OpenCommandPalette)
            );
            assert_eq!(lw.resolve(chord("ctrl+q"), at), Some(&A::Quit));
            assert_eq!(lw.resolve(chord("ctrl+shift+q"), at), Some(&A::Quit));
            assert_eq!(lw.resolve(chord("ctrl+p"), at), None);
            assert_eq!(
                mac.resolve(chord("cmd+shift+p"), at),
                Some(&A::OpenCommandPalette)
            );
            assert_eq!(mac.resolve(chord("cmd+q"), at), Some(&A::Quit));
        }
    }
    assert_eq!(
        lw.chord_for(&A::OpenCommandPalette),
        Some(chord("ctrl+shift+p"))
    );
    assert_eq!(
        lw.chord_for(&A::Quit),
        Some(chord("ctrl+q")),
        "the default, not the twin"
    );
}

/// The note menu (M6 G3) is the TUI's F2, from every panel and under vim
/// (the vim engine reads no F-keys), on both platforms.
#[test]
fn the_file_operations_chord_from_every_panel() {
    for keymap in [Keymap::defaults(LinuxWindows), Keymap::defaults(MacOs)] {
        for panel in PANELS {
            for vim in [false, true] {
                assert_eq!(
                    keymap.resolve(chord("f2"), ctx(panel, vim)),
                    Some(&A::FileOperations)
                );
            }
        }
        assert_eq!(keymap.chord_for(&A::FileOperations), Some(chord("f2")));
    }
}

/// Copying the selected list row (M7a G1) is primary+C (and its Ctrl+Shift
/// twin) from the rail and the slots only: in the editor, vim or not, the
/// chord stays the editor's copy.
#[test]
fn the_copy_row_chord_is_the_slots_only() {
    let lw = Keymap::defaults(LinuxWindows);
    let mac = Keymap::defaults(MacOs);
    for panel in PANELS {
        for vim in [false, true] {
            let at = ctx(panel, vim);
            let want = (panel != Panel::Editor).then_some(&A::YankRow);
            assert_eq!(lw.resolve(chord("ctrl+c"), at), want, "{panel:?}");
            assert_eq!(lw.resolve(chord("ctrl+shift+c"), at), want, "{panel:?}");
            assert_eq!(mac.resolve(chord("cmd+c"), at), want, "{panel:?}");
        }
    }
    assert_eq!(lw.chord_for(&A::YankRow), Some(chord("ctrl+c")));
}

/// The finder (Ctrl/Cmd+O), today's journal (Ctrl/Cmd+J) and the quick
/// note (Ctrl/Cmd+Shift+W) (M6 G4), from every panel and under vim too: the
/// vim engine reads none of them. The two bare Ctrl+letters get their
/// Ctrl+Shift twins; bare Ctrl+W is the desktop's close, the tab's.
#[test]
fn the_finder_journal_and_quick_note_chords_from_every_panel() {
    let lw = Keymap::defaults(LinuxWindows);
    let mac = Keymap::defaults(MacOs);
    for panel in PANELS {
        for vim in [false, true] {
            let at = ctx(panel, vim);
            for (written, action) in [
                ("ctrl+o", A::OpenNote),
                ("ctrl+shift+o", A::OpenNote),
                ("ctrl+j", A::NewJournal),
                ("ctrl+shift+j", A::NewJournal),
                ("ctrl+shift+w", A::QuickNote),
            ] {
                assert_eq!(lw.resolve(chord(written), at), Some(&action), "{written}");
            }
            assert_eq!(lw.resolve(chord("ctrl+w"), at), Some(&A::CloseTab));
            assert_eq!(mac.resolve(chord("cmd+o"), at), Some(&A::OpenNote));
            assert_eq!(mac.resolve(chord("cmd+j"), at), Some(&A::NewJournal));
            assert_eq!(mac.resolve(chord("cmd+shift+w"), at), Some(&A::QuickNote));
        }
    }
    assert_eq!(lw.chord_for(&A::OpenNote), Some(chord("ctrl+o")));
    assert_eq!(lw.chord_for(&A::NewJournal), Some(chord("ctrl+j")));
    assert_eq!(lw.chord_for(&A::QuickNote), Some(chord("ctrl+shift+w")));
}

/// The search browser (Ctrl/Cmd+K, M7b G4) from every panel and under vim
/// too: the vim engine reads no Ctrl+K. The bare Ctrl+K gets its Ctrl+Shift
/// twin; macOS's Cmd+K has none.
#[test]
fn the_search_chord_from_every_panel() {
    let lw = Keymap::defaults(LinuxWindows);
    let mac = Keymap::defaults(MacOs);
    for panel in PANELS {
        for vim in [false, true] {
            let at = ctx(panel, vim);
            assert_eq!(lw.resolve(chord("ctrl+k"), at), Some(&A::SearchNotes));
            assert_eq!(lw.resolve(chord("ctrl+shift+k"), at), Some(&A::SearchNotes));
            assert_eq!(mac.resolve(chord("cmd+k"), at), Some(&A::SearchNotes));
        }
    }
    assert_eq!(lw.chord_for(&A::SearchNotes), Some(chord("ctrl+k")));
    assert_eq!(mac.chord_for(&A::SearchNotes), Some(chord("cmd+k")));
}

/// The saved searches (F3, M7b G6) from every panel, vim or not; saving
/// the query (Ctrl/Cmd+D and Ctrl+Shift+D) only from the rail or a slot:
/// in the editor the chord stays the editor's (vim's half-page down), and
/// the search browser takes it by [`Keymap::is_bound`] whatever opened it.
#[test]
fn the_saved_search_chords() {
    let lw = Keymap::defaults(LinuxWindows);
    let mac = Keymap::defaults(MacOs);
    for panel in PANELS {
        for vim in [false, true] {
            let at = ctx(panel, vim);
            assert_eq!(lw.resolve(chord("f3"), at), Some(&A::OpenSavedSearches));
            assert_eq!(mac.resolve(chord("f3"), at), Some(&A::OpenSavedSearches));
            let save = (panel != Panel::Editor).then_some(&A::SaveCurrentQuery);
            assert_eq!(lw.resolve(chord("ctrl+d"), at), save, "{panel:?} vim={vim}");
            assert_eq!(lw.resolve(chord("ctrl+shift+d"), at), save, "{panel:?}");
            assert_eq!(mac.resolve(chord("cmd+d"), at), save, "{panel:?}");
        }
    }
    for written in ["ctrl+d", "ctrl+shift+d"] {
        assert!(lw.is_bound(chord(written), &A::SaveCurrentQuery));
    }
    assert!(mac.is_bound(chord("cmd+d"), &A::SaveCurrentQuery));
    assert!(!lw.is_bound(chord("ctrl+k"), &A::SaveCurrentQuery));
    assert_eq!(lw.chord_for(&A::SaveCurrentQuery), Some(chord("ctrl+d")));
    assert_eq!(lw.chord_for(&A::OpenSavedSearches), Some(chord("f3")));
}

/// The Ask Thread (F6, M7c G10a) from every panel, vim or not, on both
/// platforms: the TUI's default `ActionShortcuts` chord. It is no longer
/// among the actions shipped unbound.
#[test]
fn open_ask_is_f6_from_every_panel() {
    let lw = Keymap::defaults(LinuxWindows);
    let mac = Keymap::defaults(MacOs);
    for panel in PANELS {
        for vim in [false, true] {
            let at = ctx(panel, vim);
            assert_eq!(lw.resolve(chord("f6"), at), Some(&A::OpenAsk), "{panel:?}");
            assert_eq!(mac.resolve(chord("f6"), at), Some(&A::OpenAsk), "{panel:?}");
        }
    }
    assert_eq!(lw.chord_for(&A::OpenAsk), Some(chord("f6")));
    assert_eq!(mac.chord_for(&A::OpenAsk), Some(chord("f6")));
    for p in PLATFORMS {
        assert!(
            default_chords(&A::OpenAsk, p).is_some(),
            "{p:?}: OpenAsk is bound, not marked unbound"
        );
    }
}

/// The workspace switcher (F5, M8d) from every panel, vim or not, on both
/// platforms: the TUI's default chord, no longer shipped unbound.
#[test]
fn switch_workspace_is_f5_from_every_panel() {
    let lw = Keymap::defaults(LinuxWindows);
    let mac = Keymap::defaults(MacOs);
    for panel in PANELS {
        for vim in [false, true] {
            let at = ctx(panel, vim);
            assert_eq!(
                lw.resolve(chord("f5"), at),
                Some(&A::SwitchWorkspace),
                "{panel:?}"
            );
            assert_eq!(
                mac.resolve(chord("f5"), at),
                Some(&A::SwitchWorkspace),
                "{panel:?}"
            );
        }
    }
    assert_eq!(lw.chord_for(&A::SwitchWorkspace), Some(chord("f5")));
    assert_eq!(mac.chord_for(&A::SwitchWorkspace), Some(chord("f5")));
    for p in PLATFORMS {
        assert!(
            default_chords(&A::SwitchWorkspace, p).is_some(),
            "{p:?}: SwitchWorkspace is bound, not marked unbound"
        );
    }
}

/// The leader gateway (M6 G2) is primary+Shift+B from every panel, under vim
/// too; bare Ctrl+B stays unbound (desktop "Bold"), and the gateway has no
/// twin of its own.
#[test]
fn the_leader_chord_from_every_panel() {
    let lw = Keymap::defaults(LinuxWindows);
    let mac = Keymap::defaults(MacOs);
    for panel in PANELS {
        for vim in [false, true] {
            let at = ctx(panel, vim);
            assert_eq!(lw.resolve(chord("ctrl+shift+b"), at), Some(&A::Leader));
            assert_eq!(lw.resolve(chord("ctrl+b"), at), None);
            assert_eq!(mac.resolve(chord("cmd+shift+b"), at), Some(&A::Leader));
            assert_eq!(mac.resolve(chord("cmd+b"), at), None);
        }
    }
    assert_eq!(lw.chord_for(&A::Leader), Some(chord("ctrl+shift+b")));
    assert_eq!(mac.chord_for(&A::Leader), Some(chord("cmd+shift+b")));
}

/// A vim twin yields to another action's default: Ctrl+Shift+W is the
/// quick note, not Ctrl+W's (the tab close's) twin. With the quick note
/// rebound, Ctrl+W gets its twin back.
#[test]
fn a_vim_twin_yields_to_another_default() {
    let lw = Keymap::defaults(LinuxWindows);
    let twins: Vec<_> = lw
        .bindings
        .iter()
        .filter(|b| b.chord == chord("ctrl+shift+w"))
        .map(|b| b.action.clone())
        .collect();
    assert_eq!(twins, vec![A::QuickNote]);
    // Ctrl+W itself still closes the tab in the vim editor (nothing pending).
    assert_eq!(
        lw.resolve(chord("ctrl+w"), ctx(Panel::Editor, true)),
        Some(&A::CloseTab)
    );
    let (keymap, errors) = Keymap::load(
        LinuxWindows,
        &overrides(&[("QuickNote", &["primary+alt+b"])]),
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        keymap.resolve(chord("ctrl+shift+w"), ctx(Panel::Editor, true)),
        Some(&A::CloseTab)
    );
    assert_eq!(
        keymap.resolve(chord("ctrl+alt+b"), ctx(Panel::Rail, false)),
        Some(&A::QuickNote)
    );
}

#[test]
fn the_defaults_load_without_errors() {
    for p in PLATFORMS {
        let (keymap, errors) = Keymap::load(p, &BTreeMap::new());
        assert!(errors.is_empty(), "{p:?}: {errors:?}");
        assert_eq!(keymap, Keymap::defaults(p));
    }
}

/// ADR-0050 consequence: a shared action added later needs a GUI default or
/// an explicit "unbound" mark. The mark is `default_chords` returning `None`
/// (its match has no wildcard, so the compiler asks first); this pins that a
/// marked-bound action really holds a chord, and that the layout-tree
/// commands (ADR-0052) are bound.
#[test]
fn every_shared_action_has_a_gui_binding_or_is_marked_unbound() {
    let tree_commands = [
        A::SplitSlotRight,
        A::SplitSlotDown,
        A::CloseSlot,
        A::MoveViewToNeighbour(Dir::Left),
        A::MoveViewToNeighbour(Dir::Right),
        A::MoveViewToNeighbour(Dir::Up),
        A::MoveViewToNeighbour(Dir::Down),
        A::ToggleZoom,
        A::ToggleRail,
    ];
    for p in PLATFORMS {
        for action in &tree_commands {
            assert!(ActionShortcuts::ALL.contains(action), "{action}");
            assert!(default_chords(action, p).is_some(), "{p:?} {action}");
        }
    }
    for p in PLATFORMS {
        let keymap = Keymap::defaults(p);
        for action in ActionShortcuts::ALL {
            let bound = keymap.bindings.iter().any(|b| b.action == action);
            match default_chords(&action, p) {
                Some(chords) => {
                    assert!(
                        !chords.is_empty(),
                        "{p:?} {action}: empty list, mark it None"
                    );
                    assert!(bound, "{p:?} {action}: has defaults but no binding");
                }
                None => assert!(!bound, "{p:?} {action}: marked unbound but bound"),
            }
        }
    }
}

/// The drawer-era arrangement actions (the move-to-drawer, split / unsplit
/// and slot-swap ones) are gone from the shared vocabulary (the TUI never
/// bound them), so no GUI default, override or palette row
/// can name them: `[gui.keys]` reports them as unknown actions, and their
/// old split / unsplit chords hold nothing.
#[test]
fn retired_drawer_actions_are_gone_from_the_gui_keymap() {
    let retired = [
        "MoveViewToLeftDrawer",
        "MoveViewToRightDrawer",
        "SplitDrawerSlot",
        "UnsplitDrawerSlot",
        "MoveViewUp",
        "MoveViewDown",
    ];
    for name in retired {
        assert!(
            ActionShortcuts::try_from(name.to_string()).is_err(),
            "{name}"
        );
        assert!(
            ActionShortcuts::ALL.iter().all(|a| a.to_string() != name),
            "{name}"
        );
        let (_, errors) = Keymap::load(LinuxWindows, &overrides(&[(name, &["alt+shift+s"])]));
        assert_eq!(
            errors,
            vec![KeymapError::UnknownAction { name: name.into() }]
        );
    }
    for p in PLATFORMS {
        let keymap = Keymap::defaults(p);
        let pre = if p == MacOs { "cmd+" } else { "" };
        for key in ["s", "u"] {
            let written = format!("{pre}alt+shift+{key}");
            for panel in PANELS {
                assert_eq!(
                    keymap.resolve(chord(&written), ctx(panel, false)),
                    None,
                    "{p:?} {written} {panel:?}"
                );
            }
        }
    }
}

/// The layout-tree commands' defaults (ADR-0052) are unique: each chord
/// resolves to its command from the rail and every slot, no other default
/// holds it in an overlapping scope. The editor keeps the move chords;
/// the split and close chords act there too, on its active group
/// (ADR-0058).
#[test]
fn layout_tree_defaults_hold_unique_chords() {
    let expected = [
        ("alt+shift+r", A::SplitSlotRight),
        ("alt+shift+d", A::SplitSlotDown),
        ("alt+shift+w", A::CloseSlot),
        ("alt+shift+left", A::MoveViewToNeighbour(Dir::Left)),
        ("alt+shift+right", A::MoveViewToNeighbour(Dir::Right)),
        ("alt+shift+up", A::MoveViewToNeighbour(Dir::Up)),
        ("alt+shift+down", A::MoveViewToNeighbour(Dir::Down)),
    ];
    for p in PLATFORMS {
        let keymap = Keymap::defaults(p);
        let pre = if p == MacOs { "cmd+" } else { "" };
        for (written, action) in &expected {
            let written = format!("{pre}{written}");
            let key = chord(&written);
            let holders: Vec<&ActionShortcuts> = keymap
                .bindings
                .iter()
                .filter(|b| b.chord == key)
                .map(|b| &b.action)
                .collect();
            assert_eq!(holders, [action], "{p:?} {written}");
            let everywhere = matches!(
                action,
                A::SplitSlotRight | A::SplitSlotDown | A::CloseSlot | A::MoveViewToNeighbour(_)
            );
            for panel in PANELS {
                let want = (everywhere || panel != Panel::Editor).then_some(action);
                assert_eq!(
                    keymap.resolve(key, ctx(panel, false)),
                    want,
                    "{p:?} {written} {panel:?}"
                );
            }
        }
        let (_, errors) = Keymap::load(p, &BTreeMap::new());
        assert!(errors.is_empty(), "{p:?}: {errors:?}");
    }
}

/// Zoom (ADR-0053) has exactly one holder of its default chord on every
/// platform (Alt+M, Cmd+Alt+M on macOS), live from every
/// panel in plain and vim mode (vim reads no Alt chord), with no vim
/// twin and no load error.
#[test]
fn toggle_zoom_has_a_unique_default_chord_on_every_platform() {
    for p in PLATFORMS {
        let keymap = Keymap::defaults(p);
        let written = if p == MacOs { "cmd+alt+m" } else { "alt+m" };
        let key = chord(written);
        let mine: Vec<KeyInput> = keymap
            .bindings
            .iter()
            .filter(|b| b.action == A::ToggleZoom)
            .map(|b| b.chord)
            .collect();
        assert_eq!(mine, [key], "{p:?}: ToggleZoom's chords");
        let holders: Vec<&ActionShortcuts> = keymap
            .bindings
            .iter()
            .filter(|b| b.chord == key)
            .map(|b| &b.action)
            .collect();
        assert_eq!(holders, [&A::ToggleZoom], "{p:?} {written}");
        for panel in PANELS {
            for vim in [false, true] {
                for query in [false, true] {
                    assert_eq!(
                        keymap.resolve(
                            key,
                            KeyCtx {
                                panel,
                                vim,
                                query,
                                browser: query,
                                vim_pending: false,
                                vim_insert: false,
                            }
                        ),
                        Some(&A::ToggleZoom),
                        "{p:?} {written} {panel:?} vim={vim} query={query}"
                    );
                }
            }
        }
        assert!(!keymap.vim_shadows(&A::ToggleZoom, key), "{p:?}");
        let (_, errors) = Keymap::load(p, &BTreeMap::new());
        assert!(errors.is_empty(), "{p:?}: {errors:?}");
    }
}

/// Toggling the rail (ADR-0055 §8) has exactly one holder of its default
/// chord on every platform (primary+shift+A), live from every panel in plain
/// and vim mode (vim reads no Ctrl+Shift chord), with no vim twin and no load
/// error.
#[test]
fn toggle_rail_has_a_unique_default_chord_on_every_platform() {
    for p in PLATFORMS {
        let keymap = Keymap::defaults(p);
        let written = if p == MacOs { "cmd+alt+a" } else { "alt+a" };
        let key = chord(written);
        let mine: Vec<KeyInput> = keymap
            .bindings
            .iter()
            .filter(|b| b.action == A::ToggleRail)
            .map(|b| b.chord)
            .collect();
        assert_eq!(mine, [key], "{p:?}: ToggleRail's chords");
        let holders: Vec<&ActionShortcuts> = keymap
            .bindings
            .iter()
            .filter(|b| b.chord == key)
            .map(|b| &b.action)
            .collect();
        assert_eq!(holders, [&A::ToggleRail], "{p:?} {written}");
        for panel in PANELS {
            for vim in [false, true] {
                for query in [false, true] {
                    assert_eq!(
                        keymap.resolve(
                            key,
                            KeyCtx {
                                panel,
                                vim,
                                query,
                                browser: query,
                                vim_pending: false,
                                vim_insert: false,
                            }
                        ),
                        Some(&A::ToggleRail),
                        "{p:?} {written} {panel:?} vim={vim} query={query}"
                    );
                }
            }
        }
        assert!(!keymap.vim_shadows(&A::ToggleRail, key), "{p:?}");
        let (_, errors) = Keymap::load(p, &BTreeMap::new());
        assert!(errors.is_empty(), "{p:?}: {errors:?}");
    }
}

/// Every bound action can be triggered from every context its scope covers,
/// in plain and in vim mode — the vim twins are what keep the editor-scoped
/// ones reachable under vim on Linux/Windows. Where a query is typed, an
/// everywhere chord may be shadowed by the query scope's (F1), and only so.
#[test]
fn every_default_action_is_reachable_in_every_context() {
    for p in PLATFORMS {
        let keymap = Keymap::defaults(p);
        for action in ActionShortcuts::ALL {
            if default_chords(&action, p).is_none() {
                continue;
            }
            for panel in PANELS {
                for vim in [false, true] {
                    for query in [false, true] {
                        let at = KeyCtx {
                            panel,
                            vim,
                            query,
                            browser: query,
                            vim_pending: false,
                            vim_insert: false,
                        };
                        if !Scope::of(&action).applies(at) {
                            continue;
                        }
                        let mut chords = keymap.bindings.iter().filter(|b| b.action == action);
                        let reachable = chords
                            .clone()
                            .any(|b| keymap.resolve(b.chord, at) == Some(&action));
                        let shadowed = query
                            && chords.all(|b| {
                                keymap
                                    .resolve(b.chord, at)
                                    .is_some_and(|got| Scope::of(got).shadows())
                            });
                        assert!(
                            reachable || shadowed,
                            "{p:?} {action} unreachable in {at:?}"
                        );
                    }
                }
            }
        }
    }
}

/// The exhaustive resolution table: each probe chord in the editor (plain,
/// vim), in a slot and on the rail. A slot or the rail resolves the same
/// with vim on or off (the vim rule is the editor's), which the loop checks.
fn resolution(platform: Platform, rows: &[(&str, [Option<ActionShortcuts>; 3])]) {
    let keymap = Keymap::defaults(platform);
    for (written, [plain, vim, elsewhere]) in rows {
        let c = chord(written);
        let got = |panel, v| keymap.resolve(c, ctx(panel, v)).cloned();
        assert_eq!(
            got(Panel::Editor, false),
            *plain,
            "{platform:?} {written} plain editor"
        );
        assert_eq!(
            got(Panel::Editor, true),
            *vim,
            "{platform:?} {written} vim editor"
        );
        for panel in [
            Panel::Rail,
            Panel::Drawer(Side::Left),
            Panel::Drawer(Side::Right),
        ] {
            for v in [false, true] {
                assert_eq!(
                    got(panel, v),
                    *elsewhere,
                    "{platform:?} {written} {panel:?} vim={v}"
                );
            }
        }
    }
}

#[test]
fn linux_windows_resolution_table() {
    let find = Some(A::FindInBuffer);
    let replace = Some(A::ReplaceInBuffer);
    let toggle = Some(A::ToggleSidebar);
    let left = Some(A::FocusRegion(Dir::Left));
    let right = Some(A::FocusRegion(Dir::Right));
    let up = Some(A::FocusRegion(Dir::Up));
    let down = Some(A::FocusRegion(Dir::Down));
    let next = Some(A::FocusEditor);
    let prev = Some(A::FocusSidebar);
    let search = Some(A::SearchNotes);
    let new_tab = Some(A::NewTab);
    let new_note = Some(A::NewNote);
    let new_dir = Some(A::NewDirectory);
    let close_tab = Some(A::CloseTab);
    let next_tab = Some(A::NextTab);
    let prev_tab = Some(A::PrevTab);
    let files = Some(A::OpenFileBrowser);
    let follow = Some(A::FollowLink);
    let to_left = Some(A::MoveViewToNeighbour(Dir::Left));
    let to_right = Some(A::MoveViewToNeighbour(Dir::Right));
    let split_right = Some(A::SplitSlotRight);
    let split_down = Some(A::SplitSlotDown);
    let close_slot = Some(A::CloseSlot);
    let right_side = Some(A::ToggleRightDrawer);
    let view_up = Some(A::MoveViewToNeighbour(Dir::Up));
    let view_down = Some(A::MoveViewToNeighbour(Dir::Down));
    resolution(
        LinuxWindows,
        &[
            //                 plain editor  vim editor  slot / rail
            ("ctrl+f", [find.clone(), find.clone(), None]),
            ("ctrl+shift+f", [search.clone(), search.clone(), search]),
            ("ctrl+h", [replace.clone(), replace.clone(), None]),
            ("ctrl+shift+h", [replace.clone(), replace, None]),
            ("alt+l", [toggle.clone(), toggle.clone(), toggle]),
            (
                "alt+r",
                [right_side.clone(), right_side.clone(), right_side],
            ),
            // The tab chords (ADR-0058). Neither has a vim twin: Ctrl+T
            // opts out, Ctrl+Shift+W is the quick note's. Ctrl+Shift+T is
            // free.
            ("ctrl+t", [new_tab.clone(), new_tab.clone(), new_tab]),
            ("ctrl+shift+t", [None, None, None]),
            ("ctrl+w", [close_tab.clone(), close_tab.clone(), close_tab]),
            (
                "ctrl+shift+w",
                [Some(A::QuickNote), Some(A::QuickNote), Some(A::QuickNote)],
            ),
            ("ctrl+e", [files.clone(), files.clone(), files.clone()]),
            ("ctrl+shift+e", [files.clone(), files.clone(), files]),
            ("ctrl+n", [new_note.clone(), new_note.clone(), new_note]),
            ("ctrl+shift+n", [new_dir.clone(), new_dir.clone(), new_dir]),
            ("ctrl+enter", [follow.clone(), follow, None]),
            // Vim reads Ctrl+R (redo): the sort dialog's chord is the rail's
            // and the slots' only, so the editor keeps it, vim or not.
            ("ctrl+r", [None, None, Some(A::OpenSortDialog)]),
            ("ctrl+shift+r", [None, None, Some(A::OpenSortDialog)]),
            ("alt+left", [left.clone(), left.clone(), left]),
            ("alt+right", [right.clone(), right.clone(), right]),
            ("alt+up", [up.clone(), up.clone(), up]),
            ("alt+down", [down.clone(), down.clone(), down]),
            ("alt+e", [next.clone(), next.clone(), next]),
            ("alt+f", [prev.clone(), prev.clone(), prev]),
            ("ctrl+tab", [next_tab.clone(), next_tab.clone(), next_tab]),
            (
                "ctrl+shift+tab",
                [prev_tab.clone(), prev_tab.clone(), prev_tab],
            ),
            // Moving views and, in the editor, its active group.
            (
                "alt+shift+left",
                [to_left.clone(), to_left.clone(), to_left],
            ),
            (
                "alt+shift+right",
                [to_right.clone(), to_right.clone(), to_right],
            ),
            ("alt+shift+up", [view_up.clone(), view_up.clone(), view_up]),
            (
                "alt+shift+down",
                [view_down.clone(), view_down.clone(), view_down],
            ),
            // Split and close act on the focused leaf: a slot, or the
            // editor's active group (ADR-0058).
            (
                "alt+shift+r",
                [split_right.clone(), split_right.clone(), split_right],
            ),
            (
                "alt+shift+d",
                [split_down.clone(), split_down.clone(), split_down],
            ),
            (
                "alt+shift+w",
                [close_slot.clone(), close_slot.clone(), close_slot],
            ),
            // The retired drawer-era split / unsplit chords are free.
            ("alt+shift+s", [None, None, None]),
            ("alt+shift+u", [None, None, None]),
            // Saving FIND's query: the slots' only; the editor (vim's
            // half-page down) keeps Ctrl+D.
            ("ctrl+d", [None, None, Some(A::SaveCurrentQuery)]),
            ("ctrl+shift+d", [None, None, Some(A::SaveCurrentQuery)]),
            // The old right-side chord is free again.
            ("ctrl+alt+b", [None, None, None]),
            // Not bound here: they reach the editor (or nothing).
            ("cmd+f", [None, None, None]),
            ("cmd+alt+left", [None, None, None]),
            ("ctrl+left", [None, None, None]),
            ("alt+g", [None, None, None]),
        ],
    );
}

#[test]
fn macos_resolution_table() {
    let find = Some(A::FindInBuffer);
    let replace = Some(A::ReplaceInBuffer);
    let toggle = Some(A::ToggleSidebar);
    let left = Some(A::FocusRegion(Dir::Left));
    let right = Some(A::FocusRegion(Dir::Right));
    let up = Some(A::FocusRegion(Dir::Up));
    let down = Some(A::FocusRegion(Dir::Down));
    let next = Some(A::FocusEditor);
    let prev = Some(A::FocusSidebar);
    let search = Some(A::SearchNotes);
    let new_tab = Some(A::NewTab);
    let new_note = Some(A::NewNote);
    let new_dir = Some(A::NewDirectory);
    let close_tab = Some(A::CloseTab);
    let next_tab = Some(A::NextTab);
    let prev_tab = Some(A::PrevTab);
    let files = Some(A::OpenFileBrowser);
    let follow = Some(A::FollowLink);
    let to_left = Some(A::MoveViewToNeighbour(Dir::Left));
    let to_right = Some(A::MoveViewToNeighbour(Dir::Right));
    let split_right = Some(A::SplitSlotRight);
    let split_down = Some(A::SplitSlotDown);
    let close_slot = Some(A::CloseSlot);
    let right_side = Some(A::ToggleRightDrawer);
    let view_up = Some(A::MoveViewToNeighbour(Dir::Up));
    let view_down = Some(A::MoveViewToNeighbour(Dir::Down));
    resolution(
        MacOs,
        &[
            //                 plain editor  vim editor  slot / rail
            ("cmd+f", [find.clone(), find, None]),
            ("cmd+alt+f", [replace.clone(), replace, None]),
            ("cmd+alt+l", [toggle.clone(), toggle.clone(), toggle]),
            ("cmd+t", [new_tab.clone(), new_tab.clone(), new_tab]),
            ("cmd+w", [close_tab.clone(), close_tab.clone(), close_tab]),
            ("cmd+e", [files.clone(), files.clone(), files]),
            ("cmd+n", [new_note.clone(), new_note.clone(), new_note]),
            ("cmd+shift+n", [new_dir.clone(), new_dir.clone(), new_dir]),
            ("cmd+enter", [follow.clone(), follow, None]),
            ("cmd+alt+left", [left.clone(), left.clone(), left]),
            ("cmd+alt+right", [right.clone(), right.clone(), right]),
            ("cmd+alt+up", [up.clone(), up.clone(), up]),
            ("cmd+alt+down", [down.clone(), down.clone(), down]),
            ("cmd+alt+e", [next.clone(), next.clone(), next]),
            ("cmd+alt+s", [prev.clone(), prev.clone(), prev]),
            ("ctrl+tab", [next_tab.clone(), next_tab.clone(), next_tab]),
            (
                "ctrl+shift+tab",
                [prev_tab.clone(), prev_tab.clone(), prev_tab],
            ),
            (
                "cmd+alt+shift+left",
                [to_left.clone(), to_left.clone(), to_left],
            ),
            (
                "cmd+alt+shift+right",
                [to_right.clone(), to_right.clone(), to_right],
            ),
            (
                "cmd+alt+shift+up",
                [view_up.clone(), view_up.clone(), view_up],
            ),
            (
                "cmd+alt+shift+down",
                [view_down.clone(), view_down.clone(), view_down],
            ),
            // Split and close act on the focused leaf: a slot, or the
            // editor's active group (ADR-0058).
            (
                "cmd+alt+shift+r",
                [split_right.clone(), split_right.clone(), split_right],
            ),
            (
                "cmd+alt+shift+d",
                [split_down.clone(), split_down.clone(), split_down],
            ),
            (
                "cmd+alt+shift+w",
                [close_slot.clone(), close_slot.clone(), close_slot],
            ),
            // The retired drawer-era split / unsplit chords are free.
            ("cmd+alt+shift+s", [None, None, None]),
            ("cmd+alt+shift+u", [None, None, None]),
            (
                "cmd+alt+r",
                [right_side.clone(), right_side.clone(), right_side],
            ),
            // Option+letter types text on macOS: the Linux side and focus
            // chords are not ours there.
            ("alt+e", [None, None, None]),
            ("alt+f", [None, None, None]),
            ("alt+l", [None, None, None]),
            ("alt+r", [None, None, None]),
            ("cmd+shift+t", [None, None, None]),
            ("cmd+alt+b", [None, None, None]),
            // Option+Shift+Arrow is word selection on macOS: not ours.
            ("alt+shift+left", [None, None, None]),
            // Ctrl is literal and free on macOS: the editor's (and vim's).
            ("ctrl+f", [None, None, None]),
            ("ctrl+h", [None, None, None]),
            ("ctrl+t", [None, None, None]),
            ("ctrl+w", [None, None, None]),
            ("ctrl+e", [None, None, None]),
            ("ctrl+n", [None, None, None]),
            ("ctrl+enter", [None, None, None]),
            ("cmd+h", [None, None, None]),
            ("alt+left", [None, None, None]),
            ("cmd+shift+f", [search.clone(), search.clone(), search]),
        ],
    );
}

// ------------------------------------------------------------- overrides

#[test]
fn an_override_replaces_the_defaults_and_primary_follows_the_platform() {
    let o = overrides(&[("FindInBuffer", &["primary+r"])]);
    for p in PLATFORMS {
        let (keymap, errors) = Keymap::load(p, &o);
        assert!(errors.is_empty(), "{errors:?}");
        let editor = ctx(Panel::Editor, false);
        let g = parse_chord("primary+r", p).unwrap();
        assert_eq!(keymap.resolve(g, editor), Some(&A::FindInBuffer));
        let f = parse_chord("primary+f", p).unwrap();
        assert_eq!(keymap.resolve(f, editor), None, "{p:?}: default gone");
    }
    // Overrides are literal: no vim twin, so under vim Ctrl+R is vim's.
    let (keymap, _) = Keymap::load(LinuxWindows, &o);
    assert_eq!(
        keymap.resolve(chord("ctrl+r"), ctx(Panel::Editor, true)),
        None
    );
    assert_eq!(
        keymap.resolve(chord("ctrl+shift+r"), ctx(Panel::Editor, true)),
        None
    );
}

#[test]
fn an_empty_list_unbinds() {
    let (keymap, errors) = Keymap::load(LinuxWindows, &overrides(&[("ToggleSidebar", &[])]));
    assert!(errors.is_empty());
    assert!(keymap.bindings.iter().all(|b| b.action != A::ToggleSidebar));
    assert_eq!(
        keymap.resolve(chord("alt+l"), ctx(Panel::Rail, false)),
        None
    );
}

#[test]
fn every_modifier_name_is_accepted() {
    let o = overrides(&[(
        "ToggleRightDrawer",
        &["primary+1", "cmd+2", "ctrl+3", "alt+4", "shift+f6"],
    )]);
    let (keymap, errors) = Keymap::load(MacOs, &o);
    assert!(errors.is_empty(), "{errors:?}");
    let rail = ctx(Panel::Rail, false);
    for c in ["cmd+1", "cmd+2", "ctrl+3", "alt+4", "shift+f6"] {
        assert_eq!(
            keymap.resolve(chord(c), rail),
            Some(&A::ToggleRightDrawer),
            "{c}"
        );
    }
}

#[test]
fn unknown_actions_and_bad_chords_are_reported_and_the_rest_loads() {
    let o = overrides(&[
        ("NoSuchAction", &["ctrl+q"]),
        ("ToggleRightDrawer", &["ctrl+banana", "alt+g"]),
    ]);
    let (keymap, errors) = Keymap::load(LinuxWindows, &o);
    assert_eq!(
        errors,
        vec![
            KeymapError::UnknownAction {
                name: "NoSuchAction".into()
            },
            KeymapError::InvalidChord {
                action: A::ToggleRightDrawer,
                chord: "ctrl+banana".into(),
                reason: ChordError::UnknownKey("banana".into()),
            },
        ]
    );
    let rail = ctx(Panel::Rail, false);
    assert_eq!(
        keymap.resolve(chord("alt+g"), rail),
        Some(&A::ToggleRightDrawer)
    );
    // The override replaced the default.
    assert_eq!(keymap.resolve(chord("alt+r"), rail), None);
    // Defaults untouched.
    assert_eq!(
        keymap.resolve(chord("alt+l"), rail),
        Some(&A::ToggleSidebar)
    );
}

#[test]
fn a_chord_held_by_another_action_is_rejected_not_stolen() {
    // Alt+L is ToggleSidebar's default; the override loses, the default stays.
    let o = overrides(&[("ToggleRightDrawer", &["alt+l", "primary+shift+g"])]);
    let (keymap, errors) = Keymap::load(LinuxWindows, &o);
    assert_eq!(
        errors,
        vec![KeymapError::Collision {
            action: A::ToggleRightDrawer,
            chord: "alt+l".into(),
            taken_by: A::ToggleSidebar,
        }]
    );
    let slot = ctx(Panel::Drawer(Side::Left), false);
    assert_eq!(
        keymap.resolve(chord("alt+l"), slot),
        Some(&A::ToggleSidebar)
    );
    assert_eq!(
        keymap.resolve(chord("ctrl+shift+g"), slot),
        Some(&A::ToggleRightDrawer)
    );
}

#[test]
fn two_overrides_on_one_chord_keep_the_first_in_action_order() {
    let o = overrides(&[("CloseSlot", &["alt+s"]), ("SplitSlotRight", &["alt+s"])]);
    let (keymap, errors) = Keymap::load(MacOs, &o);
    // SplitSlotRight sorts before CloseSlot.
    assert_eq!(
        errors,
        vec![KeymapError::Collision {
            action: A::CloseSlot,
            chord: "alt+s".into(),
            taken_by: A::SplitSlotRight,
        }]
    );
    assert_eq!(
        keymap.resolve(chord("alt+s"), ctx(Panel::Rail, false)),
        Some(&A::SplitSlotRight)
    );
}

/// Moving a chord between actions works when the holder is rebound too.
#[test]
fn rebinding_the_holder_frees_its_chord() {
    let o = overrides(&[
        ("ToggleSidebar", &["primary+b"]),
        ("FindInBuffer", &["alt+l"]),
    ]);
    let (keymap, errors) = Keymap::load(LinuxWindows, &o);
    assert!(errors.is_empty(), "{errors:?}");
    let editor = ctx(Panel::Editor, false);
    assert_eq!(
        keymap.resolve(chord("alt+l"), editor),
        Some(&A::FindInBuffer)
    );
    assert_eq!(
        keymap.resolve(chord("ctrl+b"), editor),
        Some(&A::ToggleSidebar)
    );
}

#[test]
fn an_editor_action_may_not_share_a_chord_with_a_global_one() {
    // Both live while the editor is focused: one context, two actions.
    let o = overrides(&[("FindInBuffer", &["alt+left"])]);
    let (_, errors) = Keymap::load(LinuxWindows, &o);
    assert_eq!(
        errors,
        vec![KeymapError::Collision {
            action: A::FindInBuffer,
            chord: "alt+left".into(),
            taken_by: A::FocusRegion(Dir::Left),
        }]
    );
}

#[test]
fn legacy_action_names_are_accepted() {
    let o = overrides(&[("OpenSettings", &["primary+comma"])]);
    let (_, errors) = Keymap::load(LinuxWindows, &o);
    // `comma` is not a key name; the action name itself was understood.
    assert_eq!(
        errors,
        vec![KeymapError::InvalidChord {
            action: A::OpenPreferences,
            chord: "primary+comma".into(),
            reason: ChordError::UnknownKey("comma".into()),
        }]
    );
    let (keymap, errors) = Keymap::load(
        LinuxWindows,
        &overrides(&[("OpenSettings", &["primary+,"])]),
    );
    assert!(errors.is_empty());
    assert_eq!(
        keymap.resolve(chord("ctrl+,"), ctx(Panel::Editor, true)),
        Some(&A::OpenPreferences),
        "Ctrl+comma is no letter: the vim rule leaves it alone"
    );
}

#[test]
fn errors_read_as_config_lines() {
    let e = KeymapError::Collision {
        action: A::ToggleRightDrawer,
        chord: "alt+l".into(),
        taken_by: A::ToggleSidebar,
    };
    assert_eq!(
        e.to_string(),
        "[gui.keys] ToggleRightDrawer = \"alt+l\": already bound to ToggleSidebar"
    );
}

#[test]
fn the_current_platform_matches_the_build_target() {
    assert_eq!(Platform::current() == MacOs, cfg!(target_os = "macos"));
}

/// The sort dialog (M7b G7): Ctrl/Cmd+R and the Ctrl+Shift+R twin only from
/// the rail or a slot; never in the editor, where Ctrl+R is vim's redo
/// (and the plain editor's own key), vim or not. The search browser takes
/// it by [`Keymap::is_bound`] whatever opened it.
#[test]
fn the_sort_chord_is_scoped_to_lists() {
    let lw = Keymap::defaults(LinuxWindows);
    let mac = Keymap::defaults(MacOs);
    for panel in PANELS {
        for vim in [false, true] {
            let at = ctx(panel, vim);
            let sort = (panel != Panel::Editor).then_some(&A::OpenSortDialog);
            assert_eq!(lw.resolve(chord("ctrl+r"), at), sort, "{panel:?} vim={vim}");
            assert_eq!(lw.resolve(chord("ctrl+shift+r"), at), sort, "{panel:?}");
            assert_eq!(mac.resolve(chord("cmd+r"), at), sort, "{panel:?}");
        }
    }
    for written in ["ctrl+r", "ctrl+shift+r"] {
        assert!(lw.is_bound(chord(written), &A::OpenSortDialog));
    }
    assert!(mac.is_bound(chord("cmd+r"), &A::OpenSortDialog));
    assert_eq!(lw.chord_for(&A::OpenSortDialog), Some(chord("ctrl+r")));
}

/// The browser's "open in FIND" chord is live only with the search browser
/// open, never where FIND merely has the keyboard (the query is already
/// there), nor elsewhere.
#[test]
fn the_open_in_find_chord_is_scoped_to_the_browser() {
    let lw = Keymap::defaults(LinuxWindows);
    let key = chord("ctrl+shift+enter");
    for panel in PANELS {
        assert_eq!(
            lw.resolve(key, browser_ctx(panel)),
            Some(&A::OpenQueryInSidebar),
            "{panel:?}"
        );
        assert_eq!(lw.resolve(key, query_ctx(panel)), None, "{panel:?} FIND");
        assert_eq!(lw.resolve(key, ctx(panel, false)), None, "{panel:?}");
    }
}

// ------------------------------------------------------------------ tabs

/// No two default bindings share a chord where both are live, on either
/// platform: a chord resolves to one action (the query scopes shadow by
/// design, and are the only pairs allowed).
#[test]
fn default_chords_have_no_duplicates() {
    for p in PLATFORMS {
        let (keymap, errors) = Keymap::load(p, &BTreeMap::new());
        assert!(errors.is_empty(), "{p:?}: {errors:?}");
        for (i, a) in keymap.bindings.iter().enumerate() {
            for b in &keymap.bindings[i + 1..] {
                if a.chord != b.chord {
                    continue;
                }
                assert!(
                    !Scope::of(&a.action).overlaps(Scope::of(&b.action)),
                    "{p:?}: {} is both {} and {}",
                    describe(a.chord),
                    a.action,
                    b.action
                );
            }
        }
        // The pairs that do share a chord are only the shadowing ones.
        let shared: Vec<(String, ActionShortcuts, ActionShortcuts)> = keymap
            .bindings
            .iter()
            .enumerate()
            .flat_map(|(i, a)| {
                keymap.bindings[i + 1..]
                    .iter()
                    .filter(move |b| b.chord == a.chord)
                    .map(move |b| (describe(a.chord), a.action.clone(), b.action.clone()))
            })
            .collect();
        assert_eq!(
            shared,
            [("f(1)".to_string(), A::KeyboardHelp, A::QuerySyntax)],
            "{p:?}"
        );
    }
}

/// The tab chords and the side and focus chords they displaced, exactly as
/// the plan's constraints write them for Linux and Windows (Alt+letter), and
/// with Cmd+Option on macOS, where Option+letter types text (focus back is
/// S there: Cmd+Option+F is Replace). Each is live from every panel, plain
/// or vim (nothing pending), and nothing else holds it.
#[test]
fn tab_chords_match_the_constraints() {
    for p in PLATFORMS {
        let keymap = Keymap::defaults(p);
        // No vim twin for the new tab (it opts out), none for the close
        // (the quick note holds Ctrl+Shift+W).
        let (new, close) = if p == MacOs {
            ("cmd+t", "cmd+w")
        } else {
            ("ctrl+t", "ctrl+w")
        };
        let (focus_editor, focus_sidebar, left, right) = if p == MacOs {
            ("cmd+alt+e", "cmd+alt+s", "cmd+alt+l", "cmd+alt+r")
        } else {
            ("alt+e", "alt+f", "alt+l", "alt+r")
        };
        let expected: [(ActionShortcuts, Vec<KeyInput>); 8] = [
            (A::NewTab, vec![chord(new)]),
            (A::CloseTab, vec![chord(close)]),
            (A::NextTab, vec![chord("ctrl+tab")]),
            (A::PrevTab, vec![chord("ctrl+shift+tab")]),
            (A::FocusEditor, vec![chord(focus_editor)]),
            (A::FocusSidebar, vec![chord(focus_sidebar)]),
            (A::ToggleSidebar, vec![chord(left)]),
            (A::ToggleRightDrawer, vec![chord(right)]),
        ];
        for (action, chords) in &expected {
            assert_eq!(&keymap.chords_for(action), chords, "{p:?} {action}");
            for key in chords {
                for panel in PANELS {
                    for vim in [false, true] {
                        assert_eq!(
                            keymap.resolve(*key, ctx(panel, vim)),
                            Some(action),
                            "{p:?} {} {panel:?} vim={vim}",
                            describe(*key)
                        );
                    }
                }
                assert!(!keymap.vim_shadows(action, *key), "{p:?} {action}");
            }
        }
        // The old homes of the displaced chords now belong to the tabs.
        assert_eq!(
            keymap.resolve(chord("ctrl+tab"), ctx(Panel::Rail, false)),
            Some(&A::NextTab)
        );
        assert_eq!(
            keymap.resolve(
                parse_chord("primary+shift+t", p).unwrap(),
                ctx(Panel::Rail, false)
            ),
            None,
            "{p:?}: the right toggle's old chord is free (no new-tab twin)"
        );
        for panel in PANELS {
            assert_eq!(
                keymap.resolve(parse_chord("primary+shift+t", p).unwrap(), ctx(panel, true)),
                None,
                "{p:?} {panel:?}"
            );
        }
    }
}

/// Vim's Ctrl+W is a window prefix. With nothing pending in the vim engine
/// Ctrl+W closes the tab (the engine reads no Ctrl+W); while it holds a
/// pending sequence (`d`, `2`, `g`, ...) Ctrl+W is the engine's and no tab
/// closes. The plain editor and every other panel never hold a sequence.
#[test]
fn ctrl_w_does_not_close_a_tab_while_vim_holds_a_pending_prefix() {
    let lw = Keymap::defaults(LinuxWindows);
    let ctrl_w = chord("ctrl+w");
    let pending = |panel, vim| KeyCtx {
        vim_pending: true,
        vim_insert: false,
        ..ctx(panel, vim)
    };
    assert_eq!(lw.resolve(ctrl_w, pending(Panel::Editor, true)), None);
    assert_eq!(
        lw.resolve(ctrl_w, ctx(Panel::Editor, true)),
        Some(&A::CloseTab)
    );
    assert_eq!(
        lw.resolve(ctrl_w, ctx(Panel::Editor, false)),
        Some(&A::CloseTab)
    );
    // The rule is the vim editor's only: a slot closes the tab whatever.
    for panel in PANELS.into_iter().filter(|p| *p != Panel::Editor) {
        assert_eq!(
            lw.resolve(ctrl_w, pending(panel, true)),
            Some(&A::CloseTab),
            "{panel:?}"
        );
    }
    // Only Ctrl+W yields: the other tab chords still run mid-sequence.
    for (written, action) in [
        ("ctrl+t", A::NewTab),
        ("ctrl+tab", A::NextTab),
        ("ctrl+shift+tab", A::PrevTab),
        ("ctrl+s", A::SaveNote),
    ] {
        assert_eq!(
            lw.resolve(chord(written), pending(Panel::Editor, true)),
            Some(&action),
            "{written}"
        );
    }
    // A Ctrl+W bound by an override yields the same way: the rule follows
    // the key, vim's window prefix, not the action.
    let (keymap, errors) = Keymap::load(
        LinuxWindows,
        &overrides(&[("CloseTab", &["alt+w"]), ("SaveNote", &["primary+w"])]),
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(keymap.resolve(ctrl_w, pending(Panel::Editor, true)), None);
    assert_eq!(
        keymap.resolve(chord("alt+w"), pending(Panel::Editor, true)),
        Some(&A::CloseTab)
    );
    // macOS closes with Cmd+W, which vim never reads, pending or not.
    let mac = Keymap::defaults(MacOs);
    assert_eq!(
        mac.resolve(chord("cmd+w"), pending(Panel::Editor, true)),
        Some(&A::CloseTab)
    );
}

/// Final-review ruling: in vim Insert mode Ctrl+W is the editor's (vim's
/// delete-word), never the tab close, pending sequence or not. Normal mode
/// keeps the pending-prefix rule, and the plain editor closes the tab.
#[test]
fn ctrl_w_in_vim_insert_mode_goes_to_the_editor() {
    let lw = Keymap::defaults(LinuxWindows);
    let ctrl_w = chord("ctrl+w");
    let insert = |vim| KeyCtx {
        vim_insert: true,
        ..ctx(Panel::Editor, vim)
    };
    assert_eq!(lw.resolve(ctrl_w, insert(true)), None, "vim Insert");
    // Normal mode: with nothing pending it closes the tab; pending, not.
    assert_eq!(
        lw.resolve(ctrl_w, ctx(Panel::Editor, true)),
        Some(&A::CloseTab),
        "vim Normal, nothing pending"
    );
    let pending = KeyCtx {
        vim_pending: true,
        ..ctx(Panel::Editor, true)
    };
    assert_eq!(lw.resolve(ctrl_w, pending), None, "vim Normal, pending");
    // The plain editor never runs vim: the flag means nothing there.
    assert_eq!(
        lw.resolve(ctrl_w, ctx(Panel::Editor, false)),
        Some(&A::CloseTab),
        "plain editor"
    );
    // Only Ctrl+W yields: the other tab chords still run in Insert mode.
    for (written, action) in [
        ("ctrl+t", A::NewTab),
        ("ctrl+tab", A::NextTab),
        ("ctrl+s", A::SaveNote),
    ] {
        assert_eq!(
            lw.resolve(chord(written), insert(true)),
            Some(&action),
            "{written}"
        );
    }
    // macOS closes with Cmd+W, which vim never reads.
    assert_eq!(
        Keymap::defaults(MacOs).resolve(chord("cmd+w"), insert(true)),
        Some(&A::CloseTab)
    );
}

/// The facet view toggle ships with no chord on any platform, and a user can
/// bind it like any other action.
#[test]
fn the_facet_view_toggle_has_no_default_chord_and_can_be_bound() {
    for p in PLATFORMS {
        let keymap = Keymap::defaults(p);
        assert!(keymap.chords_for(&A::ToggleFacetView).is_empty(), "{p:?}");
        assert!(!keymap.ships_bound(&A::ToggleFacetView), "{p:?}");
    }
    let (bound, errors) = Keymap::load(
        LinuxWindows,
        &overrides(&[("ToggleFacetView", &["ctrl+alt+f"])]),
    );
    assert!(errors.is_empty(), "{errors:?}");
    let f = chord("ctrl+alt+f");
    assert_eq!(bound.chords_for(&A::ToggleFacetView), [f]);
    for panel in PANELS {
        assert_eq!(
            bound.resolve(f, ctx(panel, false)),
            Some(&A::ToggleFacetView),
            "{panel:?}"
        );
    }
}

/// The collection actions ship with no chord on any platform, and a user can
/// bind them like any other action.
#[test]
fn the_collection_actions_have_no_default_chord_and_can_be_bound() {
    for p in PLATFORMS {
        let keymap = Keymap::defaults(p);
        for action in [A::OpenQueryAsCollection, A::RefreshCollection] {
            assert!(keymap.chords_for(&action).is_empty(), "{p:?} {action:?}");
            assert!(!keymap.ships_bound(&action), "{p:?} {action:?}");
        }
    }
    let (bound, errors) = Keymap::load(
        LinuxWindows,
        &overrides(&[
            ("OpenQueryAsCollection", &["ctrl+alt+o"]),
            ("RefreshCollection", &["ctrl+alt+r"]),
        ]),
    );
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        bound.chords_for(&A::OpenQueryAsCollection),
        [chord("ctrl+alt+o")]
    );
    assert_eq!(
        bound.chords_for(&A::RefreshCollection),
        [chord("ctrl+alt+r")]
    );
    for panel in PANELS {
        assert_eq!(
            bound.resolve(chord("ctrl+alt+r"), ctx(panel, false)),
            Some(&A::RefreshCollection),
            "{panel:?}"
        );
    }
}

// ---------------------------------------------------------- the key draft

use draft::{CaptureError, KeyDraft, bindable_groups, check_capture, write_chord};

/// The draft's overrides as `(name, chords)` pairs, for a readable compare.
fn draft_overrides(draft: &KeyDraft) -> Vec<(String, Vec<String>)> {
    draft.overrides().into_iter().collect()
}

fn entry(name: &str, chords: &[&str]) -> (String, Vec<String>) {
    (
        name.to_string(),
        chords.iter().map(|c| c.to_string()).collect(),
    )
}

#[test]
fn assign_takes_the_chord_and_names_the_holder() {
    for p in PLATFORMS {
        let mut draft = KeyDraft::new(p, &BTreeMap::new());
        let e = parse_chord("primary+e", p).unwrap();
        assert_eq!(
            draft.holders(&A::OpenNote, e),
            [A::OpenFileBrowser],
            "{p:?}"
        );
        let taken = draft.assign(&A::OpenNote, e);
        assert_eq!(taken, [A::OpenFileBrowser], "{p:?}");
        assert_eq!(draft.chords(&A::OpenNote), [e], "{p:?}");
        // The holder loses the chord, and its vim twin with it: an
        // override is literal.
        assert!(draft.chords(&A::OpenFileBrowser).is_empty(), "{p:?}");
        assert_eq!(
            draft_overrides(&draft),
            [
                entry("OpenFileBrowser", &[]),
                entry("OpenNote", &["primary+e"]),
            ],
            "{p:?}"
        );
        assert!(draft.is_changed(&A::OpenNote) && draft.is_changed(&A::OpenFileBrowser));
        // The saved overrides load with no collision and the same chords.
        let (keymap, errors) = Keymap::load(p, &draft.overrides());
        assert!(errors.is_empty(), "{p:?}: {errors:?}");
        assert_eq!(keymap.chords_for(&A::OpenNote), [e]);
        assert!(keymap.chords_for(&A::OpenFileBrowser).is_empty());
    }
}

#[test]
fn assign_the_chord_the_action_already_holds_changes_nothing() {
    for p in PLATFORMS {
        let mut draft = KeyDraft::new(p, &BTreeMap::new());
        let k = parse_chord("primary+k", p).unwrap();
        assert!(draft.assign(&A::SearchNotes, k).is_empty(), "{p:?}");
        assert_eq!(
            draft.chords(&A::SearchNotes),
            Keymap::defaults(p).chords_for(&A::SearchNotes)
        );
        assert!(draft.overrides().is_empty(), "{p:?}");
        assert!(!draft.is_changed(&A::SearchNotes));
    }
}

#[test]
fn a_holder_with_two_chords_keeps_the_other() {
    for p in PLATFORMS {
        let mut draft = KeyDraft::new(p, &BTreeMap::new());
        let f = parse_chord("primary+shift+f", p).unwrap();
        assert_eq!(draft.assign(&A::NewNote, f), [A::SearchNotes], "{p:?}");
        assert_eq!(
            draft.chords(&A::SearchNotes),
            [parse_chord("primary+k", p).unwrap()],
            "{p:?}"
        );
        // On Linux and Windows the search on defaults kept Ctrl+Shift+F
        // from being find's vim twin. Overridden, it no longer does, so find
        // is pinned to its written chord: the same chords, not a change.
        let pinned: &[(String, Vec<String>)] = &[entry("FindInBuffer", &["primary+f"])];
        let expected: Vec<(String, Vec<String>)> = match p {
            LinuxWindows => pinned.to_vec(),
            MacOs => Vec::new(),
        }
        .into_iter()
        .chain([
            entry("NewNote", &["primary+shift+f"]),
            entry("SearchNotes", &["primary+k"]),
        ])
        .collect();
        assert_eq!(draft_overrides(&draft), expected, "{p:?}");
        assert!(!draft.is_changed(&A::FindInBuffer), "{p:?}");
        assert_eq!(draft.chords(&A::NewNote), [f], "{p:?}");
        let (keymap, errors) = Keymap::load(p, &draft.overrides());
        assert!(errors.is_empty(), "{p:?}: {errors:?}");
        assert_eq!(keymap.chords_for(&A::NewNote), [f], "{p:?}");
    }
}

#[test]
fn scopes_that_do_not_overlap_share_a_chord() {
    for p in PLATFORMS {
        let mut draft = KeyDraft::new(p, &BTreeMap::new());
        let f1 = chord("f1");
        assert!(draft.holders(&A::KeyboardHelp, f1).is_empty(), "{p:?}");
        assert!(draft.holders(&A::QuerySyntax, f1).is_empty(), "{p:?}");
        draft.clear(&A::QuerySyntax);
        assert!(draft.assign(&A::QuerySyntax, f1).is_empty(), "{p:?}");
        assert_eq!(draft.chords(&A::QuerySyntax), [f1]);
        assert_eq!(draft.chords(&A::KeyboardHelp), [f1]);
        assert!(draft.overrides().is_empty(), "{p:?}");
    }
}

#[test]
fn clear_then_reset_returns_to_default_and_leaves_no_override() {
    for p in PLATFORMS {
        let mut draft = KeyDraft::new(p, &BTreeMap::new());
        draft.clear(&A::OpenNote);
        assert!(draft.chords(&A::OpenNote).is_empty());
        assert!(draft.is_changed(&A::OpenNote));
        assert_eq!(draft_overrides(&draft), [entry("OpenNote", &[])]);
        assert!(draft.reset(&A::OpenNote).is_empty(), "{p:?}");
        assert_eq!(
            draft.chords(&A::OpenNote),
            Keymap::defaults(p).chords_for(&A::OpenNote),
            "{p:?}"
        );
        assert!(!draft.is_changed(&A::OpenNote));
        assert!(draft.overrides().is_empty(), "{p:?}");
        // Clearing an action that ships with no chord changes nothing.
        draft.clear(&A::ToggleFacetView);
        assert!(!draft.is_changed(&A::ToggleFacetView));
        assert!(draft.overrides().is_empty(), "{p:?}");
    }
}

#[test]
fn reset_takes_the_default_from_a_holder() {
    for p in PLATFORMS {
        let mut draft = KeyDraft::new(p, &BTreeMap::new());
        let e = parse_chord("primary+e", p).unwrap();
        draft.assign(&A::OpenNote, e);
        assert_eq!(draft.reset(&A::OpenFileBrowser), [A::OpenNote], "{p:?}");
        assert_eq!(
            draft.chords(&A::OpenFileBrowser),
            Keymap::defaults(p).chords_for(&A::OpenFileBrowser),
            "{p:?}"
        );
        assert!(draft.chords(&A::OpenNote).is_empty(), "{p:?}");
        assert_eq!(draft_overrides(&draft), [entry("OpenNote", &[])], "{p:?}");
        let (_, errors) = Keymap::load(p, &draft.overrides());
        assert!(errors.is_empty(), "{p:?}: {errors:?}");
    }
}

/// A reset gives back the vim twin too, so an action that took the twin
/// loses it, or the loader would drop its chord at the next load.
#[test]
fn reset_takes_the_vim_twin_back_from_a_holder() {
    let mut draft = KeyDraft::new(LinuxWindows, &BTreeMap::new());
    draft.clear(&A::OpenFileBrowser);
    assert!(draft.assign(&A::OpenNote, chord("ctrl+shift+e")).is_empty());
    assert_eq!(draft.reset(&A::OpenFileBrowser), [A::OpenNote]);
    assert_eq!(
        draft.chords(&A::OpenFileBrowser),
        [chord("ctrl+e"), chord("ctrl+shift+e")]
    );
    assert_eq!(draft_overrides(&draft), [entry("OpenNote", &[])]);
}

#[test]
fn overrides_drop_entries_equal_to_default() {
    // Read from a file: an entry equal to the default goes, an unknown name
    // stays as written (no row; the loader reports it), a real change stays.
    let file = overrides(&[
        ("OpenNote", &["primary+o"]),
        ("NoSuchAction", &["primary+g"]),
        ("FindInBuffer", &["primary+g"]),
    ]);
    let draft = KeyDraft::new(MacOs, &file);
    assert_eq!(
        draft_overrides(&draft),
        [
            entry("FindInBuffer", &["primary+g"]),
            entry("NoSuchAction", &["primary+g"]),
        ]
    );
    assert_eq!(draft.changes(), 1);
    assert!(!draft.is_changed(&A::OpenNote));
    assert!(draft.is_changed(&A::FindInBuffer));
    // Picking the default chord back on macOS (no twin) leaves no entry,
    // but the holder it was taken from stays cleared.
    let mut draft = KeyDraft::new(MacOs, &BTreeMap::new());
    draft.assign(&A::OpenNote, chord("cmd+e"));
    draft.assign(&A::OpenNote, chord("cmd+o"));
    assert_eq!(draft_overrides(&draft), [entry("OpenFileBrowser", &[])]);
    // reset_all forgets every change.
    draft.reset_all();
    assert!(draft.overrides().is_empty());
    assert_eq!(draft.chords(&A::OpenFileBrowser), [chord("cmd+e")]);
}

/// Picking the default chord again keeps the entry where the default has a
/// vim twin: the override is literal and drops the twin. Where it has none
/// (macOS), the entry goes.
#[test]
fn assigning_the_default_back_keeps_the_entry_only_where_a_twin_is_lost() {
    for p in PLATFORMS {
        let mut draft = KeyDraft::new(p, &BTreeMap::new());
        let o = parse_chord("primary+o", p).unwrap();
        draft.clear(&A::OpenNote);
        draft.assign(&A::OpenNote, o);
        let expected: Vec<(String, Vec<String>)> = match p {
            LinuxWindows => vec![entry("OpenNote", &["primary+o"])],
            MacOs => Vec::new(),
        };
        assert_eq!(draft_overrides(&draft), expected, "{p:?}");
        assert_eq!(draft.is_changed(&A::OpenNote), p == LinuxWindows, "{p:?}");
    }
}

/// Overriding an action can give another action its vim twin back, with
/// nothing taken. That row was not touched, so it is not changed.
#[test]
fn a_freed_vim_twin_does_not_mark_an_untouched_row_changed() {
    let p = LinuxWindows;
    let mut draft = KeyDraft::new(p, &BTreeMap::new());
    draft.clear(&A::QuickNote);
    assert_eq!(
        draft.chords(&A::CloseTab),
        [chord("ctrl+w"), chord("ctrl+shift+w")]
    );
    assert!(!draft.is_changed(&A::CloseTab), "{p:?}");
    assert!(draft.assign(&A::SearchNotes, chord("alt+s")).is_empty());
    assert!(!draft.is_changed(&A::FindInBuffer), "{p:?}");
    draft.clear(&A::NewDirectory);
    assert!(!draft.is_changed(&A::NewNote), "{p:?}");
    // A pinned holder keeps its default chords: not changed either.
    let mut draft = KeyDraft::new(p, &BTreeMap::new());
    draft.assign(&A::NewNote, chord("ctrl+shift+f"));
    assert!(draft.overrides().contains_key("FindInBuffer"), "{p:?}");
    assert!(!draft.is_changed(&A::FindInBuffer), "{p:?}");
}

#[test]
fn holders_is_empty_for_a_chord_the_action_holds() {
    let draft = KeyDraft::new(LinuxWindows, &BTreeMap::new());
    assert!(
        draft
            .holders(&A::SearchNotes, chord("ctrl+shift+f"))
            .is_empty()
    );
}

#[test]
fn check_capture_refuses_a_bare_character_and_a_bare_enter_tab_backspace() {
    let plain = |key| KeyInput::plain(key);
    assert_eq!(
        check_capture(plain(Key::Char('a'))),
        Err(CaptureError::NeedsModifier)
    );
    assert_eq!(
        check_capture(KeyInput::new(Key::Char('a'), Mods::SHIFT)),
        Err(CaptureError::NeedsModifier)
    );
    assert_eq!(
        check_capture(plain(Key::Char(' '))),
        Err(CaptureError::NeedsModifier)
    );
    for key in [
        Key::Enter,
        Key::Tab,
        Key::Backspace,
        Key::Esc,
        Key::Delete,
        Key::Left,
        Key::Home,
        Key::PageDown,
    ] {
        assert_eq!(
            check_capture(plain(key)),
            Err(CaptureError::Reserved),
            "{key:?}"
        );
    }
    assert_eq!(
        check_capture(KeyInput::new(Key::Enter, Mods::SHIFT)),
        Err(CaptureError::Reserved),
        "Shift+Enter is a plain newline"
    );
    assert_eq!(
        check_capture(KeyInput::new(Key::BackTab, Mods::SHIFT)),
        Err(CaptureError::Reserved)
    );
    for ok in [
        "f1",
        "shift+f6",
        "ctrl+e",
        "alt+l",
        "cmd+k",
        "ctrl+enter",
        "alt+left",
        "ctrl+tab",
    ] {
        assert_eq!(check_capture(chord(ok)), Ok(()), "{ok}");
    }
    // `+` joins the parts of a written chord: it cannot be saved.
    for plus in [
        KeyInput::ctrl('+'),
        KeyInput::new(Key::Char('+'), Mods::ALT),
    ] {
        assert_eq!(check_capture(plus), Err(CaptureError::PlusKey));
    }
    assert!(CaptureError::PlusKey.to_string().contains('+'));
    assert!(!CaptureError::NeedsModifier.to_string().is_empty());
    assert!(!CaptureError::Reserved.to_string().is_empty());
}

#[test]
fn write_chord_round_trips_with_parse_chord() {
    for p in PLATFORMS {
        // Every chord the default table binds, twins included.
        let keymap = Keymap::defaults(p);
        for binding in &keymap.bindings {
            let written = write_chord(binding.chord, p);
            assert_eq!(
                parse_chord(&written, p),
                Ok(binding.chord),
                "{p:?} {written}"
            );
        }
        // The defaults written with `primary` come back as written.
        for action in ActionShortcuts::ALL {
            for written in default_chords(&action, p).unwrap_or_default() {
                if written.starts_with("primary") {
                    let chord = parse_chord(written, p).unwrap();
                    assert_eq!(write_chord(chord, p), *written, "{p:?} {action:?}");
                }
            }
        }
        assert_eq!(write_chord(chord("ctrl+h"), LinuxWindows), "primary+h");
        assert_eq!(write_chord(chord("cmd+alt+f"), MacOs), "primary+alt+f");
    }
    assert_eq!(write_chord(chord("ctrl+tab"), MacOs), "ctrl+tab");
    assert_eq!(write_chord(chord("cmd+k"), LinuxWindows), "cmd+k");
    assert_eq!(
        write_chord(chord("ctrl+shift+tab"), LinuxWindows),
        "primary+shift+tab"
    );
    assert_eq!(
        write_chord(chord("ctrl+space"), LinuxWindows),
        "primary+space"
    );
    for key in [
        "escape",
        "backspace",
        "delete",
        "insert",
        "home",
        "end",
        "pageup",
        "pagedown",
        "f12",
    ] {
        let c = parse_chord(&format!("alt+{key}"), MacOs).unwrap();
        assert_eq!(write_chord(c, MacOs), format!("alt+{key}"));
    }
}

#[test]
fn every_action_is_listed_or_marked_not_bindable() {
    let rows: Vec<ActionShortcuts> = bindable_groups()
        .into_iter()
        .flat_map(|group| group.rows.into_iter().map(|row| row.action))
        .collect();
    for action in ActionShortcuts::ALL {
        let listed = rows.iter().filter(|a| **a == action).count();
        let expected = match action {
            A::Quit
            | A::OpenPreferences
            | A::SearchNotes
            | A::OpenNote
            | A::NewJournal
            | A::ToggleSidebar
            | A::OpenFileBrowser
            | A::FocusEditor
            | A::FocusSidebar
            | A::OpenSortDialog
            | A::FileOperations
            | A::FollowLink
            | A::QuickNote
            | A::YankRow
            | A::OpenSavedSearches
            | A::SaveCurrentQuery
            | A::OpenAsk
            | A::SwitchWorkspace
            | A::FindInBuffer
            | A::ReplaceInBuffer
            | A::Leader
            | A::OpenCommandPalette
            | A::FocusRegion(_)
            | A::ToggleRightDrawer
            | A::SaveNote
            | A::KeyboardHelp
            | A::QuerySyntax
            | A::SplitSlotRight
            | A::SplitSlotDown
            | A::CloseSlot
            | A::MoveViewToNeighbour(_)
            | A::ToggleZoom
            | A::OpenQueryInSidebar
            | A::ToggleRail
            | A::NewTab
            | A::CloseTab
            | A::NextTab
            | A::PrevTab
            | A::NewNote
            | A::NewDirectory
            | A::QueryBuilder
            // No default chord, but the GUI acts on them when bound: "no
            // chord until the user binds one".
            | A::ToggleFacetView
            | A::OpenQueryAsCollection
            | A::RefreshCollection => 1,
            // Nothing in the GUI acts on these. The FIND toggle is a drawer
            // view here; the desktop has the two New chords, not the menu;
            // formatting is on the palette and the leader, and a resolved
            // `Text` chord reaches no handler (ADR-0064).
            A::ToggleQueryPanel | A::NewEntry | A::Text(_) => 0,
        };
        assert_eq!(listed, expected, "{action:?}");
    }
    let titles: Vec<&str> = bindable_groups().iter().map(|g| g.title).collect();
    assert_eq!(
        titles,
        [
            "Notes",
            "Editor",
            "Layout and views",
            "Tabs",
            "Search",
            "Help and system"
        ]
    );
    assert!(
        bindable_groups()
            .iter()
            .all(|g| g.rows.iter().all(|r| !r.label.is_empty()))
    );
}

#[test]
fn a_row_names_where_its_chord_is_live() {
    let tag = |action: ActionShortcuts| {
        bindable_groups()
            .into_iter()
            .flat_map(|g| g.rows)
            .find(|r| r.action == action)
            .map(|r| r.scope_tag)
            .unwrap()
    };
    assert_eq!(tag(A::OpenNote), None);
    assert_eq!(tag(A::FindInBuffer), Some("editor only"));
    assert_eq!(tag(A::YankRow), Some("sidebar and rail only"));
    assert_eq!(tag(A::QuerySyntax), Some("where a query is typed"));
    assert_eq!(tag(A::OpenQueryInSidebar), Some("search browser only"));
}

// ------------------------------------------------------ the keymap file

mod keymap_file {
    use super::*;
    use crate::keymap::file::{FileError, export, import, read_file, write_file};
    use std::collections::BTreeMap;

    fn map(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|c| c.to_string()).collect()))
            .collect()
    }

    #[test]
    fn export_lists_cleared_actions_as_empty_arrays() {
        let text = export(
            &map(&[("ToggleRail", &[]), ("FindInBuffer", &["primary+alt+f"])]),
            "linux",
        );
        assert_eq!(
            text,
            "platform = \"linux\"\n\n[keys]\nFindInBuffer = [\"primary+alt+f\"]\nToggleRail = []\n"
        );
    }

    #[test]
    fn export_with_no_changes_is_an_empty_keys_table() {
        assert_eq!(
            export(&BTreeMap::new(), "macos"),
            "platform = \"macos\"\n\n[keys]\n"
        );
    }

    #[test]
    fn import_round_trips_export() {
        for platform in PLATFORMS {
            let overrides = map(&[
                ("FindInBuffer", &["primary+alt+j", "primary+alt+g"]),
                ("ToggleRail", &[]),
            ]);
            let got = import(&export(&overrides, "linux"), platform).unwrap();
            assert_eq!(got.overrides, overrides, "{platform:?}");
            assert!(got.dropped.is_empty(), "{platform:?}: {:?}", got.dropped);
        }
    }

    #[test]
    fn import_from_the_other_family_skips_the_source_defaults() {
        // A macOS file on Linux: the sidebar toggle equals the macOS default
        // (Cmd+Option+L), so it is no choice of the user. Open note is.
        let text = "platform = \"macos\"\n[keys]\n\
                    ToggleSidebar = [\"primary+alt+l\"]\nOpenNote = [\"primary+alt+o\"]\n";
        let got = import(text, LinuxWindows).unwrap();
        assert_eq!(got.overrides, map(&[("OpenNote", &["primary+alt+o"])]));
        assert_eq!(got.other_defaults, Some((1, "macOS")));
    }

    #[test]
    fn import_of_a_linux_file_on_a_mac_skips_the_pinned_twins() {
        let text = "platform = \"linux\"\n[keys]\n\
                    OpenNote = [\"primary+o\", \"primary+shift+o\"]\n";
        let got = import(text, MacOs).unwrap();
        assert!(got.overrides.is_empty(), "{:?}", got.overrides);
        assert_eq!(got.other_defaults, Some((1, "Linux")));
    }

    #[test]
    fn import_from_the_same_family_keeps_every_line() {
        // Linux and Windows share one table: a Windows file on Linux keeps
        // its defaults, so a changed default later is overridden by the file.
        let text =
            "platform = \"windows\"\n[keys]\nOpenNote = [\"primary+o\", \"primary+shift+o\"]\n";
        let got = import(text, LinuxWindows).unwrap();
        assert_eq!(got.overrides.len(), 1);
        assert_eq!(got.other_defaults, None);
    }

    #[test]
    fn import_without_a_known_label_keeps_every_line() {
        for head in ["", "platform = \"beos\"\n", "platform = 3\n"] {
            let text = format!("{head}[keys]\nToggleSidebar = [\"primary+alt+l\"]\n");
            let got = import(&text, LinuxWindows).unwrap();
            assert_eq!(got.overrides.len(), 1, "{head:?}");
            assert_eq!(got.other_defaults, None, "{head:?}");
        }
    }

    #[test]
    fn import_an_empty_keys_table_has_no_overrides() {
        let got = import("[keys]\n", LinuxWindows).unwrap();
        assert!(got.overrides.is_empty());
        assert!(got.dropped.is_empty());
    }

    #[test]
    fn import_reads_a_single_string_as_one_chord() {
        let got = import("[keys]\nToggleRail = \"primary+alt+r\"\n", LinuxWindows).unwrap();
        assert_eq!(got.overrides, map(&[("ToggleRail", &["primary+alt+r"])]));
    }

    #[test]
    fn import_drops_an_unknown_action_and_a_bad_chord_and_keeps_the_rest() {
        let text = "[keys]\nNoSuchAction = [\"primary+alt+r\"]\n\
                    ToggleRail = [\"primary+alt+r\", \"primary+nonsense\"]\n";
        let got = import(text, LinuxWindows).unwrap();
        assert_eq!(got.dropped.len(), 2, "{:?}", got.dropped);
        assert!(got.dropped.iter().any(|e| matches!(
            e,
            KeymapError::UnknownAction { name } if name == "NoSuchAction"
        )));
        assert!(got.dropped.iter().any(|e| matches!(
            e,
            KeymapError::InvalidChord { action: A::ToggleRail, chord, .. }
                if chord == "primary+nonsense"
        )));
        assert_eq!(
            got.overrides,
            map(&[("ToggleRail", &["primary+alt+r"])]),
            "what the loader could not read is not handed on"
        );
    }

    #[test]
    fn import_keeps_an_action_whose_only_chord_is_bad_as_cleared() {
        // The loader reads `["primary+nonsense"]` as "no chord": the override
        // stays, with no chord, as the loader applies it.
        let got = import(
            "[keys]\nToggleRail = [\"primary+nonsense\"]\n",
            LinuxWindows,
        )
        .unwrap();
        assert_eq!(got.overrides, map(&[("ToggleRail", &[])]));
        assert_eq!(got.dropped.len(), 1, "{:?}", got.dropped);
    }

    #[test]
    fn import_reports_no_clash_for_the_draft_resolves_it() {
        // The draft gives the chord to a line, so the loader's clash is no loss.
        let text = "[keys]\nToggleRail = [\"primary+alt+j\"]\nToggleZoom = [\"primary+alt+j\"]\n";
        let got = import(text, LinuxWindows).unwrap();
        assert!(got.dropped.is_empty(), "{:?}", got.dropped);
        // The text is kept as written.
        assert_eq!(got.overrides.len(), 2);
    }

    #[test]
    fn replace_takes_a_chord_from_the_default_holder() {
        // `ctrl+o` is Open note by default: a file line gives it to another
        // action, and Open note is written with what it keeps.
        let mut draft = KeyDraft::new(LinuxWindows, &BTreeMap::new());
        let taken = draft.replace(&map(&[("ToggleRail", &["primary+o"])]));
        assert!(
            taken.contains(&(A::ToggleRail, chord("ctrl+o"), A::OpenNote)),
            "{taken:?}"
        );
        assert_eq!(draft.chords(&A::ToggleRail), vec![chord("ctrl+o")]);
        assert!(!draft.chords(&A::OpenNote).contains(&chord("ctrl+o")));
        // It holds after a reload of what the draft saves.
        let saved = draft.overrides();
        let (keymap, dropped) = Keymap::load(LinuxWindows, &saved);
        assert!(dropped.is_empty(), "{dropped:?}");
        assert_eq!(keymap.chords_for(&A::ToggleRail), vec![chord("ctrl+o")]);
    }

    #[test]
    fn replace_gives_a_clash_inside_the_file_to_one_line_only() {
        let mut draft = KeyDraft::new(LinuxWindows, &BTreeMap::new());
        let text = map(&[
            ("ToggleRail", &["primary+alt+j"]),
            ("ToggleZoom", &["primary+alt+j"]),
        ]);
        draft.replace(&text);
        let holders = [A::ToggleRail, A::ToggleZoom]
            .iter()
            .filter(|a| draft.chords(a).contains(&chord("ctrl+alt+j")))
            .count();
        assert_eq!(holders, 1);
    }

    #[test]
    fn import_rejects_text_that_is_not_toml() {
        let err = import("[keys\nnot toml", LinuxWindows).unwrap_err();
        assert!(matches!(err, FileError::NotToml(_)), "{err:?}");
    }

    #[test]
    fn import_rejects_a_file_without_a_keys_table() {
        assert_eq!(
            import("[other]\nx = 1\n", LinuxWindows).unwrap_err(),
            FileError::NoKeysTable
        );
        assert_eq!(
            import("keys = 5\n", LinuxWindows).unwrap_err(),
            FileError::NoKeysTable
        );
    }

    #[test]
    fn import_accepts_a_cmd_chord_on_linux() {
        // The brief expected the loader to report it. It does not: `cmd` is
        // the Super key and parses on every platform.
        let got = import("[keys]\nToggleRail = [\"cmd+alt+r\"]\n", LinuxWindows).unwrap();
        assert_eq!(got.dropped, Vec::<KeymapError>::new());
    }

    #[test]
    fn write_file_then_read_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keymap.toml");
        let overrides = map(&[("ToggleRail", &[]), ("FindInBuffer", &["primary+alt+f"])]);
        write_file(&path, &overrides).unwrap();
        let got = read_file(&path, LinuxWindows).unwrap();
        assert_eq!(got.overrides, overrides);
        assert!(got.dropped.is_empty());
        let missing = read_file(&dir.path().join("none.toml"), LinuxWindows);
        assert!(matches!(missing, Err(FileError::Unreadable(_))));
    }
}

/// An override for an action this build does not know (a typo, or one of
/// a newer build) is no row of the draft, but the draft hands it back
/// untouched: an edit, Reset all or an import never deletes it.
#[test]
fn the_draft_keeps_overrides_for_unknown_actions() {
    let mut overrides = BTreeMap::new();
    overrides.insert(
        "FromANewerBuild".to_string(),
        vec!["primary+alt+j".to_string()],
    );
    let mut draft = KeyDraft::new(LinuxWindows, &overrides);
    assert_eq!(draft.overrides(), overrides, "loaded as written");
    assert!(!draft.has_changes(), "no row changed");
    draft.assign(&A::ToggleFacetView, KeyInput::ctrl('g'));
    assert_eq!(
        draft.overrides().get("FromANewerBuild"),
        Some(&vec!["primary+alt+j".to_string()]),
        "an edit keeps it"
    );
    assert!(draft.has_changes());
    draft.reset_all();
    assert_eq!(draft.overrides(), overrides, "Reset all keeps it");
    let mut imported = BTreeMap::new();
    imported.insert("OpenNote".to_string(), vec!["primary+g".to_string()]);
    draft.replace(&imported);
    let mut want = overrides.clone();
    want.insert("OpenNote".to_string(), vec!["primary+g".to_string()]);
    assert_eq!(draft.overrides(), want, "an import keeps it");
    assert_eq!(draft.chords(&A::OpenNote), [KeyInput::ctrl('g')]);
    assert_eq!(draft.changes(), 1);
}

#[test]
fn a_query_chord_over_an_everywhere_chord_is_shared_not_taken() {
    use draft::Shared;
    for p in PLATFORMS {
        let mut draft = KeyDraft::new(p, &BTreeMap::new());
        let f7 = parse_chord("f7", p).unwrap();
        assert!(draft.holders(&A::OpenAsk, f7).is_empty(), "{p:?}");
        assert!(draft.assign(&A::OpenAsk, f7).is_empty(), "{p:?}");
        assert_eq!(draft.chords(&A::QueryBuilder), [f7], "{p:?}");
        assert_eq!(
            draft.shared(&A::OpenAsk, f7),
            [Shared {
                other: A::QueryBuilder,
                other_wins_in: Some("Where a query is typed"),
            }],
            "{p:?}"
        );
        // From the query side: the other action has it everywhere else.
        assert_eq!(
            draft.shared(&A::QueryBuilder, f7),
            [Shared {
                other: A::OpenAsk,
                other_wins_in: None,
            }],
            "{p:?}"
        );
        // The browser's chord over an everywhere one.
        let f5 = parse_chord("f5", p).unwrap();
        draft.assign(&A::OpenQueryInSidebar, f5);
        assert_eq!(
            draft.shared(&A::SwitchWorkspace, f5),
            [Shared {
                other: A::OpenQueryInSidebar,
                other_wins_in: Some("In the search browser"),
            }],
            "{p:?}"
        );
        // Editor and Shell never meet: nothing is shared.
        let g = parse_chord("primary+g", p).unwrap();
        draft.assign(&A::FindInBuffer, g);
        draft.assign(&A::YankRow, g);
        assert!(draft.shared(&A::YankRow, g).is_empty(), "{p:?}");
        assert!(
            draft
                .shared(&A::OpenNote, parse_chord("primary+o", p).unwrap())
                .is_empty()
        );
    }
}

#[test]
fn the_draft_says_when_vim_reads_a_chord_first() {
    let draft = KeyDraft::new(Platform::LinuxWindows, &BTreeMap::new());
    let v = parse_chord("ctrl+v", Platform::LinuxWindows).unwrap();
    assert!(draft.vim_shadows(&A::OpenNote, v));
    assert!(
        !draft.vim_shadows(&A::YankRow, v),
        "Shell is never the editor"
    );
    let mac = KeyDraft::new(Platform::MacOs, &BTreeMap::new());
    assert!(!mac.vim_shadows(&A::OpenNote, v));
}

#[test]
fn a_chord_the_plain_editor_edits_with_is_flagged_for_editor_actions() {
    let chord = |w: &str| parse_chord(w, Platform::LinuxWindows).unwrap();
    for edits in [
        "ctrl+z",
        "ctrl+shift+z",
        "ctrl+y",
        "ctrl+a",
        "ctrl+c",
        "ctrl+x",
        "ctrl+v",
        "ctrl+left",
        "ctrl+shift+right",
        "ctrl+backspace",
        "alt+b",
        "cmd+c",
        "cmd+shift+z",
        "cmd+up",
    ] {
        assert!(
            draft::takes_editing_key(&A::OpenNote, chord(edits)),
            "{edits}"
        );
    }
    assert!(draft::takes_editing_key(&A::FindInBuffer, chord("ctrl+z")));
    for plain in ["ctrl+g", "ctrl+shift+c", "alt+z", "f7"] {
        assert!(
            !draft::takes_editing_key(&A::OpenNote, chord(plain)),
            "{plain}"
        );
    }
    assert!(
        !draft::takes_editing_key(&A::YankRow, chord("ctrl+z")),
        "Shell is never the editor"
    );
    assert!(!draft::takes_editing_key(&A::QueryBuilder, chord("ctrl+z")));
}
