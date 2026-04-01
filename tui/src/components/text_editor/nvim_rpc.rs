use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Convert a crossterm `KeyEvent` to a Neovim key string suitable for `nvim_feedkeys`.
///
/// Returns `None` for events that have no Neovim equivalent (e.g., modifier-only events).
/// Space maps to `<Space>` and `<` maps to `<lt>` to avoid ambiguity in Neovim's key parser.
/// Ctrl/Alt modifiers are applied to all key types, e.g. Ctrl+Up → `<C-Up>`.
pub fn key_event_to_nvim_string(key: &KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    // Characters are handled separately: the char itself already encodes shift
    // (uppercase), so we only need to apply ctrl/alt wrappers.
    if let KeyCode::Char(c) = key.code {
        if ctrl {
            return Some(format!("<C-{}>", c.to_lowercase()));
        }
        if alt {
            return Some(format!("<A-{c}>"));
        }
        return Some(match c {
            ' ' => "<Space>".into(),
            '<' => "<lt>".into(),
            _ => c.to_string(),
        });
    }

    // For all other keys, get the inner name (without angle brackets) and then
    // wrap with any modifier prefix.
    let inner: String = match key.code {
        KeyCode::Enter => "CR".into(),
        KeyCode::Backspace => "BS".into(),
        KeyCode::Delete => "Del".into(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Tab => "Tab".into(),
        // BackTab is Shift+Tab — always emitted as <S-Tab> regardless of modifiers.
        KeyCode::BackTab => return Some("<S-Tab>".into()),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::F(n) => format!("F{n}"),
        _ => return None,
    };

    Some(match (ctrl, alt, shift) {
        (true, _, _) => format!("<C-{inner}>"),
        (_, true, _) => format!("<A-{inner}>"),
        (_, _, true) => format!("<S-{inner}>"),
        _ => format!("<{inner}>"),
    })
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use ratatui::crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn letter_j_in_normal() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Char('j'), KeyModifiers::NONE)),
            Some("j".into())
        );
    }

    #[test]
    fn uppercase_j_with_shift() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Char('J'), KeyModifiers::SHIFT)),
            Some("J".into())
        );
    }

    #[test]
    fn enter_maps_to_cr() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Enter, KeyModifiers::NONE)),
            Some("<CR>".into())
        );
    }

    #[test]
    fn backspace_maps_to_bs() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Backspace, KeyModifiers::NONE)),
            Some("<BS>".into())
        );
    }

    #[test]
    fn delete_maps_to_del() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Delete, KeyModifiers::NONE)),
            Some("<Del>".into())
        );
    }

    #[test]
    fn escape_maps_to_esc() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Esc, KeyModifiers::NONE)),
            Some("<Esc>".into())
        );
    }

    #[test]
    fn tab_maps_to_tab() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Tab, KeyModifiers::NONE)),
            Some("<Tab>".into())
        );
    }

    #[test]
    fn ctrl_w_maps() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            Some("<C-w>".into())
        );
    }

    #[test]
    fn ctrl_r_maps() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Some("<C-r>".into())
        );
    }

    #[test]
    fn arrow_up() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Up, KeyModifiers::NONE)),
            Some("<Up>".into())
        );
    }

    #[test]
    fn arrow_down() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Down, KeyModifiers::NONE)),
            Some("<Down>".into())
        );
    }

    #[test]
    fn arrow_left() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Left, KeyModifiers::NONE)),
            Some("<Left>".into())
        );
    }

    #[test]
    fn arrow_right() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Right, KeyModifiers::NONE)),
            Some("<Right>".into())
        );
    }

    #[test]
    fn home_key() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Home, KeyModifiers::NONE)),
            Some("<Home>".into())
        );
    }

    #[test]
    fn end_key() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::End, KeyModifiers::NONE)),
            Some("<End>".into())
        );
    }

    #[test]
    fn page_up() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::PageUp, KeyModifiers::NONE)),
            Some("<PageUp>".into())
        );
    }

    #[test]
    fn page_down() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::PageDown, KeyModifiers::NONE)),
            Some("<PageDown>".into())
        );
    }

    #[test]
    fn f1_key() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::F(1), KeyModifiers::NONE)),
            Some("<F1>".into())
        );
    }

    #[test]
    fn f12_key() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::F(12), KeyModifiers::NONE)),
            Some("<F12>".into())
        );
    }

    #[test]
    fn alt_j() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Char('j'), KeyModifiers::ALT)),
            Some("<A-j>".into())
        );
    }

    #[test]
    fn space() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some("<Space>".into())
        );
    }

    #[test]
    fn less_than_char() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Char('<'), KeyModifiers::NONE)),
            Some("<lt>".into())
        );
    }

    #[test]
    fn backslash_char() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Char('\\'), KeyModifiers::NONE)),
            Some("\\".into())
        );
    }

    #[test]
    fn ctrl_up() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Up, KeyModifiers::CONTROL)),
            Some("<C-Up>".into())
        );
    }

    #[test]
    fn ctrl_down() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Down, KeyModifiers::CONTROL)),
            Some("<C-Down>".into())
        );
    }

    #[test]
    fn alt_left() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Left, KeyModifiers::ALT)),
            Some("<A-Left>".into())
        );
    }

    #[test]
    fn alt_right() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Right, KeyModifiers::ALT)),
            Some("<A-Right>".into())
        );
    }

    #[test]
    fn shift_up() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Up, KeyModifiers::SHIFT)),
            Some("<S-Up>".into())
        );
    }

    #[test]
    fn ctrl_page_down() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::PageDown, KeyModifiers::CONTROL)),
            Some("<C-PageDown>".into())
        );
    }

    #[test]
    fn ctrl_home() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::Home, KeyModifiers::CONTROL)),
            Some("<C-Home>".into())
        );
    }

    #[test]
    fn ctrl_end() {
        assert_eq!(
            key_event_to_nvim_string(&key(KeyCode::End, KeyModifiers::CONTROL)),
            Some("<C-End>".into())
        );
    }
}
