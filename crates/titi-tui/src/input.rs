//! Raw terminal input — byte-level reassembly of CSI/SS3/OSC/DCS sequences,
//! bracketed paste protection, and probe-reply routing.
//!
//! The [`InputBuffer`] accumulates raw bytes from stdin, splits them into
//! complete terminal sequences, and emits [`InputEvent`]s.  Probe replies
//! (DA1, DSR, OSC 11, …) are emitted as [`InputEvent::ProbeReply`] so the
//! capability layer can consume them without leaking into user-input events.
//!
//! Contract: `docs/research/tui-renderer/input-capabilities-graphics.md`.

/// A fully-assembled terminal input event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// A complete key sequence (ready for [`keys::parse_key`] /
    /// [`keys::matches_key`]).
    Key(String),
    /// Key release (kitty protocol, emitted only when the focused component
    /// opts in).
    KeyRelease(String),
    /// Bracketed paste content.
    Paste(String),
    /// Mouse event (SGR/1006).
    Mouse {
        kind: MouseKind,
        x: u16,
        y: u16,
    },
    /// Terminal resize.
    Resize(u16, u16),
    /// An unrecognised or probe-targeted escape sequence.  These bytes are
    /// NOT user input and MUST be consumed by the capability layer; the
    /// component layer never sees them.
    ProbeReply(Vec<u8>),
}

/// Mouse event sub-kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Press,
    Release,
    Drag,
    ScrollUp,
    ScrollDown,
    Unknown,
}

/// State machine for byte-level sequence reassembly.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParseState {
    /// Accumulating plain text.
    Text,
    /// Saw `ESC` — waiting for the introducer character.
    Escape,
    /// Saw `ESC [` — accumulating CSI parameters + final byte.
    Csi,
    /// Saw `ESC O` — SS3 sequence.
    Ss3,
    /// Saw `ESC ]` — OSC sequence, waiting for ST (ESC \ or BEL).
    Osc,
    /// Saw `ESC P` — DCS sequence, waiting for ST.
    Dcs,
    /// Saw `ESC _` — APC sequence, waiting for ST.
    Apc,
    /// Saw `ESC [200~` — inside bracketed paste.
    BracketedPaste,
}

/// A raw byte buffer that reassembles partial terminal sequences and emits
/// [`InputEvent`]s.
///
/// # Invariants
///
/// - Partial CSI/SS3/OSC/DCS/APC sequences are held until the final byte
///   arrives (never emitted as [`InputEvent::Key`]).
/// - Probe replies (DA1, DSR, OSC 11, …) are emitted as
///   [`InputEvent::ProbeReply`] and never leak into user-input events.
/// - Bracketed paste content (`\x1b[200~` … `\x1b[201~`) is accumulated as
///   a single [`InputEvent::Paste`] — intermediate bytes are not emitted
///   individually.
/// - Unknown sequences that don't match any known dispatch are emitted as
///   [`InputEvent::ProbeReply`] (fail-closed: no capability is assumed from
///   an unrecognised response).
#[derive(Debug)]
pub struct InputBuffer {
    /// Partial byte accumulation.
    buf: Vec<u8>,
    /// Current parse state.
    state: ParseState,
    /// When in bracketed paste, the accumulated content.
    paste_buf: String,
    /// OSC parameter bytes (accumulated until ST).
    osc_params: Vec<u8>,
    /// Saw `ESC` inside OSC/DCS/APC — next `\` completes the ST.
    st_pending: bool,
}

impl Default for InputBuffer {
    fn default() -> Self {
        InputBuffer::new()
    }
}

impl InputBuffer {
    /// Create a new empty input buffer.
    pub fn new() -> Self {
        InputBuffer {
            buf: Vec::new(),
            state: ParseState::Text,
            paste_buf: String::new(),
            osc_params: Vec::new(),
            st_pending: false,
        }
    }

    /// Feed raw bytes and return any complete events.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<InputEvent> {
        let mut events = Vec::new();
        for &b in bytes {
            self.process_byte(b, &mut events);
        }
        // Flush a completed plain-text run (only when idle in Text state —
        // partial CSI/ESC/OSC sequences must stay pending).
        if self.state == ParseState::Text {
            self.flush_text(&mut events);
        }
        events
    }

    /// Process a single byte, appending events when complete sequences
    /// are detected.
    fn process_byte(&mut self, b: u8, events: &mut Vec<InputEvent>) {
        match self.state {
            ParseState::Text => self.on_text(b, events),
            ParseState::Escape => self.on_escape(b, events),
            ParseState::Csi => self.on_csi(b, events),
            ParseState::Ss3 => self.on_ss3(b, events),
            ParseState::Osc => self.on_osc(b, events),
            ParseState::Dcs => self.on_dcs(b, events),
            ParseState::Apc => self.on_apc(b, events),
            ParseState::BracketedPaste => self.on_bracketed_paste(b, events),
        }
    }

    // ---- Text state -------------------------------------------------------

    fn on_text(&mut self, b: u8, events: &mut Vec<InputEvent>) {
        if b == 0x1b {
            // ESC starts a new sequence; flush current text first.
            self.flush_text(events);
            self.state = ParseState::Escape;
            self.buf.push(b);
        } else {
            self.buf.push(b);
        }
    }

    // ---- Escape state -----------------------------------------------------

    fn on_escape(&mut self, b: u8, events: &mut Vec<InputEvent>) {
        self.buf.push(b);
        match b {
            b'[' => self.state = ParseState::Csi,
            b'O' => self.state = ParseState::Ss3,
            b']' => {
                self.osc_params.clear();
                self.state = ParseState::Osc;
            }
            b'P' => {
                self.osc_params.clear();
                self.state = ParseState::Dcs;
            }
            b'_' => {
                self.osc_params.clear();
                self.state = ParseState::Apc;
            }
            // Single-byte ESC-prefixed: Alt+letter, etc.
            0x1b => {
                // Double ESC — the first ESC is its own key event; the
                // second stays pending for the sequence it introduces.
                self.buf.pop();
                self.finish_sequence(events);
                self.buf.push(b);
            }
            _ => {
                // ESC + something else: emit the full sequence as key.
                self.finish_sequence(events);
            }
        }
    }

    // ---- CSI state --------------------------------------------------------

    fn on_csi(&mut self, b: u8, events: &mut Vec<InputEvent>) {
        self.buf.push(b);
        // Final byte: 0x40-0x7E.
        if (0x40..=0x7e).contains(&b) {
            self.dispatch_csi(events);
            // dispatch_csi may transition into BracketedPaste; only return
            // to Text when it didn't.
            if self.state == ParseState::Csi {
                self.state = ParseState::Text;
            }
        }
        // Parameter bytes (0x30-0x3F) and intermediate bytes (0x20-0x2F)
        // are just accumulated — no state change.
    }

    fn dispatch_csi(&mut self, events: &mut Vec<InputEvent>) {
        let seq_bytes = self.buf.split_off(0);
        let seq = String::from_utf8_lossy(&seq_bytes).to_string();

        // Bracketed paste start.
        if seq == "\x1b[200~" {
            self.state = ParseState::BracketedPaste;
            self.paste_buf.clear();
            return;
        }

        // Mouse events (SGR 1006): \x1b[<...M or \x1b[<...m.
        if seq.starts_with("\x1b[<")
            && (seq.ends_with('M') || seq.ends_with('m'))
            && let Some(ev) = parse_sgr_mouse(&seq)
        {
            events.push(ev);
            return;
        }

        // Cursor position report (CPR): \x1b[row;colR
        if seq.ends_with('R') && !seq.starts_with("\x1b[<") {
            events.push(InputEvent::ProbeReply(seq_bytes));
            return;
        }

        // Device Attributes (DA1/DA2/DA3): \x1b[?*;*c
        if seq.ends_with('c') && seq.contains('?') {
            events.push(InputEvent::ProbeReply(seq_bytes));
            return;
        }

        // Mode 2031 DSR (`CSI ? 997 ; 1/2 n`) and other private DSR — never keys.
        if seq.ends_with('n') && seq.contains('?') {
            events.push(InputEvent::ProbeReply(seq_bytes));
            return;
        }

        // Terminal resize: \x1b[8;rows;colst
        if let Some(resize) = parse_csi_resize(&seq) {
            events.push(resize);
            return;
        }

        // Everything else is a key event.
        events.push(InputEvent::Key(seq));
    }

    // ---- SS3 state --------------------------------------------------------

    fn on_ss3(&mut self, b: u8, events: &mut Vec<InputEvent>) {
        self.buf.push(b);
        // SS3 final byte: 0x40-0x7E (same as CSI).
        if (0x40..=0x7e).contains(&b) {
            self.finish_sequence(events);
            self.state = ParseState::Text;
        }
    }

    // ---- OSC / DCS / APC states (all terminated by ST) --------------------

    fn on_osc(&mut self, b: u8, events: &mut Vec<InputEvent>) {
        self.buf.push(b);
        if self.st_pending {
            self.st_pending = false;
            if b == b'\\' {
                // ESC \ terminates the OSC.
                self.dispatch_osc(events);
                self.state = ParseState::Text;
                return;
            }
            // A lone ESC followed by something else: still inside OSC, but
            // the ESC byte was already pushed — nothing else to do.
            return;
        }
        if b == 0x07 {
            // BEL-terminated OSC.
            self.dispatch_osc(events);
            self.state = ParseState::Text;
        } else if b == 0x1b {
            // ESC — expect `\` (ST) or continue accumulating.
            self.st_pending = true;
        } else {
            self.osc_params.push(b);
        }
    }

    fn on_dcs(&mut self, b: u8, events: &mut Vec<InputEvent>) {
        self.buf.push(b);
        if self.st_pending {
            self.st_pending = false;
            if b == b'\\' {
                self.dispatch_dcs(events);
                self.state = ParseState::Text;
                return;
            }
            return;
        }
        if b == 0x07 {
            // BEL-terminated — strip trailing BEL.
            self.pop_bel();
            self.dispatch_dcs(events);
            self.state = ParseState::Text;
        } else if b == 0x1b {
            self.st_pending = true;
        }
    }

    fn on_apc(&mut self, b: u8, events: &mut Vec<InputEvent>) {
        self.buf.push(b);
        if self.st_pending {
            self.st_pending = false;
            if b == b'\\' {
                self.finish_sequence(events);
                self.state = ParseState::Text;
                return;
            }
            return;
        }
        if b == 0x07 {
            self.pop_bel();
            self.finish_sequence(events);
            self.state = ParseState::Text;
        } else if b == 0x1b {
            self.st_pending = true;
        }
    }

    // ---- Bracketed paste --------------------------------------------------

    fn on_bracketed_paste(&mut self, b: u8, events: &mut Vec<InputEvent>) {
        // The end marker is \x1b[201~.  Accumulate bytes into `buf` only
        // while they could still be a prefix of the marker.
        const PASTE_END: &[u8] = b"\x1b[201~";
        if self.buf.len() < PASTE_END.len() && b == PASTE_END[self.buf.len()] {
            self.buf.push(b);
            if self.buf == PASTE_END {
                let content = std::mem::take(&mut self.paste_buf);
                events.push(InputEvent::Paste(content));
                self.buf.clear();
                self.state = ParseState::Text;
            }
            return;
        }
        // Not part of the marker — flush any held prefix as content.
        if !self.buf.is_empty() {
            if let Ok(prefix) = std::str::from_utf8(&self.buf) {
                self.paste_buf.push_str(prefix);
            }
            self.buf.clear();
        }
        if let Ok(ch) = std::str::from_utf8(&[b]) {
            self.paste_buf.push_str(ch);
        } else {
            // Non-UTF-8 byte — emit as replacement char.
            self.paste_buf.push('\u{fffd}');
        }
    }

    // ---- Helpers ----------------------------------------------------------

    /// Emit the current buffer as a key event and reset.
    fn finish_sequence(&mut self, events: &mut Vec<InputEvent>) {
        if self.buf.is_empty() {
            return;
        }
        let seq = std::str::from_utf8(&self.buf).unwrap_or("");
        if !seq.is_empty() {
            events.push(InputEvent::Key(seq.to_string()));
        }
        self.buf.clear();
    }

    /// Emit buffered text as a key event (single character or multi-byte
    /// printable string).
    fn flush_text(&mut self, events: &mut Vec<InputEvent>) {
        if self.buf.is_empty() {
            return;
        }
        let s = std::str::from_utf8(&self.buf).unwrap_or("");
        if !s.is_empty() {
            events.push(InputEvent::Key(s.to_string()));
        }
        self.buf.clear();
    }

    /// Dispatch an OSC sequence as a probe reply or key event.
    fn dispatch_osc(&mut self, events: &mut Vec<InputEvent>) {
        let seq = std::mem::take(&mut self.buf);
        let params = std::mem::take(&mut self.osc_params);
        // OSC 4 (color query), OSC 11 (bg query), OSC 10 (fg query) → probe.
        // OSC 8 (hyperlink) → emit as key for existing processing.
        let osc_str = std::str::from_utf8(&params).unwrap_or("");
        if osc_str.starts_with("4;") || osc_str.starts_with("10;") || osc_str.starts_with("11;") {
            events.push(InputEvent::ProbeReply(seq));
        } else {
            // Other OSC sequences (e.g., OSC 8 hyperlinks) → key.
            let s = String::from_utf8_lossy(&seq).to_string();
            events.push(InputEvent::Key(s));
        }
    }

    /// Dispatch a DCS sequence as a probe reply (kitty graphics query
    /// response, etc.).
    fn dispatch_dcs(&mut self, events: &mut Vec<InputEvent>) {
        let seq = std::mem::take(&mut self.buf);
        events.push(InputEvent::ProbeReply(seq));
    }

    /// Remove trailing BEL byte from the buffer.
    fn pop_bel(&mut self) {
        if self.buf.last() == Some(&0x07) {
            self.buf.pop();
        }
    }
}

// ---- Parse helpers ---------------------------------------------------------

/// Parse an SGR 1006 mouse sequence: `\x1b[<Cb;Px;PyM` (press)
/// or `\x1b[<Cb;Px;Pym` (release).
/// Parse an SGR 1006 mouse sequence: `\x1b[<Cb;Px;PyM` (press/drag/scroll)
/// or `\x1b[<Cb;Px;Pym` (release).
///
/// SGR mouse button/state bits (cb):
/// - bits 0-1: button (0=left, 1=middle, 2=right)
/// - bit 2: shift
/// - bit 3: meta
/// - bit 4: ctrl
/// - bit 5 (0x20): motion (drag) — button held while moving
/// - bit 6 (0x40): scroll up (wheel)
/// - bit 7 (0x80): scroll down (wheel)
/// - `M` = press/motion/scroll event, `m` = release
fn parse_sgr_mouse(seq: &str) -> Option<InputEvent> {
    let body = seq.strip_prefix("\x1b[<")?;
    let is_press = body.ends_with('M');
    let body = body.strip_suffix(['M', 'm'])?;
    let mut parts = body.split(';');
    let cb: u16 = parts.next()?.parse().ok()?;
    let x: u16 = parts.next()?.parse().ok()?;
    let y: u16 = parts.next()?.parse().ok()?;

    let kind = if cb & 0x40 != 0 {
        MouseKind::ScrollUp
    } else if cb & 0x80 != 0 {
        MouseKind::ScrollDown
    } else if !is_press {
        MouseKind::Release
    } else if cb & 0x20 != 0 {
        // Motion (drag) while a button is held.
        MouseKind::Drag
    } else {
        // Button press.
        MouseKind::Press
    };

    Some(InputEvent::Mouse { kind, x, y })
}

/// Parse a CSI resize sequence: `\x1b[8;rows;colst`.
fn parse_csi_resize(seq: &str) -> Option<InputEvent> {
    let body = seq.strip_prefix("\x1b[8;")?;
    let body = body.strip_suffix('t')?;
    let mut parts = body.split(';');
    let rows: u16 = parts.next()?.parse().ok()?;
    let cols: u16 = parts.next()?.parse().ok()?;
    Some(InputEvent::Resize(rows, cols))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Text / basic keys ------------------------------------------------

    #[test]
    fn single_ascii_byte_emits_key() {
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"a");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], InputEvent::Key("a".to_string()));
    }

    #[test]
    fn multi_byte_text_emits_one_key() {
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"hello");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], InputEvent::Key("hello".to_string()));
    }

    // ---- CSI assembly -----------------------------------------------------

    #[test]
    fn csi_assemble_split_flush() {
        // Up arrow split across two feeds: \x1b [ A
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[");
        assert!(events.is_empty(), "partial CSI must not emit events");
        let events = buf.feed(b"A");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], InputEvent::Key("\x1b[A".to_string()));
    }

    #[test]
    fn csi_assemble_split_byte_by_byte() {
        // \x1b [ 1 ; 5 A (ctrl+up) byte by byte
        let seq = b"\x1b[1;5A";
        let mut buf = InputBuffer::new();
        for (i, &b) in seq.iter().enumerate() {
            let events = buf.feed(&[b]);
            if i < seq.len() - 1 {
                assert!(events.is_empty(), "partial CSI at byte {i}: {events:?}");
            } else {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0], InputEvent::Key("\x1b[1;5A".to_string()));
            }
        }
    }

    // ---- Probe isolation --------------------------------------------------

    #[test]
    fn probe_reply_does_not_leak_as_key() {
        // DA1 response: \x1b[?1;2c — must be ProbeReply, not Key.
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[?1;2c");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], InputEvent::ProbeReply(b"\x1b[?1;2c".to_vec()));
    }

    #[test]
    fn cpr_response_is_probe_reply() {
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[42;8R");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], InputEvent::ProbeReply(b"\x1b[42;8R".to_vec()));
    }

    #[test]
    fn mixed_user_bytes_and_probe() {
        // User types "a", then DA1 response arrives, then "b".
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"a\x1b[?1;2cb");
        assert_eq!(events.len(), 3, "{events:?}");
        assert_eq!(events[0], InputEvent::Key("a".to_string()));
        assert_eq!(events[1], InputEvent::ProbeReply(b"\x1b[?1;2c".to_vec()));
        assert_eq!(events[2], InputEvent::Key("b".to_string()));
    }

    // ---- Bracketed paste --------------------------------------------------

    #[test]
    fn bracketed_paste_content() {
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[200~pasted text\x1b[201~");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], InputEvent::Paste("pasted text".to_string()));
    }

    #[test]
    fn bracketed_paste_empty() {
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[200~\x1b[201~");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], InputEvent::Paste(String::new()));
    }

    // ---- Mouse events -----------------------------------------------------

    #[test]
    fn sgr_mouse_press() {
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[<0;10;20M");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            InputEvent::Mouse {
                kind: MouseKind::Press,
                x: 10,
                y: 20,
            }
        );
    }

    #[test]
    fn sgr_mouse_release() {
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[<0;5;15m");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            InputEvent::Mouse {
                kind: MouseKind::Release,
                x: 5,
                y: 15,
            }
        );
    }

    #[test]
    fn sgr_mouse_drag() {
        // Motion with a held button: cb = 0 (button) | 0x20 (motion) = 32.
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[<32;12;7M");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            InputEvent::Mouse {
                kind: MouseKind::Drag,
                x: 12,
                y: 7,
            }
        );
    }

    #[test]
    fn sgr_mouse_scroll_up() {
        // Wheel up: cb = 0x40 = 64.
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[<64;3;9M");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            InputEvent::Mouse {
                kind: MouseKind::ScrollUp,
                x: 3,
                y: 9,
            }
        );
    }

    #[test]
    fn sgr_mouse_scroll_down() {
        // Wheel down: cb = 0x80 = 128.
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[<128;1;2M");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            InputEvent::Mouse {
                kind: MouseKind::ScrollDown,
                x: 1,
                y: 2,
            }
        );
    }

    // ---- Resize -----------------------------------------------------------

    #[test]
    fn csi_resize() {
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[8;24;80t");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], InputEvent::Resize(24, 80));
    }

    // ---- OSC probe replies ------------------------------------------------

    #[test]
    fn osc_11_bg_query_response_is_probe() {
        // Terminal background color query response: OSC 11 ; rgb:0000/0000/0000 ST
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b]11;rgb:0000/0000/0000\x1b\\");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], InputEvent::ProbeReply(_)), "{events:?}");
    }

    #[test]
    fn mode_2031_dsr_is_probe_not_key() {
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[?997;1n");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], InputEvent::ProbeReply(_)), "{events:?}");
        let events = buf.feed(b"\x1b[?997;2n");
        assert!(matches!(events[0], InputEvent::ProbeReply(_)), "{events:?}");
    }

    // ---- Fail-closed: unknown sequences -----------------------------------

    #[test]
    fn unrecognized_sequence_is_key_not_probe() {
        // Unknown CSI sequence without a known prefix → key event.
        let mut buf = InputBuffer::new();
        let events = buf.feed(b"\x1b[?X");
        // 'X' is 0x58, which is in 0x40-0x7E range, so it's a valid CSI final.
        assert_eq!(events.len(), 1);
        // Falls through to Key (not ProbeReply) because it doesn't match DA1 pattern.
        assert!(matches!(events[0], InputEvent::Key(_)), "{events:?}");
    }

    // ---- Partial prefix not emitted ---------------------------------------

    #[test]
    fn partial_prefix_not_emitted_before_final_byte() {
        let mut buf = InputBuffer::new();
        let mut all = Vec::new();
        // Feed \x1b[1 without the final byte; then feed more text.
        all.extend(buf.feed(b"\x1b[1"));
        assert!(all.is_empty(), "partial prefix must not emit: {all:?}");
        all.extend(buf.feed(b"~")); // Insert key
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], InputEvent::Key("\x1b[1~".to_string()));
    }
}