use super::*;

pub(super) struct InferenceReadinessEvaluation {
    pub(super) status: &'static str,
    pub(super) message: Option<String>,
    pub(super) node_id: Option<String>,
}

/// Resolve the effective inference workload for an agent node. This remains
/// beside the graph adapter because it depends on TinyFlows node configuration.
pub(super) async fn agent_node_role(config: &Config, node_config: &Value) -> &'static str {
    if let Some(agent_ref) = node_config
        .get("agent_ref")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('='))
    {
        use crate::agent_registry::host_definition::AgentDefinitionRegistry;
        if let Err(error) = AgentDefinitionRegistry::init_global(&config.workspace_dir) {
            tracing::debug!(target: "flows", %error,
                "[flows] readiness: agent definition registry unavailable; using registry fallback");
        }
        let definition = AgentDefinitionRegistry::current().and_then(|r| r.get(agent_ref).cloned());
        let custom = crate::agent_registry::find_custom_in_config(config, agent_ref);
        if definition.is_some() || custom.is_some() {
            let entry_model = if definition.is_some() {
                None
            } else {
                custom.as_ref().and_then(|entry| entry.model.as_deref())
            };
            let effective_iteration_cap = definition
                .as_ref()
                .map(|definition| u32::try_from(definition.max_iterations).unwrap_or(u32::MAX))
                .unwrap_or(1);
            let override_model = match crate::modules::tinyflows::portability_proxy(config).await {
                Ok(proxy) => proxy
                    .call::<tinyflows_bus::portability::PreparedAgentRun>(
                        tinyflows_bus::portability::methods::PREPARE_AGENT_RUN,
                        &(tinyflows_bus::portability::PrepareAgentRun {
                            request: node_config.clone(),
                            entry_model: entry_model.map(str::to_owned),
                            system_prompt: None,
                            effective_iteration_cap,
                        },),
                    )
                    .await
                    .map(|prepared| prepared.model)
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error),
            }
            .unwrap_or_else(|error| {
                tracing::debug!(target: "flows", %error,
                    "[flows] readiness: module model preparation unavailable");
                None
            })
            .map(|model| crate::flows::agent_route::harness_model_default_override(&model));
            return crate::agent::session_host::provider_role_for_definition(
                override_model
                    .as_deref()
                    .or(config.default_model.as_deref()),
                definition.as_ref(),
            );
        }
    }
    let pinned_model = node_config
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(model) = pinned_model {
        return crate::inference::provider_model::role_for_provider_model(
            crate::inference::provider::factory::PROVIDER_OPENHUMAN,
            model,
        );
    }
    "summarization"
}

fn format_evaluation(
    evaluation: &crate::inference::readiness::ReadinessEvaluation,
) -> Option<String> {
    if evaluation.status == "ready" {
        return None;
    }
    if let [failure] = evaluation.failures.as_slice() {
        if let [_node_id] = failure.node_ids.as_slice() {
            if failure.session_wide {
                return Some(failure.message.clone());
            }
            return Some(if evaluation.status == "provider_not_configured" {
                format!(
                    "This flow's agent step needs a working AI provider, but the provider \
                     returned: '{}'. Configure your provider API key in OpenHuman Settings > \
                     Providers, then try again.",
                    failure.message
                )
            } else {
                format!(
                    "This flow's agent step needs a working AI provider: {}",
                    failure.message
                )
            });
        }
    }
    let details = evaluation
        .failures
        .iter()
        .map(|failure| {
            let nodes = failure
                .node_ids
                .iter()
                .map(|id| format!("'{id}'"))
                .collect::<Vec<_>>()
                .join(", ");
            if failure.status == "provider_not_configured" {
                format!(
                    "Node(s) {nodes} (role `{}`): the provider returned: '{}'. Configure \
                     your provider API key in OpenHuman Settings > Providers, then try again.",
                    failure.role, failure.message
                )
            } else {
                format!(
                    "Node(s) {nodes} (role `{}`): {}",
                    failure.role, failure.message
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    Some(format!(
        "This flow has {} agent step(s) that need a working AI provider:\n\n{details}",
        evaluation.failures.len()
    ))
}

/// Shared readiness evaluation for the advisory proposal status and hard run gate.
pub(super) async fn evaluate_inference_readiness(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    graph: &Value,
) -> Option<InferenceReadinessEvaluation> {
    let mut roles = Vec::new();
    for node in crate::flows::host_gates::nodes(graph).filter(|node| node.kind == "agent") {
        roles.push((
            node.id.to_owned(),
            agent_node_role(config, node.config).await,
        ));
    }
    let evaluation = crate::inference::readiness::evaluate(ctx, config, &roles).await?;
    let message = format_evaluation(&evaluation);
    let node_id = match evaluation.failures.as_slice() {
        [failure] if failure.node_ids.len() == 1 => Some(failure.node_ids[0].clone()),
        _ => None,
    };
    Some(InferenceReadinessEvaluation {
        status: evaluation.status,
        message,
        node_id,
    })
}

/// Run-time hard gate, returned in the flows domain's established error shape.
pub(crate) async fn validate_inference_readiness(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    graph: &Value,
) -> Vec<String> {
    let Some(evaluation) = evaluate_inference_readiness(ctx, config, graph).await else {
        return Vec::new();
    };
    let Some(message) = evaluation.message else {
        return Vec::new();
    };
    if let Some(node_id) = evaluation.node_id {
        vec![format!("Node '{node_id}': {message}")]
    } else {
        vec![message]
    }
}

#[cfg(test)]
#[path = "inference_readiness_tests.rs"]
mod tests;
