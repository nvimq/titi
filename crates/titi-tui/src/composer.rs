//! Input composer — bracketed paste, OSC 5522 enhanced paste, paste
//! collapse, and non-blocking message queue (Steer / FollowUp).
//!
//! Contract: `docs/research/agent-ux/README.md`.

use std::collections::VecDeque;

/// Mode for a queued message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueMode {
    Steer,
    FollowUp,
}

/// A message queued during streaming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub text: String,
    pub mode: QueueMode,
}

/// Result of collapsing a long paste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteResult {
    /// Short enough to paste inline.
    Text(String),
    /// Long paste collapsed: preview + number of hidden lines.
    Collapsed {
        preview: String,
        omitted_lines: usize,
    },
    /// Single file path → attachment marker.
    Attachment {
        name: String,
        marker: String,
    },
}

/// Parsed OSC 5522 enhanced paste payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Osc5522 {
    pub mime: String,
    pub data: Vec<u8>,
}

/// A paste-state spanning (simplified: engine tracks bracketed vs enhanced).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteState {
    None,
    Bracketed,
    // Enhanced paste (OSC 5522) is parsed at reception; state is transient.
}

/// The composer — input buffer + streaming message queue.
#[derive(Debug, Clone)]
pub struct Composer {
    /// Current editor buffer text (single-line prompt).
    pub buffer: String,
    /// Queue of messages placed during streaming.
    queue: VecDeque<Queued>,
    /// Paste state.
    paste: PasteState,
}

impl Composer {
    /// Create a new, empty composer.
    pub fn new() -> Self {
        Composer {
            buffer: String::new(),
            queue: VecDeque::new(),
            paste: PasteState::None,
        }
    }

    /// Insert a character into the buffer.
    pub fn insert(&mut self, ch: char) {
        self.buffer.push(ch);
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Set the buffer content.
    pub fn set_buffer(&mut self, text: &str) {
        self.buffer = text.to_owned();
    }

    /// Queue a message during streaming (Steer or FollowUp).
    pub fn push_queue(&mut self, text: String, mode: QueueMode) {
        self.queue.push_back(Queued { text, mode });
    }

    /// Dequeue the last queued message and return it, or `None` if empty.
    pub fn dequeue_last(&mut self) -> Option<Queued> {
        self.queue.pop_back()
    }

    /// Number of queued messages.
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the queue is empty.
    pub fn queue_is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Peek the last queued message without removing it.
    pub fn queue_peek_last(&self) -> Option<&Queued> {
        self.queue.back()
    }

    /// Set paste state.
    pub fn set_paste(&mut self, state: PasteState) {
        self.paste = state;
    }

    /// Current paste state.
    pub fn paste(&self) -> PasteState {
        self.paste
    }

    /// Collapse a long multiline paste.
    ///
    /// - If the text is a single image path (`.png`, `.jpg`, `.jpeg`, `.gif`,
    ///   `.bmp`, `.ico`) → `Attachment` with a markdown image marker.
    /// - If the text has ≤ `max_lines` lines → `Text` verbatim.
    /// - Otherwise → `Collapsed` with the first line and omitted count.
    pub fn collapse_paste(text: &str, max_lines: usize) -> PasteResult {
        // Check for a single image path.
        let trimmed = text.trim();
        if is_image_path(trimmed) {
            return PasteResult::Attachment {
                name: trimmed.to_owned(),
                marker: format!("[Image: {}]", trimmed),
            };
        }

        let lines: Vec<&str> = text.lines().collect();
        if lines.len() <= max_lines {
            return PasteResult::Text(text.to_owned());
        }

        PasteResult::Collapsed {
            preview: lines[0].to_owned(),
            omitted_lines: lines.len() - max_lines,
        }
    }
}

impl Default for Composer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// OSC 5522 parser
// ---------------------------------------------------------------------------

/// Parse an OSC 5522 enhanced paste payload.
///
/// The payload is the segment between `\x1b]5522;` and the string terminator
/// (ST `\x1b\\` or BEL `\x07`).  Format: `<mime>;<base64-data>`.
///
/// Returns `None` when the payload is malformed or base64 cannot be decoded.
pub fn parse_osc5522(payload: &str) -> Option<Osc5522> {
    let (mime, b64) = payload.split_once(';')?;
    if mime.is_empty() || b64.is_empty() {
        return None;
    }
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()?;
    Some(Osc5522 {
        mime: mime.to_owned(),
        data,
    })
}

/// Check whether `s` looks like a single image file path (no newlines, known
/// extension).
fn is_image_path(s: &str) -> bool {
    if s.contains('\n') {
        return false;
    }
    let ext = match s.rsplit_once('.') {
        Some((_, e)) => e.to_lowercase(),
        None => return false,
    };
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "ico"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Queue ------------------------------------------------------------

    #[test]
    fn queue_push_and_dequeue() {
        let mut c = Composer::new();
        c.push_queue("msg1".into(), QueueMode::Steer);
        c.push_queue("msg2".into(), QueueMode::FollowUp);
        assert_eq!(c.queue_len(), 2);

        let last = c.dequeue_last().unwrap();
        assert_eq!(last.text, "msg2");
        assert_eq!(last.mode, QueueMode::FollowUp);
        assert_eq!(c.queue_len(), 1);

        let first = c.dequeue_last().unwrap();
        assert_eq!(first.text, "msg1");
        assert_eq!(c.queue_len(), 0);
        assert!(c.dequeue_last().is_none());
    }

    #[test]
    fn dequeue_empty_returns_none() {
        let mut c = Composer::new();
        assert!(c.dequeue_last().is_none());
        assert!(c.queue_is_empty());
    }

    #[test]
    fn queue_peek_does_not_remove() {
        let mut c = Composer::new();
        c.push_queue("hello".into(), QueueMode::Steer);
        assert_eq!(c.queue_peek_last().unwrap().text, "hello");
        assert_eq!(c.queue_len(), 1, "peek does not remove");
    }

    // ---- Paste collapse ---------------------------------------------------

    #[test]
    fn short_paste_is_text() {
        let result = Composer::collapse_paste("hello world", 5);
        assert_eq!(result, PasteResult::Text("hello world".into()));
    }

    #[test]
    fn long_paste_is_collapsed() {
        let long = "first line\nsecond\nthird\nfourth\nfifth\nsixth";
        let result = Composer::collapse_paste(long, 3);
        match result {
            PasteResult::Collapsed {
                preview,
                omitted_lines,
            } => {
                assert_eq!(preview, "first line");
                assert_eq!(omitted_lines, 3, "6 lines - 3 max = 3 omitted");
            }
            other => panic!("expected Collapsed, got {other:?}"),
        }
    }

    #[test]
    fn image_path_returns_attachment() {
        let result = Composer::collapse_paste("/path/to/photo.png", 5);
        match result {
            PasteResult::Attachment { name, marker } => {
                assert_eq!(name, "/path/to/photo.png");
                assert!(marker.contains("photo.png"));
            }
            other => panic!("expected Attachment, got {other:?}"),
        }
    }

    #[test]
    fn jpeg_path_returns_attachment() {
        let result = Composer::collapse_paste("image.JPEG", 5);
        assert!(matches!(result, PasteResult::Attachment { .. }));
    }

    #[test]
    fn non_image_path_is_not_attachment() {
        let result = Composer::collapse_paste("/path/to/file.txt", 5);
        assert!(matches!(result, PasteResult::Text(_)));
    }

    #[test]
    fn multiline_image_path_is_not_attachment() {
        let result = Composer::collapse_paste("photo.png\nmore", 5);
        assert!(matches!(result, PasteResult::Text(_)), "has newline");
    }

    // ---- OSC 5522 ---------------------------------------------------------

    #[test]
    fn parse_osc5522_valid() {
        let payload = "image/png;aGVsbG8="; // base64("hello")
        let parsed = parse_osc5522(payload).unwrap();
        assert_eq!(parsed.mime, "image/png");
        assert_eq!(parsed.data, b"hello");
    }

    #[test]
    fn parse_osc5522_empty_mime() {
        assert!(parse_osc5522(";aGVsbG8=").is_none());
    }

    #[test]
    fn parse_osc5522_missing_semicolon() {
        assert!(parse_osc5522("justtext").is_none());
    }

    #[test]
    fn parse_osc5522_invalid_base64() {
        assert!(parse_osc5522("text/plain;!!!invalid!!!").is_none());
    }

    // ---- Buffer -----------------------------------------------------------

    #[test]
    fn composer_buffer_insert_clear() {
        let mut c = Composer::new();
        assert!(c.buffer.is_empty());
        c.insert('h');
        c.insert('i');
        assert_eq!(c.buffer, "hi");
        c.clear();
        assert!(c.buffer.is_empty());
    }

    #[test]
    fn composer_set_buffer() {
        let mut c = Composer::new();
        c.set_buffer("hello");
        assert_eq!(c.buffer, "hello");
    }
}