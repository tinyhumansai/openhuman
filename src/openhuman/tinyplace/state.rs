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

/// Shared tiny.place state: lazy-built client keyed to one base URL.
pub(crate) struct TinyPlaceState {
    /// Lazily initialised on first [`TinyPlaceState::client`] call.
    client: OnceCell<TinyPlaceClient>,
    /// Backend base URL (from `TINYPLACE_API_BASE_URL` env or staging default).
    pub(crate) base_url: String,
}

impl TinyPlaceState {
    /// Build from the environment.  `TINYPLACE_API_BASE_URL` overrides the
    /// default production endpoint (set it to the staging host for testing).
    pub(crate) fn from_env() -> Self {
        // Treat a blank/whitespace override as unset so it can't produce an
        // invalid base_url (mirrors wallet::defaults env handling).
        let base_url = std::env::var("TINYPLACE_API_BASE_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "https://api.tiny.place".to_string());
        log::debug!("{LOG_PREFIX} state created base_url={base_url}");
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
