//! OpenHuman host configuration for TinyHumans' managed chat model.
//!
//! Transport, request adaptation, response projection and managed-auth
//! provenance belong to TinyInference's `TinyHumansModel`. This module only
//! supplies OpenHuman's live credential resolver and client policy.

use std::path::PathBuf;
use tinyinference_llm::providers::tinyhumans::{
    TinyHumansCredential, TinyHumansCredentialKind, TinyHumansFailurePoint, TinyHumansModel,
};

use super::managed_session::{publish_managed_auth_failure, resolve_session_bearer};
use crate::backend::url::effective_api_url;
use crate::client::endpoint::is_managed_endpoint_for_api_key;

/// Provider label used by the hosted-backend error-code classifier. This is
/// diagnostic classification only; managed-auth ownership uses the owner's
/// typed provenance callback below.
pub const PROVIDER_LABEL: &str = "tinyhumans";

/// Where the managed model reads its credentials from.
#[derive(Debug, Clone, Default)]
pub struct ManagedOptions {
    /// Directory holding the credential store (`auth-profiles.json`, API key).
    /// `None` resolves the process default (`~/.openhuman`).
    pub openhuman_dir: Option<PathBuf>,
    /// Whether the credential store is encrypted at rest.
    pub secrets_encrypt: bool,
    /// An auth-profile name to use instead of the default app session.
    pub auth_profile_override: Option<String>,
}

impl ManagedOptions {
    fn state_dir(&self) -> PathBuf {
        self.openhuman_dir.clone().unwrap_or_else(|| {
            directories::UserDirs::new()
                .map(|dirs| dirs.home_dir().join(".openhuman"))
                .unwrap_or_else(|| PathBuf::from(".openhuman"))
        })
    }

    #[cfg(test)]
    async fn resolve_bearer(&self, endpoint: &str) -> anyhow::Result<String> {
        self.resolve_bearer_kind(endpoint)
            .await
            .map(|(bearer, _)| bearer)
    }

    /// The bearer, and whether it is the API key (`true`) or the session JWT.
    async fn resolve_bearer_kind(&self, endpoint: &str) -> anyhow::Result<(String, bool)> {
        if let Some(key) = openhuman_embed::__host::credentials::api_key::get_api_key_in(
            &self.state_dir(),
            self.secrets_encrypt,
        )
        .await?
        {
            if !is_managed_endpoint_for_api_key(endpoint) {
                anyhow::bail!(
                    "refusing to send the TinyHumans API key to an unmanaged or insecure \
                     endpoint: {} — use the managed backend or loopback for local testing",
                    endpoint_origin_for_log(endpoint)
                );
            }
            log::debug!(
                "[providers][openhuman-backend] authenticating managed inference with api-key"
            );
            return Ok((key, true));
        }
        resolve_session_bearer(
            endpoint,
            &self.state_dir(),
            self.secrets_encrypt,
            self.auth_profile_override.as_deref(),
            "openhuman_backend_model.resolve_bearer",
        )
        .await
        .map(|token| (token, false))
    }
}

/// Returns only a URL origin for diagnostics, never userinfo, path, query, or
/// fragment data. Invalid and opaque URLs use a constant placeholder.
pub(super) fn endpoint_origin_for_log(endpoint: &str) -> String {
    url::Url::parse(endpoint)
        .ok()
        .map(|url| url.origin().ascii_serialization())
        .filter(|origin| origin != "null")
        .unwrap_or_else(|| "<invalid endpoint>".to_string())
}

/// Constructs the owning TinyInference model with OpenHuman's rotating
/// credential source and host-specific connection policy.
pub(super) fn build_chat_model(
    api_url: Option<&str>,
    options: &ManagedOptions,
    model: &str,
    native_tool_calling: bool,
    thread_id: Option<&str>,
    observe_auth_failures: bool,
) -> anyhow::Result<TinyHumansModel> {
    let endpoint = effective_api_url(&api_url.map(str::to_owned));
    let resolver_options = options.clone();
    let resolver_endpoint = endpoint.clone();
    // The owner hands each request's credential kind back on that request's
    // auth failure, so an API-key 401 (a rejected key) is never read as an
    // expired session, even while the stored credential changes mid-flight.
    let mut managed = TinyHumansModel::new_with_credential(
        endpoint,
        model,
        super::tiers::MODEL_MANAGED_DEFAULT,
        move || {
            let options = resolver_options.clone();
            let endpoint = resolver_endpoint.clone();
            Box::pin(async move {
                let (bearer, api_key) = options
                    .resolve_bearer_kind(&endpoint)
                    .await
                    .map_err(|error| tinyinference_llm::Error::Model(error.to_string()))?;
                let kind = if api_key {
                    TinyHumansCredentialKind::ApiKey
                } else {
                    TinyHumansCredentialKind::Session
                };
                Ok(TinyHumansCredential { bearer, kind })
            })
        },
    )?
    .with_cache_namespace("openhuman")
    .with_native_tool_calling(native_tool_calling)
    .with_thread_id(thread_id)
    .with_error_observer(log_managed_error);

    if observe_auth_failures {
        managed = managed.with_managed_auth_failure_handler(publish_managed_auth_failure);
    }
    Ok(managed)
}

/// Constructs the native managed model with a credential read once from the
/// doctor's read-only snapshot. The closure passed to TinyInference can only
/// return that frozen value; this path installs no mutable session-expiry hook.
pub(super) fn build_chat_model_for_inspection(
    api_url: Option<&str>,
    model: &str,
    bearer: String,
    native_tool_calling: bool,
    thread_id: Option<&str>,
) -> anyhow::Result<TinyHumansModel> {
    let endpoint = effective_api_url(&api_url.map(str::to_owned));
    let managed = TinyHumansModel::new(
        endpoint,
        model,
        super::tiers::MODEL_MANAGED_DEFAULT,
        move || {
            let bearer = bearer.clone();
            Box::pin(async move { Ok(bearer) })
        },
    )?
    .with_cache_namespace("openhuman")
    .with_native_tool_calling(native_tool_calling)
    .with_thread_id(thread_id)
    .with_error_observer(log_managed_error);
    // Do not attach `with_managed_auth_failure_handler`: inspection must not
    // publish expiry state or mutate the selected credential profile.
    Ok(managed)
}

fn log_managed_error(error: &tinyinference_llm::Error, point: TinyHumansFailurePoint) {
    let operation = match point {
        TinyHumansFailurePoint::Invoke => "invoke",
        TinyHumansFailurePoint::StreamOpen => "stream",
        TinyHumansFailurePoint::StreamItem => "stream (in-band)",
        TinyHumansFailurePoint::Readiness => return,
    };
    match error {
        tinyinference_llm::Error::Provider(provider) => {
            log::warn!(
                "[providers][openhuman-backend] managed {operation} failed: status={:?} code={:?} provider={} retryable={} detail={}",
                provider.status,
                provider.code,
                provider.provider,
                provider.retryable,
                tinyinference_core::sanitize::sanitize_api_error(&provider.message),
            );
        }
        other => log::warn!(
            "[providers][openhuman-backend] managed {operation} failed (non-provider error): {}",
            tinyinference_core::sanitize::sanitize_api_error(&other.to_string()),
        ),
    }
}

#[cfg(test)]
#[path = "managed_model_tests.rs"]
mod tests;
