//! UI integration tests for layout presets (ADR-0052 §5): `layouts.toml`
//! beside `config.toml` is read when the window opens and on "Reload layout
//! presets", applied by "Apply layout preset: <name>" into the live
//! `[gui.layout]`, and re-applied by "Reset to preset". The app reads the
//! file and never writes it. A real `Workspace` in a headless window.
#![cfg(test)]

use std::path::{Path, PathBuf};

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext};
use kimun_notes::logic::layout::Panel;
use kimun_notes::logic::layout_tree::spec::{self, ChildSpec};
use kimun_notes::logic::layout_tree::{Axis, LayoutTree, Leaf};
use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::rag_caps::Caps;
use kimun_notes::settings::gui::GuiSettings;

use kimun_notes::rag::RagStatus;

use super::palette::PaletteRun;
use super::presets::{self, PRESETS_FILE, PresetCommand};
use super::tests::{Shell, open_shell, saved, temp_config};
use super::{LayoutStore, rail_cell_id, slot_id};
use crate::shell_tree;

/// A TOML document, one line per entry.
fn lines(lines: &[&str]) -> String {
    lines.join("\n") + "\n"
}

/// Two valid presets: "research" (files | editor | outline over links and
/// tags) and "focus" (editor | tags, weighted 3 : 1).
fn two_presets() -> String {
    lines(&[
        "[preset.research]",
        "split = \"horizontal\"",
        "children = [",
        "    { weight = 0.2, views = [\"files\"], shown = \"files\" },",
        "    { weight = 0.6, editor = true },",
        "    { weight = 0.2, split = \"vertical\", children = [",
        "        { views = [\"outline\"], shown = \"outline\" },",
        "        { views = [\"links\", \"tags\"], shown = \"links\" },",
        "    ] },",
        "]",
        "",
        "[preset.focus]",
        "split = \"horizontal\"",
        "children = [",
        "    { weight = 3, editor = true },",
        "    { views = [\"tags\"], shown = \"tags\" },",
        "]",
    ])
}

/// The presets file in `config`'s directory.
fn presets_path(config: &Path) -> PathBuf {
    config.parent().unwrap().join(PRESETS_FILE)
}

/// A shell persisting to `config`, with `presets` (when given) written to
/// `layouts.toml` beside it first.
fn open_with(cx: &mut TestAppContext, config: &Path, presets: Option<&str>) -> Shell {
    if let Some(text) = presets {
        std::fs::write(presets_path(config), text).unwrap();
    }
    open_shell(
        cx,
        1200.,
        &GuiSettings::default(),
        "",
        false,
        LayoutStore::File(config.to_path_buf()),
    )
}

/// Run a preset command as its palette row does.
fn run(shell: &Shell, cx: &mut TestAppContext, command: PresetCommand) {
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.run_palette_row(PaletteRun::Preset(command), window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
}

fn apply(shell: &Shell, cx: &mut TestAppContext, name: &str) {
    run(shell, cx, PresetCommand::Apply(name.to_string()));
}

/// Every flash shown so far.
fn flashes(shell: &Shell, cx: &mut TestAppContext) -> Vec<(String, bool)> {
    cx.read(|cx| {
        shell
            .workspace
            .read(cx)
            .flashes
            .iter()
            .map(|(text, error)| (text.to_string(), *error))
            .collect()
    })
}

fn live(shell: &Shell, cx: &mut TestAppContext) -> LayoutTree<DrawerView> {
    cx.read(|cx| shell.workspace.read(cx).layout.clone())
}

/// What `name` of `text` builds (all views available but SEM and ASK).
fn built(text: &str, name: &str) -> LayoutTree<DrawerView> {
    let file = spec::parse_presets(text).unwrap();
    spec::build(&file.preset[name], Caps::default()).unwrap().0
}

/// The layout the saved `[gui]` restores.
fn saved_tree(config: &Path) -> LayoutTree<DrawerView> {
    shell_tree::restore(&saved(config), shell_tree::RESTORE_CAPS)
}

/// The palette rows' labels and why each is blocked.
fn rows(shell: &Shell, cx: &mut TestAppContext) -> Vec<(String, Option<&'static str>)> {
    cx.read(|cx| {
        shell
            .workspace
            .read(cx)
            .palette_entries(Panel::Editor, cx)
            .iter()
            .map(|e| (e.label().to_string(), e.blocked()))
            .collect()
    })
}

#[test]
fn missing_presets_file_is_no_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let load = presets::load(dir.path());
    assert!(load.presets.is_empty());
    assert_eq!(load.error, None);
    // An empty file, or one with only comments, is no presets either.
    std::fs::write(dir.path().join(PRESETS_FILE), "").unwrap();
    let load = presets::load(dir.path());
    assert!(load.presets.is_empty());
    assert_eq!(load.error, None);
    std::fs::write(dir.path().join(PRESETS_FILE), "# none yet\n").unwrap();
    assert_eq!(presets::load(dir.path()).error, None);
    // Valid presets load by name.
    std::fs::write(dir.path().join(PRESETS_FILE), two_presets()).unwrap();
    let load = presets::load(dir.path());
    assert_eq!(load.error, None);
    assert_eq!(
        load.presets.keys().collect::<Vec<_>>(),
        ["focus", "research"]
    );
}

/// A window opened with no presets file says nothing about it, offers no
/// preset to apply, and keeps "Reload" (and a disabled "Reset").
#[gpui_kit::test]
fn a_window_without_presets_flashes_nothing(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let shell = open_with(cx, &config, None);
    assert!(flashes(&shell, cx).is_empty());
    let listed = rows(&shell, cx);
    assert!(
        !listed
            .iter()
            .any(|(l, _)| l.starts_with("Apply layout preset"))
    );
    assert!(listed.contains(&("Reload layout presets".to_string(), None)));
    assert!(listed.contains(&(
        "Reset to preset".to_string(),
        Some(presets::NO_ACTIVE_PRESET)
    )));
}

#[gpui_kit::test]
fn malformed_toml_flashes_once_and_keeps_current_layout(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let shell = open_with(cx, &config, Some("[preset.research\nsplit = 1\n"));
    let before = live(&shell, cx);
    assert_eq!(before, LayoutTree::default_layout(DrawerView::Files));
    for _ in 0..3 {
        cx.update_window(shell.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
    let shown = flashes(&shell, cx);
    assert_eq!(shown.len(), 1, "{shown:?}");
    let (text, error) = &shown[0];
    assert!(*error);
    assert!(text.contains(PRESETS_FILE), "{text}");
    // The TOML parser's own message, with its line and column.
    assert!(text.contains("line 1"), "{text}");
    assert_eq!(live(&shell, cx), before);
    assert!(
        !rows(&shell, cx)
            .iter()
            .any(|(l, _)| l.starts_with("Apply layout preset"))
    );
    // A reload of the still-broken file says so again, once.
    run(&shell, cx, PresetCommand::Reload);
    assert_eq!(flashes(&shell, cx).len(), 2);
    assert_eq!(live(&shell, cx), before);
}

#[gpui_kit::test]
fn a_typo_in_a_leaf_names_the_preset_and_key(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let text = lines(&[
        "[preset.research]",
        "split = \"horizontal\"",
        "children = [",
        "    { veiws = [\"files\"] },",
        "    { editor = true },",
        "]",
    ]);
    let shell = open_with(cx, &config, Some(&text));
    let shown = flashes(&shell, cx);
    assert_eq!(shown.len(), 1, "{shown:?}");
    let message = &shown[0].0;
    assert!(!message.contains("untagged"), "{message}");
    assert!(message.contains("\"research\""), "{message}");
    assert!(message.contains("children[0]"), "{message}");
    assert!(message.contains("\"veiws\""), "{message}");
}

#[gpui_kit::test]
fn apply_preset_copies_into_live_state_and_saves(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let shell = open_with(cx, &config, Some(&two_presets()));
    assert!(flashes(&shell, cx).is_empty());
    let listed = rows(&shell, cx);
    for name in ["focus", "research"] {
        let label = format!("Apply layout preset: {name}");
        assert!(listed.contains(&(label, None)), "{listed:?}");
    }

    apply(&shell, cx, "research");
    let want = built(&two_presets(), "research");
    let got = live(&shell, cx);
    assert_eq!(spec::to_spec(&got), spec::to_spec(&want));
    let slots = got.slots();
    assert_eq!(slots.len(), 3);
    assert_eq!(got.shown(slots[2]), Some(DrawerView::Links));
    // The live state on disk is the preset's, and names it.
    let gui = saved(&config);
    assert_eq!(gui.layout_preset().as_deref(), Some("research"));
    assert_eq!(spec::to_spec(&saved_tree(&config)), spec::to_spec(&want));
    // No flash for a clean apply.
    assert!(flashes(&shell, cx).is_empty());
    // Focus rests on the editor of the new tree.
    assert_eq!(
        got.focused(),
        kimun_notes::logic::layout_tree::Focus::Leaf(Leaf::Group(got.active_group()))
    );
    // "Reset to preset" is offered now.
    assert!(rows(&shell, cx).contains(&("Reset to preset".to_string(), None)));
}

/// A preset naming SEM, applied before the server reports (caps still
/// the default, SEM not offered), keeps SEM where the preset puts it, live
/// and in the saved `[gui.layout]`, with no notice: availability is the
/// rail's concern, as for the startup restore (Ruling 18). When the server
/// then says it can search, SEM's cell comes and the view stays shown.
#[gpui_kit::test]
fn apply_preset_keeps_a_view_the_server_does_not_offer_yet(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let text = lines(&[
        "[preset.sem]",
        "split = \"horizontal\"",
        "children = [",
        "    { views = [\"files\", \"semantic\"], shown = \"semantic\" },",
        "    { editor = true },",
        "]",
    ]);
    let shell = open_with(cx, &config, Some(&text));
    assert!(!cx.read(|cx| shell.workspace.read(cx).rag_caps().semantic));
    apply(&shell, cx, "sem");
    let got = live(&shell, cx);
    let left = got.slots()[0];
    assert_eq!(
        got.slot_views(left),
        [DrawerView::Files, DrawerView::Semantic]
    );
    assert_eq!(got.shown(left), Some(DrawerView::Semantic));
    assert!(flashes(&shell, cx).is_empty(), "{:?}", flashes(&shell, cx));
    let back = saved_tree(&config);
    assert_eq!(
        back.slot_views(back.slots()[0]),
        [DrawerView::Files, DrawerView::Semantic]
    );
    assert_eq!(back.shown(back.slots()[0]), Some(DrawerView::Semantic));
    assert_eq!(saved(&config).layout_preset().as_deref(), Some("sem"));

    // The server comes up: SEM is offered on the rail and still shown.
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            w.on_rag_status(
                RagStatus::Online {
                    llm_available: false,
                },
                cx,
            )
        });
        window.render_frame(cx);
        assert!(
            window
                .try_find(rail_cell_id(DrawerView::Semantic))
                .is_some()
        );
        assert!(window.try_find(slot_id(DrawerView::Semantic)).is_some());
    })
    .unwrap();
    assert_eq!(live(&shell, cx).shown(left), Some(DrawerView::Semantic));
}

/// Before the server reports, a preset placing SEM and ASK keeps both
/// placements (live and saved): ASK is a regular view (ADR-0053), so a
/// preset may place it like any other; only its rail entry waits for the
/// server. No notice: nothing was left out.
#[gpui_kit::test]
fn a_preset_can_place_ask(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let text = lines(&[
        "[preset.rag]",
        "split = \"horizontal\"",
        "children = [",
        "    { views = [\"files\"], shown = \"files\" },",
        "    { editor = true },",
        "    { split = \"vertical\", children = [",
        "        { views = [\"semantic\"], shown = \"semantic\" },",
        "        { views = [\"ask\", \"links\"], shown = \"ask\" },",
        "    ] },",
        "]",
    ]);
    let shell = open_with(cx, &config, Some(&text));
    apply(&shell, cx, "rag");
    let got = live(&shell, cx);
    let slots = got.slots();
    assert_eq!(slots.len(), 3);
    assert_eq!(got.shown(slots[1]), Some(DrawerView::Semantic));
    assert_eq!(got.view_slot(&DrawerView::Ask), Some(slots[2]));
    assert_eq!(got.shown(slots[2]), Some(DrawerView::Ask));
    assert_eq!(
        got.slot_views(slots[2]),
        [DrawerView::Ask, DrawerView::Links]
    );
    let shown = flashes(&shell, cx);
    assert!(shown.is_empty(), "nothing skipped: {shown:?}");
    // Saved as the startup restore reads it back: SEM and ASK kept.
    let back = saved_tree(&config);
    assert_eq!(back.shown(back.slots()[1]), Some(DrawerView::Semantic));
    assert_eq!(back.shown(back.slots()[2]), Some(DrawerView::Ask));
    assert_eq!(spec::to_spec(&back), spec::to_spec(&got));
}

#[gpui_kit::test]
fn editing_the_file_changes_nothing_until_applied(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let shell = open_with(cx, &config, Some(&two_presets()));
    apply(&shell, cx, "focus");
    let applied = live(&shell, cx);
    let on_disk = saved(&config);

    // "focus" now puts FIND beside the editor instead of TAGS.
    let edited = two_presets().replace(
        "{ views = [\"tags\"], shown = \"tags\" }",
        "{ views = [\"find\"], shown = \"find\" }",
    );
    assert_ne!(edited, two_presets());
    std::fs::write(presets_path(&config), &edited).unwrap();
    cx.update_window(shell.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(live(&shell, cx), applied);
    // A reload reads the file but leaves the live layout alone.
    run(&shell, cx, PresetCommand::Reload);
    assert_eq!(live(&shell, cx), applied);
    assert_eq!(saved(&config), on_disk);
    // Applied again, the edit lands.
    apply(&shell, cx, "focus");
    let got = live(&shell, cx);
    assert_eq!(spec::to_spec(&got), spec::to_spec(&built(&edited, "focus")));
    assert_eq!(got.view_slot(&DrawerView::Tags), None);
}

#[gpui_kit::test]
fn invalid_preset_reports_each_error_and_keeps_current_layout(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let text = lines(&[
        "[preset.broken]",
        "split = \"sideways\"",
        "children = [",
        "    { views = [\"nope\", \"files\"], shown = \"tags\" },",
        "    { views = [\"files\"], weight = -1 },",
        "]",
        "",
        "[preset.worse]",
        "split = \"horizontal\"",
        "children = [",
        "    { views = [\"a\", \"b\", \"c\", \"d\", \"e\"] },",
        "    { editor = true },",
        "]",
    ]);
    let shell = open_with(cx, &config, Some(&text));
    let before = live(&shell, cx);

    apply(&shell, cx, "broken");
    assert_eq!(live(&shell, cx), before);
    // Nothing was written: no config file yet.
    assert!(!config.exists());
    let shown = flashes(&shell, cx);
    assert_eq!(shown.len(), 1, "one flash for every error: {shown:?}");
    let (message, error) = &shown[0];
    assert!(*error);
    assert!(message.contains("\"broken\""), "{message}");
    // Unknown axis, unknown view, shown not in views: the first three;
    // then a duplicate, a bad weight and no editor group are counted.
    assert!(message.contains("\"sideways\""), "{message}");
    assert!(message.contains("\"nope\""), "{message}");
    assert!(message.contains("\"tags\""), "{message}");
    assert!(message.contains("(and 3 more)"), "{message}");

    apply(&shell, cx, "worse");
    assert_eq!(live(&shell, cx), before);
    let message = &flashes(&shell, cx)[1].0;
    for id in ["\"a\"", "\"b\"", "\"c\""] {
        assert!(message.contains(id), "{message}");
    }
    assert!(message.contains("(and 2 more)"), "{message}");
    // Still no preset recorded; "Reset" stays disabled.
    assert!(rows(&shell, cx).contains(&(
        "Reset to preset".to_string(),
        Some(presets::NO_ACTIVE_PRESET)
    )));
}

#[gpui_kit::test]
fn reset_to_preset_discards_dragged_weights(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let shell = open_with(cx, &config, Some(&two_presets()));
    apply(&shell, cx, "research");
    let want = spec::to_spec(&built(&two_presets(), "research"));

    // A divider dragged (the editor's share) and a view moved.
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            let group = Leaf::Group(w.layout.active_group());
            assert!(w.layout.resize_after(group, 0.8));
            let first = w.layout.slots()[0];
            w.layout.move_view(DrawerView::Links, first);
            w.save_layout(cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    assert_ne!(spec::to_spec(&saved_tree(&config)), want);

    run(&shell, cx, PresetCommand::Reset);
    assert_eq!(spec::to_spec(&live(&shell, cx)), want);
    assert_eq!(spec::to_spec(&saved_tree(&config)), want);
    assert_eq!(saved(&config).layout_preset().as_deref(), Some("research"));
}

/// The preset recorded by an earlier session is the one "Reset" re-applies;
/// gone from the file, "Reset" is disabled with the reason.
#[gpui_kit::test]
fn reset_follows_the_recorded_preset_and_is_disabled_when_gone(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    std::fs::write(&config, "[gui]\nlayout_preset = \"focus\"\n").unwrap();
    let shell = open_with(cx, &config, Some(&two_presets()));
    assert!(rows(&shell, cx).contains(&("Reset to preset".to_string(), None)));
    run(&shell, cx, PresetCommand::Reset);
    assert_eq!(
        spec::to_spec(&live(&shell, cx)),
        spec::to_spec(&built(&two_presets(), "focus"))
    );
    // "focus" is taken out of the file and the file reloaded.
    let only_research = two_presets()
        .split("[preset.focus]")
        .next()
        .unwrap()
        .to_string();
    std::fs::write(presets_path(&config), only_research).unwrap();
    run(&shell, cx, PresetCommand::Reload);
    assert!(
        rows(&shell, cx).contains(&("Reset to preset".to_string(), Some(presets::PRESET_GONE)))
    );
}

#[gpui_kit::test]
fn apply_preset_clears_session_only_state(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let shell = open_with(cx, &config, Some(&two_presets()));
    cx.update_window(shell.handle, |_, window, cx| {
        shell.workspace.update(cx, |w, cx| {
            // A side toggled off, a split's empty slot, a zoomed leaf.
            w.toggle_side(kimun_notes::logic::layout::Side::Left, window, cx);
            let left = w.layout.slots()[0];
            let new = w.layout.split_slot(left, Axis::Vertical).unwrap();
            w.opened_empty.insert(new);
            w.layout.zoom(Some(Leaf::Group(w.layout.active_group())));
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.read(|cx| {
        let w = shell.workspace.read(cx);
        assert!(w.side_memory.iter().any(|m| !m.is_empty()));
        assert!(!w.opened_empty.is_empty());
        assert!(w.layout.zoomed().is_some());
    });

    apply(&shell, cx, "research");
    cx.read(|cx| {
        let w = shell.workspace.read(cx);
        assert!(w.opened_empty.is_empty());
        assert!(w.side_memory.iter().all(Vec::is_empty));
        // Zoom lives in the tree, which the preset replaced.
        assert_eq!(w.layout.zoomed(), None);
    });
}

/// The app reads `layouts.toml` and never creates, writes or touches it:
/// after loads, applies, a failed apply, a reload and a reset, its bytes
/// and modification time are those it had; with no file, none appears.
#[gpui_kit::test]
fn the_app_never_creates_or_modifies_the_presets_file(cx: &mut TestAppContext) {
    let (_dir, config) = temp_config();
    let text = two_presets() + &lines(&["", "[preset.bad]", "views = [\"nope\"]"]);
    let shell = open_with(cx, &config, Some(&text));
    let path = presets_path(&config);
    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    apply(&shell, cx, "research");
    apply(&shell, cx, "bad");
    run(&shell, cx, PresetCommand::Reload);
    apply(&shell, cx, "focus");
    run(&shell, cx, PresetCommand::Reset);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), mtime);

    let (_dir, config) = temp_config();
    let shell = open_with(cx, &config, None);
    run(&shell, cx, PresetCommand::Reload);
    run(&shell, cx, PresetCommand::Reset);
    assert!(!presets_path(&config).exists());
}

/// The spec type is the logic layer's: a loaded preset is a `ChildSpec`.
#[test]
fn loaded_presets_are_specs() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join(PRESETS_FILE), two_presets()).unwrap();
    let load = presets::load(dir.path());
    let focus: &ChildSpec = &load.presets["focus"];
    assert!(spec::build(focus, Caps::default()).is_ok());
}
