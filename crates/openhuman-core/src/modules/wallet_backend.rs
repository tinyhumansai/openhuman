//! The authenticated wallet-backend hook.
//!
//! The wallet module asks its host for three network reads/quotes (the supported
//! routes, a swap transaction, a bridge transaction) through the
//! `WalletBackend` callback. The core does not know who answers them: a host
//! that has such a service registers a handler here, keyed by id, and the
//! callback forwards to the first registered handler. With nothing registered
//! the callback fails with the `BACKEND_UNAVAILABLE:` sentinel, exactly like a
//! core with no hosted connection.
//!
//! The registry is generic: it carries a [`WalletBackendCall`] and a JSON
//! answer and knows nothing about the service behind it.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use crate::config::Config;
use crate::core::slots::HookSlot;

/// One wallet-backend request, as the wallet module asked for it.
#[derive(Debug, Clone, PartialEq)]
pub enum WalletBackendCall {
    /// The chains the backend can swap/bridge between.
    Routes,
    /// A single-chain swap quote and transaction for `body`.
    SwapTransaction(Value),
    /// A cross-chain bridge quote and transaction for `body`.
    BridgeTransaction(Value),
}

impl From<tinywallet_bus::wire::WalletBackendRequest> for WalletBackendCall {
    fn from(request: tinywallet_bus::wire::WalletBackendRequest) -> Self {
        use tinywallet_bus::wire::WalletBackendRequest as Request;
        match request {
            Request::Routes => Self::Routes,
            Request::SwapTransaction { body } => Self::SwapTransaction(body),
            Request::BridgeTransaction { body } => Self::BridgeTransaction(body),
        }
    }
}

/// The pending answer of a [`WalletBackendHandler`].
pub type WalletBackendFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;

/// A registered wallet-backend handler. It receives the loaded config so it can
/// resolve its own endpoint and credential.
pub type WalletBackendHandler =
    Arc<dyn Fn(Config, WalletBackendCall) -> WalletBackendFuture + Send + Sync>;

static HANDLERS: HookSlot<WalletBackendHandler> = HookSlot::new("modules::wallet_backend");

/// Registers `handler` under `id` for the whole process. Re-registering an id
/// replaces it in place.
pub fn register_wallet_backend<F>(id: impl Into<String>, handler: F)
where
    F: Fn(Config, WalletBackendCall) -> WalletBackendFuture + Send + Sync + 'static,
{
    HANDLERS.register(id, Arc::new(handler));
}

/// Registers a handler for the context built from `extensions`.
pub fn register_wallet_backend_in<F>(
    extensions: &crate::core::slots::Extensions,
    id: impl Into<String>,
    handler: F,
) where
    F: Fn(Config, WalletBackendCall) -> WalletBackendFuture + Send + Sync + 'static,
{
    HANDLERS.register_in(extensions, id, Arc::new(handler));
}

/// Removes the process-wide handler `id`.
pub fn unregister_wallet_backend(id: &str) -> bool {
    HANDLERS.unregister(id)
}

/// Forwards `call` to the first registered handler, or answers with the
/// `BACKEND_UNAVAILABLE:` sentinel when none is registered.
pub(super) async fn call_wallet_backend(
    config: Config,
    call: WalletBackendCall,
) -> Result<Value, String> {
    match HANDLERS.first() {
        Some(handler) => {
            tracing::debug!("[wallet-backend] forwarding to the registered handler");
            handler(config, call).await
        }
        None => {
            tracing::debug!("[wallet-backend] no handler registered");
            Err(format!(
                "{}wallet backend is not available in this build",
                crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
            ))
        }
    }
}

#[cfg(test)]
#[path = "wallet_backend_tests.rs"]
mod tests;
