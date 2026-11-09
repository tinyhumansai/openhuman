//! The `inbound` RPC namespace: `inbound.stats`, `inbound.dead`,
//! `inbound.redrive`, and for external sources `inbound.offer`,
//! `inbound.results` and `inbound.result_ack`. Each answers
//! `COMPONENT_DISABLED:eventqueues` when the component is off.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

#[derive(Debug, Deserialize)]
struct DeadParams {
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RedriveParams {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultAckParams {
    turn_id: String,
}

const FUNCTIONS: [&str; 6] = ["stats", "dead", "redrive", "offer", "results", "result_ack"];

/// Every `inbound.*` schema.
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    FUNCTIONS.iter().map(|function| schemas(function)).collect()
}

/// Every `inbound.*` controller.
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("stats"),
            handler: handle_stats,
        },
        RegisteredController {
            schema: schemas("dead"),
            handler: handle_dead,
        },
        RegisteredController {
            schema: schemas("redrive"),
            handler: handle_redrive,
        },
        RegisteredController {
            schema: schemas("offer"),
            handler: handle_offer,
        },
        RegisteredController {
            schema: schemas("results"),
            handler: handle_results,
        },
        RegisteredController {
            schema: schemas("result_ack"),
            handler: handle_result_ack,
        },
    ]
}

/// The schema of `inbound.<function>`.
pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "stats" => ControllerSchema {
            namespace: "inbound",
            function: "stats",
            description: "Inbound event queue depths (ready, delayed, leased, dead) and \
                          outcome counts (delivered, deferred, dropped, dead, ...).",
            inputs: vec![],
            outputs: vec![json_output("result", "Queue depths and outcome counts.")],
        },
        "dead" => ControllerSchema {
            namespace: "inbound",
            function: "dead",
            description: "List inbound events parked in the dead-letter queue.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::BoundedU64 {
                    min: 0,
                    max: u64::from(u32::MAX),
                })),
                comment: "Maximum number of items (default 50, at most 500).",
                required: false,
            }],
            outputs: vec![json_output("result", "Dead-lettered items.")],
        },
        "redrive" => ControllerSchema {
            namespace: "inbound",
            function: "redrive",
            description: "Put one dead-lettered inbound event back into the pipeline.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "The dead item's id, as `inbound.dead` lists it.",
                required: true,
            }],
            outputs: vec![json_output("result", "Whether the item was redriven.")],
        },
        "offer" => ControllerSchema {
            namespace: "inbound",
            function: "offer",
            description: "Offer one event of a configured external source \
                          (`[inbound.sources.<source>]`) to the inbound queues. Events are \
                          debounced per key into batches, each run as one turn of the \
                          source's pinned agent. A redelivered `event_id` answers `duplicate`.",
            inputs: vec![
                string_input(
                    "source",
                    "A configured source name, e.g. `x:mentions`.",
                    true,
                ),
                string_input("key", "What the source's events are batched by.", true),
                string_input(
                    "event_id",
                    "The source's own id; a redelivery is a duplicate.",
                    true,
                ),
                string_input("text", "The event's text.", true),
                FieldSchema {
                    name: "payload",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "The source's payload, handed to the agent as is.",
                    required: false,
                },
                u64_input(
                    "received_at_ms",
                    "When the source received it (default now).",
                ),
            ],
            outputs: vec![json_output(
                "result",
                "`{disposition}`: `accepted`, `duplicate`, or why it ended at ingress.",
            )],
        },
        "results" => ControllerSchema {
            namespace: "inbound",
            function: "results",
            description: "List unacknowledged results of pinned-agent turns, oldest first.",
            inputs: vec![
                string_input("source", "Only this source's results.", false),
                u64_input("since_ms", "Only results started at or after this time."),
                u64_input(
                    "limit",
                    "Maximum number of results (default 50, at most 500).",
                ),
                string_input("status", "`running`, `completed` or `failed`.", false),
            ],
            outputs: vec![json_output(
                "result",
                "`{results: [{turn_id, source, key, agent_id, status, text?, error?, \
                 event_ids, blocked: [{event_id, reason}], created_ms, completed_ms?}]}`.",
            )],
        },
        "result_ack" => ControllerSchema {
            namespace: "inbound",
            function: "result_ack",
            description: "Acknowledge one pinned-agent result; it is no longer listed.",
            inputs: vec![string_input(
                "turn_id",
                "The result's turn id, as `inbound.results` lists it.",
                true,
            )],
            outputs: vec![json_output("result", "`{acked: true}`.")],
        },
        _ => ControllerSchema {
            namespace: "inbound",
            function: "unknown",
            description: "Unknown inbound function.",
            inputs: vec![],
            outputs: vec![json_output("error", "Lookup error details.")],
        },
    }
}

async fn config_and_client(
    ctx: &DispatchCtx,
) -> Result<(crate::config::Config, tinybus::module::ModuleClient), String> {
    let config = ctx.load_config().await?;
    let client = crate::modules::ops::client(config.clone());
    Ok((config, client))
}

fn handle_stats(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let (config, client) = config_and_client(&ctx).await?;
        to_json(super::ops::stats(&config, &client).await?)
    })
}

fn handle_dead(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = deserialize_params::<DeadParams>(params)?;
        let (config, client) = config_and_client(&ctx).await?;
        to_json(super::ops::dead(&config, &client, params.limit).await?)
    })
}

fn handle_redrive(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = deserialize_params::<RedriveParams>(params)?;
        let (config, client) = config_and_client(&ctx).await?;
        to_json(super::ops::redrive(&config, &client, &params.id).await?)
    })
}

fn handle_offer(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = deserialize_params::<super::ops::OfferParams>(params)?;
        let (config, client) = config_and_client(&ctx).await?;
        to_json(super::ops::offer(&config, &client, params).await?)
    })
}

fn handle_results(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = deserialize_params::<super::ops::ResultsParams>(params)?;
        let config = ctx.load_config().await?;
        let store = results_store(&config)?;
        to_json(super::ops::results(&config, &store, params)?)
    })
}

fn handle_result_ack(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let params = deserialize_params::<ResultAckParams>(params)?;
        let config = ctx.load_config().await?;
        let store = results_store(&config)?;
        to_json(super::ops::result_ack(&config, &store, &params.turn_id)?)
    })
}

fn results_store(config: &crate::config::Config) -> Result<super::results::Results, String> {
    crate::core::runtime::components::require_config(config, super::settings::COMPONENT)?;
    super::results::Results::open(config).map_err(|error| format!("inbound results: {error:#}"))
}

fn string_input(name: &'static str, comment: &'static str, required: bool) -> FieldSchema {
    FieldSchema {
        name,
        ty: if required {
            TypeSchema::String
        } else {
            TypeSchema::Option(Box::new(TypeSchema::String))
        },
        comment,
        required,
    }
}

fn u64_input(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::BoundedU64 {
            min: 0,
            max: u64::MAX,
        })),
        comment,
        required: false,
    }
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
