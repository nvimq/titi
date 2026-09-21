//! Secrets never reach the memory index.
//!
//! A memory is injected into the system prompt of every later turn, so a key
//! stored once is a key shown forever. The value is replaced before the write;
//! the fact that a key existed is kept, because "the deploy token was rotated"
//! is worth remembering and the token is not.

use regex::Regex;
use std::sync::LazyLock;

/// What was removed, so the caller can say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redaction {
    pub text: String,
    pub removed: usize,
}

/// Replaces secret-shaped spans with a fixed mask.
pub fn redact(text: &str) -> Redaction {
    let mut out = text.to_owned();
    let mut removed = 0;
    for pattern in PATTERNS.iter() {
        let count = pattern.find_iter(&out).count();
        if count > 0 {
            out = pattern.replace_all(&out, MASK).into_owned();
            removed += count;
        }
    }
    Redaction { text: out, removed }
}

const MASK: &str = "[redacted]";

/// Shapes that are a secret and almost nothing else.
///
/// A bare hex string is not here: commit hashes and colours look the same.
/// The patterns require a prefix a person would recognise as a credential.
static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // OpenAI, Anthropic, GitHub (classic + fine-grained), Slack, Stripe.
        r"\bsk-[A-Za-z0-9_\-]{16,}",
        r"\bsk-ant-[A-Za-z0-9_\-]{16,}",
        r"\bgh[pousr]_[A-Za-z0-9]{20,}",
        r"\bgithub_pat_[A-Za-z0-9_]{16,}",
        r"\bxox[baprs]-[A-Za-z0-9\-]{10,}",
        r"\b(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{16,}",
        // AWS access key id.
        r"\bAKIA[0-9A-Z]{16}",
        // PEM blocks and JWTs.
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
        // An assignment whose value is long enough to be a token.
        r#"(?i)\b(?:api[_-]?key|token|secret|password|passwd)\b\s*[:=]\s*['"]?[A-Za-z0-9_\-\./+]{12,}"#,
    ]
    .into_iter()
    .map(|p| Regex::new(p).expect("secret pattern compiles"))
    .collect()
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_masked_and_the_sentence_survives() {
        let redacted = redact("deployed with sk-proj-abc1234567890xyz and it worked");
        assert_eq!(redacted.removed, 1);
        assert!(!redacted.text.contains("abc1234567890xyz"));
        assert!(redacted.text.contains("deployed with"));
        assert!(redacted.text.contains("[redacted]"));
    }

    #[test]
    fn a_commit_hash_is_not_a_secret() {
        let hash = "deadbeefcafebabe0123456789abcdef";
        let redacted = redact(&format!("fixed in {hash}"));
        assert_eq!(redacted.removed, 0);
        assert!(redacted.text.contains(hash));
    }

    #[test]
    fn an_assigned_token_is_masked() {
        let redacted = redact("token: supersecretvalue12345");
        assert_eq!(redacted.removed, 1);
        assert!(!redacted.text.contains("supersecretvalue12345"));
    }

    #[test]
    fn prose_about_tokens_passes() {
        let redacted = redact("the token check uses < not <=");
        assert_eq!(redacted.removed, 0);
    }
}
