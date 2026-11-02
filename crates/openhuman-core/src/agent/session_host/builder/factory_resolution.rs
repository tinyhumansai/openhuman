//! Agent-definition fallback resolution and provider-role selection for session construction.

/// Resolve the provider/workload role for a session build.
///
/// Explicit `hint:<role>` markers route to their workload; everything else
/// (incl. the legacy `default_model` tier the bootstrap pinned) falls through
/// to `chat` so `chat_provider` drives the user-facing turn.
pub(crate) fn provider_role_for(default_model: Option<&str>) -> &'static str {
    match default_model.map(str::trim) {
        Some("hint:agentic") => "agentic",
        Some("hint:coding") => "coding",
        Some("hint:summarization") => "summarization",
        Some("hint:reasoning") => "reasoning",
        _ => "chat",
    }
}

/// Resolve the initial harness workload without constructing a session.
/// Flow readiness shares this path so it checks the provider used at runtime.
pub(crate) fn provider_role_for_definition(
    default_model: Option<&str>,
    target_def: Option<&crate::agent_registry::host_definition::AgentDefinition>,
) -> &'static str {
    let master_hint = default_model
        .is_none()
        .then(|| {
            target_def.and_then(|def| {
                if def.id == "orchestrator" {
                    match &def.model {
                        tinyagents_definition::ModelSpec::Hint(hint) => {
                            Some(format!("hint:{hint}"))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
        })
        .flatten();
    provider_role_for(master_hint.as_deref().or(default_model))
}

/// Resolve the model source, workload role, and native-tool capability for a session.
///
/// Session construction and triage direct turns share the same resolved source when one
/// is already available; otherwise the ordinary provider factory owns construction.
pub(crate) async fn resolve_session_model(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &crate::config::Config,
    agent_id: &str,
    target_def: Option<&crate::agent_registry::host_definition::AgentDefinition>,
    turn_model_source: Option<&(crate::agent::tinyagents::TurnModelSource, String)>,
) -> anyhow::Result<(&'static str, String, bool)> {
    // The Master OpenHumanSessionHost definition selects the coding workload so the default
    // user-facing turn has a model suitable for the direct inspect → edit → verify loop.
    // Legacy/no-definition callers retain the configured chat default.
    let provider_role = provider_role_for_definition(config.default_model.as_deref(), target_def);
    // Retry/backoff is owned by the crate RetryPolicy at the harness model call. The turn path
    // must not wrap the provider again or it will double-retry. Cross-route fallback likewise
    // belongs to the crate registry FallbackPolicy.
    let (mut model_name, supports_native) = match turn_model_source {
        Some((source, model_name)) => (model_name.clone(), source.supports_native_tool_calling()),
        None => {
            let (model, model_name) =
                crate::inference::provider::create_chat_model_with_model_id_in(
                    ctx,
                    provider_role,
                    config,
                    config.default_temperature,
                )
                .await?;
            let supports_native = model.profile().is_none_or(|profile| profile.tool_calling);
            (model_name, supports_native)
        }
    };
    log::info!(
        "[session-builder] agent_id={} provider_role={} resolved_model={} supports_native_tools={}",
        agent_id,
        provider_role,
        model_name,
        supports_native
    );
    let target_agent_id = target_def
        .map(|def| def.id.as_str())
        .unwrap_or("orchestrator");
    let target_is_lead = target_def
        .map(|def| !def.subagents.is_empty())
        .unwrap_or(true);
    if let Some(pinned_model) = config.configured_agent_model(target_agent_id, target_is_lead) {
        log::debug!(
            "[session-builder] agent_id={} using config-level model pin model={}",
            target_agent_id,
            pinned_model
        );
        model_name = pinned_model.to_string();
    }
    Ok((provider_role, model_name, supports_native))
}

#[cfg(test)]
#[path = "factory_resolution_tests.rs"]
mod tests;
