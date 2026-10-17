//! Per-function [`ControllerSchema`] definitions for the `threads` namespace.

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub(crate) fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "threads",
            function: "list",
            description: "List conversation threads.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with thread summaries and count.",
                required: true,
            }],
        },
        "upsert" => ControllerSchema {
            namespace: "threads",
            function: "upsert",
            description: "Create or refresh a conversation thread.",
            inputs: vec![
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::String,
                    comment: "Stable thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "title",
                    ty: TypeSchema::String,
                    comment: "Human-readable thread title.",
                    required: true,
                },
                FieldSchema {
                    name: "created_at",
                    ty: TypeSchema::String,
                    comment: "RFC3339 timestamp for first thread creation.",
                    required: true,
                },
                FieldSchema {
                    name: "labels",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional list of labels to assign.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the resulting thread summary.",
                required: true,
            }],
        },
        "create_new" => ControllerSchema {
            namespace: "threads",
            function: "create_new",
            description: "Create a new conversation thread with auto-generated ID and title.",
            inputs: vec![
                FieldSchema {
                    name: "labels",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Optional labels to assign to the new thread.",
                    required: false,
                },
                FieldSchema {
                    name: "action_dir",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional absolute working folder for the thread's agent; empty or absent uses the global action_dir.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the created thread summary.",
                required: true,
            }],
        },
        "messages_list" => ControllerSchema {
            namespace: "threads",
            function: "messages_list",
            description: "List messages for a conversation thread.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with messages and count.",
                required: true,
            }],
        },
        "message_append" => ControllerSchema {
            namespace: "threads",
            function: "message_append",
            description: "Append a message to a conversation thread.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "message",
                    ty: TypeSchema::Json,
                    comment: "Message payload to append.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the appended message payload.",
                required: true,
            }],
        },
        "message_update" => ControllerSchema {
            namespace: "threads",
            function: "message_update",
            description: "Patch metadata on an existing conversation message.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "message_id",
                    ty: TypeSchema::String,
                    comment: "Message identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "extra_metadata",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Replacement message metadata object.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the updated message payload.",
                required: true,
            }],
        },
        "generate_title" => ControllerSchema {
            namespace: "threads",
            function: "generate_title",
            description:
                "Generate a short thread title from the first user message and assistant reply.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "assistant_message",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment:
                        "Optional completed assistant reply to use instead of the stored first agent message.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the resulting thread summary.",
                required: true,
            }],
        },
        "update_labels" => ControllerSchema {
            namespace: "threads",
            function: "update_labels",
            description: "Update labels for a conversation thread.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "labels",
                    ty: TypeSchema::Json,
                    comment: "List of labels to assign.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the resulting thread summary.",
                required: true,
            }],
        },
        "update_working_dir" => ControllerSchema {
            namespace: "threads",
            function: "update_working_dir",
            description: "Bind or clear the working folder of a thread that has no messages yet.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "action_dir",
                    ty: TypeSchema::String,
                    comment: "Absolute folder path; empty clears back to the global action_dir.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the resulting thread summary.",
                required: true,
            }],
        },
        "update_title" => ControllerSchema {
            namespace: "threads",
            function: "update_title",
            description: "Set a user-specified title on a conversation thread.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "title",
                    ty: TypeSchema::String,
                    comment: "New title for the thread.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the resulting thread summary.",
                required: true,
            }],
        },
        "delete" => ControllerSchema {
            namespace: "threads",
            function: "delete",
            description: "Delete a conversation thread and its message log.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "deleted_at",
                    ty: TypeSchema::String,
                    comment: "RFC3339 deletion timestamp.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with deletion status.",
                required: true,
            }],
        },
        "purge" => ControllerSchema {
            namespace: "threads",
            function: "purge",
            description: "Remove all conversation threads and messages.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with deleted thread/message counts.",
                required: true,
            }],
        },
        "turn_state_get" => ControllerSchema {
            namespace: "threads",
            function: "turn_state_get",
            description: "Fetch the persisted in-flight turn snapshot for a thread, if any.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope wrapping the turn state (may be null).",
                required: true,
            }],
        },
        "turn_state_list" => ControllerSchema {
            namespace: "threads",
            function: "turn_state_list",
            description:
                "List every persisted turn snapshot — used to surface interrupted turns on cold boot.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the list of turn snapshots and a count.",
                required: true,
            }],
        },
        "turn_state_history" => ControllerSchema {
            namespace: "threads",
            function: "turn_state_history",
            description:
                "List every persisted turn snapshot for one thread, newest first — the per-turn process history.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the thread's turn snapshots and a count.",
                required: true,
            }],
        },
        "turn_state_get_turn" => ControllerSchema {
            namespace: "threads",
            function: "turn_state_get_turn",
            description: "Fetch one specific turn of a thread by its producing request id.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "request_id",
                    ty: TypeSchema::String,
                    comment: "Producing request id of the turn.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope wrapping the turn state (may be null).",
                required: true,
            }],
        },
        "turn_state_clear" => ControllerSchema {
            namespace: "threads",
            function: "turn_state_clear",
            description: "Delete the persisted turn snapshot for a thread.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope reporting whether a snapshot was removed.",
                required: true,
            }],
        },
        "token_usage" => ControllerSchema {
            namespace: "threads",
            function: "token_usage",
            description: "Total a thread's persisted token/cost usage from its session transcripts.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the thread's token/cost totals (zeros when no turns yet).",
                required: true,
            }],
        },
        "transcript_get" => ControllerSchema {
            namespace: "threads",
            function: "transcript_get",
            description:
                "Project a thread's settled transcript (read from the session store) into typed display items, newest-first paginated.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "cursor",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Opaque pagination cursor from a prior page's nextCursor; absent starts at the newest item.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max items to return (default 50, capped at 500).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with the newest-first page of display items, total, and nextCursor.",
                required: true,
            }],
        },
        "goal_get" => ControllerSchema {
            namespace: "threads",
            function: "goal_get",
            description:
                "Read a thread's current goal (Codex-style completion contract), or null when it has none.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope wrapping the goal (may be null).",
                required: true,
            }],
        },
        "todos_get" => ControllerSchema {
            namespace: "threads",
            function: "todos_get",
            description: "Read a thread's current session todo list.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread identifier.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope wrapping the todo list (empty when never written).",
                required: true,
            }],
        },
        "edit_message" => ControllerSchema {
            namespace: "threads",
            function: "edit_message",
            description:
                "Edit a past user message: cancel the thread's in-flight turn, drop that message and everything after it, and restart the turn with the new content.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "message_id",
                    ty: TypeSchema::String,
                    comment: "Id of the user message to edit (from threads.messages_list).",
                    required: true,
                },
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "Replacement message content.",
                    required: true,
                },
                FieldSchema {
                    name: "client_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Socket client id to attribute the restarted turn to.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "request_id",
                ty: TypeSchema::String,
                comment: "Request id of the restarted turn.",
                required: true,
            }],
        },
        "regenerate" => ControllerSchema {
            namespace: "threads",
            function: "regenerate",
            description:
                "Regenerate a past assistant reply (or, with no message_id, the thread's last turn): cancel the in-flight turn, drop the answer and everything after it, and restart with the same prompt.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "Thread identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "message_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Id of the assistant reply to regenerate (from threads.messages_list); omit to regenerate the last turn.",
                    required: false,
                },
                FieldSchema {
                    name: "client_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Socket client id to attribute the restarted turn to.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "request_id",
                ty: TypeSchema::String,
                comment: "Request id of the restarted turn.",
                required: true,
            }],
        },
        "search" => ControllerSchema {
            namespace: "threads",
            function: "search",
            description:
                "Search message text across every thread (global search). Hits are newest first, each with a snippet around the match.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Text to find; blank returns no hits.",
                    required: true,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max hits (default 20, capped at 100).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Envelope with hits: threadId, messageId, role, snippet, createdAt.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "threads",
            function: "unknown",
            description: "Unknown threads controller function.",
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
