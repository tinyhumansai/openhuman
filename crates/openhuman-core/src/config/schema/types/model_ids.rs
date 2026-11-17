//! Provider-neutral workload roles used by inference routing.

/// Public, unprefixed workload roles understood by the generic inference router.
///
/// Keep this config/API view in canonical order. The adjacent regression test
/// ties it to TinyInference's native `WORKLOAD_ROUTE_TIERS` so the host does
/// not silently grow a second routing vocabulary.
pub const WORKLOAD_ROLES: [&str; 7] = [
    "chat",
    "reasoning",
    "agentic",
    "coding",
    "burst",
    "summarization",
    "vision",
];
