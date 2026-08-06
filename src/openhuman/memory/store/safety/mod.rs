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

use crate::openhuman::memory::store::types::NamespaceDocumentInput;

pub use tinycortex::memory::store::safety::{
    has_likely_pii, has_likely_secret, sanitize_json, sanitize_text, SanitizationReport, Sanitized,
};

/// Canonical storage form of a caller-supplied memory **identifier** — a
/// namespace, a document key, or a KV key.
///
/// An identifier is an address, not content: whatever this returns is what the
/// row is stored under, so every read / update / delete that addresses a row by
/// identifier has to canonicalize through this same function, or it looks up a
/// row the write never created (#5164).
///
/// Two properties make that safe, and both follow the split the crate's PII
/// module documents between its **strict boundary predicate** and its **lenient
/// content scrubber**:
///
/// * **Strict gating.** Only identifiers that trip [`has_likely_pii`] —
///   formatted / keyword-gated national IDs (`ssn-123-45-6789`,
///   `cliente-RFC-VECJ880326XK4`, `cuit-20-11111111-2`) — are rewritten.
///   `redact_pii` on its own also rewrites bare digit-run shapes, and the
///   scanners legitimately build identifiers out of those: WhatsApp JIDs
///   (`12025551234-1543890267@g.us`), iMessage `+1…` chat ids, millisecond
///   timestamps, padded counters. Rewriting those maps two distinct contacts
///   onto one `(namespace, key)`, where the upsert's `ON CONFLICT … DO UPDATE`
///   has one contact's document silently overwrite the other's.
/// * **Idempotence.** The `[REDACTED_PII_*]` placeholders carry no PII pattern
///   of their own, so canonicalizing an already-canonical identifier is a
///   no-op — which is what lets read paths canonicalize unconditionally.
pub fn canonical_identifier(value: &str) -> String {
    if !has_likely_pii(value) {
        return value.to_string();
    }
    pii::redact_pii(value).value
}

/// Canonical storage form of a document key: the exact transform
/// `upsert_document` / `upsert_document_metadata_only` apply before writing the
/// `memory_docs.key` column (trim, then [`canonical_identifier`]).
///
/// Single-sourced so the by-key read paths (`Memory::get`, `Memory::forget`)
/// cannot drift from the write path. Drift there is invisible — the lookup
/// simply misses, the caller treats the row as absent and writes again, which
/// is the unthrottled loop #5164 was reported for.
pub fn canonical_document_key(key: &str) -> String {
    canonical_identifier(key.trim())
}

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

    /// #5164: identifiers are storage addresses, so canonicalization follows
    /// the **strict** boundary predicate. Formatted / keyword-gated national IDs
    /// are rewritten; the bare digit-run shapes the scanners build identifiers
    /// out of are left alone (rewriting those maps distinct contacts onto one
    /// `(namespace, key)` and the upsert silently overwrites).
    #[test]
    fn canonical_identifier_rewrites_only_strict_pii() {
        for identifier in [
            "ssn-123-45-6789",
            "cliente-RFC-VECJ880326XK4",
            "cuit-20-11111111-2",
            "user/111.444.777-35",
        ] {
            let canonical = canonical_identifier(identifier);
            assert_ne!(
                canonical, identifier,
                "strict PII identifier must be canonicalized: {identifier}"
            );
            assert!(
                canonical.contains("[REDACTED_PII_"),
                "expected a redaction placeholder, got: {canonical}"
            );
        }

        for identifier in [
            // WhatsApp group JID / 1:1 JID / broadcast, iMessage E.164 chat id,
            // telegram numeric peer id, padded ms timestamp, plain namespaces.
            "12025551234-1543890267@g.us:2026-05-30",
            "12025551234@c.us:2026-05-30",
            "imessage:+12025551234:2026-05-30",
            "4123456789:2026-05-30",
            "accepted:000001747729035001",
            "memory/global/preferences",
            "skill-gmail",
        ] {
            assert_eq!(
                canonical_identifier(identifier),
                identifier,
                "scanner-built identifier must keep its identity: {identifier}"
            );
        }
    }

    /// Read paths canonicalize unconditionally, so the transform has to be a
    /// fixed point on its own output.
    #[test]
    fn canonical_identifier_is_idempotent() {
        for identifier in ["ssn-123-45-6789", "cliente-RFC-VECJ880326XK4", "safe-key"] {
            let once = canonical_identifier(identifier);
            assert_eq!(canonical_identifier(&once), once, "not idempotent: {once}");
        }
    }

    /// `canonical_document_key` single-sources the write-path transform, trim
    /// included — otherwise `Memory::get` would address an untrimmed key that
    /// `upsert_document` never wrote.
    #[test]
    fn canonical_document_key_trims_before_canonicalizing() {
        assert_eq!(canonical_document_key("  doc-a  "), "doc-a");
        assert_eq!(
            canonical_document_key("  ssn-123-45-6789  "),
            canonical_identifier("ssn-123-45-6789")
        );
        assert_eq!(canonical_document_key("   "), "");
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
