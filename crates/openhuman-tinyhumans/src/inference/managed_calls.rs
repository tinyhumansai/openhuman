//! The [`ChatModel`] implementation of the managed model: per-call wire model,
//! reasoning and thread extensions, usage projection and failure observation.

use async_trait::async_trait;
use tinyinference_llm::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
    ProviderError,
};
use tinyinference_llm::Error as TiError;

use super::managed_model::OpenHumanBackendModel;
use super::managed_reasoning::{apply_reasoning_hint, with_thread_id};
use super::managed_session::{
    maybe_publish_provider_session_expired, maybe_publish_session_expired,
};
use super::managed_usage::project_managed_usage;

/// Log the raw upstream failure at the managed inference dispatch boundary
/// (#5503, part d). The managed unavailability path used to surface the true
/// backend cause only after the web-chat error classifier had already collapsed
/// it to a user-facing bucket, so an operator investigating "all tiers died
/// over hours" had no record of what the backend actually returned. This is the
/// one place every managed `invoke`/`stream` failure passes through, so it's
/// where the diagnostic belongs. Structured fields (`status`/`code`/`provider`/
/// `retryable`) are low-cardinality; the message is secret-scrubbed and capped
/// by [`sanitize_api_error`] before it's logged — no tokens, no full PII.
pub(super) fn log_managed_dispatch_error(err: &TiError, operation: &str) {
    match err {
        TiError::Provider(pe) => log_managed_provider_error(pe, operation),
        other => {
            log::warn!(
                "[providers][openhuman-backend] managed {operation} failed (non-provider error): {}",
                tinyinference_core::sanitize::sanitize_api_error(&other.to_string()),
            );
        }
    }
}

/// Logs a structured provider failure; the detail is secret-scrubbed and
/// truncated because a provider error can echo request content.
pub(super) fn log_managed_provider_error(pe: &ProviderError, operation: &str) {
    log::warn!(
        "[providers][openhuman-backend] managed {operation} failed: status={:?} code={:?} provider={} retryable={} detail={}",
        pe.status,
        pe.code,
        pe.provider,
        pe.retryable,
        tinyinference_core::sanitize::sanitize_api_error(&pe.message),
    );
}

/// Gives a failure reported inside a stream the same handling as a failed
/// `stream()` call: logged, and an expired session starts re-authentication.
pub(super) fn observe_in_band_failure(item: &ModelStreamItem) {
    if let ModelStreamItem::ProviderFailed(pe) = item {
        log_managed_provider_error(pe, "stream (in-band)");
        maybe_publish_provider_session_expired(pe, "stream");
    }
}

#[async_trait]
impl ChatModel<()> for OpenHumanBackendModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    fn supports_input(
        &self,
        modality: tinyinference_llm::model::InputModality,
        mime: &str,
        source: tinyinference_llm::model::InputSource,
    ) -> bool {
        use tinyinference_llm::model::{InputModality, InputSource};
        matches!(
            (modality, source),
            (InputModality::Image, InputSource::Base64 | InputSource::Url)
        ) && matches!(
            mime,
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) || matches!(
            (modality, source),
            (InputModality::Audio, InputSource::Base64)
        ) && matches!(
            mime,
            "audio/wav" | "audio/x-wav" | "audio/mpeg" | "audio/mp3"
        )
    }

    /// Identity for harness response-cache scoping: the backend base URL and
    /// the default tier/model. The session JWT is deliberately absent — it
    /// rotates, and a key derived from it would never hit twice — and the
    /// backend resolves the tier per account anyway, so two accounts sharing
    /// a cache would need their own namespace, not a credential in the key.
    fn cache_identity(&self) -> Option<String> {
        self.base_url()
            .ok()
            .map(|base| format!("openhuman:{base}:{}", self.default_model))
    }

    async fn invoke(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let model = self.build_wire_model()?;
        let response = match model
            .invoke(
                state,
                with_thread_id(apply_reasoning_hint(request), self.thread_id.as_deref()),
            )
            .await
        {
            Ok(response) => response,
            Err(e) => {
                log_managed_dispatch_error(&e, "invoke");
                maybe_publish_session_expired(&e, "invoke");
                return Err(e);
            }
        };
        Ok(project_managed_usage(response))
    }

    async fn stream(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelStream> {
        let model = self.build_wire_model()?;
        // The backend sends its charge on an `openhuman-metadata` frame before
        // `[DONE]`; the crate's SSE parser keeps that envelope on the terminal
        // response's `raw`, so the streamed `Completed` is projected exactly like
        // the `invoke` path above and carries the charged USD.
        match model
            .stream(
                state,
                with_thread_id(apply_reasoning_hint(request), self.thread_id.as_deref()),
            )
            .await
        {
            // A failure can also arrive *inside* an HTTP 200 stream as an SSE
            // `{"error":…}` payload; it never reaches the `Err` arm (#6724).
            Ok(stream) => Ok(stream.map_items(|item| {
                observe_in_band_failure(&item);
                match item {
                    tinyinference_llm::model::ModelStreamItem::Completed(response) => {
                        tinyinference_llm::model::ModelStreamItem::Completed(project_managed_usage(
                            response,
                        ))
                    }
                    other => other,
                }
            })),
            Err(e) => {
                log_managed_dispatch_error(&e, "stream");
                maybe_publish_session_expired(&e, "stream");
                Err(e)
            }
        }
    }
}
