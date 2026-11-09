//! Business logic for the `flows::` domain: validate-on-save CRUD plus the
//! end-to-end `flows_run` / `flows_resume` path. Delegated to from
//! `schemas.rs`'s `handle_*` RPC/CLI handlers, mirroring
//! `crates/openhuman-core/src/cron/ops.rs`.
//!
//! The module is split by responsibility; host-owned operations used by RPC,
//! tools, and sibling flow modules are re-exported from here:
//!
//! | Submodule | Responsibility |
//! | --- | --- |
//! | `validation` | migrate/validate a raw graph, engine compatibility, `flows_validate` / `flows_import` |
//! | `builder_gates` | the author-time hard-gate stack, `strict_gate`, `build_builder_proposal` |
//! | `wiring_warnings` | TinyFlows output and split-path advisories via the scoped host contract callback |
//! | `inference_readiness` | B45 provider-connectivity evaluation + probe cache |
//! | `host_gates` | OpenHuman checks served to TinyFlows for one authoring call |
//! | `definitions` | create / duplicate / get / list / delete + saved-graph lookups |
//! | `updates` | update / history / rollback with optimistic concurrency |
//! | `triggers` | enable/disable, trigger binding, boot reconciliation, trigger warnings |
//! | `connections` | the `connection_ref` picker list and required-connection CTAs |
//! | `run` | `flows_run` / `flows_run_detached` entry points and run prep |
//! | `execution` | Langfuse export and approval notification |
//! | `resume` | `flows_resume` and the T-M1 graph pin |
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
use std::sync::Arc;

use serde_json::{json, Value};
// Graphs and Catalog/Runs records cross the module boundary as bus DTOs.
use crate::config::Config;
use crate::core::runtime::turn_origin::{with_origin, AgentTurnOrigin};
use crate::core::Outcome;
use crate::flows::bus;
use crate::flows::draft_store;
use crate::flows::store;
use crate::security::approval::{ApprovalChatContext, ApprovalTurnContext};
mod build_registry;
use tinyflows_bus::catalog::{FlowRun, FlowSuggestion, SuggestionStatus};
use tinyflows_bus::runs::FlowRunTrigger;
use tokio_util::sync::CancellationToken;

pub(crate) use tinyflows_bus::catalog::Workflow as Flow;

/// Overall safety bound on a single `flows_run` / `flows_resume`. Individual
/// capabilities have their own timeouts (HTTP, sandbox), but a hung LLM/tool
/// call must never let the RPC block indefinitely — this caps the whole run.
const FLOW_RUN_TIMEOUT_SECS: u64 = crate::flows::runs_adapter::RUN_TIMEOUT_SECS;

/// T-M1 fail-closed refusal: the graph hash pinned when this run parked no
/// longer matches the flow's current graph (`save_workflow` rewrote it while
// ─────────────────────────────────────────────────────────────────────────────
// Phase 2 — autonomy-tier gating of acting flow nodes
// ─────────────────────────────────────────────────────────────────────────────
//
// A `flows_run` / `flows_resume` executes under a trusted workflow origin, but
// its acting power is still bounded by the user's `[autonomy]` tier. The
// `CapsHost` adapter applies OpenHuman policy and approvals at the host
// callback; TinyFlows owns graph execution and lifecycle settlement.
//
// Before an acting node dispatches, its capability adapter
// (`crates/openhuman-core/src/flows/caps_host/`) maps the node to a
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
//     allowlist. See `caps_host::service_tools`.
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
mod connections;
mod definitions;
mod discovery;
mod drafts;
mod execution;
mod inference_readiness;
mod resume;
mod run;
mod run_management;
mod streaming;
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
pub use connections::*;
pub use definitions::*;
pub use discovery::*;
pub use drafts::*;
pub(crate) use execution::{export_run_to_langfuse, notify_pending_approval};
pub(crate) use inference_readiness::*;
pub use resume::*;
pub use run::*;
pub use run_management::*;
pub use streaming::*;
use trail_off::*;
pub use triggers::*;
pub use updates::*;
pub use validation::*;
pub(crate) use wiring_warnings::*;

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
