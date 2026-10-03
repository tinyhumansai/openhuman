//! Business logic for the `flows::` domain: validate-on-save CRUD plus the
//! end-to-end `flows_run` / `flows_resume` path. Delegated to from
//! `schemas.rs`'s `handle_*` RPC/CLI handlers, mirroring
//! `crates/openhuman-core/src/cron/ops.rs`.
//!
//! The module is split by responsibility; every operation the rest of the
//! crate reaches through `flows::ops::` is re-exported from here so the
//! external paths are unchanged:
//!
//! | Submodule | Responsibility |
//! | --- | --- |
//! | `validation` | migrate/validate a raw graph, engine compatibility, `flows_validate` / `flows_import` |
//! | `builder_gates` | the author-time hard-gate stack, `strict_gate`, `build_builder_proposal` |
//! | `wiring_warnings` | advisory Composio wiring / output-field / `split_out.path` warnings |
//! | `inference_readiness` | B45 provider-connectivity evaluation + probe cache |
//! | `tool_contract_gate` | live-catalog slug / required-arg / arg-name gate |
//! | `connection_ref_gate` | WS3 `connection_ref` toolkit + existence gate |
//! | `definitions` | create / duplicate / get / list / delete + saved-graph lookups |
//! | `updates` | update / history / rollback with optimistic concurrency |
//! | `triggers` | enable/disable, trigger binding, boot reconciliation, trigger warnings |
//! | `connections` | the `connection_ref` picker list and required-connection CTAs |
//! | `run` | `flows_run` / `flows_run_detached` entry points and run prep |
//! | `execution` | the shared run body, origin, Langfuse export, approval notification |
//! | `resume` | `flows_resume` and the T-M1 graph pin |
//! | `run_rows` | `flow_runs` row persistence, drop-guard finalizer, step settlement |
//! | `run_management` | cancel, run listing/pruning, TTL and boot sweeps |
//! | `discovery` | Flow Scout discovery and suggestion lifecycle |
//! | `streaming` | copilot/scout progress bridge onto the web-channel socket |
//! | `builder_toolset` | tools hidden from the `workflow_builder` belt per path |
//! | `builder` | `flows_build` / `flows_build_cancel` and proposal extraction |
//! | `trail_off` | the builder-convergence question backstop |
//! | `approval_manifest` | save-time approval manifest |
//! | `catalog` | tool-catalog search / contract RPCs |
//! | `drafts` | core-managed local drafts |

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use chrono::Utc;
use serde_json::{json, Value};
use tinyflows::model::{NodeKind, TriggerKind, WorkflowGraph};
// The save/run safety predicates are `tinyflows-catalog`'s: whether a graph
// fires unattended, whether it can act on the world, whether it has anything to
// do at all are properties of the graph, not of this host. Re-exported at
// `ops::` scope because the agent tools and this module's tests already name
// them there.
pub(crate) use tinyflows_catalog::graph_policy::{
    enforce_side_effect_approval, graph_has_actionable_nodes, trigger_is_automatic,
};
use tokio_util::sync::CancellationToken;

use crate::agent::turn_origin::{with_origin, AgentTurnOrigin, TrustedAutomationSource};
use crate::config::Config;
use crate::core::Outcome;
use crate::flows::bus;
use crate::flows::draft_store;
use crate::flows::run_registry;
use crate::flows::store;
use crate::flows::types::{
    FlowConnection, FlowRunStep, FlowRunTrigger, FlowSuggestion, SuggestionStatus,
};
use crate::flows::{Flow, FlowRun};
use crate::security::approval::{
    ApprovalChatContext, FlowRunContext, APPROVAL_CHAT_CONTEXT, APPROVAL_COPILOT_STREAM_CONTEXT,
    APPROVAL_FLOW_RUN_CONTEXT,
};
use tinyflows_catalog::build_registry;

/// Overall safety bound on a single `flows_run` / `flows_resume`. Individual
/// capabilities have their own timeouts (HTTP, sandbox), but a hung LLM/tool
/// call must never let the RPC block indefinitely — this caps the whole run.
const FLOW_RUN_TIMEOUT_SECS: u64 = 600;

/// How long a run may sit parked at a human-in-the-loop approval gate
/// (`pending_approval`) before the TTL sweep expires it to a terminal
/// `"cancelled"` (issue G4). Aligned with the agent tool-call `ApprovalGate`'s
/// 10-minute fail-closed TTL (`crates/openhuman-core/src/security/approval/`), so a flow HITL gate a
/// human never answers doesn't wedge a run — and its durable checkpoint —
/// forever. The two are distinct mechanisms (flow runs execute as
/// `TrustedAutomation { Workflow }`, which the tool-call gate lets through), so
/// this is a dedicated flows-side TTL, not a reuse of the approval store's.
const FLOW_PARKED_TTL_SECS: i64 = 600;

/// T-M1 fail-closed refusal: the graph hash pinned when this run parked no
/// longer matches the flow's current graph (`save_workflow` rewrote it while
/// the approval sat pending). Distinct wording from every other
/// `flows_resume` rejection so the UI/agent can tell a stale-approval refusal
/// apart from an ordinary invalid-resume error and explain it plainly rather
/// than surfacing a generic "resume failed".
const GRAPH_CHANGED_SINCE_PARK_ERROR: &str = "the workflow changed after this run was paused — \
     the pending approval no longer matches the current graph";

// ─────────────────────────────────────────────────────────────────────────────
// Phase 2 — autonomy-tier gating of acting flow nodes
// ─────────────────────────────────────────────────────────────────────────────
//
// A `flows_run` / `flows_resume` executes under a `TrustedAutomation { Workflow }`
// origin (see `workflow_origin` below), but the *acting power* of a run is still
// bounded by the user's `[autonomy]` tier — the same `SecurityPolicy`
// (`crates/openhuman-core/src/security/`) the agent tool-loop honors, built via
// `SecurityPolicy::from_config(&config.autonomy, …)` inside
// `tinyflows::caps::build_capabilities`.
//
// Before an acting node dispatches, its capability adapter
// (`crates/openhuman-core/src/flows/tinyflows/caps.rs::enforce_node_tier_gate`) maps the node to a
// `CommandClass` and consults `SecurityPolicy::gate_decision`. `Block` refuses
// outright (`[policy-blocked]` error, no dispatch); `Prompt`/`Allow` fall through
// to the process-global `ApprovalGate`, which performs the human round-trip for
// `Prompt` exactly as the agent tool-loop does. Node → class → per-tier decision:
//
//   Flow node        CommandClass   read-only     supervised    full
//   ────────────     ────────────   ──────────    ──────────    ──────────
//   http_request     Network        BLOCK         Prompt        Prompt
//   code             Write          BLOCK         Prompt        Allow
//   tool_call        (curation +    (curated +    Prompt        Prompt/Allow¹
//                     ApprovalGate)   scope gate)
//   agent (llm)      — (no acting side effect; not tier-gated, only the
//                        inference/privacy chokepoint applies)
//   state (kv)       — (host-internal flow KV; not an outbound act)
//
//   ¹ tool_call routes through the deny-by-default curation/scope gate plus the
//     ApprovalGate rather than `gate_decision`; a Network-class Composio action
//     still prompts under supervised/full and the curation gate is the hard
//     allowlist. See `caps.rs::OpenHumanTools`.
//
// `Network` is never `Allow` in any tier (always `Prompt` when not blocked), so
// even a full-tier http_request node prompts unless a pre-declared trust root /
// `auto_approve` short-circuits the ApprovalGate — matching `curl`/`shell`.
// `Write` (code) is `Allow` under full, so trusted automations run sandboxed
// code unattended; read-only blocks both outright.

mod approval_manifest;
mod builder;
mod builder_gates;
mod builder_toolset;
mod catalog;
mod connection_ref_gate;
mod connections;
mod definitions;
mod discovery;
mod drafts;
mod execution;
mod inference_readiness;
mod resume;
mod run;
mod run_management;
mod run_rows;
mod streaming;
mod tool_contract_gate;
mod trail_off;
mod triggers;
mod updates;
mod validation;
mod wiring_warnings;

pub use approval_manifest::*;
pub use builder::*;
pub(crate) use builder_gates::*;
use builder_toolset::*;
pub use catalog::*;
pub(crate) use connection_ref_gate::*;
pub use connections::*;
pub use definitions::*;
pub use discovery::*;
pub use drafts::*;
use execution::*;
pub(crate) use inference_readiness::*;
pub use resume::*;
pub use run::*;
pub use run_management::*;
use run_rows::*;
pub use streaming::*;
pub(crate) use tool_contract_gate::*;
use trail_off::*;
pub use triggers::*;
pub use updates::*;
pub use validation::*;
pub(crate) use wiring_warnings::*;

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
