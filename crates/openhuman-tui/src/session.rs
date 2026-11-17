//! The TUI's session owner: `openhuman-tinyhumans` over an in-process
//! [`CoreRuntime`]. Login-token exchange and `/auth/me` happen here, in the
//! TUI process; the core only receives the resulting credential through
//! `auth.set_credential`.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use openhuman_rpc::embed::CoreRuntime;
use openhuman_rpc::tinyhumans::{ClientHeaders, CoreLink, SessionManager};

#[path = "browser_login.rs"]
mod browser_login;
pub use browser_login::{open_browser, BrowserLogin, LoginCancellation, LoginProvider};

/// Restore the persisted account and refresh its backend profile. The manager
/// clears a rejected session and retains cached identity during an outage.
pub async fn refresh_session<L: CoreLink>(
    manager: &Arc<SessionManager<L>>,
) -> Result<openhuman_rpc::tinyhumans::SessionState, String> {
    match manager.current_user(true).await {
        Ok(_) | Err(openhuman_rpc::tinyhumans::SessionError::Rejected(_)) => {}
        Err(error) => {
            return Err(safe_session_error(
                error,
                "Could not refresh the signed-in account.",
            ))
        }
    }
    manager
        .state()
        .await
        .map_err(|error| safe_session_error(error, "Could not read the signed-in account."))
}

fn safe_session_error(error: openhuman_rpc::tinyhumans::SessionError, fallback: &str) -> String {
    match error {
        openhuman_rpc::tinyhumans::SessionError::Backend(message)
        | openhuman_rpc::tinyhumans::SessionError::Core(message)
            if message.contains("SESSION_BACKEND_MISMATCH") =>
                "SESSION_BACKEND_MISMATCH: This session belongs to another backend. Restore that backend or start sign-in again.".into(),
        _ => fallback.to_string(),
    }
}

/// `CoreLink` over `CoreRuntime::invoke` — no HTTP, no bearer.
pub struct InProcessLink(pub Arc<CoreRuntime>);

#[async_trait]
impl CoreLink for InProcessLink {
    async fn invoke(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.0.invoke(method, params).await
    }
}

static MANAGER: OnceLock<Arc<SessionManager<InProcessLink>>> = OnceLock::new();

/// The one session manager for this TUI process, bound to `runtime` on first
/// use. A TUI process hosts exactly one runtime, so later callers get the
/// same manager regardless of the handle they pass.
pub fn session_manager(runtime: &Arc<CoreRuntime>) -> Arc<SessionManager<InProcessLink>> {
    Arc::clone(MANAGER.get_or_init(|| {
        let headers = ClientHeaders::new(openhuman_rpc::tinyhumans::product_identity().as_str())
            .with_core_version(env!("CARGO_PKG_VERSION"));
        SessionManager::new(Arc::new(InProcessLink(Arc::clone(runtime))), headers)
    }))
}
