//! System-prompt cache-alignment detector (warn-only).
//!
//! Clean-room port of headroom's `CacheAligner` (Apache-2.0) in its
//! **detector-only** form: it never mutates the prompt (that would itself bust
//! the cache prefix). It scans the cache-hot zone — the system prompt — for
//! *volatile* tokens that change every launch and therefore silently prevent
//! the provider KV-cache prefix from hitting:
//!
//! - UUIDs (canonical 36-char form)
//! - ISO-8601 timestamps
//! - JWTs (three base64url segments)
//! - hex hashes (MD5/SHA1/SHA256 lengths)
//!
//! When any are found it emits one warning log line so the volatility is
//! visible. OpenHuman already takes care to keep the system prompt stable for
//! KV-cache reuse (see `with_openhuman_thread_id` and the delegation-refresh
//! "system prompt unchanged for KV cache" path); this is the diagnostic that
//! catches regressions where dynamic content leaks back into the prefix.

/// One detected volatile token: its kind and a short redacted sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolatileFinding {
    pub kind: &'static str,
    pub sample: String,
}

/// Scan a system prompt for volatile tokens. Returns the findings (empty when
/// the prefix is stable). Pure — callers decide whether/how to log.
pub fn detect_volatile(system_prompt: &str) -> Vec<VolatileFinding> {
    let mut findings = Vec::new();
    // Delimiter = anything that can't appear inside the tokens we detect.
    // Allowed inner chars: alphanumerics plus `- . : _` (UUID dashes, ISO
    // timestamp `-`/`:`, JWT `.`/`-`/`_`). So `session=<uuid>` splits cleanly.
    for tok in system_prompt
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_')))
    {
        if tok.len() < 8 {
            continue;
        }
        if is_uuid(tok) {
            findings.push(VolatileFinding {
                kind: "uuid",
                sample: redact(tok),
            });
        } else if is_jwt(tok) {
            findings.push(VolatileFinding {
                kind: "jwt",
                sample: redact(tok),
            });
        } else if is_iso8601(tok) {
            findings.push(VolatileFinding {
                kind: "iso8601",
                sample: tok.to_string(),
            });
        } else if is_hex_hash(tok) {
            findings.push(VolatileFinding {
                kind: "hex_hash",
                sample: redact(tok),
            });
        }
    }
    findings
}

/// Detect volatile tokens and, if any, emit a single warning log line.
/// Returns the number of findings (0 when the prefix is clean).
pub fn warn_if_volatile(system_prompt: &str) -> usize {
    let findings = detect_volatile(system_prompt);
    if !findings.is_empty() {
        let mut kinds: Vec<&str> = findings.iter().map(|f| f.kind).collect();
        kinds.sort_unstable();
        kinds.dedup();
        ::log::warn!(
            "[compaction][cache-align] system prompt contains {} volatile token(s) ({}) — KV-cache prefix may not hit; keep dynamic content out of the system prompt",
            findings.len(),
            kinds.join(", "),
        );
    }
    findings.len()
}

fn redact(tok: &str) -> String {
    let head: String = tok.chars().take(4).collect();
    format!("{head}…")
}

/// Canonical RFC-4122 UUID: 8-4-4-4-12 hex with dashes (36 chars). The dashless
/// 32-char form is deliberately *not* accepted — it's structurally identical to
/// an MD5 digest and would mis-classify.
fn is_uuid(tok: &str) -> bool {
    if tok.len() != 36 {
        return false;
    }
    let bytes = tok.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        let expect_dash = matches!(i, 8 | 13 | 18 | 23);
        if expect_dash {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// JWT shape: three base64url segments joined by `.`, each non-trivial. We do
/// not verify the signature — only the structure.
fn is_jwt(tok: &str) -> bool {
    let segs: Vec<&str> = tok.split('.').collect();
    if segs.len() != 3 {
        return false;
    }
    segs.iter().all(|s| {
        s.len() >= 4
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }) && tok.starts_with("ey")
}

/// Hex hash: all hex digits, length 32 (MD5) / 40 (SHA1) / 64 (SHA256).
fn is_hex_hash(tok: &str) -> bool {
    matches!(tok.len(), 32 | 40 | 64) && tok.bytes().all(|b| b.is_ascii_hexdigit())
}

/// ISO-8601-ish timestamp: `YYYY-MM-DDThh:mm:ss` (or a space separator). Every
/// numeric position is validated so junk like `2026-aa-bbTcc:dd:ee` is rejected.
fn is_iso8601(tok: &str) -> bool {
    let b = tok.as_bytes();
    if tok.len() < 19 {
        return false;
    }
    let digit = |i: usize| b[i].is_ascii_digit();
    digit(0)
        && digit(1)
        && digit(2)
        && digit(3)
        && b[4] == b'-'
        && digit(5)
        && digit(6)
        && b[7] == b'-'
        && digit(8)
        && digit(9)
        && (b[10] == b'T' || b[10] == b' ')
        && digit(11)
        && digit(12)
        && b[13] == b':'
        && digit(14)
        && digit(15)
        && b[16] == b':'
        && digit(17)
        && digit(18)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_uuid_and_timestamp() {
        let prompt = "You are an agent. session=550e8400-e29b-41d4-a716-446655440000 started 2026-06-19T15:08:00";
        let f = detect_volatile(prompt);
        assert!(f.iter().any(|x| x.kind == "uuid"), "{f:?}");
        assert!(f.iter().any(|x| x.kind == "iso8601"), "{f:?}");
    }

    #[test]
    fn detects_hash_and_jwt() {
        let prompt = "commit d41d8cd98f00b204e9800998ecf8427e token eyJhbGc.eyJzdWIi.SflKxwRJ here";
        let f = detect_volatile(prompt);
        assert!(f.iter().any(|x| x.kind == "hex_hash"), "{f:?}");
        assert!(f.iter().any(|x| x.kind == "jwt"), "{f:?}");
    }

    #[test]
    fn iso8601_rejects_non_numeric_lookalikes() {
        // Shape matches but the fields aren't digits — must not be flagged.
        let f = detect_volatile("ref 2026-aa-bbTcc:dd:ee here");
        assert!(!f.iter().any(|x| x.kind == "iso8601"), "{f:?}");
        // A real timestamp still flags.
        let g = detect_volatile("at 2026-06-21T12:34:56 today");
        assert!(g.iter().any(|x| x.kind == "iso8601"), "{g:?}");
    }

    #[test]
    fn stable_prompt_is_clean() {
        let prompt = "You are a helpful assistant. Be concise. Use the tools provided.";
        assert!(detect_volatile(prompt).is_empty());
        assert_eq!(warn_if_volatile(prompt), 0);
    }

    #[test]
    fn dashless_md5_not_uuid() {
        // 32-char hex must classify as hex_hash, never uuid.
        let f = detect_volatile("x d41d8cd98f00b204e9800998ecf8427e y");
        assert!(f.iter().any(|x| x.kind == "hex_hash"));
        assert!(!f.iter().any(|x| x.kind == "uuid"));
    }
}
