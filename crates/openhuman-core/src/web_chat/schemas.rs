//! `ControllerSchema`/`RegisteredController` definitions for the `channel`
//! RPC namespace: `web_chat`, `web_cancel`, `web_queue_status`,
//! `web_queue_clear`.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::ops::{
    channel_web_cancel, channel_web_chat_with_attachments, channel_web_queue_clear,
    channel_web_queue_remove, channel_web_queue_status,
};
use super::types::{
    ChatRequestMetadata, VoiceNoteReadParams, VoiceNoteStoreParams, WebCancelParams, WebChatParams,
    WebQueueParams, WebQueueRemoveParams,
};

pub fn all_web_channel_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("chat"),
        schemas("cancel"),
        schemas("queue_status"),
        schemas("queue_clear"),
        schemas("queue_remove"),
        schemas("voice_note_store"),
        schemas("voice_note_read"),
    ]
}

pub fn all_web_channel_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("chat"),
            handler: handle_chat,
        },
        RegisteredController {
            schema: schemas("cancel"),
            handler: handle_cancel,
        },
        RegisteredController {
            schema: schemas("queue_status"),
            handler: handle_queue_status,
        },
        RegisteredController {
            schema: schemas("queue_clear"),
            handler: handle_queue_clear,
        },
        RegisteredController {
            schema: schemas("queue_remove"),
            handler: handle_queue_remove,
        },
        RegisteredController {
            schema: schemas("voice_note_store"),
            handler: handle_voice_note_store,
        },
        RegisteredController {
            schema: schemas("voice_note_read"),
            handler: handle_voice_note_read,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "chat" => ControllerSchema {
            namespace: "channel",
            function: "web_chat",
            description: "Send a web channel message through the agent loop.",
            inputs: vec![
                required_string("client_id", "Client stream identifier."),
                required_string("thread_id", "Thread identifier."),
                required_string("message", "User message."),
                FieldSchema {
                    name: "attachments",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Typed attachments (tinychannels ContentPart[]). VoiceNote parts carry an opaque id validated by channel.web_voice_note_store. Other media parts must be opaque refs returned on the persisted user message; their stored bytes are resolved under the trusted turn scope.",
                    required: false,
                },
                optional_string(
                    "persisted_message_id",
                    "User message row that owns the opaque typed attachment refs.",
                ),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
                optional_string(
                    "locale",
                    "Optional BCP-47 UI locale (e.g. 'ar', 'zh-CN'). Drives the \"reply in this language\" system-prompt directive.",
                ),
                optional_bool(
                    "speak_reply",
                    "When true, the agent's final reply is spoken via TTS (for PTT and similar background voice flows).",
                ),
                optional_string(
                    "source",
                    "Origin of the message: \"ptt\" | \"dictation\" | \"type\" | other. Used for analytics + downstream metadata.",
                ),
                optional_u64(
                    "session_id",
                    "Optional caller-provided correlation id (PTT session id).",
                ),
                optional_string(
                    "queue_mode",
                    "Queue mode: 'interrupt' (default), 'steer', 'followup', 'collect', or 'parallel'.",
                ),
                optional_string(
                    "run_mode",
                    "Optional 'plan' | 'build' — start this turn with the thread already in the requested run mode, like the socket chat:start payload's run_mode. Unrecognized values are ignored.",
                ),
                optional_string(
                    "reasoning_effort",
                    "Optional reasoning (thinking) effort for this thread: 'none', 'minimal', 'low', 'medium', 'high' or 'xhigh' ('off' and 'max' are aliases). 'default' clears the thread's choice back to config; omitted leaves it unchanged. Unknown values are rejected.",
                ),
            ],
            outputs: vec![json_output(
                "ack",
                "Acceptance payload; queued follow-ups include request_id and sanitized queued_attachments (opaque refs only).",
            )],
        },
        "cancel" => ControllerSchema {
            namespace: "channel",
            function: "web_cancel",
            description: "Cancel in-flight web channel request for a thread.",
            inputs: vec![
                required_string("client_id", "Client stream identifier."),
                required_string("thread_id", "Thread identifier."),
                optional_string(
                    "request_id",
                    "Request id to cancel. When set, only that turn is cancelled (a stale cancel for a superseded request is ignored so the newer turn survives). Omit to stop whatever is running on the thread, including its detached background sub-agents.",
                ),
            ],
            outputs: vec![json_output(
                "ack",
                "{ cancelled, client_id, thread_id, request_id, subagents_cancelled }. `request_id` is null when no turn was running, in which case no `cancelled` chat_error follows.",
            )],
        },
        "queue_status" => ControllerSchema {
            namespace: "channel",
            function: "web_queue_status",
            description: "Get the run queue status for a thread.",
            inputs: vec![required_string("thread_id", "Thread identifier.")],
            outputs: vec![json_output("status", "Queue status payload.")],
        },
        "queue_clear" => ControllerSchema {
            namespace: "channel",
            function: "web_queue_clear",
            description: "Clear the run queue for a thread.",
            inputs: vec![required_string("thread_id", "Thread identifier.")],
            outputs: vec![json_output("result", "Queue clear result.")],
        },
        "queue_remove" => ControllerSchema {
            namespace: "channel",
            function: "web_queue_remove",
            description: "Remove one specific queued item from a thread's run queue by id.",
            inputs: vec![
                required_string("client_id", "Client stream identifier."),
                required_string("thread_id", "Thread identifier."),
                required_string("item_id", "Id of the queued item to remove."),
            ],
            outputs: vec![json_output("result", "{ thread_id, item_id, removed }.")],
        },
        "voice_note_store" => ControllerSchema {
            namespace: "channel",
            function: "web_voice_note_store",
            description: "Store the audio of a voice note the user recorded, so the thread row can point at it.",
            inputs: vec![
                required_string(
                    "note_id",
                    "Caller-chosen id (letters, digits, '-', '_'); storing the same id again replaces the audio.",
                ),
                required_string("audio_base64", "Base64 of the recorded audio."),
                optional_string("mime", "Audio mime type (default audio/webm)."),
            ],
            outputs: vec![json_output("result", "{ note_id, mime }.")],
        },
        "voice_note_read" => ControllerSchema {
            namespace: "channel",
            function: "web_voice_note_read",
            description: "Read the audio of a stored voice note (user recording or spoken assistant reply).",
            inputs: vec![required_string(
                "note_id",
                "Voice note id from the message metadata.",
            )],
            outputs: vec![json_output(
                "result",
                "{ note_id, mime, audio_base64 }; audio_base64 is null when no audio exists.",
            )],
        },
        _ => ControllerSchema {
            namespace: "channel",
            function: "unknown",
            description: "Unknown web channel controller function.",
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

fn handle_chat(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebChatParams>(params)?;
        let voice_note_id = validate_voice_note_attachment(&ctx, &p).await?;
        let stored_parts = p
            .attachments
            .iter()
            .filter(|part| !matches!(part, tinychannels_bus::ContentPart::VoiceNote { .. }))
            .cloned()
            .collect::<Vec<_>>();
        let is_queued_followup = p.queue_mode.as_deref() == Some("followup");
        let (stored_attachments, attachment_uploads) = if is_queued_followup {
            if p.persisted_message_id.is_some() {
                return Err("queued follow-ups must submit fresh attachment uploads".into());
            }
            if p.message.contains("[ATTACHMENT:")
                || p.message.contains("[IMAGE:")
                || p.message.contains("[FILE:")
            {
                return Err("typed media uploads require marker-free user text".into());
            }
            (None, (!stored_parts.is_empty()).then_some(stored_parts))
        } else if stored_parts.is_empty() {
            if p.persisted_message_id.is_some() {
                return Err("persisted_message_id requires opaque media refs".into());
            }
            (None, None)
        } else {
            crate::threads::attachments::validate_opaque_parts(&stored_parts)?;
            let persisted_message_id =
                p.persisted_message_id
                    .clone()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| "opaque media refs require persisted_message_id".to_string())?;
            (
                Some(crate::agent::queued_turn::StoredAttachmentInput {
                    persisted_message_id,
                    parts: stored_parts,
                }),
                None,
            )
        };
        let message = if stored_attachments.is_some() || is_queued_followup {
            if p.message.contains("[ATTACHMENT:")
                || p.message.contains("[IMAGE:")
                || p.message.contains("[FILE:")
            {
                return Err("opaque media refs require marker-free user text".into());
            }
            p.message.clone()
        } else {
            crate::attachments::parts::fold_into_message(&p.message, &p.attachments)
                .map_err(|e| e.to_string())?
        };
        // Recorded before the turn starts so it reads the new choice.
        crate::agent::tinyagents::apply_requested_effort(
            &ctx,
            &p.thread_id,
            p.reasoning_effort.as_deref(),
        )?;
        to_json(
            channel_web_chat_with_attachments(
                &ctx,
                &p.client_id,
                &p.thread_id,
                &message,
                p.model_override,
                p.temperature,
                p.locale,
                p.queue_mode,
                p.run_mode,
                ChatRequestMetadata {
                    speak_reply: p.speak_reply,
                    source: p.source,
                    session_id: p.session_id,
                    // Attribution is stamped later by run_chat_task once the
                    // target agent is resolved.
                    agent_id: None,
                    hold_text_stream: false,
                },
                voice_note_id,
                stored_attachments,
                attachment_uploads,
            )
            .await?,
        )
    })
}

async fn validate_voice_note_attachment(
    ctx: &DispatchCtx,
    params: &WebChatParams,
) -> Result<Option<String>, String> {
    let voice_notes = params
        .attachments
        .iter()
        .filter_map(|part| match part {
            tinychannels_bus::ContentPart::VoiceNote { voice_note } => Some(voice_note),
            _ => None,
        })
        .collect::<Vec<_>>();
    if voice_notes.len() > 1 {
        return Err("a chat message can carry only one typed voice note".into());
    }
    let Some(voice_note) = voice_notes.first() else {
        if params.source.as_deref() == Some(super::voice_notes::VOICE_NOTE_SOURCE) {
            return Err("voice-note turns require their typed audio part".into());
        }
        return Ok(None);
    };
    if params.source.as_deref() != Some(super::voice_notes::VOICE_NOTE_SOURCE) {
        return Err("typed voice-note audio requires source=voice_note".into());
    }
    let note_id = voice_note
        .media
        .opaque_id
        .as_deref()
        .ok_or_else(|| "typed voice-note audio requires its stored note id".to_string())?;
    let workspace_dir = ctx
        .load_config()
        .await
        .map_err(|error| format!("load voice note workspace: {error}"))?
        .workspace_dir;
    super::voice_notes::validate_typed_media(ctx, &workspace_dir, note_id, &voice_note.media)?;
    Ok(Some(note_id.to_string()))
}

fn handle_queue_status(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebQueueParams>(params)?;
        to_json(channel_web_queue_status(&ctx, &p.thread_id).await?)
    })
}

fn handle_queue_clear(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebQueueParams>(params)?;
        to_json(channel_web_queue_clear(&ctx, &p.thread_id).await?)
    })
}

fn handle_queue_remove(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebQueueRemoveParams>(params)?;
        to_json(channel_web_queue_remove(&ctx, &p.client_id, &p.thread_id, &p.item_id).await?)
    })
}

fn handle_voice_note_store(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<VoiceNoteStoreParams>(params)?;
        let config = ctx.load_config().await?;
        to_json(super::voice_notes::store_from_rpc(
            &ctx,
            &config.workspace_dir,
            &p.note_id,
            p.mime.as_deref().unwrap_or("audio/webm"),
            &p.audio_base64,
        )?)
    })
}

fn handle_voice_note_read(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<VoiceNoteReadParams>(params)?;
        let config = ctx.load_config().await?;
        to_json(super::voice_notes::read_for_rpc(
            &ctx,
            &config.workspace_dir,
            &p.note_id,
        )?)
    })
}

fn handle_cancel(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebCancelParams>(params)?;
        to_json(
            channel_web_cancel(&ctx, &p.client_id, &p.thread_id, p.request_id.as_deref()).await?,
        )
    })
}

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

pub(crate) fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

pub(crate) fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

pub(crate) fn optional_f64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
        comment,
        required: false,
    }
}

pub(crate) fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

pub(crate) fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment,
        required: false,
    }
}

pub(crate) fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
