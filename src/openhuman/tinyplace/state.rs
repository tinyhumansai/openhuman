//! Shared client state for the tiny.place domain.
//!
//! [`TinyPlaceState`] holds a lazily-initialised [`tinyplace::TinyPlaceClient`].
//! The client cannot be built at startup because the signer seed requires an
//! async decrypt of the wallet's encrypted mnemonic and the wallet may be
//! locked at launch time. We build it once on first access and cache it.
//!
//! The state is stored in a process-global `OnceLock` (see [`global_state`])
//! because controller handlers are `fn(Map<String,Value>) -> ControllerFuture`
//! with no injected state argument.

use std::sync::Arc;

use tokio::sync::OnceCell;

use tinyplace::{LocalSigner, TinyPlaceClient, TinyPlaceClientOptions};

const LOG_PREFIX: &str = "[tinyplace]";

/// Production tiny.place relay/API host — the default outside a staging build.
const TINYPLACE_PROD_BASE_URL: &str = "https://api.tiny.place";
/// Staging tiny.place relay/API host — the default when the OpenHuman app env
/// is `staging` and no explicit `TINYPLACE_API_BASE_URL` is set.
const TINYPLACE_STAGING_BASE_URL: &str = "https://staging-api.tiny.place";

/// Shared tiny.place state: lazy-built client keyed to one base URL.
pub(crate) struct TinyPlaceState {
    /// Lazily initialised on first [`TinyPlaceState::client`] call.
    client: OnceCell<TinyPlaceClient>,
    /// Backend base URL (from `TINYPLACE_API_BASE_URL`, else app-env default).
    pub(crate) base_url: String,
}

impl TinyPlaceState {
    /// Build from the environment.
    ///
    /// Base URL precedence: an explicit `TINYPLACE_API_BASE_URL` always wins;
    /// otherwise the default follows the OpenHuman app environment so a staging
    /// build talks to staging tiny.place and a production build talks to prod
    /// (previously the default was hardcoded to prod regardless of app env,
    /// which silently 404'd a staging instance against prod tiny.place).
    pub(crate) fn from_env() -> Self {
        let explicit = std::env::var("TINYPLACE_API_BASE_URL").ok();
        let app_env = crate::api::config::app_env_from_env();
        let base_url = resolve_base_url(explicit.as_deref(), app_env.as_deref());
        log::debug!(
            "{LOG_PREFIX} state created base_url={base_url} app_env={}",
            app_env.as_deref().unwrap_or("<unset>")
        );
        Self {
            client: OnceCell::new(),
            base_url,
        }
    }

    /// Return (or lazily build) the shared [`TinyPlaceClient`].
    ///
    /// On first call: derives the signer seed from the wallet, constructs the
    /// client, and caches it.  Subsequent calls return the cached instance.
    ///
    /// Returns `Err` if the wallet is locked/unconfigured or the seed derivation
    /// fails — the renderer should surface an "unlock wallet" prompt.
    pub(crate) async fn client(&self) -> Result<&TinyPlaceClient, String> {
        self.client
            .get_or_try_init(|| async {
                log::debug!("{LOG_PREFIX} building client base_url={}", self.base_url);
                // Derive 32-byte Ed25519 seed from the user's Solana wallet key.
                // The seed is consumed immediately; never logged or persisted.
                let seed = crate::openhuman::wallet::tinyplace_signer_seed().await?;
                let signer: Arc<dyn tinyplace::Signer> = Arc::new(
                    LocalSigner::from_seed(&seed)
                        .map_err(|e| format!("tinyplace signer init: {e}"))?,
                );
                log::debug!("{LOG_PREFIX} signer ready agent_id={}", signer.agent_id());
                Ok::<TinyPlaceClient, String>(TinyPlaceClient::new(TinyPlaceClientOptions {
                    base_url: self.base_url.clone(),
                    signer: Some(signer),
                    ..Default::default()
                }))
            })
            .await
    }
}

/// Resolve the tiny.place base URL from an explicit override + app env.
///
/// A non-blank explicit value (from `TINYPLACE_API_BASE_URL`) always wins.
/// Blank/whitespace is treated as unset (mirrors `wallet::defaults` handling)
/// so it can't produce an invalid base URL. With no explicit override, the
/// default tracks the app environment: staging → staging host, else prod.
fn resolve_base_url(explicit: Option<&str>, app_env: Option<&str>) -> String {
    if let Some(url) = explicit.map(str::trim).filter(|value| !value.is_empty()) {
        return url.to_string();
    }
    default_base_url_for_app_env(app_env).to_string()
}

/// The tiny.place host to default to for a given OpenHuman app environment.
fn default_base_url_for_app_env(app_env: Option<&str>) -> &'static str {
    if crate::api::config::is_staging_app_env(app_env) {
        TINYPLACE_STAGING_BASE_URL
    } else {
        TINYPLACE_PROD_BASE_URL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_override_always_wins() {
        // Even a staging app env must not override an explicit URL.
        assert_eq!(
            resolve_base_url(Some("https://custom.example"), Some("staging")),
            "https://custom.example"
        );
        assert_eq!(
            resolve_base_url(Some("  https://trimmed.example  "), None),
            "https://trimmed.example"
        );
    }

    #[test]
    fn blank_override_falls_through_to_app_env_default() {
        // Blank/whitespace is treated as unset.
        assert_eq!(
            resolve_base_url(Some("   "), Some("staging")),
            TINYPLACE_STAGING_BASE_URL
        );
        assert_eq!(resolve_base_url(Some(""), None), TINYPLACE_PROD_BASE_URL);
    }

    #[test]
    fn default_follows_app_env() {
        assert_eq!(
            default_base_url_for_app_env(Some("staging")),
            TINYPLACE_STAGING_BASE_URL
        );
        assert_eq!(
            default_base_url_for_app_env(Some("STAGING")),
            TINYPLACE_STAGING_BASE_URL
        );
        assert_eq!(
            default_base_url_for_app_env(Some("production")),
            TINYPLACE_PROD_BASE_URL
        );
        // Unknown / unset env defaults to prod.
        assert_eq!(default_base_url_for_app_env(None), TINYPLACE_PROD_BASE_URL);
        assert_eq!(
            default_base_url_for_app_env(Some("dev")),
            TINYPLACE_PROD_BASE_URL
        );
    }
}
