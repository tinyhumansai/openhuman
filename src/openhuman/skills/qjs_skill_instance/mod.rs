//! QuickJS Skill Instance management.
//!
//! This module provides the infrastructure to run OpenHuman skills within an isolated
//! QuickJS JavaScript environment. Each skill runs in its own dedicated context,
//! with restricted access to system resources via a set of native "ops" (bridges).
//!
//! Key characteristics of the QuickJS runtime:
//! - Contexts are `Send + Sync`, allowing for efficient use of `tokio::spawn`.
//! - Lightweight memory footprint per instance (~1-2MB).
//! - Direct memory and stack limits can be applied per instance.
//! - Asynchronous execution model integrated with the Rust event loop.

mod event_loop;
mod instance;
mod js_handlers;
mod js_helpers;
mod types;

pub use types::{BridgeDeps, QjsSkillInstance, SkillState};
