/// All icon strings used across the UI, resolved once from the `use_nerd_fonts` setting.
///
/// Build with [`Icons::new`] after loading settings, then pass `&Icons` (or a clone)
/// wherever an icon is needed — no `use_nerd_fonts` checks at render time.
#[derive(Clone)]
pub struct Icons {
    // File-list entry icons
    pub directory: &'static str,
    pub directory_up: &'static str,
    pub note: &'static str,
    pub journal: &'static str,
    pub attachment: &'static str,
    // UI chrome icons
    pub info: &'static str,
}

impl Icons {
    pub fn new(use_nerd_fonts: bool) -> Self {
        if use_nerd_fonts {
            Self {
                directory: "󰉋",
                directory_up: "󰁝",
                note: "󰈙",
                journal: "󰃭",
                attachment: "",
                info: "󰋽",
            }
        } else {
            Self {
                directory: "[D]",
                directory_up: "[^]",
                note: "[-]",
                journal: "[J]",
                attachment: "   ",
                info: "(i)",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nerd_fonts_info_icon_is_not_ascii() {
        let icons = Icons::new(true);
        assert!(!icons.info.is_ascii());
    }

    #[test]
    fn plain_icons_are_ascii() {
        let icons = Icons::new(false);
        assert!(icons.info.is_ascii());
        assert!(icons.directory.is_ascii());
        assert!(icons.directory_up.is_ascii());
        assert!(icons.note.is_ascii());
        assert!(icons.journal.is_ascii());
    }
}
