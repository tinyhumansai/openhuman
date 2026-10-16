//! Managed cloud embeddings: `POST /openai/v1/embeddings` on the hosted backend
//! with the session JWT or stored API key, wrapped in the core's egress guard.
//! Registered as the embedding factory for the managed provider id.

use std::sync::Arc;

use openhuman_embed::__host::egress::{emit_external_transfer, enforce_egress, EgressDescriptor};
use openhuman_embed::__host::inference_registry::EmbeddingRequest;
use openhuman_embed::__host::security::approval::ApprovalChatContext;
use openhuman_embed::__host::security::credentials::{AuthService, APP_SESSION_PROVIDER};
use tinyinference_embeddings::{BearerResolver, CloudEmbeddingModel, EmbeddingModel};

use crate::backend::url::effective_api_url;
use crate::client::endpoint::is_managed_endpoint_for_api_key;

/// Host-owned credential resolution and egress disclosure around the
/// crate-owned cloud transport.
///
/// The egress guard (policy check, then the `ExternalTransferPending`
/// disclosure) runs before the batch reaches the transport, so it can carry the
/// chat of the turn that made the call. One built without a chat (background
/// sync, the embedding RPC) still discloses, unrouted.
pub struct ManagedEmbeddingModel {
    inner: Arc<CloudEmbeddingModel>,
    model: String,
    chat: Option<ApprovalChatContext>,
}

impl ManagedEmbeddingModel {
    /// Builds the embedder for the credential scope `request` carries.
    pub fn from_request(request: &EmbeddingRequest<'_>) -> Self {
        Self::new(
            request.config.and_then(|config| config.api_url.clone()),
            request.state_dir.clone(),
            request.secrets_encrypt,
            request.model,
            request.dimensions,
            request.chat.clone(),
        )
    }

    /// Builds the embedder against `api_url` (or the default backend) with
    /// credentials read from `state_dir`.
    pub fn new(
        api_url: Option<String>,
        state_dir: std::path::PathBuf,
        secrets_encrypt: bool,
        model: &str,
        dimensions: usize,
        chat: Option<ApprovalChatContext>,
    ) -> Self {
        let base_url = format!(
            "{}/openai/v1",
            effective_api_url(&api_url).trim_end_matches('/')
        );
        let key_endpoint = base_url.clone();
        let bearer: BearerResolver = Arc::new(move || {
            // A stored TinyHumans API key is the bearer outright, with the
            // plaintext guard: never put the key on a non-HTTPS, non-loopback wire.
            if let Some(key) =
                openhuman_embed::__host::security::credentials::api_key::get_api_key_in(
                    &state_dir,
                    secrets_encrypt,
                )
                .map_err(|error| tinyinference_embeddings::Error::Embedding(error.to_string()))?
            {
                if !is_managed_endpoint_for_api_key(&key_endpoint) {
                    return Err(tinyinference_embeddings::Error::Validation(format!(
                        "refusing to send the TinyHumans API key over a non-HTTPS, non-loopback \
                         endpoint: {key_endpoint}"
                    )));
                }
                log::debug!("[tinyhumans][embeddings] authenticating with api-key");
                return Ok(key);
            }
            let auth = AuthService::new(&state_dir, secrets_encrypt);
            auth.get_provider_bearer_token(APP_SESSION_PROVIDER, None)
                .map_err(|error| tinyinference_embeddings::Error::Embedding(error.to_string()))?
                .filter(|token| !token.trim().is_empty())
                .ok_or_else(|| {
                    tinyinference_embeddings::Error::Validation(
                        "No backend session for cloud embeddings: log in to OpenHuman or set a \
                         TinyHumans API key"
                            .into(),
                    )
                })
        });
        let model = model.to_string();
        Self {
            inner: Arc::new(CloudEmbeddingModel::new(
                base_url,
                model.clone(),
                dimensions,
                bearer,
            )),
            model,
            chat,
        }
    }

    /// This embedder with its egress disclosures routed to `chat` (the turn's
    /// thread and client); `None` leaves them unrouted. Shares the transport,
    /// so it is cheap to do per turn.
    #[cfg(test)]
    #[must_use]
    pub fn routed_to(&self, chat: Option<ApprovalChatContext>) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            model: self.model.clone(),
            chat,
        }
    }

    fn guard(&self) -> tinyinference_embeddings::Result<()> {
        let egress = EgressDescriptor::embedding("cloud", &self.model);
        enforce_egress(&egress)
            .map_err(|error| tinyinference_embeddings::Error::Embedding(error.to_string()))?;
        emit_external_transfer(self.chat.as_ref(), egress);
        Ok(())
    }
}

#[async_trait::async_trait]
impl EmbeddingModel for ManagedEmbeddingModel {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    fn signature(&self) -> String {
        self.inner.signature()
    }

    async fn embed(&self, texts: &[String]) -> tinyinference_embeddings::Result<Vec<Vec<f32>>> {
        // Only a batch that will actually leave is guarded: an empty one answers
        // locally, and a blank input is refused by the transport.
        if !texts.is_empty() && texts.iter().all(|text| !text.trim().is_empty()) {
            self.guard()?;
        }
        self.inner.embed(texts).await
    }
}

#[cfg(test)]
#[path = "managed_embedding_tests.rs"]
mod tests;
