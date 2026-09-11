//! Auth sub-facade — the session a turn runs under.
//!
//! # When an embedder needs this
//!
//! [`HostKind::Library`](crate::core::types::HostKind::Library) accepts a
//! caller-supplied provider without an OpenHuman app login. An embedder needs a
//! real session only when it also calls authenticated TinyHumans backend
//! services, or when it deliberately selects a desktop/standalone host mode
//! whose custom-provider policy still requires an active session.
//!
//! Before this existed, every embedder reached for `Core::raw()` and hand-wrote
//! `openhuman.auth_store_session` — which is how an unrelated host ends up
//! owning a copy of the `{result, logs}` envelope heuristic and a private
//! version of the auth state struct.
//!
//! # Two kinds of session
//!
//! [`Session::backend`] is a real JWT: it is validated against `GET /auth/me`
//! before anything is persisted, and a failure means nothing is stored.
//! [`Session::local`] is the compatibility/offline form — a token ending in
//! `.local`, carrying its own user payload, which the core recognizes and does
//! not try to validate. It authorizes nothing at the backend and is not needed
//! by the default library harness.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::call::call;
use super::error::CoreError;
use crate::core::runtime::CoreRuntime;

/// A session to install into the core's credential store.
#[derive(Debug, Clone)]
pub struct Session {
    token: String,
    user: Option<serde_json::Value>,
}

impl Session {
    /// A real backend session JWT, validated against `GET /auth/me` when stored.
    pub fn backend(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            user: None,
        }
    }

    /// An offline session for a host that brings its own provider credentials.
    ///
    /// The token must have the `.local` third segment the core recognizes —
    /// this constructor builds one, so a caller cannot get the shape subtly
    /// wrong and be told only that validation failed.
    ///
    /// Grants nothing at the backend. Managed inference, billing and team calls
    /// all still fail without a real session. The default library harness does
    /// not need this to use caller-supplied inference.
    pub fn local(user_id: impl Into<String>) -> Self {
        let user_id = user_id.into();
        Self {
            // The core detects a local session purely by the third segment
            // being `local`; the first two are opaque to it.
            token: "embedded.harness.local".to_string(),
            user: Some(serde_json::json!({
                "id": user_id,
                "email": "local@openhuman.local",
            })),
        }
    }

    /// Attach or override the user payload.
    ///
    /// Required for a local session (the core refuses one without it) and
    /// ignored for a backend session, which takes its user from `/auth/me`.
    pub fn user(mut self, user: serde_json::Value) -> Self {
        self.user = Some(user);
        self
    }

    /// Whether this is the offline form.
    pub fn is_local(&self) -> bool {
        crate::openhuman::security::credentials::session_support::is_local_session_token(
            &self.token,
        )
    }
}

/// Current auth state, narrowed to what a host actually branches on.
///
/// Declared here rather than re-exporting the domain's `AuthStateResponse` so
/// the facade's contract is explicit; the extra fields that response carries are
/// desktop-UI concerns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthState {
    /// Whether a session is stored and considered live.
    #[serde(default, rename = "isAuthenticated")]
    pub is_authenticated: bool,
    /// The signed-in user id, when there is one.
    #[serde(default, rename = "userId")]
    pub user_id: Option<String>,
}

/// Typed access to the session store.
///
/// Obtained from [`Core::auth`](super::Core::auth); never constructed directly.
pub struct Auth<'a>(pub(super) &'a Arc<CoreRuntime>);

impl Auth<'_> {
    /// Store a session, replacing any existing one.
    ///
    /// # Errors
    ///
    /// [`CoreError::Rpc`] when a backend session fails `GET /auth/me` — nothing
    /// is persisted in that case, deliberately, so a caller cannot end up
    /// believing it is signed in when it is not.
    pub async fn store(&self, session: Session) -> Result<(), CoreError> {
        log::debug!("[embed][auth] storing session local={}", session.is_local());
        let _: serde_json::Value = call(
            self.0,
            "openhuman.auth_store_session",
            serde_json::json!({
                "token": session.token,
                "user": session.user,
            }),
        )
        .await?;
        Ok(())
    }

    /// Read the current auth state. Does not make a network call.
    pub async fn state(&self) -> Result<AuthState, CoreError> {
        call(self.0, "openhuman.auth_get_state", serde_json::json!({})).await
    }

    /// The stored session token, if there is one.
    ///
    /// `Ok(None)` is the signed-out state, not a failure — a host polling this
    /// to decide whether to show a login prompt must be able to tell "nobody is
    /// signed in" from "the read failed".
    ///
    /// A host needs the token itself, not just [`state`](Self::state), whenever
    /// it authenticates its *own* calls with the same session — the core and
    /// its embedder talk to one deployment, so reading it here rather than from
    /// a file on the side keeps one credential in one place.
    pub async fn token(&self) -> Result<Option<String>, CoreError> {
        #[derive(Deserialize)]
        struct TokenPayload {
            #[serde(default)]
            token: Option<String>,
        }
        let payload: TokenPayload = call(
            self.0,
            "openhuman.auth_get_session_token",
            serde_json::json!({}),
        )
        .await?;
        // A stored-but-blank token is the signed-out state spelled differently;
        // handing one back would have callers authenticate with an empty bearer.
        Ok(payload.token.filter(|token| !token.trim().is_empty()))
    }

    /// Remove the stored session.
    pub async fn clear(&self) -> Result<(), CoreError> {
        let _: serde_json::Value = call(
            self.0,
            "openhuman.auth_clear_session",
            serde_json::json!({}),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
