//! Thin HTTP wrapper over the openhuman backend's `/agent-integrations/crypto/*`
//! routes (deBridge DLN — see backend PR #852). All calls go through the shared
//! [`IntegrationClient`] so they inherit Bearer JWT auth, timeout, the
//! `{success,data}` envelope parsing, and proxy behavior.
//!
//! Responses are returned as raw `serde_json::Value` (the unwrapped `data`
//! payload) — deBridge returns a large nested quote/tx object and we pass the
//! parts we don't explicitly consume through unchanged.

use std::sync::Arc;

use serde_json::Value;

use crate::openhuman::config::Config;
use crate::openhuman::integrations::IntegrationClient;

const LOG_PREFIX: &str = "[web3]";

/// High-level client for the backend-proxied deBridge crypto operations.
#[derive(Clone)]
pub struct CryptoClient {
    inner: Arc<IntegrationClient>,
}

impl CryptoClient {
    pub fn new(inner: Arc<IntegrationClient>) -> Self {
        Self { inner }
    }

    /// Build from config; errors when the user is not signed in (no session
    /// token) — the same gate the composio tools use.
    pub fn from_config(config: &Config) -> Result<Self, String> {
        let inner = crate::openhuman::integrations::build_client(config).ok_or_else(|| {
            "web3 requires a signed-in session (no backend auth token available)".to_string()
        })?;
        Ok(Self::new(inner))
    }

    /// `GET /agent-integrations/crypto/routes` — chains deBridge can
    /// swap/bridge between.
    pub async fn routes(&self) -> Result<Value, String> {
        tracing::debug!("{LOG_PREFIX} routes");
        self.inner
            .get::<Value>("/agent-integrations/crypto/routes")
            .await
            .map_err(|e| format!("web3 routes failed: {e}"))
    }

    /// `POST /agent-integrations/crypto/swap` — single-chain swap quote + tx.
    pub async fn swap_tx(&self, body: &Value) -> Result<Value, String> {
        // Log only non-sensitive correlation fields — the body embeds wallet
        // addresses (sender / recipient) which must not be emitted in full.
        tracing::debug!("{LOG_PREFIX} swap_tx chain_id={:?}", body.get("chainId"));
        self.inner
            .post::<Value>("/agent-integrations/crypto/swap", body)
            .await
            .map_err(|e| format!("web3 swap quote failed: {e}"))
    }

    /// `POST /agent-integrations/crypto/bridge` — cross-chain bridge quote + tx.
    pub async fn bridge_tx(&self, body: &Value) -> Result<Value, String> {
        // Log only chain ids — the body embeds recipient / order-authority
        // wallet addresses which must not be emitted in full.
        tracing::debug!(
            "{LOG_PREFIX} bridge_tx src={:?} dst={:?}",
            body.get("srcChainId"),
            body.get("dstChainId")
        );
        self.inner
            .post::<Value>("/agent-integrations/crypto/bridge", body)
            .await
            .map_err(|e| format!("web3 bridge quote failed: {e}"))
    }
}
