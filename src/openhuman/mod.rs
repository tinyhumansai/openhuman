//! OpenHuman — a lightweight agent runtime for human-AI collaboration.
//!
//! The `openhuman` module is the heart of the agent-specific logic within the core.
//! It provides a comprehensive set of features for building and running AI agents,
//! including:
//! - **Configuration & Credentials**: Management of user settings and secure storage.
//! - **Agent Runtime**: Dispatchers, loops, and prompt management for agent execution.
//! - **Memory & Knowledge**: Systems for persistent storage and retrieval of information.
//! - **Channels & Providers**: Integrations with external platforms (Telegram, Discord, etc.).
//! - **Skills & Tools**: Extensible runtime for adding custom capabilities to agents.
//! - **Security & Monitoring**: Sandboxing, health checks, and audit logging.

// These modules define the public API surface for agent features.
// Many types/functions are intended for future use or integration with the frontend.
#![allow(dead_code)]

pub mod about_app;
pub mod accessibility;
pub mod agent;
pub mod app_state;
pub mod approval;
pub mod autocomplete;
pub mod billing;
pub mod channels;
pub mod composio;
pub mod config;
pub mod context;
pub mod cost;
pub mod credentials;
pub mod cron;
pub mod curated_memory;
pub mod dev_paths;
pub mod doctor;
pub mod embeddings;
pub mod encryption;
pub mod health;
pub mod heartbeat;
pub mod integrations;
pub mod learning;
pub mod life_capture;
pub mod local_ai;
pub mod memory;
pub mod migration;
pub mod node_runtime;
pub mod notifications;
pub mod overlay;
pub mod people;
pub mod provider_surfaces;
pub mod providers;
pub mod referral;
pub mod routing;
pub mod screen_intelligence;
pub mod security;
pub mod service;
pub mod skills;
pub mod socket;
pub mod subconscious;
pub mod team;
pub mod text_input;
pub mod threads;
pub mod tokenjuice;
pub mod tool_timeout;
pub mod tools;
pub mod tree_summarizer;
pub mod update;
pub mod util;
pub mod voice;
pub mod webhooks;
pub mod webview_notifications;
pub mod workspace;
