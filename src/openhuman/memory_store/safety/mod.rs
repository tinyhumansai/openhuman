//! Secret-detection and redaction for memory writes — thin host shim over
//! `tinycortex::memory::store::safety` (W3).
//!
//! The conservative secret + PII scrubbers (`has_likely_secret`,
//! `has_likely_pii`, `sanitize_text`, `sanitize_json`) + the
//! `SanitizationReport`/`Sanitized<T>` types are the crate's — now including the
//! full multilingual national-ID PII module (ported into the crate so the crate
//! `sanitize_text` matches this host's byte-for-byte). The host keeps only
//! [`sanitize_document_input`], which scrubs the host-specific
//! [`NamespaceDocumentInput`] shape by delegating each field to the crate
//! scrubbers. The retained test suite doubles as a byte-parity guard: it asserts
//! the crate scrubber still redacts every secret/PII pattern the host relied on.

pub mod pii;

use crate::openhuman::memory_store::types::NamespaceDocumentInput;

pub use tinycortex::memory::store::safety::{
    has_likely_pii, has_likely_secret, sanitize_json, sanitize_text, SanitizationReport, Sanitized,
};

/// Scrub a namespace-document input, field by field, via the crate scrubbers.
///
/// Sanitization is content-cleaning only; provenance `taint` survives untouched
/// so the write gate's taint check still sees the real source signal.
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
            taint: input.taint,
        },
        report,
    }
}

#[cfg(test)]
mod tests {
    //! Byte-parity guard over the crate scrubber: every secret/PII pattern the
    //! host used to redact must still be redacted after the port.
    use super::*;
    use serde_json::json;

    const REDACTED_SECRET: &str = "[REDACTED_SECRET]";
    const REDACTED_PRIVATE_KEY: &str = "[REDACTED_PRIVATE_KEY]";
    const MAX_JSON_SANITIZE_DEPTH: usize = 128;

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
            "nested": { "notes": "Bearer supersecretvalue", "ok": "hello" },
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
            "db_password": "p@ss", "secret_key": "abc123",
            "api_secret": "def456", "monkey": "banana"
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
    fn sanitize_text_also_redacts_pii_after_secrets() {
        let input = "Token sk-abcdefghijklmnopqrstuvwxyz; CPF 111.444.777-35; phone +15551234567";
        let sanitized = sanitize_text(input);
        assert!(!sanitized.value.contains("sk-abcdefghijklmnopqrstuvwxyz"));
        assert!(!sanitized.value.contains("111.444.777-35"));
        assert!(!sanitized.value.contains("+15551234567"));
        assert!(sanitized.value.contains("[REDACTED_PII_CPF]"));
        assert!(sanitized.value.contains("[REDACTED_PII_PHONE]"));
        assert!(sanitized.report.text_redactions >= 1);
        assert_eq!(sanitized.report.pii_redactions, 2);
    }

    #[test]
    fn sanitize_json_propagates_pii_redaction_into_nested_strings() {
        let input = json!({
            "note": "Cliente RFC VECJ880326XK4 confirmado",
            "meta": { "cuit": "20-11111111-2" }
        });
        let sanitized = sanitize_json(&input);
        assert!(sanitized.value["note"]
            .as_str()
            .unwrap_or_default()
            .contains("[REDACTED_PII_RFC]"));
        assert!(sanitized.value["meta"]["cuit"]
            .as_str()
            .unwrap_or_default()
            .contains("[REDACTED_PII_CUIT]"));
        assert!(sanitized.report.pii_redactions >= 2);
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

    #[test]
    fn sanitize_document_input_preserves_taint() {
        let input = NamespaceDocumentInput {
            namespace: "ns".into(),
            key: "k".into(),
            title: "Bearer secret123456789 visible title".into(),
            content: "content with sk-abcdefghijklmnopqrstuvwxyz".into(),
            source_type: "sync".into(),
            priority: "normal".into(),
            tags: vec!["tag1".into()],
            metadata: json!({"safe": "value"}),
            category: "core".into(),
            session_id: None,
            document_id: None,
            taint: crate::openhuman::memory::MemoryTaint::ExternalSync,
        };
        let sanitized = sanitize_document_input(input);
        assert_eq!(
            sanitized.value.taint,
            crate::openhuman::memory::MemoryTaint::ExternalSync,
            "taint must survive sanitization unchanged"
        );
        assert!(sanitized.report.text_redactions >= 1);
    }
}
