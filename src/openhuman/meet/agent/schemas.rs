//! Controller schemas for the `meet_agent` domain.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

type SchemaBuilder = fn() -> ControllerSchema;
type ControllerHandler = fn(Map<String, Value>) -> ControllerFuture;

struct Def {
    function: &'static str,
    schema: SchemaBuilder,
    handler: ControllerHandler,
}

const DEFS: &[Def] = &[
    Def {
        function: "start_session",
        schema: schema_start_session,
        handler: handle_start_session,
    },
    Def {
        function: "push_listen_pcm",
        schema: schema_push_listen_pcm,
        handler: handle_push_listen_pcm,
    },
    Def {
        function: "push_caption",
        schema: schema_push_caption,
        handler: handle_push_caption,
    },
    Def {
        function: "poll_speech",
        schema: schema_poll_speech,
        handler: handle_poll_speech,
    },
    Def {
        function: "stop_session",
        schema: schema_stop_session,
        handler: handle_stop_session,
    },
    Def {
        function: "list_calls",
        schema: schema_list_calls,
        handler: handle_list_calls,
    },
    Def {
        function: "get_call_detail",
        schema: schema_get_call_detail,
        handler: handle_get_call_detail,
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
    if let Some(d) = DEFS.iter().find(|d| d.function == function) {
        return (d.schema)();
    }
    schema_unknown()
}

fn schema_start_session() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet_agent",
        function: "start_session",
        description:
            "Open a meet-agent session keyed by request_id. The Tauri shell calls this after \
                      the meet-call window opens and before pushing PCM frames.",
        inputs: vec![
            FieldSchema {
                name: "request_id",
                ty: TypeSchema::String,
                comment: "UUID minted by openhuman.meet_join_call.",
                required: true,
            },
            FieldSchema {
                name: "sample_rate_hz",
                ty: TypeSchema::F64,
                comment: "Sample rate of inbound/outbound PCM. Default 16000.",
                required: false,
            },
            FieldSchema {
                name: "owner_display_name",
                ty: TypeSchema::String,
                comment: "Display name of the call owner (the user who launched the bot). \
                     Used by the wake-word gate as the only speaker authorised to trigger \
                     tool calls. Empty fails closed.",
                required: false,
            },
            FieldSchema {
                name: "bot_display_name",
                ty: TypeSchema::String,
                comment: "Display name the bot uses as its Meet participant tile. Used to drop \
                     the bot's own captions (self-echo filter).",
                required: false,
            },
            FieldSchema {
                name: "meet_url",
                ty: TypeSchema::String,
                comment: "Normalised Meet URL the call joined. Persisted into the recent-calls \
                     log on stop_session.",
                required: false,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the session was opened.",
                required: true,
            },
            FieldSchema {
                name: "request_id",
                ty: TypeSchema::String,
                comment: "Echoed session key.",
                required: true,
            },
            FieldSchema {
                name: "sample_rate_hz",
                ty: TypeSchema::F64,
                comment: "Echoed sample rate the session was opened with.",
                required: true,
            },
        ],
    }
}

fn schema_push_listen_pcm() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet_agent",
        function: "push_listen_pcm",
        description:
            "Push a chunk of inbound PCM (Meet → agent) into the session. May trigger a brain \
                      turn when VAD detects end-of-utterance.",
        inputs: vec![
            FieldSchema {
                name: "request_id",
                ty: TypeSchema::String,
                comment: "Session key from start_session.",
                required: true,
            },
            FieldSchema {
                name: "pcm_base64",
                ty: TypeSchema::String,
                comment:
                    "Base64-encoded PCM16LE samples at the session's sample rate. Empty allowed.",
                required: true,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the chunk was accepted.",
                required: true,
            },
            FieldSchema {
                name: "turn_started",
                ty: TypeSchema::Bool,
                comment: "True when this push closed an utterance and the brain ran a turn.",
                required: true,
            },
        ],
    }
}

fn schema_push_caption() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet_agent",
        function: "push_caption",
        description: "Push a caption line scraped from Meet's live captions DOM. The wake-word \
                      gate (\"hey openhuman\") triggers an LLM/TTS turn when fired.",
        inputs: vec![
            FieldSchema {
                name: "request_id",
                ty: TypeSchema::String,
                comment: "Session key from start_session.",
                required: true,
            },
            FieldSchema {
                name: "speaker",
                ty: TypeSchema::String,
                comment: "Speaker label scraped from Meet (display name); may be empty.",
                required: false,
            },
            FieldSchema {
                name: "text",
                ty: TypeSchema::String,
                comment: "Caption transcript (already trimmed by the page-side bridge).",
                required: true,
            },
            FieldSchema {
                name: "ts_ms",
                ty: TypeSchema::F64,
                comment: "Page-side Date.now() when the caption was queued.",
                required: false,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the caption was accepted.",
                required: true,
            },
            FieldSchema {
                name: "turn_started",
                ty: TypeSchema::Bool,
                comment:
                    "True when this caption tripped the wake word and a brain turn dispatched.",
                required: true,
            },
        ],
    }
}

fn schema_poll_speech() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet_agent",
        function: "poll_speech",
        description: "Drain any synthesized outbound PCM (agent → Meet) the session has queued.",
        inputs: vec![FieldSchema {
            name: "request_id",
            ty: TypeSchema::String,
            comment: "Session key from start_session.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the poll succeeded (even if no audio was queued).",
                required: true,
            },
            FieldSchema {
                name: "pcm_base64",
                ty: TypeSchema::String,
                comment: "Base64 PCM16LE since the last poll. Empty when nothing is queued.",
                required: true,
            },
            FieldSchema {
                name: "utterance_done",
                ty: TypeSchema::Bool,
                comment: "True when the current outbound utterance is complete.",
                required: true,
            },
            FieldSchema {
                name: "flush_pending",
                ty: TypeSchema::Bool,
                comment: "True when the shell should flush in-flight audio (barge-in). The shell must call __openhumanFlushAudio() before feeding the next PCM chunk.",
                required: true,
            },
        ],
    }
}

fn schema_stop_session() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet_agent",
        function: "stop_session",
        description: "Close the named session and return summary counters.",
        inputs: vec![FieldSchema {
            name: "request_id",
            ty: TypeSchema::String,
            comment: "Session key.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the session existed and was closed.",
                required: true,
            },
            FieldSchema {
                name: "request_id",
                ty: TypeSchema::String,
                comment: "Echoed session key.",
                required: true,
            },
            FieldSchema {
                name: "listened_seconds",
                ty: TypeSchema::F64,
                comment: "Total seconds of inbound audio processed.",
                required: true,
            },
            FieldSchema {
                name: "spoken_seconds",
                ty: TypeSchema::F64,
                comment: "Total seconds of outbound audio synthesized.",
                required: true,
            },
            FieldSchema {
                name: "turn_count",
                ty: TypeSchema::F64,
                comment: "Number of completed agent turns.",
                required: true,
            },
        ],
    }
}

fn schema_list_calls() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet_agent",
        function: "list_calls",
        description:
            "Return the most recent completed Meet calls (newest first). Reads the JSONL log written \
                      on each stop_session. Used by the Skills Meeting Bots card to show a recent-calls list.",
        inputs: vec![FieldSchema {
            name: "limit",
            ty: TypeSchema::F64,
            comment: "Max rows to return. Defaults to 50; hard-capped server-side.",
            required: false,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the read succeeded (even if no rows exist yet).",
                required: true,
            },
            FieldSchema {
                name: "calls",
                ty: TypeSchema::String,
                comment: "Array of MeetCallRecord objects, newest first.",
                required: true,
            },
            FieldSchema {
                name: "count",
                ty: TypeSchema::F64,
                comment: "Number of rows in `calls`.",
                required: true,
            },
        ],
    }
}

fn schema_get_call_detail() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet_agent",
        function: "get_call_detail",
        description:
            "Return the persisted transcript + summary for one completed Meet call, keyed by \
                      request_id. Used by the recent-calls panel to lazy-load detail when a row is \
                      expanded. `detail` is null when no detail was recorded for the call.",
        inputs: vec![FieldSchema {
            name: "request_id",
            ty: TypeSchema::String,
            comment: "request_id of the call (matches a MeetCallRecord row from list_calls).",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the read succeeded (even if no detail exists yet).",
                required: true,
            },
            FieldSchema {
                name: "detail",
                ty: TypeSchema::Json,
                comment:
                    "MeetCallDetail object (transcript + summary), or null when none recorded.",
                required: false,
            },
        ],
    }
}

fn schema_unknown() -> ControllerSchema {
    ControllerSchema {
        namespace: "meet_agent",
        function: "unknown",
        description: "Unknown meet_agent controller function.",
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

fn handle_start_session(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_start_session(p).await })
}
fn handle_push_listen_pcm(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_push_listen_pcm(p).await })
}
fn handle_push_caption(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_push_caption(p).await })
}
fn handle_poll_speech(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_poll_speech(p).await })
}
fn handle_stop_session(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_stop_session(p).await })
}
fn handle_list_calls(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_list_calls(p).await })
}

fn handle_get_call_detail(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_get_call_detail(p).await })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_handlers_match_schemas() {
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
                "start_session",
                "push_listen_pcm",
                "push_caption",
                "poll_speech",
                "stop_session",
                "list_calls",
                "get_call_detail",
            ]
        );
    }

    #[test]
    fn lookup_returns_unknown_for_missing_function() {
        assert_eq!(schemas("nope").function, "unknown");
    }

    #[test]
    fn start_session_requires_request_id() {
        let s = schema_start_session();
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["request_id"]);
    }
}
