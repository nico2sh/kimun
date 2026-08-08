// tui/tests/config_path_test.rs
//
// `--config <bare filename>`: the one config path whose parent is *empty*
// rather than absent. Creating that parent is a no-op the OS accepts, but
// canonicalizing "" is not, so the empty case has to be filtered out before it
// reaches `system::create_dir` — otherwise every such invocation aborts before
// the config is read at all.
//
// Sole test in this binary on purpose: a bare filename only means anything
// relative to the process's working directory, and moving that is safe only
// with nothing else running alongside it.

use std::path::PathBuf;

use kimun_notes::settings::AppSettings;

#[test]
fn a_bare_config_filename_loads_from_the_working_directory() {
    let dir = tempfile::TempDir::new().unwrap();
    let original = std::env::current_dir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let loaded = AppSettings::load_from_file(PathBuf::from("kimun.toml"));

    // Restore before the assertions (and before `dir` drops): Windows will not
    // remove a directory that is some process's working directory.
    std::env::set_current_dir(&original).unwrap();

    loaded.expect("a bare --config filename must load rather than abort");
    assert!(
        dir.path().join("kimun.toml").exists(),
        "the default config should have been written beside the working directory"
    );
}
