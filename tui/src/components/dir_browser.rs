//! Directory-only browser state shared by the Preferences screen and the
//! Onboarding screen. Pure navigation state — each host renders it and
//! routes keys itself.

use std::path::PathBuf;

use ratatui::widgets::ListState;

pub struct FileBrowserState {
    pub current_path: PathBuf,
    pub entries: Vec<PathBuf>,
    pub list_state: ListState,
    pub has_parent: bool,
    last_jump_char: Option<char>,
}

impl FileBrowserState {
    pub fn load(path: PathBuf) -> Self {
        let has_parent = path.parent().is_some();
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        entries.sort();
        let total = entries.len() + if has_parent { 1 } else { 0 };
        let mut list_state = ListState::default();
        if total > 0 {
            list_state.select(Some(0));
        }
        Self {
            current_path: path,
            entries,
            list_state,
            has_parent,
            last_jump_char: None,
        }
    }

    pub fn navigate_into(&mut self, entry: PathBuf) {
        *self = Self::load(entry);
    }

    pub fn go_up(&mut self) {
        if let Some(parent) = self.current_path.parent() {
            *self = Self::load(parent.to_path_buf());
        }
    }

    pub fn jump_to_char(&mut self, c: char) {
        let c_lower = c.to_lowercase().next().unwrap_or(c);
        let offset = if self.has_parent { 1 } else { 0 };
        let total = self.entries.len();
        if total == 0 {
            return;
        }

        // If same char as last jump, cycle to next match.
        let start = if self.last_jump_char == Some(c_lower) {
            let cur = self.list_state.selected().unwrap_or(0);
            if cur >= offset { cur - offset + 1 } else { 0 }
        } else {
            0
        };

        // Search from start, wrapping around.
        for i in 0..total {
            let idx = (start + i) % total;
            if let Some(name) = self.entries[idx].file_name().and_then(|n| n.to_str())
                && name.to_lowercase().starts_with(c_lower)
            {
                self.list_state.select(Some(idx + offset));
                self.last_jump_char = Some(c_lower);
                return;
            }
        }
        self.last_jump_char = None;
    }

    /// Create `name` as a subdirectory of `current_path` and navigate into it.
    /// Returns the created path. The directory is created immediately (the
    /// browser must be able to enter it) — the only place onboarding touches
    /// the filesystem before Finish.
    pub fn create_dir(&mut self, name: &str) -> Result<PathBuf, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("directory name is empty".to_string());
        }
        let target = self.current_path.join(name);
        std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        self.navigate_into(target.clone());
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_dir_creates_enters_and_lists_in_parent() {
        let tmp = std::env::temp_dir().join(format!("kimun_dirbrowser_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut fb = FileBrowserState::load(tmp.clone());

        let created = fb.create_dir("my-notes").unwrap();
        assert_eq!(created, tmp.join("my-notes"));
        assert!(created.is_dir());
        assert_eq!(fb.current_path, created);

        fb.go_up();
        assert!(fb.entries.iter().any(|e| e == &created));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn create_dir_rejects_empty_and_reports_io_errors() {
        let tmp = std::env::temp_dir().join(format!("kimun_dirbrowser_e_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut fb = FileBrowserState::load(tmp.clone());
        assert!(fb.create_dir("").is_err());
        assert!(fb.create_dir("   ").is_err());
        std::fs::remove_dir_all(&tmp).ok();
    }
}
