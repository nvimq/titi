//! The single SSE engine: turns any byte stream into [`SseFrame`]s.
//!
//! Family decoders (`openai`/`anthropic`/`gemini`) sit on top; malformed
//! `data:` payloads surface as [`SseFrame::Malformed`] and never panic, and
//! subsequent frames of the same connection are unaffected.

use smol_str::SmolStr;

/// One decoded server-sent event.
#[derive(Debug, Clone, PartialEq)]
pub enum SseFrame {
    /// `event:` + `data:` payload (event name empty when absent).
    Data { event: SmolStr, data: SmolStr },
    /// `data:` payload that failed decoding expectations — never fatal to
    /// the connection.
    Malformed { reason: SmolStr },
}

/// Incremental SSE decoder over arbitrary text chunks.
#[derive(Debug, Default)]
pub struct SseDecoder {
    event_name: Option<String>,
    data_lines: Vec<String>,
    saw_any_field: bool,
    partial: String,
}

/// Decode one SSE text block (already line-split by the caller) into a frame.
pub fn parse_frame(event_name: Option<&str>, data_lines: &[String]) -> SseFrame {
    if data_lines.is_empty() {
        return SseFrame::Malformed {
            reason: "empty data payload".into(),
        };
    }
    let data = data_lines.join("\n");
    SseFrame::Data {
        event: event_name.unwrap_or_default().into(),
        data: data.into(),
    }
}

/// Strip chat-template markers that leak into visible text (DeepSeek-class);
/// tolerant of a marker being split across chunks via a pending buffer.
#[derive(Debug)]
pub struct MarkerStripper {
    marker: String,
    pending: String,
}

impl MarkerStripper {
    pub fn new(marker: &str) -> Self {
        Self {
            marker: marker.to_owned(),
            pending: String::new(),
        }
    }

    /// Feed text, return visible text (possibly empty).
    pub fn feed(&mut self, text: &str) -> String {
        self.pending.push_str(text);
        let mut out = String::new();
        loop {
            match self.pending.find(&self.marker) {
                Some(idx) => {
                    if idx > 0 {
                        out.push_str(&self.pending[..idx]);
                    }
                    self.pending.drain(..idx + self.marker.len());
                }
                None => {
                    // Keep a suffix that could be a partial marker.
                    let keep = partial_marker_len(&self.pending, &self.marker);
                    let split = self.pending.len() - keep;
                    if split > 0 {
                        out.push_str(&self.pending[..split]);
                        self.pending.drain(..split);
                    }
                    break;
                }
            }
        }
        out
    }

    /// Flush at end of stream: emit any remaining held content.
    pub fn flush(&mut self) -> String {
        std::mem::take(&mut self.pending)
    }
}

fn partial_marker_len(text: &str, marker: &str) -> usize {
    let bytes = text.as_bytes();
    (1..=marker.len().min(text.len()))
        .rev()
        .find(|&keep| {
            text.is_char_boundary(text.len() - keep)
                && marker.as_bytes().starts_with(&bytes[text.len() - keep..])
        })
        .unwrap_or(0)
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseFrame> {
        let text = match std::str::from_utf8(chunk) {
            Ok(t) => t,
            Err(e) => {
                return vec![SseFrame::Malformed {
                    reason: format!("invalid utf-8: {e}").into(),
                }];
            }
        };
        let mut out = Vec::new();
        // Normalize \r\n and \r to \n, keeping any trailing partial line in
        // the buffer.
        let mut rest = text;
        let mut pending = std::mem::take(&mut self.partial);
        pending.push_str(rest);
        rest = &pending;
        loop {
            let Some(idx) = find_line_end(rest) else {
                break;
            };
            let (line, next) = split_line(rest, idx);
            rest = next;
            if let Some(frame) = self.line(line) {
                out.push(frame);
            }
        }
        self.partial = rest.to_owned();
        out
    }

    /// Flush at end of stream: emit a pending frame if one is complete.
    pub fn finish(&mut self) -> Option<SseFrame> {
        if !self.partial.is_empty() {
            let last = std::mem::take(&mut self.partial);
            // Feed the tail through the line parser; a dispatching blank line
            // cannot occur in a lone unterminated line, so a frame here can
            // only be produced by accumulated fields — which we flush below.
            let _ = self.line(&last);
        }
        if self.saw_any_field {
            let name = self.event_name.take();
            let lines = std::mem::take(&mut self.data_lines);
            self.saw_any_field = false;
            return Some(parse_frame(name.as_deref(), &lines));
        }
        None
    }

    fn line(&mut self, line: &str) -> Option<SseFrame> {
        if line.is_empty() {
            // Blank line dispatches the current frame.
            if self.saw_any_field {
                let name = self.event_name.take();
                let lines = std::mem::take(&mut self.data_lines);
                self.saw_any_field = false;
                return Some(parse_frame(name.as_deref(), &lines));
            }
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => {
                self.event_name = Some(value.to_owned());
                self.saw_any_field = true;
            }
            "data" => {
                self.data_lines.push(value.to_owned());
                self.saw_any_field = true;
            }
            // `id`, `retry`, comments and unknown fields are ignored.
            _ => {}
        }
        None
    }
}

fn find_line_end(s: &str) -> Option<usize> {
    s.find(['\n', '\r'])
}

fn split_line(s: &str, idx: usize) -> (&str, &str) {
    let line = &s[..idx];
    if s.as_bytes().get(idx) == Some(&b'\r') && s.as_bytes().get(idx + 1) == Some(&b'\n') {
        (line, &s[idx + 2..])
    } else {
        (line, &s[idx + 1..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(input: &str) -> Vec<SseFrame> {
        let mut dec = SseDecoder::new();
        let mut out = dec.feed(input.as_bytes());
        if let Some(f) = dec.finish() {
            out.push(f);
        }
        out
    }

    #[test]
    fn basic_data_frame() {
        let f = frames("data: hello\n\n");
        assert_eq!(
            f,
            vec![SseFrame::Data {
                event: "".into(),
                data: "hello".into()
            }]
        );
    }

    #[test]
    fn event_name_and_multiline_data() {
        let f = frames("event: delta\ndata: line1\ndata: line2\n\n");
        assert_eq!(
            f,
            vec![SseFrame::Data {
                event: "delta".into(),
                data: "line1\nline2".into()
            }]
        );
    }

    #[test]
    fn crlf_and_bare_cr_line_endings() {
        let f = frames("data: a\r\rdata: b\r\n\rdata: c\n\n");
        assert_eq!(f.len(), 3);
        assert_eq!(
            f[0],
            SseFrame::Data {
                event: "".into(),
                data: "a".into()
            }
        );
        assert_eq!(
            f[1],
            SseFrame::Data {
                event: "".into(),
                data: "b".into()
            }
        );
        assert_eq!(
            f[2],
            SseFrame::Data {
                event: "".into(),
                data: "c".into()
            }
        );
    }

    #[test]
    fn chunked_input_reassembles() {
        let mut dec = SseDecoder::new();
        assert!(dec.feed(b"data: he").is_empty());
        assert!(dec.feed(b"llo\n").is_empty());
        let out = dec.feed(b"\n");
        assert_eq!(
            out,
            vec![SseFrame::Data {
                event: "".into(),
                data: "hello".into()
            }]
        );
    }

    #[test]
    fn comments_and_unknown_fields_ignored() {
        let f = frames(": keep-alive\nid: 42\nretry: 100\ndata: x\n\n");
        assert_eq!(
            f,
            vec![SseFrame::Data {
                event: "".into(),
                data: "x".into()
            }]
        );
    }

    #[test]
    fn empty_data_is_malformed_not_panic() {
        let mut dec = SseDecoder::new();
        let out = dec.feed(b"event: ping\n\n");
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], SseFrame::Malformed { .. }));
    }

    #[test]
    fn invalid_utf8_surfaces_malformed_frame() {
        let mut dec = SseDecoder::new();
        let out = dec.feed(&[0xff, 0xfe]);
        assert!(matches!(out[0], SseFrame::Malformed { .. }));
        // decoder still usable afterwards
        let out = dec.feed(b"data: ok\n\n");
        assert_eq!(
            out[0],
            SseFrame::Data {
                event: "".into(),
                data: "ok".into()
            }
        );
    }

    #[test]
    fn malformed_does_not_eat_subsequent_frames() {
        let f = frames("data: [BROKEN\n\ndata: good\n\n");
        assert_eq!(f.len(), 2);
        assert_eq!(
            f[0],
            SseFrame::Data {
                event: "".into(),
                data: "[BROKEN".into()
            }
        );
        assert_eq!(
            f[1],
            SseFrame::Data {
                event: "".into(),
                data: "good".into()
            }
        );
    }

    #[test]
    fn finish_flushes_unterminated_frame() {
        let mut dec = SseDecoder::new();
        assert!(dec.feed(b"data: tail").is_empty());
        let out = dec.finish();
        assert_eq!(
            out,
            Some(SseFrame::Data {
                event: "".into(),
                data: "tail".into()
            })
        );
    }

    #[test]
    fn marker_stripper_handles_split_marker() {
        let mut s = MarkerStripper::new("<｜tool▁call▁begin｜>");
        assert_eq!(s.feed("hello "), "hello ");
        assert_eq!(s.feed("<｜tool▁call"), ""); // partial marker kept
        assert_eq!(s.feed("▁begin｜>world"), "world");
        assert_eq!(s.flush(), "");
    }

    #[test]
    fn marker_stripper_passes_plain_text() {
        let mut s = MarkerStripper::new("<x>");
        assert_eq!(s.feed("plain < > text"), "plain < > text");
        assert_eq!(s.flush(), "");
    }

    #[test]
    fn marker_stripper_keeps_content_between_markers() {
        // Strip-only: markers removed, content between them preserved
        // (mirrors original omp `stripDeepseekSpecialTokens` regex replace).
        let mut s = MarkerStripper::new("[[M]]");
        assert_eq!(s.feed("a[[M]]secret[[M]]b"), "asecretb");
        assert_eq!(s.flush(), "");
    }
}
