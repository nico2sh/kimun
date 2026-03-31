use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Convert a crossterm `KeyEvent` to a Neovim key string suitable for `nvim_feedkeys`.
///
/// Returns `None` for events that have no Neovim equivalent (e.g., modifier-only events).
/// Space maps to `<Space>` and `<` maps to `<lt>` to avoid ambiguity in Neovim's key parser.
pub fn key_event_to_nvim_string(key: &KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    let base = match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                return Some(format!("<C-{}>", c.to_lowercase()));
            }
            if alt {
                return Some(format!("<A-{c}>"));
            }
            match c {
                ' ' => return Some("<Space>".into()),
                '<' => return Some("<lt>".into()),
                _ => return Some(c.to_string()),
            }
        }
        KeyCode::Enter => "<CR>",
        KeyCode::Backspace => "<BS>",
        KeyCode::Delete => "<Del>",
        KeyCode::Esc => "<Esc>",
        KeyCode::Tab => "<Tab>",
        KeyCode::BackTab => "<S-Tab>",
        KeyCode::Up => "<Up>",
        KeyCode::Down => "<Down>",
        KeyCode::Left => "<Left>",
        KeyCode::Right => "<Right>",
        KeyCode::Home => "<Home>",
        KeyCode::End => "<End>",
        KeyCode::PageUp => "<PageUp>",
        KeyCode::PageDown => "<PageDown>",
        KeyCode::Insert => "<Insert>",
        KeyCode::F(n) => return Some(format!("<F{n}>")),
        _ => return None,
    };

    Some(base.to_string())
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers: mods, kind: KeyEventKind::Press, state: KeyEventState::NONE }
    }

    #[test]
    fn letter_j_in_normal() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('j'), KeyModifiers::NONE)), Some("j".into()));
    }

    #[test]
    fn uppercase_J_with_shift() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('J'), KeyModifiers::SHIFT)), Some("J".into()));
    }

    #[test]
    fn enter_maps_to_cr() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Enter, KeyModifiers::NONE)), Some("<CR>".into()));
    }

    #[test]
    fn backspace_maps_to_bs() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Backspace, KeyModifiers::NONE)), Some("<BS>".into()));
    }

    #[test]
    fn delete_maps_to_del() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Delete, KeyModifiers::NONE)), Some("<Del>".into()));
    }

    #[test]
    fn escape_maps_to_esc() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Esc, KeyModifiers::NONE)), Some("<Esc>".into()));
    }

    #[test]
    fn tab_maps_to_tab() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Tab, KeyModifiers::NONE)), Some("<Tab>".into()));
    }

    #[test]
    fn ctrl_w_maps() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('w'), KeyModifiers::CONTROL)), Some("<C-w>".into()));
    }

    #[test]
    fn ctrl_r_maps() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('r'), KeyModifiers::CONTROL)), Some("<C-r>".into()));
    }

    #[test]
    fn arrow_up() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Up, KeyModifiers::NONE)), Some("<Up>".into()));
    }

    #[test]
    fn arrow_down() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Down, KeyModifiers::NONE)), Some("<Down>".into()));
    }

    #[test]
    fn arrow_left() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Left, KeyModifiers::NONE)), Some("<Left>".into()));
    }

    #[test]
    fn arrow_right() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Right, KeyModifiers::NONE)), Some("<Right>".into()));
    }

    #[test]
    fn home_key() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Home, KeyModifiers::NONE)), Some("<Home>".into()));
    }

    #[test]
    fn end_key() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::End, KeyModifiers::NONE)), Some("<End>".into()));
    }

    #[test]
    fn page_up() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::PageUp, KeyModifiers::NONE)), Some("<PageUp>".into()));
    }

    #[test]
    fn page_down() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::PageDown, KeyModifiers::NONE)), Some("<PageDown>".into()));
    }

    #[test]
    fn f1_key() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::F(1), KeyModifiers::NONE)), Some("<F1>".into()));
    }

    #[test]
    fn f12_key() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::F(12), KeyModifiers::NONE)), Some("<F12>".into()));
    }

    #[test]
    fn alt_j() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('j'), KeyModifiers::ALT)), Some("<A-j>".into()));
    }

    #[test]
    fn space() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char(' '), KeyModifiers::NONE)), Some("<Space>".into()));
    }

    #[test]
    fn less_than_char() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('<'), KeyModifiers::NONE)), Some("<lt>".into()));
    }

    #[test]
    fn backslash_char() {
        assert_eq!(key_event_to_nvim_string(&key(KeyCode::Char('\\'), KeyModifiers::NONE)), Some("\\".into()));
    }
}
