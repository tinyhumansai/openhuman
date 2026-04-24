//! JSON-RPC / CLI schemas for the webview_apis bridge.
//!
//! Each controller is a thin proxy: read typed params out of the
//! incoming JSON, call [`super::client::request`] with the matching
//! bridge method name, return the decoded response.

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::webview_apis::rpc;

// ── registration ────────────────────────────────────────────────────────

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("gmail_list_labels"),
        schemas("gmail_list_messages"),
        schemas("gmail_search"),
        schemas("gmail_get_message"),
        schemas("gmail_send"),
        schemas("gmail_trash"),
        schemas("gmail_add_label"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("gmail_list_labels"),
            handler: rpc::handle_gmail_list_labels,
        },
        RegisteredController {
            schema: schemas("gmail_list_messages"),
            handler: rpc::handle_gmail_list_messages,
        },
        RegisteredController {
            schema: schemas("gmail_search"),
            handler: rpc::handle_gmail_search,
        },
        RegisteredController {
            schema: schemas("gmail_get_message"),
            handler: rpc::handle_gmail_get_message,
        },
        RegisteredController {
            schema: schemas("gmail_send"),
            handler: rpc::handle_gmail_send,
        },
        RegisteredController {
            schema: schemas("gmail_trash"),
            handler: rpc::handle_gmail_trash,
        },
        RegisteredController {
            schema: schemas("gmail_add_label"),
            handler: rpc::handle_gmail_add_label,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    let account = FieldSchema {
        name: "account_id",
        ty: TypeSchema::String,
        comment: "Webview account id (passed to webview_account_open). Disambiguates multi-account setups.",
        required: true,
    };
    let message_id = |c: &'static str| FieldSchema {
        name: "message_id",
        ty: TypeSchema::String,
        comment: c,
        required: true,
    };
    let messages_out = FieldSchema {
        name: "messages",
        ty: TypeSchema::Array(Box::new(TypeSchema::Ref("GmailMessage"))),
        comment: "Matching Gmail messages.",
        required: true,
    };
    match function {
        "gmail_list_labels" => ControllerSchema {
            namespace: "webview_apis",
            function: "gmail_list_labels",
            description: "List Gmail labels (system + user) visible in the sidebar.",
            inputs: vec![account],
            outputs: vec![FieldSchema {
                name: "labels",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("GmailLabel"))),
                comment: "Labels scraped from the live webview via CDP DOM snapshot.",
                required: true,
            }],
        },
        "gmail_list_messages" => ControllerSchema {
            namespace: "webview_apis",
            function: "gmail_list_messages",
            description: "List recent Gmail messages (optionally filtered by label).",
            inputs: vec![
                account,
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::U64,
                    comment: "Maximum number of messages.",
                    required: true,
                },
                FieldSchema {
                    name: "label",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Label id (INBOX, STARRED, …). None = current view.",
                    required: false,
                },
            ],
            outputs: vec![messages_out],
        },
        "gmail_search" => ControllerSchema {
            namespace: "webview_apis",
            function: "gmail_search",
            description: "Run a Gmail search query (same syntax as the web UI).",
            inputs: vec![
                account,
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Gmail search expression, e.g. 'from:x is:unread'.",
                    required: true,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::U64,
                    comment: "Maximum number of results.",
                    required: true,
                },
            ],
            outputs: vec![messages_out],
        },
        "gmail_get_message" => ControllerSchema {
            namespace: "webview_apis",
            function: "gmail_get_message",
            description: "Fetch a single Gmail message by id.",
            inputs: vec![account, message_id("Gmail message id.")],
            outputs: vec![FieldSchema {
                name: "message",
                ty: TypeSchema::Ref("GmailMessage"),
                comment: "The requested Gmail message.",
                required: true,
            }],
        },
        "gmail_send" => ControllerSchema {
            namespace: "webview_apis",
            function: "gmail_send",
            description: "Send a new email via the logged-in Gmail webview.",
            inputs: vec![
                account,
                FieldSchema {
                    name: "request",
                    ty: TypeSchema::Ref("GmailSendRequest"),
                    comment: "Send payload (to/cc/bcc/subject/body).",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "ack",
                ty: TypeSchema::Ref("SendAck"),
                comment: "Send acknowledgement.",
                required: true,
            }],
        },
        "gmail_trash" => ControllerSchema {
            namespace: "webview_apis",
            function: "gmail_trash",
            description: "Move a Gmail message to Trash.",
            inputs: vec![account, message_id("Gmail message id to trash.")],
            outputs: vec![FieldSchema {
                name: "ack",
                ty: TypeSchema::Ref("Ack"),
                comment: "Op acknowledgement.",
                required: true,
            }],
        },
        "gmail_add_label" => ControllerSchema {
            namespace: "webview_apis",
            function: "gmail_add_label",
            description: "Add a label to a Gmail message.",
            inputs: vec![
                account,
                message_id("Gmail message id to label."),
                FieldSchema {
                    name: "label",
                    ty: TypeSchema::String,
                    comment: "Label name to add.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "ack",
                ty: TypeSchema::Ref("Ack"),
                comment: "Op acknowledgement.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "webview_apis",
            function: "unknown",
            description: "Unknown webview_apis controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error.",
                required: true,
            }],
        },
    }
}

// Handler bodies live in `rpc.rs` per project convention —
// `schemas.rs` is registry-only.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_list_covers_every_op() {
        let fns: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(
            fns,
            vec![
                "gmail_list_labels",
                "gmail_list_messages",
                "gmail_search",
                "gmail_get_message",
                "gmail_send",
                "gmail_trash",
                "gmail_add_label"
            ]
        );
    }

    #[test]
    fn every_schema_declares_namespace_webview_apis() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "webview_apis", "op {} wrong ns", s.function);
        }
    }

    #[test]
    fn all_registered_controllers_has_handler_per_schema() {
        assert_eq!(all_registered_controllers().len(), 7);
    }

    // Param-helper coverage moved with the helpers into `rpc.rs` —
    // see the tests there for `require_string` / `require_u32`.
}
