//! Agent tool belt for the `workflow-builder` specialist (Phase 5b).
//!
//! These tools give the `workflow-builder` agent (see
//! `agent_registry/agents/workflow_builder/`) its full authoring surface for
//! tinyflows [`WorkflowGraph`]s in chat — 22 tools spanning propose-only
//! validation, in-place draft mutation, live reads, one bounded real Composio
//! call, run control, and persistence:
//!
//! | Tool                             | Permission | Effect                                                        |
//! | --------------------------------- | ---------- | ---------------------------------------------------------------- |
//! | [`ReviseWorkflowTool`]            | `None`     | validate a revised draft → proposal (never persists)              |
//! | [`EditWorkflowTool`]              | `None`     | mutate a draft in place (`add_node`/`remove_edge`/…) — never saves |
//! | [`ValidateWorkflowTool`]          | `None`     | read: run the full gate stack without emitting a proposal card    |
//! | [`GetFlowHistoryTool`]            | `None`     | read: a saved flow's prior graph snapshots                        |
//! | [`ListFlowRunsTool`]              | `None`     | read: a flow's run history                                        |
//! | [`ResumeFlowRunTool`]             | `Execute`  | advance a run parked on approval — approved nodes fire for real   |
//! | [`CancelFlowRunTool`]             | `Write`    | stop an in-flight/parked run; fires no new outbound effect        |
//! | [`CreateWorkflowTool`]            | `Write`    | persist a NEW flow — always born **DISABLED**                     |
//! | [`DuplicateFlowTool`]             | `Write`    | clone a saved flow — the copy is always born **DISABLED**         |
//! | [`ListConnectableToolkitsTool`]   | `None`     | read: which toolkits are already connected                        |
//! | [`ListFlowsTool`]                 | `None`     | read: list saved flows                                            |
//! | [`GetFlowTool`]                   | `None`     | read: fetch a saved flow's graph                                  |
//! | [`GetFlowRunTool`]                | `None`     | read: fetch a run's steps                                         |
//! | [`ListFlowConnectionsTool`]       | `None`     | read: connection refs (ids/names only)                            |
//! | [`SearchToolCatalogTool`]         | `None`     | read: real Composio tool slugs (live catalog)                     |
//! | [`GetToolContractTool`]           | `None`     | read: one action's FULL live contract                             |
//! | [`GetToolOutputSampleTool`]       | `ReadOnly` | ONE bounded real Composio call (Read-scope only, connected toolkit only) |
//! | [`ListAgentProfilesTool`]         | `None`     | read: selectable agent kinds (`agent_ref`)                        |
//! | [`ListNodeKindsTool`]             | `None`     | read: the DSL's node kinds                                        |
//! | [`GetNodeKindContractTool`]       | `None`     | read: one node kind's config/port/example/gotcha contract         |
//! | [`DryRunWorkflowTool`]            | `None`     | run a *draft* against MOCK capabilities — not tier-gated (F7)     |
//! | [`SaveWorkflowTool`]              | `Write`    | persist a graph onto an EXISTING flow                              |
//!
//! **Human-in-the-loop invariant.** Enabling a flow is not a tool this agent
//! has, by design, no matter which tool above it reaches for:
//! [`CreateWorkflowTool`] and [`DuplicateFlowTool`] always produce a
//! **DISABLED** flow, and [`SaveWorkflowTool`] never sets
//! `enabled`/`require_approval` (it CAN auto-disable an already-enabled flow
//! when the graph's trigger transitions from manual to automatic — see its
//! own doc). `revise_workflow` / `edit_workflow` / `validate_workflow` only
//! validate or mutate a draft and never persist. `dry_run_workflow` executes
//! only against `tinyflows`' deterministic **mock** capabilities, so no real
//! LLM / tool / HTTP / code side effect can fire from it regardless of tier
//! (F7 — it is deliberately NOT tier-gated; see its own doc).
//! [`ResumeFlowRunTool`] is the one place a non-persistence tool can cause a
//! real effect: resuming a parked run lets its already-approved nodes fire,
//! which is why it is `Execute`-gated rather than `None`/`Write`.
//!
//! The agent's full tool scope (see `agent_registry/agents/workflow_builder/
//! agent.toml`) also grants the Composio **discovery/connect** tools —
//! `composio_list_toolkits`, `composio_list_connections`, `composio_connect`
//! (defined in `composio/tools.rs`) — so the builder can link an app the
//! workflow needs before proposing. Those stay within the invariant: connect
//! is an approval-gated OAuth hand-off, and `composio_execute` (running an
//! arbitrary real action, any scope) remains deliberately OUT of scope.
//!
//! **One narrow, deliberate carve-out (B12):** [`GetToolOutputSampleTool`]
//! (`get_tool_output_sample`) DOES perform a real Composio call — but only
//! ever a `Read`-scope one (hard-refused otherwise, regardless of the user's
//! per-toolkit scope preference), and only against a toolkit the user has
//! ALREADY connected. It exists because some actions' live listings publish
//! no output schema at all (verified for every GitHub action), leaving
//! `get_tool_contract` with no ground truth for a downstream `split_out.path`
//! — this makes exactly one bounded real read to observe the actual shape
//! instead. It can never send/create/update/delete anything.

#[cfg(test)]
#[path = "builder_tools_tests.rs"]
mod tests;
include!("builder_tools_part_01.rs");
include!("builder_tools_part_02.rs");
include!("builder_tools_part_03.rs");
include!("builder_tools_part_04.rs");
include!("builder_tools_part_05.rs");
include!("builder_tools_part_06.rs");
include!("builder_tools_part_07.rs");
