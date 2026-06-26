//! Controller schemas for the `voice_assistant` domain.

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
        function: "push_audio",
        schema: schema_push_audio,
        handler: handle_push_audio,
    },
    Def {
        function: "poll_response",
        schema: schema_poll_response,
        handler: handle_poll_response,
    },
    Def {
        function: "get_status",
        schema: schema_get_status,
        handler: handle_get_status,
    },
    Def {
        function: "interrupt",
        schema: schema_interrupt,
        handler: handle_interrupt,
    },
    Def {
        function: "stop_session",
        schema: schema_stop_session,
        handler: handle_stop_session,
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
        namespace: "voice_assistant",
        function: "start_session",
        description:
            "Start a standalone voice assistant session with local STT/TTS. Returns a session_id \
             for subsequent push_audio / poll_response calls.",
        inputs: vec![
            FieldSchema {
                name: "session_id",
                ty: TypeSchema::String,
                comment: "Optional session UUID. Auto-generated when omitted.",
                required: false,
            },
            FieldSchema {
                name: "stt_provider",
                ty: TypeSchema::String,
                comment: "STT provider: \"whisper\" (local, default) or \"cloud\".",
                required: false,
            },
            FieldSchema {
                name: "tts_provider",
                ty: TypeSchema::String,
                comment: "TTS provider: \"piper\" (local, default) or \"cloud\".",
                required: false,
            },
            FieldSchema {
                name: "language",
                ty: TypeSchema::String,
                comment: "BCP-47 language hint for STT (e.g. \"en\").",
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
                name: "session_id",
                ty: TypeSchema::String,
                comment: "Session key for subsequent calls.",
                required: true,
            },
            FieldSchema {
                name: "stt_provider",
                ty: TypeSchema::String,
                comment: "Resolved STT provider name.",
                required: true,
            },
            FieldSchema {
                name: "tts_provider",
                ty: TypeSchema::String,
                comment: "Resolved TTS provider name.",
                required: true,
            },
        ],
    }
}

fn schema_push_audio() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_assistant",
        function: "push_audio",
        description:
            "Push a chunk of PCM16LE audio (16 kHz mono, base64) into the session. May trigger \
             a brain turn when VAD detects end-of-utterance.",
        inputs: vec![
            FieldSchema {
                name: "session_id",
                ty: TypeSchema::String,
                comment: "Session key from start_session.",
                required: true,
            },
            FieldSchema {
                name: "pcm_base64",
                ty: TypeSchema::String,
                comment: "Base64-encoded PCM16LE samples at 16 kHz mono. Empty = heartbeat.",
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

fn schema_poll_response() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_assistant",
        function: "poll_response",
        description: "Drain any synthesized outbound PCM and text from the session.",
        inputs: vec![FieldSchema {
            name: "session_id",
            ty: TypeSchema::String,
            comment: "Session key from start_session.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the poll succeeded.",
                required: true,
            },
            FieldSchema {
                name: "pcm_base64",
                ty: TypeSchema::String,
                comment: "Base64 PCM16LE since the last poll. Empty when nothing is queued.",
                required: true,
            },
            FieldSchema {
                name: "transcript",
                ty: TypeSchema::String,
                comment: "Last user transcript from STT.",
                required: true,
            },
            FieldSchema {
                name: "reply_text",
                ty: TypeSchema::String,
                comment: "Last assistant reply text.",
                required: true,
            },
            FieldSchema {
                name: "utterance_done",
                ty: TypeSchema::Bool,
                comment: "True when the current outbound utterance is complete.",
                required: true,
            },
        ],
    }
}

fn schema_get_status() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_assistant",
        function: "get_status",
        description: "Query the current state of a voice assistant session.",
        inputs: vec![FieldSchema {
            name: "session_id",
            ty: TypeSchema::String,
            comment: "Session key.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the session exists.",
                required: true,
            },
            FieldSchema {
                name: "session_id",
                ty: TypeSchema::String,
                comment: "Echoed session key.",
                required: true,
            },
            FieldSchema {
                name: "state",
                ty: TypeSchema::String,
                comment: "Current state: listening, processing, speaking, stopped.",
                required: true,
            },
            FieldSchema {
                name: "total_turns",
                ty: TypeSchema::F64,
                comment: "Number of completed turns.",
                required: true,
            },
            FieldSchema {
                name: "stt_provider",
                ty: TypeSchema::String,
                comment: "Active STT provider.",
                required: true,
            },
            FieldSchema {
                name: "tts_provider",
                ty: TypeSchema::String,
                comment: "Active TTS provider.",
                required: true,
            },
        ],
    }
}

fn schema_interrupt() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_assistant",
        function: "interrupt",
        description: "Interrupt (barge-in) the current TTS playback. Clears outbound audio and \
             transitions back to listening. No-op if not currently speaking.",
        inputs: vec![FieldSchema {
            name: "session_id",
            ty: TypeSchema::String,
            comment: "Session key.",
            required: true,
        }],
        outputs: vec![
            FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True when the interrupt was processed.",
                required: true,
            },
            FieldSchema {
                name: "was_speaking",
                ty: TypeSchema::Bool,
                comment: "True if the session was actively speaking when interrupted.",
                required: true,
            },
            FieldSchema {
                name: "discarded_samples",
                ty: TypeSchema::F64,
                comment: "Number of PCM samples discarded from the outbound buffer.",
                required: true,
            },
        ],
    }
}

fn schema_stop_session() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_assistant",
        function: "stop_session",
        description: "Close the voice assistant session and return summary counters.",
        inputs: vec![FieldSchema {
            name: "session_id",
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
                name: "session_id",
                ty: TypeSchema::String,
                comment: "Echoed session key.",
                required: true,
            },
            FieldSchema {
                name: "total_turns",
                ty: TypeSchema::F64,
                comment: "Number of completed agent turns.",
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
        ],
    }
}

fn schema_unknown() -> ControllerSchema {
    ControllerSchema {
        namespace: "voice_assistant",
        function: "unknown",
        description: "Unknown voice_assistant controller function.",
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
fn handle_push_audio(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_push_audio(p).await })
}
fn handle_poll_response(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_poll_response(p).await })
}
fn handle_get_status(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_get_status(p).await })
}
fn handle_interrupt(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_interrupt(p).await })
}
fn handle_stop_session(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { super::rpc::handle_stop_session(p).await })
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
                "push_audio",
                "poll_response",
                "get_status",
                "interrupt",
                "stop_session"
            ]
        );
    }

    #[test]
    fn lookup_returns_unknown_for_missing_function() {
        assert_eq!(schemas("nope").function, "unknown");
    }

    #[test]
    fn all_schemas_have_voice_assistant_namespace() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "voice_assistant");
        }
    }
}
