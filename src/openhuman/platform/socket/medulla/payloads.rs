//! Wire payloads for the medulla "harness plane" — the `medulla:task_*`
//! Socket.IO protocol that lets a medulla operator (running in the backend)
//! drive an OpenHuman agent session as a delegated sub-agent.
//!
//! See `docs/specs/session-streaming-api-spec.md` §6 in the medulla repo. All
//! payloads are camelCase on the wire to match the backend's Socket.IO
//! conventions (the harness envelope they carry stays snake_case).

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Down: backend / medulla → openhuman agent
// ─────────────────────────────────────────────────────────────────────────────

/// `medulla:task_run` — start a task in an openhuman agent session.
///
/// Creates (or resumes, when `session_id` is supplied) a session and sends
/// `instruction` as the opening prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRun {
    pub task_id: String,
    pub cycle_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub instruction: String,
    /// Which openhuman agent to run the task as (defaults to the orchestrator).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Hard wall-clock budget for the whole task, in milliseconds.
    #[serde(default)]
    pub timeout_ms: u64,
}

/// `medulla:task_send` — mid-task steering (answer a question / approval
/// decision / follow-up); `input` is delivered into the running session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSend {
    pub task_id: String,
    pub input: String,
}

/// `medulla:task_abort` — cancel the session/task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAbort {
    pub task_id: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Up: openhuman agent → backend / medulla
// ─────────────────────────────────────────────────────────────────────────────

/// `medulla:task_envelope` — one live-stream frame for a task, carrying a
/// native Medulla harness envelope (see [`super::envelope`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEnvelope {
    pub task_id: String,
    /// A serialized [`super::envelope::HarnessEnvelope`].
    pub envelope: serde_json::Value,
}

/// `medulla:task_result` — explicit completion (preferred over idle-detection).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub task_id: String,
    pub ok: bool,
    #[serde(default)]
    pub reply: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A single agent descriptor advertised in the roster.
///
/// The identity key is `id`, NOT `agentId`: the backend's roster registry keys
/// and validates on `id` (`agentRegistry.ts` `hasValidId`), and medulla-v1's
/// `AgentDescriptor` port declares the same field. An advert that names its
/// agent under any other key is silently dropped at the backend boundary, so
/// this must stay `id` on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDescriptor {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// `medulla:register_agents` — roster advertisement sent on connect. The
/// backend clears the roster when this socket disconnects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterAgents {
    pub agents: Vec<AgentDescriptor>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Capability handshake
// ─────────────────────────────────────────────────────────────────────────────

/// `medulla:capabilities_request` — the backend asks one agent to self-report.
///
/// The backend fans this out to every harness socket the user holds and
/// correlates the answer by `probe_id`; an agent that never answers costs the
/// probe its full ten-second deadline, so [`super::emit_capabilities_result`]
/// always replies, even when it has nothing interesting to say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesRequest {
    pub probe_id: String,
    #[serde(default)]
    pub agent_id: String,
}

/// `medulla:capabilities_result` — the self-report for one probe.
///
/// `capabilities` is an open bag the backend narrows through its own allowlist
/// (`sanitizeCapabilities`), so it stays raw JSON here rather than a struct this
/// side would have to keep in lockstep with the server's allowlist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesResult {
    pub probe_id: String,
    pub capabilities: serde_json::Value,
}

// ─────────────────────────────────────────────────────────────────────────────
// Workflow plane
// ─────────────────────────────────────────────────────────────────────────────

/// One saved workflow graph as advertised to the backend — enough to choose it
/// and explain the choice, never the graph itself.
///
/// Field-compatible with the `WorkflowDescriptor` port in medulla-v1, so an
/// advert crossing this boundary needs no translation layer on either side.
/// `name`/`description` skip
/// serialization when empty for exactly that reason: the Rust advert already
/// omits its empty strings, the port therefore declares them optional, and the
/// backend passes an absent key through as absent rather than fabricating `""`.
///
/// Not `Eq`: a declared input's `default` is free-form JSON, which has no total
/// equality (floats, and `NaN` among them). `PartialEq` is what comparisons here
/// actually use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDescriptor {
    /// Stable identity — the only field the wire always carries and the only one
    /// anything routes on (delegation, `get`, `runs`).
    pub id: String,
    /// Display name; omitted from the wire when blank.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Prompt surface — written like a tool description; omitted when blank.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// How many steps the graph has: a rough cost signal, rendered verbatim.
    #[serde(default)]
    pub node_count: u32,
    /// Whether this host considers the workflow runnable right now. Advisory —
    /// the reader applies no policy, this host is the one that refuses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Open vocabulary for what starts the workflow ("manual", "cron", …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_kind: Option<String>,
    /// Provenance: which roster agent owns this workflow. Usually left unset —
    /// the backend stamps the batch-level `agentId` onto any advert without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Provenance: the workspace the owning agent is deployed into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// The workflow's declared inputs — its callable signature. Lets the reader
    /// know what it must collect before asking for a run, rather than having to
    /// fetch the whole graph to find out. Empty for a workflow taking none, and
    /// omitted from the wire in that case.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<WorkflowInputDescriptor>,
}

/// One declared input of an advertised workflow — what a caller must supply to
/// run it.
///
/// Mirrors `tinyflows::model::WorkflowInput` field-for-field rather than
/// re-exporting it, for the same reason [`TaskEnvelope`] keeps its envelope as
/// raw JSON: this module is a transport vocabulary and is compiled in every
/// build, while the engine is behind the `flows` feature gate. Naming the engine
/// type here would make the medulla plane fail to build whenever flows is off.
/// The mapping from the engine type lives in `flows::medulla_bridge`, which is
/// gated and may name it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInputDescriptor {
    /// The input's name — the key a caller supplies it under.
    pub name: String,
    /// Declared JSON type: `string` | `number` | `boolean` | `json`.
    #[serde(default, rename = "type", skip_serializing_if = "String::is_empty")]
    pub ty: String,
    /// Human-readable explanation; omitted when blank.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Whether a caller must supply it.
    #[serde(default)]
    pub required: bool,
    /// Value used when the caller supplies none; omitted when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

/// `medulla:register_workflows` — the workflow advert batch, sent on connect and
/// re-sent whenever the host's store changes. The backend replaces this socket's
/// whole entry each time and drops it on disconnect, so a shrinking store is
/// communicated by re-sending the smaller list (never by a delete event).
///
/// Not `Eq` for the same reason as [`WorkflowDescriptor`], which it contains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterWorkflows {
    pub workflows: Vec<WorkflowDescriptor>,
    /// Batch-level provenance stamped onto adverts that do not name their own
    /// owner. Absent means "this host, agent unspecified".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// Which read (or the one authoring turn) a `medulla:workflow_request` asks for.
///
/// Snake-case on the wire to match the backend's `WorkflowRequestOp` union.
/// Unknown ops are a decode failure rather than a silent no-op, so a version
/// skew shows up as a reported error instead of a ten-second server timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowOp {
    /// Fetch one workflow's detail, including its graph.
    Get,
    /// The node-kind catalog, optionally filtered to one `kind`.
    NodeKinds,
    /// Recent runs of one workflow.
    Runs,
    /// A whole authoring turn on this host's own copilot.
    Copilot,
}

/// `medulla:workflow_request` — one round trip addressed to the host that
/// advertised the workflow. Which optional fields matter depends on `op`:
/// `get`/`runs` read `workflow_id`, `node_kinds` reads `kind`, `copilot` reads
/// `instruction` plus an optional `workflow_id` (absent ⇒ create).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRequest {
    pub request_id: String,
    pub op: WorkflowOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// `medulla:workflow_result` — the answer to one round trip.
///
/// `data` is OPAQUE: a graph, a node-kind catalog and a copilot outcome are all
/// host-shaped and pass through the backend verbatim (only size-bounded, at
/// 1 MiB). `ok: false` carries a readable message the orchestrator renders as a
/// tool error — a failed read must still be *answered*, because a dropped
/// request costs the server its whole deadline instead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowResult {
    pub request_id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What a copilot turn reports back: the assistant's reply, the changes it
/// actually made (derived by the host from a re-read of its own store, never
/// from the model's claim), and the id of a workflow it created.
///
/// Mirrors the `CopilotOutcome` medulla-public's authoring copilot already
/// returns and the `WorkflowCopilotOutcome` the library port declares, so it
/// crosses the seam unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopilotOutcome {
    pub reply: String,
    #[serde(default)]
    pub changes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Socket.IO event names
// ─────────────────────────────────────────────────────────────────────────────

/// Down events handled by openhuman.
pub const EVENT_TASK_RUN: &str = "medulla:task_run";
pub const EVENT_TASK_SEND: &str = "medulla:task_send";
pub const EVENT_TASK_ABORT: &str = "medulla:task_abort";
pub const EVENT_CAPABILITIES_REQUEST: &str = "medulla:capabilities_request";
pub const EVENT_WORKFLOW_REQUEST: &str = "medulla:workflow_request";

/// Up events emitted by openhuman.
pub const EVENT_TASK_ENVELOPE: &str = "medulla:task_envelope";
pub const EVENT_TASK_RESULT: &str = "medulla:task_result";
pub const EVENT_REGISTER_AGENTS: &str = "medulla:register_agents";
pub const EVENT_REGISTER_WORKFLOWS: &str = "medulla:register_workflows";
pub const EVENT_CAPABILITIES_RESULT: &str = "medulla:capabilities_result";
pub const EVENT_WORKFLOW_RESULT: &str = "medulla:workflow_result";

#[cfg(test)]
#[path = "payloads_tests.rs"]
mod tests;
