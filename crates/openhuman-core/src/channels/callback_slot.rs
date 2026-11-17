//! The channel callback slot: where an outside crate plugs in the transport a
//! channel module's host callbacks run through.
//!
//! The channel module (`tinychannels`) calls back into the host with a
//! [`ChannelBackendRequest`] and expects a [`ChannelBackendResult`]. The core
//! answers the requests it can itself; a registered callback answers the
//! progressive-delivery transport (send, edit, delete, typing) in place of the
//! built-in managed path. The callback is a plain closure over the vendor
//! contract types: the core defines no trait for it.
//!
//! With nothing registered [`bus::delivery`](super::bus) has no progressive
//! delivery transport and answers `BACKEND_UNAVAILABLE:`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tinychannels_bus::{ChannelBackendRequest, ChannelBackendResult};
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::core::slots::{Extensions, HookSlot};

/// A boxed `Send` future.
pub type CallbackFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What a registered callback is told about the request's origin.
#[derive(Clone)]
pub struct ChannelCallbackCtx {
    /// The configuration the channel session was opened with.
    pub config: Config,
    /// The channel the session serves, when the caller knows it.
    pub channel: Option<String>,
    /// Cancelled when the channel session closes.
    pub session: CancellationToken,
}

/// A registered channel callback.
pub type ChannelCallback = Arc<
    dyn Fn(
            ChannelCallbackCtx,
            ChannelBackendRequest,
        ) -> CallbackFuture<anyhow::Result<ChannelBackendResult>>
        + Send
        + Sync,
>;

static CALLBACKS: HookSlot<ChannelCallback> = HookSlot::new("channels::callback");

/// Registers the channel callback `id` for the whole process. The first
/// registered callback (registration order) serves requests.
pub fn register_channel_callback<F>(id: impl Into<String>, callback: F)
where
    F: Fn(
            ChannelCallbackCtx,
            ChannelBackendRequest,
        ) -> CallbackFuture<anyhow::Result<ChannelBackendResult>>
        + Send
        + Sync
        + 'static,
{
    CALLBACKS.register(id, Arc::new(callback));
}

/// Registers the channel callback `id` for the context built from `extensions`.
pub fn register_channel_callback_in<F>(extensions: &Extensions, id: impl Into<String>, callback: F)
where
    F: Fn(
            ChannelCallbackCtx,
            ChannelBackendRequest,
        ) -> CallbackFuture<anyhow::Result<ChannelBackendResult>>
        + Send
        + Sync
        + 'static,
{
    CALLBACKS.register_in(extensions, id, Arc::new(callback));
}

/// Removes the process-wide channel callback `id`.
pub fn unregister_channel_callback(id: &str) -> bool {
    CALLBACKS.unregister(id)
}

/// The callback that serves requests now, if one is registered.
pub(crate) fn current() -> Option<ChannelCallback> {
    CALLBACKS.first()
}

#[cfg(test)]
#[path = "callback_slot_tests.rs"]
mod tests;
