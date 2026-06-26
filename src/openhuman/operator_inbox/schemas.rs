//! Controller schemas for `operator_inbox` domain.
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use serde_json::{Map, Value};

type SB = fn() -> ControllerSchema;
type CH = fn(Map<String, Value>) -> ControllerFuture;
struct Def {
    function: &'static str,
    schema: SB,
    handler: CH,
}

const DEFS: &[Def] = &[
    Def {
        function: "triage_message",
        schema: s_triage,
        handler: h_triage,
    },
    Def {
        function: "generate_draft",
        schema: s_draft,
        handler: h_draft,
    },
    Def {
        function: "schedule_followup",
        schema: s_followup,
        handler: h_followup,
    },
    Def {
        function: "get_triage",
        schema: s_get,
        handler: h_get,
    },
    Def {
        function: "list_triage",
        schema: s_list,
        handler: h_list,
    },
    Def {
        function: "archive",
        schema: s_archive,
        handler: h_archive,
    },
    Def {
        function: "fetch_inbox",
        schema: s_fetch_inbox,
        handler: h_fetch_inbox,
    },
    Def {
        function: "send_reply",
        schema: s_send_reply,
        handler: h_send_reply,
    },
    Def {
        function: "start_poller",
        schema: s_start_poller,
        handler: h_start_poller,
    },
    Def {
        function: "stop_poller",
        schema: s_stop_poller,
        handler: h_stop_poller,
    },
];

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    DEFS.iter().map(|d| (d.schema)()).collect()
}
pub fn all_registered_controllers() -> Vec<RegisteredController> {
    DEFS.iter()
        .map(|d| RegisteredController {
            schema: (d.schema)(),
            handler: d.handler,
        })
        .collect()
}
pub fn schemas(function: &str) -> ControllerSchema {
    DEFS.iter()
        .find(|d| d.function == function)
        .map(|d| (d.schema)())
        .unwrap_or_else(s_unknown)
}

fn s_triage() -> ControllerSchema {
    ControllerSchema {
        namespace: "operator_inbox",
        function: "triage_message",
        description: "Triage an incoming message and score priority.",
        inputs: vec![
            FieldSchema {
                name: "source",
                ty: TypeSchema::String,
                comment: "email|chat|social|webhook.",
                required: false,
            },
            FieldSchema {
                name: "sender",
                ty: TypeSchema::String,
                comment: "Sender.",
                required: true,
            },
            FieldSchema {
                name: "subject",
                ty: TypeSchema::String,
                comment: "Subject.",
                required: true,
            },
            FieldSchema {
                name: "body",
                ty: TypeSchema::String,
                comment: "Body.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "triage_id",
                ty: TypeSchema::String,
                comment: "ID.",
                required: true,
            },
            FieldSchema {
                name: "priority",
                ty: TypeSchema::String,
                comment: "Priority.",
                required: true,
            },
        ],
    }
}

fn s_draft() -> ControllerSchema {
    ControllerSchema {
        namespace: "operator_inbox",
        function: "generate_draft",
        description: "Generate a reply draft.",
        inputs: vec![
            FieldSchema {
                name: "triage_id",
                ty: TypeSchema::String,
                comment: "Triage ID.",
                required: true,
            },
            FieldSchema {
                name: "tone",
                ty: TypeSchema::String,
                comment: "professional|casual|formal.",
                required: false,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "content",
                ty: TypeSchema::String,
                comment: "Draft content.",
                required: true,
            },
        ],
    }
}

fn s_followup() -> ControllerSchema {
    ControllerSchema {
        namespace: "operator_inbox",
        function: "schedule_followup",
        description: "Schedule a follow-up.",
        inputs: vec![
            FieldSchema {
                name: "triage_id",
                ty: TypeSchema::String,
                comment: "Triage ID.",
                required: true,
            },
            FieldSchema {
                name: "follow_up_at",
                ty: TypeSchema::U64,
                comment: "Unix timestamp.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "follow_up_at",
                ty: TypeSchema::U64,
                comment: "Scheduled time.",
                required: true,
            },
        ],
    }
}

fn s_get() -> ControllerSchema {
    ControllerSchema {
        namespace: "operator_inbox",
        function: "get_triage",
        description: "Get triage record.",
        inputs: vec![FieldSchema {
            name: "triage_id",
            ty: TypeSchema::String,
            comment: "ID.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "triage_id",
                ty: TypeSchema::String,
                comment: "ID.",
                required: true,
            },
            FieldSchema {
                name: "priority",
                ty: TypeSchema::String,
                comment: "Priority.",
                required: true,
            },
        ],
    }
}

fn s_list() -> ControllerSchema {
    ControllerSchema {
        namespace: "operator_inbox",
        function: "list_triage",
        description: "List all triage records.",
        inputs: vec![],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "records",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Records.",
                required: true,
            },
        ],
    }
}

fn s_archive() -> ControllerSchema {
    ControllerSchema {
        namespace: "operator_inbox",
        function: "archive",
        description: "Archive a triage record.",
        inputs: vec![FieldSchema {
            name: "triage_id",
            ty: TypeSchema::String,
            comment: "ID.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "status",
                ty: TypeSchema::String,
                comment: "New status.",
                required: true,
            },
        ],
    }
}

fn s_unknown() -> ControllerSchema {
    ControllerSchema {
        namespace: "operator_inbox",
        function: "unknown",
        description: "Unknown.",
        inputs: vec![FieldSchema {
            name: "function",
            ty: TypeSchema::String,
            comment: "Requested.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "error",
            ty: TypeSchema::String,
            comment: "Error.",
            required: true,
        }],
    }
}

fn h_triage(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_triage_message(p).await })
}
fn h_draft(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_generate_draft(p).await })
}
fn h_followup(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_schedule_followup(p).await })
}
fn h_get(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_get_triage(p).await })
}
fn h_list(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_list_triage(p).await })
}
fn h_archive(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_archive(p).await })
}
fn h_fetch_inbox(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_fetch_inbox(p).await })
}
fn h_send_reply(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_send_reply(p).await })
}
fn h_start_poller(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_start_poller(p).await })
}
fn h_stop_poller(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_stop_poller(p).await })
}

fn s_start_poller() -> ControllerSchema {
    ControllerSchema {
        namespace: "operator_inbox",
        function: "start_poller",
        description: "Start background IMAP polling loop.",
        inputs: vec![
            FieldSchema {
                name: "host",
                ty: TypeSchema::String,
                comment: "IMAP host.",
                required: true,
            },
            FieldSchema {
                name: "username",
                ty: TypeSchema::String,
                comment: "IMAP username.",
                required: true,
            },
            FieldSchema {
                name: "password",
                ty: TypeSchema::String,
                comment: "IMAP password.",
                required: true,
            },
            FieldSchema {
                name: "interval_secs",
                ty: TypeSchema::U64,
                comment: "Poll interval in seconds. Default: 120.",
                required: false,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "started",
                ty: TypeSchema::Bool,
                comment: "Whether poller was started (false if already running).",
                required: true,
            },
        ],
    }
}

fn s_stop_poller() -> ControllerSchema {
    ControllerSchema {
        namespace: "operator_inbox",
        function: "stop_poller",
        description: "Stop background IMAP polling loop.",
        inputs: vec![],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "was_running",
                ty: TypeSchema::Bool,
                comment: "Whether poller was running.",
                required: true,
            },
        ],
    }
}

fn s_fetch_inbox() -> ControllerSchema {
    ControllerSchema {
        namespace: "operator_inbox",
        function: "fetch_inbox",
        description: "Fetch new emails from configured IMAP server and auto-triage them.",
        inputs: vec![
            FieldSchema {
                name: "host",
                ty: TypeSchema::String,
                comment: "IMAP host.",
                required: true,
            },
            FieldSchema {
                name: "port",
                ty: TypeSchema::U64,
                comment: "IMAP port (993 for TLS).",
                required: false,
            },
            FieldSchema {
                name: "username",
                ty: TypeSchema::String,
                comment: "IMAP username.",
                required: true,
            },
            FieldSchema {
                name: "password",
                ty: TypeSchema::String,
                comment: "IMAP password.",
                required: true,
            },
            FieldSchema {
                name: "mailbox",
                ty: TypeSchema::String,
                comment: "Mailbox name. Default: INBOX.",
                required: false,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "fetched",
                ty: TypeSchema::U64,
                comment: "Emails fetched.",
                required: true,
            },
            FieldSchema {
                name: "triaged",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Triage results for each email.",
                required: true,
            },
        ],
    }
}

fn s_send_reply() -> ControllerSchema {
    ControllerSchema {
        namespace: "operator_inbox",
        function: "send_reply",
        description: "Send a drafted reply via SMTP.",
        inputs: vec![
            FieldSchema {
                name: "triage_id",
                ty: TypeSchema::String,
                comment: "Triage record to reply to.",
                required: true,
            },
            FieldSchema {
                name: "smtp_host",
                ty: TypeSchema::String,
                comment: "SMTP host.",
                required: true,
            },
            FieldSchema {
                name: "smtp_port",
                ty: TypeSchema::U64,
                comment: "SMTP port (587 for STARTTLS).",
                required: false,
            },
            FieldSchema {
                name: "username",
                ty: TypeSchema::String,
                comment: "SMTP username.",
                required: true,
            },
            FieldSchema {
                name: "password",
                ty: TypeSchema::String,
                comment: "SMTP password.",
                required: true,
            },
            FieldSchema {
                name: "from",
                ty: TypeSchema::String,
                comment: "From address.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Success.",
                required: true,
            },
            FieldSchema {
                name: "message_id",
                ty: TypeSchema::String,
                comment: "Sent message ID.",
                required: true,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handlers_match() {
        assert_eq!(
            all_controller_schemas().len(),
            all_registered_controllers().len()
        );
        assert_eq!(all_controller_schemas().len(), 10);
    }
    #[test]
    fn namespace() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "operator_inbox");
        }
    }
    #[test]
    fn unknown() {
        assert_eq!(schemas("nope").function, "unknown");
    }
}
