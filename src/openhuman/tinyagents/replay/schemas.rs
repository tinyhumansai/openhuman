//! Read-only JSON-RPC controllers for agent run replay + status
//! (workstream 05.x), over the C4 durable journal/status seams.
//!
//! Three controllers, all in the `agent` namespace, all **read-only** (no
//! mutation, no security/approval/sandbox bypass, no writes):
//!
//! - `openhuman.agent_run_events` — paged late-attach replay of a run's durable
//!   event stream. Params: `run_id`, `offset` (default 0), `limit` (default
//!   [`DEFAULT_EVENTS_LIMIT`], capped at [`MAX_EVENTS_LIMIT`]). Returns
//!   `{ events: [<AgentObservation>...], next_offset: <int|null> }` — `null`
//!   once the stream is drained.
//! - `openhuman.agent_run_status` — latest [`HarnessRunStatus`] for `run_id`,
//!   or `null` for an unknown run.
//! - `openhuman.agent_runs_active` — active runs, optionally filtered by
//!   `thread_id` and/or `root_run_id`. Returns `{ runs: [<HarnessRunStatus>...] }`.
//!
//! ## Serialization / DTO note
//!
//! Events are surfaced as the crate's [`AgentObservation`] serde shape and
//! statuses as the [`HarnessRunStatus`] serde shape, projected **directly** —
//! no bespoke DTO. This is deliberate: both types are exactly what the C4 layer
//! already persists as JSON in `{workspace}/tinyagents_store` (an
//! `AgentObservation` is the durable journal record; a `HarnessRunStatus` is the
//! durable status snapshot), so a direct projection is guaranteed round-trip
//! stable and cannot drift from what a replay reconstructs. `AgentObservation`
//! carries `{ event_id, run_id, parent_run_id?, root_run_id, offset, ts_ms,
//! event }`, where `event` is the internally-tagged (`"kind"`) `AgentEvent`
//! enum; `HarnessRunStatus` carries ids/lineage, `status`, `current_phase`,
//! call counters, usage/cost totals, and timestamps — never prompt text, tool
//! arguments, or provider payloads.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use tinyagents::harness::events::HarnessRunStatus;
use tinyagents::harness::observability::AgentObservation;

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::ops::{
    list_active_runs, read_run_events_page, read_run_status, DEFAULT_EVENTS_LIMIT, MAX_EVENTS_LIMIT,
};

const NAMESPACE: &str = "agent";

// ---------------------------------------------------------------------------
// Params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RunEventsParams {
    run_id: String,
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RunStatusParams {
    run_id: String,
}

#[derive(Debug, Deserialize, Default)]
struct RunsActiveParams {
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    root_run_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Responses (direct projection of the crate serde shapes — see module docs)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct RunEventsResponse {
    events: Vec<AgentObservation>,
    /// Cursor to fetch the next page, or `null` once the stream is drained.
    next_offset: Option<u64>,
}

#[derive(Debug, Serialize)]
struct RunsActiveResponse {
    runs: Vec<HarnessRunStatus>,
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// All read-only replay/status controller schemas (workstream 05.x).
pub(crate) fn all_agent_replay_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        replay_schema("run_events"),
        replay_schema("run_status"),
        replay_schema("runs_active"),
    ]
}

/// All read-only replay/status registered controllers (workstream 05.x).
pub(crate) fn all_agent_replay_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: replay_schema("run_events"),
            handler: handle_run_events,
        },
        RegisteredController {
            schema: replay_schema("run_status"),
            handler: handle_run_status,
        },
        RegisteredController {
            schema: replay_schema("runs_active"),
            handler: handle_runs_active,
        },
    ]
}

fn replay_schema(function: &str) -> ControllerSchema {
    match function {
        "run_events" => ControllerSchema {
            namespace: NAMESPACE,
            function: "run_events",
            description:
                "Read-only paged replay of a durable agent run's event stream (late-attach \
                 reconnect/backfill). Returns AgentObservations at offset >= `offset` in order, \
                 plus `next_offset` (null once drained). Never mutates state.",
            inputs: vec![
                FieldSchema {
                    name: "run_id",
                    ty: TypeSchema::String,
                    comment: "Durable run id (as minted by the journal, e.g. `run.<hex>`).",
                    required: true,
                },
                FieldSchema {
                    name: "offset",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Start stream offset (inclusive). Default 0 replays the whole run.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max events in this page. Defaults to 200, capped at 1000.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "events",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                    comment: "Ordered AgentObservations (event_id, run_id, lineage, offset, \
                              ts_ms, typed `event`).",
                    required: true,
                },
                FieldSchema {
                    name: "next_offset",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Cursor for the next page, or null when the stream is drained.",
                    required: false,
                },
            ],
        },
        "run_status" => ControllerSchema {
            namespace: NAMESPACE,
            function: "run_status",
            description:
                "Read-only latest durable status snapshot (HarnessRunStatus) for a run, or null \
                 when the run is unknown. Counters/phase/usage/cost only — never prompts or \
                 payloads.",
            inputs: vec![FieldSchema {
                name: "run_id",
                ty: TypeSchema::String,
                comment: "Durable run id to look up.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                comment: "The HarnessRunStatus snapshot, or null for an unknown run.",
                required: false,
            }],
        },
        "runs_active" => ControllerSchema {
            namespace: NAMESPACE,
            function: "runs_active",
            description:
                "Read-only listing of active (pending/running/interrupted) agent runs, optionally \
                 filtered by `thread_id` and/or `root_run_id`. Never mutates state.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Restrict to runs on this conversation thread.",
                    required: false,
                },
                FieldSchema {
                    name: "root_run_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Restrict to descendants of this root run.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "runs",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Active HarnessRunStatus snapshots matching the filter.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: NAMESPACE,
            function: "unknown",
            description: "Unknown agent replay controller.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Resolve the configured internal workspace whose `tinyagents_store/` holds the
/// journal + status stores.
async fn configured_workspace() -> Result<std::path::PathBuf, String> {
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("failed to load config: {e}"))?;
    Ok(config.workspace_dir)
}

fn handle_run_events(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: RunEventsParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let limit = payload
            .limit
            .unwrap_or(DEFAULT_EVENTS_LIMIT)
            .min(MAX_EVENTS_LIMIT);
        log::debug!(
            "[rpc] openhuman.agent_run_events run_id={} offset={} limit={}",
            payload.run_id,
            payload.offset,
            limit
        );

        let workspace = configured_workspace().await?;
        let page = read_run_events_page(&workspace, &payload.run_id, payload.offset, limit)
            .await
            .map_err(|e| format!("read run events failed: {e:#}"))?;
        log::debug!(
            "[rpc] openhuman.agent_run_events run_id={} returned={} next_offset={:?}",
            payload.run_id,
            page.events.len(),
            page.next_offset
        );

        let response = RunEventsResponse {
            events: page.events,
            next_offset: page.next_offset,
        };
        serde_json::to_value(response).map_err(|e| format!("serialize response failed: {e}"))
    })
}

fn handle_run_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: RunStatusParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        log::debug!("[rpc] openhuman.agent_run_status run_id={}", payload.run_id);

        let workspace = configured_workspace().await?;
        let status = read_run_status(&workspace, &payload.run_id)
            .await
            .map_err(|e| format!("read run status failed: {e:#}"))?;
        log::debug!(
            "[rpc] openhuman.agent_run_status run_id={} found={}",
            payload.run_id,
            status.is_some()
        );

        // Bare projection: object for a known run, `null` for an unknown one.
        serde_json::to_value(status).map_err(|e| format!("serialize response failed: {e}"))
    })
}

fn handle_runs_active(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: RunsActiveParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        log::debug!(
            "[rpc] openhuman.agent_runs_active thread_id={:?} root_run_id={:?}",
            payload.thread_id,
            payload.root_run_id
        );

        let workspace = configured_workspace().await?;
        let runs = list_active_runs(
            &workspace,
            payload.thread_id.as_deref(),
            payload.root_run_id.as_deref(),
        )
        .await
        .map_err(|e| format!("list active runs failed: {e:#}"))?;
        log::debug!("[rpc] openhuman.agent_runs_active returned={}", runs.len());

        serde_json::to_value(RunsActiveResponse { runs })
            .map_err(|e| format!("serialize response failed: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_inventory_is_stable() {
        let schemas = all_agent_replay_controller_schemas();
        assert_eq!(schemas.len(), 3);
        assert!(schemas.iter().all(|s| s.namespace == "agent"));
        let functions: Vec<&str> = schemas.iter().map(|s| s.function).collect();
        assert!(functions.contains(&"run_events"));
        assert!(functions.contains(&"run_status"));
        assert!(functions.contains(&"runs_active"));

        let controllers = all_agent_replay_registered_controllers();
        assert_eq!(controllers.len(), 3);
        // rpc method names follow openhuman.<namespace>_<function>.
        let methods: Vec<String> = controllers.iter().map(|c| c.rpc_method_name()).collect();
        assert!(methods.contains(&"openhuman.agent_run_events".to_string()));
        assert!(methods.contains(&"openhuman.agent_run_status".to_string()));
        assert!(methods.contains(&"openhuman.agent_runs_active".to_string()));
    }

    #[tokio::test]
    async fn run_events_rejects_missing_run_id() {
        let err = handle_run_events(Map::new()).await.unwrap_err();
        assert!(err.contains("invalid params"), "{err}");
    }
}
