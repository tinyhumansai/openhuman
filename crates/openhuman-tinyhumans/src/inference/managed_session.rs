//! Session handling for managed inference: the bearer a stored session token
//! yields, and the `SessionExpired` publications that follow an expired or
//! rejected session.

use tinyinference_llm::model::ProviderError;
use tinyinference_llm::providers::tinyhumans::{
    TinyHumansAuthFailure, TinyHumansAuthFailureKind, TinyHumansCredentialKind, TinyHumansOperation,
};

/// Resolve the persisted app-session token for a managed inference caller.
/// Both chat and embeddings use this path so signed-out state, expiry checks,
/// offline-local credential refusal, and the plaintext-endpoint refusal stay
/// consistent.
pub(super) async fn resolve_session_bearer(
    endpoint: &str,
    state_dir: &std::path::Path,
    secrets_encrypt: bool,
    profile_override: Option<&str>,
    operation: &str,
) -> anyhow::Result<String> {
    use openhuman_embed::__host::credentials::session_state::{
        classify_session_token, session_token_from_profile,
    };
    use openhuman_embed::__host::credentials::{AuthService, APP_SESSION_PROVIDER};

    ensure_session_endpoint(endpoint)?;
    if openhuman_embed::__host::platform::llm_gate::is_signed_out() {
        anyhow::bail!("SESSION_EXPIRED: backend session not active — sign in to resume LLM work");
    }
    let auth = AuthService::new(state_dir, secrets_encrypt);
    let profile = auth
        .get_profile(APP_SESSION_PROVIDER, profile_override)
        .await?;
    let selected_token = session_token_from_profile(profile.as_ref());
    // Precheck recorded JWT expiry before constructing any inference request.
    let check = classify_session_token(profile.as_ref(), chrono::Utc::now());
    managed_bearer(check, operation, selected_token.as_deref())
}

/// Resolve the managed backend bearer from a read-only selected-profile
/// snapshot. Inspection never refreshes credentials or publishes expiry.
pub(super) async fn managed_bearer_for_inspection(
    config: &openhuman_embed::__host::config::Config,
) -> anyhow::Result<String> {
    use openhuman_embed::__host::credentials::session_state::{
        check_session_backend, classify_session_token, is_local_session_token,
        load_app_session_profile_read_only, SessionTokenCheck,
    };

    let endpoint = format!(
        "{}/openai/v1",
        crate::backend::url::effective_api_url(&config.api_url).trim_end_matches('/')
    );
    if let Some(key) = openhuman_embed::__host::credentials::api_key::get_api_key_read_only(config)
        .await
        .map_err(|_| anyhow::anyhow!("managed API key profile is unavailable or invalid"))?
    {
        anyhow::ensure!(
            crate::client::endpoint::is_managed_endpoint_for_api_key(&endpoint),
            "managed API key endpoint is not an approved managed endpoint"
        );
        return Ok(key);
    }

    ensure_session_endpoint(&endpoint)?;
    let profile = load_app_session_profile_read_only(config)
        .await
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

/// Refuse to put the session JWT on a plaintext wire: HTTPS, or HTTP only to
/// loopback — the same rule every other backend credential caller applies.
fn ensure_session_endpoint(endpoint: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        crate::client::endpoint::is_safe_endpoint_for_managed_bearer(endpoint),
        "refusing to send the TinyHumans session token to an insecure endpoint: {} — \
         use HTTPS, or HTTP only to loopback",
        super::managed_model::endpoint_origin_for_log(endpoint)
    );
    Ok(())
}

/// The bearer a classified app-session token yields for managed inference.
///
/// Separate from the model factory's bearer resolver so the decision is
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
    check: openhuman_embed::__host::credentials::session_state::SessionTokenCheck,
    operation: &str,
    selected_token: Option<&str>,
) -> anyhow::Result<String> {
    use openhuman_embed::__host::credentials::backend_credential::LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE;
    use openhuman_embed::__host::credentials::session_state::{
        is_local_session_token, SessionTokenCheck,
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
            maybe_publish_local_session_expiry(operation, selected_token);
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
/// the managed-model bearer resolver rejects an expired
/// managed session token before a request is ever sent — mirroring
/// `require_live_session_token`'s
/// pre-flight publish so the credentials subscriber clears state and the UI
/// re-auths exactly as it would on a real backend 401. Deduped via the
/// scheduler gate so N parallel managed turns in one tick don't emit N events.
fn maybe_publish_local_session_expiry(operation: &str, selected_token: Option<&str>) {
    if openhuman_embed::__host::platform::llm_gate::is_signed_out() {
        return;
    }
    log::warn!(
        "[providers][openhuman-backend] managed session token expired locally — \
         publishing SessionExpired before any inference request"
    );
    openhuman_embed::__host::core::bus::BUS.publish(
        openhuman_embed::__host::core::events::DomainEvent::SessionExpired {
            source: operation.to_string(),
            reason: "backend session token expired locally — re-authentication required"
                .to_string(),
            credential_fingerprint: selected_token.map(
                openhuman_embed::__host::credentials::backend_credential::credential_fingerprint,
            ),
        },
    );
}

/// Publish expiry only from TinyInference's explicit managed-endpoint
/// provenance, and only for a 401 on the session credential that this failing
/// request was sent with (not whichever credential the model used last). A
/// failure without credential provenance is never treated as the session. A
/// 403 is a refusal (plan, tier, WAF) and an API-key 401 is a rejected key;
/// neither signs the user out. The provider label is a wire diagnostic and is
/// not an auth ownership signal.
pub(super) fn publish_managed_auth_failure(error: &ProviderError, context: TinyHumansAuthFailure) {
    let api_key = context.credential() != Some(TinyHumansCredentialKind::Session);
    let status = match context.kind() {
        TinyHumansAuthFailureKind::Unauthorized => 401,
        TinyHumansAuthFailureKind::Forbidden => 403,
        TinyHumansAuthFailureKind::Other => return,
    };
    if !openhuman_embed::__host::core::session_expiry::backend_status_means_session_expired(
        status, api_key,
    ) {
        log::warn!(
            "[providers][openhuman-backend] managed {status} (api_key={api_key}) is a refusal — keeping the session"
        );
        return;
    }
    let operation = match context.operation() {
        TinyHumansOperation::Invoke => "invoke",
        TinyHumansOperation::Stream => "stream",
        // Readiness deliberately remains fail-open and does not sign the user
        // out, matching its previous host behavior.
        TinyHumansOperation::Readiness => return,
    };
    let reason = tinyinference_core::sanitize::sanitize_api_error(&error.message);
    openhuman_embed::__host::core::bus::BUS.publish(
        openhuman_embed::__host::core::events::DomainEvent::SessionExpired {
            source: format!("openhuman_backend_model.{operation}({status})"),
            reason,
            credential_fingerprint: context
                .credential_fingerprint()
                .map(|fingerprint| fingerprint.to_hex()),
        },
    );
}

#[cfg(test)]
#[path = "managed_session_tests.rs"]
mod tests;
