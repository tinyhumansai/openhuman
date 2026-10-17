//! Handlers for the OpenAI OAuth flow: start, complete, Codex CLI import,
//! status, and disconnect.

use crate::core::runtime::DispatchCtx;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::{deserialize_params, to_json};
use crate::core::all::ControllerFuture;

#[derive(Debug, Deserialize)]
pub(super) struct InferenceOpenAiOAuthCompleteParams {
    #[serde(alias = "callbackUrl")]
    callback_url: String,
}

pub(super) fn handle_inference_openai_oauth_start(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(crate::inference::rpc::inference_openai_oauth_start(&config).await?)
    })
}

pub(super) fn handle_inference_openai_oauth_complete(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let payload = deserialize_params::<InferenceOpenAiOAuthCompleteParams>(params)?;
        to_json(
            crate::inference::rpc::inference_openai_oauth_complete(
                &config,
                payload.callback_url.trim(),
            )
            .await?,
        )
    })
}

pub(super) fn handle_inference_openai_oauth_import_codex_cli(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(crate::inference::rpc::inference_openai_oauth_import_codex_cli(&config).await?)
    })
}

pub(super) fn handle_inference_openai_oauth_status(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(crate::inference::rpc::inference_openai_oauth_status(&config).await?)
    })
}

pub(super) fn handle_inference_openai_oauth_disconnect(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(crate::inference::rpc::inference_openai_oauth_disconnect(&config).await?)
    })
}
