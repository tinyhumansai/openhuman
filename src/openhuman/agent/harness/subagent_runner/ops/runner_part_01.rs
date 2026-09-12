async fn filter_cached_toolkit_actions_with_current_scope(
    agent_id: &str,
    toolkit: &str,
    config: &crate::openhuman::config::Config,
    actions: &[crate::openhuman::agent::context::prompt::ConnectedIntegrationTool],
) -> Vec<crate::openhuman::agent::context::prompt::ConnectedIntegrationTool> {
    let pref =
        crate::openhuman::integrations::composio::ops::load_user_scope_pref(config, toolkit).await;
    let before = actions.len();
    let filtered: Vec<_> = actions
        .iter()
        .filter(|action| {
            crate::openhuman::integrations::composio::providers::is_action_visible_with_pref(
                &action.name,
                &pref,
            )
        })
        .cloned()
        .collect();
    tracing::debug!(
        agent_id = %agent_id,
        toolkit = %toolkit,
        cached_actions = before,
        visible_actions = filtered.len(),
        hidden_actions = before.saturating_sub(filtered.len()),
        read = pref.read,
        write = pref.write,
        admin = pref.admin,
        "[subagent_runner:typed] re-filtered cached toolkit catalogue with current user scope"
    );
    filtered
}
