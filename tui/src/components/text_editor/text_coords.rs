//! Shared byte ↔ char coordinate conversion for editor text.
//!
//! Several places convert a UTF-8 *byte* offset within a line into a Unicode
//! *char* index (nvim cursor decode, the markdown parser's offset mapping).
//! This is the one canonical per-line kernel they share.
//!
//! Not every conversion fits here: [`super::autocomplete_glue::byte_to_row_char_col`]
//! deliberately returns `None` when the offset is mid-codepoint or past the end
//! (its callers rely on that to reject malformed offsets), whereas this kernel
//! is *defensive* — it snaps to the nearest valid boundary and clamps past the
//! end. Pick the one whose contract you want.

/// Convert a byte offset within a single line to a Unicode scalar (char) index.
///
/// Defensive: if `byte_col` lands inside a multi-byte sequence it snaps back to
/// the nearest valid char boundary; if it is past the end of `line` it clamps to
/// the line's char count.
pub fn byte_col_to_char_col(line: &str, byte_col: usize) -> usize {
    let safe = (0..=byte_col.min(line.len()))
        .rev()
        .find(|&i| line.is_char_boundary(i))
        .unwrap_or(0);
    line[..safe].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii() {
        assert_eq!(byte_col_to_char_col("hello", 3), 3);
    }

    #[test]
    fn multibyte_after_char() {
        // "wørld": w=1 byte, ø=2 bytes. Byte 3 is after "wø" → char idx 2.
        assert_eq!(byte_col_to_char_col("wørld", 3), 2);
    }

    #[test]
    fn snaps_mid_codepoint() {
        // Byte 2 lands inside ø (bytes 1..3) → snaps back to boundary 1 → char 1.
        assert_eq!(byte_col_to_char_col("wørld", 2), 1);
    }

    #[test]
    fn clamps_past_end() {
        assert_eq!(byte_col_to_char_col("ab", 99), 2);
    }

    #[test]
    fn zero() {
        assert_eq!(byte_col_to_char_col("ab", 0), 0);
    }
}
