//! The **Workspace switcher**'s rules, shared by both frontends: which
//! workspaces it lists (every configured one, by name), which one is the
//! current one, where the selection starts, and what picking a row does.
//! The settings own the workspaces; this module only reads them.

use std::path::PathBuf;

use super::list::ListRow;
use crate::settings::AppSettings;

/// One row of the workspace switcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRow {
    /// The workspace's name, as configured.
    pub name: String,
    /// Its folder as configured (where a folder browser for it starts).
    pub path: PathBuf,
    /// Its folder as the settings resolve it, for display.
    pub folder: String,
    /// Whether it is the current workspace.
    pub current: bool,
}

impl WorkspaceRow {
    /// What picking the row does: the name to switch to, or `None` when it
    /// is the current workspace (nothing to do; the switcher only closes).
    pub fn switch_target(&self) -> Option<&str> {
        (!self.current).then_some(self.name.as_str())
    }
}

/// A workspace is a list row (the GUI's dialog keeps its rows and
/// selection in a [`ListCore`](super::list::ListCore)): never filtered,
/// nothing to yank.
impl ListRow for WorkspaceRow {}

/// Every configured workspace, sorted by name, the current one marked.
/// Empty when the settings hold no workspace config.
pub fn workspace_rows(settings: &AppSettings) -> Vec<WorkspaceRow> {
    let Some(config) = settings.workspace_config.as_ref() else {
        return Vec::new();
    };
    let current = &config.global.current_workspace;
    // `workspaces` is a BTreeMap: iteration is already sorted by name.
    config
        .workspaces
        .iter()
        .map(|(name, entry)| WorkspaceRow {
            name: name.clone(),
            path: entry.path.clone(),
            folder: entry.effective_path().display().to_string(),
            current: name == current,
        })
        .collect()
}

/// Where the selection starts: the current workspace's row, else the first
/// (`None` on an empty list).
pub fn initial_selection(rows: &[WorkspaceRow]) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    Some(rows.iter().position(|row| row.current).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::settings::workspace_config::WorkspaceConfig;

    fn settings(names: &[&str], current: &str) -> AppSettings {
        let mut config = WorkspaceConfig::new_empty();
        for name in names {
            config
                .add_workspace((*name).to_string(), PathBuf::from(format!("notes-{name}")))
                .unwrap();
        }
        config.global.current_workspace = current.to_string();
        let mut settings = AppSettings::default();
        settings.workspace_config = Some(config);
        settings
    }

    #[test]
    fn rows_are_sorted_by_name_with_the_current_one_marked() {
        let rows = workspace_rows(&settings(&["zeta", "alpha", "mid"], "mid"));
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["alpha", "mid", "zeta"]);
        let current: Vec<bool> = rows.iter().map(|r| r.current).collect();
        assert_eq!(current, [false, true, false]);
        assert_eq!(
            rows[0].folder,
            PathBuf::from("notes-alpha").display().to_string()
        );
        assert_eq!(rows[0].path, PathBuf::from("notes-alpha"));
        assert_eq!(initial_selection(&rows), Some(1));
    }

    #[test]
    fn no_config_or_no_workspace_is_an_empty_list() {
        assert!(workspace_rows(&AppSettings::default()).is_empty());
        let rows = workspace_rows(&settings(&[], ""));
        assert!(rows.is_empty());
        assert_eq!(initial_selection(&rows), None);
    }

    #[test]
    fn without_a_current_one_the_first_row_is_selected() {
        let rows = workspace_rows(&settings(&["b", "a"], "gone"));
        assert!(rows.iter().all(|r| !r.current));
        assert_eq!(initial_selection(&rows), Some(0));
    }

    #[test]
    fn picking_the_current_workspace_switches_to_nothing() {
        let rows = workspace_rows(&settings(&["a", "b"], "a"));
        assert_eq!(rows[0].switch_target(), None);
        assert_eq!(rows[1].switch_target(), Some("b"));
    }
}
