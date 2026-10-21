/// Build the worktree-isolation slice of a `subagent_completed`
/// [`SubagentProgressDetail`] (#3376). An empty `changed_files` collapses to
/// `None` so the renderer omits an empty "changed files" list rather than
/// showing "0 files"; a non-empty list is forwarded verbatim. `worktree_path`
/// / `dirty_status` pass through (`None` for non-isolated workers). Split out
/// so the empty/non-empty branch is unit-testable without a live DB + channel.
fn subagent_worktree_detail(
    worktree_path: Option<String>,
    changed_files: Vec<String>,
    dirty_status: Option<bool>,
) -> SubagentProgressDetail {
    SubagentProgressDetail {
        worktree_path,
        changed_files: if changed_files.is_empty() {
            None
        } else {
            Some(changed_files)
        },
        dirty_status,
        ..Default::default()
    }
}

/// Trace user attribution for a turn whose stored user payload carries no identity
/// (headless / autonomous / freshly booted cores): read the on-disk
/// app-session profile and return the user's email (preferred) or backend
/// user id. `None` when signed out or the profile is unreadable.
fn session_profile_user_attribution(config: &crate::config::Config) -> Option<String> {
    let state = crate::security::credentials::session_support::build_session_state(config).ok()?;
    state
        .user
        .as_ref()
        .and_then(|u| u.get("email"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or(state.user_id)
}

