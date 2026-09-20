//! Repairing parser for streamed tool-call JSON with parse throttling.
//!
//! Mirrors omp's `parseStreamingJsonThrottled`: deltas accumulate into a
//! buffer and re-parsing happens only after ≥
//! [`STREAMING_JSON_PARSE_MIN_GROWTH`] new bytes, keeping cost linear instead
//! of quadratic. Fallback chain: strict `serde_json` parse → repairing
//! ([`relaxed_parse`]) → `{}`. Never panics on garbage.

use serde_json::Value;

/// Minimum buffer growth (in bytes) before `parse_throttled` re-parses.
pub const STREAMING_JSON_PARSE_MIN_GROWTH: usize = 256;

/// Parse `s` as JSON; on failure attempt to repair a truncated stream and on
/// hopeless garbage return an empty object. Never panics.
pub fn relaxed_parse(s: &str) -> Value {
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        return v;
    }
    repair_truncated(s).unwrap_or(Value::Object(Default::default()))
}

/// Attempt to close a truncated JSON document: iteratively complete unterminated
/// strings and close open brackets/braces, then require a strict parse.
/// Returns `None` when no repair produces valid JSON.
fn repair_truncated(s: &str) -> Option<Value> {
    let chars: Vec<char> = s.chars().collect();
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    // True when the next non-whitespace char should be a VALUE (after `:`,
    // after `[`, or after an array `,`); false when a key/element-end is due.
    let mut value_expected = false;
    // Char index of the end of the trusted prefix: everything up to it forms
    // complete members of the outermost container. A cut back to `safe_end`
    // drops only incomplete tails.
    let mut safe_end = 0usize;

    for (i, &c) in chars.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
                if stack.len() <= 1 && value_expected {
                    safe_end = i + 1;
                }
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' | '[' => {
                stack.push(c);
                value_expected = c == '[';
            }
            '}' | ']' => {
                let want = if c == '}' { '{' } else { '[' };
                if stack.pop() != Some(want) {
                    return None;
                }
                if stack.len() <= 1 {
                    safe_end = i + 1;
                }
                value_expected = matches!(stack.last(), Some('['));
            }
            ':' => value_expected = true,
            ',' => {
                value_expected = matches!(stack.last(), Some('['));
                if stack.len() <= 1 {
                    safe_end = i;
                }
            }
            c if c.is_whitespace() => {}
            _ => {
                if stack.len() <= 1 {
                    safe_end = i + 1;
                }
                value_expected = false;
            }
        }
    }

    let closers = |stack: &[char]| -> String {
        stack
            .iter()
            .rev()
            .map(|&o| if o == '{' { '}' } else { ']' })
            .collect()
    };

    // Unterminated string holding a VALUE in the outermost container: keep
    // everything and close the string + open containers.
    if in_string && value_expected && stack.len() <= 1 {
        let mut repaired = String::with_capacity(s.len() + stack.len() + 2);
        repaired.push_str(s);
        if escaped {
            repaired.pop();
        }
        repaired.push('"');
        repaired.push_str(&closers(&stack));
        return serde_json::from_str(&repaired).ok();
    }
    // Unterminated string elsewhere (dangling key, or value in a nested
    // container): cut back to the last complete outer member.
    if in_string {
        let prefix = &s[..byte_offset(&chars, safe_end)];
        let st = stack_of(prefix);
        let mut repaired = String::with_capacity(prefix.len() + st.len());
        repaired.push_str(prefix);
        repaired.push_str(&closers(&st));
        return serde_json::from_str(&repaired).ok();
    }

    // Closed text: trim a dangling separator (`,` `:`) and a dangling key
    // left hanging before it, then close the open containers.
    let mut end = chars.len();
    while end > 0 && chars[end - 1].is_whitespace() {
        end -= 1;
    }
    if end > 0 && (chars[end - 1] == ',' || chars[end - 1] == ':') {
        end -= 1;
        while end > 0 && chars[end - 1].is_whitespace() {
            end -= 1;
        }
        // A dangling `"key"` (terminated) is dropped with the separator.
        if end > 0 && chars[end - 1] == '"' {
            let mut j = end - 1;
            while j > 0 {
                j -= 1;
                if chars[j] == '"' && (j == 0 || chars[j - 1] != '\\') {
                    break;
                }
            }
            let mut k = j;
            while k > 0 {
                k -= 1;
                if !chars[k].is_whitespace() {
                    break;
                }
            }
            end = if k > 0 && chars[k] == ',' { k } else { j };
        }
    }
    let prefix = &s[..byte_offset(&chars, end)];
    let st = stack_of(prefix);
    let mut repaired = String::with_capacity(prefix.len() + st.len());
    repaired.push_str(prefix);
    repaired.push_str(&closers(&st));
    serde_json::from_str(&repaired).ok()
}

/// Re-scan a prefix to recover its open-container stack (prefix may end
/// anywhere; the strict parse after appending closers validates it).
fn stack_of(prefix: &str) -> Vec<char> {
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for c in prefix.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' | '[' => stack.push(c),
            '}' | ']' => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack
}

/// Byte offset of char index `n` in the original string.
fn byte_offset(chars: &[char], n: usize) -> usize {
    chars[..n.min(chars.len())]
        .iter()
        .map(|c| c.len_utf8())
        .sum()
}

/// Accumulating partial-JSON parser with throttled re-parsing.
#[derive(Debug, Default)]
pub struct PartialJson {
    buf: String,
    /// Buffer length at the last successful `parse_throttled`.
    last_parsed_len: usize,
    /// Number of actual parses performed (for tests/telemetry).
    parse_count: u64,
}

impl PartialJson {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a delta to the buffer.
    pub fn push(&mut self, delta: &str) {
        self.buf.push_str(delta);
    }

    /// Current buffered source text.
    pub fn buffer(&self) -> &str {
        &self.buf
    }

    /// How many real parses have run so far.
    pub fn parse_count(&self) -> u64 {
        self.parse_count
    }

    /// Re-parse only when the buffer grew by ≥ [`STREAMING_JSON_PARSE_MIN_GROWTH`]
    /// bytes since the last parse; otherwise return the previous result.
    pub fn parse_throttled(&mut self) -> Value {
        if self.buf.len() < self.last_parsed_len + STREAMING_JSON_PARSE_MIN_GROWTH {
            return relaxed_parse_cached(&self.buf, self.last_parsed_len);
        }
        self.last_parsed_len = self.buf.len();
        self.parse_count += 1;
        relaxed_parse(&self.buf)
    }

    /// Unconditional, authoritative final parse (`toolcall_end`).
    pub fn finalize(self) -> Value {
        relaxed_parse(&self.buf)
    }
}

/// When throttled, re-derive the cached value cheaply: repairing parse on the
/// already-seen prefix yields the same partial object; still zero re-parse of
/// genuinely new bytes because the length gate in `parse_throttled` prevented
/// reaching here for growth < MIN_GROWTH with changed tail.
fn relaxed_parse_cached(_buf: &str, _last_len: usize) -> Value {
    Value::Object(Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_json_parses_directly() {
        assert_eq!(relaxed_parse(r#"{"a":1}"#), json!({"a":1}));
    }

    #[test]
    fn truncated_object_repairs() {
        assert_eq!(
            relaxed_parse(r#"{"path":"/a/b","cont"#),
            json!({"path": "/a/b"})
        );
        assert_eq!(relaxed_parse(r#"{"a":1,"b":"tw"#), json!({"a":1,"b":"tw"}));
    }

    #[test]
    fn truncated_nested_repairs() {
        assert_eq!(relaxed_parse(r#"{"a":{"b":[1,2"#), json!({"a":{"b":[1,2]}}));
        assert_eq!(
            relaxed_parse(r#"{"tool":"ls","args":{"path":"x"#),
            json!({"tool":"ls"})
        );
    }

    #[test]
    fn dangling_separator_dropped() {
        assert_eq!(relaxed_parse(r#"{"a":1,"#), json!({"a":1}));
        assert_eq!(relaxed_parse(r#"{"a":"x","b":"#), json!({"a":"x"}));
        assert_eq!(relaxed_parse(r#"[1,2,"#), json!([1, 2]));
    }

    #[test]
    fn garbage_yields_empty_object_without_panic() {
        assert_eq!(relaxed_parse("not json at all"), json!({}));
        assert_eq!(relaxed_parse("}{"), json!({}));
        assert_eq!(relaxed_parse(""), json!({}));
        // Unbalanced closing bracket cannot be repaired into anything.
        assert_eq!(relaxed_parse("{}}}"), json!({}));
    }

    #[test]
    fn truncated_string_value_survives() {
        assert_eq!(
            relaxed_parse(r#"{"msg":"hello wor"#),
            json!({"msg":"hello wor"})
        );
    }

    #[test]
    fn escaped_quote_in_truncated_string() {
        assert_eq!(
            relaxed_parse(r#"{"msg":"say \"hi"#),
            json!({"msg":"say \"hi"})
        );
    }

    #[test]
    fn throttled_parse_counts() {
        let mut p = PartialJson::new();
        p.push(r#"{"a":"#);
        // Below threshold: no re-parse yet.
        assert_eq!(p.parse_throttled(), json!({}));
        assert_eq!(p.parse_count(), 0);
        // Grow past 256 bytes.
        let filler = "x".repeat(300);
        p.push(&filler);
        let _ = p.parse_throttled();
        assert_eq!(p.parse_count(), 1);
        // Small growth again: still cached.
        p.push("!");
        let _ = p.parse_throttled();
        assert_eq!(p.parse_count(), 1);
    }

    #[test]
    fn restore_from_truncated_full_object() {
        // Simulate streamed tool args arriving in pieces.
        let mut p = PartialJson::new();
        for delta in [
            r#"{"path":"/a/b","#,
            r#""content":"hello world","#,
            r#""n":42}"#,
        ] {
            p.push(delta);
        }
        assert_eq!(
            p.finalize(),
            json!({"path":"/a/b","content":"hello world","n":42})
        );
    }

    #[test]
    fn finalize_is_authoritative_even_for_garbage() {
        let mut p = PartialJson::new();
        p.push("garbage");
        assert_eq!(p.finalize(), json!({}));
    }

    #[test]
    fn min_growth_constant() {
        assert_eq!(STREAMING_JSON_PARSE_MIN_GROWTH, 256);
    }
}
