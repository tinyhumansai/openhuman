use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use log::debug;

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

use super::types::{
    GameplayPresetInput, GameplayReviewAnalysisInput, GameplayReviewClipInput,
    GameplayReviewQuestionInput, GameplayReviewSessionInput,
};

#[derive(Deserialize)]
struct SessionIdParams {
    session_id: String,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("register_session"),
        schemas("analyze_session"),
        schemas("get_session"),
        schemas("list_sessions"),
        schemas("set_preset"),
        schemas("list_presets"),
        schemas("ask_session"),
        schemas("draft_clip_metadata"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("register_session"),
            handler: handle_register_session,
        },
        RegisteredController {
            schema: schemas("analyze_session"),
            handler: handle_analyze_session,
        },
        RegisteredController {
            schema: schemas("get_session"),
            handler: handle_get_session,
        },
        RegisteredController {
            schema: schemas("list_sessions"),
            handler: handle_list_sessions,
        },
        RegisteredController {
            schema: schemas("set_preset"),
            handler: handle_set_preset,
        },
        RegisteredController {
            schema: schemas("list_presets"),
            handler: handle_list_presets,
        },
        RegisteredController {
            schema: schemas("ask_session"),
            handler: handle_ask_session,
        },
        RegisteredController {
            schema: schemas("draft_clip_metadata"),
            handler: handle_draft_clip_metadata,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "register_session" => ControllerSchema {
            namespace: "gameplay_review",
            function: "register_session",
            description: "Register a gameplay session from imported keyframes.",
            inputs: vec![
                FieldSchema {
                    name: "game_id",
                    ty: TypeSchema::String,
                    comment: "Game identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "session_title",
                    ty: TypeSchema::String,
                    comment: "Human-readable session title.",
                    required: true,
                },
                FieldSchema {
                    name: "source_label",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional label describing where the footage came from.",
                    required: false,
                },
                FieldSchema {
                    name: "spoiler_mode",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Ref("SpoilerMode"))),
                    comment: "Optional spoiler-mode override for this session.",
                    required: false,
                },
                FieldSchema {
                    name: "preset_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional coaching preset to apply.",
                    required: false,
                },
                FieldSchema {
                    name: "frames",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("GameplayFrameInput"))),
                    comment: "Captured keyframes for the session.",
                    required: true,
                },
            ],
            outputs: vec![json_output("session", "Stored gameplay review session.")],
        },
        "analyze_session" => ControllerSchema {
            namespace: "gameplay_review",
            function: "analyze_session",
            description:
                "Analyze a gameplay session, generate highlights, and draft clip metadata.",
            inputs: vec![
                FieldSchema {
                    name: "session_id",
                    ty: TypeSchema::String,
                    comment: "Session identifier to analyze.",
                    required: true,
                },
                FieldSchema {
                    name: "max_highlights",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Optional cap on the number of highlights to generate.",
                    required: false,
                },
                FieldSchema {
                    name: "platforms",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Target platforms for clip drafts (e.g. youtube, tiktok).",
                    required: false,
                },
            ],
            outputs: vec![json_output("session", "Analyzed gameplay review session.")],
        },
        "get_session" => ControllerSchema {
            namespace: "gameplay_review",
            function: "get_session",
            description: "Fetch one gameplay review session by id.",
            inputs: vec![FieldSchema {
                name: "session_id",
                ty: TypeSchema::String,
                comment: "Session identifier.",
                required: true,
            }],
            outputs: vec![json_output("session", "Gameplay review session.")],
        },
        "list_sessions" => ControllerSchema {
            namespace: "gameplay_review",
            function: "list_sessions",
            description: "List gameplay review sessions stored in the workspace.",
            inputs: vec![FieldSchema {
                name: "game_id",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional game filter.",
                required: false,
            }],
            outputs: vec![json_output("sessions", "Gameplay review sessions.")],
        },
        "set_preset" => ControllerSchema {
            namespace: "gameplay_review",
            function: "set_preset",
            description: "Save a game-specific coaching preset.",
            inputs: vec![
                FieldSchema {
                    name: "game_id",
                    ty: TypeSchema::String,
                    comment: "Game identifier.",
                    required: true,
                },
                FieldSchema {
                    name: "display_name",
                    ty: TypeSchema::String,
                    comment: "Human-readable preset name.",
                    required: true,
                },
                FieldSchema {
                    name: "coaching_focus",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Areas of focus for coaching commentary.",
                    required: false,
                },
                FieldSchema {
                    name: "audio_feedback",
                    ty: TypeSchema::Bool,
                    comment: "Whether to surface audio cues in highlight summaries.",
                    required: false,
                },
                FieldSchema {
                    name: "spoiler_mode",
                    ty: TypeSchema::Ref("SpoilerMode"),
                    comment: "Spoiler-handling mode for this preset.",
                    required: false,
                },
                FieldSchema {
                    name: "notes",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional free-form preset notes.",
                    required: false,
                },
            ],
            outputs: vec![json_output("preset", "Saved gameplay review preset.")],
        },
        "list_presets" => ControllerSchema {
            namespace: "gameplay_review",
            function: "list_presets",
            description: "List stored gameplay coaching presets.",
            inputs: vec![],
            outputs: vec![json_output("presets", "Gameplay review presets.")],
        },
        "ask_session" => ControllerSchema {
            namespace: "gameplay_review",
            function: "ask_session",
            description: "Ask a question against a stored gameplay session.",
            inputs: vec![
                FieldSchema {
                    name: "session_id",
                    ty: TypeSchema::String,
                    comment: "Session identifier to query.",
                    required: true,
                },
                FieldSchema {
                    name: "question",
                    ty: TypeSchema::String,
                    comment: "Question text to ask against the session.",
                    required: true,
                },
            ],
            outputs: vec![json_output(
                "answer",
                "Question answer with matched highlights.",
            )],
        },
        "draft_clip_metadata" => ControllerSchema {
            namespace: "gameplay_review",
            function: "draft_clip_metadata",
            description: "Draft clip titles, descriptions, and tags for one gameplay highlight.",
            inputs: vec![
                FieldSchema {
                    name: "session_id",
                    ty: TypeSchema::String,
                    comment: "Session identifier holding the highlight.",
                    required: true,
                },
                FieldSchema {
                    name: "platform",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional target platform (defaults to all configured).",
                    required: false,
                },
                FieldSchema {
                    name: "highlight_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional highlight to focus on (defaults to all).",
                    required: false,
                },
            ],
            outputs: vec![json_output("drafts", "Draft metadata for clip publishing.")],
        },
        _ => ControllerSchema {
            namespace: "gameplay_review",
            function: "unknown",
            description: "Unknown gameplay_review controller function.",
            inputs: vec![],
            outputs: vec![json_output("error", "Lookup error details.")],
        },
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|err| err.to_string())
}

fn handle_register_session(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<GameplayReviewSessionInput>(params)?;
        debug!(
            "[gameplay_review][controller] register_session game_id={} frames={}",
            payload.game_id,
            payload.frames.len()
        );
        let result = crate::openhuman::gameplay_review::rpc::register_session(payload).await?;
        debug!(
            "[gameplay_review][controller] register_session complete session_id={} game_id={}",
            result.value.session_id, result.value.game_id
        );
        to_json(result)
    })
}

fn handle_analyze_session(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<GameplayReviewAnalysisInput>(params)?;
        debug!(
            "[gameplay_review][controller] analyze_session session_id={} max_highlights={:?} platforms={}",
            payload.session_id,
            payload.max_highlights,
            payload.platforms.len()
        );
        let result = crate::openhuman::gameplay_review::rpc::analyze_session(payload).await?;
        debug!(
            "[gameplay_review][controller] analyze_session complete session_id={} analyzed={}",
            result.value.session_id,
            result.value.analyzed_at_ms.is_some()
        );
        to_json(result)
    })
}

fn handle_get_session(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<SessionIdParams>(params)?;
        debug!(
            "[gameplay_review][controller] get_session session_id={}",
            payload.session_id
        );
        let result =
            crate::openhuman::gameplay_review::rpc::get_session(payload.session_id).await?;
        debug!(
            "[gameplay_review][controller] get_session complete session_id={} game_id={}",
            result.value.session_id, result.value.game_id
        );
        to_json(result)
    })
}

fn handle_list_sessions(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(Deserialize)]
        struct Params {
            #[serde(default)]
            game_id: Option<String>,
        }
        let payload = deserialize_params::<Params>(params)?;
        debug!(
            "[gameplay_review][controller] list_sessions game_id_filter={:?}",
            payload.game_id
        );
        let result = crate::openhuman::gameplay_review::rpc::list_sessions(payload.game_id).await?;
        debug!(
            "[gameplay_review][controller] list_sessions complete count={}",
            result.value.len()
        );
        to_json(result)
    })
}

fn handle_set_preset(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<GameplayPresetInput>(params)?;
        debug!(
            "[gameplay_review][controller] set_preset game_id={} display_name={} focus_items={}",
            payload.game_id,
            payload.display_name,
            payload.coaching_focus.len()
        );
        let result = crate::openhuman::gameplay_review::rpc::set_preset(payload).await?;
        debug!(
            "[gameplay_review][controller] set_preset complete game_id={}",
            result.value.game_id
        );
        to_json(result)
    })
}

fn handle_list_presets(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        debug!("[gameplay_review][controller] list_presets start");
        let result = crate::openhuman::gameplay_review::rpc::list_presets().await?;
        debug!(
            "[gameplay_review][controller] list_presets complete count={}",
            result.value.len()
        );
        to_json(result)
    })
}

fn handle_ask_session(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<GameplayReviewQuestionInput>(params)?;
        let session_id = payload.session_id.clone();
        debug!(
            "[gameplay_review][controller] ask_session session_id={} question_len={}",
            session_id,
            payload.question.len()
        );
        let result = crate::openhuman::gameplay_review::rpc::ask_session(payload).await?;
        debug!(
            "[gameplay_review][controller] ask_session complete session_id={} matched_highlights={} suggested_follow_up={}",
            session_id,
            result.value.matched_highlights.len(),
            result.value.suggested_follow_up.len()
        );
        to_json(result)
    })
}

fn handle_draft_clip_metadata(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<GameplayReviewClipInput>(params)?;
        let session_id = payload.session_id.clone();
        debug!(
            "[gameplay_review][controller] draft_clip_metadata session_id={} platform={:?} highlight_id={:?}",
            session_id,
            payload.platform,
            payload.highlight_id
        );
        let result = crate::openhuman::gameplay_review::rpc::draft_clip_metadata(payload).await?;
        debug!(
            "[gameplay_review][controller] draft_clip_metadata complete session_id={} drafts={}",
            session_id,
            result.value.len()
        );
        to_json(result)
    })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
