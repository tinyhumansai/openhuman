// `spawn_subagent`'s parameter schema.
//
// Textually included into `spawn_subagent.rs` beside
// `spawn_subagent_tool_impl.rs`, so it shares that module's imports. Split out
// because the impl fragment had grown to 803 lines against the layout gate's
// pin, and this is the one part of it that answers a question of its own —
// "what arguments does this tool take?" — with no dependence on the execution
// path around it. It is a pure function of the agent registry.
//
// Behaviour is unchanged: the same statements, returning the same value. The
// trait method now delegates here, because a trait impl must stay one block.

/// The `spawn_subagent` JSON schema, with `agent_id` enumerated from the live
/// registry when one is initialised.
fn spawn_subagent_parameters_schema() -> serde_json::Value {
    // Build the agent_id enum dynamically from the global registry
    // when it's been initialised. Falls back to a string-with-hint
    // when the registry hasn't been set up yet (e.g. early tests).
    let agent_ids: Vec<String> = AgentDefinitionRegistry::current()
        .map(|reg| reg.list().iter().map(|d| d.id.clone()).collect())
        .unwrap_or_default();

    let agent_id_schema = if agent_ids.is_empty() {
        json!({
            "type": "string",
            "description": "Sub-agent id (e.g. code_executor, planner, critic)."
        })
    } else {
        json!({
            "type": "string",
            "enum": agent_ids,
            "description": "Sub-agent id from the registry."
        })
    };

    json!({
        "type": "object",
        "required": ["agent_id", "prompt"],
        "properties": {
            "agent_id": agent_id_schema,
            // keep until v0.66 (review after 2027-01-11): resumed sessions freeze
            // their tool schemas; retirement needs persisted-call evidence.
            // Back-compat alias — older callers used `archetype`.
            "archetype": {
                "type": "string",
                "description": "Deprecated alias for `agent_id`. Use `agent_id` going forward."
            },
            "prompt": {
                "type": "string",
                "description": "Clear, specific instruction for the sub-agent. The sub-agent has no memory of the parent's conversation, so include all context the sub-agent needs to act."
            },
            "context": {
                "type": "string",
                "description": "Optional context blob from prior task results. Rendered as a `[Context]` block before the prompt."
            },
            "model": {
                "type": "string",
                "description": "Optional exact model id for this spawn only. Keeps the parent provider/routing, but pins the child agent to this model instead of the agent definition's default."
            },
            "dedicated_thread": {
                "type": "boolean",
                "description": "Legacy compatibility flag. Delegations now always create a persistent worker thread when parent context is available, so this flag no longer gates thread creation."
            },
            "blocking": {
                "type": "boolean",
                "description": "Explicitly run the sub-agent inline and return its final output. Defaults to false; reusable async delegation is the default."
            },
            "task_key": {
                "type": "string",
                "description": "Optional deterministic identity key for reusable async delegation. Defaults to a normalized prompt/title."
            },
            "fresh": {
                "type": "boolean",
                "description": "When true, bypass reusable subagent matching and create a fresh durable worker."
            }
        }
    })
}
