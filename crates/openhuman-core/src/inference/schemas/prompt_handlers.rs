//! Handlers that run a model: summarize, prompt, vision prompt, provider
//! model test, should-react, and sentiment analysis.

use crate::core::runtime::DispatchCtx;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::{deserialize_params, to_json};
use crate::core::all::ControllerFuture;

#[derive(Debug, Deserialize)]
pub(super) struct InferenceSummarizeParams {
    text: String,
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InferencePromptParams {
    prompt: String,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InferenceVisionPromptParams {
    prompt: String,
    image_refs: Vec<String>,
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InferenceTestChatModelParams {
    workload: String,
    provider: String,
    prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InferenceAnalyzeSentimentParams {
    message: String,
}

pub(super) fn handle_inference_summarize(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceSummarizeParams>(params)?;
        let config = ctx.load_config().await?;
        to_json(crate::inference::rpc::inference_summarize(&config, &p.text, p.max_tokens).await?)
    })
}

pub(super) fn handle_inference_prompt(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferencePromptParams>(params)?;
        let config = ctx.load_config().await?;
        to_json(
            crate::inference::rpc::inference_prompt(&config, &p.prompt, p.max_tokens, p.no_think)
                .await?,
        )
    })
}

pub(super) fn handle_inference_vision_prompt(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceVisionPromptParams>(params)?;
        let config = ctx.load_config().await?;
        to_json(
            crate::inference::rpc::inference_vision_prompt(
                &config,
                &p.prompt,
                &p.image_refs,
                p.max_tokens,
            )
            .await?,
        )
    })
}

pub(super) fn handle_inference_test_provider_model(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceTestChatModelParams>(params)?;
        let config = ctx.load_config().await?;
        to_json(
            crate::inference::rpc::inference_test_provider_model(
                &config,
                &p.workload,
                &p.provider,
                p.prompt.as_deref().unwrap_or("Hello world"),
            )
            .await?,
        )
    })
}

pub(super) fn handle_inference_analyze_sentiment(
    ctx: DispatchCtx,
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<InferenceAnalyzeSentimentParams>(params)?;
        let config = ctx.load_config().await?;
        to_json(crate::inference::rpc::inference_analyze_sentiment(&config, &p.message).await?)
    })
}
