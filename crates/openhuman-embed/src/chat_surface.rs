//! The in-process web-chat event stream, for a host that is itself an
//! interactive surface (the TUI) rather than a Socket.IO client.
//!
//! With no channel service running, a host bridges approval, plan-review and
//! artifact events onto this stream by registering the surface subscribers,
//! then reads it with [`subscribe_web_channel_events`].

pub use openhuman_core::web_chat::{
    publish_web_channel_event, register_approval_surface_subscriber,
    register_artifact_surface_subscriber, subscribe_web_channel_events, ScopedWebEvent,
    SubagentProgressDetail, WebChannelEvent, WebEventContext,
};
