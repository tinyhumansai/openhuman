//! Host policy for TinyInference's managed cloud embedding model.
//!
//! This module constructs the owning `CloudEmbeddingModel` with OpenHuman's
//! live credential resolver and the per-request egress disclosure guard. It
//! does not wrap or forward the embedding implementation.
//!
//! The doctor's read-only inspection model is the one exception to the guard:
//! it freezes the selected profile's credential, sends only the doctor's fixed
//! probe, and never asks TinySecurity, which the doctor does not load.

use std::sync::Arc;

use openhuman_embed::__host::egress::{emit_external_transfer, EgressDescriptor};
use openhuman_embed::__host::inference_registry::{
    EmbeddingCallContext, EmbeddingRequest, ProviderContext,
};
use tinyinference_embeddings::{BearerResolver, CloudEmbeddingModel, EmbeddingEgressGuard};

use super::managed_session::resolve_session_bearer;
use crate::backend::url::effective_api_url;
use crate::client::endpoint::is_managed_endpoint_for_api_key;

/// Builds the owning cloud embedding implementation with the credential scope
/// and turn context supplied by the inference registry. Every batch it sends
/// first passes the TinySecurity egress guard.
pub(super) fn from_request(request: EmbeddingRequest<ProviderContext>) -> CloudEmbeddingModel {
    let context = request
        .context
        .embedding
        .as_ref()
        .expect("embedding host context");
    guarded_cloud_model(
        context
            .config
            .as_ref()
            .and_then(|config| config.api_url.clone()),
        context,
        &request.model,
        request.dimensions,
    )
}

/// Builds the doctor's managed embedding probe from the selected profile's
/// frozen, read-only credential.
pub(super) async fn from_request_for_inspection(
    request: &EmbeddingRequest<ProviderContext>,
) -> tinyinference_embeddings::Result<CloudEmbeddingModel> {
    let context = request
        .context
        .embedding
        .as_ref()
        .expect("embedding host context");
    let config = context.config.as_ref().ok_or_else(|| {
        tinyinference_embeddings::Error::Validation(
            "selected managed embedding config is unavailable".to_owned(),
        )
    })?;
    let bearer = super::managed_session::managed_bearer_for_inspection(config)
        .await
        .map_err(|_| {
            tinyinference_embeddings::Error::Validation(
                "selected managed embedding credential is unavailable".to_owned(),
            )
        })?;
    // No egress guard: the doctor never loads TinySecurity, and this model
    // only ever carries the doctor's fixed probe string. The core runs the
    // host-side local-only check before asking for it, as for the chat probe.
    // Real embedding traffic is built by `from_request`, which keeps the guard.
    let frozen: BearerResolver = Arc::new(move || {
        let bearer = bearer.clone();
        Box::pin(async move { Ok(bearer) })
    });
    Ok(cloud_model(
        config.api_url.clone(),
        &request.model,
        request.dimensions,
        frozen,
    ))
}

/// The managed endpoint's embedding model with `bearer`, and no host policy.
fn cloud_model(
    api_url: Option<String>,
    model: &str,
    dimensions: usize,
    bearer: BearerResolver,
) -> CloudEmbeddingModel {
    CloudEmbeddingModel::new(
        managed_base_url(&api_url),
        model.to_owned(),
        dimensions,
        bearer,
    )
}

fn managed_base_url(api_url: &Option<String>) -> String {
    format!(
        "{}/openai/v1",
        effective_api_url(api_url).trim_end_matches('/')
    )
}

/// The managed endpoint's embedding model with OpenHuman's live credential
/// resolver for `context`, and the TinySecurity egress guard on every batch.
fn guarded_cloud_model(
    api_url: Option<String>,
    context: &EmbeddingCallContext,
    model: &str,
    dimensions: usize,
) -> CloudEmbeddingModel {
    let bearer = bearer_resolver(managed_base_url(&api_url), context);

    let guarded_model = model.to_owned();
    let guarded_chat = context.chat.clone();
    let egress_guard: EmbeddingEgressGuard = Arc::new(move |_, _| {
        let egress = EgressDescriptor::embedding("cloud", &guarded_model);
        openhuman_embed::__host::egress::enforce_egress_blocking(&egress)
            .map_err(|error| tinyinference_embeddings::Error::Embedding(error.to_string()))?;
        emit_external_transfer(guarded_chat.as_ref(), egress);
        Ok(())
    });

    cloud_model(api_url, model, dimensions, bearer).with_egress_guard(egress_guard)
}

/// Resolves the bearer for each embedding batch from `context`'s credential
/// scope: a stored TinyHumans API key first, else the app session, read
/// through the store bound to `context.config` when it owns that scope.
fn bearer_resolver(key_endpoint: String, context: &EmbeddingCallContext) -> BearerResolver {
    let state_dir = context.state_dir.clone();
    let secrets_encrypt = context.secrets_encrypt;
    let config = context.config.clone().map(Arc::new);
    Arc::new(move || {
        let state_dir = state_dir.clone();
        let key_endpoint = key_endpoint.clone();
        let config = config.clone();
        Box::pin(async move {
            // A stored TinyHumans API key is the bearer outright, with the
            // plaintext guard: never put the key on a non-HTTPS, non-loopback wire.
            if let Some(key) = openhuman_embed::__host::credentials::api_key::get_api_key_in(
                &state_dir,
                secrets_encrypt,
                config.as_deref(),
            )
            .await
            .map_err(|error| tinyinference_embeddings::Error::Embedding(error.to_string()))?
            {
                if !is_managed_endpoint_for_api_key(&key_endpoint) {
                    return Err(tinyinference_embeddings::Error::Validation(format!(
                        "refusing to send the TinyHumans API key over a non-HTTPS, non-loopback \
                         endpoint: {}",
                        super::managed_model::endpoint_origin_for_log(&key_endpoint)
                    )));
                }
                log::debug!("[tinyhumans][embeddings] authenticating with api-key");
                return Ok(key);
            }
            resolve_session_bearer(
                &key_endpoint,
                &state_dir,
                secrets_encrypt,
                config.as_deref(),
                None,
                "managed_embedding.resolve_bearer",
            )
            .await
            .map_err(|error| tinyinference_embeddings::Error::Embedding(error.to_string()))
        })
    })
}

#[cfg(test)]
#[path = "managed_embedding_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "managed_embedding_inspection_tests.rs"]
mod inspection_tests;
