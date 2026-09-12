//! Trust-tier helpers for memory entries surfaced into agent prompts.
//!
//! Memory entries reach the agent prompt by way of vector-recall over the
//! full memory store, which mixes content from many provenance tiers:
//!
//! - **User-authored** turns from the same chat (high trust).
//! - **Agent-authored** summaries and working-memory snapshots (high trust).
//! - **Connector-synced** content harvested from Gmail / Slack / Notion /
//!   Discord / web feeds (untrusted: anything in the body of an email, the
//!   text of a Slack DM, or a Notion page is text the agent has no a-priori
//!   reason to obey).
//!
//! Recall returns the same shape regardless of which tier the row came
//! from, so a prompt-injection paragraph that lives inside an inbound
//! email reaches the agent's working context with the same visual weight
//! as a system-issued instruction. This module is the narrowest possible
//! mitigation: a heuristic that flags potentially-untrusted entries by
//! namespace / key shape, and a wrapping helper that surrounds the entry
//! with explicit `<untrusted-source>` markers so the safety preamble and
//! the model itself have a fighting chance of distinguishing context from
//! instructions.
//!
//! A proper fix is a typed `Provenance` enum carried on every memory row,
//! populated by the ingestion pipeline. That requires a schema migration
//! across `MemoryEntry`, the SQLite store, and every namespace creator —
//! out of scope for this commit. The heuristics here intentionally err
//! toward over-wrapping: it is safer to tag a user-authored row as
//! untrusted than to leave a connector-synced one bare.

/// Conservative classifier — returns `true` when the entry is unlikely to
/// be locally-authored and therefore SHOULD be wrapped before reaching
/// the agent prompt.
///
/// Rules (any match flips to untrusted):
/// - Namespace exists and is not one of the local-authored short-list
///   (`working`, `agent`, `local`, `core`, `global`, `default`, or the
///   ingestion-internal `tree.*` namespaces that are summarised locally).
/// - Key carries a known connector prefix (`chat:`, `email:`, `notion:`,
///   `drive:`, `discord:`, `telegram:`, `whatsapp:`, `slack:`, `gmail:`,
///   `outlook:`, `imap:`, `meeting:`, `web:`).
///
/// Local-authored namespaces are an allowlist so an unrecognised namespace
/// surfaces as "untrusted" (default-deny). The mitigation is conservative
/// on purpose; refining it requires explicit provenance tagging at
/// ingest time.
/// Takes the two fields it reads rather than a `MemoryEntry`.
///
/// There are two `MemoryEntry` types in play during the module port — the
/// engine's and the contract's — and this predicate needs neither: it reads a
/// namespace and a key. Taking them directly means callers on either side can
/// use it without a conversion, and the signature says what it actually
/// depends on.
pub fn is_potentially_untrusted(namespace: Option<&str>, key: &str) -> bool {
    if let Some(ns) = namespace {
        let ns = ns.trim().to_ascii_lowercase();
        if !is_locally_authored_namespace(&ns) {
            return true;
        }
    }

    let key_lower = key.to_ascii_lowercase();
    let connector_prefixes: &[&str] = &[
        "chat:",
        "email:",
        "notion:",
        "drive:",
        "discord:",
        "telegram:",
        "whatsapp:",
        "slack:",
        "gmail:",
        "outlook:",
        "imap:",
        "meeting:",
        "web:",
    ];
    connector_prefixes.iter().any(|p| key_lower.starts_with(p))
}

fn is_locally_authored_namespace(ns: &str) -> bool {
    // Exact-match short list — everything else (including ingestion-derived
    // namespaces) is treated as untrusted by default.
    matches!(
        ns,
        "working" | "agent" | "local" | "core" | "global" | "default" | "user"
    ) || ns.starts_with("working.")
        || ns.starts_with("agent.")
        || ns.starts_with("tree.")
}

/// Wrap `content` in explicit untrusted-source markers so the agent
/// prompt visually distinguishes it from system instructions.
///
/// `source_hint` is a short, human-readable hint (`"gmail"`, `"slack"`,
/// `"connector"`, `"recall"`, …) that lands in the tag attributes so the
/// model can see which surface produced the row without revealing
/// content that should not leave the trust boundary.
///
/// Both `source_hint` and `content` are sanitised before they reach the
/// formatted string — without sanitisation a payload containing a
/// literal `</untrusted-source>` or stray quote could close or forge
/// the marker and slip back into the trusted region.
pub fn wrap_untrusted_for_agent(content: &str, source_hint: &str) -> String {
    let hint = sanitize_source_hint(source_hint);
    let safe_content = escape_untrusted_content(content);
    format!("<untrusted-source source=\"{hint}\">\n{safe_content}\n</untrusted-source>")
}

/// Strip the `source_hint` to a short identifier-shaped string so it can
/// land directly in the tag attribute without escaping. Drops anything
/// that is not ASCII alphanumeric or a small set of safe punctuation,
/// caps the length at 64 chars, and falls back to `"external"` when the
/// hint is empty after cleaning.
fn sanitize_source_hint(source_hint: &str) -> String {
    let cleaned: String = source_hint
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "external".to_string()
    } else {
        cleaned
    }
}

/// Neutralise the three HTML-ish characters that would otherwise let an
/// embedded payload break out of the `<untrusted-source>` block. Keeps
/// the substitution table tiny on purpose — we only need to prevent the
/// marker from being terminated or new attributes from being injected.
fn escape_untrusted_content(content: &str) -> String {
    content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
#[path = "memory_context_safety_tests.rs"]
mod tests;
