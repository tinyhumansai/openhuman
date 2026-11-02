//! Host policy for TinyInference's managed cloud embedding model.
//!
//! This module constructs the owning `CloudEmbeddingModel` with OpenHuman's
//! live credential resolver and the per-request egress disclosure guard. It
//! does not wrap or forward the embedding implementation.

use std::sync::Arc;

use openhuman_embed::__host::egress::{emit_external_transfer, EgressDescriptor};
use openhuman_embed::__host::inference_registry::{EmbeddingRequest, ProviderContext};
use openhuman_embed::__host::security::approval::ApprovalChatContext;
use tinyinference_embeddings::{BearerResolver, CloudEmbeddingModel, EmbeddingEgressGuard};

use super::managed_session::resolve_session_bearer;
use crate::backend::url::effective_api_url;
use crate::client::endpoint::is_managed_endpoint_for_api_key;

/// Builds the owning cloud embedding implementation with the credential scope
/// and turn context supplied by the inference registry.
pub(super) fn from_request(request: EmbeddingRequest<ProviderContext>) -> CloudEmbeddingModel {
    let context = request
        .context
        .embedding
        .as_ref()
        .expect("embedding host context");
    build_cloud_model(
        context
            .config
            .as_ref()
            .and_then(|config| config.api_url.clone()),
        context.state_dir.clone(),
        context.secrets_encrypt,
        &request.model,
        request.dimensions,
        context.chat.clone(),
        None,
    )
}

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
    Ok(build_cloud_model(
        config.api_url.clone(),
        context.state_dir.clone(),
        context.secrets_encrypt,
        &request.model,
        request.dimensions,
        None,
        Some(bearer),
    ))
}

fn build_cloud_model(
    api_url: Option<String>,
    state_dir: std::path::PathBuf,
    secrets_encrypt: bool,
    model: &str,
    dimensions: usize,
    chat: Option<ApprovalChatContext>,
    frozen_bearer: Option<String>,
) -> CloudEmbeddingModel {
    let base_url = format!(
        "{}/openai/v1",
        effective_api_url(&api_url).trim_end_matches('/')
    );
    let key_endpoint = base_url.clone();
    let bearer: BearerResolver = Arc::new(move || {
        let frozen_bearer = frozen_bearer.clone();
        let state_dir = state_dir.clone();
        let key_endpoint = key_endpoint.clone();
        Box::pin(async move {
            if let Some(bearer) = frozen_bearer {
                return Ok(bearer);
            }
            // A stored TinyHumans API key is the bearer outright, with the
            // plaintext guard: never put the key on a non-HTTPS, non-loopback wire.
            if let Some(key) = openhuman_embed::__host::credentials::api_key::get_api_key_in(
                &state_dir,
                secrets_encrypt,
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
                None,
                "managed_embedding.resolve_bearer",
            )
            .await
            .map_err(|error| tinyinference_embeddings::Error::Embedding(error.to_string()))
        })
    });

    let guarded_model = model.to_owned();
    let guarded_chat = chat;
    let egress_guard: EmbeddingEgressGuard = Arc::new(move |_, _| {
        let egress = EgressDescriptor::embedding("cloud", &guarded_model);
        openhuman_embed::__host::egress::enforce_egress_blocking(&egress)
            .map_err(|error| tinyinference_embeddings::Error::Embedding(error.to_string()))?;
        emit_external_transfer(guarded_chat.as_ref(), egress);
        Ok(())
    });

    CloudEmbeddingModel::new(base_url, model.to_owned(), dimensions, bearer)
        .with_egress_guard(egress_guard)
}

#[cfg(test)]
#[path = "managed_embedding_tests.rs"]
mod tests;
