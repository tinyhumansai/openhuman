//! Controller schema metadata and registration for the Gmail domain.
//!
//! Follows the same pattern as `src/openhuman/cron/schemas.rs`.

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::gmail::rpc;

// ---------------------------------------------------------------------------
// Public registry entry-points (consumed by src/core/all.rs)
// ---------------------------------------------------------------------------

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list_accounts"),
        schemas("connect_account"),
        schemas("disconnect_account"),
        schemas("sync_now"),
        schemas("get_stats"),
        schemas("ingest_raw_response"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list_accounts"),
            handler: rpc::handle_list_accounts,
        },
        RegisteredController {
            schema: schemas("connect_account"),
            handler: rpc::handle_connect_account,
        },
        RegisteredController {
            schema: schemas("disconnect_account"),
            handler: rpc::handle_disconnect_account,
        },
        RegisteredController {
            schema: schemas("sync_now"),
            handler: rpc::handle_sync_now,
        },
        RegisteredController {
            schema: schemas("get_stats"),
            handler: rpc::handle_get_stats,
        },
        RegisteredController {
            schema: schemas("ingest_raw_response"),
            handler: rpc::handle_ingest_raw_response,
        },
    ]
}

// ---------------------------------------------------------------------------
// Schema definitions
// ---------------------------------------------------------------------------

fn account_id_field(comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: "account_id",
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn stats_output() -> FieldSchema {
    FieldSchema {
        name: "stats",
        ty: TypeSchema::Ref("GmailSyncStats"),
        comment: "Sync stats for the requested Gmail account.",
        required: true,
    }
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list_accounts" => ControllerSchema {
            namespace: "gmail",
            function: "list_accounts",
            description: "List all connected Gmail accounts with their last-sync stats.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "accounts",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("GmailSyncStats"))),
                comment: "Connected Gmail accounts ordered by connected_at desc.",
                required: true,
            }],
        },

        "connect_account" => ControllerSchema {
            namespace: "gmail",
            function: "connect_account",
            description: "Register a Gmail account and start a 15-minute recurring sync job.",
            inputs: vec![
                account_id_field("Opaque stable account identifier (matches webview account_id)."),
                FieldSchema {
                    name: "email",
                    ty: TypeSchema::String,
                    comment: "Google account email address.",
                    required: true,
                },
            ],
            outputs: vec![stats_output()],
        },

        "disconnect_account" => ControllerSchema {
            namespace: "gmail",
            function: "disconnect_account",
            description:
                "Disconnect a Gmail account: cancel cron job, wipe memory namespace, remove record.",
            inputs: vec![account_id_field(
                "Identifier of the Gmail account to disconnect.",
            )],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "account_id",
                            ty: TypeSchema::String,
                            comment: "Account that was disconnected.",
                            required: true,
                        },
                        FieldSchema {
                            name: "disconnected",
                            ty: TypeSchema::Bool,
                            comment: "Always true on success.",
                            required: true,
                        },
                    ],
                },
                comment: "Disconnection result.",
                required: true,
            }],
        },

        "sync_now" => ControllerSchema {
            namespace: "gmail",
            function: "sync_now",
            description: "Trigger an on-demand sync signal for one Gmail account.",
            inputs: vec![account_id_field("Identifier of the Gmail account to sync.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "account_id",
                            ty: TypeSchema::String,
                            comment: "Account id.",
                            required: true,
                        },
                        FieldSchema {
                            name: "status",
                            ty: TypeSchema::String,
                            comment: "Always 'sync_triggered' on success.",
                            required: true,
                        },
                    ],
                },
                comment: "Sync trigger result.",
                required: true,
            }],
        },

        "get_stats" => ControllerSchema {
            namespace: "gmail",
            function: "get_stats",
            description: "Return sync stats for a single Gmail account.",
            inputs: vec![account_id_field(
                "Identifier of the Gmail account to inspect.",
            )],
            outputs: vec![stats_output()],
        },

        "ingest_raw_response" => ControllerSchema {
            namespace: "gmail",
            function: "ingest_raw_response",
            description: "Ingest a raw Gmail sync response body captured by the CDP MITM scanner. \
                          Strips the JSONP prefix if present, parses the envelope, extracts \
                          GmailMessage records, and calls ingest_batch.",
            inputs: vec![
                account_id_field("Identifier of the Gmail account the body belongs to."),
                FieldSchema {
                    name: "url",
                    ty: TypeSchema::String,
                    comment: "The request URL (for source attribution).",
                    required: true,
                },
                FieldSchema {
                    name: "body",
                    ty: TypeSchema::String,
                    comment: "Raw response body string (may have JSONP prefix).",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "ingested",
                            ty: TypeSchema::I64,
                            comment: "Number of messages successfully ingested.",
                            required: true,
                        },
                        FieldSchema {
                            name: "errors",
                            ty: TypeSchema::I64,
                            comment: "Number of messages that failed ingestion.",
                            required: true,
                        },
                    ],
                },
                comment: "Ingestion result summary.",
                required: true,
            }],
        },

        _other => ControllerSchema {
            namespace: "gmail",
            function: "unknown",
            description: "Unknown gmail controller function.",
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_controller_schemas_covers_six_functions() {
        let names: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(
            names,
            vec![
                "list_accounts",
                "connect_account",
                "disconnect_account",
                "sync_now",
                "get_stats",
                "ingest_raw_response",
            ]
        );
    }

    #[test]
    fn all_registered_controllers_matches_schema_count() {
        assert_eq!(
            all_controller_schemas().len(),
            all_registered_controllers().len()
        );
    }

    #[test]
    fn connect_account_requires_account_id_and_email() {
        let s = schemas("connect_account");
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert!(required.contains(&"account_id"));
        assert!(required.contains(&"email"));
    }

    #[test]
    fn unknown_function_returns_placeholder() {
        let s = schemas("does-not-exist");
        assert_eq!(s.function, "unknown");
    }
}
