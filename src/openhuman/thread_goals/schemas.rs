//! Controller schemas + JSON-RPC handlers for the `thread_goals` namespace.
//!
//! Methods are exposed as `openhuman.thread_goals_<function>`:
//! `get`, `set`, `complete`, `pause`, `resume`, `clear`. Handlers load the
//! active config (for `workspace_dir`), delegate to [`super::ops`], and
//! serialise the [`RpcOutcome`] into the CLI-compatible JSON shape.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::ops;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

/// All `thread_goals` controller schemas (advertised to CLI + RPC consumers).
pub fn all_thread_goals_controller_schemas() -> Vec<ControllerSchema> {
    FUNCTIONS.iter().map(|f| schemas(f)).collect()
}

/// Registered `thread_goals` controllers (schema + handler pairs).
pub fn all_thread_goals_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schemas("set"),
            handler: handle_set,
        },
        RegisteredController {
            schema: schemas("complete"),
            handler: handle_complete,
        },
        RegisteredController {
            schema: schemas("pause"),
            handler: handle_pause,
        },
        RegisteredController {
            schema: schemas("resume"),
            handler: handle_resume,
        },
        RegisteredController {
            schema: schemas("clear"),
            handler: handle_clear,
        },
    ]
}

const FUNCTIONS: &[&str] = &["get", "set", "complete", "pause", "resume", "clear"];

fn thread_id_input() -> FieldSchema {
    FieldSchema {
        name: "thread_id",
        ty: TypeSchema::String,
        comment: "The conversation thread the goal belongs to.",
        required: true,
    }
}

fn goal_output() -> FieldSchema {
    FieldSchema {
        name: "result",
        ty: TypeSchema::Json,
        comment: "{ goal } — the current thread goal (or null when absent).",
        required: true,
    }
}

/// Schema definitions for every `thread_goals` function.
fn schemas(function: &str) -> ControllerSchema {
    match function {
        "get" => ControllerSchema {
            namespace: "thread_goals",
            function: "get",
            description: "Get the thread-level goal for a thread (or null).",
            inputs: vec![thread_id_input()],
            outputs: vec![goal_output()],
        },
        "set" => ControllerSchema {
            namespace: "thread_goals",
            function: "set",
            description: "Create or replace the thread-level goal. Changing the objective \
                          mints a new goal id and resets usage counters.",
            inputs: vec![
                thread_id_input(),
                FieldSchema {
                    name: "objective",
                    ty: TypeSchema::String,
                    comment: "The durable objective the agent should keep pursuing.",
                    required: true,
                },
                FieldSchema {
                    name: "token_budget",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment:
                        "Optional token ceiling; exceeding it flips the goal to budget_limited.",
                    required: false,
                },
            ],
            outputs: vec![goal_output()],
        },
        "complete" => ControllerSchema {
            namespace: "thread_goals",
            function: "complete",
            description: "Mark the thread goal complete.",
            inputs: vec![thread_id_input()],
            outputs: vec![goal_output()],
        },
        "pause" => ControllerSchema {
            namespace: "thread_goals",
            function: "pause",
            description: "Pause an active thread goal.",
            inputs: vec![thread_id_input()],
            outputs: vec![goal_output()],
        },
        "resume" => ControllerSchema {
            namespace: "thread_goals",
            function: "resume",
            description: "Resume a paused thread goal.",
            inputs: vec![thread_id_input()],
            outputs: vec![goal_output()],
        },
        "clear" => ControllerSchema {
            namespace: "thread_goals",
            function: "clear",
            description: "Clear (delete) the thread goal.",
            inputs: vec![thread_id_input()],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "{ removed } — whether a goal existed and was removed.",
                required: true,
            }],
        },
        other => panic!("unknown thread_goals function: {other}"),
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<ThreadIdParams>(Value::Object(params))?;
        to_json(ops::get(&config.workspace_dir, &req.thread_id).await?)
    })
}

fn handle_set(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<SetParams>(Value::Object(params))?;
        to_json(
            ops::set(
                &config.workspace_dir,
                &req.thread_id,
                &req.objective,
                req.token_budget,
            )
            .await?,
        )
    })
}

fn handle_complete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<ThreadIdParams>(Value::Object(params))?;
        to_json(ops::complete(&config.workspace_dir, &req.thread_id).await?)
    })
}

fn handle_pause(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<ThreadIdParams>(Value::Object(params))?;
        to_json(ops::pause(&config.workspace_dir, &req.thread_id).await?)
    })
}

fn handle_resume(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<ThreadIdParams>(Value::Object(params))?;
        to_json(ops::resume(&config.workspace_dir, &req.thread_id).await?)
    })
}

fn handle_clear(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<ThreadIdParams>(Value::Object(params))?;
        to_json(ops::clear(&config.workspace_dir, &req.thread_id).await?)
    })
}

// ── Param structs + helpers ──────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ThreadIdParams {
    thread_id: String,
}

#[derive(serde::Deserialize)]
struct SetParams {
    thread_id: String,
    objective: String,
    #[serde(default)]
    token_budget: Option<u64>,
}

fn parse_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_all_controllers() {
        let controllers = all_thread_goals_registered_controllers();
        assert_eq!(controllers.len(), FUNCTIONS.len());
        let methods: Vec<String> = controllers
            .iter()
            .map(|c| format!("{}.{}", c.schema.namespace, c.schema.function))
            .collect();
        for f in FUNCTIONS {
            let expected = format!("thread_goals.{f}");
            assert!(methods.contains(&expected), "missing {expected}");
        }
    }

    #[test]
    fn schemas_and_controllers_stay_in_sync() {
        assert_eq!(
            all_thread_goals_controller_schemas().len(),
            all_thread_goals_registered_controllers().len()
        );
    }
}
