//! Hosted channel delivery and messaging.
//!
//! The managed channel callback: one closure over the vendor contract
//! (`tinychannels_bus::ChannelBackendRequest -> ChannelBackendResult`) that
//! [`register`] hands to the core's channel callback slot
//! (`openhuman_embed::__host::channel_callback`). It serves
//!
//! - progressive delivery (send, edit, delete, typing) through
//!   [`delivery`], on the hosted client's `channels/{c}/...` routes, and
//! - reactions and threads through [`messaging`].
//!
//! The core keeps only the generic slot dispatch; with this callback
//! unregistered, those calls answer the `BACKEND_UNAVAILABLE:` sentinel.

#[cfg(feature = "channels")]
mod delivery;
#[cfg(feature = "channels")]
mod messaging;

#[cfg(feature = "channels")]
use openhuman_embed::__host::channel_callback::{
    register_channel_callback, CallbackFuture, ChannelCallbackCtx,
};
#[cfg(feature = "channels")]
use tinychannels_bus::{ChannelBackendRequest, ChannelBackendResult};

use crate::install::InstallOptions;

/// The id the callback registers under.
pub const CALLBACK_ID: &str = "tinyhumans";

/// Serves one callback request.
#[cfg(feature = "channels")]
pub(crate) async fn serve(
    ctx: ChannelCallbackCtx,
    request: ChannelBackendRequest,
) -> anyhow::Result<ChannelBackendResult> {
    use ChannelBackendRequest as Request;
    use ChannelBackendResult as Result;
    match request {
        Request::ProgressiveTransport(request) => Ok(Result::ProgressiveTransport(
            delivery::execute(&ctx, request).await,
        )),
        Request::SendReaction { channel, reaction } => {
            messaging::send_reaction(&ctx.config, &channel, reaction)
                .await
                .map(Result::SendReaction)
        }
        Request::CreateThread { channel, title } => {
            messaging::create_thread(&ctx.config, &channel, &title)
                .await
                .map(Result::Thread)
        }
        Request::UpdateThread {
            channel,
            thread_id,
            action,
        } => messaging::update_thread(&ctx.config, &channel, &thread_id, &action)
            .await
            .map(Result::Thread),
        Request::ListThreads { channel, active } => {
            messaging::list_threads(&ctx.config, &channel, active)
                .await
                .map(Result::Threads)
        }
        _ => {
            log::debug!("[tinyhumans] channels: request not served by the managed callback");
            anyhow::bail!("request is not served by the managed channel callback")
        }
    }
}

/// Registers this area's hooks with the core. Idempotent.
pub(crate) fn register(_options: &InstallOptions) {
    #[cfg(feature = "channels")]
    {
        register_channel_callback(CALLBACK_ID, |ctx, request| -> CallbackFuture<_> {
            Box::pin(serve(ctx, request))
        });
        log::debug!("[tinyhumans] channels: managed channel callback registered");
    }
    #[cfg(not(feature = "channels"))]
    log::trace!("[tinyhumans] channels: feature off, nothing to register");
}

#[cfg(all(test, feature = "channels"))]
#[path = "mod_tests.rs"]
mod tests;
