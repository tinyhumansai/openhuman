//! The host side of the `tinymemory` seam.
//!
//! `tinymemory-core` holds the substance of the memory subsystem — the store,
//! the summary tree, the sync pipelines, ingestion, recall. It names no
//! OpenHuman type. Everything it needs from us arrives through the traits in
//! [`tinymemory_api::host`], and this module is where we implement them.
//!
//! Three impls, and one wiring call:
//!
//! - [`MemoryHostConfig`] for [`Config`] — the ~two dozen config values the
//!   memory subsystem reads, as accessor methods. `tinymemory_core::Config` is
//!   the alias `dyn MemoryHostConfig`, so extracted code that took
//!   `config: &Config` before still does, and our concrete `Config` coerces at
//!   the call site.
//! - [`MemoryEventSink`] for [`BusEventSink`] — maps the core's memory-domain
//!   events onto [`DomainEvent`] and publishes them on our bus. The core never
//!   learns `DomainEvent` exists; that enum is OpenHuman's own vocabulary and
//!   spans agents, channels, cron and tools as well as memory.
//! - The embedding provider trait is not implemented here — it is *defined* in
//!   `tinymemory_api::host` and re-exported from
//!   [`crate::openhuman::inference::embeddings`], so our existing providers
//!   already implement it.
//!
//! Call [`install_memory_event_sink`] once during startup, before any memory
//! work begins. Until it runs, the core drops its events silently rather than
//! failing — see `tinymemory_api::events`.

use std::path::PathBuf;
use std::sync::Arc;

use tinymemory_api::host::{
    CloudProviderCreds, ComposioMode, LocalAiConfig, MemoryConfig, MemoryEvent, MemoryEventSink,
    MemoryHostConfig, MemoryTreeConfig, SchedulerGateConfig,
};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::config::Config;

// ── Config ──────────────────────────────────────────────────────────────────

#[async_trait::async_trait]
impl MemoryHostConfig for Config {
    fn workspace_dir(&self) -> &PathBuf {
        &self.workspace_dir
    }

    fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    fn memory_tree_content_root(&self) -> PathBuf {
        Config::memory_tree_content_root(self)
    }

    fn memory(&self) -> &MemoryConfig {
        &self.memory
    }

    fn memory_tree(&self) -> &MemoryTreeConfig {
        &self.memory_tree
    }

    fn scheduler_gate(&self) -> &SchedulerGateConfig {
        &self.scheduler_gate
    }

    fn local_ai(&self) -> &LocalAiConfig {
        &self.local_ai
    }

    fn cloud_providers(&self) -> &Vec<CloudProviderCreds> {
        &self.cloud_providers
    }

    fn embeddings_provider(&self) -> Option<&str> {
        self.embeddings_provider.as_deref()
    }

    fn memory_provider(&self) -> Option<&str> {
        self.memory_provider.as_deref()
    }

    fn workload_local_model(&self, workload: &str) -> Option<String> {
        Config::workload_local_model(self, workload)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn to_arc(&self) -> Arc<dyn MemoryHostConfig> {
        Arc::new(self.clone())
    }

    fn api_url(&self) -> Option<&str> {
        self.api_url.as_deref()
    }

    fn effective_backend_api_url(&self) -> String {
        // Resolution (env override, staging/prod default, trailing-slash
        // normalisation) lives in `api::config` and must not be re-derived on
        // the other side of the seam.
        crate::api::config::effective_backend_api_url(&self.api_url)
    }

    fn session_token(&self) -> Result<Option<String>, String> {
        crate::api::jwt::get_session_token(self)
    }

    fn default_model(&self) -> Option<&str> {
        self.default_model.as_deref()
    }

    fn default_temperature(&self) -> f64 {
        self.default_temperature
    }

    fn output_language(&self) -> Option<&str> {
        self.output_language.as_deref()
    }

    fn memory_sync_interval_secs(&self) -> Option<u64> {
        self.memory_sync_interval_secs
    }

    fn onboarding_completed(&self) -> bool {
        self.onboarding_completed
    }

    fn secrets_encrypt(&self) -> bool {
        self.secrets.encrypt
    }

    fn composio(&self) -> ComposioMode {
        ComposioMode {
            mode: self.composio.mode.clone(),
            entity_id: self.composio.entity_id.clone(),
            api_key: self.composio.api_key.clone(),
            triage_disabled: self.composio.triage_disabled,
            // `None` = the whole inbox window, which is exactly what this host
            // does today. The field is new on the contract (tinymemory#115-ish)
            // and scopes the *background* Gmail sync to a search query;
            // OpenHuman's `[integrations.composio]` has no such setting, so
            // there is nothing to forward and inventing a default here would
            // silently narrow what gets synced. Exposing it needs a config
            // field, its schema and a migration — a follow-up, not this branch.
            gmail_sync_query: None,
        }
    }

    fn memory_sources_json(&self) -> anyhow::Result<serde_json::Value> {
        // Serde-mediated because `MemorySourceEntry` is a tinycortex type and
        // `tinymemory-api` must not depend on the engine crate — that would
        // drag SQLite into the contract crate the dependency-light guarantee
        // exists to protect.
        Ok(serde_json::to_value(&self.memory_sources)?)
    }

    fn set_memory_sources_json(&mut self, value: serde_json::Value) -> anyhow::Result<()> {
        // Decode into a temporary first: a partial decode must not leave the
        // live registry half-written.
        let decoded = serde_json::from_value(value)?;
        self.memory_sources = decoded;
        Ok(())
    }

    fn composio_source_caps_migration_version(&self) -> u32 {
        self.composio_source_caps_migration_version
    }

    fn set_composio_source_caps_migration_version(&mut self, version: u32) {
        self.composio_source_caps_migration_version = version;
    }

    fn apply_env_overrides(&mut self) {
        Config::apply_env_overrides(self);
    }

    async fn save(&self) -> anyhow::Result<()> {
        Config::save(self).await
    }
}

// ── Events ──────────────────────────────────────────────────────────────────

/// Publishes the memory subsystem's events onto OpenHuman's event bus.
#[derive(Debug, Clone, Copy, Default)]
pub struct BusEventSink;

impl MemoryEventSink for BusEventSink {
    fn publish(&self, event: MemoryEvent) {
        // Not every memory event is a bus event. `LocalModelUnavailable` is a
        // durable user-facing error and rides the web channel instead, so the
        // mapping is allowed to answer "handled, nothing for the bus".
        if let Some(domain_event) = into_domain_event(event) {
            BUS.publish(domain_event);
        }
    }
}

/// Wire [`BusEventSink`] into `tinymemory-core`. Call once during startup,
/// before any memory work begins.
pub fn install_memory_event_sink() {
    tinymemory_api::events::set_event_sink(Arc::new(BusEventSink));
    log::debug!("[memory:host] event sink installed");
}

/// Structural mapping, deliberately exhaustive on the core's side so a new
/// [`MemoryEvent`] variant is a compile error here rather than an event that
/// silently never reaches the bus.
///
/// `None` means the event was handled here and has no bus equivalent — not
/// that it was dropped.
fn into_domain_event(event: MemoryEvent) -> Option<DomainEvent> {
    let domain_event =
        match event {
            MemoryEvent::SyncStageChanged {
                trigger,
                stage,
                provider,
                connection_id,
                detail,
                source_id,
            } => DomainEvent::MemorySyncStageChanged {
                trigger,
                stage,
                provider,
                connection_id,
                detail,
                source_id,
            },
            MemoryEvent::IngestionStarted {
                document_id,
                title,
                namespace,
                queue_depth,
            } => DomainEvent::MemoryIngestionStarted {
                document_id,
                title,
                namespace,
                queue_depth,
            },
            MemoryEvent::IngestionCompleted {
                document_id,
                namespace,
                success,
                elapsed_ms,
                queue_depth,
            } => DomainEvent::MemoryIngestionCompleted {
                document_id,
                namespace,
                success,
                elapsed_ms,
                queue_depth,
            },
            MemoryEvent::DocumentCanonicalized {
                source_id,
                source_kind,
                chunks_written,
                chunk_ids,
                canonicalized_at,
                body_preview,
            } => DomainEvent::DocumentCanonicalized {
                source_id,
                source_kind,
                chunks_written,
                chunk_ids,
                canonicalized_at,
                body_preview,
            },
            MemoryEvent::TreeSummarizerHourCompleted {
                namespace,
                node_id,
                token_count,
            } => DomainEvent::TreeSummarizerHourCompleted {
                namespace,
                node_id,
                token_count,
            },
            MemoryEvent::TreeSummarizerPropagated {
                namespace,
                node_id,
                level,
                token_count,
            } => DomainEvent::TreeSummarizerPropagated {
                namespace,
                node_id,
                level,
                token_count,
            },
            MemoryEvent::TreeSummarizerRebuildCompleted {
                namespace,
                total_nodes,
            } => DomainEvent::TreeSummarizerRebuildCompleted {
                namespace,
                total_nodes,
            },
            MemoryEvent::TreeBuildProgress {
                phase,
                step,
                tree_scope,
                level,
                item_count,
                detail,
            } => DomainEvent::MemoryTreeBuildProgress {
                phase,
                step,
                tree_scope,
                level,
                item_count,
                detail,
            },
            MemoryEvent::EmbeddingModelUnhealthy(reason) => DomainEvent::EmbeddingModelUnhealthy {
                provider: reason.provider,
                model: reason.model,
                fallback_provider: reason.fallback_provider,
                message: reason.message,
            },
            MemoryEvent::DriverBindFailed {
                configured_driver,
                bound_driver,
                reason,
            } => DomainEvent::MemoryDriverBindFailed {
                configured_driver,
                bound_driver,
                reason,
            },
            MemoryEvent::DiffSnapshotTaken {
                snapshot_id,
                source_id,
                source_kind,
                item_count,
                trigger,
            } => DomainEvent::MemoryDiffSnapshotTaken {
                snapshot_id,
                source_id,
                source_kind,
                item_count,
                trigger,
            },
            MemoryEvent::DiffMarkedRead {
                source_ids,
                snapshot_ids,
            } => DomainEvent::MemoryDiffMarkedRead {
                source_ids,
                snapshot_ids,
            },
            MemoryEvent::ComposioIntegrationsChanged { toolkits } => {
                DomainEvent::ComposioIntegrationsChanged { toolkits }
            }
            MemoryEvent::SyncRequested { channel_id } => {
                DomainEvent::MemorySyncRequested { channel_id }
            }
            // Goes to the durable UserErrorCenter over the web channel, not the bus.
            MemoryEvent::LocalModelUnavailable { origin } => {
                crate::openhuman::memory::tree::health::user_error::
                publish_local_model_unavailable_user_error(&origin);
                return None;
            }
            // Also web-channel-only (openhuman#5820): the engine already
            // quarantined and rebuilt, so there is nothing for a bus subscriber to
            // do — the user needs to hear it, durably.
            MemoryEvent::StoreCorruptQuarantined {
                origin,
                quarantined_path,
            } => {
                crate::openhuman::memory::tree::health::user_error::
                publish_store_corrupt_user_error(&origin, quarantined_path.as_deref());
                return None;
            }
        };
    Some(domain_event)
}
