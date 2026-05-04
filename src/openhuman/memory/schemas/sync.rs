//! Schemas and handlers for memory-sync and ingestion-status RPC methods.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::memory::rpc::{self, SyncChannelParams};

use super::{parse_params, to_json};

pub(super) const FUNCTIONS: &[&str] = &["sync_channel", "sync_all", "ingestion_status"];

pub(super) fn controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema("sync_channel").unwrap(),
            handler: handle_sync_channel,
        },
        RegisteredController {
            schema: schema("sync_all").unwrap(),
            handler: handle_sync_all,
        },
        RegisteredController {
            schema: schema("ingestion_status").unwrap(),
            handler: handle_ingestion_status,
        },
    ]
}

pub(super) fn schema(function: &str) -> Option<ControllerSchema> {
    Some(match function {
        "sync_channel" => ControllerSchema {
            namespace: "memory",
            function: "sync_channel",
            description: "Request a memory sync for a specific channel. Publishes MemorySyncRequested on the event bus. No ingestion consumers exist yet; this is a hook for future pull-based subscribers.",
            inputs: vec![FieldSchema {
                name: "channel_id",
                ty: TypeSchema::String,
                comment: "ID of the channel to sync.",
                required: true,
            }],
            outputs: vec![
                FieldSchema { name: "requested", ty: TypeSchema::Bool, comment: "Always true when the event was published.", required: true },
                FieldSchema { name: "channel_id", ty: TypeSchema::String, comment: "Echo of the channel_id that was requested.", required: true },
            ],
        },
        "sync_all" => ControllerSchema {
            namespace: "memory",
            function: "sync_all",
            description: "Request a memory sync for all channels. Publishes MemorySyncRequested { channel_id: None } on the event bus.",
            inputs: vec![],
            outputs: vec![FieldSchema { name: "requested", ty: TypeSchema::Bool, comment: "Always true when the event was published.", required: true }],
        },
        "ingestion_status" => ControllerSchema {
            namespace: "memory",
            function: "ingestion_status",
            description: "Returns the current memory-ingestion status (whether a job is running, the in-flight document, queue depth, and most recent completion). Safe to poll.",
            inputs: vec![],
            outputs: vec![
                FieldSchema { name: "running", ty: TypeSchema::Bool, comment: "True while an ingestion job is running on the local extraction LLM.", required: true },
                FieldSchema { name: "current_document_id", ty: TypeSchema::Option(Box::new(TypeSchema::String)), comment: "Document id of the in-flight job.", required: false },
                FieldSchema { name: "current_title", ty: TypeSchema::Option(Box::new(TypeSchema::String)), comment: "Title of the in-flight document.", required: false },
                FieldSchema { name: "current_namespace", ty: TypeSchema::Option(Box::new(TypeSchema::String)), comment: "Namespace of the in-flight document.", required: false },
                FieldSchema { name: "queue_depth", ty: TypeSchema::U64, comment: "Number of jobs waiting behind the current one.", required: true },
                FieldSchema { name: "last_completed_at", ty: TypeSchema::Option(Box::new(TypeSchema::I64)), comment: "Unix-ms timestamp of the most recent completion.", required: false },
                FieldSchema { name: "last_document_id", ty: TypeSchema::Option(Box::new(TypeSchema::String)), comment: "Document id of the most recent completed job.", required: false },
                FieldSchema { name: "last_success", ty: TypeSchema::Option(Box::new(TypeSchema::Bool)), comment: "Whether the most recent job succeeded.", required: false },
            ],
        },
        _ => return None,
    })
}

fn handle_sync_channel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = parse_params::<SyncChannelParams>(params)?;
        to_json(rpc::memory_sync_channel(payload).await?)
    })
}

fn handle_sync_all(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::memory_sync_all().await?) })
}

fn handle_ingestion_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::memory_ingestion_status().await?) })
}
