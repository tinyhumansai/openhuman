//! The [`Flow`] entity: a saved automation workflow definition.
//!
//! Wraps `tinyflows::model::WorkflowGraph` with the metadata OpenHuman needs to
//! store, list, and track runs for a saved flow. The graph itself is the
//! portable, tinyflows-owned contract (validated + migrated on load); this
//! struct is the OpenHuman-side record around it.

use serde::{Deserialize, Serialize};
use tinyflows::model::WorkflowGraph;

/// How a flow run was started. Stamped onto the run's Langfuse trace as a
/// `trigger:<kind>` tag plus `trigger` metadata so runs can be filtered by
/// origin in the Langfuse UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowRunTrigger {
    /// An explicit run request over RPC/CLI (the Workflows UI "Run" button).
    Rpc,
    /// A `FlowScheduleTick` cron dispatch (`schedule` trigger node).
    Schedule,
    /// A `ComposioTriggerReceived` dispatch (`app_event` trigger node).
    AppEvent,
    /// A human-in-the-loop resume of a paused run (`flows_resume`).
    Resume,
}

impl FlowRunTrigger {
    /// Stable snake_case identifier used in Langfuse tags/metadata.
    pub fn as_str(&self) -> &'static str {
        match self {
            FlowRunTrigger::Rpc => "rpc",
            FlowRunTrigger::Schedule => "schedule",
            FlowRunTrigger::AppEvent => "app_event",
            FlowRunTrigger::Resume => "resume",
        }
    }
}

/// The result of validating a candidate `tinyflows` graph without persisting
/// it — returned by `openhuman.flows_validate` (PHASE 3c) and used to surface
/// structural errors and non-fatal warnings (e.g. "this trigger kind never
/// fires automatically yet") to an authoring surface *before* a flow is saved.
///
/// A graph is `valid` when it passes `tinyflows::validate::validate` after
/// migration; `errors` carries the single structural error when it does not.
/// `warnings` is orthogonal to validity — a `valid` graph can still carry
/// warnings (it saves and enables fine, it just won't behave as an author
/// might expect), and an invalid graph reports no warnings (there's nothing to
/// warn about a graph that won't compile).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FlowValidation {
    /// True when the graph is structurally valid (migrates + validates).
    pub valid: bool,
    /// Structural validation errors (empty when `valid`). Today at most one —
    /// `tinyflows::validate::validate` returns the first error it hits.
    pub errors: Vec<String>,
    /// Non-fatal warnings: the graph is accepted, but something about it is
    /// worth flagging (e.g. an unfired trigger kind). Never blocks save/enable.
    pub warnings: Vec<String>,
}

/// The result of importing a workflow definition (native tinyflows JSON or an
/// n8n export) via `openhuman.flows_import` (PHASE 4d) — the normalized,
/// migrated + validated [`WorkflowGraph`] plus any non-fatal import warnings
/// (unmapped n8n node types, untranslated expressions, a synthesized/demoted
/// trigger, …).
///
/// **Import never persists.** This is the same contract as
/// [`FlowValidation`]: the graph comes back ready for the editable canvas as a
/// *draft*, and only the user's explicit Save (the existing `flows_create`
/// gate) writes it. A structurally invalid graph is reported as an `Err` on the
/// RPC (validation is authoritative), not as an `FlowImport` with `valid:
/// false` — there is no partial-import row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlowImport {
    /// The normalized workflow graph, migrated to the current schema and
    /// structurally validated. Ready to open on the canvas as an unsaved draft.
    pub graph: WorkflowGraph,
    /// Non-fatal import warnings surfaced next to the draft. Empty for a clean
    /// native import; an n8n import populates it with any approximations made.
    pub warnings: Vec<String>,
}

/// A saved automation workflow: a `tinyflows` graph plus OpenHuman-side
/// bookkeeping (enablement, run history summary).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    /// Stable identifier (UUID) for this flow.
    pub id: String,
    /// Human-readable name shown in the Workflows UI.
    pub name: String,
    /// Whether this flow may currently be triggered (B2) / run.
    pub enabled: bool,
    /// The validated, migrated workflow graph.
    pub graph: WorkflowGraph,
    /// RFC3339 creation timestamp.
    pub created_at: String,
    /// RFC3339 last-update timestamp.
    pub updated_at: String,
    /// RFC3339 timestamp of the most recent run, if any.
    pub last_run_at: Option<String>,
    /// Outcome of the most recent run: `"completed"` | `"pending_approval"` | `"failed"`.
    pub last_status: Option<String>,
    /// "Require approval for outbound actions" (issue B2). When `true`, the
    /// approval gate does NOT auto-allow this flow's `TrustedAutomation
    /// { Workflow }` trust root — every external_effect tool/HTTP call the
    /// flow makes still parks for a real decision, regardless of how the run
    /// was triggered. See `src/openhuman/approval/gate.rs` and
    /// `src/openhuman/agent/turn_origin.rs::TrustedAutomationSource::Workflow`.
    #[serde(default)]
    pub require_approval: bool,
}

/// One step of a persisted [`FlowRun`] (run-history inspector).
///
/// As of issue G2 (live run observation) these are persisted **incrementally**
/// as each non-trigger node finishes, by
/// `flows::observability::FlowRunObserver::on_step_finish`, which maps a live
/// `tinyflows::observability::ExecutionStep` (carrying real `status` +
/// `duration_ms`) onto this type. The prior post-hoc reconstruction from
/// `RunOutcome.output["nodes"]` (see `flows::ops::reconstruct_steps`) now only
/// fills in steps the observer missed (e.g. a trigger node, which does not
/// emit an `on_step_finish`) — those carry no `status`/`duration_ms` and keep
/// the `port` the reconstruction recovers.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FlowRunStep {
    /// The node's id within the flow's graph.
    pub node_id: String,
    /// The node's emitted items for this run (`output["nodes"][id]["items"]`,
    /// or the live `ExecutionStep.output` when observed incrementally).
    pub output: serde_json::Value,
    /// The output port the node routed on, if it picked one (branching /
    /// switch nodes) — `output["nodes"][id]["port"]`. Only recovered by the
    /// post-hoc reconstruction; the live observer does not carry a port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    /// Live step outcome, when this step was observed incrementally:
    /// `"success"` | `"error"`. `None` for a step recovered post-hoc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Wall-clock duration of the node's executor in milliseconds, when
    /// observed incrementally. `None` for a step recovered post-hoc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Data-binding diagnostics from the engine: each config `=`-expression
    /// that resolved to `null` during this step, as
    /// `{ "location": "args.to", "expression": "=item.to" }`. Lets the run
    /// view point at the exact unresolved wiring. Empty for clean steps and
    /// for steps recovered post-hoc.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<serde_json::Value>,
}

/// A resolvable connection the flows UI / agent picker can attach to a node's
/// `connection_ref`. Aggregated by `openhuman.flows_list_connections` from two
/// host-side sources:
///
/// - **Composio connected accounts** (`kind = "composio"`) — each active OAuth
///   integration instance, emitted as a ready-to-use
///   `"composio:<toolkit>:<connection_id>"` ref (the exact shape
///   `tinyflows::caps::composio_connection_id` parses back on execution).
/// - **Named HTTP credentials** (`kind = "http"`) — each stored injection
///   template, emitted as `"http_cred:<name>"` (the shape
///   `tinyflows::caps::http_cred_name` parses).
///
/// **Security contract:** carries only non-secret identity — the
/// `connection_ref` string plus a display label (and toolkit/scheme hints).
/// It NEVER carries secret material (OAuth tokens, bearer tokens, passwords,
/// API keys). Those stay server-side and are injected only inside the
/// `tinyflows::caps` adapters at execution time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlowConnection {
    /// The ready-to-use `connection_ref` value to stamp onto a node:
    /// `"composio:<toolkit>:<connection_id>"` or `"http_cred:<name>"`.
    pub connection_ref: String,
    /// Source kind: `"composio"` | `"http"`.
    pub kind: String,
    /// Human-readable label for the picker, e.g. `"Gmail · user@example.com"`
    /// or `"stripe (bearer)"`. Never contains secret material.
    pub display: String,
    /// Composio toolkit slug (`kind = "composio"` only), e.g. `"gmail"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolkit: Option<String>,
    /// HTTP credential injection scheme (`kind = "http"` only):
    /// `"bearer"` | `"basic"` | `"header"`. Not a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
}

/// A persisted record of one `flows_run` / `flows_resume` invocation, for the
/// B3 run-history inspector. Written by `flows::store` from `flows::ops`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowRun {
    /// Stable identifier for this run — the same value as `thread_id` (the
    /// tinyflows checkpointer key), so a run row can be found either way.
    pub id: String,
    /// The flow this run belongs to.
    pub flow_id: String,
    /// The tinyflows checkpointer thread id (needed to `flows_resume`).
    pub thread_id: String,
    /// Run status. Not an enum (kept a free-form `String` for forward-compat
    /// with statuses added by newer builds), but the vocabulary is fixed:
    /// `"running"` | `"completed"` | `"completed_with_warnings"` |
    /// `"pending_approval"` | `"failed"` | `"cancelled"` (issue G4 — a run
    /// cancelled via `flows_cancel_run`, or a parked `pending_approval` run
    /// swept by the TTL expiry). `"completed_with_warnings"` (run honesty,
    /// PR2) is a terminal status like `"completed"`, but at least one settled
    /// [`FlowRunStep`] carries non-empty `diagnostics` (a `=`-binding that
    /// resolved to `null`) even though no step outright errored. All of
    /// `completed` / `completed_with_warnings` / `failed` / `cancelled` are
    /// terminal.
    pub status: String,
    /// RFC3339 timestamp when the run started.
    pub started_at: String,
    /// RFC3339 timestamp when the run last settled (completed/paused/failed).
    /// `None` while a run row is still `"running"`.
    pub finished_at: Option<String>,
    /// Reconstructed per-node steps (see [`FlowRunStep`]).
    #[serde(default)]
    pub steps: Vec<FlowRunStep>,
    /// Node ids paused awaiting human approval when `status ==
    /// "pending_approval"`; empty otherwise.
    #[serde(default)]
    pub pending_approvals: Vec<String>,
    /// Error message when `status == "failed"`.
    #[serde(default)]
    pub error: Option<String>,
}

/// Lifecycle status of a [`FlowSuggestion`] discovery card.
///
/// A freshly discovered suggestion starts `New`. The user can `Dismiss` it (it
/// stays persisted so a later discovery run can dedupe against it and won't
/// re-surface a rejected idea) or act on it — once the suggestion's flow is
/// actually saved via `flows_create`, the frontend marks it `Built`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SuggestionStatus {
    /// Freshly discovered, awaiting the user's decision. The default.
    #[default]
    New,
    /// The user dismissed the card; kept for dedupe, never re-surfaced.
    Dismissed,
    /// The user built (saved) a flow from this suggestion.
    Built,
}

impl SuggestionStatus {
    /// The stable lowercase token persisted in SQLite / crossed over RPC.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Dismissed => "dismissed",
            Self::Built => "built",
        }
    }

    /// Parse a persisted/RPC token back into a status. Unknown tokens fall
    /// back to [`SuggestionStatus::New`] (forward-compatible with any status a
    /// newer build might persist), so a stale row never hard-errors a read.
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "dismissed" => Self::Dismissed,
            "built" => Self::Built,
            _ => Self::New,
        }
    }
}

/// A concrete, buildable workflow idea proposed by the `flow_discovery` agent
/// (the "Flow Scout"). Persisted to the `flow_suggestions` table and surfaced
/// as a card in the Flows page "Suggested for you" section.
///
/// **Not a graph.** A suggestion is a *pitch* the user can accept, not a
/// validated [`WorkflowGraph`]. Its [`Self::build_prompt`] is the natural-language
/// brief handed to the `workflow_builder` agent when the user clicks "Build
/// this"; that agent turns it into a real graph proposal for the user to save.
/// This keeps the discovery agent read-only and the authoring pipeline unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlowSuggestion {
    /// Stable identifier (a content hash of the normalized title, so re-running
    /// discovery dedupes identical ideas rather than piling duplicates).
    pub id: String,
    /// Short, human-friendly title, e.g. `"Auto-file email receipts"`.
    pub title: String,
    /// One-sentence description of what the workflow would do, e.g.
    /// `"When a Gmail receipt arrives, add a row to your expenses sheet."`
    pub one_liner: String,
    /// Why this is being suggested to *this* user — grounded in what the agent
    /// observed (a recurring thread, a stated goal in memory, a connected app),
    /// e.g. `"You forward receipts to yourself most weeks."`
    pub rationale: String,
    /// Which trigger the workflow would likely use, as a hint for the card and
    /// the builder: `"schedule"` | `"app_event"` | `"manual"` (free-form; only
    /// those three self-fire in this host).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_hint: Option<String>,
    /// Plain-language outline of the steps, one per element, e.g.
    /// `["Watch Gmail for receipts", "Extract amount + vendor", "Append a Sheet row"]`.
    #[serde(default)]
    pub steps_outline: Vec<String>,
    /// `connection_ref` values the agent grounded against real
    /// `flows_list_connections` output (never invented), so the card can show
    /// "uses your Gmail" and the builder can stamp them verbatim.
    #[serde(default)]
    pub suggested_connections: Vec<String>,
    /// Real Composio action slugs the agent grounded via `search_tool_catalog`
    /// (never hallucinated). Empty when the workflow is HTTP/agent-only.
    #[serde(default)]
    pub suggested_slugs: Vec<String>,
    /// The natural-language brief handed to `workflow_builder` on "Build this".
    /// Self-contained: trigger + steps + connections, enough for the builder to
    /// author a graph without re-deriving the idea.
    pub build_prompt: String,
    /// Agent's self-rated confidence in `[0.0, 1.0]` that this is a genuinely
    /// useful, buildable automation for the user — used to rank cards.
    #[serde(default)]
    pub confidence: f64,
    /// Lifecycle status (see [`SuggestionStatus`]).
    #[serde(default)]
    pub status: SuggestionStatus,
    /// RFC3339 timestamp when the suggestion was first discovered.
    pub created_at: String,
    /// The `flows_discover` run that produced this suggestion (correlation for
    /// observability); `None` for suggestions authored outside a tracked run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tinyflows::model::{Node, NodeKind};

    fn sample_graph() -> WorkflowGraph {
        WorkflowGraph {
            nodes: vec![Node {
                id: "t".to_string(),
                kind: NodeKind::Trigger,
                type_version: 1,
                name: "Trigger".to_string(),
                config: serde_json::Value::Null,
                ports: Vec::new(),
                position: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn flow_round_trips_through_json() {
        let flow = Flow {
            id: "flow_1".to_string(),
            name: "demo".to_string(),
            enabled: true,
            graph: sample_graph(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            last_run_at: None,
            last_status: None,
            require_approval: false,
        };
        let json = serde_json::to_string(&flow).expect("serialize");
        let back: Flow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, flow.id);
        assert_eq!(back.graph, flow.graph);
        assert!(back.last_run_at.is_none());
        assert!(!back.require_approval);
    }

    #[test]
    fn flow_require_approval_defaults_false_when_omitted_from_json() {
        // Legacy/serialized JSON authored before the field existed must still
        // deserialize (SQLite rows are migrated via `add_column_if_missing`,
        // but any bare JSON fixture should also default safely).
        let json = serde_json::json!({
            "id": "flow_1",
            "name": "demo",
            "enabled": true,
            "graph": sample_graph(),
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
        });
        let flow: Flow = serde_json::from_value(json).expect("deserialize");
        assert!(!flow.require_approval);
    }

    #[test]
    fn flow_run_round_trips_through_json() {
        let run = FlowRun {
            id: "flow:flow_1:run-uuid".to_string(),
            flow_id: "flow_1".to_string(),
            thread_id: "flow:flow_1:run-uuid".to_string(),
            status: "completed".to_string(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            finished_at: Some("2026-01-01T00:00:01Z".to_string()),
            steps: vec![FlowRunStep {
                node_id: "t".to_string(),
                output: serde_json::json!([{"json": {"hello": "world"}}]),
                port: None,
                ..Default::default()
            }],
            pending_approvals: Vec::new(),
            error: None,
        };
        let json = serde_json::to_string(&run).expect("serialize");
        let back: FlowRun = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, run.id);
        assert_eq!(back.steps.len(), 1);
        assert_eq!(back.steps[0].node_id, "t");
        assert!(back.steps[0].port.is_none());
    }

    #[test]
    fn flow_run_step_omits_port_when_none() {
        let step = FlowRunStep {
            node_id: "n".to_string(),
            output: serde_json::Value::Null,
            port: None,
            ..Default::default()
        };
        let v = serde_json::to_value(&step).unwrap();
        assert!(v.get("port").is_none());
    }

    #[test]
    fn suggestion_status_token_round_trips() {
        for st in [
            SuggestionStatus::New,
            SuggestionStatus::Dismissed,
            SuggestionStatus::Built,
        ] {
            assert_eq!(SuggestionStatus::from_str_lossy(st.as_str()), st);
        }
        // Unknown tokens fall back to New rather than erroring.
        assert_eq!(
            SuggestionStatus::from_str_lossy("something_new"),
            SuggestionStatus::New
        );
        assert_eq!(SuggestionStatus::default(), SuggestionStatus::New);
    }

    #[test]
    fn flow_suggestion_round_trips_through_json() {
        let s = FlowSuggestion {
            id: "sug_abc".to_string(),
            title: "Auto-file email receipts".to_string(),
            one_liner: "When a Gmail receipt arrives, add a row to your expenses sheet."
                .to_string(),
            rationale: "You forward receipts to yourself most weeks.".to_string(),
            trigger_hint: Some("app_event".to_string()),
            steps_outline: vec![
                "Watch Gmail for receipts".to_string(),
                "Extract amount + vendor".to_string(),
            ],
            suggested_connections: vec!["composio:gmail:conn_1".to_string()],
            suggested_slugs: vec!["GMAIL_NEW_GMAIL_MESSAGE".to_string()],
            build_prompt: "Build a workflow that…".to_string(),
            confidence: 0.82,
            status: SuggestionStatus::New,
            created_at: "2026-07-05T00:00:00Z".to_string(),
            source_run_id: Some("run-1".to_string()),
        };
        let json = serde_json::to_string(&s).expect("serialize");
        let back: FlowSuggestion = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, s);
    }

    #[test]
    fn flow_suggestion_defaults_optional_fields() {
        // A minimal pitch (no trigger/steps/connections/slugs/status/run) must
        // deserialize with safe defaults.
        let json = serde_json::json!({
            "id": "sug_min",
            "title": "Daily digest",
            "one_liner": "Summarize your unread mail each morning.",
            "rationale": "You check mail first thing.",
            "build_prompt": "Build a scheduled digest…",
            "created_at": "2026-07-05T00:00:00Z",
        });
        let s: FlowSuggestion = serde_json::from_value(json).expect("deserialize");
        assert!(s.trigger_hint.is_none());
        assert!(s.steps_outline.is_empty());
        assert!(s.suggested_connections.is_empty());
        assert!(s.suggested_slugs.is_empty());
        assert_eq!(s.confidence, 0.0);
        assert_eq!(s.status, SuggestionStatus::New);
        assert!(s.source_run_id.is_none());
    }
}
