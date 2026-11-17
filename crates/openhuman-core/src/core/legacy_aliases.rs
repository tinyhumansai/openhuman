// keep until v0.66 (review after 2027-01-11): shipped-client analytics and
// Sentry method-count coverage are required before removing both wire tables.
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
    // The session RPCs became the kind-agnostic credential pair once the
    // core stopped validating sessions itself (`auth.set_credential` /
    // `auth.clear_credential`). A pre-migration bundle's `store_session`
    // params (`token`, `userId`, `user`) are a subset of `set_credential`'s;
    // its `clear_session` maps to a clear of every kind, which on a desktop
    // (never an API-key holder) is the same thing.
    (
        "openhuman.auth_clear_session",
        "openhuman.auth_clear_credential",
    ),
    (
        "openhuman.auth_store_session",
        "openhuman.auth_set_credential",
    ),
    // #3565: old desktop clients called the channels controller with a dotted
    // namespace/function spelling before the canonical
    // `openhuman.<namespace>_<function>` form was established.
    ("channels.list", "openhuman.channels_list"),
    // MCP clients — old method names that appeared in Sentry (CORE-RUST-DR/DS/DT/DV/DW).
    // Callers used dotted namespace, bare `mcp_list`, `mcp_servers_list`, and
    // `mcp_clients_list` before the canonical `mcp_clients_installed_list` was
    // introduced in PR #2409. `tool_registry_call` was an early mis-spelling of
    // `mcp_clients_tool_call` that shipped in at least one older bundle.
    // `mcp_clients.list` sorts before all `openhuman.*` entries (m < o).
    ("mcp_clients.list", "openhuman.mcp_clients_installed_list"),
    ("openhuman.channels.list", "openhuman.channels_list"),
    (
        "openhuman.get_analytics_settings",
        "openhuman.config_get_analytics_settings",
    ),
    (
        "openhuman.get_composio_trigger_settings",
        "openhuman.config_get_composio_trigger_settings",
    ),
    (
        "openhuman.get_dashboard_settings",
        "openhuman.config_get_dashboard_settings",
    ),
    ("openhuman.get_config", "openhuman.config_get"),
    (
        "openhuman.get_runtime_flags",
        "openhuman.config_get_runtime_flags",
    ),
    (
        "openhuman.mcp_clients_list",
        "openhuman.mcp_clients_installed_list",
    ),
    ("openhuman.mcp_list", "openhuman.mcp_clients_installed_list"),
    (
        "openhuman.mcp_servers_list",
        "openhuman.mcp_clients_installed_list",
    ),
    ("openhuman.ping", "core.ping"),
    (
        "openhuman.set_browser_allow_all",
        "openhuman.config_set_browser_allow_all",
    ),
    (
        "openhuman.tool_registry_call",
        "openhuman.mcp_clients_tool_call",
    ),
    // #3294: old desktop bundles called the tool-registry diagnostics
    // controller with the dotted `tool_registry.diagnostics` spelling, before
    // the canonical `openhuman.<namespace>_<function>` form
    // (`openhuman.tool_registry_diagnostics`) was established. Without this
    // alias the Tool Policy diagnostics panel's RPC failed with "unknown
    // method" on those clients.
    (
        "tool_registry.diagnostics",
        "openhuman.tool_registry_diagnostics",
    ),
    (
        "openhuman.update_analytics_settings",
        "openhuman.config_update_analytics_settings",
    ),
    (
        "openhuman.update_autonomy_settings",
        "openhuman.config_update_autonomy_settings",
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
        "openhuman.inference_update_local_settings",
    ),
    (
        "openhuman.update_memory_settings",
        "openhuman.config_update_memory_settings",
    ),
    (
        "openhuman.update_model_settings",
        "openhuman.inference_update_model_settings",
    ),
    (
        "openhuman.update_runtime_settings",
        "openhuman.config_update_runtime_settings",
    ),
    (
        "openhuman.workspace_onboarding_flag_exists",
        "openhuman.config_workspace_onboarding_flag_exists",
    ),
    (
        "openhuman.workspace_onboarding_flag_set",
        "openhuman.config_workspace_onboarding_flag_set",
    ),
    (
        "openhuman.local_ai_agent_chat",
        "openhuman.inference_agent_chat",
    ),
    (
        "openhuman.local_ai_agent_chat_simple",
        "openhuman.inference_agent_chat_simple",
    ),
    (
        "openhuman.local_ai_diagnostics",
        "openhuman.inference_diagnostics",
    ),
    // bare `health_snapshot` (no namespace prefix) was used by older clients
    // before the canonical `openhuman.health_snapshot` form was established.
    ("health_snapshot", "openhuman.health_snapshot"),
    // Dotted / bare health probes from older clients and SDK callers (#3566,
    // Sentry CORE-2C). The canonical method is `openhuman.health_snapshot`
    // (namespace `health`, function `snapshot`); these legacy spellings fell
    // through to the unknown-method path and produced Sentry noise. There is no
    // distinct `status`/`get` health handler — the snapshot already carries the
    // health verdict (`healthy`/`degraded`/`critical_unhealthy`), so all four
    // variants alias to the snapshot.
    ("health", "openhuman.health_snapshot"),
    ("health.get", "openhuman.health_snapshot"),
    ("health.snapshot", "openhuman.health_snapshot"),
    ("health.status", "openhuman.health_snapshot"),
    // `openhuman.system_info` was used by older clients / SDK callers before
    // the method was namespaced under `health` as `openhuman.health_system_info`.
    // Sentry CORE-RUST-G0 (issue 6340)
    ("openhuman.system_info", "openhuman.health_system_info"),
    (
        "openhuman.local_ai_test_connection",
        "openhuman.inference_test_connection",
    ),
    (
        "openhuman.local_ai_transcribe",
        "openhuman.inference_transcribe",
    ),
    (
        "openhuman.local_ai_transcribe_bytes",
        "openhuman.inference_transcribe_bytes",
    ),
    ("openhuman.local_ai_tts", "openhuman.inference_tts"),
    (
        "openhuman.providers_list_models",
        "openhuman.inference_list_models",
    ),
];

/// Returns the server-side legacy → canonical RPC alias table.
///
/// Keep this as the single Rust metadata source for alias consumers and tests;
/// drift guards compare it with the frontend catalog in
/// `app/src/services/rpcMethods.ts`.
fn legacy_aliases() -> &'static [(&'static str, &'static str)] {
    LEGACY_ALIASES
}

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
    for (legacy, canonical) in legacy_aliases() {
        if *legacy == method {
            return canonical;
        }
    }
    method
}

#[cfg(test)]
#[path = "legacy_aliases_tests.rs"]
mod tests;
