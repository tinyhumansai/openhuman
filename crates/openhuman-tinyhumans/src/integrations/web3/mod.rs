//! The web3 crypto client (`/agent-integrations/crypto/*`, deBridge DLN) and
//! the wallet-backend handler that serves the wallet module's callbacks with it.
//!
//! The core's wallet host forwards the module's `WalletBackend` requests to the
//! handler registered here ([`register`]); with this crate absent the callback
//! answers `BACKEND_UNAVAILABLE:`.

pub mod client;

use openhuman_embed::__host::wallet_backend::{register_wallet_backend, WalletBackendCall};

pub use client::CryptoClient;

/// The id the wallet-backend handler registers under.
pub const HANDLER_ID: &str = "tinyhumans-web3";

/// Registers the wallet-backend handler with the core. Idempotent.
pub(crate) fn register() {
    register_wallet_backend(HANDLER_ID, |config, call| {
        Box::pin(async move {
            let client = CryptoClient::from_config(&config)?;
            match call {
                WalletBackendCall::Routes => client.routes().await,
                WalletBackendCall::SwapTransaction(body) => client.swap_tx(&body).await,
                WalletBackendCall::BridgeTransaction(body) => client.bridge_tx(&body).await,
            }
        })
    });
}
