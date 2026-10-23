//! `[security.autonomy] enabled = false` — the shipped default.
//!
//! Two halves, and both matter: with the policy disabled a shell is a shell and
//! a working directory is writable, **but** credential stores and system roots
//! are still unreachable. The second half is the one a future edit is most
//! likely to break, so it is asserted here beside the first.

use super::*;

fn disabled_policy() -> SecurityPolicy {
    SecurityPolicy {
        enabled: false,
        ..SecurityPolicy::default()
    }
}

#[test]
fn config_default_is_disabled_but_struct_default_is_enabled() {
    // The shipped default: nothing in a stock config turns the policy on.
    assert!(!crate::config::AutonomyConfig::default().enabled);
    // The fail-closed fallback for a policy built with no config at all.
    assert!(SecurityPolicy::default().enabled);
}

#[test]
fn from_config_carries_the_disabled_flag() {
    let cfg = crate::config::AutonomyConfig::default();
    let policy = SecurityPolicy::from_config(&cfg, Path::new("/ws"), Path::new("/act"));
    assert!(!policy.enabled);
}

// A disabled policy allows every command class; that table is the module's
// and is pinned on the real module by `released_module_tier_gate_matches_the_inlined_table`.
