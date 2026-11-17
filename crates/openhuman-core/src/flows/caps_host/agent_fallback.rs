use super::*;

impl OpenHumanAgentRunner {
    /// Persona-shaping single-completion fallback for a custom
    /// [`AgentRegistryEntry`](crate::agent_registry::AgentRegistryEntry)
    /// with no harness definition — the pre-Phase-A behavior, kept so custom
    /// agents don't regress.
    pub(super) async fn run_via_registry_fallback(
        &self,
        dispatch: &crate::core::runtime::DispatchCtx,
        agent_ref: &str,
        request: Value,
        conn: Option<&str>,
    ) -> Result<Value> {
        // Resolve + validate the requested agent kind against the registry.
        let entry = crate::agent_registry::get_agent(dispatch, agent_ref)
            .await
            .map_err(EngineError::Capability)?
            .ok_or_else(|| {
                EngineError::Capability(format!(
                    "agent node: unknown agent_ref '{agent_ref}' (neither a harness definition nor \
                     a custom agent registry entry)"
                ))
            })?;
        if !entry.enabled {
            return Err(EngineError::Capability(format!(
                "agent node: agent_ref '{agent_ref}' is disabled"
            )));
        }

        tracing::debug!(
            target: "flows",
            agent_ref,
            has_system_prompt = entry.system_prompt.is_some(),
            model = entry.model.as_deref().unwrap_or("<role-default>"),
            "[flows] agent_runner: applying custom registered agent-kind persona to the completion"
        );

        // Shape the completion by the agent kind: prepend the agent's system
        // prompt (its persona) ahead of the node's messages, and adopt its model
        // when the node didn't pin one. The completion itself runs through the
        // same provider path as a plain agent turn (OpenHumanLlm::complete), so
        // structured-output / envelope behavior is identical.
        let prepared = self
            .prepare_agent_run(
                request.clone(),
                entry.model.as_deref(),
                entry.system_prompt.as_deref(),
                0,
            )
            .await?;
        let mut completion_request = request.clone();
        if !completion_request.is_object() {
            completion_request = Value::Object(Default::default());
        }
        if let Value::Object(object) = &mut completion_request {
            object.remove("input_context");
            object.remove("messages");
            object.remove("output_parser");
            object.remove("response_format");
            object.remove("timeout_secs");
            object.insert("prompt".to_owned(), Value::String(prepared.prompt));
            if let Some(model) = prepared.model {
                object.insert("model".to_owned(), Value::String(model));
            }
        }
        OpenHumanLlm {
            ctx: self.ctx.clone(),
            config: self.config.clone(),
        }
        .complete_with_format_request(completion_request, request, conn)
        .await
    }
}
