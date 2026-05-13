//! Server-side legacy RPC method aliases.
//!
//! Mirrors the frontend's `LEGACY_METHOD_ALIASES` table in
//! `app/src/services/rpcMethods.ts`. The frontend rewrites outgoing method
//! names for clients that just updated; this module rewrites incoming
//! method names for clients that haven't updated yet (older shipped bundles
//! in the wild). Together they form a symmetric migration safety net:
//! either side can be the one that's behind, and the call still resolves.
//!
//! When adding or removing an entry here, keep
//! `app/src/services/rpcMethods.ts:LEGACY_METHOD_ALIASES` in sync. The two
//! tables are intentionally identical: the same legacy → canonical map
//! applied at both ends of the wire.
//!
//! The rewrite is a pure key-to-key lookup. No domain branches, no
//! parameter inspection — if a method isn't in the table, it passes through
//! untouched.

/// Legacy → canonical RPC method name pairs.
///
/// Order doesn't matter for correctness, but is kept alphabetical by legacy
/// key for easier diffing against the frontend table.
const LEGACY_ALIASES: &[(&str, &str)] = &[
    (
        "openhuman.get_analytics_settings",
        "openhuman.config_get_analytics_settings",
    ),
    (
        "openhuman.get_composio_trigger_settings",
        "openhuman.config_get_composio_trigger_settings",
    ),
    ("openhuman.get_config", "openhuman.config_get"),
    (
        "openhuman.get_runtime_flags",
        "openhuman.config_get_runtime_flags",
    ),
    ("openhuman.ping", "core.ping"),
    (
        "openhuman.set_browser_allow_all",
        "openhuman.config_set_browser_allow_all",
    ),
    (
        "openhuman.update_analytics_settings",
        "openhuman.config_update_analytics_settings",
    ),
    (
        "openhuman.update_browser_settings",
        "openhuman.config_update_browser_settings",
    ),
    (
        "openhuman.update_composio_trigger_settings",
        "openhuman.config_update_composio_trigger_settings",
    ),
    (
        "openhuman.update_local_ai_settings",
        "openhuman.config_update_local_ai_settings",
    ),
    (
        "openhuman.update_memory_settings",
        "openhuman.config_update_memory_settings",
    ),
    (
        "openhuman.update_model_settings",
        "openhuman.config_update_model_settings",
    ),
    (
        "openhuman.update_runtime_settings",
        "openhuman.config_update_runtime_settings",
    ),
    (
        "openhuman.update_screen_intelligence_settings",
        "openhuman.config_update_screen_intelligence_settings",
    ),
    (
        "openhuman.workspace_onboarding_flag_exists",
        "openhuman.config_workspace_onboarding_flag_exists",
    ),
    (
        "openhuman.workspace_onboarding_flag_set",
        "openhuman.config_workspace_onboarding_flag_set",
    ),
];

/// Resolves a legacy RPC method name to its canonical form, if any.
///
/// Returns the canonical name when `method` is a known legacy alias;
/// otherwise returns `method` unchanged. This function is idempotent:
/// calling it on an already-canonical name (or any unrelated name) is a
/// no-op.
///
/// Returns a borrow that lives for at least the input's lifetime — the
/// matched-canonical branch returns `&'static`, the pass-through branch
/// returns the input borrow; elision picks the tighter input lifetime.
pub fn resolve_legacy(method: &str) -> &str {
    for (legacy, canonical) in LEGACY_ALIASES {
        if *legacy == method {
            return canonical;
        }
    }
    method
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_legacy_rewrites_every_table_entry() {
        for (legacy, canonical) in LEGACY_ALIASES {
            assert_eq!(
                resolve_legacy(legacy),
                *canonical,
                "expected legacy alias {legacy} to resolve to {canonical}",
            );
        }
    }

    #[test]
    fn resolve_legacy_rewrites_composio_trigger_settings() {
        // The specific case observed in Sentry: older bundles called the
        // bare `openhuman.update_composio_trigger_settings` against a core
        // that only registers the namespaced form.
        assert_eq!(
            resolve_legacy("openhuman.update_composio_trigger_settings"),
            "openhuman.config_update_composio_trigger_settings",
        );
    }

    #[test]
    fn resolve_legacy_passes_through_unknown_methods() {
        assert_eq!(
            resolve_legacy("openhuman.memory_list_namespaces"),
            "openhuman.memory_list_namespaces"
        );
        assert_eq!(resolve_legacy("does.not.exist"), "does.not.exist");
        assert_eq!(resolve_legacy(""), "");
    }

    #[test]
    fn resolve_legacy_is_idempotent_for_canonical_names() {
        // Canonical names already match what the registry expects;
        // running them through the resolver must be a no-op so callers
        // can wrap the lookup unconditionally.
        for (_, canonical) in LEGACY_ALIASES {
            assert_eq!(
                resolve_legacy(canonical),
                *canonical,
                "canonical {canonical} must pass through unchanged",
            );
        }
    }

    #[test]
    fn resolve_legacy_returned_str_equals_table_value() {
        // Sanity check: the function returns the canonical str slice from
        // the table when it matches, not a copy of the input.
        let out = resolve_legacy("openhuman.ping");
        assert_eq!(out, "core.ping");
    }
}
