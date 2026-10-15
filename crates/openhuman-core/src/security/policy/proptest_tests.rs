//! Property-based tests for the path check's adversarial-input surface
//! (plan.md §6.3 — proptest was previously absent). Paths arrive as fully
//! untrusted strings from the LLM/user, so the central property is "never
//! panics on any input". The command classifier these once also covered is the
//! TinySecurity module's now, and is fuzzed there.

use super::types::SecurityPolicy;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// The string-level path check must never panic on arbitrary paths
    /// (traversal, NUL bytes, tilde, URL-encoding, unicode).
    #[test]
    fn path_string_check_never_panics(path in ".*") {
        let p = SecurityPolicy::default();
        let _ = p.is_path_string_allowed(&path);
    }
}
