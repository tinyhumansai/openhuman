//! Controller schema definitions and registered handlers for the
//! `agent_meetings` domain.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

type SchemaBuilder = fn() -> ControllerSchema;
type ControllerHandler = fn(Map<String, Value>) -> ControllerFuture;

struct BackendMeetControllerDef {
    function: &'static str,
    schema: SchemaBuilder,
    handler: ControllerHandler,
}

const DEFS: &[BackendMeetControllerDef] = &[
    BackendMeetControllerDef {
        function: "join",
        schema: schema_join,
        handler: handle_join_wrap,
    },
    BackendMeetControllerDef {
        function: "leave",
        schema: schema_leave,
        handler: handle_leave_wrap,
    },
    BackendMeetControllerDef {
        function: "harness_response",
        schema: schema_harness_response,
        handler: handle_harness_response_wrap,
    },
];

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    DEFS.iter().map(|def| (def.schema)()).collect()
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    DEFS.iter()
        .map(|def| RegisteredController {
            schema: (def.schema)(),
            handler: def.handler,
        })
        .collect()
}

fn schema_join() -> ControllerSchema {
    ControllerSchema {
        namespace: "agent_meetings",
        function: "join",
        description: "Ask the backend to join a meeting via Recall.ai bot. Supports \
                      Google Meet, Zoom, Teams, and Webex. Emits bot:join over Socket.IO; \
                      the backend streams events back (bot:reply, bot:harness, bot:transcript, bot:left).",
        inputs: vec![
            FieldSchema {
                name: "meet_url",
                ty: TypeSchema::String,
                comment: "Meeting URL (Google Meet, Zoom, Teams, or Webex).",
                required: true,
            },
            FieldSchema {
                name: "display_name",
                ty: TypeSchema::String,
                comment: "Display name for the bot in the meeting. Defaults to OpenHuman.",
                required: false,
            },
            FieldSchema {
                name: "platform",
                ty: TypeSchema::String,
                comment: "Platform: gmeet, zoom, teams, or webex. Auto-detected from URL if omitted.",
                required: false,
            },
            FieldSchema {
                name: "agent_name",
                ty: TypeSchema::String,
                comment: "Optional AI agent display name forwarded to the backend bot.",
                required: false,
            },
            FieldSchema {
                name: "system_prompt",
                ty: TypeSchema::String,
                comment: "Optional custom meeting system prompt forwarded to the backend bot.",
                required: false,
            },
            FieldSchema {
                name: "mascot_id",
                ty: TypeSchema::String,
                comment: "Optional mascot ID selecting which Rive character appears in the meeting (e.g. \"yellow\").",
                required: false,
            },
            FieldSchema {
                name: "rive_colors",
                ty: TypeSchema::Json,
                comment: "Optional Rive mascot color overrides forwarded to the backend bot.",
                required: false,
            },
            FieldSchema {
                name: "respond_to_participant",
                ty: TypeSchema::String,
                comment: "Only respond to this participant's messages. Case-insensitive substring match \
                          against the speaker name in the transcript. Omit to respond to everyone.",
                required: false,
            },
            FieldSchema {
                name: "wake_phrase",
                ty: TypeSchema::String,
                comment: "Wake phrase the participant must say before the bot responds. \
                          When set, captions without this phrase are silently dropped. \
                          The phrase is stripped before the text reaches the LLM.",
                required: false,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the join request was emitted.",
                required: true,
            },
            FieldSchema {
                name: "meet_url",
                ty: TypeSchema::String,
                comment: "Normalized meeting URL.",
                required: true,
            },
            FieldSchema {
                name: "platform",
                ty: TypeSchema::String,
                comment: "Resolved platform: gmeet, zoom, teams, or webex.",
                required: true,
            },
        ],
    }
}

fn schema_leave() -> ControllerSchema {
    ControllerSchema {
        namespace: "agent_meetings",
        function: "leave",
        description: "Ask the backend bot to leave the current meeting.",
        inputs: vec![FieldSchema {
            name: "reason",
            ty: TypeSchema::String,
            comment: "Optional leave reason. Defaults to 'requested'.",
            required: false,
        }],
        outputs: vec![FieldSchema {
            name: "ok",
            ty: TypeSchema::Bool,
            comment: "True when the leave request was emitted.",
            required: true,
        }],
    }
}

fn schema_harness_response() -> ControllerSchema {
    ControllerSchema {
        namespace: "agent_meetings",
        function: "harness_response",
        description: "Send a tool execution result back to the backend's meeting LLM so \
                      it can incorporate the result in the next conversation turn.",
        inputs: vec![FieldSchema {
            name: "result",
            ty: TypeSchema::String,
            comment: "The tool execution result text.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "ok",
            ty: TypeSchema::Bool,
            comment: "True when the response was emitted.",
            required: true,
        }],
    }
}

fn handle_join_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::ops::handle_join(params).await })
}

fn handle_leave_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::ops::handle_leave(params).await })
}

fn handle_harness_response_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::ops::handle_harness_response(params).await })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_controllers_match_schemas() {
        let schema_fns: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        let handler_fns: Vec<_> = all_registered_controllers()
            .into_iter()
            .map(|c| c.schema.function)
            .collect();
        assert_eq!(schema_fns, handler_fns);
        assert_eq!(schema_fns, vec!["join", "leave", "harness_response"]);
    }

    #[test]
    fn join_schema_has_correct_namespace() {
        let s = schema_join();
        assert_eq!(s.namespace, "agent_meetings");
        assert_eq!(s.function, "join");
    }
}
