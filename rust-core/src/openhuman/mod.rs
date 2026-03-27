//! OpenHuman — lightweight agent runtime for OpenHuman.
//!
//! Ported from OpenHuman (MIT-licensed). Provides:
//! - Health registry for component monitoring
//! - Security policy, secrets, audit, pairing, and sandboxing
//! - Daemon supervisor with exponential backoff
//! - Agent runtime (dispatcher, loop, prompt, etc.)
//! - Providers, tools, memory, observability, approval, and skills

// These modules define the public API surface for future agent features.
// Many types/functions are not yet consumed but are intentionally exported.
#![allow(dead_code)]

pub mod accessibility;
pub mod agent;
pub mod approval;
pub mod channels;
pub mod config;
pub mod cost;
pub mod cron;
pub mod daemon;
pub mod doctor;
pub mod gateway;
pub mod hardware;
pub mod health;
pub mod heartbeat;
pub mod identity;
pub mod integrations;
pub mod memory;
pub mod migration;
pub mod multimodal;
pub mod observability;
pub mod onboard;
pub mod peripherals;
pub mod providers;
pub mod rag;
pub mod runtime;
pub mod security;
pub mod service;
pub mod skillforge;
pub mod skills;
pub mod tools;
pub mod tunnel;
pub mod util;
