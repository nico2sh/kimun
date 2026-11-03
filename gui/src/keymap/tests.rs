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
    KeyCtx { panel, vim }
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
            lw("ctrl+t", A::ToggleSidebar),
            lw("ctrl+shift+t", A::ToggleSidebar),
            lw("ctrl+e", A::OpenFileBrowser),
            lw("ctrl+shift+e", A::OpenFileBrowser),
            lw("ctrl+tab", A::FocusEditor),
            lw("ctrl+shift+tab", A::FocusSidebar),
            lw("ctrl+n", A::FollowLink),
            lw("ctrl+shift+n", A::FollowLink),
            lw("ctrl+enter", A::FollowLink),
            lw("ctrl+f", A::FindInBuffer),
            lw("ctrl+shift+f", A::FindInBuffer),
            lw("ctrl+h", A::ReplaceInBuffer),
            lw("ctrl+shift+h", A::ReplaceInBuffer),
            lw("alt+left", A::FocusRegion(Dir::Left)),
            lw("alt+right", A::FocusRegion(Dir::Right)),
            lw("alt+up", A::FocusRegion(Dir::Up)),
            lw("alt+down", A::FocusRegion(Dir::Down)),
            lw("ctrl+s", A::SaveNote),
            lw("ctrl+shift+s", A::SaveNote),
        ]
    );
    assert_eq!(
        table(&Keymap::defaults(MacOs)),
        vec![
            lw("cmd+t", A::ToggleSidebar),
            lw("cmd+e", A::OpenFileBrowser),
            lw("ctrl+tab", A::FocusEditor),
            lw("ctrl+shift+tab", A::FocusSidebar),
            lw("cmd+n", A::FollowLink),
            lw("cmd+enter", A::FollowLink),
            lw("cmd+f", A::FindInBuffer),
            lw("cmd+alt+f", A::ReplaceInBuffer),
            lw("cmd+alt+left", A::FocusRegion(Dir::Left)),
            lw("cmd+alt+right", A::FocusRegion(Dir::Right)),
            lw("cmd+alt+up", A::FocusRegion(Dir::Up)),
            lw("cmd+alt+down", A::FocusRegion(Dir::Down)),
            lw("cmd+s", A::SaveNote),
        ]
    );
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

/// Follow link (M5b-2 G4) is primary+N and primary+Enter in the editor,
/// under vim too (the vim engine reads neither Ctrl+N nor Ctrl+Enter), and
/// nothing elsewhere. Shift+Enter stays the editor's plain newline.
#[test]
fn follow_link_is_primary_n_or_primary_enter_in_the_editor() {
    let lw = Keymap::defaults(LinuxWindows);
    for written in ["ctrl+n", "ctrl+shift+n", "ctrl+enter"] {
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
    for written in ["cmd+n", "cmd+enter"] {
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
/// marked-bound action really holds a chord.
#[test]
fn every_shared_action_is_bound_or_marked_unbound() {
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

/// Every bound action can be triggered from every panel its scope covers, in
/// plain and in vim mode — the vim twins are what keep the editor-scoped
/// ones reachable under vim on Linux/Windows.
#[test]
fn every_default_action_is_reachable_in_every_context() {
    for p in PLATFORMS {
        let keymap = Keymap::defaults(p);
        for action in ActionShortcuts::ALL {
            if default_chords(&action, p).is_none() {
                continue;
            }
            for panel in PANELS {
                if !Scope::of(&action).applies(panel) {
                    continue;
                }
                for vim in [false, true] {
                    let reachable = keymap
                        .bindings
                        .iter()
                        .filter(|b| b.action == action)
                        .any(|b| keymap.resolve(b.chord, ctx(panel, vim)) == Some(&action));
                    assert!(
                        reachable,
                        "{p:?} {action} unreachable in {panel:?} vim={vim}"
                    );
                }
            }
        }
    }
}

/// The exhaustive resolution table: each probe chord in the editor (plain,
/// vim), in a drawer and on the rail. A drawer or the rail resolves the same
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
    let files = Some(A::OpenFileBrowser);
    let follow = Some(A::FollowLink);
    resolution(
        LinuxWindows,
        &[
            //                 plain editor  vim editor  drawer / rail
            ("ctrl+f", [find.clone(), find.clone(), None]),
            ("ctrl+shift+f", [find.clone(), find, None]),
            ("ctrl+h", [replace.clone(), replace.clone(), None]),
            ("ctrl+shift+h", [replace.clone(), replace, None]),
            ("ctrl+t", [toggle.clone(), toggle.clone(), toggle.clone()]),
            ("ctrl+shift+t", [toggle.clone(), toggle.clone(), toggle]),
            ("ctrl+e", [files.clone(), files.clone(), files.clone()]),
            ("ctrl+shift+e", [files.clone(), files.clone(), files]),
            ("ctrl+n", [follow.clone(), follow.clone(), None]),
            ("ctrl+shift+n", [follow.clone(), follow.clone(), None]),
            ("ctrl+enter", [follow.clone(), follow, None]),
            // Vim reads these: the app gives them up in the vim editor.
            ("ctrl+r", [None, None, None]),
            ("alt+left", [left.clone(), left.clone(), left]),
            ("alt+right", [right.clone(), right.clone(), right]),
            ("alt+up", [up.clone(), up.clone(), up]),
            ("alt+down", [down.clone(), down.clone(), down]),
            ("ctrl+tab", [next.clone(), next.clone(), next]),
            ("ctrl+shift+tab", [prev.clone(), prev.clone(), prev]),
            // Not bound here: they reach the editor (or nothing).
            ("cmd+f", [None, None, None]),
            ("cmd+alt+left", [None, None, None]),
            ("ctrl+left", [None, None, None]),
            ("ctrl+d", [None, None, None]),
            ("alt+f", [None, None, None]),
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
    let files = Some(A::OpenFileBrowser);
    let follow = Some(A::FollowLink);
    resolution(
        MacOs,
        &[
            //                 plain editor  vim editor  drawer / rail
            ("cmd+f", [find.clone(), find, None]),
            ("cmd+alt+f", [replace.clone(), replace, None]),
            ("cmd+t", [toggle.clone(), toggle.clone(), toggle]),
            ("cmd+e", [files.clone(), files.clone(), files]),
            ("cmd+n", [follow.clone(), follow.clone(), None]),
            ("cmd+enter", [follow.clone(), follow, None]),
            ("cmd+alt+left", [left.clone(), left.clone(), left]),
            ("cmd+alt+right", [right.clone(), right.clone(), right]),
            ("cmd+alt+up", [up.clone(), up.clone(), up]),
            ("cmd+alt+down", [down.clone(), down.clone(), down]),
            ("ctrl+tab", [next.clone(), next.clone(), next]),
            ("ctrl+shift+tab", [prev.clone(), prev.clone(), prev]),
            // Ctrl is literal and free on macOS: the editor's (and vim's).
            ("ctrl+f", [None, None, None]),
            ("ctrl+h", [None, None, None]),
            ("ctrl+t", [None, None, None]),
            ("ctrl+e", [None, None, None]),
            ("ctrl+n", [None, None, None]),
            ("ctrl+enter", [None, None, None]),
            ("cmd+h", [None, None, None]),
            ("alt+left", [None, None, None]),
            ("cmd+shift+f", [None, None, None]),
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
        keymap.resolve(chord("ctrl+t"), ctx(Panel::Rail, false)),
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
        ("ToggleRightDrawer", &["ctrl+banana", "alt+r"]),
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
        keymap.resolve(chord("alt+r"), rail),
        Some(&A::ToggleRightDrawer)
    );
    // Defaults untouched.
    assert_eq!(
        keymap.resolve(chord("ctrl+t"), rail),
        Some(&A::ToggleSidebar)
    );
}

#[test]
fn a_chord_held_by_another_action_is_rejected_not_stolen() {
    // Ctrl+T is ToggleSidebar's default; the override loses, the default stays.
    let o = overrides(&[("ToggleRightDrawer", &["primary+t", "primary+shift+r"])]);
    let (keymap, errors) = Keymap::load(LinuxWindows, &o);
    assert_eq!(
        errors,
        vec![KeymapError::Collision {
            action: A::ToggleRightDrawer,
            chord: "primary+t".into(),
            taken_by: A::ToggleSidebar,
        }]
    );
    let drawer = ctx(Panel::Drawer(Side::Left), false);
    assert_eq!(
        keymap.resolve(chord("ctrl+t"), drawer),
        Some(&A::ToggleSidebar)
    );
    assert_eq!(
        keymap.resolve(chord("ctrl+shift+r"), drawer),
        Some(&A::ToggleRightDrawer)
    );
}

#[test]
fn two_overrides_on_one_chord_keep_the_first_in_action_order() {
    let o = overrides(&[
        ("SplitDrawerSlot", &["alt+s"]),
        ("MoveViewToLeftDrawer", &["alt+s"]),
    ]);
    let (keymap, errors) = Keymap::load(MacOs, &o);
    // MoveViewToLeftDrawer sorts before SplitDrawerSlot.
    assert_eq!(
        errors,
        vec![KeymapError::Collision {
            action: A::SplitDrawerSlot,
            chord: "alt+s".into(),
            taken_by: A::MoveViewToLeftDrawer,
        }]
    );
    assert_eq!(
        keymap.resolve(chord("alt+s"), ctx(Panel::Rail, false)),
        Some(&A::MoveViewToLeftDrawer)
    );
}

/// Moving a chord between actions works when the holder is rebound too.
#[test]
fn rebinding_the_holder_frees_its_chord() {
    let o = overrides(&[
        ("ToggleSidebar", &["primary+b"]),
        ("FindInBuffer", &["primary+t"]),
    ]);
    let (keymap, errors) = Keymap::load(LinuxWindows, &o);
    assert!(errors.is_empty(), "{errors:?}");
    let editor = ctx(Panel::Editor, false);
    assert_eq!(
        keymap.resolve(chord("ctrl+t"), editor),
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
        chord: "primary+t".into(),
        taken_by: A::ToggleSidebar,
    };
    assert_eq!(
        e.to_string(),
        "[gui.keys] ToggleRightDrawer = \"primary+t\": already bound to ToggleSidebar"
    );
}

#[test]
fn the_current_platform_matches_the_build_target() {
    assert_eq!(Platform::current() == MacOs, cfg!(target_os = "macos"));
}
