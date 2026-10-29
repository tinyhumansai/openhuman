//! Per-call credential selection and multimodal endpoint routing for cloud LLMs.
//!
//! A fleet wraps native models, never copies network adapters. Every call picks a
//! healthy credential and retries only failures known to be safe to reroute.
//! A live stream is NEVER replayed after emitting content; its terminal event
//! still updates the pool and drop releases the key lease.
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tinyinference_llm::message::{ContentBlock, Message};
use tinyinference_llm::model::{
    ChatModel, DeferredHandle, DeferredStatus, InputModality, InputSource, ModelProfile,
    ModelRequest, ModelResponse, ModelStream, ModelStreamItem, ProviderError,
};

use super::key_pool::{self, KeyFailure, RotationStrategy};

// Guard against amplifying a provider-wide 401/429 incident across the entire
// configured key fleet. Four total attempts (initial + three fallbacks) per call.
const MAX_KEY_ATTEMPTS_PER_CALL: usize = 4;

/// Detect actual image input (not merely a vision-capable model).
pub(super) fn request_has_image(request: &ModelRequest) -> bool {
    request.messages.iter().any(|message| {
        let blocks: &[ContentBlock] = match message {
            Message::User(m) => &m.content,
            Message::Assistant(m) => &m.content,
            Message::Tool(m) => &m.content,
            _ => &[],
        };
        blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::Image(_)))
    })
}

#[derive(Clone, Copy)]
struct RetryDecision {
    failure: KeyFailure,
    after_ms: Option<u64>,
}
/// A 403 is ambiguous: providers also return it for unsupported models,
/// denied tools and malformed requests. Never evict a credential based on
/// loose substring matches (e.g. `invalid_model`, `unauthorized_model`).
/// Only documented credential-scoped code families permit another key.
fn forbidden_key_failure(code: &str) -> Option<KeyFailure> {
    match code.trim().to_ascii_lowercase().as_str() {
        "invalid_api_key"
        | "invalid_api_key_error"
        | "api_key_invalid"
        | "invalid_token"
        | "invalid_credentials"
        | "authentication_error"
        | "unauthorized"
        | "unauthorized_client"
        | "revoked"
        | "key_revoked"
        | "api_key_revoked"
        | "token_revoked" => Some(KeyFailure::Authentication),
        "quota_exhausted"
        | "quota_exceeded"
        | "insufficient_quota"
        | "resource_exhausted"
        | "rate_limit_exceeded"
        | "rate_limit_error"
        | "rate_limited" => Some(KeyFailure::RateLimited),
        _ => None,
    }
}
fn classify(
    status: Option<u16>,
    code: Option<&str>,
    retry_after_ms: Option<u64>,
) -> Option<RetryDecision> {
    let failure = match status {
        Some(401) => KeyFailure::Authentication,
        Some(429) => KeyFailure::RateLimited,
        Some(403) => forbidden_key_failure(code?)?,
        _ => return None,
    };
    Some(RetryDecision {
        failure,
        after_ms: retry_after_ms,
    })
}
fn provider_decision(error: &ProviderError) -> Option<RetryDecision> {
    // A partial answer belongs to the caller. Never replace it with another
    // credential's replay, even if the terminal status normally permits retry.
    if error.partial_message.is_some() {
        return None;
    }
    classify(error.status, error.code.as_deref(), error.retry_after_ms).or_else(|| {
        // Typed transient transport/server failures may try another credential
        // before any content is exposed. They are not evidence of a bad key or
        // exhausted quota, so preserve its eligibility for a later call.
        (error.retryable && matches!(error.status, None | Some(408) | Some(500..=599))).then_some(
            RetryDecision {
                failure: KeyFailure::Other,
                after_ms: error.retry_after_ms,
            },
        )
    })
}
fn model_decision(error: &tinyinference_llm::Error) -> Option<RetryDecision> {
    match error {
        tinyinference_llm::Error::Provider(e) => provider_decision(e),
        _ => None,
    }
}

const FLEET_DEFERRED_METADATA_KEY: &str = "_openhuman_fleet_route_v1";

#[derive(Clone, Serialize, Deserialize)]
struct DeferredRoute {
    version: u8,
    key_fingerprint: String,
    image: bool,
}

/// A domain-separated digest preserves route identity across app restarts
/// without storing raw API keys in the provider-issued deferred handle.
fn credential_fingerprint(secret: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"openhuman-model-fleet-deferred-route-v1:");
    digest.update(secret.as_bytes());
    let bytes = digest.finalize();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn stamp_deferred_handle(handle: &mut DeferredHandle, key: &str, image: bool) {
    let route = DeferredRoute {
        version: 1,
        key_fingerprint: credential_fingerprint(key),
        image,
    };
    if let Ok(value) = serde_json::to_value(route) {
        handle
            .metadata
            .insert(FLEET_DEFERRED_METADATA_KEY.to_owned(), value);
    }
}
fn decode_deferred_handle(handle: &DeferredHandle) -> tinyinference_llm::Result<DeferredRoute> {
    let value = handle
        .metadata
        .get(FLEET_DEFERRED_METADATA_KEY)
        .ok_or_else(|| {
            tinyinference_llm::Error::Validation(
                "deferred call is missing model-fleet routing metadata".into(),
            )
        })?;
    let route: DeferredRoute = serde_json::from_value(value.clone())?;
    if route.version != 1 {
        return Err(tinyinference_llm::Error::Validation(
            "unsupported model-fleet deferred route version".into(),
        ));
    }
    Ok(route)
}

struct Lease {
    scope: String,
    key: String,
}
impl Lease {
    /// The key-pool selector has already reserved this active lease atomically.
    fn from_reserved(scope: &str, key: &str) -> Self {
        Self {
            scope: scope.to_owned(),
            key: key.to_owned(),
        }
    }
    fn record(&self, failure: Option<RetryDecision>) {
        key_pool::record_result_for(
            &self.scope,
            &self.key,
            failure.map(|failure| failure.failure),
            failure.and_then(|failure| failure.after_ms),
        );
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        key_pool::release(&self.scope, &self.key);
    }
}

/// Key-index alignment is fixed within this instance. The shared key_pool uses
/// raw secret identity for outcomes so reconfiguration in a parallel instance
/// cannot corrupt a different credential's health.
pub(super) struct CredentialFleetModel {
    scope: String,
    keys: Vec<String>,
    models: Vec<Arc<dyn ChatModel<()>>>,
    image_models: Option<Vec<Arc<dyn ChatModel<()>>>>,
    strategy: RotationStrategy,
}
impl CredentialFleetModel {
    pub(super) fn new(
        scope: String,
        keys: Vec<String>,
        models: Vec<Arc<dyn ChatModel<()>>>,
        image_models: Option<Vec<Arc<dyn ChatModel<()>>>>,
        strategy: RotationStrategy,
    ) -> Self {
        assert!(!models.is_empty() && models.len() == keys.len());
        assert!(image_models
            .as_ref()
            .is_none_or(|m| m.len() == models.len()));
        Self {
            scope,
            keys,
            models,
            image_models,
            strategy,
        }
    }
    fn route(&self, idx: usize, image: bool) -> &Arc<dyn ChatModel<()>> {
        if image {
            if let Some(ref group) = self.image_models {
                return &group[idx];
            }
        }
        &self.models[idx]
    }
}
#[async_trait]
impl ChatModel<()> for CredentialFleetModel {
    fn profile(&self) -> Option<&ModelProfile> {
        self.models.first().and_then(|model| model.profile())
    }
    fn supports_input(&self, modality: InputModality, mime: &str, source: InputSource) -> bool {
        // An alternate image route is only selected when the request actually
        // carries an image. Audio/video/document inputs still use the primary
        // models, so capability probes must reflect that same route.
        let models = if modality == InputModality::Image {
            self.image_models.as_ref().unwrap_or(&self.models)
        } else {
            &self.models
        };
        models
            .first()
            .is_some_and(|model| model.supports_input(modality, mime, source))
    }
    fn cache_identity(&self) -> Option<String> {
        self.models.first().and_then(|model| model.cache_identity())
    }
    async fn invoke(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let image = request_has_image(&request);
        let mut tried = Vec::new();
        let mut last_error = None;
        loop {
            if tried.len() >= MAX_KEY_ATTEMPTS_PER_CALL {
                break;
            }
            let Some((index, key)) = key_pool::choose_key_and_acquire_for_run(
                &self.scope,
                &self.keys,
                &tried,
                self.strategy,
                request
                    .correlation
                    .as_ref()
                    .map(|correlation| correlation.run_id.as_str()),
            ) else {
                break;
            };
            tried.push(index);
            let lease = Lease::from_reserved(&self.scope, &key);
            match self
                .route(index, image)
                .invoke(state, request.clone())
                .await
            {
                Ok(result) => {
                    lease.record(None);
                    return Ok(result);
                }
                Err(error) => {
                    let decision = model_decision(&error);
                    lease.record(Some(decision.unwrap_or(RetryDecision {
                        failure: KeyFailure::Other,
                        after_ms: None,
                    })));
                    if decision.is_none() {
                        return Err(error);
                    }
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| tinyinference_llm::Error::Model(
            "No available API keys for this provider; check rate limits or replace invalid keys".to_string()
        )))
    }
    async fn stream(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelStream> {
        use futures::StreamExt;

        let image = request_has_image(&request);
        let mut tried = Vec::new();
        let mut last_error = None;
        loop {
            if tried.len() >= MAX_KEY_ATTEMPTS_PER_CALL {
                break;
            }
            let Some((index, key)) = key_pool::choose_key_and_acquire_for_run(
                &self.scope,
                &self.keys,
                &tried,
                self.strategy,
                request
                    .correlation
                    .as_ref()
                    .map(|correlation| correlation.run_id.as_str()),
            ) else {
                break;
            };
            tried.push(index);
            let lease = Lease::from_reserved(&self.scope, &key);
            let mut live = match self
                .route(index, image)
                .stream(state, request.clone())
                .await
            {
                Ok(stream) => stream,
                Err(error) => {
                    let decision = model_decision(&error);
                    lease.record(Some(decision.unwrap_or(RetryDecision {
                        failure: KeyFailure::Other,
                        after_ms: None,
                    })));
                    if decision.is_none() {
                        return Err(error);
                    }
                    last_error = Some(error);
                    continue;
                }
            };

            // OpenAI-compatible stream adapters can return a stream successfully
            // and then emit an immediate 401/429 ProviderFailed event. Peek at
            // the pre-content preamble so callers never see an attempt that
            // should have been routed to another healthy credential. Never
            // replay once content, usage or a deferred handle is exposed.
            let metadata = live.metadata().clone();
            let mut prefix = Vec::new();
            loop {
                match live.next().await {
                    Some(ModelStreamItem::Started) if prefix.len() < 4 => {
                        prefix.push(ModelStreamItem::Started);
                    }
                    Some(ModelStreamItem::ProviderFailed(error))
                        if provider_decision(&error).is_some() =>
                    {
                        lease.record(provider_decision(&error));
                        last_error = Some(tinyinference_llm::Error::Provider(Box::new(error)));
                        break;
                    }
                    Some(item) => {
                        prefix.push(item);
                        let joined =
                            ModelStream::new(Box::pin(futures::stream::iter(prefix).chain(live)))
                                .with_metadata(metadata);
                        // The lease lives in the mapped stream. Dropping an
                        // unfinished stream still releases its concurrency slot.
                        let mut recorded = false;
                        return Ok(joined.map_items(move |mut item| {
                            if !recorded {
                                match &mut item {
                                    ModelStreamItem::Completed(_) => {
                                        lease.record(None);
                                        recorded = true;
                                    }
                                    ModelStreamItem::Deferred(handle) => {
                                        stamp_deferred_handle(handle, &lease.key, image);
                                        lease.record(None);
                                        recorded = true;
                                    }
                                    ModelStreamItem::ProviderFailed(error) => {
                                        lease.record(Some(provider_decision(error).unwrap_or(
                                            RetryDecision {
                                                failure: KeyFailure::Other,
                                                after_ms: None,
                                            },
                                        )));
                                        recorded = true;
                                    }
                                    ModelStreamItem::Failed(_) => {
                                        lease.record(Some(RetryDecision {
                                            failure: KeyFailure::Other,
                                            after_ms: None,
                                        }));
                                        recorded = true;
                                    }
                                    _ => {}
                                }
                            }
                            item
                        }));
                    }
                    None => {
                        lease.record(Some(RetryDecision {
                            failure: KeyFailure::Other,
                            after_ms: None,
                        }));
                        return Err(tinyinference_llm::Error::Model(
                            "provider stream ended before returning content or a terminal result"
                                .into(),
                        ));
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| tinyinference_llm::Error::Model(
            "No available API keys for this provider; check rate limits or replace invalid keys".into()
        )))
    }

    async fn fetch_deferred(
        &self,
        handle: &DeferredHandle,
    ) -> tinyinference_llm::Result<DeferredStatus> {
        let route = decode_deferred_handle(handle)?;
        let index = self
            .keys
            .iter()
            .position(|secret| credential_fingerprint(secret) == route.key_fingerprint)
            .ok_or_else(|| {
                tinyinference_llm::Error::Validation(
                    "the credential used for this deferred call has been removed".into(),
                )
            })?;
        let mut original = handle.clone();
        original.metadata.remove(FLEET_DEFERRED_METADATA_KEY);
        self.route(index, route.image)
            .fetch_deferred(&original)
            .await
    }
}

#[cfg(test)]
#[path = "credential_fleet_tests.rs"]
mod tests;
