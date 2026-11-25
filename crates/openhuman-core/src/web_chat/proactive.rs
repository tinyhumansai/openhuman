//! Web-surface delivery for proactive messages.
//!
//! This subscriber belongs to `web_chat`, rather than the optional external
//! channels runtime, because web proactive messages are available even when
//! the `channels` feature is disabled.

use crate::core::events::DomainEvent;
use crate::core::runtime::DispatchCtx;
use crate::web_chat::event_context;
use crate::web_chat::{publish_web_channel_event_in, WebChannelEvent};
use async_trait::async_trait;
use std::sync::Arc;
use tinybus::EventHandler;

/// Register the web destination for proactive messages once the global bus is
/// ready. A pre-bootstrap attempt is deferred without consuming the `Once`.
pub(crate) fn register_proactive_subscriber() {
    if crate::core::bus::BUS.get().is_none() {
        tracing::warn!("[proactive] deferred web subscriber registration — bus not initialized");
        return;
    }

    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| {
        if let Some(handle) = crate::core::bus::BUS.subscribe(Arc::new(WebProactiveSubscriber)) {
            std::mem::forget(handle);
            tracing::debug!("[proactive] web subscriber registered");
        } else {
            tracing::warn!("[proactive] failed to register web subscriber — bus not initialized");
        }
    });
}

pub(crate) struct WebProactiveSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for WebProactiveSubscriber {
    fn name(&self) -> &str {
        "web_chat::proactive"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["cron"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ProactiveMessageRequested {
            source,
            message,
            job_name,
            tenant,
        } = event
        else {
            return;
        };

        let Some(ctx) = DispatchCtx::for_event(tenant.as_ref()) else {
            tracing::warn!(source = %source, "[proactive] skipping web delivery without a live tenant context");
            return;
        };

        let thread_id = format!("proactive:{}", job_name.as_deref().unwrap_or("system"));
        publish_web_channel_event_in(
            &ctx,
            WebChannelEvent::ProactiveMessage {
                context: event_context("system", thread_id, uuid::Uuid::new_v4().to_string()),
                full_response: Some(message.clone()),
                message: None,
                success: Some(true),
                persisted_message_id: None,
                parts: None,
            },
        );

        tracing::debug!(
            source = %source,
            thread_id = %format!("proactive:{}", job_name.as_deref().unwrap_or("system")),
            message_len = message.len(),
            "[proactive] delivered to web surface"
        );
    }
}
