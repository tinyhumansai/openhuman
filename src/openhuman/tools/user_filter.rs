use std::collections::HashSet;

/// Maps UI-level tool toggle IDs (stored in app state) to the Rust tool
/// `name()` values they control. Tools not covered by any mapping entry
/// are always retained — only tools that appear here are filterable.
const TOOL_ID_TO_RUST_NAMES: &[(&str, &[&str])] = &[
    ("shell", &["shell"]),
    ("detect_tools", &["detect_tools"]),
    ("install_tool", &["install_tool"]),
    ("git_operations", &["git_operations"]),
    ("file_read", &["file_read", "read_diff", "csv_export"]),
    ("file_write", &["file_write", "update_memory_md"]),
    ("screenshot", &["screenshot"]),
    ("image_info", &["image_info"]),
    ("browser_open", &["browser_open"]),
    ("browser", &["browser"]),
    ("http_request", &["http_request"]),
    ("web_search", &["web_search_tool"]),
    ("memory_store", &["memory_store"]),
    ("memory_recall", &["memory_recall"]),
    ("memory_forget", &["memory_forget"]),
    (
        "cron",
        &[
            "cron_add",
            "cron_list",
            "cron_remove",
            "cron_update",
            "cron_run",
            "cron_runs",
        ],
    ),
    ("schedule", &["schedule"]),
    // Self-update tools (issue #1435). Filterable so the onboarding
    // tool-toggle surface can default them off and let users opt in.
    // `update_check` is read-only; `update_apply` is gated by both the
    // tool-level autonomy check and `config.update.rpc_mutations_enabled`.
    ("update", &["update_check", "update_apply"]),
    // Knowledge & memory — overextending tools (agent-tool expansion). Listed
    // so onboarding can default them OFF; read/bounded-write siblings are not
    // listed and stay always-retained.
    (
        "people_refresh_address_book",
        &["people_refresh_address_book"],
    ),
    (
        "skill_manage",
        &["skill_create", "skill_install_from_url", "skill_uninstall"],
    ),
    ("thread_destructive", &["thread_delete", "thread_purge_all"]),
    (
        "billing_writes",
        &[
            "billing_purchase_plan",
            "billing_top_up_credits",
            "billing_create_coinbase_charge",
            "billing_create_setup_intent",
            "billing_update_card",
            "billing_delete_card",
            "billing_redeem_coupon",
            "billing_update_auto_recharge",
        ],
    ),
    (
        "team_admin",
        &[
            "team_create",
            "team_update",
            "team_delete",
            "team_switch",
            "team_join",
            "team_leave",
            "team_create_invite",
            "team_revoke_invite",
            "team_remove_member",
            "team_change_member_role",
        ],
    ),
    (
        "service_lifecycle",
        &[
            "service_start",
            "service_stop",
            "service_restart",
            "service_shutdown",
            "service_install",
            "service_uninstall",
            "daemon_host_prefs_set",
        ],
    ),
    (
        "screen_permissions",
        &[
            "screen_intelligence_request_permissions",
            "screen_intelligence_request_permission",
        ],
    ),
    (
        "mcp_manage",
        &["mcp_registry_install", "mcp_registry_uninstall"],
    ),
    (
        "workspace_manage",
        &[
            "workspace_update_persona",
            "workspace_reset_persona",
            "workspace_init",
        ],
    ),
    (
        "learning_manage",
        &[
            "learning_update_facet",
            "learning_pin_facet",
            "learning_unpin_facet",
            "learning_forget_facet",
            "learning_rebuild_cache",
            "learning_reset_cache",
            "learning_save_profile",
            "learning_enrich_profile",
        ],
    ),
    // Task & workflow productivity — overextending tools (agent-tool
    // expansion). Only the destructive/persistent-config mutators are listed
    // here so the onboarding toggle surface can default them OFF and let users
    // opt in; the read-only + bounded-write siblings (e.g. `artifact_list`,
    // `todo_add`, `task_source_fetch`) are intentionally NOT listed, so they
    // are always-retained infrastructure. Grouped one toggle per risk family.
    ("agent_workflow_uninstall", &["agent_workflow_uninstall"]),
    ("artifact_delete", &["artifact_delete"]),
    (
        "todo_destructive",
        &["todo_remove", "todo_replace", "todo_clear"],
    ),
    (
        "task_source_manage",
        &[
            "task_source_add",
            "task_source_update",
            "task_source_remove",
        ],
    ),
];

/// All Rust tool names that are filterable (union of all mapping values).
/// Any tool whose name is NOT in this set is infrastructure and always retained.
fn all_filterable_tool_names() -> HashSet<&'static str> {
    TOOL_ID_TO_RUST_NAMES
        .iter()
        .flat_map(|(_, names)| names.iter().copied())
        .collect()
}

/// Expand persisted tool-preference entries into Rust tool `name()` values.
///
/// Accepts both formats we may find in app state:
/// - Rust tool names (new format)
/// - UI toggle IDs (legacy / partial-rollout format)
///
/// Unknown entries are ignored.
fn expand_enabled_tool_names(enabled_tool_names: &[String]) -> HashSet<String> {
    let mut expanded = HashSet::new();
    for entry in enabled_tool_names {
        if let Some((_, rust_names)) = TOOL_ID_TO_RUST_NAMES.iter().find(|(id, _)| id == &entry) {
            for name in *rust_names {
                expanded.insert((*name).to_string());
            }
            continue;
        }

        if TOOL_ID_TO_RUST_NAMES
            .iter()
            .flat_map(|(_, names)| names.iter().copied())
            .any(|name| name == entry)
        {
            expanded.insert(entry.clone());
        }
    }
    expanded
}

/// Given the list of enabled tools from app state, retain only tools that are
/// either infrastructure (not filterable) or explicitly enabled.
///
/// An empty `enabled_tool_names` list means "all enabled" (default / not yet
/// configured) — the filter is a no-op in that case.
pub(crate) fn filter_tools_by_user_preference(
    tools: &mut Vec<Box<dyn crate::openhuman::tools::Tool>>,
    enabled_tool_names: &[String],
) {
    if enabled_tool_names.is_empty() {
        // Empty list means all tools are enabled (user has not configured preferences yet).
        return;
    }

    let filterable = all_filterable_tool_names();

    let allowed = expand_enabled_tool_names(enabled_tool_names);
    if allowed.is_empty() {
        log::warn!(
            "[tool-filter] enabled_tools was non-empty but none matched known UI IDs or tool names; leaving tools unfiltered for safety"
        );
        return;
    }

    let before = tools.len();
    tools.retain(|tool| {
        let name = tool.name();
        // Infrastructure tools not covered by any mapping entry are always retained.
        if !filterable.contains(name) {
            return true;
        }
        allowed.contains(name)
    });
    let after = tools.len();

    if before != after {
        log::debug!(
            "[tool-filter] filtered tools by user preference: {} → {} tools ({} removed)",
            before,
            after,
            before - after
        );
    }
}

#[cfg(test)]
mod tests {
    use super::expand_enabled_tool_names;

    #[test]
    fn expands_legacy_ui_toggle_ids_to_rust_tool_names() {
        let allowed = expand_enabled_tool_names(&["cron".to_string(), "web_search".to_string()]);
        assert!(allowed.contains("cron_add"));
        assert!(allowed.contains("cron_list"));
        assert!(allowed.contains("web_search_tool"));
    }

    #[test]
    fn keeps_direct_rust_tool_names() {
        let allowed =
            expand_enabled_tool_names(&["cron_add".to_string(), "memory_store".to_string()]);
        assert!(allowed.contains("cron_add"));
        assert!(allowed.contains("memory_store"));
    }

    #[test]
    fn ignores_unknown_entries() {
        let allowed = expand_enabled_tool_names(&["totally_unknown".to_string()]);
        assert!(allowed.is_empty());
    }
}
