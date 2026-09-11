//! Pins the orchestrator's parallel fan-out routing (#4754, re-anchored for #5757).
//!
//! An agent-efficiency eval found the orchestrator never fanning out workers
//! concurrently: parallel/"separate researcher for each"/council prompts either
//! single-spawned or issued serial `spawn_subagent` calls 145-200s apart
//! (each sub-agent finishing before the next started), defeating the request.
//!
//! Root cause was routing, not harness concurrency, and it is still routing
//! that these assertions guard. What changed is the primitive being routed to.
//!
//! #5757 (`02d81f6cf`) retired `spawn_parallel_agents` along with five other
//! sub-agent tools, cutting the surface from 11 tools to 3. A dedicated fan-out
//! tool was judged "a second way to say spawn again" now that
//! `spawn_async_subagent` is always async: it returns a task id immediately, so
//! N spawns issued together are already N workers running concurrently. The
//! retirement is pinned both ways in `agents/loader.rs` — three tools required,
//! six asserted absent — so it is deliberate and enforced, not drift.
//!
//! The anti-pattern this file exists to catch is therefore unchanged — a prompt
//! that lets fan-out serialize — but the guidance it anchors on had to move
//! with the tool. Anchoring on the retired name is what left this suite
//! asserting against a prompt that no longer mentions it.

const ORCHESTRATOR_PROMPT: &str =
    include_str!("../src/openhuman/agent/registry/agents/orchestrator/prompt.md");

#[test]
fn prompt_routes_fanout_to_concurrent_async_spawns() {
    let prompt = ORCHESTRATOR_PROMPT.to_lowercase();

    // The fan-out guidance must exist and name the primitive that actually
    // runs concurrently. Post-#5757 that is `spawn_async_subagent`; the
    // assertion is deliberately on the *current* tool rather than on whichever
    // name happened to be right in 2026, because a prompt naming a retired
    // tool is the failure this file is meant to catch.
    assert!(
        ORCHESTRATOR_PROMPT.contains("spawn_async_subagent"),
        "orchestrator prompt must name the concurrent spawn primitive (#4754, #5757)"
    );

    // It must say that fan-out IS several spawns — the sentence that replaced
    // "use one spawn_parallel_agents call". Without it a model reading the
    // prompt has no instruction to issue them together, which is exactly the
    // serialization the eval measured.
    assert!(
        prompt.contains("fan-out is just several") || prompt.contains("n spawns"),
        "orchestrator prompt must state that fan-out is several spawns issued \
         together, not a sequence of dependent ones (#4754, #5757)"
    );

    // And it must state that they run concurrently. "Several spawns" is only
    // the fix if the spawns overlap; a prompt that dropped this could be read
    // as endorsing the 145-200s serial gaps the eval found.
    assert!(
        prompt.contains("run concurrently") || prompt.contains("concurrently"),
        "orchestrator prompt must state that several spawns run concurrently, \
         which is what makes fan-out a fan-out (#4754, #5757)"
    );
}

/// The retired fan-out tool must not come back in the prompt without coming
/// back in `agent.toml` — a prompt teaching a tool the orchestrator cannot call
/// is worse than one that teaches nothing, because the model spends a turn
/// discovering it. `agents/loader.rs` already pins the tool list itself; this
/// pins the half of the contract that lives in prose.
#[test]
fn prompt_does_not_teach_a_retired_subagent_tool() {
    for retired in [
        "spawn_parallel_agents",
        "wait_subagent",
        "steer_subagent",
        "close_subagent",
        "wait_loop",
    ] {
        assert!(
            !ORCHESTRATOR_PROMPT.contains(retired),
            "orchestrator prompt teaches `{retired}`, which #5757 retired from \
             agent.toml — re-adding one means re-adding it in both places"
        );
    }
}

// `spawn_subagent_description_redirects_fanout_to_parallel` was removed here.
//
// It required `spawn_subagent`'s description to redirect fan-out to
// `spawn_parallel_agents` — a tool #5757 retired. It still passed, because the
// description still says it, which made it the same orphan as the two above:
// a test pinning guidance for a tool nothing can call.
//
// It is deleted rather than re-anchored because there is no correct name to
// re-anchor it to. `spawn_subagent` survives in exactly one agent —
// `trigger_reactor/agent.toml` — and that agent's tool list is
// `spawn_subagent` alone: no `spawn_parallel_agents`, and no
// `spawn_async_subagent` either. So the redirect is dangling for its only
// caller, and pointing it at the new tool would leave it just as dangling.
//
// The underlying defect is in `src/`, not in a test: `spawn_subagent`'s
// description sends its only caller to a tool that caller does not have.
// Deciding what it should say instead — give `trigger_reactor` a fan-out
// affordance, or drop the redirect — is a product call for #5757's author, so
// it is reported rather than guessed at here. Encoding a guess as an assertion
// is what left this file asserting against a retired tool in the first place.
