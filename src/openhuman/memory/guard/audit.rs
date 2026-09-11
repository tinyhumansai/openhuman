//! Step 7: the guard's tracing span and its audit event.
//!
//! ## What may be logged, and what may never be
//!
//! Memory content is the most sensitive data in the product. Nothing in this
//! module — span field, log line, or bus event — carries a memory body, a
//! recall query, a namespace *key*, or a document title. What it carries is
//! **shapes**: the driver id, the contract method, the namespace, char counts,
//! and hit counts. A namespace is an operator-chosen bucket name and is already
//! logged verbatim by the embedded driver; a key is caller data and is not.
//!
//! That is also why this module publishes a purpose-built
//! [`DomainEvent::MemoryGuardDenied`] rather than reusing
//! [`DomainEvent::MemoryRecalled`], which carries the raw query string. Firing
//! that one from the guard would push user query text onto the bus on every
//! call.
//!
//! Where an identifier genuinely helps correlate a report — a key, a source id
//! — use [`redact`], which returns an 8-hex-char SHA-256 prefix: stable enough
//! to correlate two log lines, useless for recovering the value. Note that
//! [`redact`] is a **log** redactor only; the content-side scrubber for egress
//! is `memory::safety::sanitize_text`, selected in
//! [`GuardPolicy::redact_outbound`](super::GuardPolicy::redact_outbound).
//!
//! ## Denials only on the bus
//!
//! Success is logged (at `debug`, into the file-only core log) but is **not**
//! published. One bus event per memory read would flood every subscriber on the
//! hot path for no operator benefit; a refusal is the rare, actionable event.

use crate::openhuman::memory::api::capabilities::Capability;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::util::redact::redact;

use super::policy::GuardPolicy;

/// Grep prefix for every guard log line, matching `[memory:binding]` and
/// `[memory:driver:embedded]`.
pub const LOG_PREFIX: &str = "[memory:guard]";

/// The tracing span every guarded call runs inside.
///
/// Carries the three correlation fields the spec asks for — `driver_id`, the
/// capability family, and the namespace — plus the contract method, so two
/// calls into the same family are distinguishable in a trace.
pub fn guard_span(
    policy: &GuardPolicy,
    capability: Capability,
    method: &str,
    namespace: &str,
) -> tracing::Span {
    tracing::debug_span!(
        "memory_guard",
        driver_id = policy.driver_id(),
        capability = capability.as_str(),
        method = method,
        namespace = namespace,
    )
}

/// Namespace placeholder for the contract methods that address no namespace
/// (maintenance, portability, goals). Better than an empty field, which reads
/// as "the namespace was lost".
pub const NO_NAMESPACE: &str = "-";

/// Log a call the guard let through. Shapes only.
pub fn trace_allowed(policy: &GuardPolicy, method: &str, namespace: &str, chars: usize) {
    log::debug!(
        "{LOG_PREFIX} allowed driver={} class={} method={method} namespace={namespace} \
         content_chars={chars}",
        policy.driver_id(),
        policy.class(),
    );
}

/// Log the effect of a budget, when it actually bit. Silent when it did not, so
/// the log is a record of truncation rather than a per-call heartbeat.
pub fn trace_budget(policy: &GuardPolicy, method: &str, dropped: usize, trimmed_chars: usize) {
    if dropped == 0 && trimmed_chars == 0 {
        return;
    }
    log::debug!(
        "{LOG_PREFIX} budget applied driver={} method={method} dropped={dropped} \
         trimmed_chars={trimmed_chars}",
        policy.driver_id(),
    );
}

/// Log and publish a refusal.
///
/// Called from [`GuardPolicy::denied`](super::GuardPolicy::denied), so every
/// deny path audits by construction rather than by each call site remembering
/// to. `publish_global` is synchronous and a no-op before the bus is
/// initialised, so this is safe pre-boot with no `#[cfg(test)]` guard — the
/// same property `binding::build` relies on.
pub fn publish_guard_denied(policy: &GuardPolicy, method: &str, reason: &str) {
    log::warn!(
        "{LOG_PREFIX} DENIED driver={} class={} method={method}: {reason}",
        policy.driver_id(),
        policy.class(),
    );
    BUS.publish(DomainEvent::MemoryGuardDenied {
        driver_id: policy.driver_id().to_string(),
        method: method.to_string(),
        reason: reason.to_string(),
    });
}

/// A caller-supplied identifier in a form that is safe to log: an 8-hex-char
/// digest, never the value. Use for keys, node ids, and source ids when a log
/// line genuinely needs to correlate two calls.
pub fn correlate(value: &str) -> String {
    redact(value)
}
