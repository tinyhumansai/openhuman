//! The `skills` module provides the core runtime and management system for OpenHuman skills.
//!
//! It includes:
//! - A QuickJS-based engine for executing skill code (`qjs_engine.rs`)
//! - A registry for managing installed and active skills (`skill_registry.rs`)
//! - Manifest parsing and validation for skill metadata (`manifest.rs`)
//! - Various operations for skill lifecycle management (`ops.rs`, `registry_ops.rs`)

pub mod bridge;
pub mod bus;
pub mod cron_scheduler;
pub mod manifest;
pub mod ops;
pub mod ping_scheduler;
pub mod preferences;
pub mod qjs_engine;
pub mod qjs_skill_instance;
pub mod quickjs_libs;
mod registry_cache;
pub mod registry_ops;
pub mod registry_types;
mod schemas;
pub mod skill_registry;
pub mod types;
pub mod utils;
pub mod working_memory;

pub use ops::*;
pub use qjs_engine::{global_engine, require_engine, set_global_engine};
pub use schemas::{
    all_controller_schemas as all_skills_controller_schemas,
    all_registered_controllers as all_skills_registered_controllers,
};
