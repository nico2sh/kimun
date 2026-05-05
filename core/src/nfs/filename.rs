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

const MAX_FILENAME_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidNameReason {
    Empty,
    DisallowedChars(Vec<char>),
    LeadingDots,
    TrailingDot,
    LeadingOrTrailingWhitespace,
    ReservedWindowsName,
    TooLong { actual: usize, max: usize },
}

#[derive(Debug, Clone)]
pub struct InvalidFilenameError {
    pub name: String,
    pub reasons: Vec<InvalidNameReason>,
}

impl std::fmt::Display for InvalidFilenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "name '{}' is invalid:", self.name)?;
        for (i, r) in self.reasons.iter().enumerate() {
            let sep = if i == 0 { " " } else { "; " };
            match r {
                InvalidNameReason::Empty => write!(f, "{sep}is empty")?,
                InvalidNameReason::DisallowedChars(chars) => {
                    let list: Vec<String> = chars.iter().map(|c| format!("'{c}'")).collect();
                    write!(f, "{sep}contains disallowed characters {}", list.join(", "))?;
                }
                InvalidNameReason::LeadingDots => write!(f, "{sep}starts with two or more dots")?,
                InvalidNameReason::TrailingDot => write!(f, "{sep}ends with a dot")?,
                InvalidNameReason::LeadingOrTrailingWhitespace => {
                    write!(f, "{sep}has leading or trailing whitespace")?
                }
                InvalidNameReason::ReservedWindowsName => {
                    write!(f, "{sep}is a Windows reserved name")?
                }
                InvalidNameReason::TooLong { actual, max } => {
                    write!(f, "{sep}is {actual} chars (max {max})")?
                }
            }
        }
        Ok(())
    }
}

impl std::error::Error for InvalidFilenameError {}

pub fn validate_filename(name: &str) -> Result<(), InvalidFilenameError> {
    let mut reasons = Vec::new();

    if name.is_empty() {
        reasons.push(InvalidNameReason::Empty);
        return Err(InvalidFilenameError {
            name: name.to_string(),
            reasons,
        });
    }

    let mut bad: Vec<char> = name.chars().filter(|c| is_disallowed_char(*c)).collect();
    if !bad.is_empty() {
        bad.sort_unstable();
        bad.dedup();
        reasons.push(InvalidNameReason::DisallowedChars(bad));
    }
    if has_invalid_leading_dots(name) {
        reasons.push(InvalidNameReason::LeadingDots);
    }
    if name.trim_end().ends_with('.') {
        reasons.push(InvalidNameReason::TrailingDot);
    }
    if name.starts_with(char::is_whitespace) || name.ends_with(char::is_whitespace) {
        reasons.push(InvalidNameReason::LeadingOrTrailingWhitespace);
    }
    if is_windows_reserved(name) {
        reasons.push(InvalidNameReason::ReservedWindowsName);
    }
    let len = name.chars().count();
    if len > MAX_FILENAME_LEN {
        reasons.push(InvalidNameReason::TooLong {
            actual: len,
            max: MAX_FILENAME_LEN,
        });
    }

    if reasons.is_empty() {
        Ok(())
    } else {
        Err(InvalidFilenameError {
            name: name.to_string(),
            reasons,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disallowed_chars_match_legacy_set() {
        for c in [
            '\\', '/', ':', '*', '?', '"', '<', '>', '|', '[', ']', '^', '#',
        ] {
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
        for n in [
            "CON", "con", "Prn.txt", "AUX", "nul", "com1", "COM9", "lpt1", "LPT9",
        ] {
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

    #[test]
    fn validate_accepts_simple_lowercase() {
        assert!(validate_filename("notes").is_ok());
        assert!(validate_filename("my-vault").is_ok());
        assert!(validate_filename("v1.0").is_ok());
    }

    #[test]
    fn validate_rejects_empty() {
        let err = validate_filename("").unwrap_err();
        assert_eq!(err.reasons, vec![InvalidNameReason::Empty]);
    }

    #[test]
    fn validate_lists_disallowed_chars_deduped_sorted() {
        let err = validate_filename("foo/bar?baz/qux?").unwrap_err();
        assert!(matches!(
            err.reasons.as_slice(),
            [InvalidNameReason::DisallowedChars(chars)] if *chars == vec!['/', '?']
        ));
    }

    #[test]
    fn validate_rejects_windows_reserved_case_insensitive() {
        for name in ["con", "CON", "Prn", "nul.txt"] {
            let err = validate_filename(name).unwrap_err();
            assert!(err
                .reasons
                .contains(&InvalidNameReason::ReservedWindowsName));
        }
    }

    #[test]
    fn validate_rejects_leading_dots_and_trailing_dot() {
        assert!(validate_filename("..foo")
            .unwrap_err()
            .reasons
            .contains(&InvalidNameReason::LeadingDots));
        assert!(validate_filename("foo.")
            .unwrap_err()
            .reasons
            .contains(&InvalidNameReason::TrailingDot));
    }

    #[test]
    fn validate_rejects_leading_or_trailing_whitespace() {
        assert!(validate_filename(" foo")
            .unwrap_err()
            .reasons
            .contains(&InvalidNameReason::LeadingOrTrailingWhitespace));
        assert!(validate_filename("foo ")
            .unwrap_err()
            .reasons
            .contains(&InvalidNameReason::LeadingOrTrailingWhitespace));
    }

    #[test]
    fn validate_rejects_overlong() {
        let name = "a".repeat(65);
        let err = validate_filename(&name).unwrap_err();
        assert!(err.reasons.iter().any(|r| matches!(
            r,
            InvalidNameReason::TooLong {
                actual: 65,
                max: 64
            }
        )));
    }

    #[test]
    fn validate_collects_multiple_reasons() {
        let err = validate_filename(" CON/foo. ").unwrap_err();
        let reasons = err.reasons;
        assert!(reasons
            .iter()
            .any(|r| matches!(r, InvalidNameReason::DisallowedChars(_))));
        assert!(reasons.contains(&InvalidNameReason::LeadingOrTrailingWhitespace));
        assert!(reasons.contains(&InvalidNameReason::TrailingDot));
    }

    #[test]
    fn display_message_lists_offending_chars() {
        let err = validate_filename("a/b!c?").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("'a/b!c?'"));
        assert!(msg.contains("/"));
        assert!(msg.contains("?"));
    }
}
