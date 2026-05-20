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

pub mod bus;
pub mod handoff;
pub mod pipeline;
pub mod pointing;
pub mod schemas;
pub mod session;
pub mod types;

pub use schemas::{
    all_desktop_companion_controller_schemas, all_desktop_companion_registered_controllers,
};
