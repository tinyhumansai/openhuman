//! Business logic for the `flows::` domain: validate-on-save CRUD plus the
//! end-to-end `flows_run` / `flows_resume` path. Delegated to from
//! `schemas.rs`'s `handle_*` RPC/CLI handlers, mirroring
//! `src/openhuman/cron/ops.rs`.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tinyflows::model::{NodeKind, TriggerKind, WorkflowGraph};
use tokio_util::sync::CancellationToken;

use crate::openhuman::agent::turn_origin::{with_origin, AgentTurnOrigin, TrustedAutomationSource};
use crate::openhuman::config::Config;
use crate::openhuman::flows::build_registry;
use crate::openhuman::flows::bus;
use crate::openhuman::flows::draft_store;
use crate::openhuman::flows::run_registry;
use crate::openhuman::flows::store;
use crate::openhuman::flows::types::{
    FlowConnection, FlowRunStep, FlowRunTrigger, FlowSuggestion, SuggestionStatus,
};
use crate::openhuman::flows::{flow_namespace, Flow, FlowRun};
use crate::openhuman::memory::store::MemoryClientRef;
use crate::openhuman::security::approval::{
    ApprovalChatContext, FlowRunContext, APPROVAL_CHAT_CONTEXT, APPROVAL_COPILOT_STREAM_CONTEXT,
    APPROVAL_FLOW_RUN_CONTEXT,
};
use crate::rpc::RpcOutcome;

/// Overall safety bound on a single `flows_run` / `flows_resume`. Individual
/// capabilities have their own timeouts (HTTP, sandbox), but a hung LLM/tool
/// call must never let the RPC block indefinitely — this caps the whole run.
const FLOW_RUN_TIMEOUT_SECS: u64 = 600;

/// How long a run may sit parked at a human-in-the-loop approval gate
/// (`pending_approval`) before the TTL sweep expires it to a terminal
/// `"cancelled"` (issue G4). Aligned with the agent tool-call `ApprovalGate`'s
/// 10-minute fail-closed TTL (`src/openhuman/security/approval/`), so a flow HITL gate a
/// human never answers doesn't wedge a run — and its durable checkpoint —
/// forever. The two are distinct mechanisms (flow runs execute as
/// `TrustedAutomation { Workflow }`, which the tool-call gate lets through), so
/// this is a dedicated flows-side TTL, not a reuse of the approval store's.
const FLOW_PARKED_TTL_SECS: i64 = 600;

/// Stable host-validation code for a topology that the currently vendored
/// TinyFlows/TinyAgents barrier-relief implementation cannot execute safely.
const UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN: &str = "unsupported_nested_conditional_fan_in";
const UNSUPPORTED_MAIN_PORT_CONDITIONAL_FAN_IN: &str = "unsupported_main_port_conditional_fan_in";

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
// (`src/openhuman/security/`) the agent tool-loop honors, built via
// `SecurityPolicy::from_config(&config.autonomy, …)` inside
// `tinyflows::caps::build_capabilities`.
//
// Before an acting node dispatches, its capability adapter
// (`src/openhuman/flows/tinyflows/caps.rs::enforce_node_tier_gate`) maps the node to a
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

/// Runs a raw graph JSON value through `tinyflows::migrate::migrate` (upgrade
/// an older-schema definition to current), deserializes it, and rejects a
/// structurally invalid graph via `tinyflows::validate::validate` — so a bad
/// graph is caught at the door, before it's ever persisted.
///
/// `pub(crate)` (not private) so `flows::tools::ProposeWorkflowTool` (issue
/// B4 — agent-first workflow authoring) can run a candidate graph through the
/// exact same validate/migrate path `flows_create` uses below, without
/// duplicating it. The tool only calls this — never `flows_create` itself —
/// which is what keeps the "the agent can never create a flow" invariant
/// intact: this function validates and returns, it has no persistence effect.
pub(crate) fn validate_and_migrate_graph(graph_json: Value) -> Result<WorkflowGraph, String> {
    let graph = migrate_and_deserialize_graph(graph_json)?;
    tinyflows::validate::validate(&graph).map_err(|e| e.to_string())?;
    ensure_engine_compatible(&graph)?;
    Ok(graph)
}

/// Detects fan-in predecessors controlled by more than one branching decision.
///
/// TinyFlows lowers every fan-in edge as a waiting edge and registers a
/// barrier relief for conditional predecessors. The current lowering chooses
/// only the first upstream brancher, while TinyAgents cannot prove reachability
/// through a second brancher. Depending on node declaration order, that can
/// either relieve the barrier before the real predecessor runs (silently
/// dropping its data) or leave the fan-in unfired. Fail closed until the
/// vendored engine models nested decisions directly.
///
/// This intentionally mirrors TinyFlows' topology classification rather than
/// limiting the check to `merge` nodes: any node with multiple incoming edges
/// is lowered as a fan-in barrier. A predecessor reachable from the trigger by
/// `main`-only edges is unconditional and needs no relief, so it is safe.
pub(crate) fn engine_compatibility_errors(
    graph: &WorkflowGraph,
) -> Vec<crate::openhuman::flows::FlowValidationError> {
    engine_compatibility_errors_with_max_depth(graph, max_sub_workflow_depth(graph))
}

/// Same walk as [`engine_compatibility_errors`], but with the inline-nesting
/// budget passed in rather than recomputed from `graph`'s own trigger.
///
/// [`referenced_workflow_compatibility_errors`] needs this: a saved child
/// reached partway through the root's referenced-workflow chain must still be
/// checked to the *remaining* depth the root's own `max_sub_workflow_depth`
/// allows, not to the child's own (possibly lower/default) declared cap —
/// the engine's runtime depth counter is one budget shared across the whole
/// inline-plus-referenced call chain, so a fan-in the child's own cap would
/// not reach can still be reached from the root.
pub(crate) fn engine_compatibility_errors_with_max_depth(
    graph: &WorkflowGraph,
    max_depth: u64,
) -> Vec<crate::openhuman::flows::FlowValidationError> {
    let mut errors = Vec::new();
    collect_engine_compatibility_errors(graph, 0, max_depth, &mut errors);
    errors
}

/// The nesting cap this graph declares on its trigger, or the engine default.
///
/// The static walk below has to descend as deep as the run actually will, or a
/// graph that legitimately nests past the default would stop being checked
/// exactly where it starts being interesting.
pub(crate) fn max_sub_workflow_depth(graph: &WorkflowGraph) -> u64 {
    graph
        .trigger()
        .and_then(|t| t.config.get("max_sub_workflow_depth"))
        .and_then(serde_json::Value::as_u64)
        .filter(|n| *n > 0)
        .unwrap_or(tinyflows::engine::MAX_SUB_WORKFLOW_DEPTH)
}

fn collect_engine_compatibility_errors(
    graph: &WorkflowGraph,
    depth: u64,
    max_depth: u64,
    errors: &mut Vec<crate::openhuman::flows::FlowValidationError>,
) {
    errors.extend(graph_engine_compatibility_errors(graph));
    if depth >= max_depth {
        return;
    }

    for node in &graph.nodes {
        if node.kind != NodeKind::SubWorkflow {
            continue;
        }
        let Some(inline) = node.config.get("workflow") else {
            continue;
        };
        let Ok(child) = serde_json::from_value::<WorkflowGraph>(inline.clone()) else {
            // TinyFlows reports malformed inline children as capability errors;
            // this gate is specifically for otherwise-deserializable unsafe
            // topologies.
            continue;
        };
        let first_child_error = errors.len();
        collect_engine_compatibility_errors(&child, depth + 1, max_depth, errors);
        for error in &mut errors[first_child_error..] {
            error.message = format!("Inline sub_workflow node '{}': {}", node.id, error.message);
        }
    }
}

fn graph_engine_compatibility_errors(
    graph: &WorkflowGraph,
) -> Vec<crate::openhuman::flows::FlowValidationError> {
    let Some(trigger) = graph.trigger() else {
        return Vec::new();
    };
    let mut errors = Vec::new();

    // The edges that close a cycle, from the engine's own classifier rather
    // than a second implementation here — this gate mirrors TinyFlows' fan-in
    // lowering, so the two must agree on which edges count. A back-edge is a
    // loop head's re-entry, not a predecessor it barriers on, and counting it
    // would report every legal loop as an unrelieved fan-in.
    let loop_edges = tinyflows::engine::back_edges(graph);

    for fan_in in &graph.nodes {
        let incoming: Vec<&str> = graph
            .edges
            .iter()
            .filter(|edge| edge.to_node == fan_in.id)
            .filter(|edge| !loop_edges.contains(&(edge.from_node.clone(), edge.to_node.clone())))
            .map(|edge| edge.from_node.as_str())
            .collect();
        if incoming.len() <= 1 {
            continue;
        }

        for predecessor in incoming {
            // Reaching a router itself unconditionally does not make the edge
            // it selects into the fan-in unconditional. Let router
            // predecessors reach the port-aware analysis below.
            if !is_branching_node(graph, predecessor)
                && reaches_on_main_edges(graph, &trigger.id, predecessor, &fan_in.id)
            {
                continue;
            }

            let mut controlling_branchers = 0usize;
            let mut controlled_via_main_port = false;
            for candidate in &graph.nodes {
                let is_router = matches!(candidate.kind, NodeKind::Condition | NodeKind::Switch);
                let ports: HashSet<&str> = graph
                    .edges
                    .iter()
                    .filter(|edge| edge.from_node == candidate.id)
                    .map(|edge| edge.from_port.as_str())
                    .collect();
                if ports.len() < 2 && !is_router {
                    continue;
                }
                // When the router is itself the incoming predecessor, its
                // branch edge must be tested against the fan-in (asking whether
                // that edge reaches the router again can never succeed).
                let controlled_target = if candidate.id == predecessor {
                    fan_in.id.as_str()
                } else {
                    predecessor
                };
                let reaches_from_port = |port: &str| {
                    reaches_via_port(graph, &candidate.id, port, controlled_target, &fan_in.id)
                };
                let any_port_reaches = ports.iter().any(|port| reaches_from_port(port));
                // A router with one wired output still has unwired runtime
                // choices that emit no successor, so that sole edge cannot
                // prove unconditional reachability. Router reconvergence is
                // only deterministic when every runtime choice is wired:
                // both condition outcomes, or a switch fallback. Generic
                // multi-port nodes retain their existing all-port behavior.
                let routing_choices_are_exhaustive = match candidate.kind {
                    NodeKind::Condition => ports.contains("true") && ports.contains("false"),
                    NodeKind::Switch => ports.contains("default"),
                    _ => true,
                };
                let can_prove_all_routing_choices = if is_router {
                    routing_choices_are_exhaustive
                } else {
                    ports.len() >= 2
                };
                let every_port_deterministically_reaches = can_prove_all_routing_choices
                    && ports.iter().all(|port| {
                        reaches_deterministically_via_port(
                            graph,
                            &candidate.id,
                            port,
                            controlled_target,
                            &fan_in.id,
                        )
                    });
                // A multi-port node only controls this predecessor when the
                // predecessor is reachable from it but not guaranteed by a
                // deterministic path on every routing choice. This matches
                // TinyAgents' relief proof, which stops at another router.
                if any_port_reaches && !every_port_deterministically_reaches {
                    controlling_branchers += 1;
                    controlled_via_main_port |= ports.contains("main") && reaches_from_port("main");
                }
            }

            let (code, routing_kind) = if controlled_via_main_port {
                (
                    UNSUPPORTED_MAIN_PORT_CONDITIONAL_FAN_IN,
                    "a conditional branch labelled 'main'",
                )
            } else if controlling_branchers >= 2 {
                (
                    UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN,
                    "nested conditional routing",
                )
            } else {
                continue;
            };
            errors.push(crate::openhuman::flows::FlowValidationError {
                code: code.to_string(),
                message: format!(
                    "Fan-in node '{}' has predecessor '{}' behind {routing_kind}; \
                     this topology is temporarily unsupported because it can silently lose \
                     merged data. Flatten the conditional branch or join it before this fan-in.",
                    fan_in.id, predecessor
                ),
                node_id: Some(fan_in.id.clone()),
                field: None,
            });
        }
    }

    errors
}

fn ensure_engine_compatible(graph: &WorkflowGraph) -> Result<(), String> {
    match engine_compatibility_errors(graph).into_iter().next() {
        Some(error) => Err(format!("{}: {}", error.code, error.message)),
        None => Ok(()),
    }
}

/// Host-aware compatibility check, including saved descendants that graph-only
/// validation cannot inspect. Authoring boundaries use it before persistence;
/// execution boundaries use it before compiling a root run/resume or returning
/// a resolver graph, so an unsafe descendant cannot run after earlier effects.
fn ensure_config_aware_engine_compatible(
    config: &Config,
    graph: &WorkflowGraph,
) -> Result<(), String> {
    match config_aware_engine_compatibility_errors(config, graph)
        .into_iter()
        .next()
    {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn reaches_on_main_edges(graph: &WorkflowGraph, from: &str, to: &str, stop: &str) -> bool {
    if from == to {
        return true;
    }
    let mut stack: Vec<&str> = if is_branching_node(graph, from) {
        Vec::new()
    } else {
        graph
            .edges
            .iter()
            .filter(|edge| edge.from_node == from && edge.from_port == "main")
            .map(|edge| edge.to_node.as_str())
            .collect()
    };
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if node == to {
            return true;
        }
        if node == stop || !seen.insert(node) {
            continue;
        }
        // Port labels are arbitrary. A node with multiple distinct output
        // ports is runtime-selective even when one label happens to be `main`,
        // so nothing beyond it is unconditionally reachable.
        if is_branching_node(graph, node) {
            continue;
        }
        stack.extend(
            graph
                .edges
                .iter()
                .filter(|edge| edge.from_node == node && edge.from_port == "main")
                .map(|edge| edge.to_node.as_str()),
        );
    }
    false
}

fn is_branching_node(graph: &WorkflowGraph, node_id: &str) -> bool {
    graph.nodes.iter().any(|node| {
        node.id == node_id && matches!(node.kind, NodeKind::Condition | NodeKind::Switch)
    }) || graph
        .edges
        .iter()
        .filter(|edge| edge.from_node == node_id)
        .map(|edge| edge.from_port.as_str())
        .collect::<HashSet<_>>()
        .len()
        >= 2
}

fn reaches_via_port(
    graph: &WorkflowGraph,
    brancher: &str,
    port: &str,
    target: &str,
    stop: &str,
) -> bool {
    let mut stack: Vec<&str> = graph
        .edges
        .iter()
        .filter(|edge| edge.from_node == brancher && edge.from_port == port)
        .map(|edge| edge.to_node.as_str())
        .collect();
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if node == target {
            return true;
        }
        if node == stop || !seen.insert(node) {
            continue;
        }
        stack.extend(
            graph
                .edges
                .iter()
                .filter(|edge| edge.from_node == node)
                .map(|edge| edge.to_node.as_str()),
        );
    }
    false
}

fn reaches_deterministically_via_port(
    graph: &WorkflowGraph,
    brancher: &str,
    port: &str,
    target: &str,
    stop: &str,
) -> bool {
    graph
        .edges
        .iter()
        .filter(|edge| edge.from_node == brancher && edge.from_port == port)
        .any(|edge| reaches_on_main_edges(graph, &edge.to_node, target, stop))
}

/// Runs a raw graph JSON value through migration + deserialization **without**
/// the structural `validate` step. Splits the two so a caller that wants
/// *every* structural error (via `tinyflows::validate::validate_all`) can run
/// validation itself — a pre-validation failure here (unparseable JSON, an
/// unmigrateable schema) is genuinely a single error, whereas structural
/// validation can surface many at once.
pub(crate) fn migrate_and_deserialize_graph(graph_json: Value) -> Result<WorkflowGraph, String> {
    let migrated = tinyflows::migrate::migrate(graph_json).map_err(|e| e.to_string())?;
    let graph: WorkflowGraph = serde_json::from_value(migrated).map_err(|e| e.to_string())?;
    Ok(graph)
}

/// Maps a portable `tinyflows` [`ValidationError`](tinyflows::error::ValidationError)
/// into the host's structured [`FlowValidationError`], carrying its stable
/// `code`, anchoring `node_id`, and human `message`. One place so the mapping
/// stays consistent across `flows_validate` and the builder gate stack.
pub(crate) fn to_flow_validation_error(
    err: &tinyflows::error::ValidationError,
) -> crate::openhuman::flows::FlowValidationError {
    crate::openhuman::flows::FlowValidationError {
        code: err.code().to_string(),
        message: err.to_string(),
        node_id: err.node_id().map(str::to_string),
        field: None,
    }
}

/// The single canonical definition of the builder hard-gate stack: the
/// author-time gates that reject (not warn) a graph an agent must not propose
/// or persist — engine compatibility, binding-resolvability, agent-ref
/// resolvability, connection-ref, tool-contract, and required-arg
/// resolvability, in increasing cost order.
///
/// Returns an empty `Vec` when the graph passes; otherwise the first failing
/// gate's node-level error messages (short-circuiting, so an expensive later
/// gate never runs on a graph already known to be broken). Every plane that
/// gates an agent-authored graph — `build_builder_proposal` (propose / revise /
/// edit), `save_workflow`, and the `strict` create/update RPC path — routes
/// through here, so they cannot drift (audit F3: agent saves and UI saves used
/// to validate differently).
///
/// Assumes `graph` is already structurally valid (run
/// `validate_and_migrate_graph` / `validate_all` first) — these gates check
/// resolvability/contracts on a compilable graph.
///
/// Author-gate for `oh:storage_upload_file`: its literal `path` arg must be
/// workspace-relative. Uploads are confined to the agent workspace by the
/// runtime `resolve_upload_path` (a canonicalized path that escapes `action_dir`
/// is rejected), so an absolute path like `/tmp/report.html` or one climbing out
/// with `..` cannot work — it fails mid-run at the upload step. The prompt tells
/// the builder to use a relative path, but the model reliably ignores that and
/// copies an absolute path from a prior flow's example, so this enforces it in
/// code (a hard, actionable author-gate) rather than trusting the prose.
///
/// Only LITERAL paths are checked: a `=`-expression resolves from upstream data
/// at runtime and is out of scope here (the runtime check still applies). An
/// absent `path` is left to the required-arg gate.
pub(crate) fn validate_upload_paths(graph: &WorkflowGraph) -> Vec<String> {
    const UPLOAD_SLUG: &str = "oh:storage_upload_file";
    let mut errors = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::ToolCall {
            continue;
        }
        if node.config.get("slug").and_then(Value::as_str) != Some(UPLOAD_SLUG) {
            continue;
        }
        let Some(raw) = node
            .config
            .get("args")
            .and_then(|a| a.get("path"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let path = raw.trim();
        // Dynamic (resolved at runtime) or absent — not a literal we can check here.
        if path.is_empty() || path.starts_with('=') {
            continue;
        }
        let escapes_via_parent = path.split(['/', '\\']).any(|seg| seg == "..");
        if std::path::Path::new(path).is_absolute() || escapes_via_parent {
            errors.push(format!(
                "Node '{}': `oh:storage_upload_file` path `{path}` must be workspace-relative \
                 (e.g. `report.html`). Uploads are confined to the agent workspace, so an \
                 absolute path (`/tmp/...`, `/Users/...`) or one escaping with `..` is rejected \
                 at run time. Use a relative path, and have the producing node write the file to \
                 that same relative path.",
                node.id
            ));
        }
    }
    errors
}

pub(crate) async fn run_builder_gates(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    let compatibility_errors = config_aware_engine_compatibility_errors(config, graph);
    if !compatibility_errors.is_empty() {
        return compatibility_errors;
    }
    // Cheap, sync: a binding guaranteed to resolve null / wrong at runtime.
    let binding_errors = validate_binding_resolvability(graph);
    if !binding_errors.is_empty() {
        return binding_errors;
    }
    // Cheap, sync: an `oh:storage_upload_file` literal `path` that is absolute or
    // escapes the workspace. The runtime `resolve_upload_path` rejects it, but the
    // model reliably ignores the prompt's "use a workspace-relative path" rule and
    // copies an absolute `/tmp/...` path from prior flows, so enforce it in code.
    let upload_path_errors = validate_upload_paths(graph);
    if !upload_path_errors.is_empty() {
        return upload_path_errors;
    }
    // Cheap: an `agent` node's `agent_ref` that would hit the runtime's
    // `RegistryFallback` "unknown agent_ref" hard error mid-run. Almost always a
    // pure in-memory harness-registry lookup; only a ref that ISN'T a harness
    // definition falls through to a local config read (custom agent registry).
    let agent_ref_errors = validate_agent_refs(config, graph).await;
    if !agent_ref_errors.is_empty() {
        return agent_ref_errors;
    }
    // NOTE (B45 design correction, judge finding on live run 104aab90):
    // provider-connectivity (issue B45 — signed out, or a managed-backend
    // account with no provider API key configured) is deliberately NOT a
    // hard author gate here. It used to reject `propose_workflow` /
    // `edit_workflow` outright, which meant a graph whose only problem was
    // "not runnable yet" could never even be SHOWN to the user — the copilot
    // detected the problem, could not propose past it, and trailed off with
    // no proposal at all. `evaluate_inference_readiness` still runs (see
    // `build_builder_proposal` below) and surfaces `inference_status` /
    // `inference_message` as an ADVISORY warning on the proposal payload, so
    // authoring always succeeds and the UI can render a "connect your
    // provider" nudge alongside the built workflow. The hard rejection moved
    // to run time instead — see `validate_inference_readiness`'s use in
    // `run_flow_body`, which fails a real run cleanly before the engine
    // executes rather than blocking the author from ever seeing the graph.
    //
    // Async, live connection list: a tool_call whose `connection_ref` names the
    // wrong toolkit for its slug, or a connection id the user doesn't actually
    // have (WS3 — the transcript bug where a TIKTOK connection id was wired onto
    // Twitter/Gmail nodes and every author-time gate returned ok). Cheap:
    // one connection-list fetch, no per-node catalog round trips.
    let connection_ref_errors = validate_connection_refs(config, graph).await;
    if !connection_ref_errors.is_empty() {
        return connection_ref_errors;
    }
    // Async, live catalog: a tool_call whose slug isn't a real Composio action
    // or whose real required args aren't all wired.
    let contract_errors = validate_tool_contracts(config, graph).await;
    if !contract_errors.is_empty() {
        return contract_errors;
    }
    // Async, sandbox run: a required outbound arg that looks wired but resolves
    // null in a mock execution.
    validate_required_arg_resolvability(graph).await
}

/// Checks literal `workflow_id` children reachable from an authoring candidate.
///
/// Pure graph validation can recurse through inline children, but resolving a
/// saved child requires the host store. Keep that lookup in the config-aware
/// builder gate so strict RPC and agent-authored proposals/saves cannot bless a
/// parent that is already known to fail at execution. Dynamic `=` expressions,
/// missing ids, and store failures retain their existing runtime diagnostics;
/// this gate only rejects a saved graph whose topology is demonstrably unsafe.
fn referenced_workflow_compatibility_errors(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    // Descend as deep as the root graph declared it may nest, for the same
    // reason as the inline walk above.
    let max_depth = max_sub_workflow_depth(graph);
    let mut pending = vec![(graph.clone(), 0_u64, Vec::<String>::new())];
    // Record the shallowest visit, not just whether an id was seen. The same
    // child can be referenced by multiple branches; a deep DFS visit must not
    // suppress a later shallower visit that has more depth budget remaining.
    let mut visited_depths = std::collections::HashMap::<String, u64>::new();

    while let Some((current, depth, path)) = pending.pop() {
        if depth >= max_depth {
            continue;
        }

        for node in &current.nodes {
            if node.kind != NodeKind::SubWorkflow {
                continue;
            }

            let mut child_path = path.clone();
            child_path.push(node.id.clone());

            let inline = node.config.get("workflow");
            let configured_workflow_id = node
                .config
                .get("workflow_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty());
            // Structural validation requires exactly one source and runs before
            // this helper. Retain that precedence defensively if a future caller
            // passes an invalid graph directly: do not inspect either source as
            // though TinyFlows could choose between them at runtime.
            if inline.is_some() && configured_workflow_id.is_some() {
                continue;
            }

            if let Some(inline) = inline {
                if let Ok(child) = serde_json::from_value::<WorkflowGraph>(inline.clone()) {
                    pending.push((child, depth + 1, child_path.clone()));
                }
                continue;
            }

            let Some(workflow_id) = configured_workflow_id.filter(|id| !id.starts_with('=')) else {
                continue;
            };
            let child_depth = depth + 1;
            if visited_depths
                .get(workflow_id)
                .is_some_and(|seen_depth| *seen_depth <= child_depth)
            {
                continue;
            }
            visited_depths.insert(workflow_id.to_string(), child_depth);

            let Ok(Some(child)) = load_flow_graph(config, workflow_id) else {
                continue;
            };
            // Thread the root's remaining depth budget through, not the
            // child's own cap — see `engine_compatibility_errors_with_max_depth`'s
            // doc comment.
            let remaining_depth = max_depth.saturating_sub(child_depth);
            if let Some(error) = engine_compatibility_errors_with_max_depth(&child, remaining_depth)
                .into_iter()
                .next()
            {
                return vec![format!(
                    "Sub_workflow path '{}' references workflow_id '{}' with an unsupported \
                     engine topology: {}: {}",
                    child_path.join(" -> "),
                    workflow_id,
                    error.code,
                    error.message
                )];
            }
            pending.push((child, child_depth, child_path));
        }
    }

    Vec::new()
}

/// Returns the complete engine-topology gate for a graph in its host context.
/// The graph-only half covers inline descendants; the config-aware half follows
/// literal saved-workflow references. Authoring and execution boundaries share
/// this helper so neither can accept a graph the other must reject.
pub(crate) fn config_aware_engine_compatibility_errors(
    config: &Config,
    graph: &WorkflowGraph,
) -> Vec<String> {
    let direct = engine_compatibility_errors(graph);
    if !direct.is_empty() {
        return direct
            .into_iter()
            .map(|error| format!("{}: {}", error.code, error.message))
            .collect();
    }
    referenced_workflow_compatibility_errors(config, graph)
}

/// Strict-mode gate for the create/update RPC path (audit F3): validates
/// `graph_json` structurally (surfacing every error at once) and then runs the
/// same [`run_builder_gates`] the agent tools enforce, returning `Err` with a
/// combined, model-consumable message if anything fails.
///
/// The UI/RPC create/update path stays permissive by default (a human editing
/// on the canvas may save a work-in-progress graph); passing `strict: true`
/// opts that call into the *same* gates an agent save must pass, so the two
/// planes converge on one definition instead of diverging.
pub(crate) async fn strict_gate(config: &Config, graph_json: &Value) -> Result<(), String> {
    let graph = migrate_and_deserialize_graph(graph_json.clone())?;
    let structural = tinyflows::validate::validate_all(&graph);
    if !structural.is_empty() {
        let messages: Vec<String> = structural.iter().map(ToString::to_string).collect();
        return Err(format!(
            "strict validation failed — the graph is structurally invalid:\n{}",
            messages.join("\n")
        ));
    }
    let gate_errors = run_builder_gates(config, &graph).await;
    if !gate_errors.is_empty() {
        return Err(format!(
            "strict validation failed:\n{}",
            gate_errors.join("\n\n")
        ));
    }
    Ok(())
}

/// Runs the full builder hard-gate stack on an already structurally-valid
/// `graph` and, if it passes, builds the `workflow_proposal` payload the
/// propose/revise/edit tools all return.
///
/// The single home for the gate sequence (engine compatibility →
/// binding-resolvability → tool-contract → required-arg resolvability) plus
/// summary/warning assembly,
/// so `revise_workflow` and `edit_workflow` cannot drift. `retry_tool` names
/// the tool in the "fix … and call `<tool>` again" guidance so each caller's
/// error text points the agent back at the right tool.
///
/// `draft_id` / `flow_id` are OPTIONAL persistence-state context echoed onto
/// the payload (the draft this proposal's edit lives on, and the saved flow it
/// derives from / targets). The payload ALWAYS carries `"persisted": false` so
/// a proposal can never be mistaken for a save confirmation — the exact false
/// belief the WS2 audit caught (an agent read a proposal as "written onto the
/// saved flow"). Actual persistence only happens via `save_workflow` /
/// `create_workflow` / `flows_draft_promote`.
///
/// Returns `Ok(payload)` on success, or `Err(message)` with a
/// model-consumable, fix-and-retry error when a gate rejects the graph. The
/// caller is responsible for structural validation (`validate_and_migrate_graph`
/// / `validate_all`) *before* calling this — these gates assume a compilable
/// graph.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_builder_proposal(
    config: &Config,
    retry_tool: &str,
    name: &str,
    graph: &WorkflowGraph,
    require_approval: bool,
    revision: bool,
    instruction: Option<String>,
    draft_id: Option<String>,
    flow_id: Option<String>,
) -> Result<Value, String> {
    // The full builder hard-gate stack, run through the single canonical
    // runner so every proposal/save/strict-RPC path gates identically (F3).
    let gate_errors = run_builder_gates(config, graph).await;
    if !gate_errors.is_empty() {
        return Err(format!(
            "{}\n\nFix these and call {retry_tool} again.",
            gate_errors.join("\n\n")
        ));
    }

    let summary = crate::openhuman::flows::tools::build_summary(graph);
    let mut warnings = graph_trigger_warnings(graph);
    warnings.extend(graph_wiring_warnings(config, graph).await);
    // Connector onboarding (Phase 5, item 18): tell the proposal card which
    // toolkits this graph needs and whether they're connected, so it can render
    // "Connect <toolkit>" CTAs instead of a bare gate error later.
    let required_connections = compute_required_connections(config, graph).await;
    // B45 (design correction): the LLM-provider-connectivity evaluation is
    // ADVISORY here, never a rejection — `run_builder_gates` above no longer
    // includes it (that used to hard-block `propose_workflow`/`edit_workflow`
    // on a graph the copilot couldn't then show the user at all — judge
    // finding on live run 104aab90). So `evaluation.status` here can
    // legitimately be `"ready"`, `"signed_out"`, `"provider_not_configured"`,
    // or `"error"` — the UI renders a "Connect a provider" / "Sign in" CTA
    // for the non-ready cases, alongside the toolkit-connection CTAs above.
    // The graph is proposed regardless of this value. Computed via the same
    // shared, cached evaluator the run-time preflight (`validate_inference_readiness`
    // in `run_flow_body`) consumes, so a run right after this proposal reads
    // the cached result instead of re-probing the network.
    let inference_readiness = evaluate_inference_readiness(config, graph).await;
    let graph_value = serde_json::to_value(graph).map_err(|e| e.to_string())?;

    tracing::info!(
        target: "flows",
        %name,
        node_count = graph.nodes.len(),
        require_approval,
        warning_count = warnings.len(),
        revision,
        "[flows] build_builder_proposal: proposal ready for user review"
    );

    let mut payload = json!({
        "type": "workflow_proposal",
        "revision": revision,
        // A proposal is NEVER a persisted flow — it is a candidate the user
        // still has to accept/save. Stamp this unconditionally so the payload
        // can't be misread as a save confirmation (WS2 audit).
        "persisted": false,
        "name": name,
        "graph": graph_value,
        "require_approval": require_approval,
        "summary": summary,
        "warnings": warnings,
        "required_connections": required_connections,
    });
    // Only present when the graph has at least one applicable `agent` node;
    // a tool_call-only graph omits both fields entirely rather than claiming
    // a meaningless "ready".
    if let Some(evaluation) = inference_readiness {
        payload["inference_status"] = json!(evaluation.status);
        if let Some(message) = evaluation.message {
            payload["inference_message"] = json!(message);
        }
    }
    if let Some(instruction) = instruction {
        payload["instruction"] = json!(instruction);
    }
    // Echo the persistence-state handles so the agent can iterate/persist
    // against the right ids (the draft the edit lives on; the flow it targets).
    if let Some(draft_id) = draft_id {
        payload["draft_id"] = json!(draft_id);
    }
    if let Some(flow_id) = flow_id {
        payload["flow_id"] = json!(flow_id);
    }
    Ok(payload)
}

/// Stable snake_case label for a [`TriggerKind`], matching its serde wire
/// discriminator — used in loud author-facing warnings (not derived via serde
/// so the exact human string is unmistakable at the call site).
fn trigger_kind_label(kind: &TriggerKind) -> &'static str {
    match kind {
        TriggerKind::Manual => "manual",
        TriggerKind::Schedule => "schedule",
        TriggerKind::Webhook => "webhook",
        TriggerKind::AppEvent => "app_event",
        TriggerKind::Form => "form",
        TriggerKind::ExecuteByWorkflow => "execute_by_workflow",
        TriggerKind::ChatMessage => "chat_message",
        TriggerKind::Evaluation => "evaluation",
        TriggerKind::System => "system",
    }
}

/// Whether a flow's trigger kind currently produces *automatic* runs in this
/// host. Only three kinds fire today:
/// - `manual` — runnable on demand via `flows_run` (no automatic dispatch, but
///   that's the whole contract of a manual trigger — never a surprise).
/// - `schedule` — a `cron` job drives `FlowScheduleTick` (see
///   [`bind_schedule_trigger`]).
/// - `app_event` — matched against `ComposioTriggerReceived` at dispatch time
///   (see `flows::bus::FlowTriggerSubscriber`).
///
/// Everything else (`webhook`, `chat_message`, `form`, `execute_by_workflow`,
/// `evaluation`, `system`) is *accepted and saved* but has no wired dispatch
/// path yet — enabling such a flow silently produces a flow that never runs
/// itself. [`graph_trigger_warnings`] turns that silence into a loud warning.
fn trigger_kind_fires(kind: &TriggerKind) -> bool {
    matches!(
        kind,
        TriggerKind::Manual | TriggerKind::Schedule | TriggerKind::AppEvent
    )
}

/// Whether `graph`'s trigger fires **without a human in the loop** — i.e. on
/// a timer, an inbound webhook, or a connected-app event, as opposed to
/// `manual` (only ever fired by an explicit `flows_run`). Used by
/// [`flows_create`] (issue B29 — save/enable safety, Rule 1) to decide
/// whether a freshly-saved flow may persist `enabled: true` or must persist
/// `enabled: false` until the user arms it explicitly via
/// `flows_set_enabled`.
///
/// Deliberately broader than [`trigger_kind_fires`]: `webhook` is not yet
/// wired to auto-dispatch in this host (see that fn's doc), but it WILL fire
/// unattended the moment it is — so a webhook-trigger flow must not be handed
/// to the user pre-armed either. Returns `false` for a graph with no single
/// resolvable trigger node or no `trigger_kind` discriminator (never a
/// surprise — it never self-fires).
pub(crate) fn trigger_is_automatic(graph: &WorkflowGraph) -> bool {
    let Some(trigger) = graph.trigger() else {
        return false;
    };
    let Some(kind_value) = trigger.config.get("trigger_kind") else {
        return false;
    };
    let Ok(kind) = serde_json::from_value::<TriggerKind>(kind_value.clone()) else {
        return false;
    };
    matches!(
        kind,
        TriggerKind::Schedule | TriggerKind::AppEvent | TriggerKind::Webhook
    )
}

/// Whether `graph` contains a node that can produce a real outbound side
/// effect — `tool_call` (a curated integration action), `http_request`, or
/// `code` (sandboxed but Turing-complete, can reach the network). Used by
/// [`flows_create`] (issue B29, Rule 2) to force `require_approval: true` on
/// any graph that can act on the world, regardless of what the caller
/// passed. A graph built only from `trigger` / `agent` / `transform` /
/// `condition` / data-flow nodes is read-only and unaffected.
pub(crate) fn graph_has_outbound_side_effect(graph: &WorkflowGraph) -> bool {
    graph.nodes.iter().any(|n| {
        matches!(
            n.kind,
            NodeKind::ToolCall | NodeKind::HttpRequest | NodeKind::Code
        )
    })
}

/// Shared Rule 2 enforcement (issue B29, and its `flows_update` compound-bypass
/// closure): forces `require_approval` to `true` when `graph` contains an
/// outbound side-effect node, no matter what the caller asked for. Used by both
/// [`flows_create`] and [`flows_update`] so a flow can never persist
/// `require_approval: false` alongside a `tool_call` / `http_request` / `code`
/// node — on create OR on a later edit that *adds* such a node to a
/// previously-read-only graph.
///
/// Returns `(effective_require_approval, was_forced)`: `was_forced` is `true`
/// only when the caller's own toggle was `false` but a side-effect node
/// required the override — callers use it to decide whether to emit the
/// loud "forced to true" log/result note.
pub(crate) fn enforce_side_effect_approval(
    graph: &WorkflowGraph,
    caller_require_approval: bool,
) -> (bool, bool) {
    let has_side_effect = graph_has_outbound_side_effect(graph);
    let effective_require_approval = caller_require_approval || has_side_effect;
    let was_forced = has_side_effect && !caller_require_approval;
    (effective_require_approval, was_forced)
}

/// Whether `graph` has anything for [`flows_run`] to actually *do* — i.e. at
/// least one non-`trigger` node **reachable from the trigger** by following
/// directed edges. A graph made of nothing but a bare `trigger` node (or a
/// `trigger` plus unreachable/disconnected nodes — even ones wired to each
/// other by their own edges, just not to the trigger) can compile and "run"
/// cleanly while producing no work whatsoever — the exact live finding this
/// guards: a trigger-only flow reported `status="completed"
/// pending_approvals=0` having done nothing, which reads as a successful
/// automation to anyone not staring at the node count. Used by `flows_run`
/// to attach a human-readable note to an otherwise-silent "success".
///
/// Deliberately a reachability walk rather than "any edge at all exists":
/// `nodes.len() > 1 && !edges.is_empty()` would count a disconnected
/// component's internal edges as actionable even though nothing downstream
/// of the trigger ever runs.
pub(crate) fn graph_has_actionable_nodes(graph: &WorkflowGraph) -> bool {
    let Some(trigger) = graph.trigger() else {
        // No single resolvable trigger to walk from — fall back to the
        // coarse "any non-trigger node wired up by an edge" check so a
        // malformed/ambiguous-trigger graph doesn't spuriously suppress the
        // empty-flow note.
        return graph.nodes.iter().any(|n| n.kind != NodeKind::Trigger) && !graph.edges.is_empty();
    };

    let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut stack = vec![trigger.id.as_str()];
    while let Some(current) = stack.pop() {
        if !visited.insert(current) {
            continue;
        }
        for next in graph.successors(current) {
            if !visited.contains(next) {
                stack.push(next);
            }
        }
    }

    visited
        .into_iter()
        .filter_map(|id| graph.node(id))
        .any(|n| n.kind != NodeKind::Trigger)
}

/// Produces host-side, **non-fatal** validation warnings for a graph — today
/// exactly one: "this trigger kind does not fire automatically yet". Returns
/// an empty vec when the trigger fires (`manual`/`schedule`/`app_event`), when
/// the graph has no single resolvable trigger node, or when the trigger has no
/// `trigger_kind` discriminator (a legacy/manual-only graph authored before
/// B2 simply never self-fires — not a warnable surprise, matching
/// `bus::extract_trigger_kind`'s "no automatic binding" treatment).
///
/// This lives host-side (NOT in `tinyflows::validate`, which is host-agnostic
/// and only does structural checks) because "which trigger kinds this host has
/// wired" is an OpenHuman fact, not a property of the portable graph.
pub(crate) fn graph_trigger_warnings(graph: &WorkflowGraph) -> Vec<String> {
    let Some(trigger) = graph.trigger() else {
        return Vec::new();
    };
    let Some(kind_value) = trigger.config.get("trigger_kind") else {
        return Vec::new();
    };
    let kind: TriggerKind = match serde_json::from_value(kind_value.clone()) {
        Ok(k) => k,
        Err(_) => return Vec::new(),
    };
    if trigger_kind_fires(&kind) {
        return Vec::new();
    }
    let label = trigger_kind_label(&kind);
    vec![format!(
        "Trigger kind '{label}' does not fire automatically yet — this flow will be saved and \
         can be enabled, but nothing will run it on its own until that trigger is wired up. Run \
         it manually with flows_run, or switch to a `schedule` or `app_event` trigger."
    )]
}

/// Author-time wiring warnings for Composio `tool_call` nodes: flags every
/// **required** arg (per the action's schema, best-effort cached lookup) that
/// is absent or a literal `null` in `config.args` — the exact mis-wiring that
/// would later fail the run's required-arg preflight.
///
/// Static by design: an arg carrying an `=`-expression counts as wired (only
/// the runtime preflight can tell whether it resolves), a `=`-derived slug is
/// skipped (can't know the action), and native `oh:` tools are skipped (no
/// Composio schema). Best-effort like the runtime preflight — no schema, no
/// warning, never a block.
pub(crate) async fn graph_wiring_warnings(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::openhuman::flows::tinyflows::caps::{composio_required_args, missing_required_args};

    let mut warnings = Vec::new();
    for node in &graph.nodes {
        if node.kind != tinyflows::model::NodeKind::ToolCall {
            continue;
        }
        let Some(slug) = node.config.get("slug").and_then(Value::as_str) else {
            continue;
        };
        // `=`-derived slugs are resolved at runtime; native tools have no
        // Composio schema to check against.
        if slug.starts_with('=') || slug.starts_with("oh:") {
            continue;
        }
        let Some(required) = composio_required_args(config, slug).await else {
            tracing::debug!(target: "flows", node = %node.id, %slug, "[flows] wiring check: no schema — skipping node");
            continue;
        };
        let args = node.config.get("args").cloned().unwrap_or(Value::Null);
        for missing in missing_required_args(&required, &args) {
            tracing::warn!(
                target: "flows",
                node = %node.id,
                %slug,
                arg = %missing,
                "[flows] wiring check: required arg not wired"
            );
            warnings.push(format!(
                "Node '{}': required arg `{missing}` of `{slug}` is not wired — set \
                 args.{missing}, e.g. \"=nodes.<upstream_id>.item.json.<field>\" (an agent \
                 feeding this value needs an output schema — `output_parser.schema` — so its \
                 fields are addressable).",
                node.id
            ));
        }
    }

    warnings.extend(graph_output_field_warnings(config, graph).await);
    warnings.extend(graph_split_out_path_warnings(config, graph).await);
    warnings
}

/// Author-time WARN (systemic tool-contract fix, Part 2c): any
/// `=nodes.<id>.item.json.data.<field>` binding — anywhere in the graph, not
/// just `tool_call` args — whose `<id>` names a `tool_call` node calling a
/// REAL Composio action with a KNOWN live output schema, but whose `<field>`
/// is not one of that action's real `output_fields`. Also warns (a distinct
/// message) when the binding is missing the `data.` segment entirely — a
/// Composio `tool_call`'s real runtime output always wraps its payload in
/// `data` (`ComposioExecuteResponse`; see
/// [`crate::openhuman::flows::tinyflows::caps::ToolContract::output_fields`]'s doc),
/// so `=nodes.<id>.item.json.<field>` (no `data.`) is GUARANTEED to resolve
/// `null` even when `<field>` names a real output field — that used to be
/// silently accepted here (B1: the exact bug that produces a hollow run).
/// Advisory, not fatal: a binding to an unknown field could still resolve to
/// something useful at runtime for an action whose output schema is
/// incomplete, so this warns rather than rejects — mirroring
/// `graph_wiring_warnings`'s existing required-arg warnings.
///
/// Skipped entirely when the referenced action's output schema is
/// **unknown** (`ToolContract::output_schema` is `None`) — there is nothing
/// real to check the field against, so warning would just be noise (or a
/// false positive for a still-legitimate binding). Also skipped for a
/// binding that dereferences `.item.<field>` without `.json` on an
/// enveloping node — that shape is already a HARD reject in
/// [`validate_binding_resolvability`], not a warning here.
///
/// Also skipped for a binding that addresses the whole payload
/// (`=nodes.<id>.item.json.data`, e.g. as an agent `input_context`) or one
/// of `ComposioExecuteResponse`'s OTHER top-level envelope fields —
/// `successful`, `error`, `costUsd`, `markdownFormatted` — which live
/// alongside `data`, not inside it. `OpenHumanTools::invoke` serializes the
/// whole `ComposioExecuteResponse` verbatim, so these ARE real
/// `.item.json.<x>` fields with no `data.` prefix; flagging them as
/// "missing the `data.` segment" would rewire an already-correct binding to
/// a nonsense path (e.g. suggesting `.item.json.data.successful`).
async fn graph_output_field_warnings(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::openhuman::flows::tinyflows::caps::fetch_live_toolkit_catalog;
    use crate::openhuman::memory::sync::composio::providers::toolkit_from_slug;

    let mut warnings = Vec::new();
    for node in &graph.nodes {
        for (location, expr) in collect_expressions(&node.config) {
            let Some((ref_id, has_json, field_path)) = parse_node_binding(&expr) else {
                continue;
            };
            if !has_json {
                continue;
            }
            let Some(ref_node) = graph.node(&ref_id) else {
                continue;
            };
            if ref_node.kind != NodeKind::ToolCall {
                continue;
            }
            let Some(ref_slug) = ref_node.config.get("slug").and_then(Value::as_str) else {
                continue;
            };
            if ref_slug.starts_with('=') || ref_slug.starts_with("oh:") {
                continue;
            }
            let Some(ref_toolkit) = toolkit_from_slug(ref_slug) else {
                continue;
            };
            let Some(catalog) = fetch_live_toolkit_catalog(config, &ref_toolkit).await else {
                continue;
            };
            let Some(contract) = catalog
                .iter()
                .find(|c| c.slug.eq_ignore_ascii_case(ref_slug))
            else {
                continue;
            };
            // B12: a real-output probe (`get_tool_output_sample`) for this
            // exact slug overrides the schema-derived `output_fields` — most
            // relevant for an action whose live listing publishes no output
            // schema at all (e.g. every GitHub action, verified live).
            let contract =
                crate::openhuman::flows::tinyflows::caps::apply_probe_override(contract.clone());
            // Nothing real to check `field_path` against — schema unknown AND
            // no probed output fields either.
            if contract.output_schema.is_none() && contract.output_fields.is_empty() {
                continue;
            }

            // Whole-payload access (`.item.json.data`, e.g. an agent's
            // `input_context`) or one of `ComposioExecuteResponse`'s OTHER
            // top-level envelope fields — these live alongside `data`, not
            // inside it, and are real fields regardless of this action's
            // `output_fields` (see this fn's doc). Not a "missing `data.`"
            // mistake.
            const COMPOSIO_ENVELOPE_METADATA_FIELDS: &[&str] =
                &["successful", "error", "costUsd", "markdownFormatted"];
            if field_path == "data"
                || COMPOSIO_ENVELOPE_METADATA_FIELDS
                    .contains(&field_path.split('.').next().unwrap_or(&field_path))
            {
                continue;
            }

            // A real Composio tool_call's payload is always nested one level
            // under `data` (see this fn's doc) — a binding missing that
            // segment is wrong regardless of whether the rest of the path
            // happens to name a real field.
            let Some(field) = field_path.strip_prefix("data.") else {
                tracing::warn!(
                    target: "flows",
                    node = %node.id,
                    %location,
                    ref_node = %ref_id,
                    ref_slug,
                    %field_path,
                    "[flows] wiring check: downstream binding is missing the Composio `data.` wrapper segment"
                );
                warnings.push(format!(
                    "Node '{}': binding `{location}` (`{expr}`) reads `.item.json.{field_path}` off \
                     tool_call `{ref_id}` (`{ref_slug}`), but a Composio tool_call's real output \
                     wraps its payload in `data` — this resolves null at runtime. Bind via \
                     `=nodes.{ref_id}.item.json.data.{field_path}` instead.",
                    node.id
                ));
                continue;
            };
            let field = field.split('.').next().unwrap_or(field);
            if !contract.output_fields.iter().any(|f| f == field) {
                tracing::warn!(
                    target: "flows",
                    node = %node.id,
                    %location,
                    ref_node = %ref_id,
                    ref_slug,
                    %field,
                    output_fields = ?contract.output_fields,
                    "[flows] wiring check: downstream binding reads a field not in the tool's real output_fields"
                );
                warnings.push(format!(
                    "Node '{}': binding `{location}` (`{expr}`) reads field `{field}` off \
                     tool_call `{ref_id}` (`{ref_slug}`), but that is not one of its real \
                     output fields ({}) — call get_tool_contract {{ slug: \"{ref_slug}\" }} to \
                     see the real output field names.",
                    node.id,
                    contract.output_fields.join(", "),
                ));
            }
        }
    }
    warnings
}

/// Given a Composio action's payload-only `output_schema` (see
/// [`crate::openhuman::flows::tinyflows::caps::ToolContract::output_fields`]'s doc —
/// NEVER includes the runtime `data` envelope) and a `split_out.path`
/// addressed relative to the ENVELOPE (`json.<envelope_field…>`, e.g.
/// `"json.data"` or `"json.data.issues"`), resolves whether the path lands on
/// something that is DEFINITELY not an array.
///
/// `Some(true)` — non-array (an object or scalar): a `split_out` over this
/// path fans out over exactly ONE item, the classic "wrong array path"
/// signal [`graph_split_out_path_warnings`]'s generic enforcement flags.
/// `Some(false)` — array: the path is fine. `None` — the path can't be
/// resolved against the schema at all (an unpublished/unknown nested field,
/// or a path missing the `data.` segment entirely) — stay silent rather than
/// guess; that's a distinct failure mode from "resolves to a non-array".
fn schema_says_path_is_non_array(output_schema: &Value, configured_path: &str) -> Option<bool> {
    let relative = configured_path
        .strip_prefix("json.")
        .unwrap_or(configured_path);
    if relative == "data" {
        // Whole-payload access (`json.data`) — non-array unless the payload's
        // own root schema type is literally "array" (a bare-array response,
        // e.g. a REST endpoint that returns `[...]` directly), in which case
        // `json.data` legitimately IS the real list.
        let ty = output_schema.get("type").and_then(Value::as_str)?;
        return Some(ty != "array");
    }
    let rest = relative.strip_prefix("data.").filter(|r| !r.is_empty())?;
    let mut node = output_schema;
    for seg in rest.split('.') {
        node = node.get("properties")?.get(seg)?;
    }
    let ty = node.get("type").and_then(Value::as_str)?;
    Some(ty != "array")
}

/// Author-time WARN/suggest (systemic tool-contract fix, Part 2d, extended by
/// B12): a `split_out` node whose direct predecessor is a `tool_call` calling
/// a REAL Composio action, checked two ways:
///
/// 1. **KNOWN `primary_array_path`** (see
///    [`crate::openhuman::flows::tinyflows::caps::compute_composio_array_path`] —
///    this already bakes in the `data.` segment Composio's execute-response
///    wrapper adds, so `expected` below comes out `"json.data.<…>"` with no
///    extra handling needed here — and, via
///    [`crate::openhuman::flows::tinyflows::caps::apply_probe_override`], a real
///    `get_tool_output_sample` probe for this slug overrides a schema that
///    never named an array at all): if the configured `config.path` doesn't match the
///    `json.<primary_array_path>` convention, suggest the real path.
/// 2. **UNKNOWN `primary_array_path`, but a KNOWN `output_schema`/probe that
///    proves the configured path is definitely NOT an array** (B12
///    enforcement, "regardless" of whether a correct path can be suggested —
///    catches the class at build time even when nothing to suggest is
///    derivable): warn generically. This is exactly the live bug this fix
///    closes — `GITHUB_LIST_REPOSITORY_ISSUES` publishes no output schema at
///    all, so a builder without a probe guessed the whole-payload
///    `"json.data"`, silently fanning out over ONE item (the `{issues:
///    [...]}` container) instead of the real per-issue list.
///
/// Both are advisory: a mismatched/non-array path degrades the fan-out (or
/// silently produces one item instead of many) rather than crashing.
///
/// Skipped entirely when `split_out`'s predecessor isn't a `tool_call` at all
/// (no envelope/array-path convention applies), or when NEITHER a
/// `primary_array_path` NOR an `output_schema` is known (truly nothing to
/// check against).
async fn graph_split_out_path_warnings(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::openhuman::flows::tinyflows::caps::{
        apply_probe_override, fetch_live_toolkit_catalog,
    };
    use crate::openhuman::memory::sync::composio::providers::toolkit_from_slug;

    let mut warnings = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::SplitOut {
            continue;
        }
        let configured_path = node.config.get("path").and_then(Value::as_str);

        for edge in graph.edges.iter().filter(|e| e.to_node == node.id) {
            let Some(pred) = graph.node(&edge.from_node) else {
                continue;
            };
            if pred.kind != NodeKind::ToolCall {
                continue;
            }
            let Some(pred_slug) = pred.config.get("slug").and_then(Value::as_str) else {
                continue;
            };
            if pred_slug.starts_with('=') || pred_slug.starts_with("oh:") {
                continue;
            }
            let Some(pred_toolkit) = toolkit_from_slug(pred_slug) else {
                continue;
            };
            let Some(catalog) = fetch_live_toolkit_catalog(config, &pred_toolkit).await else {
                continue;
            };
            let Some(contract) = catalog
                .iter()
                .find(|c| c.slug.eq_ignore_ascii_case(pred_slug))
            else {
                continue;
            };
            // B12: a real-output probe overrides the schema-derived
            // `primary_array_path` for this exact slug when one is cached.
            let contract = apply_probe_override(contract.clone());

            match contract.primary_array_path.as_deref() {
                Some(primary) => {
                    let expected = format!("json.{primary}");
                    if configured_path != Some(expected.as_str()) {
                        tracing::warn!(
                            target: "flows",
                            node = %node.id,
                            predecessor = %pred.id,
                            pred_slug,
                            configured_path,
                            %expected,
                            "[flows] wiring check: split_out.path does not match the predecessor tool's real array path"
                        );
                        let configured_display = configured_path
                            .map(|p| format!("\"{p}\""))
                            .unwrap_or_else(|| "unset".to_string());
                        warnings.push(format!(
                            "Node '{}': split_out.path is {configured_display} but its predecessor \
                             tool_call `{}` (`{pred_slug}`) wraps its real array at `{expected}` — set \
                             config.path to \"{expected}\" to fan out over the actual response list.",
                            node.id, pred.id,
                        ));
                    }
                }
                // No known array anywhere in this action's real output — the
                // generic non-array enforcement is the only thing left that
                // can catch a wrong path here (nothing to suggest, but a
                // known-non-array hit is still a strong signal).
                None => {
                    let Some(cp) = configured_path else { continue };
                    let Some(schema) = contract.output_schema.as_ref() else {
                        continue;
                    };
                    if schema_says_path_is_non_array(schema, cp) == Some(true) {
                        tracing::warn!(
                            target: "flows",
                            node = %node.id,
                            predecessor = %pred.id,
                            pred_slug,
                            configured_path = cp,
                            "[flows] wiring check: split_out.path resolves to a non-array — likely the wrong array path"
                        );
                        warnings.push(format!(
                            "Node '{}': split_out.path is \"{cp}\" but tool_call `{}` (`{pred_slug}`)'s \
                             known real output does not name an array at that path (or names no array \
                             property at all) — this fans out over a single object instead of a real \
                             list. If the action's real output nests the list under a named field (e.g. \
                             `data.issues`), call get_tool_output_sample {{ slug: \"{pred_slug}\" }} to \
                             sample the real response, then re-check with get_tool_contract.",
                            node.id, pred.id,
                        ));
                    }
                }
            }
        }
    }
    warnings
}

// ─────────────────────────────────────────────────────────────────────────────
// Enforcing binding-resolvability gate
// ─────────────────────────────────────────────────────────────────────────────
//
// `graph_wiring_warnings` (above) is advisory — it, and `dry_run_workflow`'s
// null-resolution check (issue #4586), only WARN the author that a binding
// resolves null. Neither is consulted by the builder before it proposes or
// saves a graph, so a warned-about-but-ignored binding still ships. The
// functions below are the HARD counterpart: `validate_binding_resolvability`
// statically proves a `tool_call` node's `args` bindings are resolvable
// *before* `propose_workflow`/`revise_workflow`/`save_workflow` accept the
// graph at all (see their call sites), so the LLM builder is forced to fix
// the wiring rather than merely being told about it.

/// Node kinds whose real capability adapter wraps its structured output in
/// the stable `{ json, text, raw }` envelope (`src/openhuman/flows/tinyflows/caps.rs`):
/// a binding into one of these must dereference `.item.json.<field>`, never
/// `.item.<field>` directly — the latter reads the envelope wrapper itself
/// (an object with `json`/`text`/`raw` keys), not the field inside it, and
/// resolves `null` at runtime. Every other node kind (`code`, `transform`,
/// `split_out`, `merge`, `output_parser`, `sub_workflow`, `trigger`,
/// `condition`, `switch`) emits its item directly with no envelope, so no
/// convention applies to a binding that targets one of them.
const ENVELOPING_KINDS: &[NodeKind] = &[NodeKind::Agent, NodeKind::ToolCall, NodeKind::HttpRequest];

/// Recursively collects every `=`-prefixed expression leaf in a config
/// `Value` tree, paired with its dotted location (array elements as numeric
/// segments, e.g. `"args.cc.0"`) — the same location convention as
/// `tinyflows::expr::resolve_traced`. Unlike that function this never
/// evaluates an expression against a scope; it only locates the leaves so
/// [`validate_binding_resolvability`] can statically pattern-match them.
fn collect_expressions(value: &Value) -> Vec<(String, String)> {
    fn walk(value: &Value, location: &str, out: &mut Vec<(String, String)>) {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    let child = if location.is_empty() {
                        k.clone()
                    } else {
                        format!("{location}.{k}")
                    };
                    walk(v, &child, out);
                }
            }
            Value::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    let child = if location.is_empty() {
                        i.to_string()
                    } else {
                        format!("{location}.{i}")
                    };
                    walk(v, &child, out);
                }
            }
            Value::String(s) if tinyflows::expr::is_expression(s) => {
                out.push((location.to_string(), s.clone()));
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, "", &mut out);
    out
}

/// Matches the dotted-path form of a node-output binding —
/// `=nodes.<ref_id>.item[.json].<field_path>` — returning `(ref_id, has_json,
/// field_path)`. `has_json` is `true` when the expression dereferenced the
/// `{json,text,raw}` envelope wrapper (`.item.json.<field_path>`) rather than
/// the item directly (`.item.<field_path>`).
///
/// `field_path` captures the FULL remaining dotted path, not just its first
/// segment — e.g. `"data.messages"` for `.item.json.data.messages`. This
/// matters for a Composio `tool_call` ref, whose real output additionally
/// wraps the field in `data` (see [`crate::openhuman::flows::tinyflows::caps::ToolContract::output_fields`]'s
/// doc): callers that need to check field membership against a schema with
/// no such wrapper (e.g. an `agent` node's `output_parser.schema`) should
/// compare against just `field_path`'s first segment.
///
/// Only the dotted-path form is recognized here — the equivalent jq form
/// (e.g. `=.nodes["ref"].items[0].field`) is an arbitrary jq program, not a
/// fixed grammar, so it is not statically pattern-matched; that form is still
/// covered dynamically by `dry_run_workflow`'s null-resolution check (#4586),
/// which actually evaluates the expression at run time.
fn parse_node_binding(expr: &str) -> Option<(String, bool, String)> {
    fn node_binding_regex() -> &'static regex::Regex {
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        RE.get_or_init(|| {
            regex::Regex::new(
                r"^=nodes\.([A-Za-z_][A-Za-z0-9_]*)\.item(?:\.(json))?\.([A-Za-z_][A-Za-z0-9_.]*)",
            )
            .expect("static regex is valid")
        })
    }
    let caps = node_binding_regex().captures(expr)?;
    let ref_id = caps.get(1)?.as_str().to_string();
    let has_json = caps.get(2).is_some();
    let field_path = caps.get(3)?.as_str().trim_end_matches('.').to_string();
    if field_path.is_empty() {
        return None;
    }
    Some((ref_id, has_json, field_path))
}

/// Human-readable label for a [`NodeKind`], for
/// [`validate_binding_resolvability`]'s envelope-violation message.
fn node_kind_label(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Agent => "an agent",
        NodeKind::ToolCall => "a tool_call",
        NodeKind::HttpRequest => "an http_request",
        _ => "a node",
    }
}

/// jaq keywords/operators that read as valid jq syntax rather than natural-
/// language prose; used by [`agent_prompt_looks_like_invalid_jq`]'s bareword
/// scan so a genuine jq program (`if`/`then`/`else`/`end`, `and`/`or`,
/// `reduce`/`foreach`, a `def`, …) is never mistaken for prose.
const JQ_KEYWORDS: &[&str] = &[
    "and", "or", "not", "if", "then", "elif", "else", "end", "as", "def", "reduce", "foreach",
    "try", "catch", "import", "include", "label",
];

/// Best-effort detector for an agent-node `config.prompt` `=`-expression that
/// is natural-language prose accidentally written in the `=`-binding
/// convention, rather than a real jq program — the exact failure this check
/// exists to catch: a builder writes something like `"=You are given an
/// email: .item. Classify it…"`, which is not a valid jq program (jq's
/// grammar has no rule for two bare identifiers in a row with nothing but
/// whitespace between them — an operator or pipe is required), so
/// `tinyflows::expr::evaluate` silently resolves it to `null` (its contract:
/// "compile/run errors never panic, they yield `Value::Null`") and the agent
/// turn then runs with an **empty prompt**.
///
/// `tinyflows` doesn't expose a compile-only jq check — `run_jq` is a private
/// helper in `tinyflows::expr` and the module's evaluation contract is
/// deliberately "never panics, malformed programs silently yield null" — so
/// this is a conservative pattern match rather than a real compiler
/// round-trip: quoted jq string literals are stripped first (so quoted prose
/// inside a legitimate concatenation like `="Hi " + .item.name` is never
/// scanned — this includes respecting a `\"` escape inside the string, so a
/// quoted literal like `="Say \"hi\" to " + .item.name` doesn't desync the
/// quote-toggle and leak its trailing prose into the bareword scan), then the
/// remainder is scanned for **two or more consecutive** whitespace-separated
/// barewords that are neither jq keywords nor path segments (`.foo`,
/// `.foo.bar`) — a real jq program never juxtaposes two bare identifiers like
/// that. Deliberately narrow (2+ in a row, not 1): a false negative here just
/// leaves prose alone (nothing new was broken); a false positive would reject
/// a legitimate author's graph.
fn agent_prompt_looks_like_invalid_jq(expr_body: &str) -> bool {
    let mut stripped = String::with_capacity(expr_body.len());
    let mut in_str = false;
    let mut chars = expr_body.chars();
    while let Some(c) = chars.next() {
        // An escaped char inside a jq string literal (`\"`, `\\`, `\n`, …) —
        // consume both the backslash and the escaped char without toggling
        // `in_str`, so an escaped quote never prematurely ends the string.
        if in_str && c == '\\' {
            chars.next();
            continue;
        }
        if c == '"' {
            in_str = !in_str;
            continue;
        }
        if !in_str {
            stripped.push(c);
        }
    }

    let mut consecutive_bare_words = 0u32;
    for tok in stripped.split_whitespace() {
        let core = tok.trim_matches(|c: char| !c.is_ascii_alphabetic());
        let is_bare_word = !core.is_empty()
            && core.chars().all(|c| c.is_ascii_alphabetic())
            && !tok.starts_with('.')
            && !tok.contains('.')
            && !JQ_KEYWORDS.contains(&core.to_ascii_lowercase().as_str());
        if is_bare_word {
            consecutive_bare_words += 1;
            if consecutive_bare_words >= 2 {
                return true;
            }
        } else {
            consecutive_bare_words = 0;
        }
    }
    false
}

/// Statically proves every `tool_call` node's `config.args` bindings are
/// resolvable, rejecting the graph (a non-empty `Vec` = reject; empty =
/// pass) when one is GUARANTEED to resolve `null` (or the wrong value) at
/// runtime. See the [module section](self) header for why this exists
/// alongside the advisory `graph_wiring_warnings`/`dry_run_workflow` checks.
///
/// Scoped to `tool_call` `args` for the field-addressability checks below —
/// an `agent` node's free-text prompt has no static output schema to enforce
/// a `nodes.<ref>.item.<field>` reference against, so a prose string that
/// merely *mentions* such a path is left alone (degrades output quality, but
/// doesn't break execution the way a `null` tool argument does). The ONE
/// `agent`-prompt case this pass DOES reject is narrower and execution-
/// breaking in its own right: `config.prompt` itself being a `=`-expression
/// that reads as prose rather than a jq program (see
/// [`agent_prompt_looks_like_invalid_jq`]) — that doesn't just degrade
/// output, it guarantees `null`, i.e. an EMPTY prompt, exactly the
/// `input_context` bug this whole gate was added to prevent (see the
/// `flows/agents/workflow_builder/prompt.md` convention: `input_context`
/// carries data, `prompt` stays a plain instruction).
///
/// For every `=nodes.<ref>.item[.json].<field>` binding found in a
/// `tool_call`'s `args` (via [`collect_expressions`] + [`parse_node_binding`]):
/// - a `<ref>` that doesn't resolve to a node in the graph is skipped — a
///   dangling reference is already a `tinyflows::validate::validate`
///   structural error, caught upstream of this pass.
/// - a `<ref>` that IS an [`ENVELOPING_KINDS`] node and the expression used
///   `.item.<field>` (no `.json`) is REJECTED: it dereferences the envelope
///   wrapper, not the field inside it.
/// - a `<ref>` that is an `agent` node is REJECTED unless it declares
///   `config.output_parser.schema` with an object `properties` map
///   containing `<field>` — the exact shape a real run's output-parser
///   sub-port enforces; without it the agent's structured output has no
///   addressable `<field>`.
/// - a `<ref>` that is `tool_call`/`http_request` only gets the envelope
///   check above — neither has a static output schema to check field
///   membership against ahead of a real run.
/// - any other referenced kind (`code`, `transform`, `split_out`, `merge`,
///   `output_parser`, `sub_workflow`, `trigger`, `condition`, `switch`) has no
///   schema or envelope convention to enforce and is accepted.
pub(crate) fn validate_binding_resolvability(graph: &WorkflowGraph) -> Vec<String> {
    let mut errors = Vec::new();

    // Agent-prompt gate: reject a `prompt` that reads as prose written in the
    // `=`-binding convention (see `agent_prompt_looks_like_invalid_jq`'s doc) —
    // it is GUARANTEED to resolve `null`, handing the agent an empty prompt.
    // A plain (non-`=`) prompt, or a real jq/dotted-path expression, is
    // unaffected.
    for node in &graph.nodes {
        if node.kind != NodeKind::Agent {
            continue;
        }
        // Both runtime paths (`build_completion_messages` and
        // `node_request_to_prompt` in `tinyflows/caps.rs`) fall through to a
        // non-empty `messages` array once `prompt` resolves to `null` — which
        // is exactly what this bad `=`-expression prompt does. So a node that
        // declares real `messages` never actually runs on the null prompt;
        // rejecting the graph for it would be a false positive against a
        // vestigial/unused legacy `prompt` field.
        let messages_supply_the_turn = node
            .config
            .get("messages")
            .and_then(Value::as_array)
            .is_some_and(|entries| !entries.is_empty());
        if messages_supply_the_turn {
            continue;
        }
        let Some(prompt) = node.config.get("prompt").and_then(Value::as_str) else {
            continue;
        };
        if !tinyflows::expr::is_expression(prompt) {
            continue;
        }
        let body = prompt[1..].trim();
        if agent_prompt_looks_like_invalid_jq(body) {
            errors.push(format!(
                "Node '{}': `prompt` (`{prompt}`) looks like natural-language text written as \
                 a `=`-expression, not a valid jq program — it will resolve to `null` at \
                 runtime, handing the agent an EMPTY prompt. Fix: feed upstream data through \
                 `config.input_context` (e.g. `\"input_context\": \"=item\"`) and make `prompt` \
                 a plain instruction with no leading `=`.",
                node.id
            ));
        }
    }

    for node in &graph.nodes {
        if node.kind != NodeKind::ToolCall {
            continue;
        }
        let Some(args) = node.config.get("args") else {
            continue;
        };
        for (location, expr) in collect_expressions(args) {
            let Some((ref_id, has_json, field_path)) = parse_node_binding(&expr) else {
                continue;
            };
            let Some(ref_node) = graph.node(&ref_id) else {
                continue;
            };

            if ENVELOPING_KINDS.contains(&ref_node.kind) && !has_json {
                errors.push(format!(
                    "Node '{}': arg `{location}` (`{expr}`) uses `.item.{field_path}` on {} node \
                     `{ref_id}`, but agent/tool_call/http_request nodes wrap output in {{json, \
                     text, raw}} — use `=nodes.{ref_id}.item.json.{field_path}` instead.",
                    node.id,
                    node_kind_label(&ref_node.kind),
                ));
                continue;
            }

            if ref_node.kind == NodeKind::Agent {
                // Agent output has no Composio `data` wrapper — the schema's
                // top-level properties are checked against just the FIRST
                // segment of the bound path (agents don't publish nested
                // output schemas here).
                let field = field_path.split('.').next().unwrap_or(&field_path);
                let has_field = ref_node
                    .config
                    .get("output_parser")
                    .and_then(|p| p.get("schema"))
                    .filter(|s| !s.is_null())
                    .and_then(|s| s.get("properties"))
                    .and_then(Value::as_object)
                    .is_some_and(|props| props.contains_key(field));
                if !has_field {
                    errors.push(format!(
                        "Node '{}': arg `{location}` (`{expr}`) binds to agent node `{ref_id}`, \
                         which has no `output_parser.schema` declaring `{field}` — its \
                         structured output has no addressable `{field}`, so this binding \
                         resolves null at runtime. Fix: add `{field}` to node `{ref_id}`'s \
                         output_parser.schema and bind via `=nodes.{ref_id}.item.json.{field}`.",
                        node.id
                    ));
                }
            }
        }
    }
    errors
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent-ref resolvability gate: an `agent` node's `agent_ref` must name a
// real agent, not the runtime's `RegistryFallback` "unknown agent_ref" case
// ─────────────────────────────────────────────────────────────────────────────
//
// `run_via_registry_fallback` (`tinyflows/caps.rs`) hard-errors mid-run with
// "unknown agent_ref '…'" the moment an `agent` node's `config.agent_ref`
// doesn't resolve to either a harness `AgentDefinition` or a custom agent
// registry entry. Today that is the FIRST time an author finds out — the
// graph proposes, saves, and even passes every other builder gate, then
// fails on the very node whose whole job was to run. This gate moves that
// same check to propose/edit/save time so a broken `agent_ref` is rejected
// before it's ever persisted, using the exact resolution the runtime uses
// (`route_for_agent_ref` + `agent_registry::get_agent`) rather than
// re-implementing it.
//
// A plain `agent` node with NO `agent_ref` is unaffected (and must stay
// that way) — it runs on the default LLM completion (`caps.llm`), never
// touches `OpenHumanAgentRunner`'s routing at all, so there is nothing to
// resolve.

/// Rejects an `agent` node whose `config.agent_ref` would hit the runtime's
/// `RegistryFallback` "unknown agent_ref" hard error mid-run
/// (`run_via_registry_fallback` in `tinyflows/caps.rs`) — a real ref is one
/// that resolves via [`crate::openhuman::flows::tinyflows::caps::route_for_agent_ref`]
/// to a harness [`AgentDefinition`](crate::openhuman::agent::harness::definition::AgentDefinition)
/// (`AgentRoute::Harness`), OR — when it routes to `AgentRoute::RegistryFallback`
/// — resolves to an *enabled*
/// [`AgentRegistryEntry`](crate::openhuman::agent::registry::AgentRegistryEntry)
/// via [`crate::openhuman::agent::registry::get_agent`]. Both are exactly the
/// checks `OpenHumanAgentRunner::run_agent` performs at run time, reused here
/// rather than duplicated so the two planes cannot drift.
///
/// A node with no `agent_ref` (or a blank one) is a plain agent node — it
/// runs on the default LLM completion, never reaches this routing at all —
/// and is skipped, not rejected. A registry lookup failure (e.g. config
/// unavailable) fails OPEN (skipped, logged) like the sibling
/// `validate_connection_refs` gate: this gate must never false-reject a
/// graph because of a transient local read.
///
/// Takes `config` for two reasons. First (CodeRabbit/Codex review on #5114):
/// one-shot contexts — the generic `openhuman <namespace> <function>` CLI
/// dispatcher (`default_state()`, no bootstrap), cron, tests — may reach this
/// gate before the full server bootstrap has called
/// [`AgentDefinitionRegistry::init_global`]. Without it, `route_for_agent_ref`
/// sees an empty global registry and routes EVERY ref — including a real
/// workspace-TOML harness definition — to `RegistryFallback`, which then only
/// checks the custom agent registry and would reject a valid harness agent
/// as unknown. So this gate defensively (re-)initialises the harness registry
/// itself, same idempotent (`OnceLock`) idiom as
/// `memory_goals::enrich::enrich`, before resolving any ref — the two planes
/// (author-time gate and `OpenHumanAgentRunner::run_agent` at actual run
/// time) then always see the same registry state. Second, it threads through
/// to `agent_registry::get_agent`'s underlying config load.
///
/// Also lazily caches the custom agent registry snapshot on the first
/// `RegistryFallback` node (CodeRabbit nitpick): a graph with several
/// non-harness `agent_ref`s previously triggered one `config_rpc::
/// load_config_with_timeout` per node; an all-`Harness`/no-custom-ref graph
/// still never reads it at all.
pub(crate) async fn validate_agent_refs(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::openhuman::agent::harness::AgentDefinitionRegistry;
    use crate::openhuman::agent::registry::AgentRegistryEntry;
    use crate::openhuman::flows::tinyflows::caps::{route_for_agent_ref, AgentRoute};

    let mut errors = Vec::new();
    let mut harness_registry_init_attempted = false;
    let mut custom_registry: Option<Result<Vec<AgentRegistryEntry>, String>> = None;

    for node in &graph.nodes {
        if node.kind != NodeKind::Agent {
            continue;
        }
        let Some(agent_ref) = node.config.get("agent_ref").and_then(Value::as_str) else {
            continue;
        };
        let agent_ref = agent_ref.trim();
        if agent_ref.is_empty() {
            continue;
        }

        if !harness_registry_init_attempted && AgentDefinitionRegistry::global().is_none() {
            harness_registry_init_attempted = true;
            if let Err(e) = AgentDefinitionRegistry::init_global(&config.workspace_dir) {
                tracing::debug!(
                    target: "flows",
                    error = %e,
                    "[flows] agent-ref check: harness registry init failed — falling through \
                     to route resolution with whatever state is available"
                );
            }
        }

        match route_for_agent_ref(agent_ref) {
            AgentRoute::Harness => {
                tracing::debug!(
                    target: "flows",
                    node = %node.id,
                    %agent_ref,
                    "[flows] agent-ref check: resolves to a harness agent definition"
                );
            }
            AgentRoute::RegistryFallback => {
                if custom_registry.is_none() {
                    custom_registry =
                        Some(crate::openhuman::agent::registry::list_agents(true).await);
                }
                match custom_registry.as_ref().expect("just populated") {
                    Ok(entries) => match entries.iter().find(|entry| entry.id == agent_ref) {
                        Some(entry) if entry.enabled => {
                            tracing::debug!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: resolves to an enabled custom agent \
                                 registry entry"
                            );
                        }
                        Some(_disabled) => {
                            tracing::warn!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: agent_ref is registered but disabled — \
                                 rejecting"
                            );
                            errors.push(format!(
                                "Node '{}': `agent_ref` `{agent_ref}` is registered but currently \
                                 disabled — enable it (or pick another agent_ref via \
                                 list_agent_profiles) before this node can run.",
                                node.id
                            ));
                        }
                        None => {
                            tracing::warn!(
                                target: "flows",
                                node = %node.id,
                                %agent_ref,
                                "[flows] agent-ref check: unknown agent_ref — neither a harness \
                                 definition nor a custom agent registry entry — rejecting"
                            );
                            errors.push(format!(
                                "Node '{}': `agent_ref` `{agent_ref}` is not a real agent — it \
                                 names neither a built-in agent definition nor a custom agent \
                                 registry entry, and would fail at run time with an \"unknown \
                                 agent_ref\" error. Call list_agent_profiles to see the real, \
                                 selectable agent_ref values.",
                                node.id
                            ));
                        }
                    },
                    Err(e) => {
                        tracing::debug!(
                            target: "flows",
                            node = %node.id,
                            %agent_ref,
                            error = %e,
                            "[flows] agent-ref check: custom agent registry lookup unavailable — \
                             skipping (fail-open)"
                        );
                    }
                }
            }
        }
    }
    errors
}

// ─────────────────────────────────────────────────────────────────────────────
// Inference-readiness check: provider-connectivity (issue B45)
// ─────────────────────────────────────────────────────────────────────────────
//
// An `agent` node's completion (`OpenHumanLlm::complete` in
// `tinyflows/caps.rs`) resolves a chat model exactly like every other
// inference caller in this host — but no check previously inspected that
// resolution at all. `compute_required_connections` only walks `tool_call`
// Composio nodes; an `agent` node's own hard dependency, a working LLM
// provider, went completely unchecked. The confirmed failure: a signed-in
// user whose managed-backend account has no provider API key configured gets
// an HTTP 400 `{"success":false,"error":"API key not configured for
// provider","errorCode":"BAD_REQUEST"}` — but only mid-run, wrapped several
// layers deep as `capability error: graph error: capability error: model
// error: ...`.
//
// **Design correction (judge finding on live run 104aab90 — see git log for
// the full writeup):** this was originally wired in as a HARD author gate
// (`run_builder_gates`), rejecting `propose_workflow`/`edit_workflow`
// outright. In practice that meant a graph whose only problem was "the user
// hasn't configured a provider yet" could never be proposed at all — the
// copilot detected `provider_not_configured`, tried to propose anyway, was
// blocked, and trailed off with no workflow shown to the user. The correct
// placement is:
//
// - **Author time (`build_builder_proposal`)** — ADVISORY ONLY. Authoring
//   always succeeds; `evaluate_inference_readiness`'s result rides along on
//   the proposal payload as `inference_status`/`inference_message` so the UI
//   can render a "connect your provider" nudge next to the built workflow.
// - **Run time (`run_flow_body`)** — HARD gate. A real run (never
//   `dry_run_workflow`, which is a sandbox) checks readiness before invoking
//   the tinyflows engine and fails the run row cleanly with an actionable
//   message if the graph's agent node(s) can't currently reach a provider —
//   see `validate_inference_readiness`'s call site in `run_flow_body`.
//
// Two layers, cheapest and most decisive first:
//
// - **Layer 1 (sync)** — the desktop session itself: signed out
//   (`scheduler_gate::is_signed_out`), or no valid `app-session` JWT
//   (`inference::provider::factory::verify_session_active`, the exact check
//   every custom-provider construction already gates on).
// - **Layer 2 (async, cached)** — one cheap real probe per DISTINCT resolved
//   role (`inference::provider::probe_inference_readiness`) to catch the
//   "signed in but no provider API key configured for this account" class of
//   failure that Layer 1 cannot see. A graph can mix agent nodes pinned to
//   different models (e.g. one `hint:reasoning`, one plain `chat`) that route
//   to different provider configs — each distinct role is probed once, not
//   once per node, and every probe's result caches BOTH a successful and a
//   definitively-negative result for a short TTL — a propose → edit → save →
//   run authoring/run burst hits the network at most once per role per TTL
//   window, whichever way the probe comes back. This is safe to cache
//   negative because `probe_inference_readiness` (and, beneath it,
//   `OpenHumanBackendModel::probe_readiness`) already fails OPEN (`Ok(())`)
//   on anything transient — a timeout, a transport error, a 5xx — so an
//   `Err` reaching this cache is always the definitive, config-level "not
//   ready" signal, never a flake that a naive cache would freeze in place.
//
// [`evaluate_inference_readiness`] is the single evaluation both
// [`validate_inference_readiness`] (the hard gate) and
// [`build_builder_proposal`]'s `inference_status` payload field consume, so
// the gate and the UI-facing status can never disagree.

/// Cache TTL for the Layer-2 managed-backend/role probe.
const INFERENCE_PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Cache key: (workload role, session identity). `config.config_path` stands
/// in for "session identity" — within one desktop process there is exactly
/// one active config/session, so this is stable in production, while
/// distinct `Config`s (as every test builds its own `tempfile` workspace)
/// naturally get distinct cache entries instead of bleeding a cached result
/// from one test/session into an unrelated one. Keying on `role` alone would
/// NOT be enough: two different sessions (or two tests) can both resolve the
/// literal role `"summarization"` to entirely different, unrelated outcomes.
type InferenceProbeCacheKey = (String, std::path::PathBuf);
/// A cached probe outcome: when it was taken, and the definitive result.
type InferenceProbeCacheEntry = (std::time::Instant, Result<(), String>);
/// The probe cache map, factored out to keep the `static` type readable
/// (clippy::type-complexity).
type InferenceProbeCacheMap =
    std::collections::HashMap<InferenceProbeCacheKey, InferenceProbeCacheEntry>;

/// Process-global cache of Layer-2 probe outcomes, keyed by
/// [`InferenceProbeCacheKey`]. Both `Ok` and `Err` entries are served from
/// cache within [`INFERENCE_PROBE_CACHE_TTL`] (design correction, B45 —
/// previously only `Ok` was cached, so a signed-in-but-unconfigured account
/// re-hit the network on every one of `edit_workflow` / `validate_workflow` /
/// `propose_workflow` / a run's own preflight in a single authoring turn — up
/// to 4 network round trips observed in one live judge-flagged turn). A
/// cached `Err` is still only ever the definitive class (see the module doc
/// above on fail-open) — a fixed provider becomes visible again at most
/// `INFERENCE_PROBE_CACHE_TTL` later, or immediately on sign-out/back-in via
/// [`invalidate_inference_probe_cache_if_signed_out`].
static INFERENCE_PROBE_CACHE: LazyLock<std::sync::Mutex<InferenceProbeCacheMap>> =
    LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Invalidate every cached Layer-2 probe result. Checked defensively on every
/// call so a signed-out session (whether the initial one or a later
/// account-switch) can never serve a stale cached "ready" — the moment
/// `is_signed_out` flips true the next successful probe starts a fresh TTL
/// window. Clears the whole cache rather than just the current key: a
/// sign-out is a session-wide event, not scoped to one role.
fn invalidate_inference_probe_cache_if_signed_out() {
    if crate::openhuman::cron::scheduler_gate::is_signed_out() {
        INFERENCE_PROBE_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

async fn cached_probe_inference_readiness(role: &str, config: &Config) -> Result<(), String> {
    invalidate_inference_probe_cache_if_signed_out();

    let key: InferenceProbeCacheKey = (role.to_string(), config.config_path.clone());

    if let Some((checked_at, result)) = INFERENCE_PROBE_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .cloned()
    {
        if checked_at.elapsed() < INFERENCE_PROBE_CACHE_TTL {
            tracing::debug!(
                target: "flows",
                role,
                cached_ready = result.is_ok(),
                "[flows] inference-readiness: reusing cached probe result"
            );
            return result;
        }
    }

    let result =
        crate::openhuman::inference::provider::probe_inference_readiness(role, config).await;
    INFERENCE_PROBE_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, (std::time::Instant::now(), result.clone()));
    result
}

/// The workload role an `agent` node's completion effectively runs on —
/// mirrors the exact mapping `OpenHumanLlm::complete` (`tinyflows/caps.rs`)
/// applies, so this probe checks the same route the node will actually
/// dispatch to at run time. Precedence (findings A+B on this gate):
///
/// 1. Node `config.model` — a managed tier or `hint:*` alias, translated via
///    [`role_for_model_tier`](crate::openhuman::inference::provider::role_for_model_tier).
/// 2. A static (non-`=`) `agent_ref` whose custom
///    [`AgentRegistryEntry`](crate::openhuman::agent::registry::AgentRegistryEntry)
///    itself pins a `model` (e.g. `hint:reasoning`) — resolved the same way
///    [`OpenHumanAgentRunner::run_via_harness`](crate::openhuman::flows::tinyflows::caps::OpenHumanAgentRunner)
///    does via `resolve_node_model(&request, entry_model)`, using the same
///    sync, config-only accessor
///    ([`find_custom_in_config`](crate::openhuman::agent::registry::find_custom_in_config))
///    it calls.
/// 3. Otherwise, caps.rs's own default role (`"summarization"`, its fallback
///    absent a `role` field on the completion request).
///
/// A static `agent_ref` that instead resolves to a shipped/TOML harness
/// `AgentDefinition` (`AgentRoute::Harness`) can *also* pin a model via
/// `ModelSpec::Exact`/`ModelSpec::Hint` — but `ModelSpec::Inherit` (the
/// default) resolves against the *parent* agent's live model at spawn time,
/// which this static, pre-run gate has no parent turn to read. Resolving only
/// the Exact/Hint cases here — while silently mis-defaulting every
/// `Inherit`-using definition — would be a half-correct, fragile lookup, so
/// this case falls back to the default role rather than guess.
/// TODO(B45): resolve agent_ref-pinned model for harness `AgentDefinition`s
/// once a parent-model-free resolution path exists.
fn agent_node_role(config: &Config, node: &tinyflows::model::Node) -> &'static str {
    let pinned_model = node
        .config
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(model) = pinned_model {
        return crate::openhuman::inference::provider::role_for_model_tier(model);
    }

    let static_agent_ref = node
        .config
        .get("agent_ref")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('='));
    if let Some(agent_ref) = static_agent_ref {
        if let Some(entry_model) =
            crate::openhuman::agent::registry::find_custom_in_config(config, agent_ref)
                .and_then(|entry| entry.model)
        {
            let entry_model = entry_model.trim();
            if !entry_model.is_empty() {
                return crate::openhuman::inference::provider::role_for_model_tier(entry_model);
            }
        }
    }

    "summarization"
}

/// Classifies an inference-readiness failure message into the fixed wire
/// vocabulary `build_builder_proposal`'s `inference_status` payload and this
/// gate's prose both use (`"signed_out" | "provider_not_configured" |
/// "error"`).
///
/// Defensive ordering: a message that still smells like a dead session (an
/// unlikely race between this gate's own signed-out check and the async
/// probe) is classified `signed_out` before the more specific
/// `provider_not_configured` pattern; anything else falls back to the generic
/// `error` bucket (a BYOK-incomplete config, an unknown provider slug, a
/// local-only privacy-mode block, …) rather than mislabeling it as a
/// provider-key problem.
fn classify_inference_error_message(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("session_expired") || lower.contains("sign in") {
        "signed_out"
    } else if lower.contains("api key not configured") {
        "provider_not_configured"
    } else {
        "error"
    }
}

/// Outcome of [`evaluate_inference_readiness`] for a graph that has at least
/// one applicable `agent` node.
struct InferenceReadinessEvaluation {
    /// One of `"ready"`, `"signed_out"`, `"provider_not_configured"`, `"error"`
    /// — the fixed vocabulary shared with the proposal payload.
    status: &'static str,
    /// User-actionable prose; `None` only when `status == "ready"`.
    message: Option<String>,
    /// The offending node id, when applicable (absent for `"ready"`).
    node_id: Option<String>,
}

/// Evaluate the B45 provider-connectivity gate for `graph`.
///
/// Returns `None` when the graph has no `agent` node at all — a tool_call-only
/// graph never pays this check's cost. A dynamic `=`-derived `agent_ref` node
/// is still in scope (finding C): its concrete route is not knowable
/// statically, so its exact per-model role can't be resolved, but the node
/// still means "this graph runs inference" — it stays in scope for Layer 1
/// (signed-out/session) and gets a default-role Layer 2 probe. Only the
/// per-model role resolution is skipped for such a node, never the whole
/// check.
///
/// Every DISTINCT role across the graph's applicable `agent` nodes is probed
/// (findings A+B): Layer 1 (signed-out/session) runs once for the whole
/// graph — every agent node shares one backend session — then Layer 2 runs
/// once per distinct role (via [`cached_probe_inference_readiness`], so a
/// role already probed elsewhere in this process within the TTL is served
/// from cache). `status`/`message` report `provider_not_configured`/`error`
/// if ANY role's probe fails, naming every offending node and role.
async fn evaluate_inference_readiness(
    config: &Config,
    graph: &WorkflowGraph,
) -> Option<InferenceReadinessEvaluation> {
    let agent_nodes: Vec<&tinyflows::model::Node> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Agent)
        .collect();

    let first_node = *agent_nodes.first()?;

    // Layer 1: signed-out is the cheapest, most decisive check. Session-wide
    // — checked once for the whole graph, not per node/role.
    if crate::openhuman::cron::scheduler_gate::is_signed_out() {
        tracing::debug!(
            target: "flows",
            node = %first_node.id,
            "[flows] inference-readiness: signed out — rejecting"
        );
        return Some(InferenceReadinessEvaluation {
            status: "signed_out",
            message: Some(
                "Inference unavailable: you are signed out. Sign in to OpenHuman to run agent \
                 nodes."
                    .to_string(),
            ),
            node_id: Some(first_node.id.clone()),
        });
    }
    // Skipped under `#[cfg(test)]`, matching every other call site of this
    // exact check (`factory.rs`'s `unresolved_chat_model_error` and friends):
    // unit-test configs use a fresh `tempfile::tempdir()` workspace with no
    // stored `app-session` JWT by design, so this would otherwise reject
    // every agent-node graph built by the hundreds of existing flows tests
    // that have nothing to do with session state. Layer 2 below still fails
    // OPEN on a construction failure caused by a genuinely missing session
    // (see `OpenHumanBackendModel::probe_readiness`'s own doc), so production
    // behavior for a real signed-out desktop user is unchanged — only the
    // (redundant, in that case) early rejection here is test-only skipped.
    #[cfg(not(test))]
    if let Err(e) = crate::openhuman::inference::provider::factory::verify_session_active(config) {
        tracing::debug!(
            target: "flows",
            node = %first_node.id,
            error = %e,
            "[flows] inference-readiness: no active backend session — rejecting"
        );
        return Some(InferenceReadinessEvaluation {
            status: "signed_out",
            message: Some(format!(
                "Inference unavailable: {e} Sign in to OpenHuman to run agent nodes."
            )),
            node_id: Some(first_node.id.clone()),
        });
    }

    // Layer 2: each node's effective role, grouped so every DISTINCT role is
    // probed exactly once (a graph with several agent nodes pinning the same
    // role must not pay the network/cache-lookup cost twice). `BTreeMap` for
    // deterministic iteration/message ordering (test-friendly, and stable
    // prose across runs).
    let mut nodes_by_role: std::collections::BTreeMap<&'static str, Vec<String>> =
        std::collections::BTreeMap::new();
    for node in &agent_nodes {
        let role = agent_node_role(config, node);
        nodes_by_role.entry(role).or_default().push(node.id.clone());
    }

    let mut failures: Vec<(&'static str, String, Vec<String>)> = Vec::new();
    for (role, node_ids) in &nodes_by_role {
        tracing::debug!(
            target: "flows",
            nodes = ?node_ids,
            role,
            "[flows] inference-readiness: probing managed-backend/role readiness"
        );
        if let Err(msg) = cached_probe_inference_readiness(role, config).await {
            tracing::warn!(
                target: "flows",
                nodes = ?node_ids,
                role,
                "[flows] inference-readiness: probe rejected — {msg}"
            );
            failures.push((role, msg, node_ids.clone()));
        }
    }

    if failures.is_empty() {
        return Some(InferenceReadinessEvaluation {
            status: "ready",
            message: None,
            node_id: None,
        });
    }

    // Defensive ordering matches `classify_inference_error_message`'s own doc:
    // `signed_out` (unlikely to reach Layer 2, given the Layer 1 check above,
    // but a race is not impossible) outranks `provider_not_configured`, which
    // outranks the generic `error` bucket.
    let statuses: Vec<&'static str> = failures
        .iter()
        .map(|(_, msg, _)| classify_inference_error_message(msg))
        .collect();
    let status = if statuses.contains(&"signed_out") {
        "signed_out"
    } else if statuses.contains(&"provider_not_configured") {
        "provider_not_configured"
    } else {
        "error"
    };

    // Single failing role naming a single node: keep the original flat
    // message shape (no node-list preamble) so the existing single-node
    // contract/tests read exactly as before. Anything broader (several
    // failing roles, or one role shared by several nodes) names every
    // offending node/role explicitly, since a flat message can no longer
    // unambiguously point at "the" offending node.
    if let [(_role, msg, node_ids)] = failures.as_slice() {
        if let [node_id] = node_ids.as_slice() {
            let message = if status == "provider_not_configured" {
                format!(
                    "This flow's agent step needs a working AI provider, but the provider \
                     returned: '{msg}'. Configure your provider API key in OpenHuman Settings > \
                     Providers, then try again."
                )
            } else {
                format!("This flow's agent step needs a working AI provider: {msg}")
            };
            return Some(InferenceReadinessEvaluation {
                status,
                message: Some(message),
                node_id: Some(node_id.clone()),
            });
        }
    }

    let message = failures
        .iter()
        .map(|(role, msg, node_ids)| {
            let nodes = node_ids
                .iter()
                .map(|id| format!("'{id}'"))
                .collect::<Vec<_>>()
                .join(", ");
            let role_status = classify_inference_error_message(msg);
            if role_status == "provider_not_configured" {
                format!(
                    "Node(s) {nodes} (role `{role}`): the provider returned: '{msg}'. Configure \
                     your provider API key in OpenHuman Settings > Providers, then try again."
                )
            } else {
                format!("Node(s) {nodes} (role `{role}`): {msg}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    Some(InferenceReadinessEvaluation {
        status,
        message: Some(format!(
            "This flow has {} agent step(s) that need a working AI provider:\n\n{message}",
            failures.len()
        )),
        node_id: None,
    })
}

/// The B45 provider-connectivity check as a gate-shaped `Vec<String>`: empty
/// when the graph's `agent` node(s) (if any) can currently reach a working
/// LLM provider, otherwise the offending node's error, naming it.
///
/// **No longer wired into `run_builder_gates`** (design correction — see the
/// module doc above): authoring is never blocked by this. Its one production
/// caller is `run_flow_body`'s run-time preflight, which fails a real run
/// cleanly before the tinyflows engine executes rather than hard-blocking the
/// author from proposing/saving the graph in the first place. See the module
/// doc above for the two-layer evaluation design.
pub(crate) async fn validate_inference_readiness(
    config: &Config,
    graph: &WorkflowGraph,
) -> Vec<String> {
    let Some(evaluation) = evaluate_inference_readiness(config, graph).await else {
        return Vec::new();
    };
    if evaluation.status == "ready" {
        return Vec::new();
    }
    let message = evaluation
        .message
        .unwrap_or_else(|| "This flow's agent step needs a working AI provider.".to_string());
    match evaluation.node_id {
        Some(node_id) => vec![format!("Node '{node_id}': {message}")],
        None => vec![message],
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool-contract enforcement gate (systemic tool-contract fix, Part 2)
// ─────────────────────────────────────────────────────────────────────────────
//
// `validate_binding_resolvability` (above) statically proves a binding's
// SHAPE is sound (envelope dereference, agent output schema). It has no
// opinion on whether a `tool_call` node's `slug` is a REAL Composio action,
// or whether the args it wires cover that action's REAL required set — a
// builder could pass a hallucinated slug (`SLACK_POST_MESSAGE_TO_CHANNEL`,
// which 404s at runtime) or omit a genuinely required arg, and
// `validate_binding_resolvability` would have nothing to say about either.
// [`validate_tool_contracts`] is that missing HARD gate, grounded in
// [`crate::openhuman::flows::tinyflows::caps::fetch_live_toolkit_catalog`] — the
// FULL LIVE Composio catalog, not the static curated subset.

/// Statically proves every `tool_call` node's `config.slug` is a REAL action
/// in the LIVE Composio catalog for its toolkit, and that every one of that
/// action's REAL required args is present (non-null) in `config.args` —
/// rejecting the graph (a non-empty `Vec` = reject; empty = pass) when
/// either check fails. Wired into `propose_workflow` / `revise_workflow` /
/// `save_workflow` alongside [`validate_binding_resolvability`].
///
/// Skipped for a `slug` that is `=`-derived (resolved from upstream/trigger
/// data at runtime — nothing to check statically) or a native `oh:` tool (no
/// Composio contract at all).
///
/// **Best-effort on catalog availability, not on catalog CONTENT**: when the
/// live-catalog fetch itself fails (no backend session, network error) the
/// node is SKIPPED with a debug log — never rejected — because a
/// hallucinated slug can only be confirmed hallucinated once the real
/// catalog was actually reachable; `graph_wiring_warnings`'s
/// `composio_required_args` checks share this exact contract. Once the
/// catalog IS reachable, though, both checks below are HARD: an unreal slug
/// or a missing required arg rejects the graph outright, unlike the
/// advisory output-field/`split_out.path` WARNs in `graph_wiring_warnings`
/// (Part 2c/2d) — those degrade gracefully because a binding to an unknown
/// field can't be proven wrong, whereas a nonexistent slug or a missing
/// required arg are both provably broken.
/// Whether OpenHuman ships a STATIC curated catalog for `toolkit`. This is the
/// exact condition both [`validate_tool_contracts`]'s curation gate and
/// `tinyflows::caps::flow_tool_allowed`'s runtime Path A use to decide a toolkit
/// is a hard curated-only allowlist: for such a toolkit a real-but-uncurated
/// action is rejected on EVERY real run, so the author-time gate and the early
/// builder-tool warnings (`get_tool_contract` / `search_tool_catalog`) must all
/// agree on it — one home for the check so they cannot drift.
pub(crate) fn toolkit_has_curated_catalog(toolkit: &str) -> bool {
    use crate::openhuman::memory::sync::composio::providers::{catalog_for_toolkit, get_provider};
    get_provider(toolkit)
        .and_then(|p| p.curated_tools())
        .or_else(|| catalog_for_toolkit(toolkit))
        .is_some()
}

pub(crate) async fn validate_tool_contracts(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::openhuman::flows::tinyflows::caps::{
        fetch_live_toolkit_catalog, missing_required_args, unsupported_arg_names,
    };
    use crate::openhuman::memory::sync::composio::providers::toolkit_from_slug;

    let mut errors = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::ToolCall {
            continue;
        }
        let Some(slug) = node.config.get("slug").and_then(Value::as_str) else {
            continue;
        };
        // `=`-derived slugs resolve from upstream/trigger data at runtime —
        // nothing to check statically. Native `oh:` tools have no Composio
        // contract.
        if slug.starts_with('=') || slug.starts_with("oh:") {
            continue;
        }
        let Some(toolkit) = toolkit_from_slug(slug) else {
            continue;
        };
        let Some(catalog) = fetch_live_toolkit_catalog(config, &toolkit).await else {
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                %toolkit,
                "[flows] tool-contract check: live catalog fetch failed — skipping (best-effort, never false-rejects)"
            );
            continue;
        };

        let Some(contract) = catalog.iter().find(|c| c.slug.eq_ignore_ascii_case(slug)) else {
            tracing::warn!(
                target: "flows",
                node = %node.id,
                %slug,
                %toolkit,
                "[flows] tool-contract check: slug is not a real action in the live catalog — rejecting"
            );
            errors.push(format!(
                "Node '{}': `{slug}` is not a real action in the `{toolkit}` toolkit's live \
                 Composio catalog — use search_tool_catalog {{ query: ..., toolkit: \"{toolkit}\" \
                 }} to find a real action slug.",
                node.id
            ));
            continue;
        };

        // Mirror `flow_tool_allowed`'s Path A: a toolkit OpenHuman ships a
        // static curated catalog for is a hard curated-only allowlist at
        // RUNTIME — `find_curated` rejects any slug that isn't one of the
        // curated actions, regardless of whether it's a real live action.
        // `search_tool_catalog`/`get_tool_contract` deliberately surface
        // real-but-uncurated actions too (ranking signal only, never
        // hidden — see `ToolContract::is_curated`'s doc), so without this
        // check a graph could pass authoring/save with a real-but-uncurated
        // action on a curated toolkit and then fail every run with "tool
        // not permitted". Hold authoring to the same bar the runtime gate
        // enforces instead of loosening the runtime gate.
        let has_static_catalog = toolkit_has_curated_catalog(&toolkit);
        if has_static_catalog && !contract.is_curated {
            tracing::warn!(
                target: "flows",
                node = %node.id,
                %slug,
                %toolkit,
                "[flows] tool-contract check: slug is real but not curated for a statically-catalogued toolkit — rejecting to match the runtime allowlist"
            );
            errors.push(format!(
                "Node '{}': `{slug}` is a real `{toolkit}` action but not one of OpenHuman's \
                 curated actions for `{toolkit}` — the runtime tool gate only allows curated \
                 actions for toolkits with a curated catalog, so this would be rejected on \
                 every run. Use search_tool_catalog {{ query: ..., toolkit: \"{toolkit}\" }} and \
                 pick a result with `featured: true`.",
                node.id
            ));
            continue;
        }

        let args = node.config.get("args").cloned().unwrap_or(Value::Null);
        let missing = missing_required_args(&contract.required_args, &args);
        if !missing.is_empty() {
            tracing::warn!(
                target: "flows",
                node = %node.id,
                %slug,
                ?missing,
                "[flows] tool-contract check: required arg(s) missing or null — rejecting"
            );
            let list = missing
                .iter()
                .map(|m| format!("`{m}`"))
                .collect::<Vec<_>>()
                .join(", ");
            errors.push(format!(
                "Node '{}': tool_call `{slug}` is missing required arg(s) {list} — wire each \
                 from an upstream node's output, e.g. \"{}\": \
                 \"=nodes.<node_id>.item.json.<field>\" (call get_tool_contract {{ slug: \
                 \"{slug}\" }} for the exact required_args list).",
                node.id, missing[0]
            ));
        }

        // [B13] Arg-NAME validity: `missing_required_args` only proves a
        // required arg is PRESENT — it says nothing about whether every arg
        // the builder wired is actually a property this action's schema
        // recognizes. A misnamed/unsupported field (the live bug: wiring
        // `SLACK_SEND_MESSAGE` with `text` when the action wants
        // `markdown_text`) sails through the check above unrejected — a
        // value IS present, just under the wrong key — and only surfaces as
        // a runtime 400 from the real provider. `unsupported_arg_names`
        // returns `None` when the schema can't be used to validate names
        // (unknown schema, or `additionalProperties: true`) — that case is
        // deliberately never rejected here (best-effort, same posture as the
        // rest of this gate).
        if let Some(unsupported) = unsupported_arg_names(contract.input_schema.as_ref(), &args) {
            if !unsupported.is_empty() {
                let valid_names: Vec<String> = contract
                    .input_schema
                    .as_ref()
                    .and_then(|s| s.get("properties"))
                    .and_then(Value::as_object)
                    .map(|props| {
                        let mut names: Vec<String> = props.keys().cloned().collect();
                        names.sort();
                        names
                    })
                    .unwrap_or_default();
                tracing::warn!(
                    target: "flows",
                    node = %node.id,
                    %slug,
                    ?unsupported,
                    ?valid_names,
                    "[flows] tool-contract check: arg name(s) not declared by the action's \
                     input schema — rejecting"
                );
                let bad_list = unsupported
                    .iter()
                    .map(|m| format!("`{m}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let valid_suffix = if valid_names.is_empty() {
                    String::new()
                } else {
                    format!(
                        " — valid arg names for `{slug}` are: {}",
                        valid_names.join(", ")
                    )
                };
                errors.push(format!(
                    "Node '{}': tool_call `{slug}` has unsupported arg name(s) {bad_list} — not \
                     a property of this action's input schema{valid_suffix}. Call \
                     get_tool_contract {{ slug: \"{slug}\" }} and use the exact property names \
                     from `input_schema` (never guess an arg name).",
                    node.id
                ));
            }
        }
    }
    errors
}

// ─────────────────────────────────────────────────────────────────────────────
// Connection-ref gate (WS3): a Composio tool_call's `connection_ref` must name
// a real connected account of the RIGHT toolkit
// ─────────────────────────────────────────────────────────────────────────────
//
// Transcript audit: the user's connections were `twitter →
// composio:twitter:ca_JX6QU88UfSk4`, `gmail → composio:gmail:ca_vX_WA8FsqNmE`,
// `tiktok → composio:tiktok:ca_LPCp3WQpaDma`. The agent wired
// `composio:twitter:ca_LPCp3WQpaDma` and `composio:gmail:ca_LPCp3WQpaDma` (the
// TIKTOK id) onto the Twitter and Gmail tool_call nodes. dry_run / validate /
// propose all returned ok:true — nothing cross-checked the id against the user's
// real connections, nor the ref's toolkit segment against the slug — and it
// would fail on the first real run. This gate closes that gap: it parses the
// ref, enforces the toolkit segment matches the slug (needs no I/O), and — when
// the live connection list is reachable — that the id names a real connected
// account of that toolkit, naming the correct ref when it can.

/// Parses a `composio:<toolkit>:<id>` connection_ref into its `(toolkit, id)`
/// segments. Mirrors [`crate::openhuman::flows::tinyflows::caps::composio_connection_id`]'s
/// rsplit for the id (everything after the LAST `:`), taking everything between
/// the `composio:` prefix and that last `:` as the toolkit. Returns `None` for
/// anything that isn't this shape (missing `composio:` prefix, no `:` after it,
/// or an empty toolkit/id segment).
fn parse_composio_connection_ref(conn_ref: &str) -> Option<(&str, &str)> {
    let rest = conn_ref.strip_prefix("composio:")?;
    let (toolkit, id) = rest.rsplit_once(':')?;
    if toolkit.trim().is_empty() || id.trim().is_empty() {
        return None;
    }
    Some((toolkit.trim(), id.trim()))
}

/// First connected account `connection_ref` for `toolkit` (case-insensitive)
/// from `conns`, used to name the correct ref in a rejection's "did you mean"
/// hint. `None` when the toolkit has no connection at all.
fn first_connection_ref_for_toolkit(conns: &[FlowConnection], toolkit: &str) -> Option<String> {
    conns
        .iter()
        .find(|c| {
            c.toolkit
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(toolkit))
        })
        .map(|c| c.connection_ref.clone())
}

/// Hard gate: for every Composio `tool_call` node carrying a `connection_ref`,
/// prove the ref names a real connected account of the SAME toolkit as the
/// slug. Fetches the live connection list once (same source
/// [`flows_list_connections`] reads) and delegates the pure matching to
/// [`validate_connection_refs_against`].
///
/// Fail-open on I/O: if the Composio connection list is unreachable (backend
/// outage), the id-existence check is SKIPPED (a `tracing::debug!` records it)
/// so a real connection is never false-rejected during an outage — but the
/// toolkit-mismatch check, which needs no I/O, still runs.
pub(crate) async fn validate_connection_refs(
    config: &Config,
    graph: &WorkflowGraph,
) -> Vec<String> {
    let connections: Option<Vec<FlowConnection>> =
        match crate::openhuman::integrations::composio::ops::composio_list_connections(config).await
        {
            Ok(outcome) => Some(build_flow_connections(
                outcome.value.connections,
                Vec::new(),
                // Identity isn't needed for this existence/toolkit-mismatch
                // check — only `connection_ref` and `toolkit` are read.
                &[],
            )),
            Err(e) => {
                tracing::debug!(
                    target: "flows",
                    error = %e,
                    "[flows] connection-ref check: composio connection list unavailable — \
                     skipping id-existence check (fail-open); toolkit-mismatch check still runs"
                );
                None
            }
        };
    validate_connection_refs_against(graph, connections.as_deref())
}

/// Pure connection-ref validator (no I/O) so the gate's decision logic is
/// unit-testable without a live Composio backend. `connections` is `Some(list)`
/// when the live connection list was fetched (possibly empty — a genuine "no
/// connections" state), or `None` when it was unavailable (fail-open: the
/// id-existence check is skipped, only the toolkit-mismatch check runs).
fn validate_connection_refs_against(
    graph: &WorkflowGraph,
    connections: Option<&[FlowConnection]>,
) -> Vec<String> {
    use crate::openhuman::memory::sync::composio::providers::toolkit_from_slug;

    let mut errors = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::ToolCall {
            continue;
        }
        let Some(slug) = node.config.get("slug").and_then(Value::as_str) else {
            continue;
        };
        // `=`-derived slugs resolve at runtime; native `oh:` tools have no
        // Composio connection to name.
        if slug.starts_with('=') || slug.starts_with("oh:") {
            continue;
        }
        // A MISSING `connection_ref` stays allowed (unchanged): a Composio
        // tool_call with no ref runs against the ambient signed-in account and
        // the flow prompts for a connection at first run.
        let Some(conn_ref) = node.config.get("connection_ref").and_then(Value::as_str) else {
            continue;
        };
        if conn_ref.trim().is_empty() {
            continue;
        }
        let Some(slug_toolkit) = toolkit_from_slug(slug) else {
            continue;
        };

        let Some((ref_toolkit, ref_id)) = parse_composio_connection_ref(conn_ref) else {
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %conn_ref,
                matched = false,
                "[flows] connection-ref check: malformed ref — rejecting"
            );
            errors.push(format!(
                "Node '{}': `connection_ref` `{conn_ref}` is malformed — a Composio account ref \
                 must look like `composio:<toolkit>:<connection_id>` (e.g. \
                 `composio:{slug_toolkit}:<id>`). Call list_flow_connections and copy a \
                 `connection_ref` value verbatim.",
                node.id
            ));
            continue;
        };

        // Toolkit segment vs the slug's toolkit — needs no I/O.
        if !ref_toolkit.eq_ignore_ascii_case(&slug_toolkit) {
            let suggestion = connections
                .and_then(|conns| first_connection_ref_for_toolkit(conns, &slug_toolkit));
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %ref_toolkit,
                %ref_id,
                matched = false,
                "[flows] connection-ref check: toolkit segment does not match the slug's toolkit — rejecting"
            );
            let hint = match suggestion {
                Some(r) => format!(" — did you mean `{r}`?"),
                None => format!(
                    " — no `{slug_toolkit}` account is connected; connect one with \
                     composio_connect (or ask the user to), then use its `connection_ref`"
                ),
            };
            errors.push(format!(
                "Node '{}': `connection_ref` `{conn_ref}` names the `{ref_toolkit}` toolkit but the \
                 tool_call slug `{slug}` is a `{slug_toolkit}` action{hint}.",
                node.id
            ));
            continue;
        }

        // Existence check: the id must name a real connected account of this
        // toolkit. Skipped (fail-open) when the connection list is unavailable.
        let Some(conns) = connections else {
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %ref_id,
                "[flows] connection-ref check: toolkit matches; id-existence check skipped (connections unavailable)"
            );
            continue;
        };
        // The id must belong to a connection OF THIS TOOLKIT — not merely
        // exist somewhere. The transcript bug was a real TIKTOK connection id
        // stamped onto a `composio:twitter:` ref: the id exists globally, but
        // it is not a Twitter account, so it must still be rejected.
        let id_exists = conns.iter().any(|c| {
            c.toolkit
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(&slug_toolkit))
                && parse_composio_connection_ref(&c.connection_ref)
                    .is_some_and(|(_, cid)| cid.eq_ignore_ascii_case(ref_id))
        });
        if id_exists {
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                toolkit = %slug_toolkit,
                %ref_id,
                matched = true,
                "[flows] connection-ref check: ref resolves to a real connected account — ok"
            );
            continue;
        }
        // Unknown id. Name the right ref for this toolkit if one exists.
        match first_connection_ref_for_toolkit(conns, &slug_toolkit) {
            Some(r) => {
                tracing::debug!(
                    target: "flows",
                    node = %node.id,
                    %slug,
                    toolkit = %slug_toolkit,
                    %ref_id,
                    matched = false,
                    "[flows] connection-ref check: unknown id; toolkit has a different connected account — rejecting"
                );
                errors.push(format!(
                    "Node '{}': `connection_ref` `{conn_ref}` does not match any connected \
                     `{slug_toolkit}` account — did you mean `{r}`? Call list_flow_connections and \
                     copy a `connection_ref` value verbatim.",
                    node.id
                ));
            }
            None => {
                tracing::debug!(
                    target: "flows",
                    node = %node.id,
                    %slug,
                    toolkit = %slug_toolkit,
                    %ref_id,
                    matched = false,
                    "[flows] connection-ref check: no connected account for this toolkit — rejecting"
                );
                errors.push(format!(
                    "Node '{}': `connection_ref` `{conn_ref}` names a `{slug_toolkit}` account, but \
                     no `{slug_toolkit}` account is connected — connect one with composio_connect \
                     (or ask the user to), then use its `connection_ref`.",
                    node.id
                ));
            }
        }
    }
    errors
}

// ─────────────────────────────────────────────────────────────────────────────
// Required-arg resolvability gate (issue B18)
// ─────────────────────────────────────────────────────────────────────────────
//
// `validate_tool_contracts` (above) proves a required arg is PRESENT
// (`missing_required_args`: absent or literal `null`) — it has no opinion on
// whether an arg wired to a real-looking `=`-expression actually RESOLVES to
// something at runtime, and it says nothing at all about an arg the live
// schema doesn't individually mark `required` even though the PROVIDER
// enforces it as a business rule — e.g. `GMAIL_SEND_EMAIL.subject`/`.body`
// are each individually optional in the schema, but Gmail rejects a send
// where BOTH are empty ("At least one of 'subject' or 'body' must be
// provided with non-empty content"). A builder can wire either to an
// upstream path that looks fully wired but resolves `null`, and neither
// static check above has anything to say about it.
//
// `crate::openhuman::flows::builder_tools::DryRunWorkflowTool` already
// detects exactly this class of null resolution (`null_resolutions`) by
// running the graph through the same MOCK sandbox — but only as information
// the agent is *instructed* (by prompt, not enforced in code) to act on
// before calling `propose_workflow`/`save_workflow`. Nothing previously
// stopped those tools from persisting the graph anyway.
// [`validate_required_arg_resolvability`] closes that gap: it re-runs the
// identical sandbox check and escalates ANY arg of a real (non-`=`-derived,
// non-native) `tool_call` node that resolved `null` to a hard reject, wired
// into `propose_workflow` / `revise_workflow` / `save_workflow` alongside
// [`validate_binding_resolvability`] and [`validate_tool_contracts`].

/// Wall-clock bound on the sandbox run this gate performs. Mirrors
/// `builder_tools::DRY_RUN_TIMEOUT_SECS`'s purpose but kept short: unlike the
/// opt-in `dry_run_workflow` tool, this check runs on EVERY
/// propose/revise/save call, so a slow or pathological draft must not stall
/// authoring.
const REQUIRED_ARG_NULL_CHECK_TIMEOUT_SECS: u64 = 15;

/// Sandbox-executes `graph` against `tinyflows`' deterministic MOCK
/// capabilities (the same shape `DryRunWorkflowTool` uses — see this
/// section's module doc) and returns one human-readable error per arg of a
/// real (non-`=`-derived, non-native) `tool_call` node whose `=`-expression
/// resolved to `null` during that run **and** whose expression is wired to a
/// specific upstream node's output (directly, via the implicit
/// `item`/`items` scope, or explicitly via `nodes.<id>...`) rather than to
/// the trigger.
///
/// This run always sandboxes against `json!({})` as the trigger payload (see
/// below), so any arg wired to trigger-scoped data — `=item.<field>` /
/// `=items...` fed directly from the trigger node, or `=run.<field>` (the
/// trigger metadata itself) — legitimately resolves `null` here even though a
/// real webhook/app-event/manual trigger WILL populate it at runtime. Hard
/// gate that on an empty mock run would reject every ordinary trigger-bound
/// workflow (Codex feedback on PR #4826). Only a `null` resolved from a
/// genuine upstream **node** reference is escalated — that's the real B18
/// bug this gate exists to catch: an arg wired to a node output path that can
/// never resolve (e.g. `GMAIL_SEND_EMAIL.subject =
/// "=nodes.build_body.item.subject"` where `build_body` never produces
/// `subject`), which stays broken no matter what the trigger payload is.
///
/// Deliberately does **not** wrap the mock `ToolInvoker` in
/// [`crate::openhuman::flows::tinyflows::caps::PreflightToolInvoker`] the way
/// `DryRunWorkflowTool` does: that wrapper aborts the WHOLE sandbox run the
/// instant a node with a `stop` `on_error` policy (the default) hits a
/// schema-required null arg, which would lose the per-field diagnostic this
/// gate exists to report for every OTHER node — and this check cares about
/// EVERY arg, not just ones the schema happens to mark `required`. The plain
/// mock tool invoker always "succeeds" (a deterministic echo), so the run
/// settles and every node's config-resolution diagnostics get captured
/// regardless of on_error policy or schema required-ness.
///
/// Best-effort, same posture as [`validate_tool_contracts`]: a compile
/// failure (structural errors are already caught by
/// [`validate_and_migrate_graph`] before this gate ever runs) or a sandbox
/// error/timeout is SKIPPED — never turned into a false rejection. This
/// check only ever adds a diagnostic the sandbox actually observed.
pub(crate) async fn validate_required_arg_resolvability(graph: &WorkflowGraph) -> Vec<String> {
    use crate::openhuman::flows::builder_tools::CapturingObserver;
    use crate::openhuman::flows::tinyflows::caps::{
        SchemaAwareMockAgentRunner, SchemaAwareMockLlm,
    };

    let Ok(compiled) = tinyflows::compiler::compile(graph) else {
        return Vec::new();
    };

    let mut caps = tinyflows::caps::mock::mock_capabilities_with_agent(SchemaAwareMockAgentRunner);
    // Same fix as `DryRunWorkflowTool`: a plain agent node (no `agent_ref`)
    // routes to the `llm` slot, not the runner above, so the vendored `MockLlm`
    // echo would fail its `output_parser.schema` sub-port and make this gate
    // reject a correct graph (which is why `propose_workflow` was rejecting
    // valid graphs). The schema-aware mock LLM honors the schema instead.
    caps.llm = Arc::new(SchemaAwareMockLlm);

    let observer = Arc::new(CapturingObserver::default());
    let observer_dyn: Arc<dyn tinyflows::observability::RunObserver> = observer.clone();
    let run = tinyflows::engine::run_with_observer(&compiled, json!({}), &caps, &observer_dyn);
    if tokio::time::timeout(
        std::time::Duration::from_secs(REQUIRED_ARG_NULL_CHECK_TIMEOUT_SECS),
        run,
    )
    .await
    .is_err()
    {
        // Timed out — a different class of problem than this gate exists to
        // catch; never block authoring on it here.
        return Vec::new();
    }
    // A sandbox `Err` outcome here is a compile/capability issue unrelated
    // to null args (the plain mock invoker never itself fails) — surfaced by
    // the other gates / `dry_run_workflow` instead; this gate only adds
    // diagnostics from a run that actually settled, so an error is silently
    // skipped rather than turned into a (misleading) empty-errors success.

    let tool_call_slugs: std::collections::HashMap<&str, &str> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::ToolCall)
        .filter_map(|n| {
            let slug = n.config.get("slug").and_then(Value::as_str)?;
            Some((n.id.as_str(), slug))
        })
        .collect();

    // The trigger node's id, if any — used below to tell a trigger-scoped
    // `item`/`items` reference (the direct predecessor IS the trigger) apart
    // from a real upstream-node reference. Graphs are expected to have
    // exactly one trigger; `flows_validate` rejects zero/multiple before this
    // gate ever runs, so `first()` here doesn't hide ambiguity.
    let trigger_id: Option<&str> = graph
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Trigger)
        .map(|n| n.id.as_str());

    let mut errors = Vec::new();
    for step in observer.steps() {
        let Some(&slug) = tool_call_slugs.get(step.node_id.as_str()) else {
            continue;
        };
        // `=`-derived slugs resolve from upstream/trigger data at runtime;
        // native `oh:` tools have no external-provider rejection mode.
        if slug.starts_with('=') || slug.starts_with("oh:") {
            continue;
        }
        for diag in &step.diagnostics {
            let Some(field) = diag.location.strip_prefix("args.") else {
                continue;
            };
            if is_trigger_scoped_expression(&diag.expression, graph, &step.node_id, trigger_id) {
                // Legitimately empty in this gate's `{}` mock run — the real
                // trigger (webhook/app-event/manual) will populate it. Not
                // the B18 broken-wiring case this gate exists to catch.
                tracing::debug!(
                    target: "flows",
                    node = %step.node_id,
                    %slug,
                    %field,
                    expression = %diag.expression,
                    "[flows] required-arg resolvability check: trigger-scoped null in empty \
                     mock run — not rejecting"
                );
                continue;
            }
            // A null bound to the OUTPUT of an upstream Composio-or-native
            // `tool_call` node is UNVERIFIABLE in this echo sandbox — the mock
            // renders BOTH a Composio and a native `oh:` `tool_call` as
            // `{tool, args, connection}` and can NEVER produce their real output
            // fields (`.item.json.data.<field>` for Composio, `.item.json.<field>`
            // for a native tool), so a downstream binding to one resolves `null`
            // here even when the wiring is perfectly correct. Hard-rejecting it
            // (WS6) would block a possibly-correct graph from ever being proposed
            // — the exact false-negative the transcript audit caught, and the one
            // that made this gate reject #5148's own native-attachment chain.
            // Downgrade to a debug-logged skip; `dry_run_workflow` remains the
            // surface that reports it (as an `unverifiable` diagnostic the agent
            // can act on via get_tool_contract / get_tool_output_sample).
            if let Some(upstream) =
                mock_opaque_tool_call_upstream_ref(&diag.expression, graph, &step.node_id)
            {
                tracing::debug!(
                    target: "flows",
                    node = %step.node_id,
                    %slug,
                    %field,
                    upstream = %upstream,
                    expression = %diag.expression,
                    "[flows] required-arg resolvability check: arg binds to a Composio-or-native \
                     tool_call's output — UNVERIFIABLE in the echo sandbox (the mock cannot \
                     produce real tool output fields), not rejecting; dry_run_workflow \
                     reports it instead"
                );
                continue;
            }
            tracing::warn!(
                target: "flows",
                node = %step.node_id,
                %slug,
                %field,
                expression = %diag.expression,
                "[flows] required-arg resolvability check: arg resolved null in sandbox — \
                 rejecting"
            );
            errors.push(format!(
                "Node '{}': arg `{field}` of `{slug}` (`{}`) resolved to `null` during a \
                 sandboxed test run — an empty/missing `{field}` can be rejected by the real \
                 provider at runtime (e.g. Gmail rejects a send with no subject or body). \
                 Rewire it from an upstream node's output that actually has a value — call \
                 dry_run_workflow to see exactly which upstream field is null — or drop the \
                 field from args if it isn't really needed.",
                step.node_id, diag.expression
            ));
        }
    }
    errors
}

/// Returns the node id an explicit `nodes.<id>...` expression addresses —
/// either the legacy dotted shorthand (`=nodes.build_body.item.subject`) or
/// the jq bracket form (`=.nodes["build_body"].item.subject`) — or `None` if
/// the expression's root isn't the `nodes` scope key at all. The expression
/// scope's shape (`item` / `items` / `run` / `nodes`) is documented on
/// `tinyflows`'s `expr` module and `nodes::expr_scope`.
fn explicit_nodes_ref(expr: &str) -> Option<&str> {
    let body = expr.strip_prefix('=')?.trim();
    let body = body.strip_prefix('.').unwrap_or(body);
    let rest = body.strip_prefix("nodes")?;
    if let Some(after_dot) = rest.strip_prefix('.') {
        // Dotted shorthand: `nodes.<id>.item.<field>` — the id ends at the
        // next `.` or `[`.
        let id = after_dot.split(['.', '[']).next()?;
        (!id.is_empty()).then_some(id)
    } else if let Some(after_bracket) = rest.strip_prefix('[') {
        // jq bracket form: `nodes["<id>"]` / `nodes['<id>']`.
        let after_bracket = after_bracket.trim_start();
        let after_bracket = after_bracket
            .strip_prefix('"')
            .or_else(|| after_bracket.strip_prefix('\''))
            .unwrap_or(after_bracket);
        let id = after_bracket.split(['"', '\'', ']']).next()?;
        (!id.is_empty()).then_some(id)
    } else {
        // `rest` is empty (bare `nodes`) or continues some other identifier
        // (e.g. a hypothetical `nodesomething` — not this scope key at all).
        None
    }
}

/// Whether a null-resolved config expression on `node_id` is scoped to the
/// TRIGGER's data rather than a specific upstream node's output — and
/// therefore legitimately empty in [`validate_required_arg_resolvability`]'s
/// `{}` mock run rather than evidence of broken wiring (see that function's
/// doc comment and the Codex feedback it links).
///
/// - `=run...` always addresses the trigger payload/metadata directly
///   (`crate::openhuman::flows::tinyflows`'s `expr_scope` docs) — always
///   trigger-scoped.
/// - `=nodes.<id>...` / `=.nodes["<id>"]...` explicitly names an upstream
///   node. Trigger-scoped only if `<id>` IS the trigger node; naming any
///   other node is exactly the B18 broken-wiring case this gate exists to
///   catch, so it is never treated as trigger-scoped.
/// - `=item...` / `=items...` implicitly addresses `node_id`'s direct
///   predecessor(s) output. Trigger-scoped only when EVERY incoming edge to
///   `node_id` comes from the trigger node — a fan-in that mixes the trigger
///   with a real upstream node, or an `item`/`items` reference fed entirely
///   by real upstream nodes, keeps the existing (reject) behavior, since a
///   node that already ran in the sandbox is expected to have produced its
///   real, deterministic output.
/// - Anything else (a jq expression not rooted at one of the above, or a
///   malformed one) is conservatively treated as NOT trigger-scoped, matching
///   this gate's pre-existing behavior.
fn is_trigger_scoped_expression(
    expr: &str,
    graph: &WorkflowGraph,
    node_id: &str,
    trigger_id: Option<&str>,
) -> bool {
    let body = expr.strip_prefix('=').unwrap_or(expr).trim();
    let body = body.strip_prefix('.').unwrap_or(body);

    if body == "run" || body.starts_with("run.") || body.starts_with("run[") {
        return true;
    }

    if let Some(referenced_id) = explicit_nodes_ref(expr) {
        return trigger_id == Some(referenced_id);
    }

    let is_item_scoped = body == "item"
        || body.starts_with("item.")
        || body.starts_with("item[")
        || body == "items"
        || body.starts_with("items.")
        || body.starts_with("items[");
    if !is_item_scoped {
        return false;
    }

    let Some(trigger_id) = trigger_id else {
        return false;
    };
    let mut predecessors = graph
        .edges
        .iter()
        .filter(|e| e.to_node == node_id)
        .peekable();
    predecessors.peek().is_some() && predecessors.all(|e| e.from_node == trigger_id)
}

/// If a null-resolved config expression on `node_id` is bound to the OUTPUT of
/// an upstream **`tool_call`** node whose sandbox output is an opaque echo — a
/// Composio curated action OR a native `oh:` tool (anything but a `=`-derived
/// dynamic slug) — returns that upstream node's id; otherwise `None`.
///
/// The dry-run / gate sandbox renders BOTH a Composio `tool_call` and a native
/// `oh:` `tool_call` as a deterministic echo (`{tool, args, connection}`) and
/// can NEVER produce their real output fields, so a downstream binding off such
/// a node (`.item.json.data.<field>` for Composio, or `.item.json.<field>` for
/// a native tool after `native_tool_payload`'s unwrap) resolves `null` in the
/// sandbox **even when the wiring is correct** — the binding is UNVERIFIABLE
/// here, not necessarily broken. Callers use this to tell that honest-
/// uncertainty case apart from a genuinely broken binding (one wired to an
/// `agent` / `transform` / `code` / trigger upstream, whose real output the
/// sandbox DOES produce, so a null there IS a real bug).
///
/// The native `oh:` case is why this exists beyond Composio: #5148's guidance
/// prescribes a `produce -> oh:storage_upload_file -> oh:storage_get_link ->
/// send` chain where the send binds `=nodes.get_link.item.json.url`; excluding
/// native upstreams here made the gate hard-reject that exact (correct) chain.
///
/// Handles both addressing forms the engine can trace:
/// - explicit `=nodes.<id>...` / `=.nodes["<id>"]...` (parsed via
///   [`explicit_nodes_ref`]), and
/// - implicit `=item...` / `=items...`, resolved against `node_id`'s direct
///   predecessor — but only when there is exactly ONE incoming edge, so an
///   ambiguous fan-in is never mis-attributed to a single upstream node.
///
/// Anything else (a `=run...` trigger reference, a jq expression not rooted at
/// one of the above, or a reference to a non-`tool_call` / `=`-dynamic node)
/// returns `None`.
pub(crate) fn mock_opaque_tool_call_upstream_ref<'a>(
    expr: &str,
    graph: &'a WorkflowGraph,
    node_id: &str,
) -> Option<&'a str> {
    let referenced_id: String = if let Some(id) = explicit_nodes_ref(expr) {
        id.to_string()
    } else {
        let body = expr.strip_prefix('=').unwrap_or(expr).trim();
        let body = body.strip_prefix('.').unwrap_or(body);
        let is_item_scoped = body == "item"
            || body.starts_with("item.")
            || body.starts_with("item[")
            || body == "items"
            || body.starts_with("items.")
            || body.starts_with("items[");
        if !is_item_scoped {
            return None;
        }
        let mut preds = graph
            .edges
            .iter()
            .filter(|e| e.to_node == node_id)
            .map(|e| e.from_node.as_str());
        let first = preds.next()?;
        if preds.next().is_some() {
            // Ambiguous fan-in — cannot attribute the null to one upstream node.
            return None;
        }
        first.to_string()
    };
    let node = graph.nodes.iter().find(|n| n.id == referenced_id)?;
    if node.kind != NodeKind::ToolCall {
        return None;
    }
    let slug = node.config.get("slug").and_then(Value::as_str)?;
    // A `=`-derived slug is a dynamic runtime slug we can't reason about. But a
    // native `oh:` tool_call IS opaque-echoed by the mock exactly like a
    // Composio one, so its downstream null is equally unverifiable, not broken —
    // do NOT exclude it (that exclusion made the gate reject #5148's own chain).
    if slug.starts_with('=') {
        return None;
    }
    Some(node.id.as_str())
}

/// Validates a candidate graph without persisting it — the same
/// migrate/validate path `flows_create` and `ProposeWorkflowTool` use — and
/// reports structural errors alongside non-fatal trigger warnings
/// ([`graph_trigger_warnings`]). Backs `openhuman.flows_validate` (PHASE 3c):
/// an authoring surface can call this to preview validity + warnings before a
/// save. Pure (no persistence, no config) — `valid == false` is a normal
/// result, NOT an `Err`; `Err` is reserved for internal serialization faults
/// (there are none on this path today).
pub fn flows_validate(graph_json: Value) -> RpcOutcome<crate::openhuman::flows::FlowValidation> {
    use crate::openhuman::flows::FlowValidation;
    tracing::debug!(target: "flows", "[flows] flows_validate: validating candidate graph");
    // Split migrate/deserialize (a genuinely single failure) from structural
    // validation (which can surface many problems at once). A pre-validation
    // failure short-circuits with one error; a deserializable graph is then run
    // through `validate_all` so the author sees every structural problem in one
    // pass instead of one round-trip per error.
    let graph = match migrate_and_deserialize_graph(graph_json) {
        Ok(graph) => graph,
        Err(error) => {
            tracing::debug!(target: "flows", %error, "[flows] flows_validate: graph could not be migrated/parsed");
            return RpcOutcome::single_log(
                FlowValidation {
                    valid: false,
                    errors: vec![error.clone()],
                    error_details: vec![crate::openhuman::flows::FlowValidationError {
                        code: "unparseable_graph".to_string(),
                        message: error,
                        node_id: None,
                        field: None,
                    }],
                    warnings: Vec::new(),
                },
                "flow validation failed",
            );
        }
    };

    let structural = tinyflows::validate::validate_all(&graph);
    if !structural.is_empty() {
        let error_details: Vec<_> = structural.iter().map(to_flow_validation_error).collect();
        let errors: Vec<String> = error_details.iter().map(|e| e.message.clone()).collect();
        tracing::debug!(
            target: "flows",
            error_count = errors.len(),
            "[flows] flows_validate: graph is structurally invalid"
        );
        return RpcOutcome::single_log(
            FlowValidation {
                valid: false,
                errors,
                error_details,
                warnings: Vec::new(),
            },
            "flow validation failed",
        );
    }

    let error_details = engine_compatibility_errors(&graph);
    if !error_details.is_empty() {
        let errors = error_details
            .iter()
            .map(|error| error.message.clone())
            .collect();
        tracing::debug!(
            target: "flows",
            error_count = error_details.len(),
            "[flows] flows_validate: graph uses an unsupported engine topology"
        );
        return RpcOutcome::single_log(
            FlowValidation {
                valid: false,
                errors,
                error_details,
                warnings: Vec::new(),
            },
            "flow validation failed",
        );
    }

    let warnings = graph_trigger_warnings(&graph);
    for warning in &warnings {
        tracing::warn!(target: "flows", warning = %warning, "[flows] flows_validate: non-fatal validation warning");
    }
    tracing::debug!(
        target: "flows",
        node_count = graph.nodes.len(),
        warning_count = warnings.len(),
        "[flows] flows_validate: graph is structurally valid"
    );
    RpcOutcome::single_log(
        FlowValidation {
            valid: true,
            errors: Vec::new(),
            error_details: Vec::new(),
            warnings,
        },
        "flow validated",
    )
}

/// Imports a workflow definition WITHOUT persisting it (PHASE 4d), normalizing
/// it into a migrated + validated [`WorkflowGraph`] the UI opens as an editable
/// canvas *draft*. Two source formats, selected by `format`:
///
/// - `"native"` — a tinyflows `WorkflowGraph` JSON (the same shape
///   `flows_create` accepts). Run straight through [`validate_and_migrate_graph`].
/// - `"n8n"` — an n8n workflow export, mapped best-effort by
///   [`crate::openhuman::flows::n8n_import`] into a `WorkflowGraph` (unmapped
///   node types become annotated placeholders, expressions translated where
///   trivial) and THEN run through the same migrate + validate path, so the
///   host engine is the authority on the result's validity.
/// - `None`/`"auto"` — auto-detect: n8n exports carry a `connections` object /
///   `type`-discriminated nodes ([`n8n_import::looks_like_n8n`]); everything
///   else is treated as native.
///
/// Returns `Err` when the (post-mapping) graph is structurally invalid or the
/// JSON is unparseable — import declines rather than handing the canvas a graph
/// that can't be saved. On success the `warnings` carry every non-fatal import
/// approximation (n8n only; native import is warning-free).
///
/// Like `flows_validate`, this is pure: NO persistence, NO enablement. The
/// user's later Save (the existing `flows_create` gate) is the only write.
pub fn flows_import(
    graph_json: Value,
    format: Option<String>,
) -> Result<RpcOutcome<crate::openhuman::flows::FlowImport>, String> {
    use crate::openhuman::flows::{n8n_import, FlowImport};

    let requested = format
        .as_deref()
        .unwrap_or("auto")
        .trim()
        .to_ascii_lowercase();
    let is_n8n = match requested.as_str() {
        "n8n" => true,
        "native" | "tinyflows" => false,
        "auto" | "" => n8n_import::looks_like_n8n(&graph_json),
        other => {
            return Err(format!(
                "unknown import format '{other}' (expected 'native' or 'n8n')"
            ))
        }
    };
    tracing::debug!(
        target: "flows",
        requested_format = %requested,
        resolved = if is_n8n { "n8n" } else { "native" },
        "[flows] flows_import: importing workflow definition"
    );

    let (candidate, mut warnings) = if is_n8n {
        let mapped = n8n_import::map_n8n_workflow(&graph_json)?;
        // Re-serialize the mapped graph so it re-enters the exact same
        // migrate + validate path a native import takes (single source of truth
        // for validity), rather than trusting the mapper's in-memory graph.
        let value = serde_json::to_value(&mapped.graph).map_err(|e| e.to_string())?;
        (value, mapped.warnings)
    } else {
        (graph_json, Vec::new())
    };

    let graph = validate_and_migrate_graph(candidate)?;
    // Host-side trigger warnings apply to both formats (e.g. an imported
    // webhook trigger that this host does not yet self-fire).
    warnings.extend(graph_trigger_warnings(&graph));
    tracing::debug!(
        target: "flows",
        node_count = graph.nodes.len(),
        warning_count = warnings.len(),
        "[flows] flows_import: import normalized and validated"
    );
    Ok(RpcOutcome::single_log(
        FlowImport { graph, warnings },
        "flow imported",
    ))
}

/// Creates a new flow from a name and a raw graph JSON value.
///
/// Issue B29 (save/enable safety) — two server-side rules apply here,
/// authoritative regardless of what the caller passed, so no creation path
/// (prompt bar, scratch/template modal, proposal "save & enable", copilot
/// `save_workflow`, …) can silently hand the user an armed, unattended
/// automation:
///
/// - **Rule 1** ([`trigger_is_automatic`]): a graph whose trigger fires
///   without a human in the loop (`schedule` / `app_event` / `webhook`)
///   persists **disabled**. The user arms it explicitly via
///   `flows_set_enabled` — the same toggle already used everywhere else. A
///   `manual` trigger (or no trigger-kind discriminator at all) still
///   persists enabled: it only ever runs via an explicit `flows_run`, so
///   there is no surprise, and gating it would just add friction.
///
///   This means a caller that represents an explicit user-arming action
///   (e.g. `WorkflowProposalCard`'s "Save & enable" click,
///   `app/src/components/chat/WorkflowProposalCard.tsx`) must check the
///   returned [`Flow`]'s `enabled` field and follow up with
///   `flows_set_enabled(id, true)` when it comes back `false` — otherwise
///   the button's own label lies to the user. That follow-up call is a
///   legitimate, explicit enable, not the silent copilot auto-arm this rule
///   exists to prevent (the copilot's `save_workflow` path has no such
///   follow-up and stays disabled).
/// - **Rule 2** ([`graph_has_outbound_side_effect`]): a graph containing any
///   `tool_call` / `http_request` / `code` node — the three kinds that can
///   produce a real outbound effect — forces `require_approval: true`,
///   overriding whatever the caller passed. A read-only graph (only
///   `trigger` / `agent` / `transform` / `condition` / data-flow nodes) is
///   unaffected.
///
/// An enabled flow still has its automatic-dispatch side effect bound
/// immediately (e.g. the schedule-trigger cron job registered), reusing the
/// same [`bind_trigger`] helper `flows_set_enabled` uses — but per Rule 1
/// that now only happens for a `manual`-triggered (or trigger-kind-less)
/// flow. Best-effort, same as `flows_set_enabled`: a binding failure is
/// logged, not fatal to create.
pub async fn flows_create(
    config: &Config,
    name: String,
    graph_json: Value,
    require_approval: bool,
) -> Result<RpcOutcome<Flow>, String> {
    let graph = validate_and_migrate_graph(graph_json)?;
    ensure_config_aware_engine_compatible(config, &graph)?;

    // Rule 1: automatic triggers create DISABLED — the user must arm them
    // explicitly.
    let enabled = !trigger_is_automatic(&graph);

    // Rule 2: any outbound side-effect node forces require_approval, no
    // matter what the caller asked for.
    let (effective_require_approval, side_effect_forced) =
        enforce_side_effect_approval(&graph, require_approval);
    if side_effect_forced {
        tracing::info!(
            target: "flows",
            %name,
            "[flows] flows_create: forcing require_approval=true — graph contains outbound \
             side-effect node(s) (tool_call / http_request / code)"
        );
    }

    tracing::debug!(
        target: "flows",
        %name,
        node_count = graph.nodes.len(),
        enabled,
        require_approval = effective_require_approval,
        "[flows] flows_create: persisting new flow"
    );
    let flow = store::create_flow(config, name, graph, effective_require_approval, enabled)
        .map_err(|e| e.to_string())?;

    if flow.enabled {
        tracing::debug!(target: "flows", flow_id = %flow.id, "[flows] flows_create: flow is enabled — binding automatic-dispatch trigger");
        bind_trigger(config, &flow);
    }

    let mut logs = vec!["flow created".to_string()];
    if !enabled {
        let trigger_label = flow
            .graph
            .trigger()
            .and_then(|t| t.config.get("trigger_kind"))
            .and_then(Value::as_str)
            .unwrap_or("automatic");
        logs.push(format!(
            "Flow created DISABLED because it has an automatic trigger ({trigger_label}). \
             Enable it explicitly (flows_set_enabled) when you are ready for it to fire."
        ));
    }
    if side_effect_forced {
        logs.push(
            "require_approval forced to true because the graph contains outbound side-effect \
             nodes (tool_call / http_request / code)."
                .to_string(),
        );
    }

    publish_flow_changed(&flow.id, "created", "system");
    Ok(RpcOutcome::new(flow, logs))
}

/// Duplicates a saved flow: creates an independent copy of its graph under a
/// new id/timestamps, with the name suffixed `" (copy)"`. The copy is created
/// **disabled** (`enabled = false`) and therefore **not** schedule/app_event
/// trigger-bound — unlike [`flows_create`], which binds a trigger for an
/// enabled flow, this deliberately calls no [`bind_trigger`], so a duplicate
/// can never immediately fire. Run history does not carry over. The user
/// enables it explicitly (via `flows_set_enabled`) once they've reviewed the
/// copy, at which point its trigger binds like any other flow.
pub async fn flows_duplicate(config: &Config, id: &str) -> Result<RpcOutcome<Flow>, String> {
    let source = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{id}' not found"))?;
    let new_name = format!("{} (copy)", source.name);
    tracing::debug!(target: "flows", source_id = %id, %new_name, "[flows] flows_duplicate: creating disabled, unbound copy");
    let flow =
        store::insert_duplicate_flow(config, &source, new_name).map_err(|e| e.to_string())?;
    // Intentionally NO bind_trigger: a duplicate is disabled and must stay
    // inert (no schedule/trigger dispatch) until the user enables it.
    publish_flow_changed(&flow.id, "created", "system");
    Ok(RpcOutcome::single_log(
        flow,
        format!("flow duplicated from {id}"),
    ))
}

/// Loads one flow by id.
pub async fn flows_get(config: &Config, id: &str) -> Result<RpcOutcome<Flow>, String> {
    let flow = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{id}' not found"))?;
    Ok(RpcOutcome::single_log(flow, format!("flow loaded: {id}")))
}

/// Loads a saved flow's portable [`WorkflowGraph`] by id, for the
/// `sub_workflow`-by-`workflow_id` resolver capability
/// (`tinyflows::caps::WorkflowResolver`, implemented in
/// `src/openhuman/flows/tinyflows/caps.rs`).
///
/// Returns `Ok(None)` when no flow with that id exists (the resolver turns that
/// into a capability error naming the missing id), and `Err` only on a store
/// failure. Kept sync (the underlying [`store::get_flow`] is sync) so the
/// resolver can call it directly from its async method without a runtime hop.
pub fn load_flow_graph(config: &Config, id: &str) -> Result<Option<WorkflowGraph>, String> {
    tracing::debug!(target: "flows", flow_id = %id, "[flows] load_flow_graph: loading saved flow graph for sub_workflow resolver");
    let graph = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .map(|flow| flow.graph);
    tracing::debug!(
        target: "flows",
        flow_id = %id,
        found = graph.is_some(),
        "[flows] load_flow_graph: resolver lookup complete"
    );
    Ok(graph)
}

/// Resolver-only saved-graph lookup. Authoring tools use [`load_flow_graph`]
/// so a legacy draft can still be opened and repaired; execution resolves only
/// graphs the current engine can run safely.
pub(crate) fn load_engine_compatible_flow_graph(
    config: &Config,
    id: &str,
) -> Result<Option<WorkflowGraph>, String> {
    let graph = load_flow_graph(config, id)?;
    if let Some(graph) = graph.as_ref() {
        ensure_config_aware_engine_compatible(config, graph)
            .map_err(|error| format!("workflow_id '{id}' is engine-incompatible: {error}"))?;
    }
    Ok(graph)
}

/// Lists every saved flow.
///
/// A corrupt or newer-schema-than-this-build `graph_json` row is skipped
/// rather than failing the whole list (R-M4 — see `store::list_flow_rows`);
/// when that happens it must not be silent, so a skip is both logged
/// (`[flows]`-prefixed, id + error only — never row content) and surfaced in
/// the RPC's `logs` so the UI can tell the user "N workflows could not be
/// loaded" instead of silently rendering a shorter list than actually exists.
pub async fn flows_list(config: &Config) -> Result<RpcOutcome<Vec<Flow>>, String> {
    let (flows, skipped) = store::list_flows(config).map_err(|e| e.to_string())?;
    if skipped > 0 {
        tracing::warn!(
            target: "flows",
            skipped,
            loaded = flows.len(),
            "[flows] flows_list: skipped corrupt/unmigratable flow_definitions rows"
        );
        Ok(RpcOutcome::new(
            flows,
            vec![format!(
                "flows listed ({skipped} workflow{} could not be loaded and were skipped)",
                if skipped == 1 { "" } else { "s" }
            )],
        ))
    } else {
        Ok(RpcOutcome::single_log(flows, "flows listed"))
    }
}

/// Lists the connection sources a flow node's `connection_ref` can attach to:
/// Composio connected accounts (`kind = "composio"`) and stored HTTP
/// credentials (`kind = "http"`). This is the picker source for the Workflows
/// UI (and the agent's flow-authoring surface) — it returns ids + display
/// labels + kind ONLY, never any secret material.
///
/// The two sources are aggregated independently and are individually
/// fault-tolerant: a transient Composio backend/network failure (or an
/// unconfigured Direct-mode key) yields zero Composio entries but still returns
/// the HTTP credential half, and vice-versa. A failure in one source never
/// fails the whole picker.
pub async fn flows_list_connections(
    config: &Config,
) -> Result<RpcOutcome<Vec<FlowConnection>>, String> {
    tracing::debug!(
        "[flows] rpc flows_list_connections: aggregating composio + http_cred picker sources"
    );
    let mut logs = Vec::new();

    // 1. Composio connected accounts. Direct mode without a configured key
    //    already short-circuits to an empty list (a valid setup state, not an
    //    error); a backend outage returns Err — tolerate it so the picker still
    //    surfaces HTTP credentials.
    let composio_conns =
        match crate::openhuman::integrations::composio::ops::composio_list_connections(config).await
        {
            Ok(outcome) => {
                tracing::debug!(
                    count = outcome.value.connections.len(),
                    "[flows] flows_list_connections: composio source returned connections"
                );
                outcome.value.connections
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[flows] flows_list_connections: composio source unavailable — \
                     returning http_cred entries only"
                );
                logs.push(format!(
                    "flows_list_connections: composio source unavailable ({e})"
                ));
                Vec::new()
            }
        };

    // 2. Named HTTP credentials — secret-free summaries (the store never hands
    //    out secret material here; injection happens server-side in
    //    `tinyflows::caps::OpenHumanHttp`).
    let http_creds =
        match crate::openhuman::security::credentials::HttpCredentialsStore::from_config(config)
            .list()
        {
            Ok(list) => {
                tracing::debug!(
                    count = list.len(),
                    "[flows] flows_list_connections: http_cred store returned summaries"
                );
                list
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[flows] flows_list_connections: http_cred store read failed — \
                     returning composio entries only"
                );
                logs.push(format!(
                    "flows_list_connections: http_cred store unavailable ({e})"
                ));
                Vec::new()
            }
        };

    // Connected-account identities (email/handle/platform user id), synced
    // via each toolkit's whoami-style call (e.g. Slack `SLACK_TEST_AUTH`) on
    // connection sync. Loaded once here so `build_flow_connections` can stay
    // a pure, unit-testable matcher.
    let identities =
        crate::openhuman::integrations::composio::providers::profile::load_connected_identities();
    tracing::debug!(
        count = identities.len(),
        "[flows] flows_list_connections: identity-cache load"
    );
    let connections = build_flow_connections(composio_conns, http_creds, &identities);
    tracing::debug!(
        total = connections.len(),
        "[flows] flows_list_connections: aggregated picker sources"
    );
    logs.push(format!(
        "flows_list_connections: {} connection(s)",
        connections.len()
    ));
    Ok(RpcOutcome::new(connections, logs))
}

/// Fold Composio connected accounts + named HTTP credentials into the flat,
/// secret-free [`FlowConnection`] picker list. Only ACTIVE Composio connections
/// are surfaced — a pending/expired OAuth account cannot execute a tool, so it
/// would be a dead pick. Pure (no I/O) so the aggregation shape is
/// unit-testable without a live backend; `identities` is loaded once by the
/// caller and matched in here.
///
/// Each Composio connection is also matched against `identities` (keyed by
/// `(toolkit, connection_id)`, both normalized the same way
/// `enrich_connections_with_identity` in `composio::ops::connections` does)
/// to attach `platform_user_id` — the connected account's own member id
/// (e.g. Slack `U123ABC`). This is what lets the workflow builder wire a
/// self-targeted action ("DM me") to the user's own account instead of
/// guessing a public channel.
fn build_flow_connections(
    composio: Vec<crate::openhuman::integrations::composio::ComposioConnection>,
    http: Vec<crate::openhuman::security::credentials::HttpCredentialSummary>,
    identities: &[crate::openhuman::integrations::composio::providers::profile::ConnectedIdentity],
) -> Vec<FlowConnection> {
    use crate::openhuman::integrations::composio::providers::profile::normalize_connection_identifier;

    let identity_lookup: std::collections::HashMap<(String, String), &_> = identities
        .iter()
        .map(|id| {
            (
                (
                    normalize_connection_identifier(&id.source),
                    normalize_connection_identifier(&id.identifier),
                ),
                id,
            )
        })
        .collect();

    let mut out = Vec::with_capacity(composio.len() + http.len());
    for conn in composio {
        if !conn.is_active() {
            tracing::debug!(
                toolkit = %conn.toolkit,
                connection_id = %conn.id,
                status = %conn.status,
                "[flows] flows_list_connections: skipping non-active composio connection"
            );
            continue;
        }
        let toolkit = conn.normalized_toolkit();
        let lookup_key = (
            normalize_connection_identifier(&toolkit),
            normalize_connection_identifier(&conn.id),
        );
        let platform_user_id = identity_lookup
            .get(&lookup_key)
            .and_then(|identity| identity.user_id.clone());
        tracing::debug!(
            toolkit = %toolkit,
            connection_id = %conn.id,
            has_platform_user_id = platform_user_id.is_some(),
            "[flows] flows_list_connections: resolved platform_user_id for composio connection"
        );
        out.push(FlowConnection {
            // Exactly the shape `tinyflows::caps::composio_connection_id` parses.
            connection_ref: format!("composio:{}:{}", toolkit, conn.id),
            kind: "composio".to_string(),
            display: composio_connection_display(&toolkit, &conn),
            toolkit: Some(toolkit),
            scheme: None,
            platform_user_id,
        });
    }
    for cred in http {
        out.push(FlowConnection {
            // Exactly the shape `tinyflows::caps::http_cred_name` parses.
            connection_ref: format!("http_cred:{}", cred.name),
            kind: "http".to_string(),
            display: http_credential_display(&cred),
            toolkit: None,
            scheme: Some(cred.scheme),
            platform_user_id: None,
        });
    }
    out
}

/// Human-readable picker label for a Composio connected account, e.g.
/// `"Gmail · user@example.com"`. Prefers email, then workspace/team, then
/// handle; falls back to the title-cased toolkit alone when no identity is
/// cached. The identity fields are display metadata (already surfaced by
/// `composio_list_connections`), never secret material.
fn composio_connection_display(
    toolkit: &str,
    conn: &crate::openhuman::integrations::composio::ComposioConnection,
) -> String {
    let title = title_case_toolkit(toolkit);
    let identity = conn
        .account_email
        .as_deref()
        .or(conn.workspace.as_deref())
        .or(conn.username.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match identity {
        Some(id) => format!("{title} · {id}"),
        None => title,
    }
}

/// Human-readable picker label for a named HTTP credential, e.g.
/// `"stripe (bearer)"`. Only the (non-secret) name + scheme — never the value.
fn http_credential_display(
    cred: &crate::openhuman::security::credentials::HttpCredentialSummary,
) -> String {
    format!("{} ({})", cred.name, cred.scheme)
}

/// Title-case a toolkit slug for display: `"gmail"` → `"Gmail"`,
/// `"google_calendar"` → `"Google Calendar"`. Best-effort cosmetic only.
fn title_case_toolkit(toolkit: &str) -> String {
    let trimmed = toolkit.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed
        .split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Publishes a [`DomainEvent::FlowChanged`](crate::core::event_bus::DomainEvent::FlowChanged)
/// so an open Workflows list/canvas refetches (bridged to a `flow:changed`
/// socket event) — the observability half of audit F6. Best-effort broadcast;
/// `actor` is a coarse hint (`"system"` for RPC-driven changes today).
fn publish_flow_changed(flow_id: &str, kind: &str, actor: &str) {
    tracing::debug!(target: "flows", %flow_id, kind, actor, "[flows] publishing FlowChanged");
    crate::core::event_bus::publish_global(crate::core::event_bus::DomainEvent::FlowChanged {
        flow_id: flow_id.to_string(),
        kind: kind.to_string(),
        actor: actor.to_string(),
    });
    // Re-advertise the workflow set to the medulla backend. This is the single
    // funnel every store mutation passes through (create / duplicate / update /
    // delete / enable), and the backend replaces a socket's whole entry on each
    // registration — so re-sending here is what keeps a remote orchestrator from
    // reasoning about a set that no longer exists. A no-op (one debug log, no
    // task spawned) when no bridge is installed, which is every build that is
    // not talking to a backend, and every test.
    crate::openhuman::platform::socket::medulla::workflows::emit_register_workflows();
}

/// Maps a store-level [`FlowUpdateError`](store::FlowUpdateError) to the RPC
/// error string. A concurrency conflict is encoded as a JSON object the UI can
/// parse (`{ code: "version_conflict", message, current }`) so it can offer a
/// reload/diff instead of silently clobbering; other variants are plain text.
fn map_flow_update_error(e: store::FlowUpdateError) -> String {
    match e {
        store::FlowUpdateError::NotFound => "flow not found".to_string(),
        store::FlowUpdateError::Conflict(current) => serde_json::to_string(&json!({
            "code": "version_conflict",
            "message": "This flow changed since you loaded it. Reload to see the latest \
                        version, then reapply your change.",
            "current": *current,
        }))
        .unwrap_or_else(|_| "version_conflict".to_string()),
        store::FlowUpdateError::Store(err) => err.to_string(),
    }
}

/// Updates a flow's name, graph, and/or `require_approval` toggle.
/// Re-validates the graph (whether newly supplied or the existing one)
/// before persisting, same as `flows_create`.
///
/// When the caller supplies a new `graph_json` and the flow is (still)
/// enabled, re-binds the automatic-dispatch trigger if the trigger
/// kind/config actually changed (e.g. a new schedule cron expression) —
/// otherwise the stale binding from the old graph would keep firing on the
/// old cadence, or a newly-added schedule would never get bound at all.
/// Skipped entirely for a name/`require_approval`-only update (no
/// `graph_json` supplied), since the trigger definitely didn't change.
///
/// **B29 Rule 1 analogue for saves** (save/enable safety — same issue
/// `flows_create` guards at creation time, see its doc): `flows_create`
/// refuses to persist an automatic-trigger graph (`schedule` / `app_event` /
/// `webhook`, see [`trigger_is_automatic`]) as `enabled`, but that guard only
/// runs once, at creation. Without an equivalent here, a flow created
/// `enabled: true` with a manual/no-op trigger could later have an
/// automatic-trigger graph saved onto it — via the `save_workflow` agent
/// tool, the canvas Save button, a proposal apply, or any other
/// `flows_update` caller — and go LIVE immediately with no user review
/// (confirmed live: a flow started firing on an unreviewed 8am schedule).
/// So: when the *new* graph's trigger is automatic and the *previous*
/// graph's trigger was NOT automatic (a manual/none → automatic
/// transition), this forces the persisted `enabled` back to `false` in the
/// same store write — the user must explicitly re-arm via
/// `flows_set_enabled` after reviewing the new trigger. An automatic →
/// automatic re-edit (e.g. tweaking a cron expression) is left alone — the
/// user already opted in once, and re-disarming on every edit would just be
/// friction.
///
/// The override is applied **unconditionally** on a manual/none → automatic
/// transition — it does *not* gate on whether the flow *looked* enabled in
/// the `existing` read above. That read is a snapshot taken before
/// `store::update_flow_graph`'s own guarded UPDATE re-reads the row; a
/// concurrent `flows_set_enabled(id, true)` landing in the gap would leave
/// this snapshot stale while the row is actually enabled by the time the
/// guarded UPDATE runs — and since `set_enabled` bumps `updated_at` too,
/// such a race wouldn't even trip the optimistic-concurrency conflict, it
/// would just silently persist the automatic graph as enabled (the exact
/// bug this rule exists to close). Gating on the stale `existing.enabled`
/// re-opens that race; forcing the override on every transition, enabled-or-
/// not, is exactly as safe as Rule 1's at-create version — a transition on
/// an already-disabled flow is just a no-op write of `enabled=false` over
/// `enabled=false`.
pub async fn flows_update(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph_json: Option<Value>,
    require_approval: Option<bool>,
    expected_version: Option<String>,
) -> Result<RpcOutcome<Flow>, String> {
    flows_update_inner(
        config,
        id,
        name,
        graph_json,
        require_approval,
        expected_version,
        false,
    )
    .await
}

/// Update a flow while atomically disarming any automatic-trigger graph.
///
/// Remote authoring surfaces use this variant so revising a schedule,
/// app-event, or webhook flow never preserves a prior local opt-in to run the
/// old graph. The same guarded store write persists the graph and
/// `enabled=false`, so no trigger can observe the revised graph armed between
/// two writes.
pub(crate) async fn flows_update_disarming_automatic(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph_json: Option<Value>,
    require_approval: Option<bool>,
    expected_version: Option<String>,
) -> Result<RpcOutcome<Flow>, String> {
    flows_update_inner(
        config,
        id,
        name,
        graph_json,
        require_approval,
        expected_version,
        true,
    )
    .await
}

async fn flows_update_inner(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph_json: Option<Value>,
    require_approval: Option<bool>,
    expected_version: Option<String>,
    disarm_automatic: bool,
) -> Result<RpcOutcome<Flow>, String> {
    let existing = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{id}' not found"))?;

    let new_name = name.unwrap_or_else(|| existing.name.clone());
    let new_require_approval = require_approval.unwrap_or(existing.require_approval);
    let graph_changed = graph_json.is_some();
    let graph = match graph_json {
        Some(raw) => {
            let graph = validate_and_migrate_graph(raw)?;
            ensure_config_aware_engine_compatible(config, &graph)?;
            graph
        }
        None => {
            tinyflows::validate::validate(&existing.graph).map_err(|e| e.to_string())?;
            existing.graph.clone()
        }
    };
    // B29 Rule 1 analogue: disarm every manual/none → automatic trigger
    // transition, unconditionally. `now_auto` is safe to compute here (it
    // only depends on `graph`, THIS call's own incoming graph — never
    // stale). The "was it automatic before" half of the transition,
    // however, is NOT decided here: R-m2 found that gating on the
    // ops-level `existing.graph` read let a concurrent write race this
    // call and slip an automatic-trigger graph through with `enabled: true`
    // — `existing` can be arbitrarily stale by the time
    // `store::update_flow_graph` actually performs its guarded write. That
    // decision now lives inside `update_flow_graph`, computed against the
    // row it just re-read there (see its doc comment).
    let now_auto = trigger_is_automatic(&graph);
    let forced_automatic_disarm = disarm_automatic && now_auto;
    tracing::debug!(
        target: "flows",
        flow_id = %id,
        now_auto,
        currently_enabled = existing.enabled,
        forced_automatic_disarm,
        "[flows] flows_update: auto-trigger disarm decision inputs (transition itself decided \
         store-side against a fresh read, see update_flow_graph)"
    );

    // Rule 2 analogue (compound-bypass closure): re-apply the same outbound
    // side-effect check `flows_create` applies on save — via the shared
    // [`enforce_side_effect_approval`] helper — so an update that *adds* a
    // tool_call/http_request/code node to a previously read-only graph can
    // never persist `require_approval: false` just because the update path
    // trusted the caller's toggle unconditionally.
    let (effective_require_approval, side_effect_forced) =
        enforce_side_effect_approval(&graph, new_require_approval);
    if side_effect_forced {
        tracing::info!(
            target: "flows",
            flow_id = %id,
            "[flows] flows_update: forcing require_approval=true — graph contains outbound \
             side-effect node(s) (tool_call / http_request / code)"
        );
    }

    tracing::debug!(
        target: "flows",
        flow_id = %id,
        has_expected = expected_version.is_some(),
        require_approval = effective_require_approval,
        side_effect_forced,
        "[flows] flows_update: persisting changes"
    );
    // The auto-disarm decision (both the unconditional manual→automatic
    // transition and `disarm_automatic`'s forced-remote-authoring variant)
    // is made INSIDE `update_flow_graph`, against the row it re-reads right
    // before its guarded UPDATE — see R-m2 above and that function's doc
    // comment. `enabled_override: None` here means "no explicit force from
    // this caller"; the disarm, if any, still applies on top of that.
    let updated = store::update_flow_graph(
        config,
        id,
        new_name,
        graph,
        effective_require_approval,
        None,
        disarm_automatic,
        expected_version.as_deref(),
    )
    .map_err(map_flow_update_error)?;

    // Best-effort, POST-write: did the flow actually transition from
    // enabled to disabled as part of this update? Derived from the real
    // before/after state (`existing.enabled` vs `updated.enabled`) rather
    // than re-predicting the decision — the decision itself already
    // happened store-side against a fresh read, so this is purely for the
    // info log / result message wording below and can't desync from what
    // was actually persisted.
    let should_disarm = now_auto && existing.enabled && !updated.enabled;
    if should_disarm {
        tracing::info!(
            target: "flows",
            flow_id = %id,
            "[flows] flows_update: auto-disabled automatic-trigger graph pending explicit re-arm"
        );
    }

    if graph_changed && updated.enabled {
        let trigger_unchanged = bus::extract_trigger_kind(&existing)
            == bus::extract_trigger_kind(&updated)
            && bus::extract_trigger_config(&existing) == bus::extract_trigger_config(&updated);
        if !trigger_unchanged {
            tracing::debug!(target: "flows", flow_id = %id, "[flows] flows_update: trigger changed on an enabled flow — rebinding automatic-dispatch trigger");
            unbind_trigger(config, &existing);
            bind_trigger(config, &updated);
        }
    }

    publish_flow_changed(id, "updated", "system");
    let mut logs = vec![format!("flow updated: {id}")];
    if should_disarm {
        let reason = if forced_automatic_disarm {
            "Flow was auto-disabled because this authoring surface revised an automatic trigger \
             (schedule / app_event / webhook). Enable it explicitly (flows_set_enabled) once \
             you've reviewed the revision."
        } else {
            "Flow was auto-disabled because its trigger changed from manual to automatic \
             (schedule / app_event / webhook). Enable it explicitly (flows_set_enabled) once \
             you've reviewed the new trigger."
        };
        logs.push(reason.to_string());
    }
    if side_effect_forced {
        logs.push(
            "require_approval forced to true because the graph contains outbound side-effect \
             nodes (tool_call / http_request / code)."
                .to_string(),
        );
    }
    Ok(RpcOutcome::new(updated, logs))
}

/// Lists a flow's revision history (prior graph snapshots), newest first,
/// capped at `limit` (audit F6). The safety rail that makes rollback possible.
pub fn flows_get_history(
    config: &Config,
    id: &str,
    limit: usize,
) -> Result<RpcOutcome<Vec<crate::openhuman::flows::FlowRevision>>, String> {
    let revisions = store::list_revisions(config, id, limit).map_err(|e| e.to_string())?;
    let count = revisions.len();
    Ok(RpcOutcome::single_log(
        revisions,
        format!("flow history: {id} ({count} revisions)"),
    ))
}

/// Rolls a flow back to a prior revision by restoring that revision's graph
/// through the normal update path — which itself snapshots the current graph as
/// a new revision, so a rollback is itself undoable. Honours optimistic
/// concurrency via `expected_version`.
pub async fn flows_rollback(
    config: &Config,
    id: &str,
    revision_id: &str,
    expected_version: Option<String>,
) -> Result<RpcOutcome<Flow>, String> {
    let rev = store::revision_by_id(config, id, revision_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("revision '{revision_id}' not found for flow '{id}'"))?;

    tracing::debug!(target: "flows", flow_id = %id, %revision_id, "[flows] flows_rollback: restoring prior revision");
    flows_update(
        config,
        id,
        Some(rev.name),
        Some(rev.graph),
        Some(rev.require_approval),
        expected_version,
    )
    .await
}

/// Deletes a flow by id.
///
/// Unbinds the flow's automatic-dispatch trigger (e.g. the schedule-trigger
/// cron job) *before* removing the flow definition. `flow_runs` cascades on
/// delete via a same-database `FOREIGN KEY ... ON DELETE CASCADE`, but a
/// bound cron job lives in the entirely separate `cron.db` — it does NOT
/// cascade — so skipping this would orphan the cron job, leaving it pointing
/// at a now-nonexistent `flow_id` forever. Best-effort: a lookup failure
/// (flow already gone, store error) is logged and does not block the delete
/// itself — `store::remove_flow` below still errors clearly if `id` doesn't
/// exist.
pub async fn flows_delete(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    flows_delete_impl(config, id, None).await
}

/// Backs [`flows_delete`]. `memory_client_override`, when `Some`, is used in
/// place of the process-global memory client for the namespace-clear step
/// below — mirrors `bus::FlowRunDigestSubscriber`'s `with_memory` seam.
///
/// The process-global client (`memory::global`) is a single shared `OnceLock`
/// that any test in the binary may rebind to its own tempdir workspace, so a
/// test asserting this clear step deterministically must not depend on it —
/// injecting a directly-constructed [`MemoryClientRef`] lets the test seed
/// and read back through the SAME instance `flows_delete` itself writes to,
/// with no race against the global.
async fn flows_delete_impl(
    config: &Config,
    id: &str,
    memory_client_override: Option<MemoryClientRef>,
) -> Result<RpcOutcome<Value>, String> {
    match store::get_flow(config, id) {
        Ok(Some(flow)) => unbind_trigger(config, &flow),
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(target: "flows", flow_id = %id, error = %e, "[flows] flows_delete: failed to load flow before unbind — proceeding with delete anyway");
        }
    }

    store::remove_flow(config, id).map_err(|e| e.to_string())?;
    tracing::debug!(target: "flows", flow_id = %id, "[flows] flows_delete: removed");

    // Best-effort: purge the flow's pre-authorized tool trust with its row —
    // a deleted flow must not leave dangling `flow_tool_trust` grants that a
    // future flow reusing the same id (or a stale run) could inherit. Never
    // fails the delete: the flow row is already gone regardless.
    if let Some(gate) = crate::openhuman::security::approval::ApprovalGate::try_global() {
        match gate.delete_flow_trust(id, None) {
            Ok(removed) if removed > 0 => {
                tracing::info!(target: "flows", flow_id = %id, removed, "[flows] flows_delete: purged flow tool trust grants");
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(target: "flows", flow_id = %id, error = %e, "[flows] flows_delete: failed to purge flow tool trust");
            }
        }
    }

    // Best-effort: clear this flow's private memory namespace along with its
    // row — a deleted flow must not leave stray `flow_memory_remember`
    // entries or run digests behind. Never fails the delete itself: the flow
    // row is already gone by this point regardless of what happens here.
    let memory_namespace = flow_namespace(id);
    let client_result = match memory_client_override {
        Some(client) => Ok(client),
        None => crate::openhuman::memory::ops::helpers::active_memory_client().await,
    };
    match client_result {
        Ok(client) => {
            if let Err(e) = client.clear_namespace(&memory_namespace).await {
                tracing::warn!(target: "flows", flow_id = %id, namespace = %memory_namespace, error = %e, "[flows] flows_delete: failed to clear flow memory namespace");
            }
        }
        Err(e) => {
            tracing::warn!(target: "flows", flow_id = %id, namespace = %memory_namespace, error = %e, "[flows] flows_delete: memory client unavailable — could not clear flow memory namespace");
        }
    }

    publish_flow_changed(id, "deleted", "system");
    Ok(RpcOutcome::new(
        json!({ "id": id, "removed": true }),
        vec![format!("flow removed: {id}")],
    ))
}

/// Enables or disables a flow. Enable/disable now (B2) binds/tears down the
/// flow's automatic trigger:
/// - `schedule` — registers/removes the backing `cron` job
///   (`cron::add_flow_schedule_job` / `cron::remove_job`) so
///   `flows::bus::FlowTriggerSubscriber` gets a `FlowScheduleTick` on the
///   configured cadence.
/// - `app_event` — no enable-time side effect needed: the subscriber matches
///   every `ComposioTriggerReceived` against `store::list_enabled_flows` at
///   dispatch time, so the `enabled` flag alone gates it.
/// - `webhook` — **not implemented** in B2 (best-effort deviation, see
///   `bind_trigger`'s webhook arm below and
///   `my_docs/ohxtf/b2-triggers-trust/01-triggers-and-trust.md` §1); logged,
///   not silently skipped.
/// - `manual` / anything else — no binding needed; `flows_run` always works.
///
/// `flows_run` still runs a disabled flow on demand (mirrors
/// `cron::rpc::cron_run`'s "Run Now always works" behavior) — `enabled` only
/// gates *automatic* trigger-driven dispatch.
pub async fn flows_set_enabled(
    config: &Config,
    id: &str,
    enabled: bool,
) -> Result<RpcOutcome<Flow>, String> {
    let flow = store::set_enabled(config, id, enabled).map_err(|e| e.to_string())?;

    if enabled {
        bind_trigger(config, &flow);
    } else {
        unbind_trigger(config, &flow);
    }

    let mut logs = vec![format!("flow {id} enabled={enabled}")];
    // When enabling, loudly surface any unfired-trigger-kind warning in the
    // result (a structured `warning:`-prefixed log), not just a silent tracing
    // line — so an enable of a flow that will never fire itself (webhook,
    // chat_message, form, …) is impossible to miss at the call site.
    if enabled {
        for warning in graph_trigger_warnings(&flow.graph) {
            tracing::warn!(
                target: "flows",
                flow_id = %id,
                warning = %warning,
                "[flows] flows_set_enabled: enabling a flow whose trigger kind does not fire yet"
            );
            logs.push(format!("warning: {warning}"));
        }
    }

    publish_flow_changed(id, "enabled_changed", "system");
    Ok(RpcOutcome::new(flow, logs))
}

/// Registers the automatic-dispatch side effect for `flow`'s trigger kind, if
/// any. Best-effort: a binding failure is logged and does not fail the
/// `flows_set_enabled` call — the flow is still saved as enabled, it just
/// won't fire automatically until the underlying issue (invalid schedule,
/// cron store error, …) is fixed.
fn bind_trigger(config: &Config, flow: &Flow) {
    match bus::extract_trigger_kind(flow) {
        Some(TriggerKind::Schedule) => bind_schedule_trigger(config, flow),
        Some(TriggerKind::Webhook) => log_webhook_trigger_deferred(flow, true),
        _ => {
            // `app_event` needs no enable-time binding (matched at dispatch
            // time against `list_enabled_flows`); `manual`/`form`/others have
            // no automatic-dispatch concept at all.
        }
    }
}

/// Tears down the automatic-dispatch side effect for `flow`'s trigger kind,
/// mirroring [`bind_trigger`]. Best-effort, same rationale.
fn unbind_trigger(config: &Config, flow: &Flow) {
    match bus::extract_trigger_kind(flow) {
        Some(TriggerKind::Schedule) => unbind_schedule_trigger(config, &flow.id),
        Some(TriggerKind::Webhook) => log_webhook_trigger_deferred(flow, false),
        _ => {}
    }
}

/// Registers (or refreshes) the `cron` job backing a `schedule`-trigger
/// flow. Idempotent — re-uses an existing binding via
/// `cron::find_flow_schedule_job` rather than creating a duplicate, so this
/// is safe to call both from `flows_set_enabled` and from boot
/// reconciliation ([`reconcile_schedule_triggers_on_boot`]).
fn bind_schedule_trigger(config: &Config, flow: &Flow) {
    let Some(trigger_config) = bus::extract_trigger_config(flow) else {
        tracing::warn!(target: "flows", flow_id = %flow.id, "[flows] schedule trigger: flow has no single trigger node — cannot bind cron job");
        return;
    };
    let Some(schedule_raw) = trigger_config.get("schedule").cloned() else {
        tracing::warn!(target: "flows", flow_id = %flow.id, "[flows] schedule trigger config is missing `schedule` — cannot bind cron job");
        return;
    };
    let schedule: crate::openhuman::cron::Schedule = match serde_json::from_value(schedule_raw) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(target: "flows", flow_id = %flow.id, error = %e, "[flows] invalid schedule trigger config — cannot bind cron job");
            return;
        }
    };

    match crate::openhuman::cron::find_flow_schedule_job(config, &flow.id) {
        Ok(Some(existing)) => {
            let patch = crate::openhuman::cron::CronJobPatch {
                enabled: Some(true),
                schedule: Some(schedule),
                ..Default::default()
            };
            if let Err(e) = crate::openhuman::cron::update_job(config, &existing.id, patch) {
                tracing::warn!(target: "flows", flow_id = %flow.id, cron_job_id = %existing.id, error = %e, "[flows] failed to refresh existing schedule-trigger cron job");
            } else {
                tracing::debug!(target: "flows", flow_id = %flow.id, cron_job_id = %existing.id, "[flows] refreshed existing schedule-trigger cron job");
            }
        }
        Ok(None) => match crate::openhuman::cron::add_flow_schedule_job(config, &flow.id, schedule)
        {
            Ok(job) => {
                tracing::info!(target: "flows", flow_id = %flow.id, cron_job_id = %job.id, "[flows] registered schedule-trigger cron job")
            }
            Err(e) => {
                tracing::warn!(target: "flows", flow_id = %flow.id, error = %e, "[flows] failed to register schedule-trigger cron job")
            }
        },
        Err(e) => {
            tracing::warn!(target: "flows", flow_id = %flow.id, error = %e, "[flows] failed to look up existing schedule-trigger cron job");
        }
    }
}

/// Removes the `cron` job backing a `schedule`-trigger flow, if one exists.
fn unbind_schedule_trigger(config: &Config, flow_id: &str) {
    match crate::openhuman::cron::find_flow_schedule_job(config, flow_id) {
        Ok(Some(job)) => {
            if let Err(e) = crate::openhuman::cron::remove_job(config, &job.id) {
                tracing::warn!(target: "flows", %flow_id, cron_job_id = %job.id, error = %e, "[flows] failed to remove schedule-trigger cron job");
            } else {
                tracing::debug!(target: "flows", %flow_id, cron_job_id = %job.id, "[flows] removed schedule-trigger cron job");
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(target: "flows", %flow_id, error = %e, "[flows] failed to look up schedule-trigger cron job for teardown");
        }
    }
}

/// Webhook trigger binding is a documented B2 stub (best-effort deviation):
/// registering a real inbound route requires provisioning a backend tunnel
/// (`webhooks::ops::create_tunnel`, a network call to the signed-in backend
/// account) plus a UI surface to show the resulting URL to the user — both
/// are B3 territory. Rather than silently doing nothing, this logs a clear,
/// actionable warning every time a `webhook`-trigger flow is enabled/disabled
/// so the gap is diagnosable. `flows::bus::FlowTriggerSubscriber` logs the
/// matching deferral on the inbound side (`WebhookIncomingRequest`).
fn log_webhook_trigger_deferred(flow: &Flow, enabled: bool) {
    tracing::warn!(
        target: "flows",
        flow_id = %flow.id,
        enabled,
        "[flows] webhook trigger binding is not implemented in B2 (requires backend tunnel \
         provisioning + a UI surface for the resulting URL) — this flow will not fire \
         automatically from an inbound webhook until that lands"
    );
}

/// Boot-time reconciliation: registers the `cron` job for every enabled,
/// `schedule`-trigger flow. Idempotent (delegates to [`bind_schedule_trigger`],
/// which re-uses an existing binding) — mirrors
/// `cron::seed::seed_proactive_agents_on_boot`'s "ensure jobs exist for
/// already-onboarded users upgrading from an older build" pattern, so a
/// flow enabled on a build that predates this cron binding (or whose binding
/// was lost some other way) gets its schedule re-registered on the next
/// boot without the user having to toggle it off and on.
pub async fn reconcile_schedule_triggers_on_boot(config: &Config) -> Result<(), String> {
    let (flows, skipped) = store::list_enabled_flows(config).map_err(|e| e.to_string())?;
    if skipped > 0 {
        // R-M4: a corrupt/unmigratable row must not abort boot reconciliation
        // for every other enabled flow — skipped rows are logged loudly
        // (never their content) so the gap is diagnosable.
        tracing::warn!(target: "flows", skipped, "[flows] reconcile_schedule_triggers_on_boot: skipped corrupt/unmigratable flow rows");
    }
    let mut reconciled = 0usize;
    for flow in &flows {
        if matches!(bus::extract_trigger_kind(flow), Some(TriggerKind::Schedule)) {
            bind_schedule_trigger(config, flow);
            reconciled += 1;
        }
    }
    tracing::debug!(target: "flows", scanned = flows.len(), reconciled, skipped, "[flows] boot reconciliation of schedule-trigger cron jobs complete");
    Ok(())
}

/// Reads a settled run's durable [`tinyflows::engine::GraphObservation`]
/// slice back out of the per-run journal (keyed by the tinyagents-minted
/// `graph_run_id`) and exports it to Langfuse as one trace. Best-effort by
/// construction: any journal read failure is logged and swallowed, and the
/// exporter itself never fails the run. Skips the journal read entirely when
/// `observability.share_usage_data` is off.
async fn export_run_to_langfuse(
    config: &Config,
    flow_name: &str,
    flow_id: &str,
    thread_id: &str,
    status: &str,
    trigger: FlowRunTrigger,
    journal: &tinyflows::engine::InMemoryGraphEventJournal,
    graph_run_id: &str,
) {
    if !config.observability.share_usage_data {
        tracing::debug!(
            target: "flows",
            flow_id = %flow_id,
            "[flows] langfuse export skipped: observability.share_usage_data is off"
        );
        return;
    }
    use tinyflows::engine::GraphEventJournal as _;
    let observations = match journal.read_from(graph_run_id, 0).await {
        Ok(observations) => observations,
        Err(e) => {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                graph_run_id = %graph_run_id,
                error = %e,
                "[flows] langfuse export skipped: could not read run journal"
            );
            return;
        }
    };
    tracing::debug!(
        target: "flows",
        flow_id = %flow_id,
        %thread_id,
        graph_run_id = %graph_run_id,
        observation_count = observations.len(),
        "[flows] exporting flow run trace to Langfuse"
    );
    crate::openhuman::flows::tinyflows::langfuse_export::export_flow_run_trace(
        config,
        flow_name,
        flow_id,
        thread_id,
        status,
        trigger,
        &observations,
    )
    .await;
}

/// Runs a saved flow end-to-end: compile → build capabilities → durable
/// checkpointed run → record the outcome onto the flow's summary fields and
/// into a `flow_runs` history row.
///
/// Uses `tinyflows::engine::run_with_checkpointer` (not the simpler `run`) so
/// a run that pauses at a human-in-the-loop approval gate is durably
/// checkpointed and can survive a process restart (resumed later via
/// [`flows_resume`]; see
/// `my_docs/ohxtf/b1-engine-seam-domain/05-checkpointer-and-state.md`).
///
/// The whole run is scoped under `AgentTurnOrigin::TrustedAutomation {
/// Workflow }` (issue B2) regardless of caller (an interactive RPC "Run" or
/// an automatic trigger dispatch from `flows::bus::FlowTriggerSubscriber`):
/// the trust argument is about the *flow* (a saved, validated graph whose
/// `tool_call`/`http_request` nodes are pre-declared), not about who started
/// the run — see `TrustedAutomationSource::Workflow`'s doc and
/// `my_docs/ohxtf/b2-triggers-trust/01-triggers-and-trust.md` §3.
/// `input` is the free-form trigger payload (reachable as `=run.trigger.…`);
/// `inputs` supplies values for the flow's *declared* workflow inputs by name
/// (reachable as `=inputs.<name>`). The two are separate channels — see
/// [`tinyflows::engine::RunInput`]. A declared-input problem (missing required
/// value, wrong type, undeclared key) is rejected before any run row exists.
pub async fn flows_run(
    config: &Config,
    flow_id: &str,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
) -> Result<RpcOutcome<Value>, String> {
    // Prep synchronously (validate + compile-check + resolve inputs + mint the
    // run id), insert the initial `running` row, and announce it, then hand off
    // to the shared run body. Both the synchronous "Run" RPC path (this fn) and
    // the detached agent path ([`flows_run_detached`]) reuse `run_flow_body` so
    // a single [`RunRowFinalizer`] guards the row on every exit — bug B42.
    let prepared = prepare_flow_run(config, flow_id, &inputs)?;
    let thread_id = prepared.thread_id.clone();
    let no_actionable_nodes = prepared.no_actionable_nodes;
    let resolved_inputs = prepared.inputs;

    // Register BEFORE the row exists, so a `flows_cancel_run` can never observe
    // a `running` row that no live run owns (see [`run_flow_body`]'s doc).
    let (cancel_token, run_guard) = run_registry::register(&thread_id);
    start_flow_run_row(config, &thread_id, flow_id);
    publish_flow_run_started(flow_id, &thread_id);

    run_flow_body(
        Arc::new(config.clone()),
        prepared.flow,
        flow_id.to_string(),
        thread_id,
        input,
        resolved_inputs,
        trigger,
        no_actionable_nodes,
        cancel_token,
        run_guard,
    )
    .await
}

/// Agent-initiated `run_flow` entry point (bug B41). Unlike [`flows_run`], this
/// does NOT block on the engine: the tinyagents harness caps a single tool call
/// at 120s, but any flow whose first real node is a live-research agent node
/// (`web_search` + `web_fetch` + `parallel_research`) inherently runs longer
/// than that, so a blocking `run_flow` tool call could *never* succeed for a
/// realistic flow — it died at exactly 120s, orphaning the run row (bug B42).
///
/// Instead this validates + compile-checks the flow synchronously (so a broken
/// flow still returns an immediate, actionable error to the agent), inserts the
/// `running` row, publishes `FlowRunStarted`, then spawns [`run_flow_body`] on a
/// background task and returns `{ run_id, status: "running", detached: true }`
/// in well under 120s. The copilot already polls `get_flow_run(run_id)` (seen
/// in live traces), so it observes the run settle to a terminal state on its
/// own cadence. Also exposed over RPC as `flows.run_detached` (see
/// `schemas::handle_run_detached`) — the UI "Run" control (canvas + Workflows
/// list) calls that entry point directly, and the trigger bus
/// (`flows::bus::spawn_run`) fires runs the same fire-and-forget way. Combined
/// with B42's finalizer + boot sweep, a detached run ALWAYS settles to a
/// terminal row even if the process dies mid-run.
///
/// `input` / `inputs` mean exactly what they do on [`flows_run`]: the trigger
/// payload and the flow's declared inputs. Both are validated synchronously, so
/// the agent still gets an immediate, actionable error for a bad call.
pub async fn flows_run_detached(
    config: &Config,
    flow_id: &str,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
) -> Result<RpcOutcome<Value>, String> {
    let prepared = prepare_flow_run(config, flow_id, &inputs)?;
    let thread_id = prepared.thread_id.clone();
    let no_actionable_nodes = prepared.no_actionable_nodes;
    let resolved_inputs = prepared.inputs;

    // Register BEFORE the `run_id` becomes observable to the agent. The spawned
    // task below may not be polled for some time, so registering inside it
    // would leave a window where a `flows_cancel_run` on the returned `run_id`
    // sees no in-flight run, settles the row `cancelled` + drops the
    // checkpoint, and the background run then executes the flow's real side
    // effects anyway and overwrites that terminal status. Registering here
    // means such a cancel always takes the signalled branch and this run's own
    // cancellation arm unwinds it. See [`run_flow_body`]'s doc.
    let (cancel_token, run_guard) = run_registry::register(&thread_id);
    start_flow_run_row(config, &thread_id, flow_id);
    publish_flow_run_started(flow_id, &thread_id);

    tracing::info!(
        target: "flows",
        flow_id = %flow_id,
        run_id = %thread_id,
        "[flows] flows_run_detached: registered + spawning background run; returning run_id immediately"
    );

    let config_arc = Arc::new(config.clone());
    let flow = prepared.flow;
    let flow_id_owned = flow_id.to_string();
    let body_thread_id = thread_id.clone();
    tokio::spawn(async move {
        if let Err(e) = run_flow_body(
            config_arc,
            flow,
            flow_id_owned,
            body_thread_id,
            input,
            resolved_inputs,
            trigger,
            no_actionable_nodes,
            cancel_token,
            run_guard,
        )
        .await
        {
            // The row is already reconciled by the body's terminal write /
            // finalizer — this only logs that the detached run ended in error.
            tracing::warn!(target: "flows", error = %e, "[flows] flows_run_detached: background run ended with error (row already reconciled)");
        }
    });

    let result = json!({
        "run_id": thread_id,
        "flow_id": flow_id,
        "status": "running",
        "detached": true,
    });
    Ok(RpcOutcome::single_log(
        result,
        format!("flow run started (detached): {thread_id}"),
    ))
}

/// A validated, ready-to-execute flow run: the loaded [`Flow`], the freshly
/// minted `thread_id` (== run id / checkpointer key), and whether the graph has
/// no actionable nodes. Produced by [`prepare_flow_run`] and consumed by both
/// `flows_run` entry points.
struct PreparedFlowRun {
    flow: Flow,
    thread_id: String,
    no_actionable_nodes: bool,
    /// The flow's declared inputs resolved against the caller's values —
    /// defaults applied, one entry per declaration.
    inputs: serde_json::Map<String, Value>,
}

/// Synchronous prep shared by [`flows_run`] and [`flows_run_detached`]: loads
/// the flow, warns on an actionless graph, rejects an engine-incompatible
/// topology, compile-checks the graph so a broken flow fails fast *before* any
/// `running` row is inserted, resolves the caller's declared-input values, and
/// mints the run's `thread_id`. Returns an error (never a wedged row) if the
/// flow can't run at all.
///
/// Input resolution happens *here* rather than being left to the engine so a
/// bad call never creates a `running` row, a thread id, or a registry entry.
/// The engine re-resolves the same values (it is the authority on its own
/// contract); doing it twice is cheap and keeps this host from having to trust
/// its own copy of the rules.
fn prepare_flow_run(
    config: &Config,
    flow_id: &str,
    inputs: &serde_json::Map<String, Value>,
) -> Result<PreparedFlowRun, String> {
    let flow = store::get_flow(config, flow_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{flow_id}' not found"))?;

    // Live finding: a graph with no actionable nodes (only a `trigger`, or a
    // `trigger` plus nodes with no edges wiring them up) compiles and "runs"
    // cleanly but does nothing — and previously reported
    // `status="completed" pending_approvals=0` indistinguishably from a real
    // run, reading as "triggered but nothing happened" was actually a
    // success. Surface it loudly instead of letting it pass silently: warn
    // now (independent of how the run below turns out), and attach a
    // human-readable note to the returned outcome so the UI can show
    // "nothing to run" rather than a bare "completed".
    let no_actionable_nodes = !graph_has_actionable_nodes(&flow.graph);
    if no_actionable_nodes {
        tracing::warn!(
            target: "flows",
            flow_id = %flow_id,
            "[flows] flows_run: flow has no actionable nodes — nothing to execute"
        );
    }

    // `store::get_flow` already ran the stored `graph_json` through
    // `tinyflows::migrate::migrate` before deserializing, so `flow.graph` is
    // always on the current schema here.
    //
    // Author-time validation cannot protect definitions persisted by an older
    // OpenHuman build. Re-check immediately before compilation so an upgrade
    // fails explicitly instead of silently committing incomplete merge data.
    if let Err(error) = ensure_config_aware_engine_compatible(config, &flow.graph) {
        tracing::warn!(
            target: "flows",
            flow_id = %flow_id,
            %error,
            "[flows] flows_run: rejected — unsupported engine topology"
        );
        return Err(error);
    }
    // Compile-check up front so a structurally broken graph fails the caller
    // immediately, before a `running` row exists. `run_flow_body` recompiles
    // (cheap) to actually execute.
    tinyflows::compiler::compile(&flow.graph).map_err(|e| e.to_string())?;

    // Declared inputs, before anything observable exists for this run.
    let resolved_inputs =
        tinyflows::model::resolve_inputs(&flow.graph.inputs, inputs).map_err(|e| {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                input = %e.input_name(),
                code = %e.code(),
                "[flows] flows_run: rejected — bad workflow input"
            );
            e.to_string()
        })?;

    let thread_id = format!("flow:{flow_id}:{}", uuid::Uuid::new_v4());
    tracing::debug!(
        target: "flows",
        flow_id = %flow_id,
        thread_id = %thread_id,
        require_approval = flow.require_approval,
        "[flows] flows_run: prepared checkpointed run"
    );

    Ok(PreparedFlowRun {
        flow,
        thread_id,
        no_actionable_nodes,
        inputs: resolved_inputs,
    })
}

/// Announces a freshly-started run on the global event bus so the frontend run
/// list flips to `running` immediately. Factored out of [`flows_run`] so both
/// entry points publish identically.
fn publish_flow_run_started(flow_id: &str, thread_id: &str) {
    tracing::debug!(
        target: "flows",
        flow_id = %flow_id,
        run_id = %thread_id,
        "[flows] flows_run: publishing FlowRunStarted"
    );
    crate::core::event_bus::publish_global(crate::core::event_bus::DomainEvent::FlowRunStarted {
        flow_id: flow_id.to_string(),
        run_id: thread_id.to_string(),
    });
}

/// Human-readable reason stamped on a run row that the [`RunRowFinalizer`]
/// drop-guard reconciles because its run future was dropped mid-flight (harness
/// tool abort, chat turn end, runtime shutdown, panic) before any terminal
/// write landed. Surfaced verbatim in the run-details sidebar (bug B42c) so a
/// cancelled/timed-out run reads as interrupted rather than a blank spinner.
const INTERRUPTED_DROP_REASON: &str =
    "Run interrupted before completion — it was cancelled, timed out, or the app shut down mid-run.";

/// Cancellation-safe finalizer for a live `flow_runs` row (bug B42).
///
/// While a run's engine future is awaiting, dropping that future — the harness
/// 120s tool abort, a chat turn ending, tokio runtime shutdown, or a panic —
/// would otherwise leave the row wedged at `status="running"`, `error=NULL`,
/// `steps=[]` forever, which the run-details sidebar renders as a perpetual
/// blank spinner. Held across the await, this guard writes a terminal
/// `"interrupted"` status + human reason on `Drop` UNLESS it has been
/// explicitly [`disarm`](Self::disarm)ed after a real terminal write. The
/// `armed` flag is a single-task `Cell` (the guard never crosses tasks by
/// reference), so the type stays `Send` for `tokio::spawn`.
struct RunRowFinalizer {
    config: Arc<Config>,
    thread_id: String,
    flow_id: String,
    armed: std::cell::Cell<bool>,
}

impl RunRowFinalizer {
    fn new(config: Arc<Config>, thread_id: &str, flow_id: &str) -> Self {
        Self {
            config,
            thread_id: thread_id.to_string(),
            flow_id: flow_id.to_string(),
            armed: std::cell::Cell::new(true),
        }
    }

    /// Disarm the guard after a real terminal write (success/failure/cancel/
    /// pause) has already finalized the row, so `Drop` becomes a no-op.
    fn disarm(&self) {
        self.armed.set(false);
    }
}

impl Drop for RunRowFinalizer {
    fn drop(&mut self) {
        if !self.armed.get() {
            return;
        }
        tracing::warn!(
            target: "flows",
            flow_id = %self.flow_id,
            thread_id = %self.thread_id,
            "[flows] RunRowFinalizer: run future dropped before settling — reconciling orphaned 'running' row to 'interrupted'"
        );
        // Preserve whatever steps the live observer already persisted.
        let observed = current_persisted_steps(&self.config, &self.thread_id);
        finish_flow_run_row(
            &self.config,
            &self.thread_id,
            &self.flow_id,
            "interrupted",
            &observed,
            &[],
            Some(INTERRUPTED_DROP_REASON),
            None,
        );
        // Keep the flow-definition summary in step with the row, exactly as the
        // success/failure/cancel arms and the boot sweep do — otherwise the
        // runs list keeps advertising the *previous* run's `last_status` /
        // `last_run_at` for a flow whose latest run was interrupted.
        // `record_run` is synchronous, so it is safe in `Drop`.
        if let Err(e) = store::record_run(&self.config, &self.flow_id, "interrupted") {
            tracing::warn!(
                target: "flows",
                flow_id = %self.flow_id,
                thread_id = %self.thread_id,
                error = %e,
                "[flows] RunRowFinalizer: failed to update flow summary for interrupted run"
            );
        }
    }
}

/// Executes an already-prepared, already-`running`-row-inserted flow run to a
/// terminal state, finalizing the `flow_runs` row on every exit path.
///
/// Split out of [`flows_run`] (bugs B41/B42) so the synchronous and detached
/// entry points share ONE run body — and so a single [`RunRowFinalizer`]
/// reconciles the row to `"interrupted"` if this future is dropped mid-await
/// before any terminal write lands. The caller MUST have already
/// [`run_registry::register`]ed `thread_id` (handing the token + guard in
/// here), inserted the initial `running` row ([`start_flow_run_row`]) and
/// published `FlowRunStarted`.
///
/// **Registration is the caller's job on purpose.** It used to happen here, but
/// on the detached path that left a window: `flows_run_detached` returned the
/// `run_id` to the agent before the spawned task had registered, so a
/// `flows_cancel_run` landing in that gap saw `is_in_flight == false`, took the
/// "parked/stale" branch, wrote a terminal `cancelled` row and dropped the
/// checkpoint — while this body then started and executed the flow's real
/// side effects anyway, finally overwriting `cancelled` with its own terminal
/// status. Registering before the `run_id` is observable makes the cancel
/// always take the signalled branch instead. `_run_guard` is held for the whole
/// body and deregisters on any exit, including the early returns below.
async fn run_flow_body(
    config_arc: Arc<Config>,
    flow: Flow,
    flow_id: String,
    thread_id: String,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
    no_actionable_nodes: bool,
    cancel_token: tokio_util::sync::CancellationToken,
    _run_guard: run_registry::RunGuard,
) -> Result<RpcOutcome<Value>, String> {
    let config: &Config = config_arc.as_ref();
    let flow_id: &str = flow_id.as_str();

    // B42 drop-guard, armed BEFORE the first `.await` in this body (R-M5).
    //
    // The caller has already inserted the `running` row, so every await from
    // here on is a window in which dropping this future would strand that row.
    // The guard used to be constructed ~150 lines below, immediately around the
    // engine call — which left the inference-readiness preflight directly below
    // (a real network probe on a cache miss) unguarded: a client disconnect or
    // an aborted detached task during that probe dropped the future before any
    // finalizer existed, and the row stayed a perpetual `running` spinner until
    // the NEXT process boot sweep (the in-process one had already run). Arming
    // it here covers the whole awaiting region; every settled path below still
    // disarms it after its own terminal write.
    let finalizer = RunRowFinalizer::new(config_arc.clone(), &thread_id, flow_id);

    // B45 run-time preflight (design correction — see the "Inference-readiness
    // check" module doc above): an `agent` node needs a working LLM provider
    // to run at all, but that is no longer enforced as an author-time gate —
    // `propose_workflow`/`edit_workflow`/`save_workflow` always succeed now,
    // so a graph can reach here whose agent node(s) cannot currently complete.
    // Catch that HERE, before the tinyflows engine (and any upstream
    // fetch/prep nodes) does real work for nothing, and finalize the run row
    // as `failed` with a clear, actionable message instead of the opaque,
    // several-layers-deep "capability error: graph error: capability error:
    // model error: ... API key not configured for provider" a mid-run failure
    // surfaces as. Reuses `validate_inference_readiness` — backed by the same
    // cached evaluation `build_builder_proposal`'s advisory `inference_status`
    // warns on — so a run right after a proposal/edit reads the cached
    // negative (`INFERENCE_PROBE_CACHE`) instead of re-probing the network.
    // Returns an empty `Vec` (no-op here) for a tool_call-only graph, and is
    // never consulted by `dry_run_workflow` (sandbox runs are exempt by
    // design — that tool doesn't route through `run_flow_body` at all).
    let inference_errors = validate_inference_readiness(config, &flow.graph).await;
    if !inference_errors.is_empty() {
        let detail = inference_errors.join(" ");
        let msg = format!("This flow's AI step needs a working AI provider to run. {detail}");
        tracing::warn!(
            target: "flows",
            flow_id,
            "[flows] run_flow_body: inference-readiness preflight failed — finalizing run as \
             failed without invoking the engine: {msg}"
        );
        if let Err(rec_err) = store::record_run(config, flow_id, "failed") {
            tracing::warn!(
                target: "flows",
                flow_id,
                error = %rec_err,
                "[flows] run_flow_body: failed to record failed run (inference preflight)"
            );
        }
        let observed = current_persisted_steps(config, &thread_id);
        finish_flow_run_row(
            config,
            &thread_id,
            flow_id,
            "failed",
            &observed,
            &[],
            Some(&msg),
            None,
        );
        finalizer.disarm();
        return Err(msg);
    }

    // Recompile to execute — the entry point already compile-checked to fail
    // fast before the running row existed. A failure *now* (after the row was
    // inserted) must finalize the row as failed, never orphan it.
    let compiled = match tinyflows::compiler::compile(&flow.graph) {
        Ok(compiled) => compiled,
        Err(e) => {
            let msg = e.to_string();
            tracing::warn!(target: "flows", flow_id, error = %msg, "[flows] run_flow_body: compile failed after start row inserted");
            let observed = current_persisted_steps(config, &thread_id);
            finish_flow_run_row(
                config,
                &thread_id,
                flow_id,
                "failed",
                &observed,
                &[],
                Some(&msg),
                None,
            );
            finalizer.disarm();
            return Err(msg);
        }
    };

    // Scope the state store per-flow so two flows never collide on a state key.
    let caps = crate::openhuman::flows::tinyflows::build_capabilities(
        config_arc.clone(),
        format!("flow:{flow_id}"),
    );
    let checkpointer = match crate::openhuman::flows::tinyflows::open_flow_checkpointer(config) {
        Ok(checkpointer) => checkpointer,
        Err(e) => {
            let msg = e.to_string();
            tracing::warn!(target: "flows", flow_id, error = %msg, "[flows] run_flow_body: checkpointer open failed after start row inserted");
            let observed = current_persisted_steps(config, &thread_id);
            finish_flow_run_row(
                config,
                &thread_id,
                flow_id,
                "failed",
                &observed,
                &[],
                Some(&msg),
                None,
            );
            finalizer.disarm();
            return Err(msg);
        }
    };

    // Record a failed attempt so `last_run_at`/`last_status` reflect reality
    // (a stop-policy engine/capability failure or a timeout) rather than
    // leaving the prior success/pending state on the flow. Preserve whatever
    // steps the observer persisted live (don't wipe them back to `[]`).
    let record_failed = |error: &str| {
        if let Err(rec_err) = store::record_run(config, flow_id, "failed") {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                error = %rec_err,
                "[flows] flows_run: failed to record failed run"
            );
        }
        let observed = current_persisted_steps(config, &thread_id);
        finish_flow_run_row(
            config,
            &thread_id,
            flow_id,
            "failed",
            &observed,
            &[],
            Some(error),
            None,
        );
    };

    let origin = workflow_origin(flow_id, flow.require_approval);
    // Per-run in-memory journal: tinyflows records every graph event as a
    // durable GraphObservation under the run's tinyagents run id, which the
    // post-run Langfuse export reads back. Process-local and dropped with the
    // run — never persisted.
    let journal = Arc::new(tinyflows::engine::InMemoryGraphEventJournal::new());
    // Live run observer (issue G2): persists each finished step into the
    // `flow_runs` row as it happens and streams a `FlowRunProgress` event to
    // the frontend, so the durable + journaled path also reports live.
    let observer: Arc<dyn tinyflows::observability::RunObserver> = Arc::new(
        crate::openhuman::flows::tinyflows::observability::FlowRunObserver::new(
            Arc::new(config.clone()),
            flow_id,
            thread_id.clone(),
        ),
    );
    // Scope the flow/run correlation (issue flow-approval-surface, PR2)
    // alongside the `Workflow` origin so a tool call the engine dispatches
    // can, if it parks in the `ApprovalGate`, stamp its `PendingApproval` with
    // `source_context = Flow { flow_id, run_id }` — the origin alone only
    // carries `flow_id`. See `approval::gate::APPROVAL_FLOW_RUN_CONTEXT`.
    let run = APPROVAL_FLOW_RUN_CONTEXT.scope(
        FlowRunContext {
            flow_id: flow_id.to_string(),
            run_id: thread_id.clone(),
        },
        with_origin(
            origin,
            tinyflows::engine::run_with_checkpointer_journaled_observed(
                &compiled,
                tinyflows::engine::RunInput::new(input).with_inputs(inputs),
                &caps,
                checkpointer,
                &thread_id,
                journal.clone(),
                &observer,
            ),
        ),
    );
    let timed = tokio::time::timeout(std::time::Duration::from_secs(FLOW_RUN_TIMEOUT_SECS), run);
    tokio::pin!(timed);
    // (The B42 drop-guard is armed near the top of this fn, before the first
    // `.await` — see `finalizer` there.)
    // Race the run against a cancellation signal (issue G4). `biased` checks the
    // cancel arm first so a `flows_cancel_run` that lands right as the run
    // settles still wins deterministically.
    let journaled = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => {
            tracing::info!(target: "flows", flow_id = %flow_id, thread_id = %thread_id, "[flows] flows_run: cancelled mid-run");
            if let Err(e) = store::record_run(config, flow_id, "cancelled") {
                tracing::warn!(target: "flows", flow_id = %flow_id, error = %e, "[flows] flows_run: failed to record cancelled run");
            }
            let observed = current_persisted_steps(config, &thread_id);
            finish_flow_run_row(
                config,
                &thread_id,
                flow_id,
                "cancelled",
                &observed,
                &[],
                Some("run cancelled"),
                None,
            );
            finalizer.disarm();
            drop_checkpoint(config, &thread_id).await;
            return Ok(RpcOutcome::single_log(
                json!({
                    "output": Value::Null,
                    "pending_approvals": Vec::<String>::new(),
                    "thread_id": thread_id,
                    "cancelled": true,
                }),
                format!("flow run cancelled: {thread_id}"),
            ));
        }
        result = &mut timed => match result {
            Ok(Ok(journaled)) => journaled,
            Ok(Err(e)) => {
                record_failed(&e.to_string());
                finalizer.disarm();
                tracing::warn!(target: "flows", flow_id = %flow_id, error = %e, "[flows] flows_run: run failed");
                return Err(e.to_string());
            }
            Err(_elapsed) => {
                let msg = format!("flow run timed out after {FLOW_RUN_TIMEOUT_SECS}s");
                record_failed(&msg);
                finalizer.disarm();
                tracing::warn!(target: "flows", flow_id = %flow_id, timeout_secs = FLOW_RUN_TIMEOUT_SECS, "[flows] flows_run: run timed out");
                return Err(msg);
            }
        },
    };
    let outcome = journaled.outcome;

    let settled = settle_steps(config, &thread_id, &outcome.output);
    let (status, error) = finalize_terminal_status(&settled, &outcome.pending_approvals);
    // T-M1: pin the graph this run just executed only on the write that parks
    // it — `flows_resume` recomputes and compares this hash against the
    // *current* flow graph before it will honour the approval. See
    // `compute_graph_hash`'s doc.
    let graph_hash = (status == "pending_approval")
        .then(|| compute_graph_hash(&flow.graph, flow.require_approval))
        .flatten();
    // Finalize the run row (and disarm the drop-guard) BEFORE the flow-summary
    // write, so a `record_run` failure can never leave the row wedged at
    // `running` — the row's terminal state is the correctness-critical write;
    // the summary is best-effort observability (see `start_flow_run_row`).
    finish_flow_run_row(
        config,
        &thread_id,
        flow_id,
        status,
        &settled,
        &outcome.pending_approvals,
        error.as_deref(),
        graph_hash.as_deref(),
    );
    finalizer.disarm();
    if let Err(e) = store::record_run(config, flow_id, status) {
        tracing::warn!(target: "flows", flow_id = %flow_id, status, error = %e, "[flows] flows_run: failed to record run summary (run row already finalized)");
    }
    export_run_to_langfuse(
        config,
        &flow.name,
        flow_id,
        &thread_id,
        status,
        trigger,
        &journal,
        &journaled.graph_run_ids.run_id,
    )
    .await;
    notify_pending_approval(&flow, &thread_id, &outcome.pending_approvals);

    tracing::info!(
        target: "flows",
        flow_id = %flow_id,
        status,
        pending_approvals = outcome.pending_approvals.len(),
        no_actionable_nodes,
        "[flows] flows_run: finished"
    );

    const NO_ACTIONABLE_NODES_NOTE: &str = "This flow's graph has no actionable nodes beyond \
         its trigger (no downstream action nodes, or no edges connecting them) — the run \
         completed without doing anything. Add and wire up at least one action node.";

    let mut result = json!({
        "output": outcome.output,
        "pending_approvals": outcome.pending_approvals,
        "thread_id": thread_id,
    });
    let mut logs = vec![format!("flow run {status}")];
    if no_actionable_nodes {
        result["note"] = json!(NO_ACTIONABLE_NODES_NOTE);
        logs.push(NO_ACTIONABLE_NODES_NOTE.to_string());
    }

    Ok(RpcOutcome::new(result, logs))
}

/// Resumes a `flows_run` that paused at a human-in-the-loop approval gate,
/// continuing it from the durable checkpoint (`thread_id`) with
/// `approvals` newly granted. The UI approval card (B3) calls this once the
/// user decides. See `tinyflows::engine::resume_with_checkpointer`'s doc for
/// the resume mechanics.
///
/// **Host-side approval guard (issue B2 finding #3):** tinyflows 0.2's
/// `resume_with_checkpointer` treats the resume call itself as approval of
/// whatever gate paused the run — its `approvals` argument is advisory only,
/// not enforced inside the crate (`flows_resume(..., approvals: [])` on a
/// paused run would otherwise still complete it). So before ever calling
/// into the engine, this loads the persisted `flow_runs` row for
/// `thread_id` (`flow_runs.id == thread_id`) and requires that `approvals`
/// names at least one of that row's *actually* pending node ids. A run
/// that isn't currently `pending_approval` (already completed, failed, or
/// unknown) is rejected outright — resuming an already-settled thread_id is
/// no longer treated as a harmless no-op, it's a clear error.
pub async fn flows_resume(
    config: &Config,
    flow_id: &str,
    thread_id: &str,
    approvals: Vec<String>,
    rejections: Vec<String>,
) -> Result<RpcOutcome<Value>, String> {
    let flow = store::get_flow(config, flow_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{flow_id}' not found"))?;

    let run_record = store::get_flow_run(config, thread_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            format!("no paused run to resume: no run recorded for thread '{thread_id}'")
        })?;
    if run_record.flow_id != flow_id {
        return Err(format!(
            "no paused run to resume: run '{thread_id}' belongs to flow '{}', not '{flow_id}'",
            run_record.flow_id
        ));
    }
    if run_record.status != "pending_approval" {
        return Err(format!(
            "no paused run to resume: run '{thread_id}' is not pending approval (status: {})",
            run_record.status
        ));
    }
    // A gate can't be both approved and denied in the same resume — that's an
    // ambiguous instruction, reject it up front.
    if let Some(dup) = approvals.iter().find(|a| rejections.contains(a)) {
        return Err(format!(
            "gate '{dup}' cannot be both approved and rejected in the same resume"
        ));
    }
    // Same host-side guard the approvals path uses (see this fn's doc): the
    // engine trusts whatever the resume delivers, so require that the caller's
    // approvals/rejections actually name a currently-pending gate before ever
    // touching the engine. A denial (issue G4) is enforced the same way — a
    // rejection naming a pending gate is a valid resume just as an approval is.
    let matches_pending = approvals
        .iter()
        .chain(rejections.iter())
        .any(|a| run_record.pending_approvals.contains(a));
    if !matches_pending {
        tracing::warn!(
            target: "flows",
            flow_id = %flow_id,
            %thread_id,
            ?approvals,
            ?rejections,
            pending = ?run_record.pending_approvals,
            "[flows] flows_resume: rejected — caller approvals/rejections name none of the pending gates"
        );
        return Err(format!(
            "no pending approval matches: approvals {approvals:?} / rejections {rejections:?} do \
             not name any of the currently pending gates {:?} for run '{thread_id}'",
            run_record.pending_approvals
        ));
    }

    // T-M1 — stale-approval graph pin. The approval card the user acted on
    // described the graph as it existed at park time. If `save_workflow` (or
    // any other `flows_update`) rewrote the flow's graph while the run sat
    // `pending_approval`, resuming would compile the CURRENT graph against
    // the OLD checkpoint and fire whatever the *new* config of the approved
    // node id now does — under an approval the user never actually saw.
    // `flows_update` deliberately has no in-flight/pending-run guard (that
    // would let a stale park hold a flow hostage for the whole TTL), so this
    // is the fail-closed boundary instead: refuse and settle the run rather
    // than execute. A `None` pin (a legacy row from before this guard
    // existed, or a graph that failed to hash at park time) is treated as
    // "unknown — allow, with a warning" so upgrading mid-park can never
    // strand an otherwise-valid in-flight approval.
    match run_record.graph_hash.as_deref() {
        Some(expected_hash) => {
            let current_hash = compute_graph_hash(&flow.graph, flow.require_approval);
            if current_hash.as_deref() != Some(expected_hash) {
                tracing::warn!(
                    target: "flows",
                    flow_id = %flow_id,
                    %thread_id,
                    expected_hash,
                    current_hash = ?current_hash,
                    "[flows] flows_resume: refusing — the flow's graph changed after this run \
                     parked (T-M1 stale-approval guard)"
                );
                // Settle the row FIRST and treat the guarded write as the
                // authority, exactly as `flows_cancel_run` does (see its
                // ORDER MATTERS note) — this refusal runs BEFORE this call
                // claims the run, so a concurrent resume can legitimately own
                // it by now:
                //
                //   1. Resume B reads the flow and computes a matching hash.
                //   2. `flows_update` rewrites the flow.
                //   3. Resume A reads it, computes a MISMATCH, and lands here.
                //   4. Resume B wins `mark_run_resuming`, flips the row to
                //      `running`, and starts executing approved side effects.
                //
                // `finish_flow_run_row`'s guard admits `running` as well as
                // `pending_approval`, so a blind write from A would relabel
                // B's live row `cancelled`, overwrite `last_status`, and drop
                // a checkpoint B is actively using. Acting only when the write
                // actually matched keeps A's refusal from touching B's run.
                //
                // A is refused either way: its own view of the graph is stale,
                // so it must never proceed regardless of who owns the row.
                let observed = current_persisted_steps(config, thread_id);
                let settled_by_us = finish_flow_run_row(
                    config,
                    thread_id,
                    flow_id,
                    "cancelled",
                    &observed,
                    &[],
                    Some(GRAPH_CHANGED_SINCE_PARK_ERROR),
                    None,
                );
                if settled_by_us {
                    if let Err(e) = store::record_run(config, flow_id, "cancelled") {
                        tracing::warn!(
                            target: "flows",
                            flow_id = %flow_id,
                            %thread_id,
                            error = %e,
                            "[flows] flows_resume: failed to record run summary (stale-approval refusal)"
                        );
                    }
                    // The checkpoint is for a graph that no longer exists as
                    // approved; drop it rather than leave it resumable against
                    // a future graph edit that happens to hash back to the
                    // same value.
                    drop_checkpoint(config, thread_id).await;
                } else {
                    tracing::info!(
                        target: "flows",
                        flow_id = %flow_id,
                        %thread_id,
                        "[flows] flows_resume: stale-approval refusal did not settle the row — another \
                         resume or cancel owns it now; leaving its status and checkpoint untouched"
                    );
                }
                return Err(GRAPH_CHANGED_SINCE_PARK_ERROR.to_string());
            }
        }
        None => {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                "[flows] flows_resume: no graph_hash pinned for this parked run (legacy row \
                 predating the T-M1 guard, or the graph failed to hash at park time) — allowing \
                 the resume without a graph-pin check"
            );
        }
    }

    // A pending checkpoint may have been created before this compatibility
    // gate shipped, so resume is an independent authoritative boundary.
    if let Err(error) = ensure_config_aware_engine_compatible(config, &flow.graph) {
        if let Err(rec_err) = store::record_run(config, flow_id, "failed") {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                error = %rec_err,
                "[flows] flows_resume: failed to record compatibility rejection"
            );
        }
        let observed = current_persisted_steps(config, thread_id);
        finish_flow_run_row(
            config,
            thread_id,
            flow_id,
            "failed",
            &observed,
            &[],
            Some(&error),
            None,
        );
        tracing::warn!(
            target: "flows",
            flow_id = %flow_id,
            %thread_id,
            %error,
            "[flows] flows_resume: rejected — unsupported engine topology"
        );
        return Err(error);
    }
    let compiled = tinyflows::compiler::compile(&flow.graph).map_err(|e| e.to_string())?;
    let config_arc = Arc::new(config.clone());
    let caps = crate::openhuman::flows::tinyflows::build_capabilities(
        config_arc.clone(),
        format!("flow:{flow_id}"),
    );
    let checkpointer = crate::openhuman::flows::tinyflows::open_flow_checkpointer(config)
        .map_err(|e| e.to_string())?;

    // Run-lifecycle parity with `flows_run` (R-M1). A resume executes the flow's
    // real approved side effects for up to `FLOW_RUN_TIMEOUT_SECS`, so it needs
    // the same three guards the run path has had since B41/B42 — it had none:
    //
    //  1. `run_registry::register` — without an entry, `flows_cancel_run` saw
    //     `is_in_flight == false`, took its "parked/stale" branch, wrote a
    //     terminal `cancelled` row and dropped the checkpoint out from under
    //     this still-executing resume. Registering makes the cancel take the
    //     signalled branch, which this fn now honours in the `select!` below.
    //  2. `mark_run_resuming` — flips the row off `pending_approval` so the
    //     parked-run TTL sweep stops matching a resume that is actively
    //     running.
    //  3. `RunRowFinalizer` — if this future is dropped mid-await (client
    //     disconnect during the long await), the row is reconciled to
    //     `interrupted` instead of being stranded at its old status.
    //
    // Register BEFORE the status flip for the same reason `flows_run` registers
    // before inserting its row: never let a cancel observe a live-looking row
    // that no registered run owns.
    let (cancel_token, _run_guard) = run_registry::register(thread_id);
    match store::mark_run_resuming(config, thread_id) {
        Ok(true) => {}
        Ok(false) => {
            // The guarded flip matched nothing: the run was cancelled or
            // TTL-expired between the status check above and here. Refuse
            // rather than executing approved side effects for a run that is no
            // longer live.
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                "[flows] flows_resume: run left 'pending_approval' before the resume could claim it — refusing"
            );
            return Err(format!(
                "no paused run to resume: run '{thread_id}' was cancelled or expired before the \
                 resume could start"
            ));
        }
        Err(e) => {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                error = %e,
                "[flows] flows_resume: failed to mark run as resuming"
            );
            return Err(e.to_string());
        }
    }
    let finalizer = RunRowFinalizer::new(config_arc, thread_id, flow_id);

    tracing::debug!(
        target: "flows",
        flow_id = %flow_id,
        %thread_id,
        approval_count = approvals.len(),
        rejection_count = rejections.len(),
        "[flows] flows_resume: resuming checkpointed run"
    );

    let origin = workflow_origin(flow_id, flow.require_approval);
    // Same per-run journal as `flows_run`: the resumed execution mints a new
    // tinyagents run id, so its observation slice is read under that id.
    let journal = Arc::new(tinyflows::engine::InMemoryGraphEventJournal::new());
    // Live observer (issue G2): the resumed run fires `on_step_finish` for each
    // node that runs after the interrupt boundary, so downstream steps are
    // persisted + streamed live too, keyed by the same `thread_id`/run row.
    let observer: Arc<dyn tinyflows::observability::RunObserver> = Arc::new(
        crate::openhuman::flows::tinyflows::observability::FlowRunObserver::new(
            Arc::new(config.clone()),
            flow_id,
            thread_id.to_string(),
        ),
    );
    // `rejections` (issue G4 — deny semantics): a denied gate routes to its
    // `error` port (recovery branch) or, if it has none, fails the run. The
    // empty-rejections case is byte-for-byte the prior approve-only resume.
    //
    // Same flow/run correlation scope as `flows_run` (see its comment) — a
    // resumed run can dispatch further tool calls that park, and those parks
    // need `source_context` too.
    let run = APPROVAL_FLOW_RUN_CONTEXT.scope(
        FlowRunContext {
            flow_id: flow_id.to_string(),
            run_id: thread_id.to_string(),
        },
        with_origin(
            origin,
            tinyflows::engine::resume_with_checkpointer_journaled_observed(
                &compiled,
                &caps,
                checkpointer,
                thread_id,
                approvals,
                rejections,
                journal.clone(),
                &observer,
            ),
        ),
    );

    // Terminal-write helper for the two failure arms. Row FIRST, then the
    // best-effort summary — see the settle path below for why the order matters.
    let record_failed = |msg: &str| {
        let observed = current_persisted_steps(config, thread_id);
        finish_flow_run_row(
            config,
            thread_id,
            flow_id,
            "failed",
            &observed,
            &[],
            Some(msg),
            None,
        );
        if let Err(e) = store::record_run(config, flow_id, "failed") {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                error = %e,
                "[flows] flows_resume: failed to record run summary (run row already finalized)"
            );
        }
    };

    let timed = tokio::time::timeout(std::time::Duration::from_secs(FLOW_RUN_TIMEOUT_SECS), run);
    tokio::pin!(timed);
    // Race the resume against a cancellation signal, exactly as `run_flow_body`
    // does. `biased` checks the cancel arm first so a `flows_cancel_run` landing
    // as the resume settles still wins deterministically.
    let journaled = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => {
            tracing::info!(target: "flows", flow_id = %flow_id, %thread_id, "[flows] flows_resume: cancelled mid-resume");
            let observed = current_persisted_steps(config, thread_id);
            finish_flow_run_row(
                config,
                thread_id,
                flow_id,
                "cancelled",
                &observed,
                &[],
                Some("run cancelled"),
                None,
            );
            finalizer.disarm();
            if let Err(e) = store::record_run(config, flow_id, "cancelled") {
                tracing::warn!(target: "flows", flow_id = %flow_id, error = %e, "[flows] flows_resume: failed to record cancelled run");
            }
            drop_checkpoint(config, thread_id).await;
            return Ok(RpcOutcome::single_log(
                json!({
                    "output": Value::Null,
                    "pending_approvals": Vec::<String>::new(),
                    "thread_id": thread_id,
                    "cancelled": true,
                }),
                format!("flow resume cancelled: {thread_id}"),
            ));
        }
        result = &mut timed => match result {
            Ok(Ok(journaled)) => journaled,
            Ok(Err(e)) => {
                record_failed(&e.to_string());
                finalizer.disarm();
                tracing::warn!(target: "flows", flow_id = %flow_id, %thread_id, error = %e, "[flows] flows_resume: run failed");
                return Err(e.to_string());
            }
            Err(_elapsed) => {
                let msg = format!("flow resume timed out after {FLOW_RUN_TIMEOUT_SECS}s");
                record_failed(&msg);
                finalizer.disarm();
                tracing::warn!(target: "flows", flow_id = %flow_id, %thread_id, timeout_secs = FLOW_RUN_TIMEOUT_SECS, "[flows] flows_resume: run timed out");
                return Err(msg);
            }
        },
    };
    let outcome = journaled.outcome;

    let settled = settle_steps(config, thread_id, &outcome.output);
    let (status, error) = finalize_terminal_status(&settled, &outcome.pending_approvals);
    // T-M1: a resumed run can itself re-park at a further gate — pin the
    // (already-verified-current, see the graph-hash check above) graph again
    // so a *second* stale-approval window is guarded exactly like the first.
    let graph_hash = (status == "pending_approval")
        .then(|| compute_graph_hash(&flow.graph, flow.require_approval))
        .flatten();
    // Finalize the run row (and disarm the drop-guard) BEFORE the flow-summary
    // write, matching `flows_run` (R-M3). This used to be inverted here, with
    // `record_run` propagating via `?`: a concurrent flow delete made the
    // summary write fail and returned early, leaving the row stranded at
    // `pending_approval` even though the engine had completed and its side
    // effects had fired — which the TTL sweep would later relabel `cancelled`.
    // The row's terminal state is the correctness-critical write; the summary is
    // best-effort observability.
    finish_flow_run_row(
        config,
        thread_id,
        flow_id,
        status,
        &settled,
        &outcome.pending_approvals,
        error.as_deref(),
        graph_hash.as_deref(),
    );
    finalizer.disarm();
    if let Err(e) = store::record_run(config, flow_id, status) {
        tracing::warn!(
            target: "flows",
            flow_id = %flow_id,
            %thread_id,
            status,
            error = %e,
            "[flows] flows_resume: failed to record run summary (run row already finalized)"
        );
    }
    export_run_to_langfuse(
        config,
        &flow.name,
        flow_id,
        thread_id,
        status,
        FlowRunTrigger::Resume,
        &journal,
        &journaled.graph_run_ids.run_id,
    )
    .await;
    notify_pending_approval(&flow, thread_id, &outcome.pending_approvals);

    tracing::info!(
        target: "flows",
        flow_id = %flow_id,
        %thread_id,
        status,
        pending_approvals = outcome.pending_approvals.len(),
        "[flows] flows_resume: finished"
    );

    Ok(RpcOutcome::single_log(
        json!({
            "output": outcome.output,
            "pending_approvals": outcome.pending_approvals,
            "thread_id": thread_id,
        }),
        format!("flow resume {status}"),
    ))
}

/// Lists the most recent runs for a flow (newest first), for the B3
/// run-history inspector. Runs a lazy parked-run TTL sweep first (see
/// [`sweep_expired_parked_runs`]) so the listing reflects any run that has now
/// aged out of `pending_approval`.
pub async fn flows_list_runs(
    config: &Config,
    flow_id: &str,
    limit: usize,
) -> Result<RpcOutcome<Vec<FlowRun>>, String> {
    sweep_expired_parked_runs(config).await;
    let runs = store::list_flow_runs(config, flow_id, limit).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        runs,
        format!("flow runs listed: {flow_id}"),
    ))
}

/// List the most recent runs across ALL flows, newest first — backs the
/// aggregate "All runs" page. Each returned run carries its `flow_id` so the UI
/// can group/label by workflow.
pub async fn flows_list_all_runs(
    config: &Config,
    limit: usize,
) -> Result<RpcOutcome<Vec<FlowRun>>, String> {
    sweep_expired_parked_runs(config).await;
    let runs = store::list_all_flow_runs(config, limit).map_err(|e| e.to_string())?;
    let count = runs.len();
    Ok(RpcOutcome::single_log(
        runs,
        format!("all flow runs listed: {count} run(s)"),
    ))
}

/// Manually prunes a flow's run history down to the retention cap
/// ([`store::MAX_FLOW_RUNS_PER_FLOW`]), deleting only terminal runs outside the
/// newest-N window. Never removes a `running` or `pending_approval` run — a
/// parked run must survive for a later `flows_resume`. Pruning also happens
/// automatically on every new-run insert; this RPC exposes it for an explicit
/// on-demand sweep (e.g. a maintenance action). Returns the number of runs
/// pruned.
pub async fn flows_prune_runs(config: &Config, flow_id: &str) -> Result<RpcOutcome<Value>, String> {
    let keep = store::MAX_FLOW_RUNS_PER_FLOW;
    let pruned = store::prune_flow_runs(config, flow_id, keep).map_err(|e| e.to_string())?;
    tracing::info!(target: "flows", flow_id, pruned, keep, "[flows] flows_prune_runs: manual retention sweep");
    Ok(RpcOutcome::single_log(
        json!({ "flow_id": flow_id, "pruned": pruned, "kept": keep }),
        format!("flow runs pruned: {flow_id} ({pruned} removed)"),
    ))
}

/// Loads a single flow run record by id (== `thread_id`). Runs the lazy
/// parked-run TTL sweep first so a stale parked run is reported as `cancelled`
/// rather than perpetually `pending_approval`.
pub async fn flows_get_run(config: &Config, run_id: &str) -> Result<RpcOutcome<FlowRun>, String> {
    sweep_expired_parked_runs(config).await;
    let run = store::get_flow_run(config, run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow run '{run_id}' not found"))?;
    Ok(RpcOutcome::single_log(
        run,
        format!("flow run loaded: {run_id}"),
    ))
}

/// Lazy TTL sweep (issue G4): expires every parked `pending_approval` run older
/// than [`FLOW_PARKED_TTL_SECS`] to a terminal `"cancelled"`, updates the flow
/// summary, and drops each expired run's durable checkpoint so it can't be
/// resumed. Mirrors the `approval` domain's expire-on-read idiom
/// (`approval::store::expire_stale`): called at the top of the run-read paths
/// rather than from a dedicated background timer, so it needs no scheduler.
///
/// Best-effort by construction — a sweep failure is logged and swallowed, never
/// failing the read that triggered it. The `flows_resume` status guard already
/// rejects any non-`pending_approval` run, so a swept run is unresumable the
/// instant its row flips, independent of the checkpoint drop.
pub async fn sweep_expired_parked_runs(config: &Config) -> usize {
    let now = Utc::now();
    let cutoff = (now - chrono::Duration::seconds(FLOW_PARKED_TTL_SECS)).to_rfc3339();
    let now_str = now.to_rfc3339();
    let error_msg = format!("parked run expired after {FLOW_PARKED_TTL_SECS}s awaiting approval");

    let swept = match store::expire_parked_runs(config, &cutoff, &now_str, &error_msg) {
        Ok(swept) => swept,
        Err(e) => {
            tracing::warn!(target: "flows", error = %e, "[flows] parked-run TTL sweep failed (read continues)");
            return 0;
        }
    };
    for (run_id, flow_id) in &swept {
        if let Err(e) = store::record_run(config, flow_id, "cancelled") {
            tracing::warn!(target: "flows", run_id, flow_id, error = %e, "[flows] TTL sweep: failed to update flow summary for expired run");
        }
        // Announce the terminal transition (R-m4). `expire_parked_runs` writes
        // the row directly rather than going through `finish_flow_run_row`, so
        // without this the sweep was the one terminal path that emitted no
        // `FlowRunFinished` — the boot sweep already publishes its own. Purely
        // event-driven consumers (the runs rail) would otherwise not observe a
        // TTL-expired run settle until their next poll.
        tracing::debug!(
            target: "flows",
            run_id,
            flow_id,
            "[flows] TTL sweep: publishing FlowRunFinished for expired parked run"
        );
        crate::core::event_bus::publish_global(
            crate::core::event_bus::DomainEvent::FlowRunFinished {
                flow_id: flow_id.to_string(),
                run_id: run_id.to_string(),
                status: "cancelled".to_string(),
            },
        );
        drop_checkpoint(config, run_id).await;
    }
    if !swept.is_empty() {
        tracing::info!(target: "flows", count = swept.len(), ttl_secs = FLOW_PARKED_TTL_SECS, "[flows] parked-run TTL sweep expired stale runs");
    }
    swept.len()
}

/// Boot-time orphan sweep (bug B42, part b): reconciles every `flow_runs` row
/// still at `status = 'running'` that has **no live in-process run** to a
/// terminal `"interrupted"`. A hard crash / SIGKILL / power loss leaves the
/// [`RunRowFinalizer`] drop-guard no chance to run, so a `running` row from the
/// prior process would otherwise stay wedged forever, rendering as a perpetual
/// blank spinner in the run-details sidebar.
///
/// Two independent guards keep the sweep off a run that **this** process owns:
///
/// 1. **A boot floor.** Only rows whose `started_at` predates
///    [`PROCESS_RUN_FLOOR`] are candidates at all, so a row this process
///    inserted is provably out of scope regardless of registration timing —
///    which is what the sweep is actually for: rows left by a *prior* process.
///    Sweeping a live run would not merely mislabel it (its own terminal write
///    would correct that) — it would `drop_checkpoint` it mid-run, and that is
///    unrecoverable.
/// 2. **The in-flight registry.** [`run_registry::is_in_flight`] gates each
///    surviving candidate. Both run entry points now register **before**
///    inserting the row, so within this process a `running` row is never
///    unregistered; this guard covers clock skew and rows stamped by a
///    differently-skewed process.
///
/// The two are deliberately redundant: either alone would be sufficient today,
/// and neither depends on the other's ordering assumption holding.
///
/// Each swept run also updates the flow summary, announces a terminal
/// `FlowRunFinished`, and drops its durable checkpoint (a `running` row is never
/// resumable — only `pending_approval` is). Best-effort by construction: a store
/// error is logged and the sweep returns what it managed.
pub async fn sweep_orphaned_running_runs_on_boot(config: &Config) -> usize {
    let now_str = Utc::now().to_rfc3339();
    const REASON: &str =
        "Run interrupted by an app restart — no live run was executing this row after boot.";

    let floor: &str = PROCESS_RUN_FLOOR.as_str();
    tracing::debug!(target: "flows", floor, "[flows] boot sweep: reconciling only runs started before this process");
    let candidates = match store::list_running_run_ids(config, floor) {
        Ok(candidates) => candidates,
        Err(e) => {
            tracing::warn!(target: "flows", error = %e, "[flows] boot sweep: failed to list running runs (skipping)");
            return 0;
        }
    };
    if candidates.is_empty() {
        return 0;
    }
    tracing::debug!(target: "flows", count = candidates.len(), "[flows] boot sweep: examining running rows for orphans");

    let mut swept = 0usize;
    for (run_id, flow_id) in candidates {
        if run_registry::is_in_flight(&run_id) {
            tracing::debug!(target: "flows", run_id = %run_id, flow_id = %flow_id, "[flows] boot sweep: run is live in-process — leaving it running");
            continue;
        }
        match store::mark_run_interrupted(config, &run_id, &now_str, REASON) {
            Ok(true) => {
                swept += 1;
                if let Err(e) = store::record_run(config, &flow_id, "interrupted") {
                    tracing::warn!(target: "flows", run_id = %run_id, flow_id = %flow_id, error = %e, "[flows] boot sweep: failed to update flow summary for reconciled run");
                }
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::FlowRunFinished {
                        flow_id: flow_id.clone(),
                        run_id: run_id.clone(),
                        status: "interrupted".to_string(),
                    },
                );
                drop_checkpoint(config, &run_id).await;
                tracing::info!(target: "flows", run_id = %run_id, flow_id = %flow_id, "[flows] boot sweep: reconciled orphaned running run to 'interrupted'");
            }
            Ok(false) => {
                tracing::debug!(target: "flows", run_id = %run_id, "[flows] boot sweep: row changed status concurrently — skipped");
            }
            Err(e) => {
                tracing::warn!(target: "flows", run_id = %run_id, error = %e, "[flows] boot sweep: failed to reconcile running run");
            }
        }
    }
    if swept > 0 {
        tracing::info!(target: "flows", count = swept, "[flows] boot sweep reconciled orphaned running runs to 'interrupted'");
    }
    swept
}

/// Cancels a flow run (issue G4), settling it to a terminal `"cancelled"`
/// status and dropping its durable checkpoint so the aborted thread can never
/// be resumed.
///
/// Two cases, distinguished by [`run_registry::cancel`]:
/// - **In-flight** (a `flows_run` / `flows_resume` currently executing its run
///   future): the token is signalled and that run's own cancellation arm writes
///   the terminal row + drops the checkpoint as it unwinds — we don't write the
///   row here, to avoid two writers racing the same `flow_runs` row.
/// - **Parked / stale** (a `pending_approval` run awaiting a human decision, or
///   a `running` row whose task is gone): no live task exists to unwind, so
///   this settles the row terminally itself and drops the checkpoint.
///
/// A run that is already terminal (`completed` / `completed_with_warnings` /
/// `failed` / `cancelled` / `interrupted`) is a clear error, not a silent
/// no-op — otherwise a settled warning run could be overwritten as
/// `"cancelled"`, corrupting the run-honesty status it already recorded, and an
/// already-`interrupted` run (reconciled by the drop-guard / boot sweep, bug
/// B42) could be clobbered back to `"cancelled"`.
pub async fn flows_cancel_run(config: &Config, run_id: &str) -> Result<RpcOutcome<Value>, String> {
    let run = store::get_flow_run(config, run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow run '{run_id}' not found"))?;

    if matches!(
        run.status.as_str(),
        "completed" | "completed_with_warnings" | "failed" | "cancelled" | "interrupted"
    ) {
        return Err(format!(
            "flow run '{run_id}' is already terminal (status: {}) — nothing to cancel",
            run.status
        ));
    }

    let signalled = run_registry::cancel(run_id);
    tracing::info!(
        target: "flows",
        run_id,
        flow_id = %run.flow_id,
        signalled,
        prior_status = %run.status,
        "[flows] flows_cancel_run: cancelling run"
    );

    if signalled {
        // The in-flight run's cancellation arm owns the terminal write + the
        // checkpoint drop; we've signalled it and return. Its settle is
        // eventual (the run future unwinds), so report "requested".
        return Ok(RpcOutcome::single_log(
            json!({ "run_id": run_id, "cancelled": true, "was_in_flight": true }),
            format!("flow run {run_id} cancellation requested"),
        ));
    }

    // Not in flight: settle the row terminally and drop the checkpoint here.
    //
    // ORDER MATTERS (R-M2). The status read above and `run_registry::cancel`
    // are two separate observations, and a live run can settle in the window
    // between them: it writes its own terminal row and deregisters, so
    // `cancel` returns `false` and we arrive here believing the run is merely
    // parked/stale. Writing `cancelled` unconditionally would then relabel a
    // fully-completed run — whose real side effects already fired — and drop a
    // checkpoint that is no longer ours to drop. So attempt the guarded row
    // write FIRST and treat it as the authority: it only matches a still-live
    // row, so `false` means the run settled underneath us. Only once it has
    // won do we record the flow summary and drop the checkpoint.
    let observed = current_persisted_steps(config, run_id);
    let settled_by_us = finish_flow_run_row(
        config,
        run_id,
        &run.flow_id,
        "cancelled",
        &observed,
        &[],
        Some("run cancelled"),
        None,
    );
    if !settled_by_us {
        tracing::info!(
            target: "flows",
            run_id,
            flow_id = %run.flow_id,
            prior_status = %run.status,
            "[flows] flows_cancel_run: run settled concurrently — leaving its terminal status intact"
        );
        return Err(format!(
            "flow run '{run_id}' settled before it could be cancelled — its recorded outcome was \
             left untouched"
        ));
    }
    if let Err(e) = store::record_run(config, &run.flow_id, "cancelled") {
        tracing::warn!(target: "flows", run_id, flow_id = %run.flow_id, error = %e, "[flows] flows_cancel_run: failed to record cancelled status on flow summary");
    }
    drop_checkpoint(config, run_id).await;

    Ok(RpcOutcome::single_log(
        json!({ "run_id": run_id, "cancelled": true, "was_in_flight": false }),
        format!("flow run {run_id} cancelled"),
    ))
}

/// Best-effort drop of a run's durable tinyagents checkpoint thread, so a
/// cancelled (or expired) run can never be resumed from its persisted interrupt
/// boundary. Logged, never fatal — the `flow_runs` row's terminal status is the
/// authoritative "not resumable" signal (the `flows_resume` guard already
/// rejects any non-`pending_approval` status); dropping the checkpoint is
/// belt-and-suspenders that also reclaims the storage.
async fn drop_checkpoint(config: &Config, thread_id: &str) {
    match crate::openhuman::flows::tinyflows::open_flow_checkpointer(config) {
        Ok(checkpointer) => match checkpointer.delete_thread(thread_id).await {
            Ok(()) => {
                tracing::debug!(target: "flows", thread_id, "[flows] dropped durable checkpoint for cancelled/expired run")
            }
            Err(e) => {
                tracing::warn!(target: "flows", thread_id, error = %e, "[flows] failed to drop durable checkpoint")
            }
        },
        Err(e) => {
            tracing::warn!(target: "flows", thread_id, error = %e, "[flows] could not open checkpointer to drop checkpoint");
        }
    }
}

/// Builds the `TrustedAutomation { Workflow }` origin scoped around every
/// `flows_run` / `flows_resume` invocation. See `flows_run`'s doc for why
/// this applies uniformly regardless of caller.
fn workflow_origin(flow_id: &str, require_approval: bool) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: flow_id.to_string(),
        source: TrustedAutomationSource::Workflow { require_approval },
    }
}

/// RFC3339 instant at which THIS process first entered the flow-run lifecycle —
/// the floor the boot orphan sweep (bug B42) uses to bound its candidate set.
///
/// Initialized on first touch by whichever comes first: [`start_flow_run_row`]
/// (which forces it *before* stamping the row it is about to insert) or
/// [`sweep_orphaned_running_runs_on_boot`]. Either ordering yields the same
/// invariant — **every `flow_runs` row this process inserts has
/// `started_at >= *PROCESS_RUN_FLOOR`** — so a sweep restricted to
/// `started_at < *PROCESS_RUN_FLOOR` provably only ever sees rows left behind by
/// a *prior* process.
///
/// The floor makes that guarantee structural rather than a consequence of
/// registration ordering. `run_registry::is_in_flight` alone once left a window
/// — the entry points used to insert the `running` row before `run_flow_body`
/// registered, so a live run was briefly `running`-but-not-in-flight, and
/// sweeping it there would `drop_checkpoint` it mid-run (unrecoverable, unlike
/// the status, which the live run's own terminal write would fix). Registration
/// has since moved ahead of the insert, closing that window at the source too;
/// the floor stays because it holds regardless of what future callers do with
/// that ordering.
static PROCESS_RUN_FLOOR: LazyLock<String> = LazyLock::new(|| Utc::now().to_rfc3339());

/// Best-effort insert of the initial `"running"` `flow_runs` row. Logged,
/// never fails the run — run-history persistence is an observability aid,
/// not a correctness requirement of the run itself.
fn start_flow_run_row(config: &Config, thread_id: &str, flow_id: &str) {
    // Anchor the boot-sweep floor BEFORE stamping this row, so this row's
    // `started_at` can never precede it. See [`PROCESS_RUN_FLOOR`].
    LazyLock::force(&PROCESS_RUN_FLOOR);
    let started_at = Utc::now().to_rfc3339();
    if let Err(e) = store::insert_flow_run(config, thread_id, flow_id, thread_id, &started_at) {
        tracing::warn!(target: "flows", flow_id, thread_id, error = %e, "[flows] failed to persist flow run start");
    }
}

/// Best-effort finalization of a `flow_runs` row. Logged, never fails the
/// run (see [`start_flow_run_row`]).
///
/// `graph_hash` (T-M1) should be `Some(hash)` only on the write that parks the
/// row (`status == "pending_approval"`) — every other caller passes `None`,
/// which clears any stale pin now that the row is leaving (or never entered)
/// `pending_approval`. See [`compute_graph_hash`] and `store::finish_flow_run`.
fn finish_flow_run_row(
    config: &Config,
    thread_id: &str,
    flow_id: &str,
    status: &str,
    steps: &[FlowRunStep],
    pending_approvals: &[String],
    error: Option<&str>,
    graph_hash: Option<&str>,
) -> bool {
    let finished_at = Utc::now().to_rfc3339();
    match store::finish_flow_run(
        config,
        thread_id,
        status,
        &finished_at,
        steps,
        pending_approvals,
        error,
        graph_hash,
    ) {
        Err(e) => {
            tracing::warn!(target: "flows", thread_id, status, error = %e, "[flows] failed to persist flow run finish");
            return false;
        }
        // The guarded UPDATE (R-M2) matched nothing: the row had already
        // settled to a terminal status before this write. Whoever settled it
        // first also published `FlowRunFinished`, so publishing again here
        // would emit a second terminal event for one run. Report the no-op
        // instead of pretending the write landed.
        Ok(false) => {
            tracing::warn!(
                target: "flows",
                flow_id,
                thread_id,
                attempted_status = status,
                "[flows] finish_flow_run_row: row already terminal — refusing to overwrite a settled run"
            );
            return false;
        }
        Ok(true) => {}
    }

    // `status` can be `"pending_approval"` here (see `finalize_terminal_status`)
    // when the run merely paused at a gate — that isn't a finish. `flows_resume`
    // later settles under the SAME `thread_id`/`run_id`, and `useFlowRunFinished`
    // de-dupes delivered events by `${flow_id}:${run_id}` (needed because the
    // socket bridge re-emits this event under two aliases and must collapse
    // them into one `onFinish` call). Publishing here for a pause would poison
    // that dedup cache, so the real completion event after resume would be
    // dropped as an "alias replay" and the run could stay stale in the runs
    // list until the 30s poll backstop (Codex review, PR #5115). Gate the
    // publish to actual terminal statuses; the row itself is still written
    // above so poll-based fallbacks (list/get RPCs) see the paused state
    // either way.
    if status == "pending_approval" {
        tracing::debug!(
            target: "flows",
            flow_id,
            thread_id,
            status,
            "[flows] finish_flow_run_row: run paused for approval — not a finish, skipping FlowRunFinished"
        );
        return true;
    }

    tracing::debug!(
        target: "flows",
        flow_id,
        thread_id,
        status,
        "[flows] finish_flow_run_row: publishing FlowRunFinished"
    );
    crate::core::event_bus::publish_global(crate::core::event_bus::DomainEvent::FlowRunFinished {
        flow_id: flow_id.to_string(),
        run_id: thread_id.to_string(),
        status: status.to_string(),
    });
    true
}

/// Computes a stable content hash of the flow configuration a run was approved
/// against — the T-M1 stale-approval guard (see `flows_resume`'s doc).
/// Persisted on a run row the moment it parks at `pending_approval`, and
/// recompared against the **current** flow before a resume is allowed to
/// execute, so a rewrite between park and resume is detected instead of
/// silently firing the new configuration under the old approval.
///
/// Covers the graph **and `require_approval`**. The flag is not cosmetic: it
/// feeds `workflow_origin(...)`, which becomes the `AgentTurnOrigin` for the
/// whole resumed execution, and `TrustedAutomationSource::Workflow {
/// require_approval: false }` **auto-allows every `external_effect` tool call**
/// where `true` parks each one for its own human decision. It is also settable
/// independently of the graph — `flows_update(.., graph_json: None,
/// require_approval: Some(false), ..)` leaves `.graph` byte-identical. Hashing
/// the graph alone would therefore leave the exact hole this guard exists to
/// close: park at a gate, user approves, the flag is flipped to `false` with the
/// graph untouched (pin still matches), and on resume every downstream
/// outbound node that would have parked now fires unattended.
///
/// Hashes a *canonicalized* JSON serialization — `serde_json::Value`'s object
/// map preserves insertion order in this crate (the `preserve_order` feature
/// is enabled transitively via other dependencies), so the same logical graph
/// serialized through two different code paths is not guaranteed to emit its
/// object keys in the same order. [`canonicalize_json`] recursively sorts
/// every object's keys before hashing so the hash depends only on graph
/// content, never on incidental key order. Returns `None` (never panics) if
/// the graph somehow fails to serialize.
///
/// **`None` means different things on the two sides, and the resume side fails
/// CLOSED.** At park time `None` simply stores no pin, so that run later takes
/// the legacy "unknown — allow, with a warning" path. At resume time the
/// comparison is `Some(expected) != None`, which is *true*, so a hash failure
/// is treated as a mismatch: the run is refused, settled terminally, and its
/// checkpoint dropped. That is the safer direction — a run whose current graph
/// cannot be hashed is a run whose approval cannot be verified — but it is the
/// opposite of fail-open, so do not read this as a guarantee that a serialize
/// failure leaves a resumable run resumable.
fn compute_graph_hash(graph: &WorkflowGraph, require_approval: bool) -> Option<String> {
    let raw = match serde_json::to_value(graph) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                target: "flows",
                error = %e,
                "[flows] compute_graph_hash: failed to serialize graph to JSON — proceeding without a graph pin"
            );
            return None;
        }
    };
    let raw = serde_json::json!({ "graph": raw, "require_approval": require_approval });
    let canonical = canonicalize_json(&raw);
    let serialized = match serde_json::to_string(&canonical) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                target: "flows",
                error = %e,
                "[flows] compute_graph_hash: failed to serialize canonicalized graph — proceeding without a graph pin"
            );
            return None;
        }
    };
    let digest = Sha256::digest(serialized.as_bytes());
    Some(hex::encode(digest))
}

/// Recursively rewrites every JSON object's keys into sorted order, leaving
/// arrays (whose element order is semantically meaningful) and scalars
/// unchanged. See [`compute_graph_hash`] for why this is needed before
/// hashing rather than trusting `serde_json`'s default map order.
fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut sorted = serde_json::Map::new();
            for key in keys {
                sorted.insert(key.clone(), canonicalize_json(&map[key]));
            }
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

/// Reconstructs a lean per-node step list from a settled run's
/// `output["nodes"]` map.
///
/// As of issue G2 (live run observation) this is no longer the primary source
/// of run steps — `flows::observability::FlowRunObserver` persists each step
/// live as it finishes (with real `status`/`duration_ms`). This reconstruction
/// is now only a **fallback**, used by [`settle_steps`] to fill in any node the
/// observer didn't emit an `on_step_finish` for (notably the trigger node),
/// and as the whole-run source when the observer saw nothing at all.
fn reconstruct_steps(output: &Value) -> Vec<FlowRunStep> {
    let Some(nodes) = output.get("nodes").and_then(Value::as_object) else {
        return Vec::new();
    };
    nodes
        .iter()
        .map(|(node_id, slot)| FlowRunStep {
            node_id: node_id.clone(),
            output: slot.get("items").cloned().unwrap_or(Value::Null),
            port: slot.get("port").and_then(Value::as_str).map(str::to_string),
            // Reconstructed post-hoc: no live status/timing (see FlowRunStep).
            status: None,
            duration_ms: None,
            diagnostics: Vec::new(),
        })
        .collect()
}

/// Reads back whatever steps the live [`FlowRunObserver`] has already persisted
/// onto the run's row. Best-effort: a read failure yields an empty list (the
/// caller still writes a terminal row), never propagating an error into the
/// run's settle path.
///
/// [`FlowRunObserver`]: crate::openhuman::flows::tinyflows::observability::FlowRunObserver
fn current_persisted_steps(config: &Config, run_id: &str) -> Vec<FlowRunStep> {
    store::get_flow_run(config, run_id)
        .ok()
        .flatten()
        .map(|run| run.steps)
        .unwrap_or_default()
}

/// Assembles the final step list to persist at settle: the live steps the
/// observer already recorded (carrying real `status`/`duration_ms`), plus any
/// node present in the post-hoc [`reconstruct_steps`] projection that the
/// observer never emitted a step for — the trigger node, or (defensively) an
/// observer that missed a step. If the observer recorded nothing at all
/// (e.g. a run that paused immediately at a gate before any node finished),
/// falls back wholesale to the reconstruction.
fn settle_steps(config: &Config, run_id: &str, output: &Value) -> Vec<FlowRunStep> {
    let reconstructed = reconstruct_steps(output);
    let persisted = current_persisted_steps(config, run_id);
    if persisted.is_empty() {
        tracing::debug!(
            target: "flows",
            run_id,
            reconstructed = reconstructed.len(),
            "[flows] settle_steps: no live-observed steps — using post-hoc reconstruction"
        );
        return reconstructed;
    }
    let mut merged = persisted;
    let mut filled = 0usize;
    for step in reconstructed {
        if !merged.iter().any(|s| s.node_id == step.node_id) {
            merged.push(step);
            filled += 1;
        }
    }
    tracing::debug!(
        target: "flows",
        run_id,
        step_count = merged.len(),
        filled_from_reconstruction = filled,
        "[flows] settle_steps: merged live-observed steps with post-hoc reconstruction"
    );
    merged
}

/// Degrades a would-be `"completed"` status: `"failed"` if any settled step
/// errored, `"completed_with_warnings"` if any carries null-resolution
/// diagnostics, else `"completed"`.
///
/// Called only once the run has no `pending_approvals` left — precedence
/// against that case is handled by the caller (`pending_approval` always
/// wins over any of these).
fn degrade_completed_status(steps: &[FlowRunStep]) -> &'static str {
    if steps.iter().any(|s| s.status.as_deref() == Some("error")) {
        return "failed";
    }
    if steps.iter().any(|s| !s.diagnostics.is_empty()) {
        "completed_with_warnings"
    } else {
        "completed"
    }
}

/// Names the node(s) whose step settled with `status == "error"` — the
/// engine's `ExecutionStep` carries no error message of its own for a step
/// that failed under an `on_error: "continue"`/`"route"` policy (it only
/// fails the *run* future, and so gets an actual error string, when the
/// policy is `"stop"`), so this is the best available detail for
/// [`FlowRun::error`] when [`degrade_completed_status`] degrades to
/// `"failed"` without an outer run-future `Err`.
fn failed_step_error_summary(steps: &[FlowRunStep]) -> Option<String> {
    let failed_nodes: Vec<&str> = steps
        .iter()
        .filter(|s| s.status.as_deref() == Some("error"))
        .map(|s| s.node_id.as_str())
        .collect();
    if failed_nodes.is_empty() {
        None
    } else {
        Some(format!(
            "node(s) failed after retries: {}",
            failed_nodes.join(", ")
        ))
    }
}

/// Computes a settled run's terminal status and, when that status is
/// `"failed"`, an accompanying error message — shared by `flows_run` and
/// `flows_resume` so the two call sites can't drift on the
/// `pending_approval` > `degrade_completed_status` precedence or forget to
/// populate [`FlowRun::error`] (its doc contract: "Error message when
/// `status == \"failed\"`") for a run that degraded via a settled step error
/// rather than an outer run-future `Err`.
fn finalize_terminal_status(
    settled: &[FlowRunStep],
    pending_approvals: &[String],
) -> (&'static str, Option<String>) {
    if !pending_approvals.is_empty() {
        return ("pending_approval", None);
    }
    let status = degrade_completed_status(settled);
    let error = if status == "failed" {
        failed_step_error_summary(settled)
    } else {
        None
    };
    (status, error)
}

/// Milliseconds since the Unix epoch, for `CoreNotificationEvent::timestamp_ms`.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Surfaces a paused run as a `CoreNotification` (category `Agents`) with an
/// "approve" action carrying `flow_id`/`thread_id`/`node_ids`, mirroring the
/// pattern `agent_meetings::calendar`'s auto-summarize "Ask" flow uses
/// (direct `publish_core_notification` call with an action payload, not the
/// generic `DomainEvent -> event_to_notification` bridge — this is a
/// flows-specific card with flow-specific action data, not a translation of
/// an existing broadcast event). No-op when nothing is pending.
fn notify_pending_approval(flow: &Flow, thread_id: &str, pending_approvals: &[String]) {
    if pending_approvals.is_empty() {
        return;
    }

    use crate::openhuman::desktop::notifications::bus::publish_core_notification;
    use crate::openhuman::desktop::notifications::types::{
        CoreNotificationAction, CoreNotificationCategory, CoreNotificationEvent,
    };

    let action_payload = json!({
        "flow_id": flow.id,
        "thread_id": thread_id,
        "node_ids": pending_approvals,
    });

    publish_core_notification(CoreNotificationEvent {
        id: format!("flow-pending-approval:{}:{}", flow.id, thread_id),
        category: CoreNotificationCategory::Agents,
        title: "Workflow needs approval".to_string(),
        body: format!(
            "\"{}\" is waiting on {} approval{} before it can continue.",
            flow.name,
            pending_approvals.len(),
            if pending_approvals.len() == 1 {
                ""
            } else {
                "s"
            }
        ),
        // No dedicated Workflows review route exists yet (B3 ships the UI);
        // leave unset rather than link to a page that can't act on it.
        deep_link: None,
        timestamp_ms: now_ms(),
        actions: Some(vec![CoreNotificationAction {
            action_id: "approve".to_string(),
            label: "Review".to_string(),
            payload: Some(action_payload),
        }]),
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Flow Scout — workflow discovery + suggestion lifecycle
// ─────────────────────────────────────────────────────────────────────────────

/// Overall safety bound on one `flows_discover` run. The `flow_discovery` agent
/// reasons read-only over the user's data and ends by emitting
/// `suggest_workflows`; its own `max_iterations` caps the loop, but a hung
/// LLM/tool call must never let the RPC block indefinitely.
///
/// Matches [`FLOW_BUILD_TIMEOUT_SECS`] (600s): the session builder applies the
/// `flow_discovery` definition's `effective_max_iterations()` (50, not the
/// global default of 10) to this path (issue #4868), so a worst-case run at
/// ~10s/iteration can take up to ~500s — the old 300s bound could clip a
/// legitimate long discovery run before the iteration cap ever got a chance
/// to (post-merge Codex P2 finding).
const FLOW_DISCOVER_TIMEOUT_SECS: u64 = 600;

/// The canned brief handed to the `flow_discovery` agent. The agent's own
/// archetype prompt teaches the read → correlate → ground → emit loop; this is
/// just the kick-off instruction for the on-demand "Discover" action.
const FLOW_DISCOVER_PROMPT: &str = "Discover the most useful automations you could set up for me. \
     Read what you can about how I work — my goals, recurring conversations, the people and apps I \
     deal with, and the flows I already have — then propose a few concrete, buildable workflows. \
     Ground each in something you actually observed about me, and end by calling suggest_workflows.";

// ─────────────────────────────────────────────────────────────────────────────
// Copilot / scout streaming (Phase B) — bridge a builder/scout turn's live
// AgentProgress onto the web-channel socket, keyed by a chat thread, exactly
// like an interactive chat turn. Blueprint: `agent/task_dispatcher/executor.rs`.
// ─────────────────────────────────────────────────────────────────────────────

/// Where to stream a `flows_build` / `flows_discover` turn. When present, the
/// agent's progress events (`text_delta` / `thinking_delta` / `tool_call` /
/// `tool_result` / terminal `chat_done`) are published as `WebChannelEvent`s
/// tagged with this `thread_id` — the same room the shared chat pane already
/// subscribes to and decodes — so the copilot/scout UI renders streamed text,
/// tool cards, and workflow-proposal cards live instead of spinning for the
/// whole (up to 300s) headless run.
///
/// Broadcast client id is always `"system"` (like cron / task-session runs), so
/// any client viewing the thread receives the events (the frontend keys by
/// `thread_id`). The blocking `{ proposal, assistant_text }` return is
/// unchanged — streaming is purely additive, opt-in per call.
#[derive(Debug, Clone)]
pub struct FlowStreamTarget {
    /// The chat thread the copilot/scout turn streams into.
    pub thread_id: String,
    /// Per-turn correlation id (matches the frontend `request_id`). Generated
    /// when the caller doesn't supply one.
    pub request_id: String,
}

impl FlowStreamTarget {
    /// Build a streaming target from optional RPC params. Streaming is enabled
    /// only when a non-empty `thread_id` is given; a missing/blank `request_id`
    /// is filled with a fresh uuid so the turn is always correlatable. Returns
    /// `None` (headless run, prior behaviour) when no usable `thread_id`.
    pub fn from_params(thread_id: Option<String>, request_id: Option<String>) -> Option<Self> {
        let thread_id = thread_id
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())?;
        let request_id = request_id
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Some(Self {
            thread_id,
            request_id,
        })
    }
}

/// Attach the web-channel progress bridge to `agent` for a builder/scout turn.
/// Wires an mpsc channel into the agent's progress sink and spawns the bridge
/// task that translates each [`AgentProgress`] into a socket event keyed by the
/// target thread (and mirrors a `TurnStateStore` so the tool timeline replays
/// on reopen). The bridge task lives until the agent drops its progress sender
/// (turn end). `source` is a short trace-attribution label (e.g.
/// `"flows_build"`).
fn attach_flow_progress_bridge(
    agent: &mut crate::openhuman::agent::Agent,
    target: &FlowStreamTarget,
    source: &str,
    config: &Config,
) {
    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(64);
    agent.set_on_progress(Some(progress_tx));
    tracing::info!(
        target: "flows",
        thread_id = %target.thread_id,
        request_id = %target.request_id,
        source = %source,
        "[flows] progress bridge: attaching (streaming copilot/scout turn)"
    );
    crate::openhuman::web_chat::spawn_progress_bridge(
        progress_rx,
        "system".to_string(),
        target.thread_id.clone(),
        target.request_id.clone(),
        crate::openhuman::threads::turn_state::TurnStateStore::new(config.workspace_dir.clone()),
        crate::openhuman::web_chat::ChatRequestMetadata {
            source: Some(source.to_string()),
            ..Default::default()
        },
        config.clone(),
    );
}

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

/// [`flows_build`] with caller-specific tools removed in addition to the
/// standard streaming/headless safety lists.
///
/// This is intentionally crate-private: product surfaces use [`flows_build`]'s
/// normal builder belt. Host integrations that add their own persistence
/// boundary can hide tools that would bypass that boundary.
pub(crate) async fn flows_build_with_extra_hidden_tools(
    config: &Config,
    req: crate::openhuman::flows::agents::workflow_builder::builder_prompt::BuilderRequest,
    stream: Option<FlowStreamTarget>,
    extra_hidden_tools: &[&str],
) -> Result<RpcOutcome<Value>, String> {
    use crate::openhuman::agent::Agent;
    use crate::openhuman::flows::agents::workflow_builder::builder_prompt::render_prompt;

    // Reject invalid turns (e.g. a `build` with no `flow_id`) before we render a
    // brief that would tell the agent to save onto nothing.
    req.validate()?;

    let prompt = render_prompt(&req);
    tracing::info!(
        target: "flows",
        mode = ?req.mode,
        has_graph = req.graph.is_some(),
        flow_id = req.flow_id.as_deref().unwrap_or("<none>"),
        streaming = stream.is_some(),
        "[flows] flows_build: starting workflow_builder turn"
    );

    // The registry must be initialised before building a named builtin agent
    // (idempotent — mirrors `flows_discover`).
    crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(&config.workspace_dir)
        .map_err(|e| format!("failed to initialise agent registry: {e}"))?;

    // Issue #4868 — the session builder (`build_session_agent_inner`) now
    // resolves the per-agent iteration cap from the `workflow_builder`
    // `AgentDefinition` itself (`iteration_policy = "extended"` ->
    // `effective_max_iterations()` = 50), so no override is needed here.
    let mut agent = Agent::from_config_for_agent(config, "workflow_builder")
        .map_err(|e| format!("failed to build workflow_builder agent: {e:#}"))?;
    agent.set_agent_definition_name("workflow_builder".to_string());

    // Restrict the visible run-advancing tools per path (PR3:
    // flows-copilot-live-run-approval). Streaming (copilot pane, real approval
    // surface below) only hides the always-hidden `run_workflow`; headless
    // (CLI / tests / no chat thread) keeps the full historical hide-list
    // (issue #4593 / #4881) since there is no routable approval surface there.
    //
    // The reduced (copilot) hide-list is safe ONLY when the process-global
    // `ApprovalGate` is actually installed to park the unhidden
    // `run_flow`/`resume_flow_run`. `flows_build` is a public RPC and the gate
    // can be opted out (`OPENHUMAN_APPROVAL_GATE=0` on CLI/docker leaves
    // `ApprovalGate::try_global()` == `None`; desktop always installs it) — and
    // `ApprovalSecurityMiddleware` skips interception entirely when the gate is
    // absent, so the WebChat origin below would NOT park and the unhidden
    // live-run tools would execute unapproved. Fall back to the full hide-list
    // whenever the gate is not installed, regardless of `stream`. (codex #5090)
    let approval_gate_active =
        crate::openhuman::security::approval::ApprovalGate::try_global().is_some();
    if stream.is_some() && approval_gate_active {
        restrict_builder_toolset_for_copilot(&mut agent);
    } else {
        if stream.is_some() {
            tracing::warn!(
                target: "flows",
                "[flows] flows_build: streaming turn but no ApprovalGate installed \
                 (OPENHUMAN_APPROVAL_GATE off / headless) — keeping the full live-run \
                 hide-list so run_flow/resume_flow_run cannot execute unapproved"
            );
        }
        restrict_builder_toolset(&mut agent);
    }
    if !extra_hidden_tools.is_empty() {
        tracing::debug!(
            target: "flows",
            hidden = ?extra_hidden_tools,
            "[flows] flows_build: applying caller-specific hidden tools"
        );
        agent.hide_tools(extra_hidden_tools);
    }

    // When a chat thread is attached (the copilot pane), stream the builder turn
    // into it exactly like an interactive turn — text/tool deltas and the
    // `propose_workflow` tool result the frontend renders as a proposal card.
    // Best-effort — with no target the run stays headless (CLI / tests).
    if let Some(target) = &stream {
        attach_flow_progress_bridge(&mut agent, target, "flows_build", config);
    }

    // Run to completion, bounded by a wall-clock timeout. PR3
    // (flows-copilot-live-run-approval): the origin now depends on whether a
    // chat thread is attached.
    //
    // - Streaming (copilot pane): run under `AgentTurnOrigin::WebChat` with
    //   `APPROVAL_CHAT_CONTEXT` scoped alongside it — the identical
    //   double-scope pattern `web_chat::ops::run_turn_under_cancel_and_deadline`
    //   uses for a real interactive chat turn. The approval gate then PARKS
    //   (rather than auto-allows) any `external_effect` tool call instead of
    //   failing closed, and the resulting `ApprovalRequested` event routes back
    //   to this thread (`client_id: "system"` — every client auto-joins that
    //   broadcast room, matching the progress bridge above) for the existing
    //   `ApprovalRequestCard` to render. The run is additionally wrapped in the
    //   thread-id scope so descendant turns tag their trace + socket events
    //   with this thread.
    // - Headless (CLI / tests / no chat thread): unchanged `AgentTurnOrigin::Cli`
    //   — the gate auto-allows `external_effect` tools under that origin, which
    //   is why `restrict_builder_toolset` above must keep the full hide-list on
    //   this path; there is no routable approval surface here to park against.
    // Outcome of racing the run future against its wall-clock timeout and
    // (streaming only) a user Stop-button cancellation. Kept as one enum so
    // both branches below (and the settle match after) share one shape.
    enum BuildRunOutcome {
        /// The agent run itself finished (or errored) before the timeout or a
        /// cancel raced it.
        Ran(anyhow::Result<String>),
        /// `FLOW_BUILD_TIMEOUT_SECS` elapsed first.
        TimedOut,
        /// The user cancelled the turn (`flows_build_cancel`) before it
        /// finished. Streaming-only — the headless/CLI branch never
        /// registers a token, so it can never produce this.
        Cancelled,
    }

    let timed = match &stream {
        Some(target) => {
            let origin = AgentTurnOrigin::WebChat {
                thread_id: target.thread_id.clone(),
                client_id: "system".to_string(),
                request_id: Some(target.request_id.clone()),
            };
            let chat_ctx = ApprovalChatContext {
                thread_id: target.thread_id.clone(),
                client_id: "system".to_string(),
            };
            tracing::info!(
                target: "flows",
                thread_id = %target.thread_id,
                request_id = %target.request_id,
                "[flows] flows_build: streaming copilot turn — WebChat origin + \
                 APPROVAL_CHAT_CONTEXT scoped, live-run tools park for approval instead \
                 of auto-allowing (shortened to COPILOT_APPROVAL_TTL via \
                 APPROVAL_COPILOT_STREAM_CONTEXT)"
            );
            // `APPROVAL_COPILOT_STREAM_CONTEXT` scopes alongside the existing
            // chat context so any `run_flow`/`resume_flow_run` park raised by
            // this turn is clamped to the shorter `COPILOT_APPROVAL_TTL`
            // instead of the gate's full ten-minute default — a stale park on
            // a copilot pane the user may have already navigated away from
            // shouldn't idle that long. Main-chat turns never scope this, so
            // they are unaffected.
            let run = with_origin(
                origin,
                APPROVAL_CHAT_CONTEXT.scope(
                    chat_ctx,
                    APPROVAL_COPILOT_STREAM_CONTEXT.scope((), agent.run_single(&prompt)),
                ),
            );
            let run =
                tokio::time::timeout(std::time::Duration::from_secs(FLOW_BUILD_TIMEOUT_SECS), run);
            let run = crate::openhuman::agent::tinyagents::thread_context::with_thread_id(
                target.thread_id.clone(),
                run,
            );

            // Register this turn's cancellation token BEFORE racing the run,
            // so a `flows_build_cancel` call landing the instant this turn
            // starts can never miss the registration window. The run stays
            // awaited INLINE (never spawned) — spawning it would drop the
            // task-local `with_origin` / `APPROVAL_CHAT_CONTEXT.scope` /
            // `APPROVAL_COPILOT_STREAM_CONTEXT.scope` / thread-id scope
            // context above, which the approval gate + tracing depend on.
            // `tokio::select!` races the two futures on THIS task instead, so
            // every one of those scopes stays attached to the winning arm.
            let token = CancellationToken::new();
            build_registry::register_build_turn(
                target.thread_id.clone(),
                Some(target.request_id.clone()),
                token.clone(),
            );
            let outcome = tokio::select! {
                r = run => match r {
                    Ok(inner) => BuildRunOutcome::Ran(inner),
                    Err(_) => BuildRunOutcome::TimedOut,
                },
                _ = token.cancelled() => {
                    tracing::debug!(
                        target: "flows",
                        thread_id = %target.thread_id,
                        request_id = %target.request_id,
                        "[flows] flows_build: cancelled by user"
                    );
                    BuildRunOutcome::Cancelled
                }
            };
            // Unconditional — covers every exit the `select!` above can take
            // (ran to completion, errored, timed out, or was cancelled); there
            // is no early return between `register_build_turn` and here that
            // could skip it.
            build_registry::unregister_build_turn(&target.thread_id, Some(&target.request_id));
            outcome
        }
        None => {
            tracing::debug!(
                target: "flows",
                "[flows] flows_build: headless/CLI turn — Cli origin, approval gate \
                 auto-allows external_effect tools (run-advancing tools stay hidden)"
            );
            let run = with_origin(AgentTurnOrigin::Cli, agent.run_single(&prompt));
            match tokio::time::timeout(std::time::Duration::from_secs(FLOW_BUILD_TIMEOUT_SECS), run)
                .await
            {
                Ok(inner) => BuildRunOutcome::Ran(inner),
                Err(_) => BuildRunOutcome::TimedOut,
            }
        }
    };
    let (assistant_text, run_error, cancelled) = match timed {
        BuildRunOutcome::Ran(Ok(text)) => (text, None, false),
        BuildRunOutcome::Ran(Err(e)) => {
            tracing::warn!(target: "flows", error = %e, "[flows] flows_build: agent run failed");
            (
                String::new(),
                Some(format!("workflow_builder run failed: {e:#}")),
                false,
            )
        }
        BuildRunOutcome::TimedOut => {
            tracing::warn!(
                target: "flows",
                timeout_secs = FLOW_BUILD_TIMEOUT_SECS,
                "[flows] flows_build: agent run timed out"
            );
            (
                String::new(),
                Some(format!(
                    "workflow_builder run timed out after {FLOW_BUILD_TIMEOUT_SECS}s"
                )),
                false,
            )
        }
        // A user Stop is not an error (`run_error = None`) — it must not be
        // reported as a failed turn, nor fall into the trail-off backstop
        // below that synthesizes a "continue?" question for a turn that
        // quietly ran out of steam; a deliberate cancel is neither.
        BuildRunOutcome::Cancelled => (String::new(), None, true),
    };

    // Capture the proposal from the run's tool history (propose/revise/save all
    // emit the same self-describing `{ type: "workflow_proposal", … }` payload).
    // Extracted BEFORE the stream is finalized below (issue: builder
    // convergence): the trail-off backstop needs `proposal`/`capped` to decide
    // whether to override `assistant_text`, and the streamed copilot-pane chat
    // bubble must render the SAME (possibly-overridden) text as the RPC
    // response — the frontend renders from the stream, not the return value,
    // so patching only the latter would still leave an interactive user
    // staring at the original silent/status-only text.
    let proposal = extract_workflow_proposal(agent.history());

    // A user-cancelled turn settles here, clean and separate from the
    // error/trail-off paths below: `finalize_flow_stream` gets an `Ok(...)` (a
    // Stop is not an error) so the copilot pane receives the same `chat_done`
    // terminal event a normal completion would — `ChatRuntimeProvider` ends
    // the inference turn / detaches the streaming state on that event exactly
    // as it does for any other settle, so nothing is left dangling on the FE.
    // Whatever `proposal`/`assistant_text` the turn produced before the
    // cancel raced it (e.g. it had already called `propose_workflow`) is
    // still returned — cancelling doesn't discard partial progress.
    if cancelled {
        if let Some(target) = &stream {
            let terminal: Result<String, String> = Ok(assistant_text.clone());
            finalize_flow_stream(target, &terminal, &prompt).await;
        }
        tracing::info!(
            target: "flows",
            flow_id = req.flow_id.as_deref().unwrap_or("<none>"),
            has_proposal = proposal.is_some(),
            "[flows] flows_build: workflow builder turn cancelled by user"
        );
        return Ok(RpcOutcome::single_log(
            json!({
                "proposal": proposal,
                "assistant_text": assistant_text,
                "error": Value::Null,
                "capped": false,
                "trail_off": false,
            }),
            "workflow builder turn cancelled by user",
        ));
    }

    // A run that both errored AND produced no proposal is a hard failure; a run
    // that proposed before erroring still returns the proposal for review.
    if proposal.is_none() {
        if let Some(err) = &run_error {
            if let Some(target) = &stream {
                let terminal: Result<String, String> = Err(err.clone());
                finalize_flow_stream(target, &terminal, &prompt).await;
            }
            return Err(format!("workflow_builder produced no proposal: {err}"));
        }
    }

    // (B34) Whether this turn paused because it hit `max_tool_iterations`
    // rather than finishing naturally (asking a question, or proposing). A
    // capped turn with no proposal renders a raw checkpoint ("Done so far /
    // Next steps") that's indistinguishable, in the response shape alone,
    // from the agent voluntarily asking a clarifying question — `capped`
    // gives the frontend the explicit signal to render a "Continue building"
    // card instead. Scoped to `proposal.is_none()`: a turn that hit the cap
    // but still squeezed out a proposal (the checkpoint fires before the
    // final `propose_workflow` call in that ordering) has nothing left to
    // continue.
    let hit_cap = agent.last_turn_hit_cap();
    let capped = hit_cap && proposal.is_none();

    // Terminal-state guarantee (builder convergence fix): a turn can end
    // "naturally" (no more tool calls, not capped, no run error) yet still
    // produce neither a proposal nor a real question — the model ran out of
    // steam mid-build and left a status dump ("Done so far: checked
    // connections…") as its final reply. `prompt.md` tells the model to
    // always end a building turn in a proposal or a question, but a prompt
    // rule can be silently ignored; this is the fail-closed backend backstop
    // that makes it a hard invariant regardless of model behavior — the user
    // is NEVER left with silence or an unanswerable status note.
    let trail_off = !capped && proposal.is_none() && run_error.is_none();
    let assistant_text = if trail_off && !text_looks_like_question(&assistant_text) {
        let fallback = build_trail_off_fallback(agent.history());
        let combined = combine_trail_off_fallback(&fallback, &assistant_text);
        tracing::warn!(
            target: "flows",
            flow_id = req.flow_id.as_deref().unwrap_or("<none>"),
            original_len = assistant_text.len(),
            fallback_len = fallback.len(),
            combined_len = combined.len(),
            "[flows] flows_build: trail-off detected (no proposal, no cap, no question) — \
             guaranteeing a fallback question while preserving the model's original text"
        );
        combined
    } else {
        assistant_text
    };

    // Emit the terminal chat event so a client viewing the copilot thread stops
    // "processing" and finalizes the assistant bubble (the bridge streams only
    // intermediate deltas). Success delivers `chat_done`; a run error delivers
    // `chat_error`. The blocking return below is unchanged. Uses the
    // (possibly trail-off-overridden) `assistant_text` above.
    if let Some(target) = &stream {
        let terminal: Result<String, String> = match &run_error {
            None => Ok(assistant_text.clone()),
            Some(err) => Err(err.clone()),
        };
        finalize_flow_stream(target, &terminal, &prompt).await;
    }

    tracing::info!(
        target: "flows",
        flow_id = req.flow_id.as_deref().unwrap_or("<none>"),
        has_proposal = proposal.is_some(),
        hit_cap,
        capped,
        trail_off,
        "[flows] flows_build: workflow_builder turn complete"
    );
    Ok(RpcOutcome::single_log(
        json!({
            "proposal": proposal,
            "assistant_text": assistant_text,
            "error": run_error,
            "capped": capped,
            "trail_off": trail_off,
        }),
        "workflow builder turn complete",
    ))
}

/// Cancel the in-flight `flows_build` (Workflow Copilot) turn streaming into
/// `thread_id`, scoped by `request_id` — the real, working half of the
/// composer's Stop button (issue: the original FE-only version hid the
/// button but never touched the running turn, since `flows_build` runs the
/// agent inline and never registers in `web_chat::IN_FLIGHT` or
/// `task_dispatcher::ACTIVE_RUNS`).
///
/// When `request_id` is `Some`, the cancel only fires if it matches the turn
/// currently registered on `thread_id` — a stale Stop click for a
/// superseded/earlier request can't kill a newer turn that has since started
/// on the same thread (mirrors `task_dispatcher::cancel_session_scoped`,
/// #4760). `None` cancels whatever turn is on the thread. Returns whether a
/// turn was found and signalled; `false` is not an error — it just means
/// nothing was in flight to cancel (already settled, or never started).
pub async fn flows_build_cancel(
    thread_id: &str,
    request_id: Option<&str>,
) -> Result<RpcOutcome<Value>, String> {
    let cancelled = build_registry::cancel_build_turn_scoped(thread_id, request_id);
    tracing::info!(
        target: "flows",
        thread_id,
        request_id = request_id.unwrap_or("<none>"),
        cancelled,
        "[flows] flows_build_cancel: cancel request handled"
    );
    Ok(RpcOutcome::single_log(
        json!({ "cancelled": cancelled }),
        if cancelled {
            "workflow builder turn cancellation requested"
        } else {
            "no in-flight workflow builder turn to cancel"
        },
    ))
}

/// Heuristic: does `text` already contain a clear, answerable question in its
/// final paragraph? Conservative by design (issue: builder convergence) — a
/// false negative (an actual question this misses) no longer discards the
/// model's text (see `combine_trail_off_fallback`), so the safe failure mode
/// stays "add a guaranteed question on top", never "under-detect and stay
/// silent".
///
/// Regression (#4887 follow-up): the original version only checked for a `?`
/// at the very end of the text / last line, which false-negatived on the
/// extremely common LLM pattern "What's X? You can find it at Y." — a real
/// question immediately followed by a trailing instructional sentence. The
/// backstop then clobbered a specific, answerable question with a generic
/// fallback. To catch that shape, this now also scans the LAST non-empty
/// paragraph for a `?` that isn't inside inline code or a fenced code block
/// (so a literal `?` in a code sample, e.g. `WHERE id = ?`, doesn't count).
///
/// Note: the trailing-noise strip below deliberately does NOT include the
/// backtick. Stripping a trailing backtick would peel off the CLOSING
/// delimiter of a code span whose last character is `?` (e.g. `` `id = ?` ``
/// at the very end of the text), exposing that `?` as if it were a bare
/// trailing question mark and defeating the code guard entirely.
fn text_looks_like_question(text: &str) -> bool {
    let trimmed = text
        .trim()
        .trim_end_matches(['"', '\'', ')', ']', '*', '_', '.'])
        .trim_end();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.ends_with('?') {
        return true;
    }
    // The question may not be the literal last character (trailing markdown
    // like a closing code fence or list marker on its own line) — fall back
    // to the last non-blank line.
    if trimmed
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .is_some_and(|last_line| last_line.trim_end().ends_with('?'))
    {
        return true;
    }
    // Final-paragraph scan: a question can sit mid-paragraph, followed by a
    // further trailing sentence on the SAME line/paragraph ("...ID? You can
    // find it under Profile > Copy member ID."). Take the last non-blank
    // paragraph and accept it if it contains a `?` that isn't inside inline
    // code / a code fence.
    last_paragraph(trimmed)
        .as_deref()
        .is_some_and(question_mark_outside_code)
}

/// Returns the last non-blank paragraph of `text` — a maximal run of
/// consecutive non-blank lines, working backward from the end and skipping
/// any trailing blank lines first. `None` if `text` has no non-blank lines.
///
/// CodeRabbit review follow-up: this used to split on the literal `"\n\n"`
/// byte sequence, which mishandles two real shapes:
/// - **CRLF input** (`"question?\r\n\r\nstatus"`): the separator is
///   `"\r\n\r\n"`, not `"\n\n"`, so the whole text was treated as ONE
///   paragraph — an earlier question could then suppress the fallback for a
///   trailing non-question status paragraph.
/// - **Whitespace-only separator lines** (`"question?\n \nstatus"` — a blank
///   line that isn't perfectly empty): same failure, same reason.
///
/// Working line-by-line via [`str::lines`] (which normalizes CRLF) and
/// treating any all-whitespace line as blank fixes both.
fn last_paragraph(text: &str) -> Option<String> {
    let mut collected: Vec<&str> = Vec::new();
    for line in text.lines().rev() {
        if line.trim().is_empty() {
            if collected.is_empty() {
                continue; // still skipping trailing blank lines
            }
            break; // blank line marks the start of the paragraph above
        }
        collected.push(line);
    }
    if collected.is_empty() {
        return None;
    }
    collected.reverse();
    Some(collected.join("\n"))
}

/// Does `text` contain at least one *sentence-terminal* `?` that isn't
/// inside a backtick-delimited code span (inline code like `` `U...` `` or a
/// fenced block like `` ``` ``)? Follows the CommonMark code-span rule: a
/// *run* of one or more consecutive backticks opens a span, and that span is
/// closed only by the next run of the SAME length — a shorter or longer run
/// of backticks encountered while inside a span is just literal backtick
/// characters, not a delimiter.
///
/// CodeRabbit review follow-up: an earlier version tracked a running
/// per-character backtick COUNT and used its parity (even = outside code).
/// That misclassifies any multi-backtick span whose delimiter is more than
/// one backtick — e.g. ``` ``SELECT ? FROM t`` ``` opens with a 2-backtick
/// run (count 0→2, even → looks "outside" again immediately), so the `?`
/// inside a valid double-backtick span was wrongly treated as outside code.
/// Tracking delimiter run LENGTH (not raw backtick count) fixes this while
/// still handling the common single-backtick and triple-backtick-fence
/// cases, since those are just the run-length-1 and run-length-3 instances
/// of the same rule.
///
/// Codex review follow-up: a bare `?` outside code isn't necessarily a real
/// question — a status line like "Checked https://api.example/search?q=foo
/// and got 403." has one mid-token, in a URL query string. Counting that
/// would flip `text_looks_like_question` to `true` and skip
/// `combine_trail_off_fallback` entirely, leaving the user with an
/// unanswerable status note — exactly the failure mode this backstop exists
/// to prevent. So each candidate `?` is additionally required to be
/// sentence-terminal via [`is_sentence_terminal_question_mark`].
fn question_mark_outside_code(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    // `Some(n)` while scanning is inside a code span opened by a run of `n`
    // backticks; that span closes only on the next run of exactly `n`.
    let mut open_run_len: Option<usize> = None;
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            let start = i;
            while i < chars.len() && chars[i] == '`' {
                i += 1;
            }
            let run_len = i - start;
            open_run_len = match open_run_len {
                None => Some(run_len),
                Some(n) if n == run_len => None,
                Some(n) => Some(n), // mismatched run length: still inside the span
            };
            continue;
        }
        if chars[i] == '?'
            && open_run_len.is_none()
            && is_sentence_terminal_question_mark(&chars, i)
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Is the `?` at `chars[index]` sentence-terminal — i.e. does it read as an
/// actual question mark rather than a character that merely happens to be a
/// `?` mid-token (a URL query string like `search?q=foo`, a shell glob,
/// etc.)? Skips over any immediately-following closing quote/bracket
/// punctuation (`"`, `'`, right single/double quotes, `)`, `]`) and requires
/// what remains to be whitespace or the end of the text — the shape a `?`
/// takes at the end of a real sentence or clause.
fn is_sentence_terminal_question_mark(chars: &[char], index: usize) -> bool {
    let mut i = index + 1;
    while let Some(&c) = chars.get(i) {
        if matches!(c, '"' | '\'' | '\u{2019}' | '\u{201D}' | ')' | ']') {
            i += 1;
            continue;
        }
        return c.is_whitespace();
    }
    true // '?' was the last character in the paragraph.
}

/// Builder-authoring tools whose result body can explain a trail-off — the
/// authoring belt `dry_run_workflow`/`validate_workflow`/`propose_workflow`/
/// `revise_workflow`/`edit_workflow`/`save_workflow` all report either a hard
/// gate rejection (`ToolResult::error`) or a self-reported broken-graph
/// result (`"ok": false` in a successful body), so a plain-text read-only
/// tool's output is never misattributed as the blocker.
const TRAIL_OFF_BLOCKER_TOOLS: &[&str] = &[
    "dry_run_workflow",
    "validate_workflow",
    "propose_workflow",
    "revise_workflow",
    "edit_workflow",
    "save_workflow",
];

/// Synthesizes a guaranteed, user-facing fallback for a trail-off turn (no
/// proposal, not capped, no run error, and the model's own text isn't a
/// question). Scans the run's tool history for the last builder-tool result
/// that looks like a blocker (a hard-gate rejection, or a `dry_run_workflow`/
/// `validate_workflow` report with `"ok": false`) and asks the user about it;
/// falls back to a generic "what should I focus on" question when no such
/// blocker is found (the model may have simply stopped with nothing to point
/// to).
fn build_trail_off_fallback(
    history: &[crate::openhuman::agent::messages::ConversationMessage],
) -> String {
    match last_builder_tool_blocker(history) {
        Some(blocker) => format!(
            "I wasn't able to finish building this workflow. Here's where I got stuck:\n\n{blocker}\n\n\
             Could you tell me how you'd like me to resolve that, or share more detail about what's needed here?"
        ),
        None => "I wasn't able to finish building this workflow in this turn. Could you describe \
                  what you'd like in more detail, or tell me which part to focus on?"
            .to_string(),
    }
}

/// Combines the guaranteed trail-off `fallback` question with the model's own
/// `original` text instead of discarding it (#4887 follow-up, Change 2). Even
/// after loosening `text_looks_like_question`, a future false negative must
/// never destroy the model's words — it should only ever ADD the guaranteed
/// question on top. The `fallback` is prepended (so the user sees the
/// actionable question first) and the original is kept below a divider for
/// context. When `original` is empty/whitespace-only (a genuine silent
/// turn — there's nothing to preserve), returns the fallback alone rather
/// than prepending an empty divider.
fn combine_trail_off_fallback(fallback: &str, original: &str) -> String {
    let trimmed_original = original.trim();
    if trimmed_original.is_empty() {
        fallback.to_string()
    } else {
        format!("{fallback}\n\n---\n\n{trimmed_original}")
    }
}

/// Scans `history` in reverse for the last result from a
/// [`TRAIL_OFF_BLOCKER_TOOLS`] call that reads as a failure — a plain-text
/// error message (gate rejection), or a JSON body with `"ok": false` — and
/// returns a truncated, human-readable description of it. Tool names are
/// resolved by correlating each `ToolResults` entry's `tool_call_id` back to
/// the `AssistantToolCalls` message that issued it, so this never
/// misattributes an unrelated read-only tool's plain-text output as a
/// blocker.
fn last_builder_tool_blocker(
    history: &[crate::openhuman::agent::messages::ConversationMessage],
) -> Option<String> {
    use crate::openhuman::agent::messages::ConversationMessage;

    let mut call_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for message in history {
        if let ConversationMessage::AssistantToolCalls { tool_calls, .. } = message {
            for call in tool_calls {
                call_names.insert(call.id.clone(), call.name.clone());
            }
        }
    }

    for message in history.iter().rev() {
        let ConversationMessage::ToolResults(results) = message else {
            continue;
        };
        for result in results.iter().rev() {
            let Some(name) = call_names.get(&result.tool_call_id) else {
                continue;
            };
            if !TRAIL_OFF_BLOCKER_TOOLS.contains(&name.as_str()) {
                continue;
            }
            // This is the MOST RECENT authoring-belt tool result in the
            // turn (results are scanned newest-first). Whatever it reads as
            // is authoritative: a success/progress result here means any
            // earlier failure from the same tool was already resolved
            // within this turn, so we must stop at this result rather than
            // keep walking backward and surfacing a stale, already-fixed
            // blocker (see review discussion on this PR).
            return describe_tool_result_blocker(&result.content)
                .map(|desc| crate::openhuman::util::truncate_with_ellipsis(&desc, 500));
        }
    }
    None
}

/// Reads one builder tool result's content as a failure description, or
/// `None` when it reads as success/progress (a `workflow_proposal` payload,
/// or an `"ok": true` report). The whole body is the description, never one
/// hardcoded field, so this stays correct regardless of which fields a given
/// tool uses to explain its failure.
fn describe_tool_result_blocker(content: &str) -> Option<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if value.get("type").and_then(Value::as_str) == Some("workflow_proposal") {
            return None; // Success: a proposal was emitted.
        }
        if let Some(ok) = value.get("ok").and_then(Value::as_bool) {
            return if ok { None } else { Some(value.to_string()) };
        }
        // Some other structured payload with no `ok`/`type` marker this
        // function recognises — not confidently a blocker, skip it.
        return None;
    }
    // Non-JSON content: a hard-gate rejection (`ToolResult::error`) puts the
    // plain error message straight into the content — since every builder
    // tool's SUCCESS shape is JSON (a proposal or a `{ ok, ... }` report), a
    // bare string here is, by elimination, an error message.
    Some(trimmed.to_string())
}

/// Scans an agent run's conversation history for the workflow proposal a builder
/// tool emitted. `propose_workflow` / `revise_workflow` / `save_workflow` all
/// return a self-describing `{ "type": "workflow_proposal", … }` JSON string as
/// their tool result, so we match on that (the same gate the frontend uses) and
/// return the LAST one — the most recent proposal in the turn.
fn extract_workflow_proposal(
    history: &[crate::openhuman::agent::messages::ConversationMessage],
) -> Option<Value> {
    use crate::openhuman::agent::messages::ConversationMessage;
    let mut latest = None;
    for message in history {
        if let ConversationMessage::ToolResults(results) = message {
            for result in results {
                if let Ok(value) = serde_json::from_str::<Value>(&result.content) {
                    if value.get("type").and_then(Value::as_str) == Some("workflow_proposal") {
                        latest = Some(value);
                    }
                }
            }
        }
    }
    latest
}

/// Lists persisted workflow suggestions. `status` filters to one lifecycle
/// state (the UI passes `New` for the active "Suggested for you" cards); `None`
/// returns every status.
pub async fn flows_list_suggestions(
    config: &Config,
    status: Option<SuggestionStatus>,
) -> Result<RpcOutcome<Vec<FlowSuggestion>>, String> {
    let suggestions = store::list_suggestions(config, status, 100).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(suggestions, "suggestions listed"))
}

/// Marks a suggestion `dismissed` (the user rejected the card). The row is kept
/// so a later discovery run dedupes against it and won't re-surface the idea.
pub async fn flows_dismiss_suggestion(
    config: &Config,
    id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let found = store::set_suggestion_status(config, id, SuggestionStatus::Dismissed)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({ "id": id, "dismissed": found }),
        "suggestion dismissed",
    ))
}

/// Marks a suggestion `built` — called by the frontend after the user saves a
/// flow authored from this suggestion, so it drops out of the active cards.
pub async fn flows_mark_suggestion_built(
    config: &Config,
    id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let found = store::set_suggestion_status(config, id, SuggestionStatus::Built)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({ "id": id, "built": found }),
        "suggestion marked built",
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// Connector onboarding (Phase 5, item 18) — which toolkits a graph needs
// ─────────────────────────────────────────────────────────────────────────────

/// The set of Composio toolkits currently connected (lowercased), derived from
/// the same picker source the node-config credential dropdown uses.
pub(crate) async fn connected_toolkits(config: &Config) -> std::collections::HashSet<String> {
    match flows_list_connections(config).await {
        Ok(outcome) => outcome
            .value
            .iter()
            .filter_map(|c| c.toolkit.as_deref())
            .map(|t| t.to_ascii_lowercase())
            .collect(),
        Err(e) => {
            tracing::warn!(target: "flows", error = %e, "[flows] connected_toolkits: could not list connections — treating all as unconnected");
            std::collections::HashSet::new()
        }
    }
}

/// The Composio toolkits a graph needs (from its `tool_call` slugs and any
/// `app_event` trigger), each tagged connected/missing — the data behind the
/// canvas/proposal "Connect <toolkit>" CTAs (audit Phase 5, item 18). Native
/// `oh:` tools and `http_request` nodes need no Composio connection and are
/// skipped.
pub async fn compute_required_connections(config: &Config, graph: &WorkflowGraph) -> Vec<Value> {
    use crate::openhuman::memory::sync::composio::providers::toolkit_from_slug;

    // Collect required toolkits (deduped, order-preserving).
    let mut required: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |tk: String| {
        let tk = tk.to_ascii_lowercase();
        if !tk.is_empty() && seen.insert(tk.clone()) {
            required.push(tk);
        }
    };

    for node in &graph.nodes {
        if node.kind == NodeKind::ToolCall {
            if let Some(slug) = node.config.get("slug").and_then(Value::as_str) {
                // Native OpenHuman tools (`oh:<name>`) need no connection.
                if slug.starts_with("oh:") {
                    continue;
                }
                if let Some(tk) = toolkit_from_slug(slug) {
                    push(tk.to_string());
                }
            }
        }
    }
    // An app_event trigger names its toolkit directly.
    if let Some(trigger) = graph.trigger() {
        if let Some(tk) = trigger.config.get("toolkit").and_then(Value::as_str) {
            push(tk.to_string());
        }
    }

    if required.is_empty() {
        return Vec::new();
    }

    let connected = connected_toolkits(config).await;
    required
        .into_iter()
        .map(|toolkit| {
            let status = if connected.contains(&toolkit) {
                "connected"
            } else {
                "missing"
            };
            json!({ "toolkit": toolkit, "status": status })
        })
        .collect()
}

/// RPC: compute the toolkits a candidate graph needs and their connected
/// status, so the canvas/proposal can render "Connect <toolkit>" CTAs.
pub async fn flows_required_connections(
    config: &Config,
    graph_json: Value,
) -> Result<RpcOutcome<Value>, String> {
    let graph = migrate_and_deserialize_graph(graph_json)?;
    let required = compute_required_connections(config, &graph).await;
    Ok(RpcOutcome::single_log(
        json!({ "required_connections": required }),
        "required connections computed",
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// Save-time approval manifest (consolidated pre-authorization card)
// ─────────────────────────────────────────────────────────────────────────────

/// Statically compute the "approval manifest" for a graph: every ApprovalGate
/// permission a run of this flow will prompt for, so the save+enable card can
/// ask for all of them in one shot instead of parking the run node-by-node.
///
/// Mirrors — never re-implements — the runtime gating in
/// `crate::openhuman::flows::tinyflows::caps` (`OpenHumanTools::invoke` /
/// `OpenHumanHttp` / `OpenHumanCode`) and `approval::gate`'s Workflow-origin
/// branch. Because Rule 2 (`enforce_side_effect_approval`) forces
/// `require_approval: true` onto every graph with outbound side-effect nodes,
/// a run parks on EVERY gated node that lacks `(flow_id, tool_name)` trust —
/// so the manifest is precisely "the trust keys a fully pre-authorized run
/// needs".
///
/// Entry `kind`s:
/// - `"approvable"` — will park; pre-approving `tool_name` clears it.
/// - `"blocked"` — the autonomy tier `Block`s the node's class outright
///   (`enforce_node_tier_gate` refuses before dispatch); NOT approvable from
///   the card — shown informationally so the user learns at save time, not
///   at run time.
/// - `"dynamic"` — the node's slug is an inline `=` expression resolved from
///   runtime data; its trust key is unknowable at save time and it stays
///   gated (best-effort disclosure).
/// - `"agent"` — an `agent` node with an `agent_ref` runs a full harness turn
///   whose inner tool calls cannot be enumerated statically; disclosed so the
///   card never over-promises "zero prompts".
///
/// Curated Composio Read actions are excluded entirely: `CommandClass::Read`
/// is `Allow` under every tier and the runtime skips the gate for them, so
/// listing them would request grants that are never checked.
pub async fn compute_approval_manifest(config: &Config, graph: &WorkflowGraph) -> Vec<Value> {
    use crate::openhuman::flows::tinyflows::caps::classify_composio_action_for_tier;
    use crate::openhuman::security::{CommandClass, GateDecision, SecurityPolicy};

    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);

    let mut entries: Vec<Value> = Vec::new();
    // Approvable/blocked rows dedupe on the trust key (`tool_name`) — two
    // nodes calling the same tool need one grant, so they get one row.
    let mut seen_tools: HashSet<String> = HashSet::new();

    let push_gated = |entries: &mut Vec<Value>,
                      seen_tools: &mut HashSet<String>,
                      node_id: &str,
                      tool_name: String,
                      label: String,
                      class: CommandClass| {
        if !seen_tools.insert(tool_name.clone()) {
            return;
        }
        let kind = if security.gate_decision(class) == GateDecision::Block {
            "blocked"
        } else {
            "approvable"
        };
        entries.push(json!({
            "kind": kind,
            "node_id": node_id,
            "tool_name": tool_name,
            "label": label,
            "class": format!("{class:?}"),
        }));
    };

    for node in &graph.nodes {
        match node.kind {
            NodeKind::HttpRequest => {
                let url = node
                    .config
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("HTTP request");
                push_gated(
                    &mut entries,
                    &mut seen_tools,
                    &node.id,
                    "flows_http_request".to_string(),
                    format!("Call {url}"),
                    CommandClass::Network,
                );
            }
            NodeKind::Code => {
                push_gated(
                    &mut entries,
                    &mut seen_tools,
                    &node.id,
                    "flows_code".to_string(),
                    "Run sandboxed code".to_string(),
                    CommandClass::Write,
                );
            }
            NodeKind::ToolCall => {
                let slug = node.config.get("slug").and_then(Value::as_str);
                match slug {
                    Some(s) if s.trim_start().starts_with('=') => {
                        tracing::debug!(
                            target: "flows",
                            node_id = %node.id,
                            "[flows] approval manifest: dynamic `=` slug — cannot pre-approve"
                        );
                        entries.push(json!({
                            "kind": "dynamic",
                            "node_id": node.id,
                            "label": "Tool chosen at run time",
                        }));
                    }
                    Some(s)
                        if s.starts_with(
                            crate::openhuman::flows::tinyflows::caps::NATIVE_TOOL_PREFIX,
                        ) =>
                    {
                        let tool_name = s
                            .trim_start_matches(
                                crate::openhuman::flows::tinyflows::caps::NATIVE_TOOL_PREFIX,
                            )
                            .trim()
                            .to_string();
                        if tool_name.is_empty() {
                            continue; // structurally invalid; validate rejects elsewhere
                        }
                        let args = node.config.get("args").cloned().unwrap_or(json!({}));
                        // Same classifier the runtime dispatch uses. Args may
                        // contain unresolved `=` bindings, so a classification
                        // error (unknown tool, etc.) degrades conservatively
                        // to Network — over-asking is safe, under-asking
                        // re-introduces the mid-run park this feature removes.
                        let class = crate::openhuman::runtime::node::ops::classify_tool_call(
                            config, &tool_name, &args,
                        )
                        .unwrap_or(CommandClass::Network);
                        push_gated(
                            &mut entries,
                            &mut seen_tools,
                            &node.id,
                            tool_name.clone(),
                            format!("Use tool {tool_name}"),
                            class,
                        );
                    }
                    Some(s) if !s.trim().is_empty() => {
                        let class = classify_composio_action_for_tier(s).await;
                        if class == CommandClass::Read {
                            // Curated read: runtime never gates it.
                            continue;
                        }
                        push_gated(
                            &mut entries,
                            &mut seen_tools,
                            &node.id,
                            s.to_string(),
                            format!("Use {s}"),
                            class,
                        );
                    }
                    _ => {}
                }
            }
            NodeKind::Agent
                if node
                    .config
                    .get("agent_ref")
                    .and_then(Value::as_str)
                    .is_some_and(|r| !r.trim().is_empty()) =>
            {
                entries.push(json!({
                    "kind": "agent",
                    "node_id": node.id,
                    "label": "AI step — may ask for permission for its own actions",
                }));
            }
            _ => {}
        }
    }

    tracing::debug!(
        target: "flows",
        entries = entries.len(),
        "[flows] approval manifest computed"
    );
    entries
}

/// RPC: the approval manifest for a saved flow (by `id`) or a candidate
/// `graph`, joined against the flow's existing `flow_tool_trust` grants so
/// the save+enable card can ask only for what's missing.
///
/// With the approval gate uninstalled (`OPENHUMAN_APPROVAL_GATE=0`) nothing
/// ever parks, so `missing` is empty by definition and the card never shows.
pub async fn flows_approval_manifest(
    config: &Config,
    id: Option<&str>,
    graph_json: Option<Value>,
) -> Result<RpcOutcome<Value>, String> {
    tracing::debug!(target: "flows", id = ?id, has_graph = graph_json.is_some(), "[flows] flows_approval_manifest: entry");
    let (graph, flow_id) = match (id, graph_json) {
        (Some(id), _) => {
            let flow = store::get_flow(config, id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("flow not found: {id}"))?;
            // `store::get_flow` already returns a migrated, deserialized graph.
            (flow.graph, Some(id.to_string()))
        }
        (None, Some(graph_json)) => (migrate_and_deserialize_graph(graph_json)?, None),
        (None, None) => return Err("provide 'id' or 'graph'".to_string()),
    };

    let entries = compute_approval_manifest(config, &graph).await;

    let gate = crate::openhuman::security::approval::ApprovalGate::try_global();
    let gate_installed = gate.is_some();
    let trusted: HashSet<String> = match (&gate, &flow_id) {
        (Some(gate), Some(flow_id)) => gate
            .list_flow_trust(flow_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect(),
        _ => HashSet::new(),
    };

    let mut missing: Vec<String> = Vec::new();
    let mut already_trusted: Vec<String> = Vec::new();
    for entry in &entries {
        if entry.get("kind").and_then(Value::as_str) != Some("approvable") {
            continue;
        }
        let Some(tool_name) = entry.get("tool_name").and_then(Value::as_str) else {
            continue;
        };
        if !gate_installed {
            // Nothing parks without a gate; report nothing as missing.
            already_trusted.push(tool_name.to_string());
        } else if trusted.contains(tool_name) {
            already_trusted.push(tool_name.to_string());
        } else {
            missing.push(tool_name.to_string());
        }
    }

    let log = format!(
        "[flows] approval manifest: {} entr{}, {} missing grant(s)",
        entries.len(),
        if entries.len() == 1 { "y" } else { "ies" },
        missing.len()
    );
    tracing::debug!(target: "flows", entries = entries.len(), missing = missing.len(), gate_installed, "[flows] flows_approval_manifest: exit");
    Ok(RpcOutcome::single_log(
        json!({
            "entries": entries,
            "missing": missing,
            "already_trusted": already_trusted,
            "gate_installed": gate_installed,
        }),
        log,
    ))
}

// ─────────────────────────────────────────────────────────────────────────────
// Catalog RPCs for the UI (Phase 5, item 16) — one implementation, two consumers
// ─────────────────────────────────────────────────────────────────────────────

/// Searches the live Composio tool catalog (secret-free) — the RPC the in-canvas
/// tool browser calls, reusing the exact same core as the agent's
/// `search_tool_catalog` tool so the two can't drift.
pub async fn flows_search_tool_catalog(
    config: &Config,
    query: &str,
    toolkit: Option<&str>,
    limit: usize,
) -> Result<RpcOutcome<Value>, String> {
    tracing::debug!(target: "flows", %query, toolkit = toolkit.unwrap_or("<all>"), "[flows] flows_search_tool_catalog: searching live catalog");
    let tools =
        crate::openhuman::flows::builder_tools::search_live_catalog(config, query, toolkit, limit)
            .await;
    Ok(RpcOutcome::single_log(
        json!({ "tools": tools }),
        "tool catalog searched",
    ))
}

/// Fetches one Composio action's full contract (secret-free) — the RPC the
/// canvas tool browser calls to fill in an action's arg schema, reusing the same
/// core as the agent's `get_tool_contract` tool.
pub async fn flows_get_tool_contract(
    config: &Config,
    slug: &str,
) -> Result<RpcOutcome<Value>, String> {
    let slug = slug.trim();
    let Some(toolkit) =
        crate::openhuman::memory::sync::composio::providers::toolkit_from_slug(slug)
    else {
        return Err(format!(
            "Could not extract a toolkit from slug '{slug}' — it must look like \
             '<TOOLKIT>_<ACTION>' (e.g. 'GMAIL_SEND_EMAIL')."
        ));
    };
    tracing::debug!(target: "flows", %slug, %toolkit, "[flows] flows_get_tool_contract: fetching contract");
    let Some(catalog) =
        crate::openhuman::flows::tinyflows::caps::fetch_live_toolkit_catalog(config, &toolkit)
            .await
    else {
        return Err(format!(
            "Could not fetch the live Composio catalog for toolkit '{toolkit}'."
        ));
    };
    match catalog.iter().find(|c| c.slug.eq_ignore_ascii_case(slug)) {
        Some(contract) => {
            let contract =
                crate::openhuman::flows::tinyflows::caps::apply_probe_override(contract.clone());
            let value = serde_json::to_value(&contract).map_err(|e| e.to_string())?;
            Ok(RpcOutcome::single_log(
                json!({ "contract": value }),
                "tool contract fetched",
            ))
        }
        None => Err(format!(
            "'{slug}' is not a real action in the '{toolkit}' toolkit's live catalog."
        )),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Core-managed local drafts (F5) — the shared agent/canvas working copy
// ─────────────────────────────────────────────────────────────────────────────

/// Creates a new draft (a durable, non-live working copy) from a graph.
pub fn flows_draft_create(
    config: &Config,
    flow_id: Option<String>,
    name: String,
    graph: Value,
    origin: crate::openhuman::flows::DraftOrigin,
) -> Result<RpcOutcome<crate::openhuman::flows::FlowDraft>, String> {
    let draft = draft_store::create_draft(config, flow_id, name, graph, origin)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(draft, "draft created"))
}

/// Reads a draft by id (errors if it does not exist).
pub fn flows_draft_get(
    config: &Config,
    id: &str,
) -> Result<RpcOutcome<crate::openhuman::flows::FlowDraft>, String> {
    let draft = draft_store::get_draft(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("draft '{id}' not found"))?;
    Ok(RpcOutcome::single_log(draft, format!("draft loaded: {id}")))
}

/// Patches a draft's `name`/`graph`/`flow_id` (any `Some` applied) and bumps
/// `updated_at`.
pub fn flows_draft_update(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph: Option<Value>,
    flow_id: Option<Option<String>>,
) -> Result<RpcOutcome<crate::openhuman::flows::FlowDraft>, String> {
    let draft =
        draft_store::update_draft(config, id, name, graph, flow_id).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(draft, "draft updated"))
}

/// Lists all drafts, newest-updated first.
pub fn flows_draft_list(
    config: &Config,
) -> Result<RpcOutcome<Vec<crate::openhuman::flows::FlowDraft>>, String> {
    let drafts = draft_store::list_drafts(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(drafts, "drafts listed"))
}

/// Deletes a draft by id (idempotent — reports whether a file was removed).
pub fn flows_draft_delete(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    let deleted = draft_store::delete_draft(config, id).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        json!({ "id": id, "deleted": deleted }),
        "draft deleted",
    ))
}

/// Promotes a draft into a saved flow, then removes the draft file.
///
/// Runs the SAME create/update gates as a normal save (structural validation,
/// the forced `require_approval` floor for side-effect graphs, born-disabled
/// for automatic triggers) — a draft is never a back-door around them. A draft
/// with a `flow_id` updates that flow; otherwise it creates a new one. The
/// draft file is deleted only on a successful promote.
pub async fn flows_draft_promote(
    config: &Config,
    id: &str,
    require_approval: Option<bool>,
) -> Result<RpcOutcome<Flow>, String> {
    let draft = draft_store::get_draft(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("draft '{id}' not found"))?;

    tracing::debug!(
        target: "flows",
        draft_id = %id,
        promotes_to = draft.flow_id.as_deref().unwrap_or("<new flow>"),
        "[flows] flows_draft_promote: promoting draft through the create/update gates"
    );

    let outcome = match &draft.flow_id {
        Some(flow_id) => {
            flows_update(
                config,
                flow_id,
                Some(draft.name.clone()),
                Some(draft.graph.clone()),
                require_approval,
                None,
            )
            .await?
        }
        None => {
            flows_create(
                config,
                draft.name.clone(),
                draft.graph.clone(),
                require_approval.unwrap_or(false),
            )
            .await?
        }
    };

    // Only remove the draft once the flow write succeeded.
    if let Err(e) = draft_store::delete_draft(config, id) {
        tracing::warn!(target: "flows", draft_id = %id, error = %e, "[flows] flows_draft_promote: flow saved but draft file could not be removed");
    }
    Ok(outcome)
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
