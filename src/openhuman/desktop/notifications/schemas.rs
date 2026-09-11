//! Controller schema definitions and registered handlers for the
//! `notifications` domain.
//!
//! Follows the exact pattern from `src/openhuman/cron/schemas.rs`.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

type SchemaBuilder = fn() -> ControllerSchema;
type ControllerHandler = fn(Map<String, Value>) -> ControllerFuture;

struct NotificationControllerDef {
    function: &'static str,
    schema: SchemaBuilder,
    handler: ControllerHandler,
}

// ─────────────────────────────────────────────────────────────────────────────
// Schema registry
// ─────────────────────────────────────────────────────────────────────────────

const NOTIFICATION_CONTROLLER_DEFS: &[NotificationControllerDef] = &[
    NotificationControllerDef {
        function: "ingest",
        schema: schema_ingest,
        handler: handle_ingest_wrap,
    },
    NotificationControllerDef {
        function: "list",
        schema: schema_list,
        handler: handle_list_wrap,
    },
    NotificationControllerDef {
        function: "mark_read",
        schema: schema_mark_read,
        handler: handle_mark_read_wrap,
    },
    NotificationControllerDef {
        function: "settings_get",
        schema: schema_settings_get,
        handler: handle_settings_get_wrap,
    },
    NotificationControllerDef {
        function: "settings_set",
        schema: schema_settings_set,
        handler: handle_settings_set_wrap,
    },
    NotificationControllerDef {
        function: "dismiss",
        schema: schema_dismiss,
        handler: handle_dismiss_wrap,
    },
    NotificationControllerDef {
        function: "mark_acted",
        schema: schema_mark_acted,
        handler: handle_mark_acted_wrap,
    },
    NotificationControllerDef {
        function: "stats",
        schema: schema_stats,
        handler: handle_stats_wrap,
    },
    NotificationControllerDef {
        function: "core_list",
        schema: schema_core_list,
        handler: handle_core_list_wrap,
    },
    NotificationControllerDef {
        function: "core_mark_read",
        schema: schema_core_mark_read,
        handler: handle_core_mark_read_wrap,
    },
];

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    NOTIFICATION_CONTROLLER_DEFS
        .iter()
        .map(|def| (def.schema)())
        .collect()
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    NOTIFICATION_CONTROLLER_DEFS
        .iter()
        .map(|def| RegisteredController {
            schema: (def.schema)(),
            handler: def.handler,
        })
        .collect()
}

pub fn schemas(function: &str) -> ControllerSchema {
    if let Some(def) = NOTIFICATION_CONTROLLER_DEFS
        .iter()
        .find(|def| def.function == function)
    {
        return (def.schema)();
    }

    schema_unknown()
}

fn schema_ingest() -> ControllerSchema {
    ControllerSchema {
        namespace: "notification",
        function: "ingest",
        description: "Ingest a new notification from an embedded webview integration. \
                          Immediately persists the record and kicks off background triage scoring.",
        inputs: vec![
            FieldSchema {
                name: "provider",
                ty: TypeSchema::String,
                comment: "Provider slug, e.g. \"gmail\", \"slack\", \"whatsapp\".",
                required: true,
            },
            FieldSchema {
                name: "account_id",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Webview account identifier (optional).",
                required: false,
            },
            FieldSchema {
                name: "title",
                ty: TypeSchema::String,
                comment: "Short notification title / subject.",
                required: true,
            },
            FieldSchema {
                name: "body",
                ty: TypeSchema::String,
                comment: "Notification body or preview text.",
                required: true,
            },
            FieldSchema {
                name: "raw_payload",
                ty: TypeSchema::Ref("JsonObject"),
                comment: "Full raw event payload from the source for downstream use.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "id",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "UUID of the newly created notification record. Absent when skipped.",
                required: false,
            },
            FieldSchema {
                name: "skipped",
                ty: TypeSchema::Bool,
                comment: "True when the provider is disabled and the notification was not stored.",
                required: true,
            },
            FieldSchema {
                name: "reason",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Human-readable reason populated alongside `skipped=true` \
                              (e.g. \"provider_disabled\").",
                required: false,
            },
        ],
    }
}

fn schema_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "notification",
        function: "list",
        description: "Return a paginated list of ingested notifications with optional \
                          provider and minimum-importance-score filters.",
        inputs: vec![
            FieldSchema {
                name: "provider",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Filter by provider slug. Omit to return all providers.",
                required: false,
            },
            FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum number of records to return; defaults to 50.",
                required: false,
            },
            FieldSchema {
                name: "offset",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Number of records to skip for pagination; defaults to 0.",
                required: false,
            },
            FieldSchema {
                name: "min_score",
                ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                comment: "Minimum importance score 0.0–1.0. Unscored items pass through.",
                required: false,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "items",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("IntegrationNotification"))),
                comment: "Notification records ordered by received_at descending.",
                required: true,
            },
            FieldSchema {
                name: "unread_count",
                ty: TypeSchema::I64,
                comment: "Total count of unread notifications across all providers.",
                required: true,
            },
        ],
    }
}

fn schema_mark_read() -> ControllerSchema {
    ControllerSchema {
        namespace: "notification",
        function: "mark_read",
        description: "Mark a single notification as read by its id.",
        inputs: vec![FieldSchema {
            name: "id",
            ty: TypeSchema::String,
            comment: "UUID of the notification to mark as read.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "ok",
            ty: TypeSchema::Bool,
            comment: "True when the update succeeded.",
            required: true,
        }],
    }
}

fn schema_settings_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "notification",
        function: "settings_get",
        description: "Get provider-level notification routing settings.",
        inputs: vec![FieldSchema {
            name: "provider",
            ty: TypeSchema::String,
            comment: "Provider slug, e.g. \"gmail\".",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "settings",
            ty: TypeSchema::Ref("NotificationSettings"),
            comment: "Current settings for provider, defaulted if missing.",
            required: true,
        }],
    }
}

fn schema_settings_set() -> ControllerSchema {
    ControllerSchema {
        namespace: "notification",
        function: "settings_set",
        description: "Upsert provider-level notification routing settings.",
        inputs: vec![
            FieldSchema {
                name: "provider",
                ty: TypeSchema::String,
                comment: "Provider slug, e.g. \"gmail\".",
                required: true,
            },
            FieldSchema {
                name: "enabled",
                ty: TypeSchema::Bool,
                comment: "Enable/disable ingestion for this provider.",
                required: true,
            },
            FieldSchema {
                name: "importance_threshold",
                ty: TypeSchema::F64,
                comment: "Minimum score 0.0..1.0 for routing decisions.",
                required: true,
            },
            FieldSchema {
                name: "route_to_orchestrator",
                ty: TypeSchema::Bool,
                comment: "When true, allow triage react/escalate to route to orchestrator.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when settings were saved.",
                required: true,
            },
            FieldSchema {
                name: "settings",
                ty: TypeSchema::Ref("NotificationSettings"),
                comment: "The normalized (clamped) settings that were persisted.",
                required: true,
            },
        ],
    }
}

fn schema_dismiss() -> ControllerSchema {
    ControllerSchema {
        namespace: "notification",
        function: "dismiss",
        description: "Mark a notification as dismissed (user explicitly hid it).",
        inputs: vec![FieldSchema {
            name: "id",
            ty: TypeSchema::String,
            comment: "UUID of the notification to dismiss.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "ok",
            ty: TypeSchema::Bool,
            comment: "True when the update succeeded.",
            required: true,
        }],
    }
}

fn schema_mark_acted() -> ControllerSchema {
    ControllerSchema {
        namespace: "notification",
        function: "mark_acted",
        description: "Mark a notification as acted upon (user took an action from it).",
        inputs: vec![FieldSchema {
            name: "id",
            ty: TypeSchema::String,
            comment: "UUID of the notification to mark as acted.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "ok",
            ty: TypeSchema::Bool,
            comment: "True when the update succeeded.",
            required: true,
        }],
    }
}

fn schema_stats() -> ControllerSchema {
    ControllerSchema {
        namespace: "notification",
        function: "stats",
        description: "Return aggregate statistics for the notification intelligence pipeline.",
        inputs: vec![],
        outputs: vec![
            FieldSchema {
                name: "total",
                ty: TypeSchema::I64,
                comment: "Total notification count.",
                required: true,
            },
            FieldSchema {
                name: "unread",
                ty: TypeSchema::I64,
                comment: "Count of unread notifications.",
                required: true,
            },
            FieldSchema {
                name: "unscored",
                ty: TypeSchema::I64,
                comment: "Count of notifications pending triage scoring.",
                required: true,
            },
            FieldSchema {
                name: "by_provider",
                ty: TypeSchema::Map(Box::new(TypeSchema::I64)),
                comment: "Notification counts grouped by provider slug.",
                required: true,
            },
            FieldSchema {
                name: "by_action",
                ty: TypeSchema::Map(Box::new(TypeSchema::I64)),
                comment: "Notification counts grouped by triage action.",
                required: true,
            },
        ],
    }
}

fn schema_core_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "notification",
        function: "core_list",
        description: "List persisted core notifications (cron, webhook, sub-agent, triage) so \
                      the frontend can sync down events fired while the app was closed (#3805).",
        inputs: vec![
            FieldSchema {
                name: "only_unread",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "Return only unread notifications. Defaults to true.",
                required: false,
            },
            FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum number of records to return; defaults to 100.",
                required: false,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "items",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("CoreNotificationEvent"))),
                comment: "Persisted core notifications, newest first.",
                required: true,
            },
            FieldSchema {
                name: "unread_count",
                ty: TypeSchema::I64,
                comment: "Total count of unread core notifications.",
                required: true,
            },
        ],
    }
}

fn schema_core_mark_read() -> ControllerSchema {
    ControllerSchema {
        namespace: "notification",
        function: "core_mark_read",
        description: "Mark a persisted core notification as read by its id (#3805).",
        inputs: vec![FieldSchema {
            name: "id",
            ty: TypeSchema::String,
            comment: "Id of the core notification to mark as read.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "ok",
            ty: TypeSchema::Bool,
            comment: "True when a matching notification was updated.",
            required: true,
        }],
    }
}

fn schema_unknown() -> ControllerSchema {
    ControllerSchema {
        namespace: "notification",
        function: "unknown",
        description: "Unknown notification controller function.",
        inputs: vec![FieldSchema {
            name: "function",
            ty: TypeSchema::String,
            comment: "Unknown function requested.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "error",
            ty: TypeSchema::String,
            comment: "Lookup error details.",
            required: true,
        }],
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Handler wrappers (delegate to rpc.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn handle_ingest_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_ingest(params).await })
}

fn handle_list_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_list(params).await })
}

fn handle_mark_read_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_mark_read(params).await })
}

fn handle_settings_get_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_settings_get(params).await })
}

fn handle_settings_set_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_settings_set(params).await })
}

fn handle_dismiss_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_dismiss(params).await })
}

fn handle_mark_acted_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_mark_acted(params).await })
}

fn handle_stats_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_stats(params).await })
}

fn handle_core_list_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_core_list(params).await })
}

fn handle_core_mark_read_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_core_mark_read(params).await })
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
