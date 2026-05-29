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
pub mod agent_experience;
pub mod agent_tool_policy;
pub mod app_state;
pub mod approval;
pub mod artifacts;
pub mod audio_toolkit;
pub mod autocomplete;
pub mod billing;
pub mod channels;
pub mod codegraph;
pub mod composio;
pub mod config;
pub mod connectivity;
pub mod context;
pub mod cost;
pub mod credentials;
pub mod cron;
pub mod cwd_jail;
pub mod dashboard;
pub mod desktop_companion;
pub mod dev_paths;
pub mod devices;
pub mod doctor;
pub mod embeddings;
pub mod encryption;
pub mod health;
pub mod heartbeat;
pub mod http_host;
pub mod inference;
pub mod integrations;
pub mod javascript;
pub mod keyring;
pub mod learning;
pub mod mcp_audit;
pub mod mcp_client;
pub mod mcp_registry;
pub mod mcp_server;
pub mod meet;
pub mod meet_agent;
pub mod memory;
pub mod memory_archivist;
pub mod memory_conversations;
pub mod memory_entities;
pub mod memory_graph;
pub mod memory_queue;
pub mod memory_sources;
pub mod memory_store;
pub mod memory_sync;
pub mod memory_tools;
pub mod memory_tree;
pub mod migration;
pub mod migrations;
pub mod notifications;
pub mod overlay;
pub mod people;
pub mod prompt_injection;
pub mod provider_surfaces;
pub mod redirect_links;
pub mod referral;
pub mod routing;
pub mod runtime_node;
pub mod runtime_python;
pub mod scheduler_gate;
pub mod screen_intelligence;
pub mod security;
pub mod service;
pub mod skills;
pub mod socket;
pub mod startup;
pub mod subconscious;
pub mod task_sources;
pub mod team;
#[cfg(feature = "e2e-test-support")]
pub mod test_support;
pub mod text_input;
pub mod threads;
pub mod tls;
pub mod todos;
pub mod tokenjuice;
pub mod tool_registry;
pub mod tool_timeout;
pub mod tools;
pub mod update;
pub mod util;
pub mod vault;
pub mod voice;
pub mod wallet;
pub mod webhooks;
pub mod webview_accounts;
pub mod webview_apis;
pub mod webview_notifications;
pub mod whatsapp_data;
pub mod workspace;
