use std::collections::HashSet;

/// Maps UI-level tool toggle IDs (stored in app state) to the Rust tool
/// `name()` values they control. Tools not covered by any mapping entry
/// are always retained — only tools that appear here are filterable.
const TOOL_ID_TO_RUST_NAMES: &[(&str, &[&str])] = &[
    ("shell", &["shell"]),
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
];

/// All Rust tool names that are filterable (union of all mapping values).
/// Any tool whose name is NOT in this set is infrastructure and always retained.
fn all_filterable_tool_names() -> HashSet<&'static str> {
    TOOL_ID_TO_RUST_NAMES
        .iter()
        .flat_map(|(_, names)| names.iter().copied())
        .collect()
}

/// Given the list of enabled Rust tool names (already expanded from UI IDs by
/// the frontend), retain only tools that are either infrastructure (not
/// filterable) or explicitly enabled.
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

    let allowed: HashSet<&str> = enabled_tool_names.iter().map(String::as_str).collect();

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
