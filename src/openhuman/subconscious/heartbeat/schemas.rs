use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("settings_get"),
        schemas("settings_set"),
        schemas("tick_now"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("settings_get"),
            handler: handle_settings_get,
        },
        RegisteredController {
            schema: schemas("settings_set"),
            handler: handle_settings_set,
        },
        RegisteredController {
            schema: schemas("tick_now"),
            handler: handle_tick_now,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "settings_get" => ControllerSchema {
            namespace: "heartbeat",
            function: "settings_get",
            description: "Read heartbeat proactive notification settings.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "settings",
                ty: TypeSchema::Json,
                comment: "Current heartbeat settings.",
                required: true,
            }],
        },
        "settings_set" => ControllerSchema {
            namespace: "heartbeat",
            function: "settings_set",
            description: "Update heartbeat proactive notification settings.",
            inputs: vec![
                optional_bool("enabled", "Enable or disable heartbeat loop."),
                optional_u64("interval_minutes", "Tick interval in minutes."),
                optional_bool(
                    "inference_enabled",
                    "Enable subconscious inference during heartbeat ticks.",
                ),
                optional_bool(
                    "notify_meetings",
                    "Enable proactive notifications for upcoming meetings.",
                ),
                optional_bool(
                    "notify_reminders",
                    "Enable proactive notifications for reminders.",
                ),
                optional_bool(
                    "notify_relevant_events",
                    "Enable proactive notifications for urgent/relevant events.",
                ),
                optional_bool(
                    "external_delivery_enabled",
                    "Allow proactive delivery to external active channels.",
                ),
                optional_u64(
                    "meeting_lookahead_minutes",
                    "Max lookahead window (minutes) for meeting notifications.",
                ),
                optional_u64(
                    "max_calendar_connections_per_tick",
                    "Max active calendar connections polled per planner tick.",
                ),
                optional_u64(
                    "reminder_lookahead_minutes",
                    "Max lookahead window (minutes) for reminder notifications.",
                ),
                optional_string(
                    "subconscious_mode",
                    "Subconscious operating mode: off, simple, aggressive, or event_driven.",
                ),
                optional_bool(
                    "triggers_enabled",
                    "Enable the event-driven subconscious trigger pipeline.",
                ),
                optional_u64(
                    "max_promotions_per_hour",
                    "Per-hour cap on trigger promotions (long-lived session runs).",
                ),
            ],
            outputs: vec![FieldSchema {
                name: "settings",
                ty: TypeSchema::Json,
                comment: "Updated heartbeat settings.",
                required: true,
            }],
        },
        "tick_now" => ControllerSchema {
            namespace: "heartbeat",
            function: "tick_now",
            description:
                "Run one immediate heartbeat planner tick for proactive event notifications.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "summary",
                ty: TypeSchema::Json,
                comment: "Planner tick result summary.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "heartbeat",
            function: "unknown",
            description: "Unknown heartbeat controller.",
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

fn handle_settings_get(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        crate::openhuman::subconscious::heartbeat::rpc::settings_get()
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_settings_set(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let patch: crate::openhuman::subconscious::heartbeat::rpc::HeartbeatSettingsPatch =
            serde_json::from_value(Value::Object(params))
                .map_err(|e| format!("invalid heartbeat settings_set params: {e}"))?;
        crate::openhuman::subconscious::heartbeat::rpc::settings_set(patch)
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_tick_now(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        crate::openhuman::subconscious::heartbeat::rpc::tick_now()
            .await?
            .into_cli_compatible_json()
    })
}

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment,
        required: false,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}
