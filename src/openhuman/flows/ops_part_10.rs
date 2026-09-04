
/// Emit the terminal chat event a streamed builder/scout turn owes its viewers.
/// The progress bridge only streams intermediate deltas; without this the live
/// session spins forever. Mirrors how `task_dispatcher/executor.rs` finalizes a
/// streamed run: a success delivers a `chat_done` (via the shared presentation
/// path, so segmentation/reaction match a normal turn), a failure publishes a
/// `chat_error`. Broadcast as `"system"` so any viewer of the thread receives
/// it (frontend keys by `thread_id`).
async fn finalize_flow_stream(
    target: &FlowStreamTarget,
    result: &Result<String, String>,
    prompt: &str,
) {
    match result {
        Ok(text) => {
            crate::openhuman::web_chat::presentation::deliver_response(
                "system",
                &target.thread_id,
                &target.request_id,
                text,
                prompt,
                &[],
                // Builder/scout turns don't surface in the chat footer; their
                // token/cost spend is still captured by the global cost tracker.
                None,
                // No workspace in scope on this path, so the viewing client
                // stays the only persister of a flow turn's reply — unchanged
                // from before #6034, which covered the chat surfaces.
                None,
            )
            .await;
        }
        Err(err) => {
            crate::openhuman::web_chat::publish_web_channel_event(
                crate::core::socketio::WebChannelEvent {
                    event: "chat_error".to_string(),
                    client_id: "system".to_string(),
                    thread_id: target.thread_id.clone(),
                    request_id: target.request_id.clone(),
                    message: Some(err.clone()),
                    error_type: Some("agent_error".to_string()),
                    ..Default::default()
                },
            );
        }
    }
    tracing::info!(
        target: "flows",
        thread_id = %target.thread_id,
        request_id = %target.request_id,
        ok = result.is_ok(),
        "[flows] progress bridge: detached (terminal chat event emitted)"
    );
}

/// Runs the read-only `flow_discovery` agent ("Flow Scout") on demand: it reads
/// the user's memory/threads/people/connections/existing flows, grounds a few
/// automation ideas, and records them via the `suggest_workflows` tool (which
/// persists to the `flow_suggestions` table). Returns the current set of active
/// (`New`) suggestions after the run.
///
/// The agent is strictly read-only — its only write is `suggest_workflows`
/// (`PermissionLevel::None`) — so this never persists, enables, or runs a flow.
/// Turning a suggestion into a real flow is the user's separate "Build this"
/// action, which routes to `workflow_builder`.
pub async fn flows_discover(
    config: &Config,
    stream: Option<FlowStreamTarget>,
) -> Result<RpcOutcome<Vec<FlowSuggestion>>, String> {
    use crate::openhuman::agent::turn_origin::{with_origin, AgentTurnOrigin};
    use crate::openhuman::agent::Agent;

    tracing::info!(
        target: "flows",
        streaming = stream.is_some(),
        "[flows] flows_discover: starting Flow Scout discovery run"
    );

    // The registry must be initialised before building a named builtin agent
    // (mirrors `agent_registry::ops::available_tools`); it is idempotent, so a
    // second call from an already-booted core is a cheap no-op.
    crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .map_err(|e| format!("failed to initialise agent registry: {e}"))?;

    let mut agent = Agent::from_config_for_agent(config, "flow_discovery")
        .map_err(|e| format!("failed to build flow_discovery agent: {e:#}"))?;
    agent.set_agent_definition_name("flow_discovery".to_string());

    // When a chat thread is attached, stream the scout turn into it exactly like
    // an interactive turn (see `FlowStreamTarget`). Best-effort — with no target
    // the run stays headless, exactly as before.
    if let Some(target) = &stream {
        attach_flow_progress_bridge(&mut agent, target, "flows_discover", config);
    }

    // Run to completion under a CLI origin (an internal, user-initiated action —
    // the approval gate must not fail-closed on it), bounded by a wall-clock
    // timeout so a hung provider call can't wedge the RPC. When streaming, the
    // run is wrapped in the thread-id scope so descendant turns tag their trace
    // and socket events with this thread.
    let run = with_origin(AgentTurnOrigin::Cli, agent.run_single(FLOW_DISCOVER_PROMPT));
    let run = tokio::time::timeout(
        std::time::Duration::from_secs(FLOW_DISCOVER_TIMEOUT_SECS),
        run,
    );
    let timed = match &stream {
        Some(target) => {
            crate::openhuman::agent::tinyagents::thread_context::with_thread_id(
                target.thread_id.clone(),
                run,
            )
            .await
        }
        None => run.await,
    };
    // Reduce the (timeout, run) result to a single `Result<summary, error>` so
    // the terminal chat event can be emitted uniformly for the streamed case.
    let outcome: Result<String, String> = match timed {
        Ok(Ok(summary)) => {
            tracing::debug!(target: "flows", "[flows] flows_discover: agent run completed");
            Ok(summary)
        }
        Ok(Err(e)) => {
            // The agent errored. Surface it, but still return whatever
            // suggestions may already be persisted (a prior run's active set)
            // rather than hard-failing the UI.
            tracing::warn!(target: "flows", error = %e, "[flows] flows_discover: agent run failed");
            Err(format!("flow_discovery run failed: {e:#}"))
        }
        Err(_) => {
            tracing::warn!(
                target: "flows",
                timeout_secs = FLOW_DISCOVER_TIMEOUT_SECS,
                "[flows] flows_discover: agent run timed out"
            );
            Err(format!(
                "flow_discovery run timed out after {FLOW_DISCOVER_TIMEOUT_SECS}s"
            ))
        }
    };

    // Emit the terminal chat event so a client viewing the thread finalizes the
    // assistant bubble instead of spinning (the bridge only streams deltas).
    if let Some(target) = &stream {
        finalize_flow_stream(target, &outcome, FLOW_DISCOVER_PROMPT).await;
    }

    let suggestions = store::list_suggestions(config, Some(SuggestionStatus::New), 50)
        .map_err(|e| e.to_string())?;
    tracing::info!(
        target: "flows",
        count = suggestions.len(),
        "[flows] flows_discover: returning active suggestions"
    );
    Ok(RpcOutcome::single_log(
        suggestions,
        "flow discovery complete",
    ))
}

/// Overall safety bound on one `flows_build` run. The `workflow_builder` agent's
/// own `max_iterations` caps its loop, but a hung LLM/tool call must never let
/// the RPC block indefinitely.
///
/// Matches [`FLOW_RUN_TIMEOUT_SECS`] (600s): the session builder applies the
/// `workflow_builder` definition's `effective_max_iterations()` (50, not the
/// global default of 10) to this path (issue #4868), so a worst-case run at
/// ~10s/iteration can take up to ~500s — the old 300s bound would have
/// clipped a legitimate long build before the iteration cap ever got a
/// chance to.
const FLOW_BUILD_TIMEOUT_SECS: u64 = 600;

/// Tools stripped from the `workflow_builder` belt on the direct `flows_build`
/// RPC path (issue #4593; widened for `resume_flow_run`/`cancel_flow_run`
/// alongside issue #4881, which added both to the belt without extending
/// this list).
///
/// `flows_build` runs the builder under [`AgentTurnOrigin::Cli`] so the approval
/// gate does not fail-closed in a headless/streamed run — but that same origin
/// makes [`crate::openhuman::security::approval::ApprovalGate`] **auto-allow** every
/// `external_effect` tool. The flows live-runner (`run_flow`,
/// [`crate::openhuman::flows::tools`]'s `RunFlowTool`) executes a *live* saved
/// flow (real Slack/Gmail/HTTP/code effects via [`flows_run`]), so a stray call
/// during an authoring turn would fire it with no HITL confirmation. This path
/// has no routable approval surface yet (the copilot stream carries only a
/// broadcast `thread_id`, no per-user `client_id`), so rather than
/// park-then-TTL-deny we make it **unreachable** here — matching `flows_build`'s
/// contract that it "never enables or runs a flow". The tool stays available
/// (and properly gated behind a real `WebChat` approval card) when
/// `workflow_builder` is invoked as the `build_workflow` chat delegate.
///
/// `run_flow` is the live-runner on the belt today. The legacy `run_workflow`
/// name (now the unrelated harness spawn tool) is listed too as belt-and-braces
/// against a re-rename or the name ever leaking back onto this belt;
/// `hide_tools` no-ops on a name that isn't present.
///
/// `resume_flow_run` ([`builder_tools::ResumeFlowRunTool`]) is the exact same
/// concern as `run_flow`, one hop later: it is `external_effect() == true`
/// (its own description says "This ADVANCES A REAL RUN — approved outbound
/// nodes will fire") and would be auto-allowed by the same `Cli`-origin gate
/// bypass, letting an authoring turn (or a confused/prompt-injected model)
/// approve a live run's parked Slack/Gmail/HTTP node with zero human
/// confirmation — the exact HITL hole #4593 closed, reopened by #4881
/// widening the belt.
///
/// `cancel_flow_run` ([`builder_tools::CancelFlowRunTool`]) is now
/// `external_effect() == true` and ownership-checks the run against a
/// caller-named `flow_id` (T-M3 fix) — but that gate is exactly the one this
/// `Cli`-origin path auto-allows, same as `resume_flow_run` above, so the
/// ownership check alone is not a substitute for a human decision here. An
/// authoring turn still has no business tearing down a run the *user*
/// started with zero confirmation, so it stays hidden alongside the two
/// above out of caution.
///
/// `create_workflow` / `duplicate_flow` are deliberately **left visible**:
/// both are hard-forced **born disabled** (see [`builder_tools::CreateWorkflowTool`]
/// / [`builder_tools::DuplicateFlowTool`]), so even an unattended call can't
/// leave anything live — lower risk than the run/resume/cancel trio above.
const FLOWS_BUILD_HIDDEN_TOOLS: &[&str] = &[
    "run_workflow",
    "run_flow",
    "resume_flow_run",
    "cancel_flow_run",
];

/// Strip the live-run / resume / cancel tool(s) in [`FLOWS_BUILD_HIDDEN_TOOLS`]
/// from `agent`'s callable set for the direct `flows_build` RPC path.
///
/// Delegates to [`crate::openhuman::agent::Agent::hide_tools`], which removes
/// the names from the builder's (already narrow) visible belt and rebuilds the
/// session's `ToolPolicySession` so they resolve to `Deny` at the tool-call
/// boundary — a hard execution guarantee even if the model requests the tool.
/// The authoring tools (`propose`/`revise`/`save`/`dry_run`/reads/`create_workflow`/
/// `duplicate_flow`) stay visible and untouched, so the turn never fail-closes.
fn restrict_builder_toolset(agent: &mut crate::openhuman::agent::Agent) {
    tracing::debug!(
        target: "flows",
        hidden = ?FLOWS_BUILD_HIDDEN_TOOLS,
        "[flows] flows_build: hiding live-run/resume/cancel tools from builder belt"
    );
    agent.hide_tools(FLOWS_BUILD_HIDDEN_TOOLS);
}

/// Tools stripped from the `workflow_builder` belt on the STREAMING
/// (copilot-pane) `flows_build` path — the reduced sibling of
/// [`FLOWS_BUILD_HIDDEN_TOOLS`] used by [`restrict_builder_toolset`] on the
/// headless path.
///
/// PR3 (flows-copilot-live-run-approval): when a chat thread is attached
/// (`stream.is_some()`), `flows_build` now runs the builder under
/// [`AgentTurnOrigin::WebChat`] with [`APPROVAL_CHAT_CONTEXT`] scoped
/// alongside it — the exact same double-scope the main web-chat delegate uses
/// (`web_chat::ops::run_turn_under_cancel_and_deadline`). Under that origin
/// the [`crate::openhuman::security::approval::ApprovalGate`] no longer auto-allows
/// `external_effect` tools; it PARKS them for a real human decision, routed
/// back to this thread via the existing `approval_request` socket event and
/// rendered with the existing `ApprovalRequestCard` in the copilot panel. So
/// `run_flow` and `resume_flow_run` — both `external_effect() == true` — no
/// longer need to be hidden on this path: they are reachable, but gated
/// behind a real approval, exactly like a main-chat tool call.
///
/// `cancel_flow_run` stays HIDDEN on this path (codex review, #5090) — but for
/// a narrower reason than before. The original justification was that it
/// reported `external_effect() == false`, so `ApprovalSecurityMiddleware`
/// would not park it behind the approval surface, and that it cancelled an
/// arbitrary run id (e.g. one read from `list_flow_runs`) with no ownership
/// check: an unhidden call would have let a streaming copilot turn cancel ANY
/// in-flight or approval-parked run, unapproved. **The T-M3 fix closed both of
/// those gaps** — [`builder_tools::CancelFlowRunTool`] is now
/// `external_effect() == true` (so it would park behind the same real
/// `WebChat` approval card as `run_flow`/`resume_flow_run` on this path) AND
/// verifies the target run actually belongs to the caller-named `flow_id`
/// before touching it.
///
/// It is nonetheless kept hidden **deliberately**. Unhiding it would be a
/// capability expansion, not a security fix: it newly lets an authoring turn
/// tear down a run the *user* started, which is a product decision nobody has
/// taken — and hardening the tool is not a reason to take it implicitly. A
/// user can still cancel from the Runs rail. Dropping this entry is now safe
/// from a gating standpoint whenever that decision is made; that safety is
/// what the T-M3 fix bought.
///
/// `run_workflow` (the unrelated legacy skills-workflow runner sharing this
/// belt) stays hidden — belt-and-braces against a re-rename or the name ever
/// leaking back onto the `workflow_builder` toolset; `hide_tools` no-ops on a
/// name that isn't present.
const FLOWS_BUILD_COPILOT_HIDDEN_TOOLS: &[&str] = &["run_workflow", "cancel_flow_run"];

/// Strip only [`FLOWS_BUILD_COPILOT_HIDDEN_TOOLS`] from `agent`'s callable set
/// on the streaming `flows_build` path (copilot pane with a real approval
/// surface) — see that constant's doc for the full safety rationale.
fn restrict_builder_toolset_for_copilot(agent: &mut crate::openhuman::agent::Agent) {
    tracing::info!(
        target: "flows",
        hidden = ?FLOWS_BUILD_COPILOT_HIDDEN_TOOLS,
        "[flows] flows_build: streaming copilot turn — run_flow/resume_flow_run/cancel_flow_run \
         stay visible (all three gated behind the WebChat approval surface; cancel_flow_run also \
         ownership-checks the target run's flow_id — T-M3 fix); only the unrelated legacy \
         run_workflow is hidden"
    );
    agent.hide_tools(FLOWS_BUILD_COPILOT_HIDDEN_TOOLS);
}

/// Runs the `workflow_builder` agent for one authoring turn and returns its
/// proposal, invoking it as a first-class backend agent (exactly like the Flow
/// Scout `flows_discover`) rather than routing a hand-crafted delegate prompt
/// through the chat orchestrator.
///
/// The turn's natural-language brief is rendered **server-side** from the
/// structured [`BuilderRequest`](crate::openhuman::flows::agents::workflow_builder::builder_prompt::BuilderRequest)
/// (create / revise / repair / build). The agent ends by calling
/// `propose_workflow` / `revise_workflow` / `save_workflow`; we capture the
/// resulting `{ type: "workflow_proposal", … }` payload from the run's tool
/// history and return it alongside the agent's final assistant text.
///
/// Persistence stays with the agent's tools: `propose`/`revise` never persist;
/// `save_workflow` (only reachable in `build` mode with a real `flow_id`)
/// writes onto an existing flow. This op never enables or runs a flow.
pub async fn flows_build(
    config: &Config,
    req: crate::openhuman::flows::agents::workflow_builder::builder_prompt::BuilderRequest,
    stream: Option<FlowStreamTarget>,
) -> Result<RpcOutcome<Value>, String> {
    flows_build_with_extra_hidden_tools(config, req, stream, &[]).await
}
