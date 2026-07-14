//! Pins the orchestrator's parallel/council fan-out routing (#4754).
//!
//! An agent-efficiency eval found the orchestrator never fanning out workers
//! concurrently: parallel/"separate researcher for each"/council prompts either
//! single-spawned or issued serial `spawn_subagent` calls 145-200s apart
//! (each sub-agent finishing before the next started), defeating the request.
//!
//! Root cause is routing, not harness concurrency: `spawn_parallel_agents`
//! already fans out concurrently (tinyagents `map_reduce` / `buffer_unordered`)
//! as a single tool call, but the orchestrator reached for serial
//! `spawn_subagent` instead. The fix steers parallel/council/fan-out requests
//! to ONE `spawn_parallel_agents` call — in both the system prompt and the
//! `spawn_subagent` tool description the model reads when choosing a tool.
//!
//! These assertions pin that guidance so a future edit can't silently drop it.

const ORCHESTRATOR_PROMPT: &str =
    include_str!("../src/openhuman/agent_registry/agents/orchestrator/prompt.md");

const SPAWN_SUBAGENT_SRC: &str =
    include_str!("../src/openhuman/agent_orchestration/tools/spawn_subagent.rs");

#[test]
fn prompt_routes_parallel_and_council_to_spawn_parallel_agents() {
    // The parallel-fanout guidance must exist and name the concurrent primitive.
    assert!(
        ORCHESTRATOR_PROMPT.contains("spawn_parallel_agents"),
        "orchestrator prompt must route fan-out to `spawn_parallel_agents` (#4754)"
    );
    // It must explicitly cover the council / multiple-independent-opinions case
    // that the eval showed collapsing to a single spawn.
    assert!(
        ORCHESTRATOR_PROMPT.to_lowercase().contains("council"),
        "orchestrator prompt must steer council / multiple-opinions requests to \
         a parallel fan-out, not a single spawn (#4754)"
    );
    // It must warn against the serial anti-pattern (looping `spawn_subagent`),
    // which is what produced the 145-200s serial gaps.
    let p = ORCHESTRATOR_PROMPT.to_lowercase();
    assert!(
        p.contains("never a loop of") || p.contains("do **not** call `spawn_subagent` once per"),
        "orchestrator prompt must warn that repeated `spawn_subagent` serializes \
         fan-out and to use one `spawn_parallel_agents` call instead (#4754)"
    );
}

#[test]
fn spawn_subagent_description_redirects_fanout_to_parallel() {
    // The tool description is what the model reads while picking a tool, so the
    // redirect has to live there, not only in the system prompt. Anchor on the
    // description() body to avoid matching an unrelated mention elsewhere.
    let desc_start = SPAWN_SUBAGENT_SRC
        .find("fn description(&self)")
        .expect("spawn_subagent must have a description()");
    let desc = &SPAWN_SUBAGENT_SRC[desc_start..];
    let desc_body = &desc[..desc.find("fn parameters_schema").unwrap_or(desc.len())];
    assert!(
        desc_body.contains("spawn_parallel_agents"),
        "spawn_subagent's description must redirect concurrent fan-out to \
         `spawn_parallel_agents` so the model picks the parallel tool (#4754)"
    );
}
