//! Which text a vim **text object** names.
//!
//! `iw`, `a(`, `i"` — which span an object designates, separate from what an
//! operator then does with it. These are vim's conventions and not the
//! engine's: whether `aw` swallows trailing whitespace, whether `i(` includes
//! its brackets. The primitives underneath — word classes, the matching
//! bracket — belong to `ropetext` (adr/0041).

use super::rope_buffer::RopeBuffer;

/// A text object (`iw`, `a"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObject {
    Word {
        around: bool,
    },
    Pair {
        open: char,
        close: char,
        around: bool,
    },
    Quote {
        ch: char,
        around: bool,
    },
}

/// Map an object char (e.g. `w`, `(`, `"`) to a `TextObject`.
pub(super) fn object_for_char(c: char, around: bool) -> Option<TextObject> {
    match c {
        'w' => Some(TextObject::Word { around }),
        '(' | ')' | 'b' => Some(TextObject::Pair {
            open: '(',
            close: ')',
            around,
        }),
        '{' | '}' | 'B' => Some(TextObject::Pair {
            open: '{',
            close: '}',
            around,
        }),
        '[' | ']' => Some(TextObject::Pair {
            open: '[',
            close: ']',
            around,
        }),
        '<' | '>' => Some(TextObject::Pair {
            open: '<',
            close: '>',
            around,
        }),
        '"' => Some(TextObject::Quote { ch: '"', around }),
        '\'' => Some(TextObject::Quote { ch: '\'', around }),
        '`' => Some(TextObject::Quote { ch: '`', around }),
        _ => None,
    }
}

/// Apply `op` over the text object `obj` at the current cursor position.
/// Resolve `obj` at the cursor to `(row, start, end)` — half-open cols on
/// the cursor's row (text objects are single-line for now). Shared by the
/// operator path (`diw`) and the visual path (`vi(`).
pub(super) fn object_range_at_cursor(
    ta: &RopeBuffer,
    obj: TextObject,
) -> Option<(usize, usize, usize)> {
    let (row, col) = super::cursor_tuple(ta);
    let line = ta.lines().get(row)?;
    let chars: Vec<char> = line.chars().collect();
    let (start, end) = object_range(&chars, col, obj)?;
    Some((row, start, end))
}

/// Find the innermost enclosing pair `(open, close)` around `col`.
/// If the cursor is on an open bracket, that bracket is the enclosing open.
/// Otherwise scans left with depth counting (closing chars raise depth) to
/// find the nearest unmatched open, then scans right from that open with
/// depth counting to find the matching close.
pub(super) fn find_enclosing_pair(
    chars: &[char],
    col: usize,
    open: char,
    close: char,
) -> Option<(usize, usize)> {
    // Locate the open bracket that encloses col.
    let open_idx = if chars.get(col) == Some(&open) {
        col
    } else {
        let mut depth = 0usize;
        let mut found = None;
        for i in (0..col).rev() {
            if chars[i] == close {
                depth += 1;
            } else if chars[i] == open {
                if depth == 0 {
                    found = Some(i);
                    break;
                }
                depth -= 1;
            }
        }
        found?
    };
    // Find the matching close bracket scanning right from open_idx+1.
    let mut depth = 0usize;
    let mut close_idx = None;
    for (i, &ch) in chars.iter().enumerate().skip(open_idx + 1) {
        if ch == open {
            depth += 1;
        } else if ch == close {
            if depth == 0 {
                close_idx = Some(i);
                break;
            }
            depth -= 1;
        }
    }
    Some((open_idx, close_idx?))
}

/// Returns the half-open `[start, end)` char range for `obj` centred at
/// `col` within `chars`.
///
/// NOTE: text objects are **single-line** in this implementation.
/// Multi-line pair/quote spans are a later enhancement.
pub(super) fn object_range(chars: &[char], col: usize, obj: TextObject) -> Option<(usize, usize)> {
    if chars.is_empty() || col >= chars.len() {
        return None;
    }
    match obj {
        TextObject::Word { around } => {
            let is_word = |c: char| c.is_alphanumeric() || c == '_';
            // Expand left to the start of the word.
            let mut s = col;
            while s > 0 && is_word(chars[s - 1]) {
                s -= 1;
            }
            // Expand right past the end of the word.
            let mut e = col;
            while e < chars.len() && is_word(chars[e]) {
                e += 1;
            }
            if around {
                // Also consume trailing whitespace (vim `aw` behaviour).
                while e < chars.len() && chars[e].is_whitespace() {
                    e += 1;
                }
            }
            Some((s, e))
        }
        TextObject::Quote { ch, around } => {
            // Collect all positions of the quote character on this line.
            let positions: Vec<usize> = chars
                .iter()
                .enumerate()
                .filter(|&(_, &c)| c == ch)
                .map(|(i, _)| i)
                .collect();
            // Find the pair that strictly contains the cursor (p[0] <= col <= p[1]).
            // Cursor in the gap between two quoted spans returns None (no-op).
            let pair = positions
                .chunks(2)
                .find(|p| p.len() == 2 && p[0] <= col && col <= p[1])?;
            let (o, c) = (pair[0], pair[1]);
            if around {
                Some((o, c + 1))
            } else {
                Some((o + 1, c))
            }
        }
        TextObject::Pair {
            open,
            close,
            around,
        } => {
            let (o, c) = find_enclosing_pair(chars, col, open, close)?;
            if around {
                Some((o, c + 1))
            } else {
                Some((o + 1, c))
            }
        }
    }
}
