//! Ingest adapters that feed the life_capture index from external event
//! sources. Each submodule owns one source (Composio, Apple EventKit, …)
//! and normalises its native payload into the canonical `Item` shape that
//! `rpc::handle_ingest` knows how to persist + embed.

pub mod composio;

pub use composio::{register_life_capture_composio_bridge, LifeCaptureComposioBridge};
