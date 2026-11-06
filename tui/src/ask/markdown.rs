//! An answer's markdown, toolkit-free: the one set of rules both UIs draw
//! an Ask answer by (the TUI's `components::markdown_lines`, the GUI's
//! Thread). Each UI only maps the result onto its own styles.
//!
//! **Blocks are the editor's.** The whole answer goes through the editor's
//! buffer-aware markdown model ([`ParsedBuffer`], pulldown-cmark) in one
//! pass, so a line is what the note editor says it is: fenced and indented
//! code, ATX and setext headings (both rows of a setext heading), and
//! quotes, lazy continuations included. Code wins over the heading and quote
//! signals (a `#` or `>` inside a fence is literal).
//!
//! **Inline rules are the editor's too**, and apply to ordinary lines
//! (paragraphs and list items) only; code, heading and quote lines are one
//! verbatim span. On an ordinary line:
//!
//! - bold (`**`/`__`), italic (`*`/`_`) and strikethrough (`~~`) are
//!   CommonMark's (flanking, intraword `_`, nesting, backslash escapes), and
//!   their sigils are *hidden*: the spans that hold them say so, and a UI
//!   leaves them out of the text it shows. Nothing else is hidden: list
//!   markers, link brackets, escape backslashes outside emphasis and inline
//!   code's backticks stay;
//! - inline code is verbatim, backticks included, and nothing inside it is
//!   emphasis or a citation;
//! - every `[n]` marker [`citations::scan`] finds (the one home of that
//!   rule) is a citation, emphasised or not, and its span names `n`.
//!
//! The answer is split on `\n` (a final newline ends in one empty line);
//! byte ranges are into the answer for a line and into the line for a span.

use std::ops::Range;

use crate::ask::citations;
use crate::logic::editor::markdown::{
    ElementKind, MOD_BOLD, MOD_ITALIC, MOD_STRIKE, ParsedBuffer, ParsedLine,
};
use crate::logic::editor::parse_incremental::LineConstructKind;

/// What one line of an answer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// Paragraph text or a list item: inline styling and citations apply.
    Normal,
    /// A fence delimiter, a line inside a fence, or indented code.
    Code,
    /// An ATX heading, or either row of a setext heading.
    Heading,
    /// A blockquote line, lazy continuations included.
    Quote,
}

/// The inline styles over a span of an ordinary line. They combine: a
/// citation inside bold is both; inline code is only ever code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub citation: bool,
}

impl SpanStyle {
    /// No style at all.
    pub fn is_plain(self) -> bool {
        self == Self::default()
    }
}

/// A run of one line that shares its style: `range` in the line's bytes;
/// `hidden` for an emphasis sigil a UI leaves out; `citation` the `n` of
/// the `[n]` marker it is part of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerSpan {
    pub range: Range<usize>,
    pub style: SpanStyle,
    pub hidden: bool,
    pub citation: Option<usize>,
}

/// One line of an answer: where it is in the answer, what it is, and its
/// spans, in order and covering the line exactly (none for an empty line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerLine {
    pub range: Range<usize>,
    pub kind: BlockKind,
    pub spans: Vec<AnswerSpan>,
}

impl AnswerLine {
    /// The line's source text in `answer` (the answer it was parsed from).
    pub fn text<'a>(&self, answer: &'a str) -> &'a str {
        &answer[self.range.clone()]
    }

    /// The spans a UI shows: every one but the hidden sigils.
    pub fn shown_spans(&self) -> impl Iterator<Item = &AnswerSpan> {
        self.spans.iter().filter(|span| !span.hidden)
    }
}

/// A line as a UI that draws plain text shows it: [`AnswerLine::shown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownLine {
    /// The shown spans' text, end to end (the hidden sigils left out).
    pub text: String,
    pub kind: BlockKind,
    /// The styled runs, byte ranges into `text`, in order, not overlapping:
    /// adjacent spans of one style are one run; plain text has none.
    pub runs: Vec<(Range<usize>, SpanStyle)>,
    /// Each `[n]` citation's range in `text` with its `n`; adjacent markers
    /// of the same `n` are one.
    pub citations: Vec<(Range<usize>, usize)>,
}

impl AnswerLine {
    /// The line as shown: its spans laid end to end with the hidden sigils
    /// left out, their runs and citations merged. The one home of that
    /// merge for a UI that wraps and styles one string per line.
    pub fn shown(&self, answer: &str) -> ShownLine {
        let source = self.text(answer);
        let mut text = String::with_capacity(source.len());
        let mut runs: Vec<(Range<usize>, SpanStyle)> = Vec::new();
        let mut citations: Vec<(Range<usize>, usize)> = Vec::new();
        for span in self.shown_spans() {
            let start = text.len();
            text.push_str(&source[span.range.clone()]);
            let end = text.len();
            if let Some(n) = span.citation {
                match citations.last_mut() {
                    Some((range, last)) if *last == n && range.end == start => range.end = end,
                    _ => citations.push((start..end, n)),
                }
            }
            if span.style.is_plain() {
                continue;
            }
            match runs.last_mut() {
                Some((range, last)) if *last == span.style && range.end == start => {
                    range.end = end;
                }
                _ => runs.push((start..end, span.style)),
            }
        }
        ShownLine {
            text,
            kind: self.kind,
            runs,
            citations,
        }
    }
}

/// `answer`'s lines, by the rules in the module docs.
pub fn parse_answer(answer: &str) -> Vec<AnswerLine> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for line in answer.split('\n') {
        ranges.push(start..start + line.len());
        start += line.len() + 1;
    }
    let logicals: Vec<String> = ranges
        .iter()
        .map(|r| answer[r.clone()].to_string())
        .collect();
    let parsed = ParsedBuffer::parse_lines(&logicals);
    ranges
        .into_iter()
        .zip(parsed.lines.iter().zip(parsed.kinds.iter()))
        .map(|(range, (parsed_line, &construct))| {
            let text = &answer[range.clone()];
            let kind = block_kind(parsed_line, construct);
            let spans = match kind {
                _ if text.is_empty() => Vec::new(),
                BlockKind::Normal => inline_spans(text, parsed_line),
                _ => vec![AnswerSpan {
                    range: 0..text.len(),
                    style: SpanStyle::default(),
                    hidden: false,
                    citation: None,
                }],
            };
            AnswerLine { range, kind, spans }
        })
        .collect()
}

/// A row's block kind, from the editor model's coarse kind and elements.
fn block_kind(line: &ParsedLine, construct: LineConstructKind) -> BlockKind {
    if matches!(
        construct,
        LineConstructKind::FenceMarker
            | LineConstructKind::FenceContent
            | LineConstructKind::IndentedCode
    ) {
        return BlockKind::Code;
    }
    // A setext title row's coarse kind is reset to plain; its heading
    // element still spans it.
    let heading = construct == LineConstructKind::SetextUnderline
        || line.elements.iter().any(|e| {
            matches!(
                e.kind,
                ElementKind::HeadingH1 | ElementKind::HeadingH2 | ElementKind::HeadingH3
            )
        });
    if heading {
        BlockKind::Heading
    } else if line.blockquote_depth().is_some() {
        BlockKind::Quote
    } else {
        BlockKind::Normal
    }
}

/// What one char of an ordinary line is.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CharStyle {
    style: SpanStyle,
    hidden: bool,
    /// The index of its citation in the line's scan, to keep two markers
    /// side by side apart.
    citation: Option<usize>,
}

/// An ordinary line's spans: one pass over its chars, every per-char fact
/// an O(1) read of the parsed line (its masks, its innermost element).
fn inline_spans(text: &str, line: &ParsedLine) -> Vec<AnswerSpan> {
    let chars = text.chars().count();
    let mut code = vec![false; chars];
    for element in line
        .elements
        .iter()
        .filter(|e| e.kind == ElementKind::InlineCode)
    {
        for slot in &mut code[element.start_char.min(chars)..element.end_char.min(chars)] {
            *slot = true;
        }
    }
    let cites = citations::scan(text);
    let mut spans: Vec<AnswerSpan> = Vec::new();
    let mut last: Option<CharStyle> = None;
    let mut cite = 0;
    for (ci, (byte, ch)) in text.char_indices().enumerate() {
        let in_code = code[ci];
        // A sigil is hidden when the editor hides it and it belongs to an
        // emphasis (the innermost element over it): a link's brackets or
        // a code span's backticks inside bold stay.
        let hidden = !line.content_vis.get(ci).copied().unwrap_or(true)
            && line.elem_at(ci).is_some_and(|ix| {
                matches!(
                    line.elements[ix].kind,
                    ElementKind::Bold | ElementKind::Italic | ElementKind::Strikethrough
                )
            });
        while cites.get(cite).is_some_and(|c| c.range.end <= byte) {
            cite += 1;
        }
        let citation = cites
            .get(cite)
            .filter(|c| !in_code && c.range.contains(&byte))
            .map(|_| cite);
        let mods = if in_code { 0 } else { line.modifiers_at(ci) };
        let now = CharStyle {
            style: SpanStyle {
                bold: mods & MOD_BOLD != 0,
                italic: mods & MOD_ITALIC != 0,
                strike: mods & MOD_STRIKE != 0,
                code: in_code,
                citation: citation.is_some(),
            },
            hidden,
            citation,
        };
        let end = byte + ch.len_utf8();
        match spans.last_mut() {
            Some(span) if last == Some(now) => span.range.end = end,
            _ => spans.push(AnswerSpan {
                range: byte..end,
                style: now.style,
                hidden,
                citation: citation.map(|at| cites[at].index),
            }),
        }
        last = Some(now);
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_lays_spans_end_to_end_and_merges_runs() {
        let answer = "A **bold** and [1][1] see [2].";
        let shown = parse_answer(answer)[0].shown(answer);
        assert_eq!(shown.text, "A bold and [1][1] see [2].");
        let bold = shown.text.find("bold").unwrap();
        assert_eq!(shown.runs[0].0, bold..bold + 4);
        assert!(shown.runs[0].1.bold);
        let first = shown.text.find("[1]").unwrap();
        // Two adjacent markers of one `n` are one citation; `[2]` is its own.
        assert_eq!(shown.citations[0], (first..first + 6, 1));
        assert_eq!(shown.citations.len(), 2);
    }

    /// The kinds of `answer`'s lines.
    fn kinds(answer: &str) -> Vec<BlockKind> {
        parse_answer(answer).iter().map(|l| l.kind).collect()
    }

    /// One line as `(shown text, [(styled text, style)], [(cited text, n)])`.
    /// Shown runs with their style, and citations with their `n`.
    type Styled = Vec<(String, SpanStyle)>;
    type Cited = Vec<(String, usize)>;

    fn shown(line: &str) -> (String, Styled, Cited) {
        let lines = parse_answer(line);
        let l = &lines[0];
        let text = l.text(line);
        let shown: String = l.shown_spans().map(|s| &text[s.range.clone()]).collect();
        // Shown runs of one style merge, as a UI draws them (a hidden escape
        // between two shown halves splits the spans, not the run).
        let mut styled: Vec<(String, SpanStyle)> = Vec::new();
        let mut prev: Option<(SpanStyle, Option<usize>)> = None;
        for s in l.shown_spans() {
            let key = (s.style, s.citation);
            match styled.last_mut() {
                Some((t, _)) if prev == Some(key) => t.push_str(&text[s.range.clone()]),
                _ => styled.push((text[s.range.clone()].to_string(), s.style)),
            }
            prev = Some(key);
        }
        styled.retain(|(_, style)| !style.is_plain());
        let cited = l
            .shown_spans()
            .filter_map(|s| Some((text[s.range.clone()].to_string(), s.citation?)))
            .collect();
        (shown, styled, cited)
    }

    const B: SpanStyle = SpanStyle {
        bold: true,
        italic: false,
        strike: false,
        code: false,
        citation: false,
    };
    const I: SpanStyle = SpanStyle {
        bold: false,
        italic: true,
        ..B
    };
    const S: SpanStyle = SpanStyle {
        bold: false,
        strike: true,
        ..B
    };
    const C: SpanStyle = SpanStyle {
        bold: false,
        code: true,
        ..B
    };
    const CITE: SpanStyle = SpanStyle {
        bold: false,
        citation: true,
        ..B
    };

    #[test]
    fn each_block_kind_is_the_editors() {
        use BlockKind::*;
        let cases: &[(&str, &[BlockKind])] = &[
            ("plain text", &[Normal]),
            ("- a list item", &[Normal]),
            ("# Title", &[Heading]),
            ("###### h6", &[Heading]),
            ("####### too many", &[Normal]),
            ("#nospace", &[Normal]),
            ("Title\n====", &[Heading, Heading]),
            ("Title\n----", &[Heading, Heading]),
            ("> quoted", &[Quote]),
            ("> a\nb", &[Quote, Quote]),
            ("> a\n\nb", &[Quote, Normal, Normal]),
            (
                "```rust\nlet x = 1;\n```\nafter",
                &[Code, Code, Code, Normal],
            ),
            ("```\n# not a heading\n> nor a quote", &[Code, Code, Code]),
            ("    let x = 1;", &[Code]),
            ("a\n", &[Normal, Normal]),
            ("", &[Normal]),
        ];
        for (answer, want) in cases {
            assert_eq!(kinds(answer), *want, "{answer:?}");
        }
    }

    #[test]
    fn lines_cover_the_answer_and_an_empty_line_has_no_spans() {
        let answer = "one\n\n**two** [1]";
        let lines = parse_answer(answer);
        let texts: Vec<&str> = lines.iter().map(|l| l.text(answer)).collect();
        assert_eq!(texts, ["one", "", "**two** [1]"]);
        assert!(lines[1].spans.is_empty());
        for line in &lines {
            let text = line.text(answer);
            let mut at = 0;
            for span in &line.spans {
                assert_eq!(span.range.start, at, "spans are contiguous");
                at = span.range.end;
            }
            assert_eq!(at, text.len(), "spans cover {text:?}");
        }
    }

    #[test]
    fn non_normal_lines_are_one_verbatim_span() {
        for (answer, line) in [
            ("# Head [3] **b**", 0),
            ("> quoted *i*", 0),
            ("```\nlet **a** = arr[1];", 1),
        ] {
            let lines = parse_answer(answer);
            let l = &lines[line];
            assert_eq!(l.spans.len(), 1, "{answer:?}");
            let span = &l.spans[0];
            assert_eq!(span.range, 0..l.text(answer).len());
            assert!(span.style.is_plain() && !span.hidden && span.citation.is_none());
        }
    }

    #[test]
    fn each_inline_rule() {
        type Case = (
            &'static str,
            &'static str,
            &'static [(&'static str, SpanStyle)],
        );
        let cases: &[Case] = &[
            ("a **b** c", "a b c", &[("b", B)]),
            ("a __b__ c", "a b c", &[("b", B)]),
            ("an *em* word", "an em word", &[("em", I)]),
            ("an _em_ word", "an em word", &[("em", I)]),
            ("~~gone~~ here", "gone here", &[("gone", S)]),
            ("call `*x*` now", "call `*x*` now", &[("`*x*`", C)]),
            ("2 * 3 = 6", "2 * 3 = 6", &[]),
            ("width * height * depth", "width * height * depth", &[]),
            ("match *.rs and *.md", "match *.rs and *.md", &[]),
            ("foo_bar_baz", "foo_bar_baz", &[]),
            ("some__thing__glued", "some__thing__glued", &[]),
            (
                "snake_case and _real_",
                "snake_case and real",
                &[("real", I)],
            ),
            ("**unclosed bold", "**unclosed bold", &[]),
            ("an `unclosed span", "an `unclosed span", &[]),
            (r"\*not\* emphasis", r"\*not\* emphasis", &[]),
            (r"**a\*b**", "a*b", &[("a*b", B)]),
            ("**[a](u)** link", "[a](u) link", &[("[a](u)", B)]),
            ("- **x** item", "- x item", &[("x", B)]),
        ];
        for (line, want_shown, want_styled) in cases {
            let (shown, styled, _) = shown(line);
            assert_eq!(shown, *want_shown, "{line:?}");
            let want: Vec<(String, SpanStyle)> = want_styled
                .iter()
                .map(|(t, s)| (t.to_string(), *s))
                .collect();
            assert_eq!(styled, want, "{line:?}");
        }
    }

    #[test]
    fn nested_emphasis_combines() {
        let both = SpanStyle { italic: true, ..B };
        let (text, styled, _) = shown("***both*** and **b *bi* b**");
        assert_eq!(text, "both and b bi b");
        assert_eq!(
            styled,
            [
                ("both".to_string(), both),
                ("b ".to_string(), B),
                ("bi".to_string(), both),
                (" b".to_string(), B),
            ]
        );
    }

    #[test]
    fn citations_are_marked_with_their_number() {
        type Case = (
            &'static str,
            &'static [(&'static str, usize)],
            &'static [(&'static str, SpanStyle)],
        );
        let cases: &[Case] = &[
            (
                "See [1] and [12].",
                &[("[1]", 1), ("[12]", 12)],
                &[("[1]", CITE), ("[12]", CITE)],
            ),
            (
                "[1][2]",
                &[("[1]", 1), ("[2]", 2)],
                &[("[1]", CITE), ("[2]", CITE)],
            ),
            (
                "**a [3] b**",
                &[("[3]", 3)],
                &[
                    ("a ", B),
                    (
                        "[3]",
                        SpanStyle {
                            citation: true,
                            ..B
                        },
                    ),
                    (" b", B),
                ],
            ),
            (
                "`arr[1]` and [2]",
                &[("[2]", 2)],
                &[("`arr[1]`", C), ("[2]", CITE)],
            ),
            ("[[1]] is a wikilink", &[], &[]),
        ];
        for (line, want_cited, want_styled) in cases {
            let (_, styled, cited) = shown(line);
            let cited_want: Vec<(String, usize)> = want_cited
                .iter()
                .map(|(t, n)| (t.to_string(), *n))
                .collect();
            assert_eq!(cited, cited_want, "{line:?}");
            let styled_want: Vec<(String, SpanStyle)> = want_styled
                .iter()
                .map(|(t, s)| (t.to_string(), *s))
                .collect();
            assert_eq!(styled, styled_want, "{line:?}");
        }
    }

    #[test]
    fn hidden_spans_are_exactly_the_emphasis_sigils() {
        let answer = "**b** _i_ ~~s~~ `c` [1]";
        let lines = parse_answer(answer);
        let hidden: Vec<&str> = lines[0]
            .spans
            .iter()
            .filter(|s| s.hidden)
            .map(|s| &answer[s.range.clone()])
            .collect();
        assert_eq!(hidden, ["**", "**", "_", "_", "~~", "~~"]);
    }

    #[test]
    fn multibyte_text_keeps_byte_ranges() {
        let (text, styled, cited) = shown("é **ü** [1] ✓");
        assert_eq!(text, "é ü [1] ✓");
        assert_eq!(styled, [("ü".to_string(), B), ("[1]".to_string(), CITE)]);
        assert_eq!(cited, [("[1]".to_string(), 1)]);
    }
}
