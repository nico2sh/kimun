//! Server-sent-events parsing for `POST /api/answer/stream`: a pure,
//! incremental parser over raw bytes, and the mapping from a parsed event to
//! an [`AnswerEvent`]. No I/O here, so every framing case is unit-tested.

use super::dto::{AnswerEvent, AnswerResult};

/// One dispatched SSE event: its `event:` name (empty when absent) and its
/// `data:` lines joined with `\n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Incremental SSE parser. Feed it body chunks as they arrive with
/// [`SseParser::push`]; call [`SseParser::finish`] at end of stream to flush
/// a last event that had no trailing blank line.
///
/// It buffers bytes, not text, so a UTF-8 character split across two chunks is
/// reassembled before decoding. Lines end with `\n`; a `\r` before it is
/// dropped (`\r\n`). A lone `\r` is not a line end (the server never sends one).
#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: String,
    data: Vec<String>,
    has_fields: bool,
}

impl SseParser {
    /// Adds a chunk and returns the events it completed, in order.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            self.line(&line[..line.len() - 1], &mut out);
        }
        out
    }

    /// Ends the stream: a final unterminated line and a pending event without
    /// its blank line are still delivered.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let line = std::mem::take(&mut self.buf);
            self.line(&line, &mut out);
        }
        self.dispatch(&mut out);
        out
    }

    fn line(&mut self, raw: &[u8], out: &mut Vec<SseEvent>) {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.is_empty() {
            self.dispatch(out);
            return;
        }
        let line = String::from_utf8_lossy(raw);
        if line.starts_with(':') {
            return; // comment / keep-alive
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line.as_ref(), ""),
        };
        match field {
            "event" => {
                self.event = value.to_string();
                self.has_fields = true;
            }
            "data" => {
                self.data.push(value.to_string());
                self.has_fields = true;
            }
            _ => {} // id, retry, unknown: not used by this API
        }
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        if self.has_fields {
            out.push(SseEvent {
                event: std::mem::take(&mut self.event),
                data: self.data.join("\n"),
            });
        }
        self.event.clear();
        self.data.clear();
        self.has_fields = false;
    }
}

/// Maps a parsed event to an [`AnswerEvent`]. An unknown event name is `None`
/// (forward compatibility); a known one with a malformed payload becomes an
/// [`AnswerEvent::Error`] rather than being dropped silently.
pub fn to_answer_event(event: &SseEvent) -> Option<AnswerEvent> {
    #[derive(serde::Deserialize)]
    struct Step {
        text: String,
    }
    #[derive(serde::Deserialize)]
    struct Failure {
        error: String,
    }
    let bad = |what: &str, e: serde_json::Error| {
        Some(AnswerEvent::Error(format!("bad {what} event: {e}")))
    };
    match event.event.as_str() {
        "step" => match serde_json::from_str::<Step>(&event.data) {
            Ok(s) => Some(AnswerEvent::Step(s.text)),
            Err(e) => bad("step", e),
        },
        "answer" => match serde_json::from_str::<AnswerResult>(&event.data) {
            Ok(a) => Some(AnswerEvent::Answer(a)),
            Err(e) => bad("answer", e),
        },
        "error" => match serde_json::from_str::<Failure>(&event.data) {
            Ok(f) => Some(AnswerEvent::Error(f.error)),
            Err(e) => bad("error", e),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(event: &str, data: &str) -> SseEvent {
        SseEvent {
            event: event.into(),
            data: data.into(),
        }
    }

    fn parse_all(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut p = SseParser::default();
        let mut out = Vec::new();
        for c in chunks {
            out.extend(p.push(c));
        }
        out.extend(p.finish());
        out
    }

    #[test]
    fn parses_two_events() {
        let got =
            parse_all(&[b"event: step\ndata: {\"text\":\"a\"}\n\nevent: answer\ndata: {}\n\n"]);
        assert_eq!(
            got,
            vec![ev("step", "{\"text\":\"a\"}"), ev("answer", "{}")]
        );
    }

    #[test]
    fn events_split_across_chunk_boundaries() {
        let whole = b"event: step\ndata: {\"text\":\"caf\xc3\xa9\"}\n\nevent: answer\ndata: {}\n\n";
        for cut in 1..whole.len() {
            let got = parse_all(&[&whole[..cut], &whole[cut..]]);
            assert_eq!(
                got,
                vec![ev("step", "{\"text\":\"caf\u{e9}\"}"), ev("answer", "{}")],
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn crlf_line_endings() {
        let got = parse_all(&[b"event: step\r\ndata: x\r\n\r\nevent: error\r\ndata: y\r\n\r\n"]);
        assert_eq!(got, vec![ev("step", "x"), ev("error", "y")]);
    }

    #[test]
    fn multiple_data_lines_join_with_newline() {
        let got = parse_all(&[b"event: step\ndata: one\ndata: two\ndata:\ndata: four\n\n"]);
        assert_eq!(got, vec![ev("step", "one\ntwo\n\nfour")]);
    }

    #[test]
    fn comments_are_ignored() {
        let got = parse_all(&[b": keep-alive\n\n: hi\nevent: step\n: mid\ndata: x\n\n"]);
        assert_eq!(got, vec![ev("step", "x")]);
    }

    #[test]
    fn a_keep_alive_between_events_is_ignored() {
        // Axum's default keep-alive is a bare `:` comment line.
        let got = parse_all(&[
            b"event: step\ndata: a\n\n",
            b":\n\n",
            b": keep-alive\n\n",
            b"event: answer\ndata: {}\n\n",
        ]);
        assert_eq!(got, vec![ev("step", "a"), ev("answer", "{}")]);
    }

    #[test]
    fn a_final_event_without_a_blank_line_is_flushed_at_the_end() {
        let got = parse_all(&[b"event: answer\ndata: {}"]);
        assert_eq!(got, vec![ev("answer", "{}")]);
        let got = parse_all(&[b"event: step\ndata: a\n\nevent: answer\ndata: b\n"]);
        assert_eq!(got, vec![ev("step", "a"), ev("answer", "b")]);
    }

    #[test]
    fn nothing_pending_yields_nothing_at_the_end() {
        assert!(parse_all(&[b"event: step\ndata: a\n\n"]).len() == 1);
        assert!(parse_all(&[b"\n\n: c\n"]).is_empty());
    }

    #[test]
    fn maps_events_to_answer_events() {
        assert!(matches!(
            to_answer_event(&ev("step", r#"{"text":"Searching"}"#)),
            Some(AnswerEvent::Step(t)) if t == "Searching"
        ));
        match to_answer_event(&ev(
            "answer",
            r#"{"answer":"a","sources":[],"trail":["Searching"]}"#,
        )) {
            Some(AnswerEvent::Answer(a)) => {
                assert_eq!(a.answer, "a");
                assert_eq!(a.trail, vec!["Searching".to_string()]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            to_answer_event(&ev("error", r#"{"error":"boom"}"#)),
            Some(AnswerEvent::Error(t)) if t == "boom"
        ));
        assert!(to_answer_event(&ev("ping", "{}")).is_none());
        assert!(matches!(
            to_answer_event(&ev("answer", "not json")),
            Some(AnswerEvent::Error(_))
        ));
    }
}
