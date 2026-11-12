use std::path::PathBuf;

use super::*;
use crate::settings::workspace_config::WorkspaceConfig;

/// A host-absolute path from a `/`-separated literal: `resolve_workspace_path`
/// drops what `SystemPath` cannot make absolute, and `/a` has no drive on
/// Windows.
fn abs(unix_style: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!(
            "C:\\{}",
            unix_style.trim_start_matches('/').replace('/', "\\")
        ))
    } else {
        PathBuf::from(unix_style)
    }
}

/// Settings with `names` registered at `/<name>`, `current` the current one.
fn with_workspaces(names: &[&str], current: &str) -> AppSettings {
    let mut wc = WorkspaceConfig::new_empty();
    for name in names {
        wc.add_workspace((*name).to_string(), abs(&format!("/{name}")))
            .unwrap();
    }
    wc.global.current_workspace = current.to_string();
    settings_with(|s| {
        s.workspace_config = Some(wc);
    })
}

/// Default settings, edited by `edit`.
fn settings_with(edit: impl FnOnce(&mut AppSettings)) -> AppSettings {
    let mut s = AppSettings::default();
    edit(&mut s);
    s
}

/// One draft edit, named for the failure message.
type Edit = (&'static str, fn(&mut PreferencesModel));

// ── Ranges and steps ─────────────────────────────────────────────────────────

#[test]
fn leader_delay_steps_by_fifty_within_its_range() {
    assert_eq!(step_leader_delay(400, true), 450);
    assert_eq!(step_leader_delay(400, false), 350);
    assert_eq!(step_leader_delay(20, false), LEADER_DELAY_MIN_MS);
    assert_eq!(step_leader_delay(0, false), 0);
    assert_eq!(step_leader_delay(1980, true), LEADER_DELAY_MAX_MS);
    assert_eq!(
        step_leader_delay(LEADER_DELAY_MAX_MS, true),
        LEADER_DELAY_MAX_MS
    );
    // A hand-edited value past the top comes back into range going up.
    assert_eq!(step_leader_delay(3000, true), LEADER_DELAY_MAX_MS);
}

#[test]
fn autosave_steps_by_five_within_its_range() {
    assert_eq!(step_autosave(10, true), 15);
    assert_eq!(step_autosave(10, false), AUTOSAVE_MIN_SECS);
    assert_eq!(step_autosave(AUTOSAVE_MIN_SECS, false), AUTOSAVE_MIN_SECS);
    assert_eq!(
        step_autosave(2, false),
        AUTOSAVE_MIN_SECS,
        "below min clamps up"
    );
    assert_eq!(step_autosave(298, true), AUTOSAVE_MAX_SECS);
    assert_eq!(step_autosave(AUTOSAVE_MAX_SECS, true), AUTOSAVE_MAX_SECS);
}

#[test]
fn model_steppers_use_the_rules() {
    let mut m = PreferencesModel::new(&AppSettings::default());
    m.leader_timeout_ms = 400;
    m.step_leader_delay(true);
    assert_eq!(m.leader_timeout_ms, 450);
    m.autosave_interval_secs = 5;
    m.step_autosave(false);
    assert_eq!(m.autosave_interval_secs, 5);
    m.step_autosave(true);
    assert_eq!(m.autosave_interval_secs, 10);
}

// ── Sorting ──────────────────────────────────────────────────────────────────

#[test]
fn sort_field_and_order_alternate() {
    use SortFieldSetting::*;
    use SortOrderSetting::*;
    assert_eq!(next_sort_field(Name), Title);
    assert_eq!(next_sort_field(Title), Name);
    assert_eq!(next_sort_order(Ascending), Descending);
    assert_eq!(next_sort_order(Descending), Ascending);
    assert_eq!(sort_field_label(Name), "Name");
    assert_eq!(sort_field_label(Title), "Title");
    assert_eq!(sort_order_label(Ascending), "Ascending");
    assert_eq!(sort_order_label(Descending), "Descending");
}

#[test]
fn cycle_sort_touches_only_its_slot() {
    let base = PreferencesModel::new(&AppSettings::default());
    for slot in SortSlot::ALL {
        let mut m = base.clone();
        m.cycle_sort(slot);
        let changed = [
            m.default_sort_field != base.default_sort_field,
            m.default_sort_order != base.default_sort_order,
            m.journal_sort_field != base.journal_sort_field,
            m.journal_sort_order != base.journal_sort_order,
        ];
        let expected = SortSlot::ALL.map(|s| s == slot);
        assert_eq!(changed, expected, "{slot:?}");
        assert_ne!(m.sort_label(slot), base.sort_label(slot));
        m.cycle_sort(slot);
        assert_eq!(
            m.sort_label(slot),
            base.sort_label(slot),
            "two cycles return"
        );
    }
}

// ── Editor backends ──────────────────────────────────────────────────────────

#[test]
fn backend_cycles_through_all_three_both_ways() {
    use EditorBackendSetting::*;
    let mut m = PreferencesModel::new(&AppSettings::default());
    assert_eq!(m.backends(), [Plain, Vim, Nvim]);
    m.editor_backend = Plain;
    for expected in [Vim, Nvim, Plain] {
        m.cycle_backend(true);
        assert_eq!(m.editor_backend, expected);
    }
    for expected in [Nvim, Vim, Plain] {
        m.cycle_backend(false);
        assert_eq!(m.editor_backend, expected);
    }
}

#[test]
fn without_backend_drops_it_but_keeps_a_configured_one() {
    use EditorBackendSetting::*;
    let s = settings_with(|s| {
        s.editor_backend = Nvim;
    });
    let mut m = PreferencesModel::new(&s).without_backend(Nvim);
    assert_eq!(m.backends(), [Plain, Vim]);
    assert_eq!(m.editor_backend, Nvim, "the configured backend survives");
    assert!(!m.dirty(&s));
    m.cycle_backend(true);
    assert_eq!(m.editor_backend, Plain, "forward from a dropped one: first");
    m.editor_backend = Nvim;
    m.cycle_backend(false);
    assert_eq!(m.editor_backend, Vim, "backward from a dropped one: last");
    m.cycle_backend(true);
    assert_eq!(m.editor_backend, Plain, "wraps within the offered ones");

    let last = PreferencesModel::new(&s)
        .without_backend(Nvim)
        .without_backend(Vim)
        .without_backend(Plain);
    assert_eq!(
        last.backends(),
        [Plain],
        "the last backend is never dropped"
    );
}

#[test]
fn backend_labels() {
    assert_eq!(backend_label(EditorBackendSetting::Plain), "Plain");
    assert_eq!(backend_label(EditorBackendSetting::Vim), "Vim (built-in)");
    assert_eq!(backend_label(EditorBackendSetting::Nvim), "Nvim (external)");
}

// ── Server ───────────────────────────────────────────────────────────────────

#[test]
fn server_url_is_trimmed_and_empty_means_none() {
    assert_eq!(normalize_server_url(""), None);
    assert_eq!(normalize_server_url("   "), None);
    assert_eq!(
        normalize_server_url(" http://h:1 ").as_deref(),
        Some("http://h:1")
    );
    let mut m = PreferencesModel::new(&AppSettings::default());
    m.set_server_url("http://myhost:9000");
    assert_eq!(m.server_url.as_deref(), Some("http://myhost:9000"));
    m.set_server_url(" ");
    assert_eq!(m.server_url, None);
}

// ── Model construction and themes ────────────────────────────────────────────

#[test]
fn new_reads_every_setting() {
    let mut s = with_workspaces(&["main"], "main");
    s.theme = "Gruvbox Light".to_string();
    s.use_nerd_fonts = true;
    s.leader_timeout_ms = 650;
    s.default_sort_field = SortFieldSetting::Title;
    s.journal_sort_order = SortOrderSetting::Ascending;
    s.autosave_interval_secs = 30;
    s.editor_backend = EditorBackendSetting::Vim;
    let wc = s.workspace_config.as_mut().unwrap();
    wc.global.update_check = false;
    wc.global.mouse = false;
    wc.global.kimun_server_url = Some("http://h:1".to_string());

    let m = PreferencesModel::new(&s);
    assert_eq!(m.theme_name, "Gruvbox Light");
    assert_eq!(m.selected_theme().name, "Gruvbox Light");
    assert!(m.use_nerd_fonts);
    assert!(!m.update_check);
    assert!(!m.mouse);
    assert_eq!(m.leader_timeout_ms, 650);
    assert_eq!(m.default_sort_field, SortFieldSetting::Title);
    assert_eq!(m.journal_sort_order, SortOrderSetting::Ascending);
    assert_eq!(m.autosave_interval_secs, 30);
    assert_eq!(m.editor_backend, EditorBackendSetting::Vim);
    assert_eq!(m.server_url.as_deref(), Some("http://h:1"));
    assert!(!m.dirty(&s));
}

#[test]
fn unset_or_missing_theme_selects_the_default_but_keeps_the_name() {
    for configured in ["", "No Such Theme"] {
        let s = settings_with(|s| {
            s.theme = configured.to_string();
        });
        let m = PreferencesModel::new(&s);
        assert_eq!(m.selected_theme().name, Theme::default().name);
        assert_eq!(m.theme_name, configured, "the configured name is untouched");
        assert!(!m.dirty(&s));
        assert_eq!(
            resolve_theme(&m.themes, configured).name,
            Theme::default().name
        );
    }
}

#[test]
fn theme_selection_moves_wraps_and_sets_the_draft() {
    let mut m = PreferencesModel::new(&AppSettings::default());
    m.themes = vec![
        Theme::gruvbox_dark(),
        Theme::gruvbox_light(),
        Theme::catppuccin_mocha(),
    ];
    m.theme_idx = 0;
    assert_eq!(m.move_theme(-1).unwrap().name, "Catppuccin Mocha");
    assert_eq!(m.theme_name, "Catppuccin Mocha");
    assert_eq!(m.move_theme(1).unwrap().name, "Gruvbox Dark");
    assert_eq!(m.select_theme(1).unwrap().name, "Gruvbox Light");
    assert_eq!(m.theme_idx, 1);
    assert!(m.select_theme(9).is_none());
    assert_eq!(m.theme_idx, 1, "out of range changes nothing");
}

#[test]
fn theme_choices_mark_the_effective_theme() {
    let s = settings_with(|s| {
        s.theme = "Gruvbox Light".to_string();
    });
    let choices = ThemeChoices::new(&s);
    assert_eq!(choices.themes[choices.original].name, "Gruvbox Light");
    let current: Vec<&str> = choices
        .entries()
        .filter(|(_, _, current)| *current)
        .map(|(_, name, _)| name)
        .collect();
    assert_eq!(current, ["Gruvbox Light"]);
    assert_eq!(choices.entries().count(), choices.themes.len());

    let unset = ThemeChoices::new(&AppSettings::default());
    assert_eq!(unset.themes[unset.original].name, Theme::default().name);
}

#[test]
fn theme_rows_follow_the_entries_and_filter_by_name() {
    let s = settings_with(|s| {
        s.theme = "Gruvbox Light".to_string();
    });
    let choices = ThemeChoices::new(&s);
    let rows = choices.rows();
    assert_eq!(rows.len(), choices.themes.len());
    for (row, (index, name, current)) in rows.iter().zip(choices.entries()) {
        assert_eq!(
            (row.index, row.name.as_str(), row.current),
            (index, name, current)
        );
        assert_eq!(crate::logic::list::ListRow::match_text(row), Some(name));
    }
}

#[test]
fn the_preview_is_the_selected_theme_unless_it_is_the_opening_one() {
    let s = settings_with(|s| {
        s.theme = "Gruvbox Light".to_string();
    });
    let choices = ThemeChoices::new(&s);
    let other = (choices.original + 1) % choices.themes.len();
    assert_eq!(choices.preview_target(Some(other)), Some(other));
    assert_eq!(
        choices.preview_target(Some(choices.original)),
        None,
        "the opening theme's row shows the opening theme"
    );
    assert_eq!(choices.preview_target(None), None, "nothing selected");
    assert_eq!(
        choices.preview_target(Some(choices.themes.len())),
        None,
        "out of range"
    );
}

#[test]
fn with_theme_changes_the_theme_only() {
    let s = settings_with(|s| {
        s.theme = "Gruvbox Light".to_string();
        s.leader_timeout_ms = 250;
    });
    let picked = with_theme(&s, "Nord");
    assert_eq!(picked.theme, "Nord");
    let mut back = picked.clone();
    back.theme = s.theme.clone();
    assert_eq!(back, s, "nothing else changed");
}

// ── dirty / apply ────────────────────────────────────────────────────────────

/// With no theme configured (or a missing one), picking another theme is
/// dirty and going back to the one in effect (the resolved default) is
/// clean again; a configured theme left as it is stays clean.
#[test]
fn an_unset_theme_is_clean_again_once_the_effective_theme_is_back() {
    for configured in ["", "No Such Theme"] {
        let s = settings_with(|s| s.theme = configured.to_string());
        let mut m = PreferencesModel::new(&s);
        let effective = m.theme_idx;
        assert_eq!(m.selected_theme().name, Theme::default().name);
        let other = (effective + 1) % m.themes.len();
        m.select_theme(other);
        assert!(m.dirty(&s), "{configured:?}: another theme is a change");
        m.select_theme(effective);
        assert_eq!(m.theme_name, Theme::default().name);
        assert!(!m.dirty(&s), "{configured:?}: back on the effective theme");
    }
    let s = settings_with(|s| s.theme = "Gruvbox Light".to_string());
    let mut m = PreferencesModel::new(&s);
    assert!(!m.dirty(&s));
    let at = m.theme_idx;
    m.select_theme((at + 1) % m.themes.len());
    assert!(m.dirty(&s));
    m.select_theme(at);
    assert!(!m.dirty(&s), "a set theme picked back is clean");
}

#[test]
fn dirty_tracks_each_field() {
    let s = with_workspaces(&["main"], "main");
    let base = PreferencesModel::new(&s);
    assert!(!base.dirty(&s));
    let edits: [Edit; 16] = [
        ("theme", |m| m.theme_name = "Gruvbox Light".to_string()),
        ("nerd fonts", |m| m.use_nerd_fonts = !m.use_nerd_fonts),
        ("update check", |m| m.update_check = !m.update_check),
        ("mouse", |m| m.mouse = !m.mouse),
        ("reduce motion", |m| m.reduce_motion = !m.reduce_motion),
        ("editor font family", |m| {
            m.editor_font_family = Some("X".into())
        }),
        ("editor font size", |m| m.editor_font_size = Some(20)),
        ("rail", |m| {
            m.rail = crate::logic::layout_tree::RailPosition::Top
        }),
        ("leader delay", |m| m.step_leader_delay(true)),
        ("default field", |m| m.cycle_sort(SortSlot::DefaultField)),
        ("default order", |m| m.cycle_sort(SortSlot::DefaultOrder)),
        ("journal field", |m| m.cycle_sort(SortSlot::JournalField)),
        ("journal order", |m| m.cycle_sort(SortSlot::JournalOrder)),
        ("autosave", |m| m.step_autosave(true)),
        ("backend", |m| m.cycle_backend(true)),
        ("server", |m| m.set_server_url("http://h:1")),
    ];
    for (what, edit) in edits {
        let mut m = base.clone();
        edit(&mut m);
        assert!(m.dirty(&s), "{what} must make the draft dirty");
        // Applied, the draft is what the settings hold: clean again.
        let mut applied = s.clone();
        m.apply(&mut applied);
        assert!(!m.dirty(&applied), "{what} after apply");
        assert_ne!(applied, s, "{what} reaches the settings");
    }
}

#[test]
fn apply_writes_every_section() {
    let s = with_workspaces(&["main"], "main");
    let mut m = PreferencesModel::new(&s);
    m.select_theme(
        m.themes
            .iter()
            .position(|t| t.name == "Gruvbox Light")
            .unwrap(),
    );
    m.use_nerd_fonts = true;
    m.update_check = false;
    m.mouse = false;
    m.reduce_motion = true;
    m.rail = crate::logic::layout_tree::RailPosition::Bottom;
    m.leader_timeout_ms = 100;
    m.default_sort_field = SortFieldSetting::Title;
    m.default_sort_order = SortOrderSetting::Descending;
    m.journal_sort_field = SortFieldSetting::Title;
    m.journal_sort_order = SortOrderSetting::Ascending;
    m.autosave_interval_secs = 60;
    m.editor_backend = EditorBackendSetting::Vim;
    m.server_url = Some("http://h:1".to_string());

    let mut out = s.clone();
    assert!(m.apply(&mut out), "the theme changed");
    assert_eq!(out.theme, "Gruvbox Light");
    assert!(out.use_nerd_fonts);
    assert!(!out.update_check());
    assert!(!out.mouse());
    assert!(out.gui.reduce_motion());
    assert_eq!(
        out.gui.rail(),
        crate::logic::layout_tree::RailPosition::Bottom
    );
    assert_eq!(out.leader_timeout_ms, 100);
    assert_eq!(out.default_sort_field, SortFieldSetting::Title);
    assert_eq!(out.default_sort_order, SortOrderSetting::Descending);
    assert_eq!(out.journal_sort_field, SortFieldSetting::Title);
    assert_eq!(out.journal_sort_order, SortOrderSetting::Ascending);
    assert_eq!(out.autosave_interval_secs, 60);
    assert_eq!(out.editor_backend, EditorBackendSetting::Vim);
    assert_eq!(
        out.workspace_config
            .as_ref()
            .unwrap()
            .global
            .kimun_server_url
            .as_deref(),
        Some("http://h:1")
    );
    assert_eq!(
        out.workspace_config.as_ref().unwrap().workspaces,
        s.workspace_config.as_ref().unwrap().workspaces,
        "the workspace list is not the draft's"
    );
    assert!(!m.apply(&mut out), "applying again changes no theme");
}

#[test]
fn an_untouched_draft_applies_as_a_no_op() {
    let s = with_workspaces(&["main"], "main");
    let m = PreferencesModel::new(&s);
    let mut out = s.clone();
    assert!(!m.apply(&mut out));
    assert_eq!(out, s);
}

#[test]
fn global_settings_create_the_workspace_config_when_missing() {
    let s = AppSettings::default();
    let m = PreferencesModel::new(&s);
    let mut out = s.clone();
    m.apply_display(&mut out);
    assert!(
        out.workspace_config.is_some(),
        "display writes the global config"
    );
    let mut out = s.clone();
    m.apply_server(&mut out);
    assert!(
        out.workspace_config.is_some(),
        "server writes the global config"
    );
    let mut out = s.clone();
    m.apply_sorting(&mut out);
    m.apply_editor(&mut out);
    assert!(!m.apply_appearance(&mut out));
    assert_eq!(out, s, "the other sections leave it alone");
}

// ── Workspaces ───────────────────────────────────────────────────────────────

#[test]
fn has_vault_needs_a_current_workspace() {
    assert!(!has_vault(&AppSettings::default()));
    assert!(!has_vault(&with_workspaces(&["a"], "gone")));
    assert!(has_vault(&with_workspaces(&["a"], "a")));
}

#[test]
fn a_new_name_is_trimmed_and_must_be_free() {
    let s = with_workspaces(&["work"], "work");
    assert_eq!(check_new_workspace_name(&s, "  notes ").unwrap(), "notes");
    assert!(matches!(
        check_new_workspace_name(&s, "   "),
        Err(WorkspaceError::EmptyName)
    ));
    let dup = check_new_workspace_name(&s, " work ").unwrap_err();
    assert!(matches!(&dup, WorkspaceError::Duplicate(n) if n == "work"));
    assert_eq!(dup.to_string(), "Workspace 'work' already exists.");
    assert_eq!(
        check_new_workspace_name(&AppSettings::default(), "x").unwrap(),
        "x"
    );
}

#[test]
fn add_workspace_lowercases_and_creates_the_config() {
    let mut s = AppSettings::default();
    add_workspace(&mut s, "Notes", abs("/notes")).unwrap();
    assert_eq!(s.config_version, CURRENT_CONFIG_VERSION);
    let wc = s.workspace_config.as_ref().unwrap();
    assert!(wc.workspaces.contains_key("notes"));
    assert_eq!(wc.global.current_workspace, "notes", "the first is current");

    add_workspace(&mut s, "work", abs("/work")).unwrap();
    assert_eq!(
        s.workspace_config
            .as_ref()
            .unwrap()
            .global
            .current_workspace,
        "notes",
        "a later one does not take over"
    );
    assert!(matches!(
        add_workspace(&mut s, "NOTES", abs("/other")),
        Err(WorkspaceError::Duplicate(n)) if n == "notes"
    ));
    assert!(matches!(
        add_workspace(&mut s, "bad:name", abs("/bad")),
        Err(WorkspaceError::Register(
            WorkspaceConfigError::InvalidName { .. }
        ))
    ));
}

#[test]
fn rename_refiles_and_rejects_a_taken_name() {
    let mut s = with_workspaces(&["a", "b"], "a");
    assert!(rename_workspace(&mut s, "a", " c ").unwrap());
    let wc = s.workspace_config.as_ref().unwrap();
    assert!(wc.workspaces.contains_key("c") && !wc.workspaces.contains_key("a"));
    assert_eq!(wc.global.current_workspace, "c", "the current one follows");

    let err = rename_workspace(&mut s, "c", "b").unwrap_err();
    assert!(matches!(&err, WorkspaceError::Duplicate(n) if n == "b"));
    assert!(!rename_workspace(&mut s, "c", "c").unwrap(), "unchanged");
    assert!(!rename_workspace(&mut s, "c", "  ").unwrap(), "empty");
    assert!(
        !rename_workspace(&mut s, "gone", "d").unwrap(),
        "no such workspace"
    );
    assert!(!rename_workspace(&mut AppSettings::default(), "a", "b").unwrap());
}

#[test]
fn delete_refuses_the_current_and_unknown_workspaces() {
    let mut s = with_workspaces(&["a", "b"], "a");
    assert!(matches!(
        take_workspace_for_delete(&mut s, "a"),
        Err(WorkspaceError::IsCurrent(_))
    ));
    assert!(matches!(
        take_workspace_for_delete(&mut s, "nope"),
        Err(WorkspaceError::NotFound(_))
    ));
    assert!(matches!(
        take_workspace_for_delete(&mut AppSettings::default(), "a"),
        Err(WorkspaceError::NotFound(_))
    ));
    let expected = s.workspace_artifacts("b");
    let taken = take_workspace_for_delete(&mut s, "b").unwrap();
    assert_eq!(taken, expected, "the artifacts are read before removal");
    let wc = s.workspace_config.as_ref().unwrap();
    assert!(!wc.workspaces.contains_key("b"));
    assert!(wc.workspaces.contains_key("a"));
}

#[test]
fn moving_a_vault_flags_a_reindex() {
    let mut s = with_workspaces(&["main"], "main");
    s.report_indexed();
    let initial = s.clone();
    assert!(
        !set_workspace_path(&mut s, "main", abs("/main")),
        "same path"
    );
    assert!(!s.needs_indexing());
    assert_eq!(save_plan(&s, &initial), Ok(SavePlan::SaveOnly));

    assert!(set_workspace_path(&mut s, "main", abs("/moved")));
    assert!(s.needs_indexing());
    assert!(matches!(
        save_plan(&s, &initial),
        Ok(SavePlan::NeedsReindex { .. })
    ));

    assert!(
        !set_workspace_path(&mut s, "nope", abs("/x")),
        "no such entry"
    );
}

// ── Save plan ────────────────────────────────────────────────────────────────

#[test]
fn an_unmoved_vault_only_saves() {
    let initial = with_workspaces(&["main"], "main");
    let mut current = initial.clone();
    current.theme = "Gruvbox Light".to_string();
    assert_eq!(save_plan(&current, &initial), Ok(SavePlan::SaveOnly));
}

#[test]
fn a_moved_vault_needs_a_reindex_of_the_new_one() {
    let initial = with_workspaces(&["main"], "main");
    let mut current = initial.clone();
    set_workspace_path(&mut current, "main", abs("/moved"));
    let expected = SavePlan::NeedsReindex {
        workspace: SystemPath::try_absolute(abs("/moved")).unwrap(),
        cache: Some(current.index_for("main")),
    };
    assert_eq!(save_plan(&current, &initial), Ok(expected));

    // Switching to another workspace is a move as well.
    let initial = with_workspaces(&["a", "b"], "a");
    let mut current = initial.clone();
    current
        .workspace_config
        .as_mut()
        .unwrap()
        .global
        .current_workspace = "b".into();
    assert!(matches!(
        save_plan(&current, &initial),
        Ok(SavePlan::NeedsReindex { cache: Some(_), .. })
    ));
}

#[test]
fn a_vault_changed_to_none_cannot_be_saved() {
    let initial = with_workspaces(&["main"], "main");
    let mut current = initial.clone();
    current.clear_workspace();
    let err = save_plan(&current, &initial).unwrap_err();
    assert_eq!(err, SaveError::NoWorkspace);
    assert_eq!(err.to_string(), "No workspace set");
}

#[test]
fn reindex_target_is_the_current_vault_and_its_index() {
    assert!(reindex_target(&AppSettings::default()).is_none());
    let s = with_workspaces(&["main"], "main");
    let (workspace, cache) = reindex_target(&s).unwrap();
    assert_eq!(workspace, SystemPath::try_absolute(abs("/main")).unwrap());
    assert_eq!(cache, Some(s.index_for("main")));
}

#[tokio::test]
async fn reindex_runners_finish_on_an_empty_vault() {
    let vault = tempfile::TempDir::new().unwrap();
    let cache = tempfile::TempDir::new().unwrap();
    let workspace = SystemPath::try_absolute(vault.path()).unwrap();
    let index = IndexFile::in_dir(&SystemPath::try_absolute(cache.path()).unwrap(), "t");
    assert!(matches!(
        reindex_new_vault(workspace.clone(), Some(index.clone())).await,
        ReindexOutcome::Done(_)
    ));
    assert!(fast_reindex(workspace, Some(index)).await.is_ok());
}

// ── CFG view ─────────────────────────────────────────────────────────────────

#[test]
fn config_info_summarises_the_settings() {
    let s = settings_with(|s| {
        s.theme = "Gruvbox Light".to_string();
        s.leader_timeout_ms = 250;
    });
    let info = config_info(&s);
    assert_eq!(info.theme_name, "Gruvbox Light");
    assert_eq!(info.leader_timeout_ms, 250);
    assert_eq!(info.config_path, DEFAULT_CONFIG_LOCATION);
    assert_eq!(
        info.leader_key,
        s.key_bindings
            .first_combo_for(&ActionShortcuts::Leader)
            .unwrap_or_else(|| UNBOUND.to_string())
    );

    let s = settings_with(|s| {
        s.config_file = Some(abs("/cfg/config.toml"));
    });
    assert_eq!(
        config_info(&s).config_path,
        abs("/cfg/config.toml").display().to_string()
    );
    assert_eq!(config_path_label(&s), config_info(&s).config_path);
}

#[test]
fn check_server_url_takes_empty_or_an_http_address() {
    assert_eq!(check_server_url(""), Ok(None));
    assert_eq!(check_server_url("  "), Ok(None));
    assert_eq!(
        check_server_url(" http://localhost:7573 "),
        Ok(Some("http://localhost:7573".to_string()))
    );
    assert_eq!(
        check_server_url("HTTPS://kimun.example"),
        Ok(Some("HTTPS://kimun.example".to_string()))
    );
    for bad in [
        "localhost:7573",
        "ftp://h",
        "http://",
        "http:///x",
        "http://a b",
    ] {
        assert_eq!(check_server_url(bad), Err(ServerUrlError::NotHttp), "{bad}");
    }
    assert!(ServerUrlError::NotHttp.to_string().contains("http://"));
}

#[test]
fn workspace_names_share_one_rule_for_check_add_and_rename() {
    let mut s = with_workspaces(&["work", "a"], "work");
    // Case-variant of an existing name is a duplicate up front, not a silent
    // refusal after `add_workspace` lowercases it.
    assert!(matches!(
        check_new_workspace_name(&s, "Work"),
        Err(WorkspaceError::Duplicate(n)) if n == "work"
    ));
    assert_eq!(check_new_workspace_name(&s, " NEW ").unwrap(), "new");
    assert!(matches!(
        check_new_workspace_name(&s, "a/b"),
        Err(WorkspaceError::Register(
            WorkspaceConfigError::InvalidName { .. }
        ))
    ));
    // Rename: same rules.
    assert!(matches!(
        rename_workspace(&mut s, "a", "Work"),
        Err(WorkspaceError::Duplicate(_))
    ));
    assert!(matches!(
        rename_workspace(&mut s, "a", "a/b"),
        Err(WorkspaceError::Register(
            WorkspaceConfigError::InvalidName { .. }
        ))
    ));
    assert!(
        !rename_workspace(&mut s, "a", "A").unwrap(),
        "same name, other case"
    );
    assert!(rename_workspace(&mut s, "a", "B").unwrap());
    assert!(
        s.workspace_config
            .as_ref()
            .unwrap()
            .workspaces
            .contains_key("b")
    );
}

#[test]
fn steppers_snap_off_grid_and_out_of_range_values_the_right_way() {
    // Autosave: 5..=300 on a 5 grid.
    assert_eq!(step_autosave(1000, true), 300);
    assert_eq!(
        step_autosave(1000, false),
        300,
        "down from above max stays in range"
    );
    assert_eq!(step_autosave(3, true), 5);
    assert_eq!(step_autosave(3, false), 5);
    assert_eq!(step_autosave(7, true), 10);
    assert_eq!(step_autosave(7, false), 5);
    assert_eq!(
        step_autosave(10, false),
        5,
        "on-grid is unchanged behaviour"
    );
    // Leader delay: 0..=2000 on a 50 grid.
    assert_eq!(step_leader_delay(30, true), 50);
    assert_eq!(step_leader_delay(30, false), 0);
    assert_eq!(step_leader_delay(0, false), 0);
    assert_eq!(step_leader_delay(2000, true), 2000);
    assert_eq!(step_leader_delay(5000, true), 2000);
}

#[test]
fn try_set_server_url_validates_but_accepts_the_value_already_held() {
    let mut m = PreferencesModel::new(&settings_with(|s| {
        s.workspace_config
            .get_or_insert_with(WorkspaceConfig::new_empty)
            .global
            .kimun_server_url = Some("localhost:7573".into());
    }));
    // The stored (invalid) value is not an error until it is edited.
    assert_eq!(m.try_set_server_url(" localhost:7573 "), Ok(()));
    assert_eq!(m.server_url.as_deref(), Some("localhost:7573"));
    // An edit to another invalid text is refused and leaves the draft.
    assert_eq!(
        m.try_set_server_url("ftp://h"),
        Err(ServerUrlError::NotHttp)
    );
    assert_eq!(m.server_url.as_deref(), Some("localhost:7573"));
    assert_eq!(m.try_set_server_url("http://h:1"), Ok(()));
    assert_eq!(m.server_url.as_deref(), Some("http://h:1"));
    assert_eq!(m.try_set_server_url(""), Ok(()));
    assert_eq!(m.server_url, None);
}

#[test]
fn sort_choices_select_and_set_agree_with_the_cycle() {
    let mut m = PreferencesModel::new(&AppSettings::default());
    for slot in SortSlot::ALL {
        let choices = m.sort_choices(slot);
        assert!(choices.len() >= 2);
        // Every choice, set by index, reads back as itself.
        for (ix, label) in choices.iter().enumerate() {
            assert!(m.set_sort(slot, ix));
            assert_eq!(m.sort_selected(slot), Some(ix));
            assert_eq!(m.sort_label(slot), *label);
        }
        // Cycling visits every choice and wraps.
        m.set_sort(slot, 0);
        let mut seen = vec![m.sort_selected(slot).unwrap()];
        for _ in 0..choices.len() {
            m.cycle_sort(slot);
            seen.push(m.sort_selected(slot).unwrap());
        }
        assert_eq!(
            seen,
            (0..=choices.len())
                .map(|i| i % choices.len())
                .collect::<Vec<_>>()
        );
        // Out of range: refused, unchanged.
        let before = m.sort_selected(slot);
        assert!(!m.set_sort(slot, choices.len()));
        assert_eq!(m.sort_selected(slot), before);
    }
    // Slots stay independent.
    m.set_sort(SortSlot::DefaultOrder, 1);
    m.set_sort(SortSlot::JournalOrder, 0);
    assert_eq!(m.sort_selected(SortSlot::DefaultOrder), Some(1));
}

#[test]
fn picker_and_preferences_start_on_the_same_theme_row() {
    for configured in ["", "no such theme", "Gruvbox Light"] {
        let s = settings_with(|s| s.theme = configured.to_string());
        let model = PreferencesModel::new(&s);
        let choices = ThemeChoices::new(&s);
        assert_eq!(
            model.theme_idx, choices.original,
            "configured {configured:?}"
        );
    }
}

// ── WorkspaceEdit ────────────────────────────────────────────────────────────

#[test]
fn workspace_edit_add_registers_and_the_first_becomes_current() {
    let mut s = settings_with(|_| {});
    let out = WorkspaceEdit::Add {
        name: " First ".into(),
        path: abs("/first"),
    }
    .apply(&mut s)
    .unwrap();
    assert!(out.changed && out.current);
    assert_eq!(s.current_workspace_name().as_deref(), Some("first"));
    let out = WorkspaceEdit::Add {
        name: "second".into(),
        path: abs("/second"),
    }
    .apply(&mut s)
    .unwrap();
    assert!(out.changed && !out.current, "the second is not current");
    assert!(workspace_exists(&s, "second"));
    let dup = WorkspaceEdit::Add {
        name: "SECOND".into(),
        path: abs("/x"),
    }
    .apply(&mut s);
    assert!(matches!(dup, Err(WorkspaceError::Duplicate(_))));
    let empty = WorkspaceEdit::Add {
        name: "  ".into(),
        path: abs("/x"),
    }
    .apply(&mut s);
    assert!(matches!(empty, Err(WorkspaceError::EmptyName)));
}

#[test]
fn workspace_edit_rename_refuses_empty_and_duplicates() {
    let mut s = with_workspaces(&["a", "b"], "a");
    let empty = WorkspaceEdit::Rename {
        old: "a".into(),
        new: " ".into(),
    }
    .apply(&mut s);
    assert!(matches!(empty, Err(WorkspaceError::EmptyName)));
    let dup = WorkspaceEdit::Rename {
        old: "a".into(),
        new: "b".into(),
    }
    .apply(&mut s);
    assert!(matches!(dup, Err(WorkspaceError::Duplicate(_))));
    let same = WorkspaceEdit::Rename {
        old: "a".into(),
        new: "a".into(),
    }
    .apply(&mut s)
    .unwrap();
    assert!(!same.changed);
    let out = WorkspaceEdit::Rename {
        old: "a".into(),
        new: "c".into(),
    }
    .apply(&mut s)
    .unwrap();
    assert!(out.changed && out.current);
    assert_eq!(s.current_workspace_name().as_deref(), Some("c"));
}

#[test]
fn workspace_edit_delete_refuses_the_current_one() {
    let mut s = with_workspaces(&["a", "b"], "a");
    let refused = WorkspaceEdit::Delete { name: "a".into() }.apply(&mut s);
    assert!(matches!(refused, Err(WorkspaceError::IsCurrent(_))));
    let out = WorkspaceEdit::Delete { name: "b".into() }
        .apply(&mut s)
        .unwrap();
    assert!(out.changed && out.artifacts.is_some());
    assert!(!workspace_exists(&s, "b"));
}

#[test]
fn workspace_edit_set_path_says_whether_the_vault_moved() {
    let mut s = with_workspaces(&["a", "b"], "a");
    let same = WorkspaceEdit::SetPath {
        name: "b".into(),
        path: abs("/b"),
    }
    .apply(&mut s)
    .unwrap();
    assert!(!same.changed && !same.moved && !same.current);
    let other = WorkspaceEdit::SetPath {
        name: "b".into(),
        path: abs("/elsewhere"),
    }
    .apply(&mut s)
    .unwrap();
    assert!(other.moved && !other.current);
    let current = WorkspaceEdit::SetPath {
        name: "a".into(),
        path: abs("/moved"),
    }
    .apply(&mut s)
    .unwrap();
    assert!(current.moved && current.current);
    assert!(s.needs_indexing());
    let unknown = WorkspaceEdit::SetPath {
        name: "zz".into(),
        path: abs("/x"),
    }
    .apply(&mut s);
    assert!(matches!(unknown, Err(WorkspaceError::NotFound(_))));
}

#[test]
fn prepare_workspace_folder_makes_a_missing_folder() {
    let dir = tempfile::TempDir::new().unwrap();
    let target = dir.path().join("fresh");
    let made = prepare_workspace_folder(&target).unwrap();
    assert!(target.is_dir());
    assert!(made.ends_with("fresh"));
    // Already there: fine, untouched.
    assert!(prepare_workspace_folder(&target).is_ok());
}

#[test]
fn indexed_in_reads_as_a_short_duration() {
    assert_eq!(indexed_in(Duration::from_millis(1234)), "Indexed in 1.2s");
}

/// `[gui] reduce_motion` is desktop-only: read into the draft, written back
/// only when it changed, so a save that did not touch it (every TUI save)
/// never adds the key, and the rest of `[gui]` is kept.
#[test]
fn reduce_motion_is_written_only_when_changed() {
    let s = with_workspaces(&["main"], "main");
    let m = PreferencesModel::new(&s);
    assert!(!m.reduce_motion);
    let mut out = s.clone();
    m.apply(&mut out);
    assert_eq!(out.gui, s.gui, "an untouched draft adds no [gui] key");

    let mut s = s;
    s.gui.set_reduce_motion(true);
    s.gui
        .set_window_decorations(crate::settings::gui::WindowDecorations::System);
    let mut m = PreferencesModel::new(&s);
    assert!(m.reduce_motion, "read from the config");
    m.reduce_motion = false;
    let mut out = s.clone();
    m.apply(&mut out);
    assert!(!out.gui.reduce_motion());
    assert_eq!(
        out.gui.window_decorations(),
        crate::settings::gui::WindowDecorations::System,
        "the rest of [gui] is kept"
    );
}

#[test]
fn editor_font_size_steps_from_the_theme_default_and_clamps() {
    let s = with_workspaces(&["main"], "main");
    let mut m = PreferencesModel::new(&s);
    m.editor_font_size = None;
    m.step_editor_font_size(true, 13);
    assert_eq!(m.editor_font_size, Some(14));
    m.editor_font_size = Some(8);
    m.step_editor_font_size(false, 13);
    assert_eq!(m.editor_font_size, Some(8));
    m.editor_font_size = Some(32);
    m.step_editor_font_size(true, 13);
    assert_eq!(m.editor_font_size, Some(32));
    m.editor_font_size = None;
    m.step_editor_font_size(true, 5);
    assert_eq!(m.editor_font_size, Some(8));
}

#[test]
fn editor_font_is_read_into_the_draft_and_written_only_when_changed() {
    let mut s = with_workspaces(&["main"], "main");
    s.gui.set_editor_font_family(Some("Fira Code"));
    s.gui.set_editor_font_size(Some(15));
    let m = PreferencesModel::new(&s);
    assert_eq!(m.editor_font_family.as_deref(), Some("Fira Code"));
    assert_eq!(m.editor_font_size, Some(15));

    let s = with_workspaces(&["main"], "main");
    let m = PreferencesModel::new(&s);
    let mut out = s.clone();
    m.apply(&mut out);
    assert!(out.gui.is_empty(), "an untouched draft adds no [gui] key");

    let mut m = PreferencesModel::new(&s);
    m.editor_font_family = Some("Fira Code".into());
    let mut out = s.clone();
    m.apply(&mut out);
    assert_eq!(out.gui.editor_font_family().as_deref(), Some("Fira Code"));
    assert_eq!(out.gui.editor_font_size(), None);
}

/// Family names match whatever their capital letters: a config
/// `"fira code"` re-picked as the installed "Fira Code" is no change, so the
/// draft is clean and the file keeps the user's spelling.
#[test]
fn a_family_differing_only_in_case_is_no_change() {
    let mut s = with_workspaces(&["main"], "main");
    s.gui.set_editor_font_family(Some("fira code"));
    let mut m = PreferencesModel::new(&s);
    m.editor_font_family = Some("Fira Code".into());
    assert!(!m.dirty(&s), "a case-only difference is clean");
    let mut out = s.clone();
    m.apply(&mut out);
    assert_eq!(
        out.gui.editor_font_family().as_deref(),
        Some("fira code"),
        "the user's spelling is kept"
    );

    m.editor_font_family = Some("Iosevka".into());
    assert!(m.dirty(&s), "another family is a change");
    m.editor_font_family = None;
    assert!(m.dirty(&s), "the theme's font is a change");
}

#[test]
fn same_font_family_ignores_ascii_case_only() {
    assert!(same_font_family(None, None));
    assert!(same_font_family(Some("Fira Code"), Some("fira code")));
    assert!(!same_font_family(Some("Fira Code"), None));
    assert!(!same_font_family(None, Some("Fira Code")));
    assert!(!same_font_family(Some("Fira Code"), Some("Fira Mono")));
}

#[test]
fn choosing_default_removes_the_keys() {
    let mut s = with_workspaces(&["main"], "main");
    s.gui.set_editor_font_family(Some("Fira Code"));
    s.gui.set_editor_font_size(Some(15));
    let mut m = PreferencesModel::new(&s);
    m.editor_font_family = None;
    m.editor_font_size = None;
    let mut out = s.clone();
    m.apply(&mut out);
    assert_eq!(out.gui.editor_font_family(), None);
    assert_eq!(out.gui.editor_font_size(), None);
}

/// `[gui] rail` is desktop-only like `reduce_motion`: read into the draft,
/// written only when it changed, the rest of `[gui]` kept, and the default
/// position removes the key.
#[test]
fn rail_is_written_only_when_changed() {
    use crate::logic::layout_tree::RailPosition;
    let s = with_workspaces(&["main"], "main");
    let m = PreferencesModel::new(&s);
    assert_eq!(m.rail, RailPosition::Left);
    let mut out = s.clone();
    m.apply(&mut out);
    assert_eq!(out.gui, s.gui, "an untouched draft adds no [gui] key");

    let mut s = s;
    s.gui.set_rail(RailPosition::Right);
    s.gui.set_rail_hidden(true);
    let mut m = PreferencesModel::new(&s);
    assert_eq!(m.rail, RailPosition::Right, "read from the config");
    m.rail = RailPosition::Left;
    let mut out = s.clone();
    m.apply(&mut out);
    assert_eq!(out.gui.rail(), RailPosition::Left);
    assert!(out.gui.rail_hidden(), "the rest of [gui] is kept");
}
