//! The text value: a rope, a revision, and the only way to make a [`Position`].

use std::borrow::Cow;
use std::fmt;

use ropey::{Rope, RopeSlice};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};

use crate::position::{Column, Position, Revision, Span};

/// A text, as a value.
///
/// `Clone` is cheap: the rope shares its structure, so a clone is not a copy of
/// the text but *the same text*, held elsewhere. That is what lets a background
/// task, a history entry or a preview hold one without anybody duplicating a
/// buffer, and it is why a clone keeps the same [`Revision`] — only a change
/// mints a new one.
///
/// The text never contains a carriage return. A `\r\n` or a lone `\r` in the
/// input becomes `\n` on construction, so every layer above measures, wraps and
/// addresses exactly one kind of line break. Restoring a file's original line
/// endings on save is the caller's business; it has the file, this does not.
#[derive(Debug, Clone)]
pub struct Text {
    rope: Rope,
    revision: Revision,
}

impl Default for Text {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.rope.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

impl From<&str> for Text {
    /// Takes `s` as the text's content, normalising `\r\n` and lone `\r` to `\n`.
    fn from(s: &str) -> Self {
        Self {
            rope: Rope::from_str(&normalise_breaks(s)),
            revision: Revision::fresh(),
        }
    }
}

impl Text {
    /// An empty text: one row, zero characters.
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            revision: Revision::fresh(),
        }
    }

    /// Which state this text is. Every [`Position`] carries the revision it was
    /// built against, so one made against an earlier state is refused rather
    /// than read as some other place.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    /// Number of logical rows. A trailing newline opens a final empty row, so
    /// `"a\n"` is two rows and the cursor can sit on the second.
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    /// Row `row` without its line break, or `None` if there is no such row.
    ///
    /// Borrowed when the row sits inside one of the rope's chunks, owned when it
    /// straddles two. Nothing above holds a duplicate of the document, so a full
    /// pass over the text costs what it costs and a partial pass costs less.
    pub fn line(&self, row: usize) -> Option<Cow<'_, str>> {
        self.line_slice(row).map(cow_of)
    }

    /// Length of row `row` in Unicode scalars, excluding its line break.
    pub fn line_len_chars(&self, row: usize) -> Option<usize> {
        self.line_slice(row).map(|l| l.len_chars())
    }

    /// Every row, without line breaks.
    pub fn lines(&self) -> impl Iterator<Item = Cow<'_, str>> {
        (0..self.line_count()).filter_map(|row| self.line(row))
    }

    /// The text within `span`.
    ///
    /// `None` when the span addresses another revision of the text.
    pub fn slice(&self, span: Span) -> Option<Cow<'_, str>> {
        if span.revision() != self.revision {
            return None;
        }
        Some(cow_of(self.rope.byte_slice(span.byte_range())))
    }

    /// Whether `position` addresses a state this text has moved on from.
    pub fn is_stale(&self, position: Position) -> bool {
        position.revision() != self.revision
    }

    /// The position at `(row, column)`, or `None` if this text cannot address it.
    ///
    /// Refused rather than approximated: past the end of the row, past the last
    /// row, or partway through a grapheme cluster all yield `None`. A caller
    /// that gets `None` has asked for somewhere that does not exist, and a
    /// keystroke that does nothing is recoverable in a way one that edits the
    /// wrong place is not.
    pub fn position(&self, row: usize, column: Column) -> Option<Position> {
        let line = self.line_slice(row)?;
        if column.get() > line.len_chars() {
            return None;
        }
        let byte = self.rope.line_to_byte(row) + line.char_to_byte(column.get());
        if !self.is_cluster_boundary(byte) {
            return None;
        }
        Some(Position::new(byte, row, column, self.revision))
    }

    /// The position at byte offset `byte`, or `None` if that offset is past the
    /// end, inside a character, or inside a grapheme cluster.
    pub fn position_at_byte(&self, byte: usize) -> Option<Position> {
        if byte > self.rope.len_bytes()
            || !self.is_char_boundary(byte)
            || !self.is_cluster_boundary(byte)
        {
            return None;
        }
        Some(self.position_at_addressable_byte(byte))
    }

    /// The position at the start of the grapheme cluster containing `byte`.
    ///
    /// This one **approximates**, which is why it says so in its name. It exists
    /// for offsets that arrive from somewhere with a different idea of where a
    /// character ends — an external editor reporting a cursor in bytes, say —
    /// where refusing would drop the update entirely. Anything originating
    /// inside this crate should use [`Self::position_at_byte`] and be told it was
    /// wrong.
    ///
    /// `None` only when `byte` is past the end of the text.
    pub fn position_at_byte_snapped(&self, byte: usize) -> Option<Position> {
        if byte > self.rope.len_bytes() {
            return None;
        }
        let mut at = byte;
        while !self.is_char_boundary(at) {
            at -= 1;
        }
        if !self.is_cluster_boundary(at) {
            at = self.cluster_start_at_or_before(at);
        }
        Some(self.position_at_addressable_byte(at))
    }

    /// The first position in the text.
    pub fn start(&self) -> Position {
        Position::new(0, 0, Column::ZERO, self.revision)
    }

    /// The position just past the last character.
    pub fn end(&self) -> Position {
        self.position_at_addressable_byte(self.rope.len_bytes())
    }

    /// The whole text as one span.
    pub fn full_span(&self) -> Span {
        Span::new(self.start(), self.end())
    }

    /// An ordered span between two positions of *this* text.
    ///
    /// Order does not matter: the span comes back with its ends the right way
    /// round. `None` when either position addresses another revision — which is
    /// also the only place two positions are checked against each other, and the
    /// reason [`Position`] is not `Ord`.
    pub fn span(&self, a: Position, b: Position) -> Option<Span> {
        if self.is_stale(a) || self.is_stale(b) {
            return None;
        }
        Some(if a.byte() <= b.byte() {
            Span::new(a, b)
        } else {
            Span::new(b, a)
        })
    }

    // -- internals ----------------------------------------------------------

    /// Replace `bytes` with `text`, becoming a new revision.
    ///
    /// `bytes` must be a byte range this text can address; the buffer only ever
    /// derives it from [`Span`]s, which are checked. Inserted text is normalised
    /// like any other, so a paste carrying `\r\n` cannot smuggle a carriage
    /// return past the invariant.
    /// Returns how many bytes were inserted, which is not `text.len()` when the
    /// normalisation collapsed a `\r\n`.
    ///
    /// The precondition is asserted rather than trusted, because this is the one
    /// place the text is mutated and the range reaches it as bare arithmetic —
    /// `Txn` remaps its spans across earlier edits in the same transaction. An
    /// inverted or out-of-range range would panic inside the rope anyway; a byte
    /// *inside a character* would not. `byte_to_char` rounds it down, and the
    /// edit would silently land somewhere the caller never asked for, in a note
    /// that autosaves. Corrupting the text is worse than refusing to.
    pub(crate) fn splice(&mut self, bytes: std::ops::Range<usize>, text: &str) -> usize {
        assert!(bytes.start <= bytes.end, "splice range {bytes:?} is inverted");
        assert!(
            bytes.end <= self.rope.len_bytes(),
            "splice range {bytes:?} runs past the text's {} bytes",
            self.rope.len_bytes()
        );
        let start = self.char_boundary(bytes.start);
        let end = self.char_boundary(bytes.end);
        if start != end {
            self.rope.remove(start..end);
        }
        let normalised = normalise_breaks(text);
        if !normalised.is_empty() {
            self.rope.insert(start, &normalised);
        }
        self.revision = Revision::fresh();
        normalised.len()
    }

    /// Char index at `byte`, which must start a character.
    ///
    /// `Rope::byte_to_char` answers for a byte inside one by naming the
    /// character that contains it, so the round trip back is what distinguishes
    /// a boundary from an interior byte.
    fn char_boundary(&self, byte: usize) -> usize {
        let chars = self.rope.byte_to_char(byte);
        assert_eq!(
            self.rope.char_to_byte(chars),
            byte,
            "splice byte {byte} is inside a character"
        );
        chars
    }

    /// The same content as a new revision.
    ///
    /// Undo restores content, not identity: a revision names a point in the edit
    /// timeline, and going back to earlier content is a *later* point. Keeping
    /// revisions monotonic is what lets a cache and a background task compare
    /// two of them without also asking which way history was walking.
    pub(crate) fn reidentified(&self) -> Self {
        Self {
            rope: self.rope.clone(),
            revision: Revision::fresh(),
        }
    }

    /// Row containing `byte`, which must be addressable.
    pub(crate) fn row_of_byte(&self, byte: usize) -> usize {
        self.rope.byte_to_line(byte)
    }

    /// Byte offset where `row` starts.
    pub(crate) fn row_start_byte(&self, row: usize) -> usize {
        self.rope
            .line_to_byte(row.min(self.line_count().saturating_sub(1)))
    }

    /// The position a cursor takes at `byte`, snapping **forward** if the byte
    /// falls inside a cluster.
    ///
    /// Distinct from [`Self::position_at_derived_byte`] because the direction
    /// matters and the two want opposite ones. An edit's own end offset is a
    /// char boundary, but char boundaries are not cluster boundaries: typing `a`
    /// in front of a lone combining acute produces one cluster, and the offset
    /// between them addresses nothing. Snapping back — which is what
    /// `position_at_byte_snapped` does, since it names the *enclosing* cluster —
    /// would leave the cursor in front of the character just typed, so the next
    /// keystroke would land in reverse order. Forward is the only direction that
    /// keeps typing moving the way the typist is.
    pub(crate) fn position_at_cursor_byte(&self, byte: usize) -> Position {
        match self.position_at_byte(byte) {
            Some(position) => position,
            None => {
                let forward = self.next_cluster_byte(byte.min(self.len_bytes()));
                self.position_at_byte(forward)
                    .unwrap_or_else(|| self.end())
            }
        }
    }

    /// The position at `byte`, snapping if the byte is somehow not addressable.
    ///
    /// For offsets this crate derived itself — a remapped cursor, a restored
    /// history entry. They should always land on a cluster boundary; the assert
    /// is what surfaces the case that does not, in development rather than in a
    /// user's note, and the snap is what keeps it from being a panic if it ever
    /// does.
    pub(crate) fn position_at_derived_byte(&self, byte: usize) -> Position {
        match self.position_at_byte(byte) {
            Some(position) => position,
            None => {
                debug_assert!(false, "derived byte {byte} is not addressable");
                self.position_at_byte_snapped(byte.min(self.len_bytes()))
                    .unwrap_or_else(|| self.start())
            }
        }
    }

    /// Row `row` with any trailing `\n` trimmed off.
    fn line_slice(&self, row: usize) -> Option<RopeSlice<'_>> {
        let line = self.rope.get_line(row)?;
        let chars = line.len_chars();
        if chars > 0 && line.char(chars - 1) == '\n' {
            Some(line.slice(..chars - 1))
        } else {
            Some(line)
        }
    }

    /// `byte` is known to be addressable; derive the rest of the position.
    fn position_at_addressable_byte(&self, byte: usize) -> Position {
        let row = self.rope.byte_to_line(byte);
        let line_start = self.rope.line_to_byte(row);
        let column = Column::new(self.rope.byte_slice(line_start..byte).len_chars());
        Position::new(byte, row, column, self.revision)
    }

    fn is_char_boundary(&self, byte: usize) -> bool {
        let len = self.rope.len_bytes();
        if byte == 0 || byte == len {
            return true;
        }
        if byte > len {
            return false;
        }
        let (chunk, chunk_start, _, _) = self.rope.chunk_at_byte(byte);
        chunk.is_char_boundary(byte - chunk_start)
    }

    /// Whether `byte` sits between two grapheme clusters.
    ///
    /// Answered from the chunk containing `byte`, with preceding context fetched
    /// only when the segmenter asks for it — so this is a chunk-local operation,
    /// not a walk from the start of the row. That is what makes it affordable on
    /// the paths that build a position per overlay per visible row.
    fn is_cluster_boundary(&self, byte: usize) -> bool {
        let len = self.rope.len_bytes();
        if byte == 0 || byte == len {
            return true;
        }
        if byte > len || !self.is_char_boundary(byte) {
            return false;
        }
        let mut cursor = GraphemeCursor::new(byte, len, true);
        let (chunk, chunk_start, _, _) = self.rope.chunk_at_byte(byte);
        // Each PreContext asks for strictly earlier text, so this terminates;
        // the bound is a backstop against a segmenter that disagrees.
        for _ in 0..MAX_CONTEXT_REQUESTS {
            match cursor.is_boundary(chunk, chunk_start) {
                Ok(is) => return is,
                Err(GraphemeIncomplete::PreContext(upto)) => {
                    if upto == 0 {
                        return true;
                    }
                    let (pre, pre_start, _, _) = self.rope.chunk_at_byte(upto - 1);
                    cursor.provide_context(pre, pre_start);
                }
                Err(_) => return false,
            }
        }
        debug_assert!(false, "grapheme cursor kept asking for context at {byte}");
        false
    }

    /// Byte offset of the next cluster boundary after `byte`, or the end of the
    /// text.
    pub(crate) fn next_cluster_byte(&self, byte: usize) -> usize {
        self.step_cluster(byte, true)
    }

    /// Byte offset of the previous cluster boundary before `byte`, or zero.
    pub(crate) fn prev_cluster_byte(&self, byte: usize) -> usize {
        self.step_cluster(byte, false)
    }

    /// The first scalar of the cluster starting at `byte`, or `None` at the end
    /// of the text.
    ///
    /// A cluster's class — blank, word, punctuation — is its first scalar's, which
    /// is what every editor's word motions use.
    pub(crate) fn scalar_at(&self, byte: usize) -> Option<char> {
        if byte >= self.rope.len_bytes() {
            return None;
        }
        Some(self.rope.char(self.rope.byte_to_char(byte)))
    }

    /// One cluster boundary in either direction.
    ///
    /// Runs the segmenter over a window around `byte` rather than the whole row.
    /// A cluster is a handful of bytes in practice, so the window almost always
    /// suffices; when the segmenter says it needs more, it says which side, and
    /// the window grows. Stepping a row's worth of text to move the cursor one
    /// place would make an arrow key cost the length of its line.
    fn step_cluster(&self, byte: usize, forward: bool) -> usize {
        let len = self.rope.len_bytes();
        let limit = if forward { len } else { 0 };
        if byte == limit {
            return limit;
        }
        let mut back = WINDOW_BYTES;
        let mut ahead = WINDOW_BYTES;
        loop {
            let low = self.char_boundary_at_or_before(byte.saturating_sub(back));
            let high = self.char_boundary_at_or_after((byte + ahead).min(len));
            let window = cow_of(self.rope.byte_slice(low..high));
            let mut cursor = GraphemeCursor::new(byte, len, true);
            let step = if forward {
                cursor.next_boundary(&window, low)
            } else {
                cursor.prev_boundary(&window, low)
            };
            match step {
                Ok(Some(at)) => return at,
                Ok(None) => return limit,
                Err(GraphemeIncomplete::NextChunk) => ahead *= 4,
                Err(GraphemeIncomplete::PreContext(_) | GraphemeIncomplete::PrevChunk) => back *= 4,
                Err(_) => return limit,
            }
            if low == 0 && high == len {
                // The window is the whole text and the segmenter still wants
                // more, which it cannot get. Refuse to move rather than guess.
                debug_assert!(false, "grapheme cursor wants context beyond the text");
                return byte;
            }
        }
    }

    fn char_boundary_at_or_before(&self, byte: usize) -> usize {
        let mut at = byte.min(self.rope.len_bytes());
        while !self.is_char_boundary(at) {
            at -= 1;
        }
        at
    }

    fn char_boundary_at_or_after(&self, byte: usize) -> usize {
        let mut at = byte.min(self.rope.len_bytes());
        while !self.is_char_boundary(at) {
            at += 1;
        }
        at
    }

    /// Start of the grapheme cluster containing `byte`, which must be a char
    /// boundary.
    ///
    /// Walks the row rather than the chunk. Unlike [`Self::is_cluster_boundary`]
    /// this runs at one call site — an offset arriving from outside — so O(row)
    /// is the right trade for an implementation that is obviously correct.
    fn cluster_start_at_or_before(&self, byte: usize) -> usize {
        let row = self.rope.byte_to_line(byte);
        let row_start = self.rope.line_to_byte(row);
        let line = cow_of(self.rope.line(row));
        let offset = byte - row_start;
        let mut start = 0;
        for (at, _) in line.grapheme_indices(true) {
            if at > offset {
                break;
            }
            start = at;
        }
        row_start + start
    }
}

/// Backstop on [`Text::is_cluster_boundary`]'s context loop.
const MAX_CONTEXT_REQUESTS: usize = 64;

/// Bytes either side of an offset handed to the segmenter to start with. Wide
/// enough for any cluster that occurs in prose; grown on demand for ones that do
/// not.
const WINDOW_BYTES: usize = 64;

fn cow_of(slice: RopeSlice<'_>) -> Cow<'_, str> {
    match slice.as_str() {
        Some(s) => Cow::Borrowed(s),
        None => Cow::Owned(slice.to_string()),
    }
}

/// `\r\n` and lone `\r` become `\n`. Borrows when there is nothing to do, which
/// is every note not written on Windows.
fn normalise_breaks(s: &str) -> Cow<'_, str> {
    if !s.contains('\r') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "e" plus a combining acute: two chars, one cluster.
    const COMBINING: &str = "e\u{301}f";
    /// Man-woman-girl family: three scalars joined by two ZWJs, one cluster.
    const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";

    fn col(n: usize) -> Column {
        Column::new(n)
    }

    // -- splice preconditions -----------------------------------------------

    #[test]
    #[should_panic(expected = "is inside a character")]
    fn splicing_inside_a_character_is_refused() {
        // Without the check the rope rounds this down to the start of the `é`
        // and deletes a character the caller never named.
        let mut t = Text::from("héllo");
        t.splice(2..3, "");
    }

    #[test]
    #[should_panic(expected = "is inside a character")]
    fn splicing_that_ends_inside_a_character_is_refused() {
        let mut t = Text::from("héllo");
        t.splice(1..2, "");
    }

    #[test]
    #[should_panic(expected = "is inverted")]
    fn splicing_an_inverted_range_is_refused() {
        let mut t = Text::from("hello");
        // Built from values: a literal `3..1` is a lint, and the point is that
        // arithmetic can produce one where a literal never would.
        let (start, end) = (3usize, 1usize);
        t.splice(start..end, "");
    }

    #[test]
    #[should_panic(expected = "runs past the text's")]
    fn splicing_past_the_end_is_refused() {
        let mut t = Text::from("hello");
        t.splice(4..9, "");
    }

    #[test]
    fn splicing_at_the_very_end_is_allowed() {
        // The boundary the check must not exclude: an append is `len..len`.
        let mut t = Text::from("hello");
        t.splice(5..5, "!");
        assert_eq!(t.line(0).expect("one row"), "hello!");
    }

    // -- shape --------------------------------------------------------------

    #[test]
    fn empty_text_has_one_empty_row() {
        let t = Text::new();
        assert_eq!(t.line_count(), 1);
        assert_eq!(t.line(0).as_deref(), Some(""));
        assert_eq!(t.len_bytes(), 0);
    }

    #[test]
    fn trailing_newline_opens_a_final_empty_row() {
        let t = Text::from("a\n");
        assert_eq!(t.line_count(), 2);
        assert_eq!(t.line(1).as_deref(), Some(""));
    }

    #[test]
    fn no_trailing_newline_is_distinguishable_from_one() {
        assert_eq!(Text::from("a").line_count(), 1);
        assert_eq!(Text::from("a\n").line_count(), 2);
        assert_eq!(Text::from("a").to_string(), "a");
        assert_eq!(Text::from("a\n").to_string(), "a\n");
    }

    #[test]
    fn lines_come_back_without_their_break() {
        let t = Text::from("one\ntwo\nthree");
        assert_eq!(
            t.lines().map(|l| l.to_string()).collect::<Vec<_>>(),
            ["one", "two", "three"]
        );
    }

    #[test]
    fn line_past_the_end_is_none() {
        let t = Text::from("one\ntwo");
        assert!(t.line(2).is_none());
        assert!(t.position(2, col(0)).is_none());
    }

    // -- line endings -------------------------------------------------------

    #[test]
    fn crlf_normalises_and_leaves_no_carriage_return() {
        let t = Text::from("a\r\nb\r\n");
        assert_eq!(t.to_string(), "a\nb\n");
        assert_eq!(t.line(0).as_deref(), Some("a"));
        assert!(!t.to_string().contains('\r'));
    }

    #[test]
    fn lone_carriage_return_is_a_line_break() {
        let t = Text::from("a\rb");
        assert_eq!(t.line_count(), 2);
        assert_eq!(t.line(1).as_deref(), Some("b"));
    }

    #[test]
    fn only_a_newline_breaks_a_row() {
        // Ropey's default line-break set includes these; ours does not, because
        // CommonMark's does not and neither does splitting on '\n'. A row here
        // must be a row to the markdown parser as well.
        for exotic in ["\u{b}", "\u{c}", "\u{85}", "\u{2028}", "\u{2029}"] {
            let t = Text::from(format!("a{exotic}b").as_str());
            assert_eq!(
                t.line_count(),
                1,
                "{exotic:?} must be an ordinary character, not a break"
            );
        }
    }

    // -- positions ----------------------------------------------------------

    #[test]
    fn end_of_row_is_addressable_but_past_it_is_not() {
        let t = Text::from("hello\nworld");
        assert!(t.position(0, col(5)).is_some());
        assert!(t.position(0, col(6)).is_none());
    }

    #[test]
    fn column_is_chars_and_byte_is_bytes() {
        let t = Text::from("w\u{f8}rld"); // "wørld": ø is two bytes
        let p = t.position(0, col(2)).expect("char 2 is addressable");
        assert_eq!(p.column().get(), 2);
        assert_eq!(p.byte(), 3);
    }

    #[test]
    fn row_and_column_survive_the_round_trip_through_byte() {
        let t = Text::from("one\ntw\u{f8}\nthree");
        let p = t.position(1, col(3)).expect("end of row 1");
        let q = t.position_at_byte(p.byte()).expect("same place by byte");
        assert_eq!((q.row(), q.column().get()), (1, 3));
    }

    #[test]
    fn a_position_inside_a_character_is_refused() {
        let t = Text::from("w\u{f8}rld");
        assert!(t.position_at_byte(2).is_none(), "byte 2 splits ø");
    }

    #[test]
    fn a_position_inside_a_cluster_is_refused() {
        let t = Text::from(COMBINING);
        // char 1 is the combining acute — a place the renderer cannot show a
        // cursor, so the text refuses to name it.
        assert!(t.position(0, col(1)).is_none());
        assert!(t.position(0, col(0)).is_some());
        assert!(t.position(0, col(2)).is_some());
    }

    #[test]
    fn a_position_inside_a_zwj_sequence_is_refused() {
        let t = Text::from(FAMILY);
        assert!(t.position(0, col(0)).is_some());
        for interior in 1..5 {
            assert!(
                t.position(0, col(interior)).is_none(),
                "char {interior} is inside the family cluster"
            );
        }
        assert!(t.position(0, col(5)).is_some(), "past the whole cluster");
    }

    #[test]
    fn start_and_end_address_the_whole_text() {
        let t = Text::from("one\ntwo");
        assert_eq!(t.start().byte(), 0);
        assert_eq!(t.end().byte(), 7);
        assert_eq!((t.end().row(), t.end().column().get()), (1, 3));
    }

    #[test]
    fn end_of_a_text_ending_in_a_newline_is_the_empty_row() {
        let t = Text::from("a\n");
        assert_eq!((t.end().row(), t.end().column().get()), (1, 0));
    }

    // -- snapping -----------------------------------------------------------

    #[test]
    fn snapping_lands_on_the_start_of_the_cluster() {
        let t = Text::from(COMBINING);
        let acute_start = 1; // byte offset of the combining mark
        let p = t
            .position_at_byte_snapped(acute_start)
            .expect("inside the text");
        assert_eq!(p.byte(), 0, "snapped back to the start of the cluster");
    }

    #[test]
    fn snapping_a_valid_position_changes_nothing() {
        let t = Text::from("hello");
        let p = t.position_at_byte_snapped(3).expect("inside the text");
        assert_eq!(p.byte(), 3);
    }

    #[test]
    fn snapping_inside_a_character_lands_on_the_character() {
        let t = Text::from("w\u{f8}rld");
        let p = t.position_at_byte_snapped(2).expect("inside the text");
        assert_eq!(p.byte(), 1);
    }

    #[test]
    fn snapping_past_the_end_is_still_refused() {
        let t = Text::from("hello");
        assert!(t.position_at_byte_snapped(6).is_none());
    }

    // -- revisions ----------------------------------------------------------

    #[test]
    fn two_texts_never_share_a_revision() {
        let a = Text::from("same");
        let b = Text::from("same");
        assert_ne!(a.revision(), b.revision());
    }

    #[test]
    fn a_clone_is_the_same_text_and_keeps_its_revision() {
        let a = Text::from("shared");
        let b = a.clone();
        assert_eq!(a.revision(), b.revision());
        assert!(!b.is_stale(a.start()));
    }

    #[test]
    fn a_position_from_another_text_is_stale() {
        let a = Text::from("hello");
        let b = Text::from("hello");
        let p = a.position(0, col(2)).expect("addressable in a");
        assert!(b.is_stale(p));
        assert!(b.span(p, b.start()).is_none());
        assert!(b.slice(a.full_span()).is_none());
    }

    // -- spans --------------------------------------------------------------

    #[test]
    fn a_span_comes_back_ordered() {
        let t = Text::from("hello");
        let a = t.position(0, col(1)).unwrap();
        let b = t.position(0, col(4)).unwrap();
        let forward = t.span(a, b).unwrap();
        let backward = t.span(b, a).unwrap();
        assert_eq!(forward, backward);
        assert_eq!(forward.byte_range(), 1..4);
    }

    #[test]
    fn slicing_a_span_reads_the_text_between_its_ends() {
        let t = Text::from("one\ntwo\nthree");
        let a = t.position(0, col(1)).unwrap();
        let b = t.position(2, col(2)).unwrap();
        let span = t.span(a, b).unwrap();
        assert_eq!(t.slice(span).as_deref(), Some("ne\ntwo\nth"));
    }

    #[test]
    fn an_empty_span_says_so() {
        let t = Text::from("hello");
        let p = t.position(0, col(2)).unwrap();
        assert!(t.span(p, p).unwrap().is_empty());
    }

    #[test]
    fn the_full_span_is_the_whole_text() {
        let t = Text::from("one\ntwo");
        assert_eq!(t.slice(t.full_span()).as_deref(), Some("one\ntwo"));
    }

    // -- properties ---------------------------------------------------------

    mod properties {
        use super::*;
        use proptest::prelude::*;

        /// Naive reference: every cluster boundary in the text, by byte offset.
        fn cluster_boundaries(s: &str) -> Vec<usize> {
            let mut out: Vec<usize> = s.grapheme_indices(true).map(|(i, _)| i).collect();
            out.push(s.len());
            out
        }

        proptest! {
            /// A byte offset is addressable exactly when it is a cluster
            /// boundary. No clamping, no rounding, in either direction.
            #[test]
            fn addressable_bytes_are_exactly_the_cluster_boundaries(s in ".{0,200}") {
                let normalised = normalise_breaks(&s).into_owned();
                let t = Text::from(normalised.as_str());
                let expected = cluster_boundaries(&normalised);
                for byte in 0..=normalised.len() {
                    let got = t.position_at_byte(byte).is_some();
                    prop_assert_eq!(
                        got,
                        expected.contains(&byte),
                        "byte {} of {:?}", byte, normalised
                    );
                }
            }

            /// Snapping always lands on a cluster boundary at or before where it
            /// was asked, and never moves a boundary that was already fine.
            #[test]
            fn snapping_lands_on_a_boundary_at_or_before(s in ".{0,200}") {
                let normalised = normalise_breaks(&s).into_owned();
                let t = Text::from(normalised.as_str());
                let boundaries = cluster_boundaries(&normalised);
                for byte in 0..=normalised.len() {
                    let p = t.position_at_byte_snapped(byte).expect("inside the text");
                    prop_assert!(p.byte() <= byte);
                    prop_assert!(boundaries.contains(&p.byte()));
                    if boundaries.contains(&byte) {
                        prop_assert_eq!(p.byte(), byte);
                    }
                }
            }

            /// (row, column) and byte offset name the same places.
            #[test]
            fn row_column_and_byte_agree(s in ".{0,200}") {
                let normalised = normalise_breaks(&s).into_owned();
                let t = Text::from(normalised.as_str());
                for byte in cluster_boundaries(&normalised) {
                    let by_byte = t.position_at_byte(byte).expect("a boundary is addressable");
                    let by_col = t
                        .position(by_byte.row(), by_byte.column())
                        .expect("its own row and column are addressable");
                    prop_assert_eq!(by_col, by_byte);
                }
            }

            /// Rows rejoin into the text, so nothing is lost or invented by the
            /// break-trimming.
            #[test]
            fn rows_rejoin_into_the_text(s in ".{0,200}") {
                let normalised = normalise_breaks(&s).into_owned();
                let t = Text::from(normalised.as_str());
                let rejoined = t.lines().collect::<Vec<_>>().join("\n");
                prop_assert_eq!(rejoined, normalised);
            }
        }
    }
}
