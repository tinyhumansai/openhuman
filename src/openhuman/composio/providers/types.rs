//! Shared types for Composio provider implementations.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::openhuman::composio::client::{build_composio_client, ComposioClient};
use crate::openhuman::config::Config;

/// Reason a sync was triggered. Providers can use this to decide
/// whether to do a full backfill or an incremental pull.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncReason {
    /// First sync immediately after an OAuth handoff completes.
    ConnectionCreated,
    /// Periodic background sync from the scheduler.
    Periodic,
    /// Explicit user-driven sync from RPC / UI.
    Manual,
}

impl SyncReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncReason::ConnectionCreated => "connection_created",
            SyncReason::Periodic => "periodic",
            SyncReason::Manual => "manual",
        }
    }
}

/// Normalized user profile shape returned by every provider.
///
/// The shared fields (`display_name`, `email`, `username`, `avatar_url`,
/// `profile_url`)
/// cover what the desktop UI actually needs to render a connected
/// account card. Anything provider-specific (Gmail's `messagesTotal`,
/// Notion's workspace ids, …) goes into [`extras`](Self::extras) so
/// callers don't have to widen the shape every time a new toolkit
/// lands.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderUserProfile {
    pub toolkit: String,
    pub connection_id: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub profile_url: Option<String>,
    /// Provider-specific extras (raw JSON object).
    #[serde(default)]
    pub extras: serde_json::Value,
}

/// Result of a provider sync run. Mostly used for logging + UI status.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncOutcome {
    pub toolkit: String,
    pub connection_id: Option<String>,
    pub reason: String,
    pub items_ingested: usize,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub summary: String,
    /// Provider-specific extras (raw JSON object).
    #[serde(default)]
    pub details: serde_json::Value,
}

impl SyncOutcome {
    pub fn elapsed_ms(&self) -> u64 {
        self.finished_at_ms.saturating_sub(self.started_at_ms)
    }
}

/// Per-call context handed to provider methods.
///
/// `connection_id` is `None` when a method runs in a "no specific
/// connection" mode (e.g. an across-the-board periodic sync that
/// already iterated). For per-connection paths it is always populated.
#[derive(Clone)]
pub struct ProviderContext {
    pub config: Arc<Config>,
    pub client: ComposioClient,
    pub toolkit: String,
    pub connection_id: Option<String>,
}

impl ProviderContext {
    /// Build a context from the current config + a toolkit slug.
    ///
    /// Returns `None` if a [`ComposioClient`] cannot be constructed
    /// (no JWT yet — user not signed in). Callers should treat that
    /// case as "skip silently" rather than as a hard error, mirroring
    /// the existing op layer.
    pub fn from_config(
        config: Arc<Config>,
        toolkit: impl Into<String>,
        connection_id: Option<String>,
    ) -> Option<Self> {
        let client = build_composio_client(&config)?;
        Some(Self {
            config,
            client,
            toolkit: toolkit.into(),
            connection_id,
        })
    }

    /// Memory client handle if the global memory singleton is ready.
    /// Used by providers that want to persist sync snapshots.
    pub fn memory_client(&self) -> Option<crate::openhuman::memory::MemoryClientRef> {
        crate::openhuman::memory::global::client_if_ready()
    }
}
