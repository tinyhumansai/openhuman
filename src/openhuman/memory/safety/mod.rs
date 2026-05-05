//! Secret-detection and redaction helpers for memory writes.
//!
//! This module is intentionally conservative: it prefers false positives over
//! leaking credentials into long-lived memory stores.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

use crate::openhuman::memory::store::types::NamespaceDocumentInput;

const REDACTED_SECRET: &str = "[REDACTED_SECRET]";
const REDACTED_PRIVATE_KEY: &str = "[REDACTED_PRIVATE_KEY]";
const MAX_JSON_SANITIZE_DEPTH: usize = 128;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SanitizationReport {
    pub text_redactions: usize,
    pub key_redactions: usize,
    pub blocked_secret_hits: usize,
    pub depth_redactions: usize,
}

impl SanitizationReport {
    pub fn changed(&self) -> bool {
        self.text_redactions > 0
            || self.key_redactions > 0
            || self.blocked_secret_hits > 0
            || self.depth_redactions > 0
    }

    pub fn merge(self, rhs: Self) -> Self {
        Self {
            text_redactions: self.text_redactions + rhs.text_redactions,
            key_redactions: self.key_redactions + rhs.key_redactions,
            blocked_secret_hits: self.blocked_secret_hits + rhs.blocked_secret_hits,
            depth_redactions: self.depth_redactions + rhs.depth_redactions,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Sanitized<T> {
    pub value: T,
    pub report: SanitizationReport,
}

static BLOCK_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Generic PEM private key blocks, including multiline bodies.
        Regex::new(
            r"(?is)-----BEGIN(?: [A-Z]+)? PRIVATE KEY-----.*?-----END(?: [A-Z]+)? PRIVATE KEY-----",
        )
        .expect("valid private key block"),
        // SSH private key blocks.
        Regex::new(r"(?is)-----BEGIN OPENSSH PRIVATE KEY-----.*?-----END OPENSSH PRIVATE KEY-----")
            .expect("valid openssh private key block"),
        // PGP private key blocks.
        Regex::new(
            r"(?is)-----BEGIN PGP PRIVATE KEY BLOCK-----.*?-----END PGP PRIVATE KEY BLOCK-----",
        )
        .expect("valid pgp private key block"),
    ]
});

static REDACTION_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    vec![
        (
            Regex::new(r"(?i)(bearer\s+)[A-Za-z0-9._~+/=-]{8,}")
                .expect("valid bearer redaction"),
            "${1}[REDACTED]",
        ),
        (
            Regex::new(r#"(?i)(api[_-]?key\s*[=:\s]\s*["']?)[^\s"']+"#)
                .expect("valid api key redaction"),
            "${1}[REDACTED]",
        ),
        (
            Regex::new(
                r#"(?i)\b(token|access[_-]?token|refresh[_-]?token|client[_-]?secret|password|secret)\b\s*[=:\s]\s*["']?[^\s"'&]+"#,
            )
            .expect("valid token redaction"),
            "[REDACTED]",
        ),
        (
            Regex::new(r"\bsk-[A-Za-z0-9]{20,}\b").expect("valid openai key redaction"),
            "[REDACTED]",
        ),
        (
            Regex::new(r"\bgh[pousr]_[A-Za-z0-9_]{20,}\b").expect("valid github token redaction"),
            "[REDACTED]",
        ),
        (
            Regex::new(r"\bAKIA[0-9A-Z]{16}\b").expect("valid aws key redaction"),
            "[REDACTED]",
        ),
        (
            Regex::new(r"\bASIA[0-9A-Z]{16}\b").expect("valid aws sts key redaction"),
            "[REDACTED]",
        ),
        (
            Regex::new(r#"\b(?:aws_)?secret(?:_access)?_key\b\s*[=:\s]\s*["']?[A-Za-z0-9/+=]{16,}"#)
                .expect("valid aws secret key redaction"),
            "[REDACTED]",
        ),
        (
            Regex::new(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9._-]{8,}\.[A-Za-z0-9._-]{8,}\b")
                .expect("valid jwt redaction"),
            "[REDACTED]",
        ),
        (
            // Common OAuth token artifacts in URLs and payloads.
            Regex::new(
                r#"(?i)\b(access_token|refresh_token|id_token|authorization_code|code_verifier|code_challenge)\b\s*[=:\s]\s*["']?[^\s"'&]+"#,
            )
            .expect("valid oauth token redaction"),
            "[REDACTED]",
        ),
        (
            // Google API key pattern.
            Regex::new(r"\bAIza[0-9A-Za-z\-_]{35}\b").expect("valid google api key redaction"),
            "[REDACTED]",
        ),
        (
            // Anthropic key pattern.
            Regex::new(r"\bsk-ant-[A-Za-z0-9\-_]{16,}\b").expect("valid anthropic key redaction"),
            "[REDACTED]",
        ),
        (
            // OpenAI project/org scoped keys and legacy variants.
            Regex::new(r"\bsk-(?:proj|org)-[A-Za-z0-9\-_]{12,}\b")
                .expect("valid openai scoped key redaction"),
            "[REDACTED]",
        ),
        (
            // Stripe secret/restricted keys.
            Regex::new(r"\b(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{16,}\b")
                .expect("valid stripe key redaction"),
            "[REDACTED]",
        ),
        (
            // Slack tokens (bot/user/app/config).
            Regex::new(r"\bxox(?:a|b|p|s|r)-[A-Za-z0-9-]{10,}\b")
                .expect("valid slack token redaction"),
            "[REDACTED]",
        ),
        (
            // GitHub fine-grained/pat/user tokens beyond gh[pousr]_.
            Regex::new(r"\bgithub_pat_[A-Za-z0-9_]{20,}\b")
                .expect("valid github pat redaction"),
            "[REDACTED]",
        ),
        (
            // GitLab personal access token.
            Regex::new(r"\bglpat-[A-Za-z0-9\-_]{16,}\b")
                .expect("valid gitlab pat redaction"),
            "[REDACTED]",
        ),
        (
            // NPM auth token.
            Regex::new(r"\bnpm_[A-Za-z0-9]{20,}\b").expect("valid npm token redaction"),
            "[REDACTED]",
        ),
        (
            // SendGrid API key.
            Regex::new(r"\bSG\.[A-Za-z0-9_\-]{16,}\.[A-Za-z0-9_\-]{16,}\b")
                .expect("valid sendgrid key redaction"),
            "[REDACTED]",
        ),
        (
            // Twilio API key SID.
            Regex::new(r"\bSK[a-fA-F0-9]{32}\b").expect("valid twilio sid redaction"),
            "[REDACTED]",
        ),
        (
            // Azure Storage account key in key=value style payloads.
            Regex::new(r"(?i)\bAccountKey\b\s*=\s*[A-Za-z0-9+/=]{20,}")
                .expect("valid azure account key redaction"),
            "[REDACTED]",
        ),
        (
            // Generic Authorization header values beyond Bearer.
            Regex::new(r"(?i)(authorization\s*[:=]\s*)(?:basic|bearer|token)\s+[A-Za-z0-9._~+/=-]{8,}")
                .expect("valid authorization header redaction"),
            "${1}[REDACTED]",
        ),
    ]
});

pub fn has_likely_secret(value: &str) -> bool {
    if BLOCK_PATTERNS.iter().any(|pattern| pattern.is_match(value)) {
        return true;
    }
    REDACTION_PATTERNS
        .iter()
        .any(|(pattern, _)| pattern.is_match(value))
}

pub fn sanitize_text(value: &str) -> Sanitized<String> {
    let mut out = value.to_string();
    let mut report = SanitizationReport::default();

    for pattern in BLOCK_PATTERNS.iter() {
        let hits = pattern.find_iter(&out).count();
        if hits > 0 {
            report.blocked_secret_hits += hits;
            out = pattern.replace_all(&out, REDACTED_PRIVATE_KEY).into_owned();
        }
    }

    for (pattern, replacement) in REDACTION_PATTERNS.iter() {
        let hits = pattern.find_iter(&out).count();
        if hits > 0 {
            report.text_redactions += hits;
            out = pattern.replace_all(&out, *replacement).into_owned();
        }
    }

    Sanitized { value: out, report }
}

pub fn sanitize_json(value: &Value) -> Sanitized<Value> {
    sanitize_json_inner(value, 0)
}

pub fn sanitize_document_input(input: NamespaceDocumentInput) -> Sanitized<NamespaceDocumentInput> {
    let mut report = SanitizationReport::default();

    let title = sanitize_text(&input.title);
    report = report.merge(title.report);
    let content = sanitize_text(&input.content);
    report = report.merge(content.report);

    let mut tags = Vec::with_capacity(input.tags.len());
    for tag in input.tags {
        let sanitized = sanitize_text(&tag);
        report = report.merge(sanitized.report);
        tags.push(sanitized.value);
    }

    let metadata = sanitize_json(&input.metadata);
    report = report.merge(metadata.report);

    Sanitized {
        value: NamespaceDocumentInput {
            namespace: input.namespace,
            key: input.key,
            title: title.value,
            content: content.value,
            source_type: input.source_type,
            priority: input.priority,
            tags,
            metadata: metadata.value,
            category: input.category,
            session_id: input.session_id,
            document_id: input.document_id,
        },
        report,
    }
}

fn sanitize_json_inner(value: &Value, depth: usize) -> Sanitized<Value> {
    if depth >= MAX_JSON_SANITIZE_DEPTH {
        return Sanitized {
            value: Value::String(REDACTED_SECRET.to_string()),
            report: SanitizationReport {
                depth_redactions: 1,
                ..SanitizationReport::default()
            },
        };
    }

    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            let mut report = SanitizationReport::default();
            for (key, value) in map {
                if is_sensitive_key(key) {
                    report.key_redactions += 1;
                    out.insert(key.clone(), Value::String(REDACTED_SECRET.to_string()));
                    continue;
                }
                let sanitized = sanitize_json_inner(value, depth + 1);
                report = report.merge(sanitized.report);
                out.insert(key.clone(), sanitized.value);
            }
            Sanitized {
                value: Value::Object(out),
                report,
            }
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            let mut report = SanitizationReport::default();
            for item in items {
                let sanitized = sanitize_json_inner(item, depth + 1);
                report = report.merge(sanitized.report);
                out.push(sanitized.value);
            }
            Sanitized {
                value: Value::Array(out),
                report,
            }
        }
        Value::String(value) => {
            let sanitized = sanitize_text(value);
            Sanitized {
                value: Value::String(sanitized.value),
                report: sanitized.report,
            }
        }
        _ => Sanitized {
            value: value.clone(),
            report: SanitizationReport::default(),
        },
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();

    matches!(
        normalized.as_str(),
        "apikey"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "authorization"
            | "password"
            | "secret"
            | "clientsecret"
    ) || normalized.ends_with("token")
        || normalized.ends_with("apikey")
        || normalized.ends_with("clientsecret")
        || normalized.contains("password")
        || normalized.contains("secret")
        || normalized.ends_with("key")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitize_text_redacts_bearer_and_openai_key() {
        let input = "Authorization: Bearer abcdefghijklmnop and sk-1234567890123456789012345";
        let sanitized = sanitize_text(input);
        assert!(sanitized.value.contains("Bearer [REDACTED]"));
        assert!(!sanitized.value.contains("sk-1234567890123456789012345"));
        assert!(sanitized.report.text_redactions >= 2);
    }

    #[test]
    fn sanitize_text_blocks_private_key_blocks() {
        let input = "-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----";
        let sanitized = sanitize_text(input);
        assert!(sanitized.value.contains(REDACTED_PRIVATE_KEY));
        assert!(sanitized.report.blocked_secret_hits >= 1);
    }

    #[test]
    fn sanitize_json_redacts_sensitive_keys_and_nested_strings() {
        let input = json!({
            "token": "abc123",
            "nested": {
                "notes": "Bearer supersecretvalue",
                "ok": "hello"
            },
            "arr": ["sk-1234567890123456789012345", "safe"]
        });

        let sanitized = sanitize_json(&input);
        assert_eq!(sanitized.value["token"], json!(REDACTED_SECRET));
        assert_eq!(sanitized.value["nested"]["ok"], json!("hello"));
        assert!(sanitized.value["nested"]["notes"]
            .as_str()
            .unwrap_or_default()
            .contains("[REDACTED]"));
        assert!(sanitized.report.key_redactions >= 1);
        assert!(sanitized.report.text_redactions >= 2);
    }

    #[test]
    fn sanitize_json_redacts_common_sensitive_key_variants() {
        let input = json!({
            "db_password": "p@ss",
            "secret_key": "abc123",
            "api_secret": "def456",
            "monkey": "banana"
        });

        let sanitized = sanitize_json(&input);
        assert_eq!(sanitized.value["db_password"], json!(REDACTED_SECRET));
        assert_eq!(sanitized.value["secret_key"], json!(REDACTED_SECRET));
        assert_eq!(sanitized.value["api_secret"], json!(REDACTED_SECRET));
        assert_eq!(sanitized.value["monkey"], json!(REDACTED_SECRET));
        assert!(sanitized.report.key_redactions >= 4);
    }

    #[test]
    fn has_likely_secret_detects_common_patterns() {
        assert!(has_likely_secret("api_key=abc123"));
        assert!(has_likely_secret("Bearer abcdefghijklmnopqrstuvwxyz"));
        assert!(has_likely_secret("xoxb-1234567890-abcdef-ghijklmnop"));
        assert!(has_likely_secret("glpat-aaaaaaaaaaaaaaaaaaaa"));
        assert!(has_likely_secret("SG.aaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbb"));
        assert!(!has_likely_secret("I prefer rust"));
    }

    #[test]
    fn sanitize_text_redacts_more_provider_secrets() {
        let input = "auth=Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ== stripe=sk_live_12345678901234567890 npm=npm_abcdefghijklmnopqrstuvwxyz";
        let sanitized = sanitize_text(input);
        assert!(!sanitized.value.contains("sk_live_12345678901234567890"));
        assert!(!sanitized.value.contains("npm_abcdefghijklmnopqrstuvwxyz"));
        assert!(sanitized.value.contains("[REDACTED]"));
        assert!(sanitized.report.text_redactions >= 2);
    }

    #[test]
    fn sanitize_text_redacts_oauth_url_style_params() {
        let input = "https://example.com/callback?access_token=abcd1234&refresh_token=efgh5678&id_token=jwt";
        let sanitized = sanitize_text(input);
        assert!(!sanitized.value.contains("abcd1234"));
        assert!(!sanitized.value.contains("efgh5678"));
        assert!(!sanitized.value.contains("id_token=jwt"));
        assert!(sanitized.report.text_redactions >= 3);
    }

    #[test]
    fn sanitize_text_redacts_multiline_private_key_blocks() {
        let input = "BEGIN\n-----BEGIN OPENSSH PRIVATE KEY-----\nline1\nline2\n-----END OPENSSH PRIVATE KEY-----\nEND";
        let sanitized = sanitize_text(input);
        assert!(!sanitized.value.contains("OPENSSH PRIVATE KEY"));
        assert!(sanitized.value.contains(REDACTED_PRIVATE_KEY));
        assert!(sanitized.report.blocked_secret_hits >= 1);
    }

    #[test]
    fn sanitize_json_redacts_values_beyond_max_depth() {
        let mut nested = json!("leaf");
        for _ in 0..(MAX_JSON_SANITIZE_DEPTH + 2) {
            nested = json!({ "nested": nested });
        }

        let sanitized = sanitize_json(&nested);
        assert!(sanitized.report.depth_redactions >= 1);
        assert!(sanitized
            .value
            .to_string()
            .contains(&format!("\"{REDACTED_SECRET}\"")));
    }
}
