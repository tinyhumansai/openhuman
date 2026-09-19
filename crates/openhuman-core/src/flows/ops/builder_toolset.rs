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
pub(super) const FLOW_BUILD_TIMEOUT_SECS: u64 = 600;

/// Tools stripped from the `workflow_builder` belt on the direct `flows_build`
/// RPC path (issue #4593; widened for `resume_flow_run`/`cancel_flow_run`
/// alongside issue #4881, which added both to the belt without extending
/// this list).
///
/// `flows_build` runs the builder under [`AgentTurnOrigin::Cli`] so the approval
/// gate does not fail-closed in a headless/streamed run — but that same origin
/// makes [`crate::security::approval::ApprovalGate`] **auto-allow** every
/// `external_effect` tool. The flows live-runner (`run_flow`,
/// [`crate::flows::tools`]'s `RunFlowTool`) executes a *live* saved
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
pub(super) const FLOWS_BUILD_HIDDEN_TOOLS: &[&str] = &[
    "run_workflow",
    "run_flow",
    "resume_flow_run",
    "cancel_flow_run",
];

/// Strip the live-run / resume / cancel tool(s) in [`FLOWS_BUILD_HIDDEN_TOOLS`]
/// from `agent`'s callable set for the direct `flows_build` RPC path.
///
/// Delegates to [`crate::agent::OpenHumanSessionHost::hide_tools`], which removes
/// the names from the builder's (already narrow) visible belt and rebuilds the
/// session's `ToolPolicySession` so they resolve to `Deny` at the tool-call
/// boundary — a hard execution guarantee even if the model requests the tool.
/// The authoring tools (`propose`/`revise`/`save`/`dry_run`/reads/`create_workflow`/
/// `duplicate_flow`) stay visible and untouched, so the turn never fail-closes.
pub(super) fn restrict_builder_toolset(agent: &mut crate::agent::OpenHumanSessionHost) {
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
/// the [`crate::security::approval::ApprovalGate`] no longer auto-allows
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
pub(super) const FLOWS_BUILD_COPILOT_HIDDEN_TOOLS: &[&str] = &["run_workflow", "cancel_flow_run"];

/// Strip only [`FLOWS_BUILD_COPILOT_HIDDEN_TOOLS`] from `agent`'s callable set
/// on the streaming `flows_build` path (copilot pane with a real approval
/// surface) — see that constant's doc for the full safety rationale.
pub(super) fn restrict_builder_toolset_for_copilot(agent: &mut crate::agent::OpenHumanSessionHost) {
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
