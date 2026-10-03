//! Secret and PII scrubbing under the host's policy.
//!
//! The scrubbers are TinyMemory's (`tinymemory::safety`); this module pins the
//! policy every caller in this process applies, so a memory write, an approval
//! record and an offloaded artifact all redact the same way. The host policy
//! is [`tinymemory::safety::Policy::corroborated`]: a bare digit run must look
//! like a card (issuer prefix or nearby keyword) before it is redacted, so
//! timestamps and ids survive.

use serde_json::Value;
use tinymemory::safety::{Policy, Sanitized};

pub use tinymemory::safety::has_likely_secret;

/// The scrubbing policy every caller in this process applies.
#[must_use]
pub const fn host_policy() -> Policy {
    Policy::corroborated()
}

/// Redacts secrets and PII in `value`.
#[must_use]
pub fn sanitize_text(value: &str) -> Sanitized<String> {
    tinymemory::safety::sanitize_text_with(value, host_policy())
}

/// Redacts secrets and PII in a JSON value, dropping sensitive keys.
#[must_use]
pub fn sanitize_json(value: &Value) -> Sanitized<Value> {
    tinymemory::safety::sanitize_json_with(value, host_policy())
}

#[cfg(test)]
#[path = "scrub_tests.rs"]
mod tests;
