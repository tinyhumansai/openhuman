//! Session/auth helpers used by RPC and [`crate::core_server::helpers`].

use crate::config::Config;
use crate::core::runtime::DispatchCtx;

use super::profiles::{AuthProfile, AuthProfileKind, TokenSet};
use super::responses::{AuthProfileSummary, AuthStateResponse};
use super::AuthService;

use super::{APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

pub const LOCAL_SESSION_USER_ID: &str = "local";

pub fn local_session_user_id() -> String {
    let host = hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .unwrap_or_default();
    let slug = slugify_local_session_host(&host);
    format!("local-{slug}")
}

fn slugify_local_session_host(host: &str) -> String {
    let mut slug = String::with_capacity(host.len());
    let mut last_was_sep = false;

    for ch in host.trim().chars() {
        let normalized = ch.to_ascii_lowercase();
        if normalized.is_ascii_alphanumeric() {
            slug.push(normalized);
            last_was_sep = false;
        } else if !slug.is_empty() && !last_was_sep {
            slug.push('-');
            last_was_sep = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "device".to_string()
    } else {
        slug
    }
}

pub fn profile_name_or_default(value: Option<&str>) -> &str {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_AUTH_PROFILE_NAME)
}

pub fn is_local_session_token(token: &str) -> bool {
    let trimmed = token.trim();
    let mut parts = trimmed.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(_), Some(_), Some("local"), None)
    )
}

/// Whether the current RPC workspace is authenticated by the offline local
/// credential. A backend-only route may report a missing session or 401 while
/// this credential remains valid; those errors must not broadcast sign-out.
/// A failed lookup leaves the ordinary session-expiry path in place.
pub async fn current_session_is_local(ctx: &DispatchCtx) -> bool {
    let Ok(config) = ctx.load_config().await else {
        return false;
    };
    get_session_token(&config)
        .await
        .ok()
        .flatten()
        .is_some_and(|token| is_local_session_token(&token))
}

pub fn parse_fields_value(
    input: Option<serde_json::Value>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let Some(value) = input else {
        return Ok(std::collections::HashMap::new());
    };

    let Some(map) = value.as_object() else {
        return Err("fields must be a JSON object".to_string());
    };

    let mut out = std::collections::HashMap::new();
    for (key, raw) in map {
        if key.trim().is_empty() {
            return Err("fields cannot contain empty keys".to_string());
        }
        let rendered = match raw {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(s) => s.clone(),
            _ => raw.to_string(),
        };
        out.insert(key.clone(), rendered);
    }

    Ok(out)
}

fn profile_kind_label(kind: AuthProfileKind) -> String {
    match kind {
        AuthProfileKind::OAuth => "oauth".to_string(),
        AuthProfileKind::Token => "token".to_string(),
    }
}

pub fn summarize_auth_profile(
    profile: &crate::credentials::profiles::AuthProfile,
) -> AuthProfileSummary {
    let mut metadata_keys = profile
        .metadata
        .keys()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>();
    metadata_keys.sort();

    AuthProfileSummary {
        id: profile.id.clone(),
        provider: profile.provider.clone(),
        profile_name: profile.profile_name.clone(),
        kind: profile_kind_label(profile.kind),
        account_id: profile.account_id.clone(),
        workspace_id: profile.workspace_id.clone(),
        metadata_keys,
        updated_at: profile.updated_at.to_rfc3339(),
        has_token: profile.token.as_ref().is_some_and(|v| !v.trim().is_empty()),
        has_token_set: profile
            .token_set
            .as_ref()
            .map(|TokenSet { access_token, .. }| !access_token.trim().is_empty())
            .unwrap_or(false),
    }
}

fn session_user_value(
    profile: &crate::credentials::profiles::AuthProfile,
) -> Option<serde_json::Value> {
    profile
        .metadata
        .get("user_json")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
}

/// The kinds of backend credential the core can hold. The `as_str` values are
/// the `auth.set_credential` / `auth.clear_credential` / `auth.get_state` wire
/// vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    /// A TinyHumans session JWT.
    Session,
    /// A TinyHumans API key.
    ApiKey,
    /// The offline local session (`is_local_session_token`).
    Local,
}

impl CredentialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => super::responses::CREDENTIAL_SESSION,
            Self::ApiKey => super::responses::CREDENTIAL_API_KEY,
            Self::Local => super::responses::CREDENTIAL_LOCAL,
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            super::responses::CREDENTIAL_SESSION => Some(Self::Session),
            super::responses::CREDENTIAL_API_KEY => Some(Self::ApiKey),
            super::responses::CREDENTIAL_LOCAL => Some(Self::Local),
            _ => None,
        }
    }

    /// Classify a bare token the way the core always has: the local-session
    /// shape is `Local`, anything else is a session JWT. An API key is never
    /// inferred from shape — callers that hold one say so.
    pub fn classify(token: &str) -> Self {
        if is_local_session_token(token) {
            Self::Local
        } else {
            Self::Session
        }
    }
}

/// The subject of a JWT, read from its payload claims without verification.
/// Checked in order: `sub`, `userId`, `user_id`, `_id`, `id`.
pub fn user_id_from_jwt_claims(token: &str) -> Option<String> {
    let claims = crate::credentials::jwt::decode_jwt_payload(token)?;
    let obj = claims.as_object()?;
    ["sub", "userId", "user_id", "_id", "id"]
        .iter()
        .find_map(|key| obj.get(*key).and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub async fn build_session_state(config: &Config) -> Result<AuthStateResponse, String> {
    // The API key wins whenever both are present — same precedence as
    // `resolve_backend_credential`, which every backend request actually
    // authenticates with. Checked before the session profile loads: a host
    // that holds both (a runtime built with `.session(...)` and
    // `.api_key(...)`, or an inherited workspace carrying a prior session)
    // must see `credential: "api-key"`, not "session", or `auth.get_state`
    // would misidentify what's actually authenticating its requests. An API
    // key carries no user identity, so `user_id`/`user` stay `None` rather
    // than leaking the session's.
    if super::api_key::has_api_key(config).await {
        return Ok(AuthStateResponse {
            is_authenticated: true,
            user_id: None,
            user: None,
            profile_id: None,
            credential: Some(super::responses::CREDENTIAL_API_KEY.to_string()),
            expires_at: None,
            issuing_backend: None,
        });
    }
    let profile = load_app_session_profile(config).await?;
    Ok(session_state_from_profile(profile.as_ref()))
}

pub async fn get_session_token(config: &Config) -> Result<Option<String>, String> {
    let profile = load_app_session_profile(config).await?;
    session_token_for_config(config, profile.as_ref())
}

/// Release an already-loaded profile's token only for its issuing backend.
/// Snapshot callers may turn a mismatch into `None` while retaining identity.
pub fn session_token_for_config(
    config: &Config,
    profile: Option<&AuthProfile>,
) -> Result<Option<String>, String> {
    check_session_backend(config, profile)?;
    Ok(session_token_from_profile(profile))
}

/// Non-secret issuing-backend association stored beside the encrypted JWT.
/// Legacy sessions are unbound: their origin cannot be inferred retroactively.
pub const SESSION_ISSUING_BACKEND_META: &str = "session_issuing_backend";

pub fn normalize_session_backend(raw: &str) -> Result<String, String> {
    let mut parsed =
        url::Url::parse(raw.trim()).map_err(|_| "invalid issuing backend".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("invalid issuing backend".to_string());
    }
    // SessionClient addresses auth endpoints at the backend origin.
    parsed.set_path("");
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

/// Purely verifies that a stored session belongs to the selected config's
/// control-plane endpoint. It does not publish expiry events or mutate auth state.
pub fn check_session_backend(config: &Config, profile: Option<&AuthProfile>) -> Result<(), String> {
    let Some(profile) = profile else {
        return Ok(());
    };
    if session_token_from_profile(Some(profile))
        .as_deref()
        .is_some_and(is_local_session_token)
    {
        return Ok(());
    }
    let Some(issuing) = profile.metadata.get(SESSION_ISSUING_BACKEND_META) else {
        return Ok(());
    };
    let matches = config
        .resolved_endpoints
        .control_plane_url
        .clone()
        .and_then(|base| normalize_session_backend(&base).ok())
        .zip(normalize_session_backend(issuing).ok())
        .is_some_and(|(current, issuing)| current == issuing);
    if !matches {
        return Err("SESSION_BACKEND_MISMATCH: stored session belongs to a different backend; restore its backend or sign in again".to_string());
    }
    Ok(())
}

/// Metadata key under which the app-session profile records the decoded JWT
/// `exp` (RFC3339). Written at `store_session` time (`ops::store_session`).
/// Absent for local offline sessions and `exp`-less tokens.
pub const SESSION_EXPIRES_AT_META: &str = "session_expires_at";

/// Treat a token as expired this many seconds *before* its real `exp`, so an
/// in-flight request can't race the boundary into a backend 401.
const SESSION_EXPIRY_SKEW_SECS: i64 = 30;

fn session_expires_at_from_profile(
    profile: Option<&AuthProfile>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    profile?
        .metadata
        .get(SESSION_EXPIRES_AT_META)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Liveness verdict for the stored app-session token. Pure + `now`-injected so
/// the expiry decision is unit-testable without a credential store.
#[derive(Debug, PartialEq, Eq)]
pub enum SessionTokenCheck {
    /// No token stored (signed out / never authed).
    Absent,
    /// Token present but its recorded `exp` is in the past (− skew).
    Expired,
    /// Token present and within its validity window (or no recorded expiry).
    Live(String),
}

pub fn classify_session_token(
    profile: Option<&AuthProfile>,
    now: chrono::DateTime<chrono::Utc>,
) -> SessionTokenCheck {
    let Some(token) = session_token_from_profile(profile) else {
        return SessionTokenCheck::Absent;
    };
    if let Some(exp) = session_expires_at_from_profile(profile) {
        if now + chrono::Duration::seconds(SESSION_EXPIRY_SKEW_SECS) >= exp {
            return SessionTokenCheck::Expired;
        }
    }
    SessionTokenCheck::Live(token)
}

/// Canonical guard for every backend `authed_json` caller (team, billing, and
/// any future authed RPC op). Returns the live app-session token, or an error
/// the JSON-RPC layer classifies as session-expiry — **without sending a doomed
/// request**.
///
/// - absent / empty token → "no backend session token" (local, no network).
/// - token whose recorded `exp` is past (− skew) → publishes `SessionExpired`
///   **once** (so the credentials subscriber clears state and the UI re-auths,
///   exactly as on a real network 401) and returns the `SESSION_EXPIRED`
///   sentinel. The doomed 401 is never sent — this is the #3297 RCA that stops
///   the TAURI-RUST-8WY (team usage) / 8WZ (current plan)
///   flood at its source instead of demoting it after the fact.
/// - token with no recorded expiry (local offline session / `exp`-less JWT) →
///   presence-only check; the `flatten_authed_error` 401 net still covers a
///   server-side revocation that precedes the recorded `exp`.
pub async fn require_live_session_token(config: &Config) -> Result<String, String> {
    super::backend_credential::resolve_backend_credential(config)
        .await
        .map(super::backend_credential::BackendCredential::into_secret)
}

/// Load the `app-session` profile once. Callers that need both the
/// session-state view (`session_state_from_profile`) AND the raw token
/// (`session_token_from_profile`) should call this once and pass the
/// result to both helpers — every load takes the auth-profile store
/// lock, and on Windows the `app_state_snapshot` hot path used to take
/// it twice per call which materially increased lock contention
/// (Sentry: "Timed out waiting for auth profile lock").
pub async fn load_app_session_profile(config: &Config) -> Result<Option<AuthProfile>, String> {
    let auth_service = AuthService::from_config(config);
    auth_service
        .get_profile(APP_SESSION_PROVIDER, None)
        .await
        .map_err(|e| e.to_string())
}

/// Reads the selected app-session profile for doctor checks without opening
/// the ordinary mutable auth-profile store.
pub async fn load_app_session_profile_read_only(
    config: &Config,
) -> Result<Option<AuthProfile>, super::profiles::ReadOnlyAuthProfileError> {
    AuthService::get_profile_read_only(config, APP_SESSION_PROVIDER, None).await
}

pub fn session_state_from_profile(profile: Option<&AuthProfile>) -> AuthStateResponse {
    let Some(profile) = profile else {
        return AuthStateResponse {
            is_authenticated: false,
            user_id: None,
            user: None,
            profile_id: None,
            credential: None,
            expires_at: None,
            issuing_backend: None,
        };
    };

    let token = session_token_from_profile(Some(profile));
    let is_authenticated = token.is_some();
    let credential = token
        .as_deref()
        .map(|token| CredentialKind::classify(token).as_str().to_string());

    AuthStateResponse {
        is_authenticated,
        user_id: profile.metadata.get("user_id").cloned(),
        user: session_user_value(profile),
        profile_id: Some(profile.id.clone()),
        credential,
        expires_at: session_expires_at_from_profile(Some(profile)).map(|dt| dt.to_rfc3339()),
        issuing_backend: profile
            .metadata
            .get(SESSION_ISSUING_BACKEND_META)
            .and_then(|backend| normalize_session_backend(backend).ok()),
    }
}

pub fn session_token_from_profile(profile: Option<&AuthProfile>) -> Option<String> {
    // Mirror the `is_authenticated` check in `session_state_from_profile`
    // (trim + non-empty) so the two views of the same profile never
    // disagree — i.e. we never return `Some("   ")` while reporting
    // `is_authenticated = false`.
    profile
        .and_then(|entry| entry.token.as_deref())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "session_state_tests.rs"]
mod tests;
