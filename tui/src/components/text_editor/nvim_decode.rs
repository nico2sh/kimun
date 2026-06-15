//! Pure decode of the `STATE_QUERY_LUA` response into a [`DecodedState`].
//!
//! This is the **decode seam**: every nvim wire-format fact lives here and
//! nowhere else — the positional array shape, the byte-offset → char-index
//! conversion, the 1-indexed → 0-indexed shift, the mode-string → [`EditorMode`]
//! mapping, and the `getpos('v')` visual-mark math. The function is pure
//! (`Value` in, `DecodedState` out) so the whole contract is testable with
//! literal `Value`s, without spawning a real nvim process.
//!
//! Applying a [`DecodedState`] to the live `Mutex<NvimSnapshot>` — the
//! `in_flight` gate and the `content_gen`/`dirty` bookkeeping — stays in
//! `backend.rs`, because that is stateful backend bookkeeping, not decoding.

use super::snapshot::EditorMode;

/// One decoded `STATE_QUERY_LUA` response.
///
/// Mirrors the two shapes the Lua snippet can return:
/// - command mode → `[mode, cmdtype, cmdline]`
/// - every other mode → `[mode, lines, cursor, vpos]`
#[derive(Debug, Clone, PartialEq)]
pub enum DecodedState {
    /// Nvim is in command-line mode. `cmdline` includes the type prefix
    /// (e.g. `":set nu"` or `"/pattern"`).
    Command { cmdline: String },
    /// Any non-command mode: a full buffer + cursor + (maybe) selection.
    Content {
        mode: EditorMode,
        /// Buffer lines, 0-indexed. Never empty (a blank buffer decodes to
        /// `vec![String::new()]`).
        lines: Vec<String>,
        /// `(row, char_col)`, both 0-indexed. `char_col` is a Unicode scalar
        /// index, already converted from nvim's byte offset.
        cursor: (usize, usize),
        /// Active visual selection in logical `(row, char-col)` coords,
        /// 0-indexed. `None` outside visual modes. For `VisualLine` the end
        /// col is `usize::MAX`.
        visual_selection: Option<((usize, usize), (usize, usize))>,
    },
}

/// Convert a UTF-8 byte offset to a Unicode scalar (char) index.
///
/// `nvim_win_get_cursor` and `getpos()` return byte offsets. This converts them
/// to char indices so the rest of the rendering pipeline can use char-indexed
/// operations consistently. If the offset falls in the middle of a multi-byte
/// sequence it is snapped to the nearest valid char boundary.
pub fn byte_offset_to_char_idx(line: &str, byte_offset: usize) -> usize {
    // Walk backward from the offset to the nearest valid char boundary, then
    // count chars up to that point. Handles mid-codepoint offsets safely.
    let safe = (0..=byte_offset.min(line.len()))
        .rev()
        .find(|&i| line.is_char_boundary(i))
        .unwrap_or(0);
    line[..safe].chars().count()
}

/// Decode one `STATE_QUERY_LUA` response. Returns `None` when the value is not
/// the expected array or the leading mode element is missing — the caller
/// leaves the snapshot untouched in that case.
pub fn decode(value: &nvim_rs::Value) -> Option<DecodedState> {
    let arr = value.as_array()?;
    let mode_str = arr.first().and_then(|v| v.as_str())?;
    let mode = EditorMode::from_nvim_str(mode_str);

    if mode == EditorMode::Command {
        // [mode, cmdtype, cmdline]
        let cmdtype = arr.get(1).and_then(|v| v.as_str()).unwrap_or("");
        let cmdline = arr.get(2).and_then(|v| v.as_str()).unwrap_or("");
        return Some(DecodedState::Command {
            cmdline: format!("{cmdtype}{cmdline}"),
        });
    }

    // [mode, lines, cursor, vpos]

    // Lines. A blank buffer comes back as `[]` or `[""]`; normalise to a
    // single empty line so downstream code never sees a zero-length buffer.
    let lines: Vec<String> = arr
        .get(1)
        .and_then(|v| v.as_array())
        .map(|ls| {
            ls.iter()
                .filter_map(|l| l.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let lines = if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    };

    // Cursor: nvim_win_get_cursor → [row(1-indexed), col(0-indexed byte offset)].
    // Convert the byte offset to a char index so all downstream code works in
    // char-index space uniformly (independent of multi-byte character widths).
    let cursor = arr
        .get(2)
        .and_then(|v| v.as_array())
        .and_then(|c| {
            let row = c.first()?.as_u64()? as usize;
            let byte_col = c.get(1)?.as_u64()? as usize;
            let row0 = row.saturating_sub(1);
            let char_col = lines
                .get(row0)
                .map(|line| byte_offset_to_char_idx(line, byte_col))
                .unwrap_or(byte_col);
            Some((row0, char_col))
        })
        .unwrap_or((0, 0));

    // Visual selection: getpos("v") → [bufnum, lnum(1-indexed), col(1-indexed byte offset), off].
    // Convert the 1-indexed byte col to a 0-indexed char index.
    let visual_selection = if matches!(mode, EditorMode::Visual | EditorMode::VisualLine) {
        arr.get(3)
            .and_then(|v| v.as_array())
            .and_then(|p| {
                let lnum = p.get(1)?.as_u64()? as usize;
                let vcol_byte = p.get(2)?.as_u64()? as usize;
                if lnum == 0 {
                    return None;
                }
                let row0 = lnum.saturating_sub(1);
                let char_col = lines
                    .get(row0)
                    .map(|line| byte_offset_to_char_idx(line, vcol_byte.saturating_sub(1)))
                    .unwrap_or(vcol_byte.saturating_sub(1));
                Some((row0, char_col))
            })
            .map(|anchor| {
                let (mut start, mut end) = if anchor <= cursor {
                    (anchor, cursor)
                } else {
                    (cursor, anchor)
                };
                if mode == EditorMode::VisualLine {
                    start.1 = 0;
                    end.1 = usize::MAX;
                }
                (start, end)
            })
    } else {
        None
    };

    Some(DecodedState::Content {
        mode,
        lines,
        cursor,
        visual_selection,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nvim_rs::Value;

    fn s(text: &str) -> Value {
        Value::from(text)
    }
    fn u(n: u64) -> Value {
        Value::from(n)
    }
    fn arr(items: Vec<Value>) -> Value {
        Value::Array(items)
    }

    // --- byte_offset_to_char_idx ------------------------------------------

    #[test]
    fn byte_to_char_ascii() {
        assert_eq!(byte_offset_to_char_idx("hello", 3), 3);
    }

    #[test]
    fn byte_to_char_multibyte() {
        // "wørld": w=1 byte, ø=2 bytes. Byte offset 3 is after "wø" → char idx 2.
        assert_eq!(byte_offset_to_char_idx("wørld", 3), 2);
    }

    #[test]
    fn byte_to_char_snaps_mid_codepoint() {
        // Byte offset 2 lands inside ø (bytes 1..3) → snaps back to boundary 1 → char idx 1.
        assert_eq!(byte_offset_to_char_idx("wørld", 2), 1);
    }

    #[test]
    fn byte_to_char_past_end_clamps() {
        assert_eq!(byte_offset_to_char_idx("ab", 99), 2);
    }

    // --- decode: non-array / malformed ------------------------------------

    #[test]
    fn decode_non_array_is_none() {
        assert_eq!(decode(&s("nope")), None);
    }

    #[test]
    fn decode_missing_mode_is_none() {
        assert_eq!(decode(&arr(vec![])), None);
    }

    // --- decode: content mode ---------------------------------------------

    #[test]
    fn decode_normal_cursor_ascii() {
        // [mode, lines, cursor[row1, bytecol0], vpos]
        let v = arr(vec![
            s("n"),
            arr(vec![s("hello"), s("world")]),
            arr(vec![u(2), u(3)]),
            arr(vec![u(0), u(0), u(0), u(0)]),
        ]);
        let d = decode(&v).unwrap();
        assert_eq!(
            d,
            DecodedState::Content {
                mode: EditorMode::Normal,
                lines: vec!["hello".into(), "world".into()],
                cursor: (1, 3), // row 2 → row0 1; byte col 3 → char 3
                visual_selection: None,
            }
        );
    }

    #[test]
    fn decode_cursor_multibyte_converts_byte_to_char() {
        // Line "wørld"; nvim byte col 4 is after "wør" (1+2+1) → char idx 3.
        let v = arr(vec![
            s("n"),
            arr(vec![s("wørld")]),
            arr(vec![u(1), u(4)]),
            arr(vec![u(0), u(0), u(0), u(0)]),
        ]);
        match decode(&v).unwrap() {
            DecodedState::Content { cursor, .. } => assert_eq!(cursor, (0, 3)),
            other => panic!("expected Content, got {other:?}"),
        }
    }

    #[test]
    fn decode_empty_buffer_normalises_to_single_blank_line() {
        let v = arr(vec![
            s("n"),
            arr(vec![]),
            arr(vec![u(1), u(0)]),
            arr(vec![u(0), u(0), u(0), u(0)]),
        ]);
        match decode(&v).unwrap() {
            DecodedState::Content { lines, cursor, .. } => {
                assert_eq!(lines, vec![String::new()]);
                assert_eq!(cursor, (0, 0));
            }
            other => panic!("expected Content, got {other:?}"),
        }
    }

    #[test]
    fn decode_unknown_mode_is_other() {
        let v = arr(vec![
            s("t"), // terminal mode — unmapped
            arr(vec![s("x")]),
            arr(vec![u(1), u(0)]),
            arr(vec![u(0), u(0), u(0), u(0)]),
        ]);
        match decode(&v).unwrap() {
            DecodedState::Content { mode, .. } => {
                assert_eq!(mode, EditorMode::Other("t".into()))
            }
            other => panic!("expected Content, got {other:?}"),
        }
    }

    // --- decode: command mode ---------------------------------------------

    #[test]
    fn decode_command_mode_concatenates_type_and_line() {
        // [mode, cmdtype, cmdline]
        let v = arr(vec![s("c"), s(":"), s("set nu")]);
        assert_eq!(
            decode(&v).unwrap(),
            DecodedState::Command {
                cmdline: ":set nu".into()
            }
        );
    }

    #[test]
    fn decode_command_search_prefix() {
        let v = arr(vec![s("c"), s("/"), s("pattern")]);
        assert_eq!(
            decode(&v).unwrap(),
            DecodedState::Command {
                cmdline: "/pattern".into()
            }
        );
    }

    // --- decode: visual selection -----------------------------------------

    #[test]
    fn decode_visual_anchor_before_cursor() {
        // Anchor at (row1=1, col1byte=1) → (0,0); cursor (row2, bytecol2) → (0, 2... wait single line)
        // Single line "abcdef": anchor byte col 1 (1-indexed) → char 0; cursor byte col 3 → char 3.
        let v = arr(vec![
            s("v"),
            arr(vec![s("abcdef")]),
            arr(vec![u(1), u(3)]),                 // cursor → (0, 3)
            arr(vec![u(0), u(1), u(1), u(0)]),     // getpos('v') anchor → (0, 0)
        ]);
        match decode(&v).unwrap() {
            DecodedState::Content {
                visual_selection, ..
            } => assert_eq!(visual_selection, Some(((0, 0), (0, 3)))),
            other => panic!("expected Content, got {other:?}"),
        }
    }

    #[test]
    fn decode_visual_anchor_after_cursor_orders_start_end() {
        // anchor byte col 5 → char 4; cursor byte col 1 → char 1. start=cursor, end=anchor.
        let v = arr(vec![
            s("v"),
            arr(vec![s("abcdef")]),
            arr(vec![u(1), u(1)]),                 // cursor → (0, 1)
            arr(vec![u(0), u(1), u(5), u(0)]),     // anchor → (0, 4)
        ]);
        match decode(&v).unwrap() {
            DecodedState::Content {
                visual_selection, ..
            } => assert_eq!(visual_selection, Some(((0, 1), (0, 4)))),
            other => panic!("expected Content, got {other:?}"),
        }
    }

    #[test]
    fn decode_visual_line_spans_full_columns() {
        let v = arr(vec![
            s("V"),
            arr(vec![s("abc"), s("defgh")]),
            arr(vec![u(2), u(2)]),                 // cursor row2 → (1, 2)
            arr(vec![u(0), u(1), u(1), u(0)]),     // anchor row1 → (0, 0)
        ]);
        match decode(&v).unwrap() {
            DecodedState::Content {
                mode,
                visual_selection,
                ..
            } => {
                assert_eq!(mode, EditorMode::VisualLine);
                // start col forced to 0, end col forced to usize::MAX.
                assert_eq!(visual_selection, Some(((0, 0), (1, usize::MAX))));
            }
            other => panic!("expected Content, got {other:?}"),
        }
    }

    #[test]
    fn decode_visual_with_zero_lnum_anchor_is_none() {
        let v = arr(vec![
            s("v"),
            arr(vec![s("abc")]),
            arr(vec![u(1), u(0)]),
            arr(vec![u(0), u(0), u(0), u(0)]), // lnum 0 → no selection
        ]);
        match decode(&v).unwrap() {
            DecodedState::Content {
                visual_selection, ..
            } => assert_eq!(visual_selection, None),
            other => panic!("expected Content, got {other:?}"),
        }
    }
}
