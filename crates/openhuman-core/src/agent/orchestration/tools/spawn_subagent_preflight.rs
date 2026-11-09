macro_rules! spawn_subagent_preflight {
    ($dispatch:ident, $agent_id:ident, $prompt:ident, $context:ident, $model_override:ident, $task_title:ident, $task_key:ident, $force_fresh:ident, $run_context:ident, $args:ident, $detached_parent:ident) => {
        let $dispatch = $run_context.dispatch_ctx();
        let AsyncSpawnArgs {
            $agent_id,
            mut $prompt,
            $context,
            $model_override,
            $task_title,
            $task_key,
            $force_fresh,
        } = decode_async_spawn_args(&$args);

        if let Some(refusal) = refuse_empty_spawn_args(&$agent_id, &$prompt) {
            return Ok(refusal);
        }
        let Some($detached_parent) = $detached_parent else {
            return Ok(ToolResult::error(
                "spawn_subagent requires a live harness run context.",
            ));
        };
    };
}

macro_rules! spawn_subagent_definition {
    ($definition:ident, $run_context:ident, $agent_id:ident, $parent:ident) => {
        let registry = match AgentDefinitionRegistry::for_core_context(
            $run_context.dispatch_ctx().into_core(),
        ) {
            Some(registry) => registry,
            None => {
                return Ok(ToolResult::error(
                    "spawn_subagent: AgentDefinitionRegistry has not been initialised",
                ));
            }
        };
        let $definition = match registry.get(&$agent_id).cloned() {
            Some(definition) => definition,
            None => {
                let available: Vec<&str> = registry.list().iter().map(|d| d.id.as_str()).collect();
                return Ok(ToolResult::error(format!(
                    "spawn_subagent: unknown agent_id '{}'. Available: {}",
                    $agent_id,
                    available.join(", ")
                )));
            }
        };

        if !$parent.allowed_subagent_ids.contains(&$definition.id) {
            log::warn!(
                "[spawn_subagent] blocked subagent outside allowlist parent={} requested={} allowed={:?}",
                $parent.agent_definition_id,
                $definition.id,
                $parent.allowed_subagent_ids
            );
            return Ok(ToolResult::error(format!(
                "spawn_subagent: agent '{}' is not in parent agent '{}' subagents.allowlist",
                $definition.id, $parent.agent_definition_id
            )));
        }
    };
}
