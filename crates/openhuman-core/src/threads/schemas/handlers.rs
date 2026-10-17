//! Thin JSON-RPC handlers that parse params and delegate to [`super::super::ops`].

use crate::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::threads::turn_state::{
    ClearTurnStateRequest, GetTurnStateForRequestRequest, GetTurnStateRequest,
};
use crate::threads::{
    AppendConversationMessageRequest, ConversationMessagesRequest, CreateConversationThreadRequest,
    DeleteConversationThreadRequest, EmptyRequest, GenerateConversationThreadTitleRequest,
    UpdateConversationMessageRequest, UpdateConversationThreadLabelsRequest,
    UpdateConversationThreadTitleRequest, UpdateConversationThreadWorkingDirRequest,
    UpsertConversationThreadRequest,
};

use super::super::ops;

pub(super) fn handle_list(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::threads_list(&ctx, EmptyRequest {}).await?) })
}

pub(super) fn handle_upsert(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<UpsertConversationThreadRequest>(params)?;
        // A SaaS user picks thread ids for their own threads only; reserved
        // prefixes and path-like ids are refused (no-op outside SaaS).
        crate::profiles::surface::check_thread_id(&p.id)?;
        if let Some(parent) = p.parent_thread_id.as_deref() {
            crate::profiles::surface::check_thread_id(parent)?;
        }
        to_json(ops::thread_upsert(&ctx, p).await?)
    })
}

pub(super) fn handle_create_new(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<CreateConversationThreadRequest>(params)?;
        // A SaaS user acts only in their own sandbox; a caller-chosen working
        // folder would point the thread at the host.
        crate::profiles::surface::check_working_dir(p.action_dir.as_deref())?;
        to_json(ops::thread_create_new(&ctx, p).await?)
    })
}

pub(super) fn handle_messages_list(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ConversationMessagesRequest>(params)?;
        to_json(ops::messages_list(&ctx, p).await?)
    })
}

pub(super) fn handle_message_append(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<AppendConversationMessageRequest>(params)?;
        to_json(ops::message_append(&ctx, p).await?)
    })
}

pub(super) fn handle_generate_title(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<GenerateConversationThreadTitleRequest>(params)?;
        to_json(ops::thread_generate_title(&ctx, p).await?)
    })
}

pub(super) fn handle_update_labels(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<UpdateConversationThreadLabelsRequest>(params)?;
        to_json(ops::thread_update_labels(&ctx, p).await?)
    })
}

pub(super) fn handle_update_title(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<UpdateConversationThreadTitleRequest>(params)?;
        to_json(ops::thread_update_title(&ctx, p).await?)
    })
}

pub(super) fn handle_update_working_dir(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<UpdateConversationThreadWorkingDirRequest>(params)?;
        to_json(ops::thread_update_working_dir(&ctx, p).await?)
    })
}

pub(super) fn handle_message_update(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<UpdateConversationMessageRequest>(params)?;
        to_json(ops::message_update(&ctx, p).await?)
    })
}

pub(super) fn handle_delete(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<DeleteConversationThreadRequest>(params)?;
        to_json(ops::thread_delete(&ctx, p).await?)
    })
}

pub(super) fn handle_purge(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(ops::threads_purge(&ctx, EmptyRequest {}).await?) })
}

pub(super) fn handle_turn_state_get(
    _ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<GetTurnStateRequest>(params)?;
        to_json(ops::turn_state_get(p).await?)
    })
}

pub(super) fn handle_turn_state_list(
    _ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move { to_json(ops::turn_state_list(EmptyRequest {}).await?) })
}

pub(super) fn handle_turn_state_history(
    _ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<GetTurnStateRequest>(params)?;
        to_json(ops::turn_state_history(p).await?)
    })
}

pub(super) fn handle_turn_state_get_turn(
    _ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<GetTurnStateForRequestRequest>(params)?;
        to_json(ops::turn_state_get_turn(p).await?)
    })
}

pub(super) fn handle_turn_state_clear(
    _ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ClearTurnStateRequest>(params)?;
        to_json(ops::turn_state_clear(p).await?)
    })
}

pub(super) fn handle_token_usage(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ops::ThreadTokenUsageRequest>(params)?;
        to_json(ops::token_usage(&ctx, p).await?)
    })
}

pub(super) fn handle_transcript_get(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ops::TranscriptGetRequest>(params)?;
        to_json(ops::transcript_get(&ctx, p).await?)
    })
}

pub(super) fn handle_goal_get(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ops::ThreadLiveStateRequest>(params)?;
        to_json(ops::goal_get(p).await?)
    })
}

pub(super) fn handle_todos_get(_ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ops::ThreadLiveStateRequest>(params)?;
        to_json(ops::todos_get(p).await?)
    })
}

pub(super) fn handle_search(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ops::ThreadSearchRequest>(params)?;
        to_json(ops::thread_search(&ctx, p).await?)
    })
}

pub(super) fn handle_edit_message(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ops::EditMessageRequest>(params)?;
        to_json(
            ops::edit_message(&ctx, p)
                .await
                .map_err(|e| e.to_string())?,
        )
    })
}

pub(super) fn handle_regenerate(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ops::RegenerateRequest>(params)?;
        to_json(ops::regenerate(&ctx, p).await.map_err(|e| e.to_string())?)
    })
}

// ── Helpers ──────────────────────────────────────────────────────────

pub(super) fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: crate::core::Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
