//! Sub-agent provider and model resolution.
//!
//! Resolves `(provider, model)` from a declarative [`ModelSpec`], plus
//! Composio sign-in probe and the lazy toolkit action resolver.

use std::sync::Arc;

pub(crate) async fn resolve_subagent_source(
    spec: &tinyagents_definition::ModelSpec,
    agent_id: &str,
    config: Option<&crate::config::Config>,
    parent_source: crate::agent::tinyagents::TurnModelSource,
    parent_model: String,
    is_team_lead: bool,
    model_override: Option<&str>,
    temperature: f64,
) -> (crate::agent::tinyagents::TurnModelSource, String) {
    use tinyagents_definition::ModelSpec;
    if let Some(model) = model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        tracing::debug!(
            agent_id,
            model,
            "[subagent_host] using inline model override"
        );
        return (parent_source, model.to_string());
    }
    if let Some(model) = config.and_then(|cfg| cfg.configured_agent_model(agent_id, is_team_lead)) {
        tracing::debug!(
            agent_id,
            model,
            "[subagent_host] using config-level model pin"
        );
        return (parent_source, model.to_string());
    }
    match spec {
        ModelSpec::Hint(workload) => match config {
            Some(config) => {
                match crate::inference::provider::create_chat_model_with_model_id_in(
                    &parent_source.dispatch_ctx(),
                    workload,
                    config,
                    temperature,
                )
                .await
                {
                    Ok((_model, model_id)) => {
                        tracing::info!(
                            agent_id,
                            role = workload,
                            model = %model_id,
                            "[subagent_host] resolved crate-native workload source"
                        );
                        (
                            crate::agent::tinyagents::TurnModelSource::new_crate_native(
                                &parent_source.dispatch_ctx(),
                                workload.clone(),
                                Arc::new(config.clone()),
                            ),
                            model_id,
                        )
                    }
                    Err(error) => {
                        tracing::warn!(
                            agent_id,
                            role = workload,
                            %error,
                            parent_model,
                            "[subagent_host] workload model build failed; inheriting parent source"
                        );
                        (parent_source, parent_model)
                    }
                }
            }
            None => {
                tracing::warn!(
                    agent_id,
                    role = workload,
                    parent_model,
                    "[subagent_host] config unavailable; inheriting parent source"
                );
                (parent_source, parent_model)
            }
        },
        ModelSpec::Inherit => (parent_source, parent_model),
        ModelSpec::Exact(model) => (parent_source, model.clone()),
    }
}
