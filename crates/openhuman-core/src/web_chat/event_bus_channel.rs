//! The process-wide `WebChannelEvent` broadcast channel and its publishers.

use once_cell::sync::Lazy;
use tokio::sync::broadcast;

use crate::core::runtime::DispatchCtx;
use crate::web_chat::WebChannelEvent;

static EVENT_BUS: Lazy<broadcast::Sender<WebChannelEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(512);
    tx
});

pub fn subscribe_web_channel_events() -> broadcast::Receiver<WebChannelEvent> {
    EVENT_BUS.subscribe()
}

/// Publish `event` to every subscribed socket bridge and the JSON-RPC
/// `/events` stream, stamping `ts` (epoch ms) when the caller left it unset
/// so every emitted event carries a wall-clock time the frontend can use for
/// ordering/latency display without guessing at receive time.
pub fn publish_web_channel_event(event: WebChannelEvent) {
    publish_web_channel_event_in(&DispatchCtx::ambient(), event);
}

/// [`publish_web_channel_event`] with the tenant stamped from `ctx`: the one
/// the calling turn runs under, rather than the calling task's own scope.
pub fn publish_web_channel_event_in(ctx: &DispatchCtx, mut event: WebChannelEvent) {
    if event.ts.is_none() {
        event.ts = Some(crate::web_chat::progress_bridge::unix_epoch_ms());
    }
    event.stamp_tenant(ctx.tenant().ok());
    let _ = EVENT_BUS.send(event);
}
