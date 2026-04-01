//! OpenHuman — lightweight agent runtime for OpenHuman.
//!
//! Provides:
//! - Health registry for component monitoring
//! - Security policy, secrets, audit, channel pairing, and sandboxing
//! - Daemon supervisor with exponential backoff
//! - Agent runtime (dispatcher, loop, prompt, etc.)
//! - Providers, tools, memory, approval, and skills

// These modules define the public API surface for future agent features.
// Many types/functions are not yet consumed but are intentionally exported.
#![allow(dead_code)]

pub mod agent;
pub mod approval;
pub mod autocomplete;
pub mod channels;
pub mod config;
pub mod cost;
pub mod credentials;
pub mod cron;
pub mod dev_paths;
pub mod doctor;
pub mod encryption;
pub mod health;
pub mod heartbeat;
pub mod learning;
pub mod local_ai;
pub mod memory;
pub mod migration;
pub mod providers;
pub mod screen_intelligence;
pub mod security;
pub mod service;
pub mod skills;
pub mod tools;
pub mod tunnel;
pub mod util;
pub mod workspace;
