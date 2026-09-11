use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tinyflows::model::{NodeKind, TriggerKind, WorkflowGraph};
// The save/run safety predicates are `tinyflows-catalog`'s: whether a graph
// fires unattended, whether it can act on the world, whether it has anything to
// do at all are properties of the graph, not of this host. Re-exported at
// `ops::` scope because the agent tools and this module's tests already name
// them there.
pub(crate) use tinyflows_catalog::graph_policy::{
    enforce_side_effect_approval, graph_has_actionable_nodes, trigger_is_automatic,
};
// Reached only by this module's own tests, which assert the host resolves the
// predicate the two save rules are built on — `enforce_side_effect_approval`
// is what production calls.
#[cfg(test)]
pub(crate) use tinyflows_catalog::graph_policy::graph_has_outbound_side_effect;
use tokio_util::sync::CancellationToken;

use crate::openhuman::agent::turn_origin::{with_origin, AgentTurnOrigin, TrustedAutomationSource};
use crate::openhuman::config::Config;
use crate::openhuman::flows::bus;
use crate::openhuman::flows::draft_store;
use crate::openhuman::flows::run_registry;
use crate::openhuman::flows::store;
use crate::openhuman::flows::types::{
    FlowConnection, FlowRunStep, FlowRunTrigger, FlowSuggestion, SuggestionStatus,
};
use crate::openhuman::flows::{flow_namespace, Flow, FlowRun};
use crate::openhuman::security::approval::{
    ApprovalChatContext, FlowRunContext, APPROVAL_CHAT_CONTEXT, APPROVAL_COPILOT_STREAM_CONTEXT,
    APPROVAL_FLOW_RUN_CONTEXT,
};
use crate::rpc::RpcOutcome;
use tinyflows_catalog::build_registry;
// `MemoryProvider` brings `driver_id()` / `as_documents()` into scope for the
// `MemoryGuard` this file's delete path clears through. Nothing here names the
// engine crate any more — `flows_delete_impl`'s test seam took an
// `Arc<MemoryClient>` until #5560 and takes the guard now.
use tinymemory_api::provider::MemoryProvider;

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
    tinyflows::compat::ensure_compatible(&graph)?;
    Ok(graph)
}

/// Every engine-incompatible topology in `graph`, mapped onto this domain's
/// validation-error shape.
///
/// The classification is [`tinyflows::compat`]'s, and belongs there: which
/// fan-in shapes the engine's barrier relief can execute is a fact about the
/// engine, not about OpenHuman. This host used to carry the whole walk.
pub(crate) fn engine_compatibility_errors(
    graph: &WorkflowGraph,
) -> Vec<crate::openhuman::flows::FlowValidationError> {
    tinyflows::compat::errors(graph)
        .into_iter()
        .map(to_compat_validation_error)
        .collect()
}

/// Same walk, with the inline-nesting budget passed in rather than recomputed.
///
/// [`referenced_workflow_compatibility_errors`] needs this: a saved child
/// reached partway through the root's referenced-workflow chain must still be
/// checked to the *remaining* depth the root allows. The engine's runtime depth
/// counter is one budget shared across the whole inline-plus-referenced chain,
/// so a fan-in the child's own cap would not reach can still be reached from
/// the root.
pub(crate) fn engine_compatibility_errors_with_max_depth(
    graph: &WorkflowGraph,
    max_depth: u64,
) -> Vec<crate::openhuman::flows::FlowValidationError> {
    tinyflows::compat::errors_with_max_depth(graph, max_depth)
        .into_iter()
        .map(to_compat_validation_error)
        .collect()
}

/// The nesting cap `graph` declares on its trigger, or the engine default.
pub(crate) fn max_sub_workflow_depth(graph: &WorkflowGraph) -> u64 {
    tinyflows::compat::max_sub_workflow_depth(graph)
}

// The two refusal codes are `tinyflows::compat`'s, re-exported at `ops::` scope
// because this module's tests assert on them by name — which is the point of a
// stable code, and what keeps a rename upstream a compile error here rather
// than a silently-passing `contains`.
#[cfg(test)]
pub(crate) use tinyflows::compat::{
    UNSUPPORTED_MAIN_PORT_CONDITIONAL_FAN_IN, UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN,
};

fn to_compat_validation_error(
    error: tinyflows::compat::CompatibilityError,
) -> crate::openhuman::flows::FlowValidationError {
    crate::openhuman::flows::FlowValidationError {
        code: error.code.to_string(),
        message: error.message,
        node_id: error.node_id,
        field: None,
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

/// Refuses a graph whose outbound `tool_call` arguments a sandbox run proves
/// can never carry a value.
///
/// Delegates to [`tinyflows::preflight::unresolvable_tool_args`]; the whole
/// analysis is the engine's, because the mock run, the trigger-scope rule and
/// the opaque-upstream rule are all statements about the DSL. What this host
/// contributes is the one thing the crate cannot know: which slug prefix marks
/// a tool of *ours*, which has no external provider to reject the call and so
/// is skipped.
// Named at `ops::` scope because this module's tests already reach it there,
// and they are what proves this host's native-slug prefix reaches the gate.
#[cfg(test)]
pub(crate) use tinyflows::preflight::mock_opaque_tool_call_upstream_ref;

pub(crate) async fn validate_required_arg_resolvability(graph: &WorkflowGraph) -> Vec<String> {
    tinyflows::preflight::unresolvable_tool_args(
        graph,
        &[crate::openhuman::flows::tinyflows::caps::NATIVE_TOOL_PREFIX],
    )
    .await
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
