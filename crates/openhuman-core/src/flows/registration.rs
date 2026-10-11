//! Registration of the flow authoring, execution, discovery, and memory tools.

use super::builder_tools::{
    CancelFlowRunTool, CreateWorkflowTool, DryRunWorkflowTool, DuplicateFlowTool, EditWorkflowTool,
    GetFlowHistoryTool, GetFlowRunTool, GetFlowTool, GetNodeKindContractTool, GetToolContractTool,
    GetToolOutputSampleTool, ListAgentDefinitionsTool, ListConnectableToolkitsTool,
    ListFlowConnectionsTool, ListFlowRunsTool, ListFlowsTool, ListNodeKindsTool, ResumeFlowRunTool,
    ReviseWorkflowTool, SaveWorkflowTool, SearchToolCatalogTool, ValidateWorkflowTool,
};
use super::discovery_tools::SuggestWorkflowsTool;
use super::memory_tools::{FlowMemoryRecallTool, FlowMemoryRememberTool};
use super::tools::{ProposeWorkflowTool, RunFlowTool};
use crate::config::Config;
use crate::security::SecurityPolicy;
use std::sync::Arc;
use tinytools::Tool;

/// Build the flow tool family in its persisted catalogue order.
pub(crate) fn tools(config: Arc<Config>, security: &Arc<SecurityPolicy>) -> Vec<Box<dyn Tool>> {
    vec![
        // Agent-first Workflow authoring (issue B4): validates a candidate
        // graph and returns a proposal summary — never creates/enables a
        // flow itself. Only the chat UI's WorkflowProposalCard "Save &
        // enable" action calls `flows_create`.
        Box::new(ProposeWorkflowTool::new(config.clone())),
        // workflow-builder agent tool belt (Phase 5b). A deliberately narrow,
        // propose-or-read surface: revise a draft (validate-only), read saved
        // flows/runs/connections, ground tool_call slugs in the real catalog,
        // and dry-run a draft against MOCK capabilities. None of these persist
        // or enable a flow (only the user's own `flows_create` click does); the
        // read tools are `PermissionLevel::None`, and `dry_run_workflow` is
        // autonomy-tier gated + wired to deterministic mock capabilities.
        Box::new(ReviseWorkflowTool::new(config.clone())),
        // Structured incremental edits (F1): apply a small ops[] list to a base
        // graph (saved flow or inline) instead of re-emitting the whole graph,
        // then validate + gate + return a proposal (same contract as revise).
        // Proposal-only — never persists.
        Box::new(EditWorkflowTool::new(config.clone())),
        // Standalone validate (F3): run the SAME structural + hard-gate stack
        // the propose/save tools use, without emitting a proposal — a pure
        // check so the agent can self-verify a draft mid-build. Read-only.
        Box::new(ValidateWorkflowTool::new(config.clone())),
        // Read a saved flow's revision history (F6) — prior graph snapshots the
        // agent can inspect / pick a rollback target from. Read-only.
        Box::new(GetFlowHistoryTool::new(config.clone())),
        // Phase 4 self-debug loop (F4): find a failing run, resume a parked
        // run (approval-gated), or cancel a runaway one.
        Box::new(ListFlowRunsTool::new(config.clone())),
        Box::new(ResumeFlowRunTool::new(config.clone())),
        Box::new(CancelFlowRunTool::new(config.clone())),
        // Gated create (F4/F12): create a NEW flow — born disabled, approval
        // gated — and duplicate an existing one (disabled copy) for
        // clone-then-edit. Behind the Phase 3 safety rails.
        Box::new(CreateWorkflowTool::new(config.clone())),
        Box::new(DuplicateFlowTool::new(config.clone())),
        Box::new(ListFlowsTool::new(config.clone())),
        Box::new(GetFlowTool::new(config.clone())),
        Box::new(GetFlowRunTool::new(config.clone())),
        Box::new(ListFlowConnectionsTool::new(config.clone())),
        Box::new(SearchToolCatalogTool::new(config.clone())),
        // Full live contract (schemas, real required_args/output_fields,
        // primary_array_path) for one action slug found via
        // search_tool_catalog — the grounding step before WIRING a node's
        // args/downstream bindings (systemic tool-contract fix, Part 1).
        Box::new(GetToolContractTool::new(config.clone())),
        // B12: ONE bounded, READ-ONLY, REAL Composio call to derive the real
        // primary_array_path/output_fields when the live listing publishes no
        // output schema at all (verified for every GitHub action) — overrides
        // get_tool_contract's schema-derived hint for that slug from then on.
        // Read-scope actions only (hard-refused otherwise), connected
        // toolkits only — see builder_tools.rs's module doc for the carve-out
        // this makes in the workflow-builder agent's "no composio_execute"
        // invariant.
        Box::new(GetToolOutputSampleTool::new(config.clone())),
        // Ground an `agent` node's `agent_ref` in real registered agent-kind ids
        // (code_executor / critic / …) — the agent analogue of
        // search_tool_catalog. Read-only.
        Box::new(ListAgentDefinitionsTool::new()),
        // Steer toolkit choice toward what's already connected + surface which
        // toolkits a flow still needs (Phase 5, item 19). Read-only.
        Box::new(ListConnectableToolkitsTool::new(config.clone())),
        // Queryable DSL schema (F2): enumerate the 13 node kinds and fetch one
        // kind's full config-field/port/example/gotcha contract — the DSL
        // analogue of search_tool_catalog + get_tool_contract, so an agent need
        // not rely on prompt prose or memory for node config shapes. Read-only.
        Box::new(ListNodeKindsTool::new()),
        Box::new(GetNodeKindContractTool::new()),
        Box::new(DryRunWorkflowTool::new(config.clone())),
        // Real end-to-end test run of a SAVED flow (Write / external-effect). The
        // workflow-builder prompt requires it to ask the user for confirmation
        // first, and the flow's own approval gate still pauses outbound nodes.
        Box::new(RunFlowTool::new(config.clone())),
        // Persist a built graph onto an EXISTING saved flow (Write). Used only
        // when the USER explicitly asks the agent to save; the seeded build
        // turn from the Flows prompt bar is propose-only (see #4596) — Accept
        // + the canvas's own Save persist the graph. The tool itself can
        // never create a flow or change enabled/require_approval.
        Box::new(SaveWorkflowTool::new(config.clone())),
        // Flow Scout discovery: the `flow_discovery` agent's terminal emit
        // sink. Read-only reasoning over the user's data ends by calling
        // `suggest_workflows`, which persists workflow ideas for the Flows page
        // "Suggested for you" section. `PermissionLevel::None`, no external
        // effect — writes only to the agent's own suggestions store.
        Box::new(SuggestWorkflowsTool::new(config.clone())),
        // Per-flow sandboxed memory (issue #5173): lets a running flow
        // (e.g. a scheduled newsletter-digest) remember what it already did
        // — dedupe across runs — without ever touching the user's own
        // memory. Namespace is derived internally from `flow_id`.
        // `flow_memory_remember` (write) only resolves that `flow_id` from
        // the run's own trusted `TrustedAutomation { Workflow }` turn origin
        // (T-M2 fix) — a chat/orchestrator turn with no trusted run origin
        // is refused outright, never routed to a model-supplied `flow_id`.
        // `flow_memory_recall`'s `scope: "flows"` is a deliberate read-only
        // cross-flow exception — it can see every flow's namespace by
        // design, but can never be used to write outside a flow's own.
        Box::new(FlowMemoryRecallTool::new()),
        Box::new(FlowMemoryRememberTool::new(security.clone())),
    ]
}
