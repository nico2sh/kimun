//! Cross-platform filename rule primitives.
//!
//! Shared by note-filename sanitization (in `super::VaultPathSlice`) and
//! workspace-name validation (in `crate::nfs::filename::validate_filename`,
//! consumed by the TUI). The same rule set drives two different consumer
//! behaviors: nfs replaces disallowed characters; the TUI rejects them.

use std::sync::LazyLock;

use regex::Regex;

/// Disallowed: \ / : * ? " < > | [ ] ^ # and control chars (U+0000-U+001F, U+007F)
const NON_VALID_PATH_CHARS_REGEX: &str = r#"[\\/:*?"<>|\[\]\^\#\x00-\x1f\x7f]"#;
/// Two-or-more leading dots (e.g. "..foo")
const NON_VALID_PATH_NAME: &str = r#"^\.{2,}.+$"#;
/// Windows reserved device names, case-insensitive, with optional extension.
const WINDOWS_RESERVED_NAMES_REGEX: &str = r#"(?i)^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\..+)?$"#;

pub(crate) static RX_PATH_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(NON_VALID_PATH_CHARS_REGEX).unwrap());
pub(crate) static RX_PATH_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(NON_VALID_PATH_NAME).unwrap());
pub(crate) static RX_WIN_RESERVED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(WINDOWS_RESERVED_NAMES_REGEX).unwrap());

pub fn is_disallowed_char(c: char) -> bool {
    let mut buf = [0u8; 4];
    RX_PATH_CHARS.is_match(c.encode_utf8(&mut buf))
}

pub fn is_windows_reserved(name: &str) -> bool {
    RX_WIN_RESERVED.is_match(name)
}

pub fn has_invalid_leading_dots(name: &str) -> bool {
    RX_PATH_NAME.is_match(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disallowed_chars_match_legacy_set() {
        for c in ['\\', '/', ':', '*', '?', '"', '<', '>', '|', '[', ']', '^', '#'] {
            assert!(is_disallowed_char(c), "{c:?} should be disallowed");
        }
        for c in ['\u{0000}', '\u{001f}', '\u{007f}'] {
            assert!(is_disallowed_char(c), "control {c:?} should be disallowed");
        }
        for c in ['a', '1', '_', '-', '.', ' ', 'ñ'] {
            assert!(!is_disallowed_char(c), "{c:?} should be allowed");
        }
    }

    #[test]
    fn windows_reserved_detection() {
        for n in ["CON", "con", "Prn.txt", "AUX", "nul", "com1", "COM9", "lpt1", "LPT9"] {
            assert!(is_windows_reserved(n), "{n} should be reserved");
        }
        for n in ["console", "communicator", "lptest", "foo"] {
            assert!(!is_windows_reserved(n), "{n} should not be reserved");
        }
    }

    #[test]
    fn invalid_leading_dots_detection() {
        assert!(has_invalid_leading_dots("..foo"));
        assert!(has_invalid_leading_dots("...bar"));
        assert!(!has_invalid_leading_dots(".foo"));
        assert!(!has_invalid_leading_dots("foo"));
        assert!(!has_invalid_leading_dots(".."));
    }
}
