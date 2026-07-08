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
    BackendMeetControllerDef {
        function: "speak",
        schema: schema_speak,
        handler: handle_speak_wrap,
    },
    BackendMeetControllerDef {
        function: "notification_action",
        schema: schema_notification_action,
        handler: handle_notification_action_wrap,
    },
    BackendMeetControllerDef {
        function: "list_upcoming",
        schema: schema_list_upcoming,
        handler: handle_list_upcoming_wrap,
    },
    BackendMeetControllerDef {
        function: "set_event_policy",
        schema: schema_set_event_policy,
        handler: handle_set_event_policy_wrap,
    },
    BackendMeetControllerDef {
        function: "get_event_policies",
        schema: schema_get_event_policies,
        handler: handle_get_event_policies_wrap,
    },
    BackendMeetControllerDef {
        function: "generate_summary",
        schema: schema_generate_summary,
        handler: handle_generate_summary_wrap,
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
                name: "mascots",
                ty: TypeSchema::Json,
                comment: "Optional dual-mascot config (issue #4277): array of up to 2 slots, each \
                          { mascotId, name?, riveColors?, voiceId? }. When present the backend renders \
                          both mascots and alternates the speaker per reply. `name` (from the manifest) \
                          enables name-addressed routing (\"Hey Toshi …\" → that slot). Absent falls \
                          back to mascot_id.",
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
            FieldSchema {
                name: "correlation_id",
                ty: TypeSchema::String,
                comment: "Opaque correlation id echoed on all bot:* events for this session.",
                required: false,
            },
            FieldSchema {
                name: "listen_only",
                ty: TypeSchema::Bool,
                comment: "When true, the bot joins in listen-only mode (no microphone, no replies).",
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

fn schema_speak() -> ControllerSchema {
    ControllerSchema {
        namespace: "agent_meetings",
        function: "speak",
        description: "Send text to the meeting bot for TTS playback. The backend converts \
                      the text to speech and plays it into the meeting audio.",
        inputs: vec![
            FieldSchema {
                name: "text",
                ty: TypeSchema::String,
                comment: "The text to speak in the meeting.",
                required: true,
            },
            FieldSchema {
                name: "correlation_id",
                ty: TypeSchema::String,
                comment: "Optional correlation id to associate with this speak request.",
                required: false,
            },
        ],
        outputs: vec![FieldSchema {
            name: "ok",
            ty: TypeSchema::Bool,
            comment: "True when the speak request was emitted.",
            required: true,
        }],
    }
}

fn handle_speak_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::ops::handle_speak(params).await })
}

fn schema_notification_action() -> ControllerSchema {
    ControllerSchema {
        namespace: "agent_meetings",
        function: "notification_action",
        description: "Handle a click on a calendar auto-join notification button. \
                      Actions: join_listen (muted), join_active (reply mode with the \
                      'Hey Tiny' wake phrase), skip (dismiss this meeting), always_join \
                      (persist auto_join_policy=always, then join).",
        inputs: vec![
            FieldSchema {
                name: "action_id",
                ty: TypeSchema::String,
                comment: "One of: join_listen, join_active, skip, always_join.",
                required: true,
            },
            FieldSchema {
                name: "payload",
                ty: TypeSchema::Json,
                comment: "The notification action payload: { meetingId, meetUrl, title } \
                          plus an optional user-edited displayName for the bot.",
                required: false,
            },
        ],
        outputs: vec![FieldSchema {
            name: "ok",
            ty: TypeSchema::Bool,
            comment: "True when the action was handled (join emitted or session updated).",
            required: true,
        }],
    }
}

fn handle_notification_action_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::ops::handle_notification_action(params).await })
}

fn schema_list_upcoming() -> ControllerSchema {
    ControllerSchema {
        // NOTE: namespace is "meet" (not "agent_meetings") so the RPC name is
        // `openhuman.meet_list_upcoming`. The handler lives here in the
        // agent_meetings module because the logic is tightly coupled to the
        // calendar/meeting infrastructure already present in this domain.
        namespace: "meet",
        function: "list_upcoming",
        description: "List upcoming calendar meetings that have a conferencing link (Google Meet, \
                      Zoom, Teams, Webex), fetched from the user's connected Google Calendar via \
                      Composio. Returns an empty list when no calendar is connected. Sort order: \
                      soonest first. Each record includes the inferred platform, attendee count, \
                      organizer, and the global auto-join policy as the default join_policy.",
        inputs: vec![
            FieldSchema {
                name: "lookahead_minutes",
                ty: TypeSchema::U64,
                comment: "How many minutes ahead to look for meetings. Defaults to 480 (8 hours).",
                required: false,
            },
            FieldSchema {
                name: "limit",
                ty: TypeSchema::U64,
                comment:
                    "Maximum number of meetings to return. Defaults to 20. Clamped to [1, 100].",
                required: false,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Always true on success.",
                required: true,
            },
            FieldSchema {
                name: "meetings",
                ty: TypeSchema::Json,
                comment: "Array of upcoming meeting records (may be empty).",
                required: true,
            },
        ],
    }
}

fn handle_list_upcoming_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::ops::handle_list_upcoming(params).await })
}

fn schema_set_event_policy() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet",
        function: "set_event_policy",
        description: "Persist a per-event join-policy override for a specific calendar event. \
                      The stored policy takes precedence over per-platform and global defaults \
                      when the same event ID appears in meet_list_upcoming.",
        inputs: vec![
            FieldSchema {
                name: "calendar_event_id",
                ty: TypeSchema::String,
                comment: "Stable calendar provider event id (the same id returned by meet_list_upcoming).",
                required: true,
            },
            FieldSchema {
                name: "policy",
                ty: TypeSchema::String,
                comment: "Join policy for this event: \"auto\" (always join), \"ask\" (prompt), or \"skip\" (never join).",
                required: true,
            },
        ],
        outputs: vec![FieldSchema {
            name: "ok",
            ty: TypeSchema::Bool,
            comment: "True when the policy was stored successfully.",
            required: true,
        }],
    }
}

fn handle_set_event_policy_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::ops::handle_set_event_policy(params).await })
}

fn schema_get_event_policies() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet",
        function: "get_event_policies",
        description: "Retrieve stored per-event join-policy overrides for a batch of calendar \
                      event IDs. Event IDs without a stored override are omitted from the \
                      returned map.",
        inputs: vec![FieldSchema {
            name: "calendar_event_ids",
            ty: TypeSchema::Json,
            comment: "Array of calendar event id strings to look up.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "Always true on success.",
                required: true,
            },
            FieldSchema {
                name: "policies",
                ty: TypeSchema::Json,
                comment: "Object mapping calendar_event_id → policy string (\"auto\" | \"ask\" | \"skip\"). \
                          Only IDs with a stored override are included.",
                required: true,
            },
        ],
    }
}

fn handle_get_event_policies_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::ops::handle_get_event_policies(params).await })
}

fn schema_generate_summary() -> ControllerSchema {
    ControllerSchema {
        namespace: "agent_meetings",
        function: "generate_summary",
        description: "Generate a post-call summary for a recorded meeting transcript and create a \
                      summary thread on demand. Used by Ask/manual flows.",
        inputs: vec![FieldSchema {
            name: "meeting_id",
            ty: TypeSchema::String,
            comment: "Recorded meeting id / recent-call request_id to summarize.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when summary generation and thread creation succeeded.",
                required: true,
            },
            FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "Thread containing the transcript and generated summary.",
                required: true,
            },
        ],
    }
}

fn handle_generate_summary_wrap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::ops::handle_generate_summary(params).await })
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
        assert_eq!(
            schema_fns,
            vec![
                "join",
                "leave",
                "harness_response",
                "speak",
                "notification_action",
                "list_upcoming",
                "set_event_policy",
                "get_event_policies",
                "generate_summary",
            ]
        );
    }

    #[test]
    fn join_schema_has_correct_namespace() {
        let s = schema_join();
        assert_eq!(s.namespace, "agent_meetings");
        assert_eq!(s.function, "join");
    }

    #[test]
    fn generate_summary_schema_is_agent_meetings_rpc() {
        let s = schema_generate_summary();
        assert_eq!(s.namespace, "agent_meetings");
        assert_eq!(s.function, "generate_summary");
        assert!(s
            .inputs
            .iter()
            .any(|f| f.name == "meeting_id" && f.required));
        assert!(s
            .outputs
            .iter()
            .any(|f| f.name == "thread_id" && f.required));
    }
}
