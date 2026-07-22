//! Desktop companion domain — Clicky-style interaction loop.
//!
//! Ties hotkey activation, microphone capture, screen context, LLM
//! reasoning, speech synthesis, and visual pointing into a single
//! product experience. Orchestrates existing building blocks:
//!
//! - `screen_intelligence` — permission-gated capture sessions
//! - `voice` — hotkey, STT, TTS pipelines
//! - `meet_agent` — LLM orchestration patterns
//! - `overlay` — floating UI surface
//! - `provider_surfaces` — connected-app event queues
//!
//! This module is export-focused. Operational code lives in `session.rs`,
//! `pipeline.rs`, and `pointing.rs`.
//!
//! Facade for the `desktop-automation` gate (#5049): the behavioural submodules
//! (`handoff`, `pipeline`, `pointing`, `schemas`, `session`) are
//! `#[cfg(feature = "desktop-automation")]`. `types` and `bus` stay compiled in
//! both directions — both are dependency-free (serde / tokio broadcast only), and
//! the always-on `core::socketio` subscribes to `bus::subscribe_state_changed()`.
//! When off, nobody publishes, so the broadcast channel simply never fires. Only
//! the controller aggregators are stubbed.

// Dependency-free, compiled in BOTH builds: the inert types and the state-change
// broadcast bus (the always-on `core::socketio` subscribes to it).
pub mod bus;
pub mod types;

#[cfg(feature = "desktop-automation")]
pub mod handoff;
#[cfg(feature = "desktop-automation")]
pub mod pipeline;
#[cfg(feature = "desktop-automation")]
pub mod pointing;
#[cfg(feature = "desktop-automation")]
pub mod schemas;
#[cfg(feature = "desktop-automation")]
pub mod session;

#[cfg(not(feature = "desktop-automation"))]
mod stub;
#[cfg(not(feature = "desktop-automation"))]
pub use stub::*;

#[cfg(feature = "desktop-automation")]
pub use schemas::{
    all_desktop_companion_controller_schemas, all_desktop_companion_registered_controllers,
};
