/// Trace user attribution for a turn whose stored user payload carries no identity
/// (headless / autonomous / freshly booted cores): read the on-disk
/// app-session profile and return the user's email (preferred) or backend
/// user id. `None` when signed out or the profile is unreadable.
async fn session_profile_user_attribution(config: &crate::config::Config) -> Option<String> {
    let state = crate::credentials::session_state::build_session_state(config).await.ok()?;
    state
        .user
        .as_ref()
        .and_then(|u| u.get("email"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or(state.user_id)
}
