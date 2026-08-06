//! Backs the medulla harness protocol's workflow plane
//! ([`crate::openhuman::platform::socket::medulla::workflows::WorkflowBridge`]) with this
//! host's own `flows::` store.
//!
//! The transport half is host-agnostic on purpose — it moves adverts, opaque
//! JSON and one authoring turn, and parses no graph. This module is the
//! openhuman half: it projects saved [`Flow`]s onto the wire's
//! [`WorkflowDescriptor`], answers the three reads out of [`super::ops`], and
//! runs the `workflow_builder` agent for a `copilot` turn.
//!
//! **Why the copilot persists here.** In the desktop product a builder turn is
//! deliberately propose-only: [`super::ops::flows_build`] returns a candidate
//! graph and the *user* accepts it in the copilot panel, then saves it on the
//! canvas. On the medulla plane there is no panel and no user — a remote
//! orchestrator asked for a change and is waiting on the answer — so the accept
//! is performed here, by the host, from the proposal the turn produced. Two
//! guards keep that safe rather than merely convenient:
//!
//! - a **create** is persisted with `require_approval: true` unconditionally, so
//!   every outbound action of a remotely-authored workflow still parks for a
//!   human decision at run time;
//! - an **update** passes `require_approval: None`, so a remote turn can raise
//!   a workflow's approval requirement (via `flows_update`'s own side-effect
//!   enforcement) but can never lower one the user set.
//!
//! `flows_create` additionally saves any automatic-trigger graph **disabled**,
//! so nothing authored over this plane can start firing on its own.
//!
//! **Why the outcome is diffed, not reported.** [`WorkflowBridge::copilot`]'s
//! `changes` are computed by [`diff_workflow`] over the workflow this turn
//! successfully persisted, using listings taken before and after the turn —
//! never from what the agent said it did. Scoping the diff matters because a
//! copilot turn can run for minutes while a user edits unrelated workflows.
//! That is the whole reason medulla-v1 forwards the outcome verbatim instead of
//! re-deriving it: the claim that reaches the orchestrator is a statement about
//! the store, made by the store's owner.
//!
//! **Threading.** The three reads are synchronous on the trait but every
//! `flows::` op is `async`, so they bridge with a `Handle::block_on` — legal
//! because the transport only ever calls them from `spawn_blocking` (see
//! [`super::super::platform::socket::medulla::workflows`]), never on a runtime worker.
//! `copilot` is `async` end to end and needs no hop.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::model::WorkflowGraph;

use super::agents::workflow_builder::builder_prompt::{BuildMode, BuilderRequest};
use super::node_contracts::{all_node_kind_contracts, node_kind_contract, NODE_KINDS};
use super::ops;
use super::types::Flow;
use crate::openhuman::config::Config;
use crate::openhuman::platform::socket::medulla::payloads::{
    CopilotOutcome, WorkflowDescriptor, WorkflowInputDescriptor,
};
use crate::openhuman::platform::socket::medulla::workflows::{set_workflow_bridge, WorkflowBridge};

/// How many runs a `runs` read returns. The orchestrator wants "did this
/// workflow work lately", not an audit log, and the server caps a result at
/// 1 MiB — a bounded window answers the question and can never hit that cap.
const RUN_HISTORY_LIMIT: usize = 20;

/// Cap on a generated advert description. It is prompt surface rendered for
/// every workflow on every review pass, so a 40-node graph must not crowd out
/// the rest of the section.
const MAX_DESCRIPTION_CHARS: usize = 400;

/// Persistence tools hidden from a medulla copilot turn.
///
/// The generic builder exposes these for its product surfaces, but this bridge
/// deliberately owns persistence in [`apply_proposal`] so it can enforce the
/// observed version and approval floor exactly once.
const MEDULLA_COPILOT_HIDDEN_TOOLS: &[&str] = &[
    "save_workflow",
    "create_workflow",
    "duplicate_flow",
    "edit_workflow",
];

/// Install the flows-backed bridge on the medulla workflow plane.
///
/// Called once at boot, gated on the `flows` domain being live. Installing also
/// advertises immediately, so a core that connected before this ran still
/// publishes its workflows.
pub fn install(config: Arc<Config>) {
    log::debug!("[flows] installing the medulla workflow bridge over the flows store");
    set_workflow_bridge(Arc::new(FlowsWorkflowBridge::pinned(config)));
}

/// The `flows::`-backed [`WorkflowBridge`].
///
/// Holds no store handle: `flows::` is addressed by [`Config`], and the RPC
/// surface already re-reads the config per call rather than caching a snapshot
/// that a workspace switch would invalidate. `config` is `Some` only when a
/// caller pinned one (tests, and embedders that already hold a config).
pub struct FlowsWorkflowBridge {
    config: Option<Arc<Config>>,
}

impl FlowsWorkflowBridge {
    /// A bridge that resolves the process's current config on every call.
    #[must_use]
    pub fn new() -> Self {
        Self { config: None }
    }

    /// A bridge pinned to one config, for callers that already hold one and for
    /// tests, which must address a temp workspace rather than the real one.
    #[must_use]
    pub fn pinned(config: Arc<Config>) -> Self {
        Self {
            config: Some(config),
        }
    }

    /// The config to address the store with, loaded if not pinned.
    async fn config(&self) -> Result<Arc<Config>, String> {
        match &self.config {
            Some(config) => Ok(Arc::clone(config)),
            None => crate::openhuman::config::rpc::load_config_with_timeout()
                .await
                .map(Arc::new)
                .map_err(|err| format!("this agent could not read its config: {err}")),
        }
    }

    /// Run one `flows::` op to completion from a synchronous trait method.
    ///
    /// Only legal off the runtime's async threads. The transport guarantees that
    /// — every sync bridge call is already inside `spawn_blocking` — and a
    /// caller that violates it panics there rather than deadlocking, which that
    /// same `spawn_blocking` converts into a reported error.
    fn on_store<T, F>(&self, run: impl FnOnce(Arc<Config>) -> F) -> Result<T, String>
    where
        F: std::future::Future<Output = Result<T, String>>,
    {
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| "this agent's workflow store has no runtime to answer on".to_string())?;
        handle.block_on(async move {
            let config = self.config().await?;
            run(config).await
        })
    }
}

impl Default for FlowsWorkflowBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WorkflowBridge for FlowsWorkflowBridge {
    fn action_dir(&self) -> Option<String> {
        self.config
            .as_ref()
            .map(|config| config.action_dir.display().to_string())
    }

    fn list(&self) -> Vec<WorkflowDescriptor> {
        match self.try_list() {
            Ok(descriptors) => descriptors,
            Err(err) => {
                log::warn!("[flows] medulla bridge could not list workflows: {err}");
                Vec::new()
            }
        }
    }

    fn try_list(&self) -> Result<Vec<WorkflowDescriptor>, String> {
        self.on_store(|config| async move {
            ops::flows_list(&config)
                .await
                .map(|outcome| outcome.value)
                .map(|flows| flows.iter().map(describe_flow).collect::<Vec<_>>())
        })
    }

    fn get(&self, id: &str) -> Result<Value, String> {
        let id = id.to_string();
        self.on_store(|config| async move {
            let flow = ops::flows_get(&config, &id).await?.value;
            serde_json::to_value(&flow)
                .map_err(|err| format!("failed to render workflow '{id}': {err}"))
        })
    }

    fn node_kinds(&self, kind: Option<&str>) -> Result<Value, String> {
        // Purely static host+engine metadata — no store, no config, no runtime.
        match kind.map(str::trim).filter(|kind| !kind.is_empty()) {
            Some(kind) => {
                let contract = node_kind_contract(kind).ok_or_else(|| {
                    format!(
                        "'{kind}' is not a node kind here; the kinds are: {}",
                        NODE_KINDS.join(", ")
                    )
                })?;
                Ok(json!({ "kinds": [node_kind_json(&contract)] }))
            }
            None => Ok(json!({
                "kinds": all_node_kind_contracts()
                    .iter()
                    .map(node_kind_json)
                    .collect::<Vec<_>>(),
            })),
        }
    }

    fn runs(&self, id: &str) -> Result<Value, String> {
        let id = id.to_string();
        self.on_store(|config| async move {
            let runs = ops::flows_list_runs(&config, &id, RUN_HISTORY_LIMIT)
                .await?
                .value;
            Ok(json!({ "runs": runs.iter().map(run_json).collect::<Vec<_>>() }))
        })
    }

    async fn copilot(
        &self,
        instruction: &str,
        workflow_id: Option<&str>,
    ) -> Result<CopilotOutcome, String> {
        let config = self.config().await?;
        let before = ops::flows_list(&config).await?.value;
        let request = builder_request(instruction, workflow_id, &before)?;
        let expected_version = workflow_id.and_then(|id| {
            before
                .iter()
                .find(|flow| flow.id == id)
                .map(|flow| flow.updated_at.as_str())
        });

        // Headless: no chat thread to stream into, so the turn runs under the
        // CLI origin with the full run-advancing hide-list — a remote authoring
        // turn proposes a graph, it never runs one.
        let built = ops::flows_build_with_extra_hidden_tools(
            &config,
            request,
            None,
            MEDULLA_COPILOT_HIDDEN_TOOLS,
        )
        .await?
        .value;
        let mut reply = built
            .get("assistant_text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let mut applied = None;
        if let Some(proposal) = built.get("proposal").filter(|value| !value.is_null()) {
            match apply_proposal(&config, workflow_id, proposal, expected_version).await {
                Ok(saved) => applied = Some(saved),
                // The turn happened and its reply is worth returning; only the
                // save failed. Say so in the reply rather than discarding the
                // whole outcome — and let the diff below report (truthfully)
                // that nothing changed.
                Err(err) => {
                    log::warn!("[flows] medulla copilot could not persist its proposal: {err}");
                    reply = format!("{reply}\n\nThe proposed graph could not be saved: {err}");
                }
            }
        }

        let changes = applied
            .as_ref()
            .map(|saved| diff_workflow(&before, std::slice::from_ref(&saved.flow), &saved.flow.id))
            .unwrap_or_default();
        let created = applied
            .filter(|saved| saved.created)
            .map(|saved| saved.flow.id);
        Ok(CopilotOutcome {
            reply,
            changes,
            created,
        })
    }
}

/// Project one saved flow onto the advert the orchestrator reasons over.
///
/// `name`/`description` are left as plain `String`s: the wire type skips them
/// when empty, so a nameless flow omits the key rather than sending `""`.
fn describe_flow(flow: &Flow) -> WorkflowDescriptor {
    WorkflowDescriptor {
        id: flow.id.clone(),
        name: flow.name.trim().to_string(),
        description: describe_graph(&flow.graph),
        node_count: u32::try_from(flow.graph.nodes.len()).unwrap_or(u32::MAX),
        enabled: Some(flow.enabled),
        trigger_kind: trigger_kind(&flow.graph),
        // Provenance is the backend's to stamp: this host runs one identity, so
        // the batch-level `agentId` on the registration frame is the whole
        // answer and a per-advert claim would only be redundant.
        agent_id: None,
        workspace_id: None,
        inputs: describe_inputs(&flow.graph),
    }
}

/// Projects the graph's declared inputs onto the wire descriptors.
///
/// The translation exists because the medulla plane's payload module is
/// compiled in every build while the engine is behind the `flows` gate — see
/// [`WorkflowInputDescriptor`]'s doc. This side is gated, so it may name the
/// engine type.
fn describe_inputs(graph: &WorkflowGraph) -> Vec<WorkflowInputDescriptor> {
    graph
        .inputs
        .iter()
        .map(|input| WorkflowInputDescriptor {
            name: input.name.clone(),
            ty: input.ty.as_str().to_string(),
            description: input.description.clone().unwrap_or_default(),
            required: input.required,
            default: input.default.clone(),
        })
        .collect()
}

/// The open-vocabulary trigger label, when the graph has exactly one trigger
/// that names its kind. `None` covers "no trigger", "several triggers", and a
/// trigger with no `trigger_kind` — all cases where a claim would be a guess.
fn trigger_kind(graph: &WorkflowGraph) -> Option<String> {
    graph
        .trigger()?
        .config
        .get("trigger_kind")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// A one-line prose description of what a graph does, for the advert.
///
/// A flow has no author-written description field, so it is generated from the
/// graph's own step names: `"<trigger> → step → step"`. That is the same
/// information the Workflows list renders, written for a prompt instead of a
/// row, and truncated so one large graph cannot dominate the section.
fn describe_graph(graph: &WorkflowGraph) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(match trigger_kind(graph) {
        Some(kind) => format!("on {kind}"),
        None => "no trigger".to_string(),
    });
    parts.extend(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind != tinyflows::model::NodeKind::Trigger)
            .map(|node| node.name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string),
    );

    let rendered = parts.join(" → ");
    if rendered.chars().count() <= MAX_DESCRIPTION_CHARS {
        return rendered;
    }
    let truncated: String = rendered.chars().take(MAX_DESCRIPTION_CHARS).collect();
    format!("{truncated}…")
}

/// Project a node-kind contract onto the `WorkflowNodeKind` shape medulla-v1's
/// port declares — camelCase, because the backend spreads this object straight
/// onto the port type and a snake_case key would land beside the field it was
/// meant to be rather than in it.
fn node_kind_json(contract: &crate::openhuman::flows::node_contracts::NodeKindContract) -> Value {
    json!({
        "kind": contract.kind,
        "summary": contract.summary,
        "description": contract.description,
        "configFields": contract.config_fields,
        "ports": contract.ports,
        "example": contract.example,
        "notes": contract.notes,
    })
}

/// Project a run row onto the `WorkflowRunSummary` shape the port declares.
///
/// Timestamps cross as epoch milliseconds because that is what the port's
/// `startedAt`/`finishedAt` are; the store keeps RFC3339, and an unparseable
/// stamp is omitted rather than sent as `0` (which reads as 1970, not "unknown").
/// The per-node `steps` are dropped: they are the run *inspector's* payload, and
/// a delegated-workflow read only asks whether it worked.
fn run_json(run: &super::types::FlowRun) -> Value {
    let mut value = json!({
        "id": run.id,
        "workflowId": run.flow_id,
        "status": run.status,
    });
    if let Some(started) = epoch_millis(&run.started_at) {
        value["startedAt"] = json!(started);
    }
    if let Some(finished) = run.finished_at.as_deref().and_then(epoch_millis) {
        value["finishedAt"] = json!(finished);
    }
    if let Some(error) = run.error.as_deref().filter(|error| !error.is_empty()) {
        value["error"] = json!(error);
    }
    value
}

/// RFC3339 → epoch milliseconds, or `None` for a stamp this build cannot read.
fn epoch_millis(stamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(stamp)
        .ok()
        .map(|parsed| parsed.timestamp_millis())
}

/// Build the authoring-turn request for one copilot call.
///
/// Naming a workflow is a **revise** of that exact graph, so the turn is briefed
/// with the saved graph and its id; naming none is a **create**. An id the store
/// does not hold fails here rather than silently degrading into a create — the
/// orchestrator asked to change something specific, and creating a second
/// workflow instead would be the wrong kind of surprising.
fn builder_request(
    instruction: &str,
    workflow_id: Option<&str>,
    flows: &[Flow],
) -> Result<BuilderRequest, String> {
    let (mode, graph, flow_id) = match workflow_id {
        Some(id) => {
            let flow = flows
                .iter()
                .find(|flow| flow.id == id)
                .ok_or_else(|| format!("workflow '{id}' is not in this agent's store"))?;
            let graph = serde_json::to_value(&flow.graph)
                .map_err(|err| format!("failed to read workflow '{id}': {err}"))?;
            (BuildMode::Revise, Some(graph), Some(id.to_string()))
        }
        None => (BuildMode::Create, None, None),
    };
    Ok(BuilderRequest {
        mode,
        instruction: instruction.to_string(),
        graph,
        flow_id,
        run_id: None,
        error: None,
        failing_node_ids: Vec::new(),
    })
}

/// The exact row committed by [`apply_proposal`].
#[derive(Debug)]
struct AppliedProposal {
    flow: Flow,
    created: bool,
}

/// Persist the graph a builder turn proposed, returning the committed row.
///
/// See the module docs for why this host performs the accept the copilot panel
/// would otherwise perform, and for the two approval guards that bound it.
async fn apply_proposal(
    config: &Config,
    workflow_id: Option<&str>,
    proposal: &Value,
    expected_version: Option<&str>,
) -> Result<AppliedProposal, String> {
    let graph = proposal
        .get("graph")
        .filter(|graph| !graph.is_null())
        .ok_or_else(|| "the copilot proposed no graph".to_string())?
        .clone();

    match workflow_id {
        // `require_approval: None` keeps whatever the user set. A remote turn
        // may still have it forced ON by `flows_update`'s side-effect check; it
        // can never turn it off.
        Some(id) => {
            let name = proposal
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string);
            let flow = ops::flows_update_disarming_automatic(
                config,
                id,
                name,
                Some(graph),
                None,
                expected_version.map(str::to_string),
            )
            .await?
            .value;
            Ok(AppliedProposal {
                flow,
                created: false,
            })
        }
        None => {
            let name = proposal
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or("Untitled workflow")
                .to_string();
            // `true`, not the proposal's own value: nothing authored by a remote
            // instruction acts outward without a human decision at run time.
            let flow = ops::flows_create(config, name, graph, true).await?.value;
            Ok(AppliedProposal {
                flow,
                created: true,
            })
        }
    }
}

/// What actually changed in the store across a copilot turn, as one line per
/// change.
///
/// Derived from two listings, never from the agent's account of its own turn —
/// see the module docs. A turn that talked and saved nothing yields an empty
/// list, which is exactly the claim it earns.
fn diff_flows(before: &[Flow], after: &[Flow]) -> Vec<String> {
    let mut changes = Vec::new();

    for flow in after {
        let Some(previous) = before.iter().find(|candidate| candidate.id == flow.id) else {
            changes.push(format!(
                "created workflow \"{}\" ({}) with {} node(s)",
                flow.name,
                flow.id,
                flow.graph.nodes.len()
            ));
            continue;
        };
        if previous.name != flow.name {
            changes.push(format!(
                "renamed workflow {} from \"{}\" to \"{}\"",
                flow.id, previous.name, flow.name
            ));
        }
        if previous.graph != flow.graph {
            changes.push(format!(
                "rewrote the graph of workflow \"{}\" ({}): {} node(s) → {} node(s)",
                flow.name,
                flow.id,
                previous.graph.nodes.len(),
                flow.graph.nodes.len()
            ));
        }
        if previous.enabled != flow.enabled {
            let state = if flow.enabled { "enabled" } else { "disabled" };
            changes.push(format!("{state} workflow \"{}\" ({})", flow.name, flow.id));
        }
        if previous.require_approval != flow.require_approval {
            let state = if flow.require_approval {
                "now requires"
            } else {
                "no longer requires"
            };
            changes.push(format!(
                "workflow \"{}\" ({}) {state} approval for outbound actions",
                flow.name, flow.id
            ));
        }
    }

    for flow in before {
        if !after.iter().any(|candidate| candidate.id == flow.id) {
            changes.push(format!("deleted workflow \"{}\" ({})", flow.name, flow.id));
        }
    }

    changes
}

/// Diff only the workflow this copilot turn successfully persisted.
///
/// A turn may overlap arbitrary user edits elsewhere in the store. Those edits
/// are real, but they are not attributable to this turn and must not be claimed
/// in its result.
fn diff_workflow(before: &[Flow], after: &[Flow], workflow_id: &str) -> Vec<String> {
    let before: Vec<Flow> = before
        .iter()
        .filter(|flow| flow.id == workflow_id)
        .cloned()
        .collect();
    let after: Vec<Flow> = after
        .iter()
        .filter(|flow| flow.id == workflow_id)
        .cloned()
        .collect();
    diff_flows(&before, &after)
}

#[cfg(test)]
#[path = "medulla_bridge_tests.rs"]
mod tests;
