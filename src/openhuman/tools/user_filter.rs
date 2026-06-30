use std::collections::HashSet;

/// A user-toggleable tool *family*: one UI toggle that controls a set of Rust
/// tool `name()` values, plus whether that family ships enabled by default.
///
/// `default_enabled` mirrors the frontend tool catalog
/// (`app/src/utils/toolDefinitions.ts` `defaultEnabled`). It is the key to the
/// additive-safe behaviour in [`filter_tools_by_user_preference`]: a default-ON
/// family that is *absent* from a persisted snapshot is treated as a baseline
/// capability and retained (so a stale snapshot written before the family
/// existed cannot silently disable it — issue #3096), whereas a default-OFF
/// family stays opt-in and is stripped unless explicitly enabled.
struct ToolFamily {
    /// UI toggle ID stored in app state.
    id: &'static str,
    /// Rust tool `name()` values this family controls.
    rust_names: &'static [&'static str],
    /// Whether this family is enabled by default (catalog parity).
    default_enabled: bool,
}

/// Maps UI-level tool toggle IDs (stored in app state) to the Rust tool
/// `name()` values they control. Tools not covered by any mapping entry
/// are always retained — only tools that appear here are filterable.
const TOOL_FAMILIES: &[ToolFamily] = &[
    ToolFamily {
        id: "shell",
        rust_names: &["shell"],
        default_enabled: true,
    },
    // Dedicated app-launcher: always-allow, no shell exposure, no workspace_only concern.
    ToolFamily {
        id: "launch_app",
        rust_names: &["launch_app"],
        default_enabled: true,
    },
    // AXUIElement interaction: semantic UI control via macOS Accessibility API.
    // No CGEventPost, no coordinate dependency, no CEF crash risk.
    ToolFamily {
        id: "ax_interact",
        rust_names: &["ax_interact"],
        default_enabled: true,
    },
    // Multi-step UI automation (one call → whole flow). Same opt-in as
    // ax_interact; surfaced as its own catalog toggle.
    ToolFamily {
        id: "automate",
        rust_names: &["automate"],
        default_enabled: true,
    },
    // Computer control — mouse and keyboard. Gated by computer_control.enabled
    // in config (tools only register when that flag is true). Each tool also
    // overrides `external_effect` → true so the ApprovalGate fires per-action —
    // `PermissionLevel::Dangerous` alone does NOT trigger the gate (it's only a
    // static channel-capability filter); the gate keys off `external_effect_with_args`.
    ToolFamily {
        id: "computer_control",
        rust_names: &["mouse", "keyboard"],
        default_enabled: true,
    },
    // detect_tools / install_tool are filterable but not surfaced in the
    // default-ON catalog, so they stay opt-in (default-OFF).
    ToolFamily {
        id: "detect_tools",
        rust_names: &["detect_tools"],
        default_enabled: false,
    },
    ToolFamily {
        id: "install_tool",
        rust_names: &["install_tool"],
        default_enabled: false,
    },
    ToolFamily {
        id: "git_operations",
        rust_names: &["git_operations"],
        default_enabled: true,
    },
    ToolFamily {
        id: "file_read",
        rust_names: &["file_read", "read_diff", "csv_export"],
        default_enabled: true,
    },
    ToolFamily {
        id: "file_write",
        rust_names: &["file_write", "update_memory_md"],
        default_enabled: true,
    },
    ToolFamily {
        id: "screenshot",
        rust_names: &["screenshot"],
        default_enabled: true,
    },
    ToolFamily {
        id: "image_info",
        rust_names: &["image_info"],
        default_enabled: true,
    },
    ToolFamily {
        id: "browser_open",
        rust_names: &["browser_open"],
        default_enabled: false,
    },
    ToolFamily {
        id: "browser",
        rust_names: &["browser"],
        default_enabled: false,
    },
    ToolFamily {
        id: "http_request",
        rust_names: &["http_request"],
        default_enabled: false,
    },
    ToolFamily {
        id: "web_search",
        rust_names: &["web_search_tool"],
        default_enabled: true,
    },
    ToolFamily {
        id: "memory_store",
        rust_names: &["memory_store"],
        default_enabled: true,
    },
    ToolFamily {
        id: "memory_recall",
        rust_names: &["memory_recall"],
        default_enabled: true,
    },
    ToolFamily {
        id: "memory_forget",
        rust_names: &["memory_forget"],
        default_enabled: true,
    },
    ToolFamily {
        id: "cron",
        rust_names: &[
            "cron_add",
            "cron_list",
            "cron_remove",
            "cron_update",
            "cron_run",
            "cron_runs",
        ],
        default_enabled: true,
    },
    ToolFamily {
        id: "schedule",
        rust_names: &["schedule"],
        default_enabled: true,
    },
    // Self-update tools (issue #1435). Filterable so the onboarding
    // tool-toggle surface can default them off and let users opt in.
    // `update_check` is read-only; `update_apply` is gated by both the
    // tool-level autonomy check and `config.update.rpc_mutations_enabled`.
    ToolFamily {
        id: "update",
        rust_names: &["update_check", "update_apply"],
        default_enabled: false,
    },
    // Knowledge & memory — overextending tools (agent-tool expansion). Listed
    // so onboarding can default them OFF; read/bounded-write siblings are not
    // listed and stay always-retained.
    ToolFamily {
        id: "people_refresh_address_book",
        rust_names: &["people_refresh_address_book"],
        default_enabled: false,
    },
    ToolFamily {
        id: "workflow_manage",
        rust_names: &[
            "create_workflow",
            "install_workflow_from_url",
            "uninstall_workflow",
        ],
        default_enabled: false,
    },
    // Legacy alias: pre-rename `enabled_tool_names` snapshots stored this
    // family as `skill_manage`. Retain it (same rust_names) so users who had
    // already opted in keep these default-OFF tools enabled after the
    // skills→workflows rename instead of silently losing them.
    ToolFamily {
        id: "skill_manage",
        rust_names: &[
            "create_workflow",
            "install_workflow_from_url",
            "uninstall_workflow",
        ],
        default_enabled: false,
    },
    ToolFamily {
        id: "thread_destructive",
        rust_names: &["thread_delete", "thread_purge_all"],
        default_enabled: false,
    },
    ToolFamily {
        id: "billing_writes",
        rust_names: &[
            "billing_purchase_plan",
            "billing_top_up_credits",
            "billing_create_coinbase_charge",
            "billing_create_setup_intent",
            "billing_update_card",
            "billing_delete_card",
            "billing_redeem_coupon",
            "billing_update_auto_recharge",
        ],
        default_enabled: false,
    },
    ToolFamily {
        id: "team_admin",
        rust_names: &[
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
        default_enabled: false,
    },
    ToolFamily {
        id: "service_lifecycle",
        rust_names: &[
            "service_start",
            "service_stop",
            "service_restart",
            "service_shutdown",
            "service_install",
            "service_uninstall",
            "daemon_host_prefs_set",
        ],
        default_enabled: false,
    },
    ToolFamily {
        id: "screen_permissions",
        rust_names: &[
            "screen_intelligence_request_permissions",
            "screen_intelligence_request_permission",
        ],
        default_enabled: false,
    },
    ToolFamily {
        id: "mcp_manage",
        rust_names: &["mcp_registry_install", "mcp_registry_uninstall"],
        default_enabled: false,
    },
    ToolFamily {
        id: "workspace_manage",
        rust_names: &[
            "workspace_update_persona",
            "workspace_reset_persona",
            "workspace_init",
        ],
        default_enabled: false,
    },
    ToolFamily {
        id: "learning_manage",
        rust_names: &[
            "learning_update_facet",
            "learning_pin_facet",
            "learning_unpin_facet",
            "learning_forget_facet",
            "learning_rebuild_cache",
            "learning_reset_cache",
            "learning_save_profile",
            "learning_enrich_profile",
        ],
        default_enabled: false,
    },
    // Task & workflow productivity — overextending tools (agent-tool
    // expansion). Only the destructive/persistent-config mutators are listed
    // here so the onboarding toggle surface can default them OFF and let users
    // opt in; the read-only + bounded-write siblings (e.g. `artifact_list`,
    // `todo_add`, `task_source_fetch`) are intentionally NOT listed, so they
    // are always-retained infrastructure. Grouped one toggle per risk family.
    ToolFamily {
        id: "agent_workflow_uninstall",
        rust_names: &["agent_workflow_uninstall"],
        default_enabled: false,
    },
    ToolFamily {
        id: "artifact_delete",
        rust_names: &["artifact_delete"],
        default_enabled: false,
    },
    ToolFamily {
        id: "todo_destructive",
        rust_names: &["todo_remove", "todo_replace", "todo_clear"],
        default_enabled: false,
    },
    ToolFamily {
        id: "task_source_manage",
        rust_names: &[
            "task_source_add",
            "task_source_update",
            "task_source_remove",
        ],
        default_enabled: false,
    },
];

/// All Rust tool names that are filterable (union of all mapping values).
/// Any tool whose name is NOT in this set is infrastructure and always retained.
fn all_filterable_tool_names() -> HashSet<&'static str> {
    TOOL_FAMILIES
        .iter()
        .flat_map(|fam| fam.rust_names.iter().copied())
        .collect()
}

/// Resolve a Rust tool `name()` to the [`ToolFamily`] that owns it. Returns
/// `None` for infrastructure tools that no mapping entry covers.
fn family_for_rust_name(name: &str) -> Option<&'static ToolFamily> {
    TOOL_FAMILIES
        .iter()
        .find(|fam| fam.rust_names.iter().any(|n| *n == name))
}

/// Expand persisted tool-preference entries into Rust tool `name()` values.
///
/// Accepts both formats we may find in app state:
/// - Rust tool names (new format)
/// - UI toggle IDs (legacy / partial-rollout format)
///
/// Unknown entries are ignored.
const LEGACY_ALIASES: &[(&str, &str)] = &[("skill_manage", "workflow_manage")];

fn expand_enabled_tool_names(enabled_tool_names: &[String]) -> HashSet<String> {
    let mut expanded = HashSet::new();
    for entry in enabled_tool_names {
        let resolved = LEGACY_ALIASES
            .iter()
            .find(|(old, _)| *old == entry.as_str())
            .map(|(_, new)| *new)
            .unwrap_or(entry.as_str());
        if let Some(fam) = TOOL_FAMILIES.iter().find(|fam| fam.id == resolved) {
            for name in fam.rust_names {
                expanded.insert((*name).to_string());
            }
            continue;
        }

        if TOOL_FAMILIES
            .iter()
            .flat_map(|fam| fam.rust_names.iter().copied())
            .any(|name| name == entry)
        {
            expanded.insert(entry.clone());
        }
    }
    expanded
}

/// True when the persisted tool-preference snapshot opts into the mutating
/// app-control actions — i.e. the user enabled "App UI Control" (`ax_interact`)
/// or "App Automation" (`automate`) in Settings → Features → Tools.
///
/// Those tools' descriptions promise clicking buttons and typing into fields,
/// but enabling the toggle alone only made the read-only `list` action
/// available — the mutating `press` / `set_value` actions required a separate,
/// UI-less `computer_control.ax_interact_mutations` flag or Full autonomy
/// (#3762). The session builder uses this to treat enabling the tool as the
/// user-facing opt-in for those mutations, equivalent to setting that flag. The
/// actions stay approval-gated and bound by the sensitive-app denylist.
///
/// Reuses [`expand_enabled_tool_names`] so it is robust to both persisted
/// formats (UI toggle ids or Rust tool names). An empty snapshot ("not yet
/// configured" / all-enabled) is treated as no explicit opt-in, preserving the
/// safe default.
pub(crate) fn enables_app_ui_control_mutations(enabled_tool_names: &[String]) -> bool {
    if enabled_tool_names.is_empty() {
        return false;
    }
    let expanded = expand_enabled_tool_names(enabled_tool_names);
    expanded.contains("ax_interact") || expanded.contains("automate")
}

/// Given the list of enabled tools from app state, retain only tools that are
/// either infrastructure (not filterable), explicitly enabled, or a default-ON
/// capability the snapshot never explicitly opted out of.
///
/// An empty `enabled_tool_names` list means "all enabled" (default / not yet
/// configured) — the filter is a no-op in that case.
///
/// ## Additive-safe (issue #3096)
///
/// The persisted `enabled_tool_names` is a *frozen snapshot* of the user's tool
/// choices, written once by the frontend (onboarding / Settings → Tools) from
/// the catalog that existed at that moment. Treating it as a strict exhaustive
/// allowlist silently disables any **default-ON** family that was added to the
/// catalog *after* the snapshot was written — the #3096 symptom, where the
/// agent reported it lacked `cron_add` because an older snapshot predated the
/// cron family.
///
/// Fix: an absent filterable tool is stripped **only when its family is
/// default-OFF** (an opt-in capability the user never enabled). Default-ON
/// families are baseline capabilities and are retained even when absent, so a
/// stale snapshot can never silently remove a tool the user never saw. The
/// trade-off is that a default-ON family cannot be turned off purely by its
/// absence from the snapshot — a deliberate decision: silently breaking a
/// baseline capability (scheduled tasks) is far worse than keeping it
/// available. Default-OFF opt-in gating (the ~160 overextending tools from
/// #3050) is unchanged.
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
        // Explicitly enabled by the snapshot.
        if allowed.contains(name) {
            return true;
        }
        // Filterable + not explicitly enabled. Default-ON families are baseline
        // capabilities: retain them so a stale/partial snapshot can never
        // silently disable a tool (#3096). Default-OFF families stay opt-in and
        // are stripped.
        match family_for_rust_name(name) {
            Some(fam) if fam.default_enabled => {
                log::debug!(
                    "[tool-filter] retaining default-ON '{}' (family '{}' absent from persisted allowlist; baseline capability)",
                    name,
                    fam.id
                );
                true
            }
            Some(_) => false,
            // Defensive: filterable name with no resolvable family — retain.
            None => true,
        }
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
    use super::{
        enables_app_ui_control_mutations, expand_enabled_tool_names,
        filter_tools_by_user_preference,
    };
    use crate::openhuman::tools::traits::{Tool, ToolResult};
    use async_trait::async_trait;

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

    /// Minimal name-only tool stub so the filter (which only reads `name()`)
    /// can be exercised without constructing real tool implementations.
    struct FakeTool(&'static str);

    #[async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "fake"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success("ok"))
        }
    }

    fn names(tools: &[Box<dyn Tool>]) -> Vec<&str> {
        tools.iter().map(|t| t.name()).collect()
    }

    fn tools(names: &[&'static str]) -> Vec<Box<dyn Tool>> {
        names
            .iter()
            .map(|n| Box::new(FakeTool(n)) as Box<dyn Tool>)
            .collect()
    }

    #[test]
    fn empty_preference_list_is_a_noop() {
        let mut t = tools(&["cron_add", "shell", "file_read"]);
        filter_tools_by_user_preference(&mut t, &[]);
        assert_eq!(names(&t).len(), 3);
    }

    /// Regression for #3096: a non-empty snapshot that never references the
    /// cron family (e.g. written by an older build whose catalog predated the
    /// cron family) must NOT strip cron tools — cron is default-ON, a baseline
    /// capability, so it is retained.
    #[test]
    fn retains_cron_when_snapshot_predates_cron_family() {
        let mut t = tools(&["cron_add", "cron_list", "web_search_tool", "shell"]);
        // Snapshot only references web_search; the cron family is absent.
        filter_tools_by_user_preference(&mut t, &["web_search_tool".to_string()]);
        let kept = names(&t);
        assert!(
            kept.contains(&"cron_add"),
            "cron_add must survive a cron-less snapshot"
        );
        assert!(
            kept.contains(&"cron_list"),
            "cron_list must survive a cron-less snapshot"
        );
        assert!(kept.contains(&"web_search_tool"));
        // Infrastructure tool (not in any mapping) is always retained.
        assert!(kept.contains(&"shell"));
    }

    /// A default-ON sibling absent from a cron-aware snapshot is still retained
    /// (default-ON families are baseline capabilities, not absence-disabled).
    #[test]
    fn retains_default_on_cron_sibling_even_when_family_referenced() {
        let mut t = tools(&["cron_add", "cron_list", "shell"]);
        // Snapshot references the cron family via cron_list but omits cron_add.
        filter_tools_by_user_preference(&mut t, &["cron_list".to_string()]);
        let kept = names(&t);
        assert!(kept.contains(&"cron_list"));
        assert!(
            kept.contains(&"cron_add"),
            "default-ON cron_add is a baseline capability"
        );
        assert!(kept.contains(&"shell"));
    }

    /// Default-OFF families stay opt-in: absent from the snapshot ⇒ stripped.
    /// This is the opt-in gating the overextending tools (#3050) rely on.
    #[test]
    fn strips_default_off_family_when_not_opted_in() {
        let mut t = tools(&["service_start", "service_stop", "file_read", "cron_add"]);
        // Snapshot references only file_read (a default-ON family).
        filter_tools_by_user_preference(&mut t, &["file_read".to_string()]);
        let kept = names(&t);
        assert!(
            !kept.contains(&"service_start"),
            "default-OFF service_start must be stripped"
        );
        assert!(
            !kept.contains(&"service_stop"),
            "default-OFF service_stop must be stripped"
        );
        assert!(
            kept.contains(&"file_read"),
            "explicitly enabled file_read stays"
        );
        assert!(
            kept.contains(&"cron_add"),
            "default-ON cron_add stays even when absent"
        );
    }

    /// Explicitly opting into a default-OFF family retains it.
    #[test]
    fn retains_default_off_family_when_opted_in() {
        let mut t = tools(&["service_start", "service_stop", "file_read"]);
        filter_tools_by_user_preference(&mut t, &["service_lifecycle".to_string()]);
        let kept = names(&t);
        assert!(kept.contains(&"service_start"));
        assert!(kept.contains(&"service_stop"));
    }

    /// The legacy UI toggle ID form expands to the whole family.
    #[test]
    fn ui_toggle_id_enables_whole_cron_family() {
        let mut t = tools(&["cron_add", "cron_list", "cron_remove", "service_start"]);
        filter_tools_by_user_preference(&mut t, &["cron".to_string()]);
        let kept = names(&t);
        assert!(kept.contains(&"cron_add"));
        assert!(kept.contains(&"cron_list"));
        assert!(kept.contains(&"cron_remove"));
        // service_start (default-OFF) not opted in → stripped.
        assert!(!kept.contains(&"service_start"));
    }

    /// A list whose entries match no known UI ID or tool name yields an empty
    /// allowed set, tripping the safety fallback that leaves tools unfiltered.
    #[test]
    fn unrecognized_only_list_leaves_tools_unfiltered() {
        let mut t = tools(&["cron_add", "service_start"]);
        filter_tools_by_user_preference(&mut t, &["totally_unknown".to_string()]);
        assert_eq!(names(&t).len(), 2);
    }

    // #3762: enabling the App UI Control / App Automation tool is the opt-in
    // for the mutating click/type actions.
    #[test]
    fn app_ui_control_opt_in_detects_ax_interact_and_automate() {
        assert!(enables_app_ui_control_mutations(&[
            "ax_interact".to_string()
        ]));
        assert!(enables_app_ui_control_mutations(&["automate".to_string()]));
        // Present alongside other enabled tools.
        assert!(enables_app_ui_control_mutations(&[
            "cron_add".to_string(),
            "automate".to_string(),
        ]));
    }

    #[test]
    fn app_ui_control_opt_in_false_when_absent_or_empty() {
        assert!(!enables_app_ui_control_mutations(&[]));
        assert!(!enables_app_ui_control_mutations(&[
            "cron_add".to_string(),
            "service_start".to_string(),
        ]));
        // Unknown entries never opt in.
        assert!(!enables_app_ui_control_mutations(&[
            "totally_unknown".to_string()
        ]));
    }
}
