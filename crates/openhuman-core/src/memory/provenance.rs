//! OpenHuman's connector vocabulary for TinyMemory provenance classification.
//!
//! Classification and escaped prompt markers belong to `tinymemory_api`.
//! This host supplies only the key prefixes used by its ingestion adapters.

/// Builds the memory provenance policy with OpenHuman's connector prefixes.
pub fn connector_provenance_policy() -> tinymemory_api::ProvenancePolicy {
    let connector_prefixes = &[
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
    tinymemory_api::ProvenancePolicy::new(connector_prefixes)
}

#[cfg(test)]
#[path = "provenance_tests.rs"]
mod tests;
