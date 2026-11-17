//! Host-only routing envelope around TinyChannels' typed web event contract.

use serde::{Serialize, Serializer};
use tinychannels_bus::web as bus_web;

pub use bus_web::{
    ChatSuggestion, GuardrailPayload, GuardrailReason, QueueItemPayload, SubagentProgressDetail,
    SubagentUsagePayload, TurnTimingPayload, TurnUsagePayload, WebChannelEvent, WebEventContext,
};

pub(crate) fn event_context(
    client_id: impl Into<String>,
    thread_id: impl Into<String>,
    request_id: impl Into<String>,
) -> bus_web::WebEventContext {
    bus_web::WebEventContext {
        client_id: client_id.into(),
        thread_id: thread_id.into(),
        request_id: request_id.into(),
        ..Default::default()
    }
}

/// A typed wire event plus tenant stamps used only for in-process routing.
///
/// `event` remains the TinyChannels-owned wire contract. The routing fields
/// are deliberately absent from serialization and cannot be supplied by the
/// event producer's payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ScopedWebEvent {
    pub(crate) event: WebChannelEvent,
    agent: Option<String>,
    profile: Option<String>,
}

impl ScopedWebEvent {
    pub(crate) fn new(event: WebChannelEvent) -> Self {
        Self {
            event,
            agent: None,
            profile: None,
        }
    }

    /// Whether this event belongs on the named agent's stream.
    pub fn belongs_to(&self, agent: &str) -> bool {
        self.agent.as_deref() == Some(agent)
    }

    /// Whether this event belongs on the named profile's stream.
    pub fn belongs_to_profile(&self, profile: &str) -> bool {
        self.profile.as_deref() == Some(profile)
    }

    /// Addressing and ordering fields from the canonical TinyChannels event.
    pub fn context(&self) -> &bus_web::WebEventContext {
        self.event.context()
    }

    /// The canonical owner event carried by this host-routing envelope.
    pub fn wire_event(&self) -> &WebChannelEvent {
        &self.event
    }

    /// Historical Socket.IO discriminator for this canonical event.
    pub fn event_name(&self) -> &'static str {
        self.event.event_name()
    }

    /// Stamp routing metadata from the host execution context.
    pub(crate) fn stamp_tenant(&mut self, tenant: Option<crate::core::runtime::Tenant>) {
        let Some(tenant) = tenant else {
            return;
        };
        if self.agent.is_none() {
            self.agent = tenant.agent;
        }
        if self.profile.is_none() {
            self.profile = tenant.profile;
        }
    }
}

impl Serialize for ScopedWebEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.event.serialize(serializer)
    }
}

#[cfg(test)]
#[path = "channel_event_tests.rs"]
mod tests;
