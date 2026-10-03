//! `auth.set_credential` / `auth.clear_credential`: install or remove the
//! backend credential the core authenticates with.
//!
//! The core never obtains, validates, exchanges or refreshes a credential.
//! A host (the Tauri shell through `openhuman_tinyhumans::session`, the TUI, an embedder,
//! an operator's CLI) hands over a session JWT, a TinyHumans API key or the
//! offline local token together with whatever it already knows about the
//! user, and the core does the things only it can do with that: activate the
//! user's directory, persist the profile, rebind its process globals, start
//! the credential-gated services, open the scheduler gate and scope Sentry.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::Config;
use crate::core::Outcome;
use crate::security::credentials::jwt::decode_jwt_exp;
use crate::security::credentials::responses::AuthStateResponse;
use crate::security::credentials::session_support::{
    build_session_state, load_app_session_profile, local_session_user_id,
    session_token_from_profile, user_id_from_jwt_claims, CredentialKind, SESSION_EXPIRES_AT_META,
};
use crate::security::credentials::{
    api_key, identity, sentry_scope, AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};

use super::gated_services::{
    spawn_integrations_cache_warm, start_credential_gated_services, stop_credential_gated_services,
};
use super::user_scope::{
    activate_user_scope, deactivate_user_scope, rebind_after_credential_change, reload_config_or,
};

const LOG_PREFIX: &str = "[credentials][set-credential]";

/// Treat a session token as expired this many seconds before its real `exp`,
/// so an in-flight request cannot race the boundary into a backend 401. Same
/// skew `classify_session_token` applies at use time.
const EXPIRY_SKEW_SECS: i64 = 30;

/// Marks a stored user payload as not yet confirmed against the backend. A
/// host that accepts a JWT while the backend is unreachable stores this
/// placeholder and replaces it with the real `/auth/me` answer later — through
/// a second `set_credential` with the same token, which takes the refresh path.
pub const PENDING_BACKEND_VALIDATION_FIELD: &str = "pendingBackendValidation";

/// Serialises credential mutations so two hosts' callbacks cannot interleave
/// their activation / teardown side effects.
pub(crate) static CREDENTIAL_MUTATION_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

/// `auth.set_credential` parameters.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCredentialRequest {
    /// The secret: a session JWT, an API key, or the local offline token.
    pub token: String,
    /// `"session"`, `"api-key"` or `"local"`. Defaults to classifying `token`
    /// by shape; an API key must be named explicitly.
    #[serde(default)]
    pub kind: Option<String>,
    /// The backend user id. Optional when `user` carries one or the JWT has a
    /// subject claim; required otherwise for a session.
    #[serde(default, alias = "user_id")]
    pub user_id: Option<String>,
    /// The user payload (the host's `/auth/me` answer, or the local user).
    #[serde(default)]
    pub user: Option<Value>,
}

fn sanitize_user(user: Option<Value>) -> Option<Value> {
    match user {
        Some(Value::Object(map)) if map.is_empty() => None,
        Some(Value::Null) => None,
        other => other,
    }
}

fn user_id_from_payload(user: Option<&Value>) -> Option<String> {
    user.and_then(crate::security::credentials::jwt::user_id_from_profile_payload)
}

fn normalize_local_user(user: Value, local_user_id: &str) -> Value {
    let mut map = match user {
        Value::Object(map) => map,
        other => return other,
    };
    map.insert("id".to_string(), Value::String(local_user_id.to_string()));
    map.insert("_id".to_string(), Value::String(local_user_id.to_string()));
    Value::Object(map)
}

/// What `set_credential` resolved before touching any state.
struct Resolved {
    kind: CredentialKind,
    token: String,
    user_id: Option<String>,
    user: Option<Value>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn resolve(request: SetCredentialRequest) -> Result<Resolved, String> {
    let token = request.token.trim().to_string();
    if token.is_empty() {
        return Err("token is required".to_string());
    }
    let kind = match request
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        Some(raw) => CredentialKind::parse(raw).ok_or_else(|| {
            format!("unknown credential kind {raw:?}; expected session, api-key or local")
        })?,
        None => CredentialKind::classify(&token),
    };
    let explicit_user_id = request
        .user_id
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let user = sanitize_user(request.user);

    match kind {
        CredentialKind::ApiKey => Ok(Resolved {
            kind,
            token,
            user_id: None,
            user: None,
            expires_at: None,
        }),
        CredentialKind::Local => {
            let local_id = local_session_user_id();
            let user = user.ok_or_else(|| "local session requires a user payload".to_string())?;
            let user = normalize_local_user(user, &local_id);
            Ok(Resolved {
                kind,
                token,
                user_id: Some(local_id),
                user: Some(user),
                expires_at: None,
            })
        }
        CredentialKind::Session => {
            let expires_at = decode_jwt_exp(&token);
            if let Some(exp) = expires_at {
                if chrono::Utc::now() + chrono::Duration::seconds(EXPIRY_SKEW_SECS) >= exp {
                    return Err(format!(
                        "CREDENTIAL_EXPIRED: session token expired at {}; obtain a fresh one",
                        exp.to_rfc3339()
                    ));
                }
            }
            let user_id = explicit_user_id
                .or_else(|| user_id_from_payload(user.as_ref()))
                .or_else(|| user_id_from_jwt_claims(&token))
                .ok_or_else(|| {
                    "userId required: the token carries no subject claim and no user payload named one"
                        .to_string()
                })?;
            Ok(Resolved {
                kind,
                token,
                user_id: Some(user_id),
                user,
                expires_at,
            })
        }
    }
}

/// Install a backend credential. See the module docs for what the core does
/// with it; see [`SetCredentialRequest`] for what the host must supply.
///
/// Calling it again with the **same token and user** is a cheap refresh: only
/// the stored user payload and expiry are rewritten (no directory activation,
/// no purge, no service restart). A **different user** than the one stored is
/// signed out first so `active_user.toml` never points at one user while
/// another's profile exists.
pub async fn set_credential(
    config: &Config,
    request: SetCredentialRequest,
) -> Result<Outcome<AuthStateResponse>, String> {
    let resolved = resolve(request)?;
    let _mutation = CREDENTIAL_MUTATION_LOCK.lock().await;

    if resolved.kind == CredentialKind::ApiKey {
        let was_authenticated =
            crate::security::credentials::session_support::has_backend_credential(config);
        api_key::store_api_key(config, &resolved.token).map_err(|e| e.to_string())?;
        crate::integrations::composio::invalidate_connected_integrations_cache();
        // API-key-backed runs must not retain a previous session identity in
        // prompt composition or observability scope.
        identity::clear_current_user();
        sentry_scope::clear();
        if !was_authenticated {
            start_credential_gated_services(config).await;
        } else {
            spawn_integrations_cache_warm(config);
        }
        crate::cron::scheduler_gate::set_signed_out(false);
        tracing::info!(
            domain = "credentials",
            operation = "set_credential",
            "{LOG_PREFIX} api key stored"
        );
        let state = build_session_state(config)?;
        publish_credential_changed(CredentialKind::ApiKey.as_str());
        return Ok(Outcome::single_log(state, "api key stored"));
    }

    let user_id = resolved
        .user_id
        .clone()
        .expect("session and local credentials always resolve a user id");
    let mut logs = Vec::new();

    // Refresh vs. install vs. account switch, decided on the stored profile.
    let existing = load_app_session_profile(config)?;
    let existing_token = session_token_from_profile(existing.as_ref());
    let existing_user_id = existing
        .as_ref()
        .and_then(|p| p.metadata.get("user_id").cloned());
    let same_token = existing_token.as_deref() == Some(resolved.token.as_str());
    let same_user = existing_user_id.as_deref() == Some(user_id.as_str());
    let refresh = same_token && same_user;

    if existing_token.is_some() && !same_user {
        tracing::info!(
            domain = "credentials",
            operation = "set_credential",
            "{LOG_PREFIX} credential names a different user than the stored one; clearing the previous session first"
        );
        let cleared = clear_session_credential(config).await?;
        logs.extend(cleared.logs);
    }

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("user_id".to_string(), user_id.clone());
    if let Some(user) = &resolved.user {
        metadata.insert("user_json".to_string(), user.to_string());
    } else if let Some(previous) = existing
        .as_ref()
        .filter(|_| refresh)
        .and_then(|p| p.metadata.get("user_json").cloned())
    {
        // A refresh without a payload keeps the one already stored.
        metadata.insert("user_json".to_string(), previous);
    }
    if let Some(exp) = resolved.expires_at {
        // Recorded so `require_live_session_token` can reject an expired token
        // locally instead of firing a doomed backend 401 (#3297).
        metadata.insert(SESSION_EXPIRES_AT_META.to_string(), exp.to_rfc3339());
    }

    let effective_config = if refresh {
        logs.push("credential refreshed (same secret, same user)".to_string());
        config.clone()
    } else {
        logs.extend(activate_user_scope(&user_id)?);
        // Reload so auth-profiles.json, the encryption key and the workspace
        // resolve to the user-scoped location before the profile is written.
        let effective = reload_config_or(config).await?;
        if resolved.kind == CredentialKind::Local {
            match crate::config::ops::set_onboarding_completed(false).await {
                Ok(_) => {
                    logs.push("onboarding left incomplete for local session setup".to_string())
                }
                Err(error) => logs.push(format!(
                    "onboarding setup warning for local session: {error}"
                )),
            }
            logs.push("local session accepted without backend validation".to_string());
        }
        effective
    };

    let auth = AuthService::from_config(&effective_config);
    auth.store_provider_token(
        APP_SESSION_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        &resolved.token,
        metadata,
        true,
    )
    .map_err(|e| e.to_string())?;
    logs.push(format!("{} credential stored", resolved.kind.as_str()));

    if !refresh {
        crate::integrations::composio::invalidate_connected_integrations_cache();
        if let Err(error) =
            rebind_after_credential_change(&effective_config, "credential installed")
        {
            // A retry of the same credential takes the refresh fast-path, so
            // leaving its profile behind would let it skip this required bind.
            auth.remove_profile(APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
                .map_err(|rollback| {
                    format!("credential rebind failed: {error}; rollback failed: {rollback}")
                })?;
            return Err(error);
        }
        logs.push("process globals rebound after credential install".to_string());
        start_credential_gated_services(&effective_config).await;
        logs.push("credential-gated services started".to_string());
    }

    // Open the scheduler gate now that a live credential is in place; workers
    // sleeping in the paused poll loop resume at their next iteration.
    crate::cron::scheduler_gate::set_signed_out(false);
    // An API key wins over a session for every backend request. Keep the
    // process identity aligned with that effective credential.
    if api_key::has_api_key(&effective_config) {
        sentry_scope::clear();
        identity::clear_current_user();
    } else {
        sentry_scope::bind(&user_id);
        identity::set_current_user(resolved.user.clone().or_else(|| {
            existing
                .as_ref()
                .filter(|_| refresh)
                .and_then(|p| p.metadata.get("user_json").cloned())
                .and_then(|raw| serde_json::from_str(&raw).ok())
        }));
    }
    tracing::info!(
        domain = "credentials",
        operation = "set_credential",
        kind = resolved.kind.as_str(),
        refresh,
        "{LOG_PREFIX} credential installed"
    );

    let state = build_session_state(&effective_config)?;
    publish_credential_changed(resolved.kind.as_str());
    Ok(Outcome::new(state, logs))
}

/// Remove the stored credential of `kind` — or every credential when `None`.
/// A session (or local) removal runs the sign-out teardown; an API-key removal
/// only closes the scheduler gate when no other credential remains.
pub async fn clear_credential(
    config: &Config,
    kind: Option<CredentialKind>,
) -> Result<Outcome<Value>, String> {
    let _mutation = CREDENTIAL_MUTATION_LOCK.lock().await;
    let mut logs = Vec::new();
    let mut removed_session = false;
    let mut removed_api_key = false;

    // The API key is persisted beside `config`'s state dir, which is the
    // user-scoped directory while a session is active. Deactivating that
    // session moves every process global to the pre-login/signed-out
    // workspace, so a key left in place would be stranded under
    // `users/<id>` while the rest of the process reads `users/local` and
    // reports signed out (#6318). Snapshot it before teardown so it can
    // either be carried forward (a session/local-only clear) or removed
    // outright at its real location (`kind: None` clears everything, but
    // the API-key clear below runs against the post-teardown config and
    // would otherwise miss it).
    let source_api_key = if matches!(
        kind,
        None | Some(CredentialKind::Session) | Some(CredentialKind::Local)
    ) {
        api_key::get_api_key(config).map_err(|e| e.to_string())?
    } else {
        None
    };

    let mut effective_config = config.clone();
    let mut cleared_source_api_key = false;
    if matches!(
        kind,
        None | Some(CredentialKind::Session) | Some(CredentialKind::Local)
    ) {
        let outcome = clear_session_credential(config).await?;
        removed_session = outcome.value;
        logs.extend(outcome.logs);

        if removed_session {
            // Follow the same process globals the teardown just rebound to,
            // so the api-key checks below agree with `auth.get_state`.
            effective_config = reload_config_or(config).await?;
            if let Some(key) = source_api_key.as_deref() {
                if matches!(
                    kind,
                    Some(CredentialKind::Session) | Some(CredentialKind::Local)
                ) {
                    // A session/local-only clear preserves the key that was
                    // actually active (the source): carry it forward to the
                    // post-teardown workspace before the source copy is
                    // removed below, overwriting whatever the destination
                    // already held. This RPC is only removing the session —
                    // an unrelated, stale key sitting at the pre-login
                    // workspace must never silently outrank the key that was
                    // the effective credential a moment ago (#6318).
                    api_key::store_api_key(&effective_config, key).map_err(|e| e.to_string())?;
                    logs.push("api key carried forward to the signed-out workspace".to_string());
                }
                // Either the key was just moved to the post-teardown config
                // above, or `kind` is `None` and it must be removed outright.
                // `config` here is still the pre-teardown, user-scoped
                // reference, so this clears the *source* location — never
                // leave that copy behind, or a later login to the same
                // account (or an api-key clear against the wrong location)
                // would silently resurrect it (#6318).
                cleared_source_api_key =
                    api_key::clear_api_key(config).map_err(|e| e.to_string())?;
            }
        }
    }
    let config = &effective_config;

    if matches!(kind, None | Some(CredentialKind::ApiKey)) {
        removed_api_key =
            api_key::clear_api_key(config).map_err(|e| e.to_string())? || cleared_source_api_key;
        if removed_api_key {
            crate::integrations::composio::invalidate_connected_integrations_cache();
            logs.push("api key cleared".to_string());
        }
        if !crate::security::credentials::session_support::has_backend_credential(config) {
            crate::cron::scheduler_gate::set_signed_out(true);
        } else if removed_api_key {
            // Clearing the preferred API key exposes the surviving session.
            // Restore the session's process identity immediately.
            if let Some(profile) = load_app_session_profile(config)? {
                if let Some(user_id) = profile.metadata.get("user_id") {
                    sentry_scope::bind(user_id);
                }
                identity::set_current_user(
                    profile
                        .metadata
                        .get("user_json")
                        .and_then(|raw| serde_json::from_str(raw).ok()),
                );
            }
            spawn_integrations_cache_warm(config);
        }
    }

    // A session-only clear may leave an API key behind. The session teardown
    // deliberately stops the gated services and closes the scheduler gate, so
    // restore the surviving key's runtime after that teardown has completed.
    if removed_session && api_key::has_api_key(config) {
        start_credential_gated_services(config).await;
        crate::cron::scheduler_gate::set_signed_out(false);
        sentry_scope::clear();
        identity::clear_current_user();
        logs.push("credential-gated services restarted for api key".to_string());
    }

    if removed_session || removed_api_key {
        publish_credential_changed("cleared");
    }
    Ok(Outcome::new(
        json!({
            "removed": removed_session || removed_api_key,
            "removedSession": removed_session,
            "removedApiKey": removed_api_key,
        }),
        logs,
    ))
}

/// The session teardown: gate closed first so no worker fires at a backend
/// about to be invalidated, then profile removal, socket / workflow-bridge
/// teardown, user-dir deactivation, service stop, rebind to the signed-out
/// workspace, and Sentry / identity clear. Callers hold
/// [`CREDENTIAL_MUTATION_LOCK`].
async fn clear_session_credential(config: &Config) -> Result<Outcome<bool>, String> {
    let mut logs = Vec::new();
    crate::cron::scheduler_gate::set_signed_out(true);
    identity::clear_current_user();
    // Local/CLI model authentication can outlive an OpenHuman account. The old
    // account's listeners and workspace-bound channel state cannot.
    #[cfg(feature = "channels")]
    crate::channels::session::invalidate_channel_session();

    let removed = AuthService::from_config(config)
        .remove_profile(APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .map_err(|e| e.to_string())?;
    if removed {
        crate::integrations::composio::invalidate_connected_integrations_cache();
    }

    // The core process stays alive on sign-out. Tear down its authenticated
    // Socket.IO transport and the user-pinned workflow bridge so neither can
    // keep serving the signed-out account until a later reconnect.
    if let Some(manager) = crate::platform::socket::global_socket_manager() {
        if let Err(error) = manager.disconnect().await {
            tracing::warn!(%error, "{LOG_PREFIX} failed to disconnect backend socket on sign-out");
        }
    }

    deactivate_user_scope()?;
    stop_credential_gated_services(config).await;

    // Every process-global store must follow the now-active pre-login
    // workspace, or a signed-out caller keeps reading the previous account's
    // context until the process restarts.
    match crate::config::load_config_with_timeout().await {
        Ok(signed_out_config) => {
            logs.extend(rebind_after_credential_change(
                &signed_out_config,
                "credential cleared",
            )?);
            logs.push(format!(
                "process globals rebound to signed-out workspace {}",
                signed_out_config.workspace_dir.display()
            ));
        }
        Err(error) => return Err(format!("failed to resolve signed-out workspace: {error}")),
    }

    sentry_scope::clear();
    logs.push("session cleared".to_string());
    Ok(Outcome::new(removed, logs))
}

/// Historical entry point: install a session (or local) credential from a
/// bare token. Same as [`set_credential`] with the kind classified by shape.
pub async fn store_session(
    config: &Config,
    token: &str,
    user_id: Option<String>,
    user: Option<Value>,
) -> Result<Outcome<AuthStateResponse>, String> {
    set_credential(
        config,
        SetCredentialRequest {
            token: token.to_string(),
            kind: None,
            user_id,
            user,
        },
    )
    .await
}

/// Historical entry point: sign the session out. Same as
/// [`clear_credential`] for the session kind.
pub async fn clear_session(config: &Config) -> Result<Outcome<Value>, String> {
    clear_credential(config, Some(CredentialKind::Session)).await
}

/// Tell credential-derived caches (the search module's managed routes) to
/// refresh. Carries only the kind, never the credential.
fn publish_credential_changed(kind: &str) {
    tracing::debug!(kind, "{LOG_PREFIX} publishing CredentialChanged");
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::CredentialChanged {
        kind: kind.to_string(),
    });
}
