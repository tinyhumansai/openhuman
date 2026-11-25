//! Backend credential resolution and transport-facing credential types.

use crate::config::Config;
use sha2::{Digest, Sha256};

use super::session_state::{
    check_session_backend, classify_session_token, get_session_token, is_local_session_token,
    load_app_session_profile, session_token_from_profile, SessionTokenCheck,
};

const CREDENTIAL_FINGERPRINT_DOMAIN: &[u8] =
    b"tinyinference/tinyhumans-credential-fingerprint/v1\0";

/// A stable, non-secret identifier for one credential value. The domain
/// separator matches TinyInference's request-time managed-credential
/// fingerprint so the host can transport it with `SessionExpired`.
pub fn credential_fingerprint(token: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(CREDENTIAL_FINGERPRINT_DOMAIN);
    digest.update(token.as_bytes());
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Compare a request fingerprint with a credential currently in storage.
pub fn credential_fingerprint_matches(fingerprint: &str, token: &str) -> bool {
    credential_fingerprint(token).as_bytes() == fingerprint.as_bytes()
}

/// The credential a backend request authenticates with.
///
/// Two shapes because the wire differs: a session JWT rides
/// `Authorization: Bearer` everywhere, while an API key rides
/// `Authorization: Bearer` on managed inference but `x-api-key` on the SDK
/// REST routes. Callers that build SDK requests match on this; callers that
/// only need "the secret string" use [`into_secret`](Self::into_secret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendCredential {
    /// The app-session JWT stored by `auth_set_credential {kind: "session"}`.
    Session(String),
    /// The TinyHumans API key stored by `auth_set_credential {kind: "api-key"}`
    /// (see [`super::api_key`]).
    ApiKey(String),
}

impl From<&str> for BackendCredential {
    /// A bare token string is a session JWT — the historical `authed_json`
    /// contract. API keys only ever arrive typed, from
    /// [`resolve_backend_credential`].
    fn from(token: &str) -> Self {
        Self::Session(token.to_string())
    }
}

impl From<&String> for BackendCredential {
    fn from(token: &String) -> Self {
        Self::Session(token.clone())
    }
}

impl From<&BackendCredential> for BackendCredential {
    fn from(credential: &BackendCredential) -> Self {
        credential.clone()
    }
}

impl BackendCredential {
    /// The raw secret, whichever kind it is.
    pub fn into_secret(self) -> String {
        match self {
            Self::Session(s) | Self::ApiKey(s) => s,
        }
    }

    /// The raw secret, borrowed.
    pub fn secret(&self) -> &str {
        match self {
            Self::Session(s) | Self::ApiKey(s) => s,
        }
    }

    pub fn is_api_key(&self) -> bool {
        matches!(self, Self::ApiKey(_))
    }
}

/// Error [`resolve_backend_credential`] returns for the offline local session.
/// Carries [`BACKEND_UNAVAILABLE_PREFIX`](crate::core::observability::BACKEND_UNAVAILABLE_PREFIX)
/// so it classifies as an expected backend-unavailable error.
pub const LOCAL_SESSION_BACKEND_UNAVAILABLE: &str =
    "BACKEND_UNAVAILABLE: hosted account data is unavailable for the offline local session";

/// Error returned by the TinyHumans managed-inference host resolver for the
/// offline local session. The local credential authenticates
/// no TinyHumans account, so managed inference has nothing to bill or route;
/// sending the token anyway earned a backend `401` that read to the user as an
/// expired session (#6932). Distinct from
/// [`LOCAL_SESSION_BACKEND_UNAVAILABLE`] because the chat surface renders this
/// one as its own failure class.
pub const LOCAL_SESSION_MANAGED_INFERENCE_UNAVAILABLE: &str =
    "BACKEND_UNAVAILABLE: managed inference is unavailable for the offline local session";

/// Resolve the backend credential for `config`: the API key when one is
/// stored, else the live app-session token with exactly the classification
/// [`require_live_session_token`] has always applied.
///
/// The API key is checked **first**, and a stored key short-circuits every
/// session check: a library runtime never stores a session, and a desktop that
/// somehow has both has been told explicitly to use the key.
pub async fn resolve_backend_credential(config: &Config) -> Result<BackendCredential, String> {
    if let Some(key) = super::api_key::get_api_key(config)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(BackendCredential::ApiKey(key));
    }
    let profile = load_app_session_profile(config).await?;
    check_session_backend(config, profile.as_ref())?;
    let checked_token = session_token_from_profile(profile.as_ref());
    match classify_session_token(profile.as_ref(), chrono::Utc::now()) {
        // The offline local session has no TinyHumans account behind it, so a
        // hosted call is unavailable by construction — the typed sentinel lets
        // `report_error_or_expected` demote it instead of paging Sentry on
        // every background usage/announcement probe (Sentry 36649).
        SessionTokenCheck::Live(token) if is_local_session_token(&token) => {
            Err(LOCAL_SESSION_BACKEND_UNAVAILABLE.to_owned())
        }
        SessionTokenCheck::Live(token) => Ok(BackendCredential::Session(token)),
        SessionTokenCheck::Absent => {
            Err("no backend session token; run auth_store_session first".to_string())
        }
        SessionTokenCheck::Expired => {
            publish_local_session_expiry("require_live_session_token", checked_token.as_deref())
                .await;
            Err(
                "SESSION_EXPIRED: backend session token expired locally — re-authentication required"
                    .to_string(),
            )
        }
    }
}

/// Resolve a credential for a direct call to the configured backend.
/// A missing transport or unusable credential is an expected offline state.
pub async fn direct_backend_credential(config: &Config, op: &str) -> Option<BackendCredential> {
    if config.resolved_endpoints.control_plane_url.is_none() {
        log::debug!("[backend-direct] {op} skipped: no control plane resolved");
        return None;
    }
    match resolve_backend_credential(config).await {
        Ok(credential) => Some(credential),
        Err(reason) => {
            log::debug!("[backend-direct] {op} skipped: no usable backend credential ({reason})");
            None
        }
    }
}

/// The raw secret a Bearer-only backend caller should send: the stored
/// TinyHumans API key when there is one, else the stored app-session token
/// (unclassified, exactly what [`get_session_token`] returns).
///
/// For module seams that ask "what bearer can I lend right now?" and treat
/// `None` as signed out, where [`resolve_backend_credential`]'s error-for-absent
/// shape does not fit. The backend accepts a key as `Authorization: Bearer`
/// (it recognises it by prefix), so handing the key over as a bearer is
/// correct on every route a key may reach.
pub async fn backend_bearer_secret(config: &Config) -> Result<Option<String>, String> {
    if let Some(key) = super::api_key::get_api_key(config)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Some(key));
    }
    get_session_token(config).await
}

/// Whether *some* backend credential is present — an API key or a non-empty
/// app-session token — without classifying expiry. This is the boot-time
/// "signed in?" question the scheduler gate asks: an expired session still
/// counts as signed in here because the gate's job is to notice the later
/// `SessionExpired`, not to pre-empt it.
pub async fn has_backend_credential(config: &Config) -> bool {
    if super::api_key::has_api_key(config).await {
        return true;
    }
    matches!(get_session_token(config).await, Ok(Some(_)))
}

/// Announce a locally-detected session expiry on the bus so
/// `SessionExpiredSubscriber` clears credentials and the UI re-authenticates,
/// exactly as it would on a real network 401.
///
/// Callers that classify the session themselves — rather than going through
/// [`require_live_session_token`] — MUST call this on the
/// [`SessionTokenCheck::Expired`] arm. Classifying without publishing leaves an
/// expired token in the store with the scheduler gate still open, so nothing
/// ever prompts a re-auth (that regression was caught in review on #6206).
///
/// `operation` names the calling site for the log line only.
pub async fn publish_local_session_expiry(operation: &'static str, token: Option<&str>) {
    // Dedupe the publish via the scheduler gate so N parallel authed
    // callers in one tick don't emit N SessionExpired events.
    if crate::platform::llm_gate::is_signed_out() {
        return;
    }
    // An API-key runtime has no session to expire. Its backend calls never
    // reach here through `resolve_backend_credential` (the key short-circuits),
    // but a stale direct caller must not sign the whole process out over a
    // credential the runtime does not use.
    if ambient_config_has_api_key().await {
        tracing::debug!(
            domain = "credentials",
            operation = operation,
            "[credentials] session expiry ignored — runtime authenticates with an API key"
        );
        return;
    }
    tracing::info!(
        domain = "credentials",
        operation = operation,
        "[credentials] app-session token expired locally — publishing SessionExpired before any backend call"
    );
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::SessionExpired {
        source: "credentials.local_expiry_precheck".to_string(),
        reason: "backend session token expired locally — re-authentication required".to_string(),
        credential_fingerprint: token.map(credential_fingerprint),
    });
}

/// Whether the ambient runtime config carries an API key. Reads the
/// embedder-supplied config off the [`CoreContext`](crate::core::runtime::CoreContext)
/// when there is one; a host that discovers its config from disk has no API
/// key by construction (it is only ever installed by a library runtime).
pub async fn ambient_config_has_api_key() -> bool {
    let config = crate::core::runtime::CoreContext::with_current_embedder_config(Clone::clone);
    match config {
        Some(config) => super::api_key::has_api_key(&config).await,
        None => false,
    }
}
