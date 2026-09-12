//! Session/auth helpers used by RPC and [`crate::core_server::helpers`].

use crate::openhuman::config::Config;

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
    profile: &crate::openhuman::security::credentials::profiles::AuthProfile,
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
    profile: &crate::openhuman::security::credentials::profiles::AuthProfile,
) -> Option<serde_json::Value> {
    profile
        .metadata
        .get("user_json")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
}

pub fn build_session_state(config: &Config) -> Result<AuthStateResponse, String> {
    let profile = load_app_session_profile(config)?;
    Ok(session_state_from_profile(profile.as_ref()))
}

pub fn get_session_token(config: &Config) -> Result<Option<String>, String> {
    let profile = load_app_session_profile(config)?;
    Ok(session_token_from_profile(profile.as_ref()))
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
pub(crate) enum SessionTokenCheck {
    /// No token stored (signed out / never authed).
    Absent,
    /// Token present but its recorded `exp` is in the past (− skew).
    Expired,
    /// Token present and within its validity window (or no recorded expiry).
    Live(String),
}

pub(crate) fn classify_session_token(
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
///   the TAURI-RUST-8WY (`/teams/me/usage`) / 8WZ (`/payments/stripe/currentPlan`)
///   flood at its source instead of demoting it after the fact.
/// - token with no recorded expiry (local offline session / `exp`-less JWT) →
///   presence-only check; the `flatten_authed_error` 401 net still covers a
///   server-side revocation that precedes the recorded `exp`.
pub fn require_live_session_token(config: &Config) -> Result<String, String> {
    let profile = load_app_session_profile(config)?;
    match classify_session_token(profile.as_ref(), chrono::Utc::now()) {
        SessionTokenCheck::Live(token) => Ok(token),
        SessionTokenCheck::Absent => {
            Err("no backend session token; run auth_store_session first".to_string())
        }
        SessionTokenCheck::Expired => {
            publish_local_session_expiry("require_live_session_token");
            Err(
                "SESSION_EXPIRED: backend session token expired locally — re-authentication required"
                    .to_string(),
            )
        }
    }
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
pub fn publish_local_session_expiry(operation: &'static str) {
    // Dedupe the publish via the scheduler gate so N parallel authed
    // callers in one tick don't emit N SessionExpired events.
    if crate::openhuman::cron::scheduler_gate::is_signed_out() {
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
    });
}

/// Load the `app-session` profile once. Callers that need both the
/// session-state view (`session_state_from_profile`) AND the raw token
/// (`session_token_from_profile`) should call this once and pass the
/// result to both helpers — every load takes the auth-profile store
/// lock, and on Windows the `app_state_snapshot` hot path used to take
/// it twice per call which materially increased lock contention
/// (Sentry: "Timed out waiting for auth profile lock").
pub fn load_app_session_profile(config: &Config) -> Result<Option<AuthProfile>, String> {
    let auth_service = AuthService::from_config(config);
    auth_service
        .get_profile(APP_SESSION_PROVIDER, None)
        .map_err(|e| e.to_string())
}

pub fn session_state_from_profile(profile: Option<&AuthProfile>) -> AuthStateResponse {
    let Some(profile) = profile else {
        return AuthStateResponse {
            is_authenticated: false,
            user_id: None,
            user: None,
            profile_id: None,
        };
    };

    let is_authenticated = profile
        .token
        .as_ref()
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false);

    AuthStateResponse {
        is_authenticated,
        user_id: profile.metadata.get("user_id").cloned(),
        user: session_user_value(profile),
        profile_id: Some(profile.id.clone()),
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
#[path = "session_support_tests.rs"]
mod tests;
