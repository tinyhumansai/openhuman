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
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::PathBuf;

    fn frontend_rpc_catalog_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("app/src/services/rpcMethods.ts")
    }

    fn read_frontend_rpc_catalog() -> String {
        let path = frontend_rpc_catalog_path();
        fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
    }

    fn object_body_after_marker<'a>(source: &'a str, marker: &str, terminator: &str) -> &'a str {
        let marker_start = source
            .find(marker)
            .unwrap_or_else(|| panic!("missing marker `{marker}` in frontend RPC catalog"));
        let object_start = marker_start
            + source[marker_start..]
                .find('{')
                .unwrap_or_else(|| panic!("missing object start after `{marker}`"))
            + 1;
        let rest = &source[object_start..];
        let object_end = rest
            .find(terminator)
            .unwrap_or_else(|| panic!("missing terminator `{terminator}` after `{marker}`"));
        &rest[..object_end]
    }

    fn quoted_value(text: &str) -> String {
        let (quote_index, quote) = text
            .char_indices()
            .find(|(_, ch)| *ch == '\'' || *ch == '"')
            .unwrap_or_else(|| panic!("expected quoted value in `{text}`"));
        let value_start = quote_index + quote.len_utf8();
        let rest = &text[value_start..];
        let value_end = rest
            .find(quote)
            .unwrap_or_else(|| panic!("unterminated quoted value in `{text}`"));
        rest[..value_end].to_string()
    }

    fn parse_core_rpc_methods(source: &str) -> BTreeMap<String, String> {
        let body = object_body_after_marker(source, "export const CORE_RPC_METHODS", "} as const;");
        let mut methods = BTreeMap::new();
        for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
            if line.starts_with("//") {
                continue;
            }
            let (key, value) = line
                .split_once(':')
                .unwrap_or_else(|| panic!("malformed CORE_RPC_METHODS entry: `{line}`"));
            methods.insert(key.trim().to_string(), quoted_value(value));
        }
        methods
    }

    fn parse_frontend_legacy_aliases(
        source: &str,
        core_methods: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let body = object_body_after_marker(source, "export const LEGACY_METHOD_ALIASES", "};");
        let compact = body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let mut aliases = BTreeMap::new();
        for entry in compact
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            let (legacy, target_expr) = entry
                .split_once(':')
                .unwrap_or_else(|| panic!("expected legacy alias entry, got `{entry}`"));
            let legacy = quoted_value(legacy);
            let target_expr = target_expr.trim();
            let canonical = if let Some(key) = target_expr.strip_prefix("CORE_RPC_METHODS.") {
                core_methods
                    .get(key)
                    .unwrap_or_else(|| {
                        panic!("legacy alias references unknown CORE_RPC_METHODS.{key}")
                    })
                    .clone()
            } else {
                quoted_value(target_expr)
            };
            aliases.insert(legacy, canonical);
        }
        aliases
    }

    fn registered_http_methods() -> BTreeSet<String> {
        crate::core::all::all_http_method_schemas()
            .into_iter()
            .map(|method| method.method)
            .collect()
    }

    #[test]
    fn quoted_value_extracts_single_quoted_string() {
        assert_eq!(quoted_value(": 'hello'"), "hello");
    }

    #[test]
    fn quoted_value_extracts_double_quoted_string() {
        assert_eq!(quoted_value(": \"hello\""), "hello");
    }

    #[test]
    #[should_panic(expected = "expected quoted value")]
    fn quoted_value_panics_on_unquoted_text() {
        let _ = quoted_value(": bare-token");
    }

    #[test]
    #[should_panic(expected = "unterminated quoted value")]
    fn quoted_value_panics_on_unterminated_quote() {
        let _ = quoted_value(": 'open-but-never-closed");
    }

    #[test]
    fn object_body_after_marker_returns_inner_body() {
        let source = "noise\nexport const FOO = {\n  alpha: 'a',\n  beta: 'b',\n} as const;\nrest";
        let body = object_body_after_marker(source, "export const FOO", "} as const;");
        assert!(body.contains("alpha: 'a'"));
        assert!(body.contains("beta: 'b'"));
        assert!(!body.contains("rest"));
        assert!(!body.contains("noise"));
    }

    #[test]
    #[should_panic(expected = "missing marker")]
    fn object_body_after_marker_panics_when_marker_absent() {
        let _ = object_body_after_marker("nothing here", "export const MISSING", "};");
    }

    #[test]
    #[should_panic(expected = "missing terminator")]
    fn object_body_after_marker_panics_when_terminator_absent() {
        let _ = object_body_after_marker(
            "export const FOO = { alpha: 'a',",
            "export const FOO",
            "} as const;",
        );
    }

    #[test]
    fn parse_core_rpc_methods_extracts_entries_and_skips_comments() {
        let source = "export const CORE_RPC_METHODS = {\n  // a comment that should be skipped\n  alphaMethod: 'openhuman.alpha',\n  betaMethod: 'openhuman.beta',\n} as const;\n";
        let methods = parse_core_rpc_methods(source);
        assert_eq!(
            methods.get("alphaMethod").map(String::as_str),
            Some("openhuman.alpha")
        );
        assert_eq!(
            methods.get("betaMethod").map(String::as_str),
            Some("openhuman.beta")
        );
        assert_eq!(methods.len(), 2);
    }

    #[test]
    #[should_panic(expected = "malformed CORE_RPC_METHODS entry")]
    fn parse_core_rpc_methods_panics_on_non_colon_line() {
        let source =
            "export const CORE_RPC_METHODS = {\n  alphaMethod 'openhuman.alpha',\n} as const;\n";
        let _ = parse_core_rpc_methods(source);
    }

    #[test]
    fn parse_frontend_legacy_aliases_resolves_core_method_refs_and_literals() {
        let source = "export const CORE_RPC_METHODS = {\n  alphaMethod: 'openhuman.alpha',\n} as const;\n\nexport const LEGACY_METHOD_ALIASES: Record<string, CoreRpcMethod> = {\n  'openhuman.legacy_alpha': CORE_RPC_METHODS.alphaMethod,\n  'openhuman.legacy_literal': 'openhuman.literal_target',\n};\n";
        let core_methods = parse_core_rpc_methods(source);
        let aliases = parse_frontend_legacy_aliases(source, &core_methods);
        assert_eq!(
            aliases.get("openhuman.legacy_alpha").map(String::as_str),
            Some("openhuman.alpha")
        );
        assert_eq!(
            aliases.get("openhuman.legacy_literal").map(String::as_str),
            Some("openhuman.literal_target")
        );
    }

    #[test]
    #[should_panic(expected = "legacy alias references unknown CORE_RPC_METHODS")]
    fn parse_frontend_legacy_aliases_panics_on_unknown_core_method_ref() {
        let source = "export const CORE_RPC_METHODS = {\n  alphaMethod: 'openhuman.alpha',\n} as const;\n\nexport const LEGACY_METHOD_ALIASES: Record<string, CoreRpcMethod> = {\n  'openhuman.legacy_alpha': CORE_RPC_METHODS.doesNotExist,\n};\n";
        let core_methods = parse_core_rpc_methods(source);
        let _ = parse_frontend_legacy_aliases(source, &core_methods);
    }

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

    #[test]
    fn frontend_core_rpc_methods_exist_in_core_schema_registry() {
        let source = read_frontend_rpc_catalog();
        let core_methods = parse_core_rpc_methods(&source);
        let registered = registered_http_methods();
        let missing: Vec<_> = core_methods
            .values()
            .filter(|method| !registered.contains(*method))
            .cloned()
            .collect();

        assert!(
            missing.is_empty(),
            "frontend CORE_RPC_METHODS contains methods absent from all_http_method_schemas(): {missing:?}"
        );
    }

    #[test]
    fn frontend_legacy_aliases_match_server_alias_table() {
        let source = read_frontend_rpc_catalog();
        let core_methods = parse_core_rpc_methods(&source);
        let frontend_aliases = parse_frontend_legacy_aliases(&source, &core_methods);
        let server_aliases: BTreeMap<String, String> = legacy_aliases()
            .iter()
            .map(|(legacy, canonical)| ((*legacy).to_string(), (*canonical).to_string()))
            .collect();

        assert_eq!(
            frontend_aliases, server_aliases,
            "frontend LEGACY_METHOD_ALIASES must stay in sync with src/core/legacy_aliases.rs"
        );
    }

    #[test]
    fn legacy_alias_targets_exist_in_core_schema_registry() {
        let registered = registered_http_methods();
        let missing: Vec<_> = legacy_aliases()
            .iter()
            .filter(|(_, canonical)| !registered.contains(*canonical))
            .map(|(legacy, canonical)| format!("{legacy} -> {canonical}"))
            .collect();

        assert!(
            missing.is_empty(),
            "legacy aliases point at methods absent from all_http_method_schemas(): {missing:?}"
        );
    }
}
