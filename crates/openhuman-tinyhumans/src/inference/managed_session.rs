//! Session handling for managed inference: the bearer a stored session token
//! yields, and the `SessionExpired` publications that follow an expired or
//! rejected session.

use tinyinference_llm::model::ProviderError;
use tinyinference_llm::Error as TiError;

/// Resolve the managed backend bearer from a read-only selected-profile
/// snapshot. Inspection never refreshes credentials or publishes expiry.
pub(super) fn managed_bearer_for_inspection(
    config: &openhuman_embed::__host::config::Config,
) -> anyhow::Result<String> {
    use openhuman_embed::__host::security::credentials::session_support::{
        check_session_backend, classify_session_token, is_local_session_token,
        load_app_session_profile_read_only, SessionTokenCheck,
    };

    if let Some(key) =
        openhuman_embed::__host::security::credentials::api_key::get_api_key_read_only(config)
            .map_err(|_| anyhow::anyhow!("managed API key profile is unavailable or invalid"))?
    {
        let endpoint = format!(
            "{}/openai/v1",
            crate::backend::url::effective_api_url(&config.api_url).trim_end_matches('/')
        );
        anyhow::ensure!(
            crate::client::endpoint::is_managed_endpoint_for_api_key(&endpoint),
            "managed API key endpoint is not an approved managed endpoint"
        );
        return Ok(key);
    }

    let profile = load_app_session_profile_read_only(config)
        .map_err(|_| anyhow::anyhow!("managed session profile is unavailable or invalid"))?;
    check_session_backend(config, profile.as_ref())
        .map_err(|_| anyhow::anyhow!("managed session belongs to a different backend"))?;
    match classify_session_token(profile.as_ref(), chrono::Utc::now()) {
        SessionTokenCheck::Live(token) if !is_local_session_token(&token) => Ok(token),
        SessionTokenCheck::Expired => anyhow::bail!("managed session is expired"),
        SessionTokenCheck::Live(_) => {
            anyhow::bail!("offline local session cannot call managed inference")
        }
        SessionTokenCheck::Absent => anyhow::bail!("managed session credential is unavailable"),
    }
}

/// The bearer a classified app-session token yields for managed inference.
///
/// Separate from `OpenHumanBackendModel::resolve_bearer` so the decision is
/// testable without an on-disk auth profile.
///
/// `exp`-less tokens carry no recorded expiry, so `classify_session_token`
/// reports them `Live`; the offline local session is one of those, and it
/// authenticates no TinyHumans account. Sending it anyway earned a backend
/// `401 "Invalid token"`, which published `SessionExpired` and told a user who
/// was signed in locally that their session had expired (#6932). Refusing here
/// mirrors the arm `resolve_backend_credential` already applies to every
/// backend REST caller, and keeps the local credential intact.
pub(super) fn managed_bearer(
    check: openhuman_embed::__host::security::credentials::session_support::SessionTokenCheck,
) -> anyhow::Result<String> {
    use openhuman_embed::__host::security::credentials::session_support::{
        is_local_session_token, SessionTokenCheck, LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE,
    };

    match check {
        SessionTokenCheck::Live(token) if is_local_session_token(&token) => {
            log::debug!(
                "[providers][openhuman-backend] refusing managed inference for the offline local session"
            );
            anyhow::bail!(LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE)
        }
        SessionTokenCheck::Live(token) => Ok(token),
        SessionTokenCheck::Expired => {
            maybe_publish_local_session_expiry();
            anyhow::bail!(
                "SESSION_EXPIRED: backend session token expired locally — re-authentication required"
            )
        }
        SessionTokenCheck::Absent => {
            anyhow::bail!("No backend session: store a JWT via auth (app-session)")
        }
    }
}

/// Publish a `SessionExpired` event when the local `exp` precheck in
/// `OpenHumanBackendModel::resolve_bearer` rejects an expired
/// managed session token before a request is ever sent — mirroring
/// `require_live_session_token`'s
/// pre-flight publish so the credentials subscriber clears state and the UI
/// re-auths exactly as it would on a real backend 401. Deduped via the
/// scheduler gate so N parallel managed turns in one tick don't emit N events.
fn maybe_publish_local_session_expiry() {
    if openhuman_embed::__host::platform::llm_gate::is_signed_out() {
        return;
    }
    log::warn!(
        "[providers][openhuman-backend] managed session token expired locally — \
         publishing SessionExpired before any inference request"
    );
    openhuman_embed::__host::core::bus::BUS.publish(
        openhuman_embed::__host::core::events::DomainEvent::SessionExpired {
            source: "openhuman_backend_model.resolve_bearer".to_string(),
            reason: "backend session token expired locally — re-authentication required"
                .to_string(),
        },
    );
}

/// Publish a `SessionExpired` event when the backend rejects a crate-native
/// model call with `401`/`403` Unauthorized — mirroring the check in
/// the core `CrateBackedProvider::invoke` which the
/// crate-native path bypasses.
pub(super) fn maybe_publish_session_expired(err: &TiError, operation: &str) {
    if let TiError::Provider(pe) = err {
        maybe_publish_provider_session_expired(pe, operation);
    }
}

pub(super) fn maybe_publish_provider_session_expired(pe: &ProviderError, operation: &str) {
    if pe.provider.as_str() == "OpenHuman" && matches!(pe.status, Some(401 | 403)) {
        let reason = tinyinference_core::sanitize::sanitize_api_error(&pe.message);
        openhuman_embed::__host::core::bus::BUS.publish(
            openhuman_embed::__host::core::events::DomainEvent::SessionExpired {
                source: format!(
                    "openhuman_backend_model.{}({})",
                    operation,
                    pe.status.unwrap_or(0)
                ),
                reason,
            },
        );
    }
}

#[cfg(test)]
#[path = "managed_session_tests.rs"]
mod tests;
