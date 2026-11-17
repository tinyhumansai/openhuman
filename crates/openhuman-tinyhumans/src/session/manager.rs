//! Host-facing orchestration: login, store, logout, current user and state
//! on top of a [`SessionClient`], a [`CurrentUserCache`] and a [`CoreLink`].
//!
//! The core is the persistent store; this manager keeps no copy of the
//! secret. It reads the stored token back through the link when it needs to
//! refresh `/auth/me`, and pushes credentials in through `auth.set_credential`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::session::cache::{CachedUser, CurrentUserCache};
use crate::session::client::{ClientHeaders, FetchMeError, SessionClient};
use crate::session::credential::{
    decode_jwt_exp, jwt_is_live, user_id_from_jwt_claims, user_id_from_profile_payload, Credential,
    CredentialKind,
};
use crate::session::identity;
use crate::session::link::{self, CoreAuthState, CoreLink};

const LOG_PREFIX: &str = "[session][manager]";

/// Marks a stored user payload as not yet confirmed against the backend —
/// set when a JWT is accepted while the backend is unreachable, cleared once
/// `/auth/me` confirms it.
pub const PENDING_BACKEND_VALIDATION_FIELD: &str = "pendingBackendValidation";

const REVALIDATION_INITIAL_DELAY: Duration = Duration::from_secs(5);
const REVALIDATION_MAX_DELAY: Duration = Duration::from_secs(60);

/// Why a login / store did not complete. The `Display` form carries a stable
/// `PREFIX:` a frontend can classify on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// The backend refused the credential; nothing was stored.
    #[error("REJECTED: {0}")]
    Rejected(String),
    /// The JWT's `exp` is already in the past; nothing was stored.
    #[error("EXPIRED: session token has already expired")]
    Expired,
    /// The backend could not be reached and the token cannot be accepted
    /// provisionally (no live `exp`).
    #[error("TRANSIENT: {0}")]
    Transient(String),
    /// Deferred acceptance needs a user id and the token carries none.
    #[error("USER_ID_UNAVAILABLE: backend unreachable and the token carries no subject claim")]
    UserIdUnavailable,
    /// Login-token exchange failed.
    #[error("CONSUME_FAILED: {0}")]
    ConsumeFailed(String),
    /// The backend base URL could not be resolved or the client not built.
    #[error("BACKEND: {0}")]
    Backend(String),
    /// The core refused or failed the RPC.
    #[error("CORE: {0}")]
    Core(String),
    /// The local session needs a user payload.
    #[error("INVALID: {0}")]
    Invalid(String),
}

/// Everything a UI needs to render the signed-in state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    #[serde(flatten)]
    pub core: CoreAuthState,
    pub current_user: Option<Value>,
    pub current_user_stale: bool,
    pub current_user_stale_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[allow(clippy::large_enum_variant)] // Keep the public event's state inline.
pub enum SessionEvent {
    /// The credential or the current user changed.
    Changed(SessionState),
    /// The backend rejected the stored credential; it has been cleared.
    Expired { source: String },
}

pub struct SessionManager<L: CoreLink> {
    link: Arc<L>,
    headers: ClientHeaders,
    client: Mutex<Option<Arc<SessionClient>>>,
    cache: CurrentUserCache,
    events: broadcast::Sender<SessionEvent>,
    revalidation: Mutex<Option<JoinHandle<()>>>,
    /// Serialises login / logout so two callbacks cannot interleave their
    /// core-side side effects.
    mutation: tokio::sync::Mutex<()>,
}

impl<L: CoreLink> SessionManager<L> {
    pub fn new(link: Arc<L>, headers: ClientHeaders) -> Arc<Self> {
        let (events, _) = broadcast::channel(32);
        Arc::new(Self {
            link,
            headers,
            client: Mutex::new(None),
            cache: CurrentUserCache::new(),
            events,
            revalidation: Mutex::new(None),
            mutation: tokio::sync::Mutex::new(()),
        })
    }

    pub fn link(&self) -> &Arc<L> {
        &self.link
    }

    pub fn cache(&self) -> &CurrentUserCache {
        &self.cache
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    fn emit(&self, event: SessionEvent) {
        let _ = self.events.send(event);
    }

    /// The client for the backend the core is configured against. Rebuilt
    /// when the resolved base URL changes (environment / gateway switch).
    pub async fn client(&self) -> Result<Arc<SessionClient>, SessionError> {
        let base = link::resolve_backend_url(self.link.as_ref())
            .await
            .map_err(SessionError::Backend)?;
        let normalized = base.trim().trim_end_matches('/');
        {
            let guard = self.client.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(client) = guard.as_ref() {
                if client.base_url() == normalized {
                    return Ok(Arc::clone(client));
                }
            }
        }
        let client = Arc::new(
            SessionClient::new(&base, &self.headers)
                .map_err(|e| SessionError::Backend(e.to_string()))?,
        );
        log::debug!("{LOG_PREFIX} session client bound to {}", client.base_url());
        *self.client.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::clone(&client));
        Ok(client)
    }

    /// Stop a pending-session revalidation loop, if one is running.
    pub fn cancel_revalidation(&self) {
        if let Some(handle) = self
            .revalidation
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            handle.abort();
        }
    }

    async fn push(
        &self,
        credential: &Credential,
        user_id: Option<&str>,
        user: Option<&Value>,
        issuing_backend: Option<&str>,
    ) -> Result<CoreAuthState, SessionError> {
        let state = link::push_bound_credential(
            self.link.as_ref(),
            credential,
            user_id,
            user,
            issuing_backend,
        )
        .await
        .map_err(SessionError::Core)?;
        identity::set_user_id(state.user_id.clone());
        Ok(state)
    }

    async fn changed(&self) -> Result<SessionState, SessionError> {
        let state = self.state().await?;
        self.emit(SessionEvent::Changed(state.clone()));
        Ok(state)
    }

    /// Exchange a one-time login token for a JWT and store it.
    pub async fn login_with_token(
        self: &Arc<Self>,
        login_token: &str,
    ) -> Result<SessionState, SessionError> {
        let client = self.client().await?;
        self.login_with_client(login_token, client).await
    }

    /// Complete a browser callback only against its original backend.
    /// The same captured client exchanges and validates the credential.
    pub async fn login_with_token_for_backend(
        self: &Arc<Self>,
        login_token: &str,
        expected_backend: &str,
    ) -> Result<SessionState, SessionError> {
        let client = self.client_for_expected_backend(expected_backend).await?;
        self.login_with_client(login_token, client).await
    }

    async fn client_for_expected_backend(
        &self,
        expected_backend: &str,
    ) -> Result<Arc<SessionClient>, SessionError> {
        let expected = crate::session::client::normalize_backend_origin(expected_backend)
            .map_err(|_| SessionError::Backend("invalid expected login backend".into()))?;
        let client = self.client().await?;
        if client.base_url() != expected {
            return Err(SessionError::Backend(
                "SESSION_BACKEND_MISMATCH: the backend changed during sign-in; start sign-in again"
                    .into(),
            ));
        }
        Ok(client)
    }

    async fn login_with_client(
        self: &Arc<Self>,
        login_token: &str,
        client: Arc<SessionClient>,
    ) -> Result<SessionState, SessionError> {
        let jwt = client
            .consume_login_token(login_token)
            .await
            .map_err(|e| SessionError::ConsumeFailed(e.to_string()))?;
        self.store_session_token_with_client(&jwt, None, Some(client))
            .await
    }

    /// Store a session JWT (or a local offline token) after validating it.
    ///
    /// * local token → stored as-is with `user` (required);
    /// * JWT with a dead `exp` → [`SessionError::Expired`];
    /// * JWT confirmed by `/auth/me` → stored with the backend's user;
    /// * JWT refused by the backend → [`SessionError::Rejected`], nothing stored;
    /// * backend unreachable and JWT has a live `exp` and a subject claim →
    ///   stored provisionally with `pendingBackendValidation: true` and
    ///   revalidated in the background.
    pub async fn store_session_token(
        self: &Arc<Self>,
        token: &str,
        user: Option<Value>,
    ) -> Result<SessionState, SessionError> {
        self.store_session_token_with_client(token, user, None)
            .await
    }

    /// Validate a browser callback against its expected issuer before sending
    /// any credential. Configuration changes cannot change the captured client.
    pub async fn store_session_token_for_backend(
        self: &Arc<Self>,
        token: &str,
        user: Option<Value>,
        expected_backend: &str,
    ) -> Result<SessionState, SessionError> {
        let client = self.client_for_expected_backend(expected_backend).await?;
        self.store_session_token_with_client(token, user, Some(client))
            .await
    }

    async fn store_session_token_with_client(
        self: &Arc<Self>,
        token: &str,
        user: Option<Value>,
        client: Option<Arc<SessionClient>>,
    ) -> Result<SessionState, SessionError> {
        let guard = self.mutation.lock().await;
        let credential = Credential::classify(token);
        if credential.secret.is_empty() {
            return Err(SessionError::Invalid("token is required".to_string()));
        }

        if credential.is_local() {
            let user = user.filter(|u| u.as_object().is_some_and(|m| !m.is_empty()));
            if user.is_none() {
                return Err(SessionError::Invalid(
                    "local session requires a user payload".to_string(),
                ));
            }
            self.cancel_revalidation();
            self.push(&credential, None, user.as_ref(), None).await?;
            self.cache.forget();
            drop(guard);
            return self.changed().await;
        }

        let now = chrono::Utc::now();
        if decode_jwt_exp(&credential.secret).is_some()
            && jwt_is_live(&credential.secret, now).is_none()
        {
            return Err(SessionError::Expired);
        }

        let client = match client {
            Some(client) => client,
            None => self.client().await?,
        };
        match client.validate_for_store(&credential).await {
            Ok(me) => {
                let user_id = user_id_from_profile_payload(&me)
                    .or_else(|| user.as_ref().and_then(user_id_from_profile_payload))
                    .or_else(|| user_id_from_jwt_claims(&credential.secret));
                log::info!(
                    "{LOG_PREFIX} session JWT verified via GET /auth/me on {}",
                    client.base_url()
                );
                // Hand the new credential to the core before cancelling the
                // old one's revalidation loop: if `push` fails (or the core
                // restarts mid-call), the prior pending credential must keep
                // its background revalidation running rather than being left
                // provisional with nothing to confirm it, possibly
                // indefinitely for an idle TUI or host (#6318 review
                // follow-up).
                self.push(
                    &credential,
                    user_id.as_deref(),
                    Some(&me),
                    Some(client.base_url()),
                )
                .await?;
                self.cancel_revalidation();
                self.cache.seed(&client, &credential, me);
                drop(guard);
                self.changed().await
            }
            Err(FetchMeError::Rejected(reason)) => {
                log::warn!(
                    "{LOG_PREFIX} GET /auth/me rejected the session token; not stored: {reason}"
                );
                Err(SessionError::Rejected(reason))
            }
            Err(error) => {
                let reason = error.message().to_string();
                if jwt_is_live(&credential.secret, now).is_none() {
                    log::warn!("{LOG_PREFIX} backend unreachable and JWT has no live exp; not stored: {reason}");
                    return Err(SessionError::Transient(reason));
                }
                let user_id = user
                    .as_ref()
                    .and_then(user_id_from_profile_payload)
                    .or_else(|| user_id_from_jwt_claims(&credential.secret))
                    .ok_or(SessionError::UserIdUnavailable)?;
                log::warn!(
                    "{LOG_PREFIX} backend unreachable ({reason}); storing pending session for user_id={user_id} and revalidating in the background"
                );
                let pending = json!({ PENDING_BACKEND_VALIDATION_FIELD: true });
                self.cancel_revalidation();
                self.push(
                    &credential,
                    Some(&user_id),
                    Some(&pending),
                    Some(client.base_url()),
                )
                .await?;
                self.cache.forget();
                self.spawn_revalidation(credential, client.base_url().to_string());
                drop(guard);
                self.changed().await
            }
        }
    }

    /// Store a TinyHumans API key. No user identity, no backend round trip.
    pub async fn store_api_key(self: &Arc<Self>, key: &str) -> Result<SessionState, SessionError> {
        let guard = self.mutation.lock().await;
        let credential = Credential::api_key(key);
        if credential.secret.is_empty() {
            return Err(SessionError::Invalid("api key is required".to_string()));
        }
        self.cancel_revalidation();
        self.push(&credential, None, None, None).await?;
        drop(guard);
        self.changed().await
    }

    fn spawn_revalidation(self: &Arc<Self>, credential: Credential, issuing_backend: String) {
        let manager = Arc::clone(self);
        let handle = tokio::spawn(async move {
            let mut delay = REVALIDATION_INITIAL_DELAY;
            loop {
                tokio::time::sleep(delay).await;
                let Ok(client) = manager.client().await else {
                    delay = (delay * 2).min(REVALIDATION_MAX_DELAY);
                    continue;
                };
                if client.base_url() != issuing_backend {
                    // A pending credential must never be tested against a
                    // deployment other than the one it was submitted to.
                    return;
                }
                // The core is the store of record; if it no longer holds this
                // token (logout, or a newer login), this loop is stale.
                match link::core_session_token(manager.link.as_ref()).await {
                    Ok(Some(stored)) if stored == credential.secret => {}
                    _ => {
                        log::debug!("{LOG_PREFIX} pending-session revalidation stopped; token no longer stored");
                        return;
                    }
                }
                let verdict = client.fetch_me(&credential).await;
                // Re-check under the mutation lock: a logout or a newer login
                // may have landed while `/auth/me` was in flight, and neither
                // a confirmation nor a rejection of the old token may touch
                // the credential the core holds now.
                let guard = manager.mutation.lock().await;
                if !manager.core_still_holds(&credential.secret).await {
                    log::debug!("{LOG_PREFIX} pending-session revalidation stopped; token replaced during GET /auth/me");
                    return;
                }
                match verdict {
                    Ok(me) => {
                        let user_id = user_id_from_profile_payload(&me)
                            .or_else(|| user_id_from_jwt_claims(&credential.secret));
                        log::info!("{LOG_PREFIX} pending session confirmed via GET /auth/me");
                        // The backend confirmed the token, but the core
                        // handoff is itself fallible (RPC failure, a core
                        // restart mid-call). Only a successful `push` retires
                        // this loop — on failure, keep retrying with backoff
                        // instead of leaving `pendingBackendValidation` stuck
                        // forever despite a confirmed backend answer (#6318
                        // review follow-up).
                        match manager
                            .push(
                                &credential,
                                user_id.as_deref(),
                                Some(&me),
                                Some(client.base_url()),
                            )
                            .await
                        {
                            Ok(_) => {
                                manager.cache.seed(&client, &credential, me);
                                drop(guard);
                                if let Ok(state) = manager.state().await {
                                    manager.emit(SessionEvent::Changed(state));
                                }
                                return;
                            }
                            Err(e) => {
                                drop(guard);
                                log::warn!(
                                    "{LOG_PREFIX} failed to store revalidated session ({e}); retrying in {}s",
                                    delay.as_secs()
                                );
                                delay = (delay * 2).min(REVALIDATION_MAX_DELAY);
                            }
                        }
                    }
                    Err(FetchMeError::Rejected(reason)) => {
                        log::warn!(
                            "{LOG_PREFIX} pending session rejected by backend; clearing: {reason}"
                        );
                        if manager
                            .clear_session_credential("pending-revalidation")
                            .await
                        {
                            return;
                        }
                        drop(guard);
                        delay = (delay * 2).min(REVALIDATION_MAX_DELAY);
                    }
                    Err(error) => {
                        drop(guard);
                        log::debug!(
                            "{LOG_PREFIX} pending-session revalidation still failing ({}); retrying in {}s",
                            error.message(),
                            delay.as_secs()
                        );
                        delay = (delay * 2).min(REVALIDATION_MAX_DELAY);
                    }
                }
            }
        });
        *self.revalidation.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);
    }

    async fn clear_session_credential(&self, source: &str) -> bool {
        if let Err(e) =
            link::clear_credential(self.link.as_ref(), Some(CredentialKind::Session)).await
        {
            log::warn!("{LOG_PREFIX} failed to clear rejected session credential: {e}");
            return false;
        }
        self.cache.forget();
        identity::clear();
        self.emit(SessionEvent::Expired {
            source: source.to_string(),
        });
        // Re-read the core after clearing the session. An API key profile may
        // still be active and must not be reported as a signed-out state.
        match self.core_state().await {
            Ok(core) => self.emit(SessionEvent::Changed(SessionState {
                current_user: core.user.clone(),
                current_user_stale: false,
                current_user_stale_seconds: None,
                core,
            })),
            Err(error) => log::warn!(
                "{LOG_PREFIX} cleared rejected session but could not read the post-clear state: {error}"
            ),
        }
        true
    }

    /// Whether the core still holds `secret` as its session token.
    ///
    /// Refreshes read the token, then await the network, then persist what
    /// they learned; a login or logout can land in between. Callers take
    /// `mutation` and re-check with this before acting on a result, so a
    /// verdict about a superseded token never touches the current one.
    async fn core_still_holds(&self, secret: &str) -> bool {
        matches!(
            link::core_session_token(self.link.as_ref()).await,
            Ok(Some(stored)) if stored == secret
        )
    }

    /// Sign out: clear the session (or local) credential in the core and
    /// forget the current user.
    pub async fn logout(&self) -> Result<SessionState, SessionError> {
        {
            let _guard = self.mutation.lock().await;
            self.cancel_revalidation();
            link::clear_credential(self.link.as_ref(), Some(CredentialKind::Session))
                .await
                .map_err(SessionError::Core)?;
            self.cache.forget();
            identity::clear();
        }
        self.changed().await
    }

    /// Remove a stored API key.
    pub async fn clear_api_key(&self) -> Result<SessionState, SessionError> {
        {
            let _guard = self.mutation.lock().await;
            link::clear_credential(self.link.as_ref(), Some(CredentialKind::ApiKey))
                .await
                .map_err(SessionError::Core)?;
        }
        self.changed().await
    }

    /// The core's own view of the credential it holds.
    pub async fn core_state(&self) -> Result<CoreAuthState, SessionError> {
        let state = link::core_auth_state(self.link.as_ref())
            .await
            .map_err(SessionError::Core)?;
        identity::set_user_id(state.user_id.clone());
        Ok(state)
    }

    /// The current user: cached `/auth/me` for a session credential
    /// (refreshed per the cache policy, or unconditionally with `force`),
    /// the stored payload for a local session or an API key.
    ///
    /// A backend rejection clears the credential and reports
    /// [`SessionError::Rejected`]; an availability failure serves the stored
    /// user marked stale.
    pub async fn current_user(&self, force: bool) -> Result<CachedUser, SessionError> {
        let core = self.core_state().await?;
        self.current_user_for(&core, force).await
    }

    async fn current_user_for(
        &self,
        core: &CoreAuthState,
        force: bool,
    ) -> Result<CachedUser, SessionError> {
        if !core.is_authenticated {
            return Ok(CachedUser {
                user: None,
                stale: false,
                stale_seconds: None,
            });
        }
        let stored = || CachedUser {
            user: core.user.clone(),
            stale: false,
            stale_seconds: None,
        };
        if core.kind() != Some(CredentialKind::Session) {
            return Ok(stored());
        }
        let client = self.client().await?;
        if core
            .issuing_backend
            .as_deref()
            .is_some_and(|issuing| issuing != client.base_url())
        {
            return Err(SessionError::Backend("SESSION_BACKEND_MISMATCH: stored session belongs to a different backend; restore its backend or sign in again".into()));
        }
        let Some(secret) = link::core_session_token(self.link.as_ref())
            .await
            .map_err(SessionError::Core)?
        else {
            return Ok(stored());
        };
        let credential = Credential::session(secret);
        match self.cache.get_or_refresh(&client, &credential, force).await {
            Ok(cached) => {
                if cached.user.is_some() && user_is_pending(core.user.as_ref()) {
                    // The shell (or a previous process) accepted this token
                    // while the backend was down; the confirmation just came in.
                    let _guard = self.mutation.lock().await;
                    if self.core_still_holds(&credential.secret).await {
                        let user_id = cached.user.as_ref().and_then(user_id_from_profile_payload);
                        if let Err(e) = self
                            .push(
                                &credential,
                                user_id.as_deref(),
                                cached.user.as_ref(),
                                Some(client.base_url()),
                            )
                            .await
                        {
                            log::warn!(
                                "{LOG_PREFIX} failed to store confirmed pending session: {e}"
                            );
                        }
                    } else {
                        log::debug!(
                            "{LOG_PREFIX} pending session confirmed after the token was replaced or cleared; not stored"
                        );
                    }
                }
                Ok(cached)
            }
            Err(FetchMeError::Rejected(reason)) => {
                let _guard = self.mutation.lock().await;
                if !self.core_still_holds(&credential.secret).await {
                    // The rejection is for a token the core no longer holds
                    // (a newer login or a logout raced this refresh); the
                    // current credential is untouched and the caller polls
                    // again.
                    log::debug!(
                        "{LOG_PREFIX} GET /auth/me rejected a superseded session token; ignoring: {reason}"
                    );
                    let mut fallback = stored();
                    fallback.stale = true;
                    return Ok(fallback);
                }
                log::warn!(
                    "{LOG_PREFIX} GET /auth/me rejected the stored session; signing out: {reason}"
                );
                self.clear_session_credential("auth/me").await;
                Err(SessionError::Rejected(reason))
            }
            Err(FetchMeError::Superseded) => {
                log::debug!(
                    "{LOG_PREFIX} current-user refresh was superseded; retrying with the current core credential"
                );
                Box::pin(self.current_user(force)).await
            }
            Err(error) => {
                log::debug!("{LOG_PREFIX} serving stored user; refresh failed: {error}");
                let mut fallback = stored();
                fallback.stale = true;
                Ok(fallback)
            }
        }
    }

    /// Core state plus the current user, in one shape.
    pub async fn state(&self) -> Result<SessionState, SessionError> {
        loop {
            let core = self.core_state().await?;
            let current = match self.current_user_for(&core, false).await {
                Ok(current) => current,
                Err(SessionError::Rejected(_)) => return Ok(SessionState::default()),
                Err(error) => return Err(error),
            };
            // `current_user_for` retries with a new credential when a refresh
            // is superseded. Do not combine that retried user with the core
            // snapshot from before the credential change.
            if self.core_state().await? != core {
                log::debug!(
                    "{LOG_PREFIX} session state changed while reading current user; retrying"
                );
                continue;
            }
            return Ok(SessionState {
                current_user: current.user.or_else(|| core.user.clone()),
                current_user_stale: current.stale,
                current_user_stale_seconds: current.stale_seconds,
                core,
            });
        }
    }
}

fn user_is_pending(user: Option<&Value>) -> bool {
    user.and_then(Value::as_object)
        .and_then(|m| m.get(PENDING_BACKEND_VALIDATION_FIELD))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
