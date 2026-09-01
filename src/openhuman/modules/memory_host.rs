//! Host-owned callbacks used by the separately compiled TinyMemory module.
//!
//! This file is the bus-served twin of `memory/host_impls.rs`. That file
//! installs the engine's seam traits as process globals, which only works while
//! the engine is compiled into this binary; these interfaces serve the same
//! capabilities to an engine that is *not*, over the module's connection.
//!
//! # Which seams are here, and why only these
//!
//! A seam earns an interface here when the capability genuinely belongs to the
//! host and the module cannot answer it from anything it was handed at load
//! time. Embedding and chat qualify because the credential, the provider
//! routing and the cost accounting all live here (see `tinymemory-module`'s
//! `embedding.rs` for the long form of that argument). Event publishing, error
//! reporting and spaCy qualify because their destinations are host subsystems.
//! Composio qualifies for the same reason one size up: the client is a *host
//! agent tool*, mode dispatch reads host config, and the OAuth session it
//! depends on is refreshed by host code.
//!
//! Seams that are *not* here are not omissions to be filled in later. The
//! module answers `ConfigLoader` from the `ModuleConfig` it was handed —
//! proxying it would mean asking the host to re-read a config the module
//! already has, with two answers free to disagree — and the scheduler gate and
//! the shutdown sequencer cannot cross a bus at all: one is synchronous, one
//! hands back a `tokio::sync::Notify`, and one takes a Rust closure.
//!
//! # The interfaces are the whole contract
//!
//! Nothing here is discoverable by the module except through the well-known
//! name it proxies. A module built against a newer contract that asks for an
//! interface an older host never served gets a name-resolution failure, which
//! is why the module side reports that failure by name rather than degrading to
//! a plausible-looking empty answer.

use crate::core::bus::BUS;
use crate::openhuman::config::Config;
use std::sync::Arc;
use tinybus::ObjectPath;
use tinyconnectors_bus::{ComposioConnection, ComposioExecuteResponse};
use tinyinference::model::{ModelRequest, ModelResponse};
use tinymemory_api::host::{MemoryEvent, SpacyResponse};

const EMBEDDING_NAME: &str = "ai.tinyhumans.tinymemory.EmbeddingHost";
const EMBEDDING_PATH: &str = "/ai/tinyhumans/tinymemory/EmbeddingHost";
const CHAT_NAME: &str = "ai.tinyhumans.tinymemory.ChatHost";
const CHAT_PATH: &str = "/ai/tinyhumans/tinymemory/ChatHost";
const COMPOSIO_NAME: &str = "ai.tinyhumans.tinymemory.ComposioHost";
const COMPOSIO_PATH: &str = "/ai/tinyhumans/tinymemory/ComposioHost";
const RUNTIME_NAME: &str = "ai.tinyhumans.tinymemory.RuntimeHost";
const RUNTIME_PATH: &str = "/ai/tinyhumans/tinymemory/RuntimeHost";

#[derive(Clone)]
struct EmbeddingCallbacks(Arc<Config>);

#[tinybus::interface(name = "ai.tinyhumans.tinymemory.EmbeddingHost")]
impl EmbeddingCallbacks {
    async fn embed(
        &self,
        provider: String,
        model: String,
        dimensions: usize,
        texts: Vec<String>,
    ) -> tinybus::Result<Vec<Vec<f32>>> {
        let api_key = crate::openhuman::inference::embeddings::resolve_api_key(&self.0, &provider);
        let endpoint = self
            .0
            .cloud_providers
            .iter()
            .find(|candidate| candidate.slug == provider)
            .map(|candidate| candidate.endpoint.as_str())
            .filter(|endpoint| !endpoint.is_empty());
        let embedder =
            crate::openhuman::inference::embeddings::create_embedding_provider_with_config(
                &self.0, &provider, &model, dimensions, &api_key, endpoint,
            )
            .map_err(method_error)?;
        let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
        embedder.embed(&borrowed).await.map_err(method_error)
    }
}

#[derive(Clone)]
struct ChatCallbacks(Arc<Config>);

#[tinybus::interface(name = "ai.tinyhumans.tinymemory.ChatHost")]
impl ChatCallbacks {
    async fn complete(
        &self,
        role: String,
        request: ModelRequest,
    ) -> tinybus::Result<ModelResponse> {
        let model = resolve_chat_model(&role, &self.0).map_err(method_error)?;
        model.invoke(&(), request).await.map_err(method_error)
    }
}

/// Resolve the model a module-side chat call runs on, by role.
///
/// The `"summarization"` role is special-cased through the tree summarizer's
/// provider ladder (`tree_runtime::ops::create_provider`) rather than the
/// role factory, because every memory fold the module performs — an explicit
/// `tree_summarizer_run`/`rebuild`, the scheduled `seal`/`cascade` passes, and
/// the archivist's recap `summarise` — reaches the host through this one seam,
/// and the ladder is where the host's routing *policy* lives: local Ollama
/// when `local_ai.runtime_enabled`, the configured cloud provider only under
/// `memory_tree.cloud_summarization_opt_in`, and a refusal otherwise.
///
/// Routing the role factory directly here was a consent hole, not just a
/// preference miss: with local AI enabled and the cloud opt-in `false`, the
/// host-side `create_provider` precondition in `tree_runtime::ops` succeeds
/// (a local model is constructible), and the blind role factory then resolved
/// `"summarization"` to the configured cloud provider anyway — memory content
/// leaving the machine against an explicit opt-out. The ladder cannot make
/// that move: local wins while it is enabled, and cloud requires the opt-in.
///
/// Every other role keeps the role factory unchanged.
fn resolve_chat_model(
    role: &str,
    config: &Config,
) -> anyhow::Result<std::sync::Arc<dyn tinyinference::model::ChatModel<()>>> {
    if role == "summarization" {
        let (model, _) = crate::openhuman::memory::tree::tree_runtime::ops::create_provider(config)
            .map_err(anyhow::Error::msg)?;
        return Ok(model);
    }
    let (model, _) = crate::openhuman::inference::provider::create_chat_model_with_model_id(
        role,
        config,
        config.default_temperature,
    )?;
    Ok(model)
}

/// Composio, as the engine's sync pipelines need it.
///
/// # Why the `&Config` argument is not on the wire
///
/// `ComposioHost`'s four trait methods each take a `&tinymemory_core::Config`.
/// That argument does not appear on any method below, and it must not: in
/// module mode it is the *engine's* config, built from the `ModuleConfig` the
/// module was loaded with, and it is not this process's `Config`. Sending it
/// would mean the host resolving a Composio client against a config the host
/// did not write and cannot trust to be current. `ChatHost` made the same call
/// — `Complete` takes a role and a request, and nothing else — and the module
/// side of that proxy simply drops the ambient context on the floor.
///
/// So the config each method here works from is the host's own, and the only
/// question is *which* host config.
///
/// # Why every method re-reads the config from disk
///
/// The install-time snapshot is the wrong answer, and quietly so. Composio's
/// state changes underneath a running process on the two events that matter
/// most: an OAuth completion in the browser, and a `set_api_key` RPC. Both
/// write config and credentials and neither restarts anything. An
/// `IsAvailable` answered from a snapshot taken at launch reports "not signed
/// in" for the rest of the session to a user who signed in a minute ago, and
/// the sync layer treats that as *skip silently* — the exact
/// looks-empty-rather-than-broken failure the seam exists to prevent.
///
/// `memory/host_impls.rs` gets liveness a cheaper way: its async methods
/// re-read from disk, and its two synchronous probes recover the caller's
/// config, which the engine's own loops keep fresh. With no caller config to
/// recover, a fresh read is what "current as of the call" costs here. It is
/// bounded — the probes sit on periodic sync paths, a handful of reads per
/// tick, next to network calls that dominate them.
///
/// The two probes cannot fail, so a config that will not load falls back to the
/// install-time snapshot rather than inventing a "no" — an unreadable config
/// file is not evidence that the user disconnected Composio.
#[derive(Clone)]
struct ComposioCallbacks(Arc<Config>);

#[tinybus::interface(name = "ai.tinyhumans.tinymemory.ComposioHost")]
impl ComposioCallbacks {
    async fn list_connections(&self) -> tinybus::Result<Vec<ComposioConnection>> {
        use crate::openhuman::integrations::composio::client::{
            create_composio_client, direct_list_connections, ComposioClientKind,
        };
        let config = self.live_config().await.map_err(method_error)?;
        let response = match create_composio_client(&config)
            .map_err(|e| method_error(format!("create_composio_client: {e:#}")))?
        {
            ComposioClientKind::Backend(client) => client
                .list_connections()
                .await
                .map_err(|e| method_error(format!("list_connections (backend): {e:#}")))?,
            ComposioClientKind::Direct(direct) => {
                direct_list_connections(&direct).await.map_err(|e| {
                    // [#1166 / Sentry TAURI-RUST-X9] The v3 `/connected_accounts`
                    // 401 shape has to reach the observability classifier, and it
                    // only fires on a message carrying the `[composio-direct]`
                    // anchor. Report it here rather than relying on the caller:
                    // by the time this reaches the module it is wrapped in a
                    // `MethodFailed` envelope, and the classifier that keys on
                    // the anchor runs in this process, not that one.
                    let rendered = format!("[composio-direct] list_connections (direct): {e:#}");
                    crate::openhuman::integrations::composio::ops::report_composio_op_error(
                        "list_connections",
                        &rendered,
                    );
                    method_error(rendered)
                })?
            }
        };
        Ok(response.connections)
    }

    /// Run one Composio tool.
    ///
    /// `entity_id` and `connection_id` travel even though backend mode ignores
    /// both: which mode is in force is resolved on this side, at call time, and
    /// a caller that omitted them would silently lose the connection pin the
    /// moment a user switched to direct mode.
    async fn execute(
        &self,
        tool: String,
        arguments: Option<serde_json::Value>,
        entity_id: String,
        connection_id: Option<String>,
    ) -> tinybus::Result<ComposioExecuteResponse> {
        use crate::openhuman::integrations::composio::client::{
            create_composio_client, direct_execute, ComposioClientKind,
        };
        let config = self.live_config().await.map_err(method_error)?;
        match create_composio_client(&config).map_err(|e| method_error(format!("{e:#}")))? {
            ComposioClientKind::Backend(client) => client
                .execute_tool(&tool, arguments)
                .await
                .map_err(|e| method_error(format!("{e:#}"))),
            ComposioClientKind::Direct(direct) => direct_execute(
                &direct,
                &tool,
                arguments,
                &entity_id,
                connection_id.as_deref(),
            )
            .await
            .map_err(|e| method_error(format!("{e:#}"))),
        }
    }

    /// The direct-mode Composio API key, or `None` when direct mode is unset.
    ///
    /// This is the one method here that hands a credential to the module, and
    /// it is worth being explicit about: `EmbeddingHost` deliberately refuses
    /// to, and keeps the key on this side of `Embed` instead. The difference is
    /// that the engine's `composio_config` builds its *own* HTTP client from
    /// this key, so a `None` here is not a degraded answer the caller routes
    /// around — it is a hard "direct-mode sync cannot run".
    ///
    /// Narrowing that would mean moving the direct-mode sync client behind an
    /// `Execute`-shaped method, which is a change to the engine's contract and
    /// not something to smuggle in through its host half.
    async fn api_key(&self) -> tinybus::Result<Option<String>> {
        let config = self.live_config_or_installed().await;
        let stored = crate::openhuman::security::credentials::get_composio_api_key(config.as_ref())
            .ok()
            .flatten();
        let configured = config
            .composio
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);
        Ok(stored.or(configured))
    }

    /// The app-session bearer proxied ("backend") Composio mode authenticates
    /// with.
    ///
    /// Answered from a LIVE config read rather than the install-time snapshot,
    /// and that is the whole reason this is a bus member instead of a field on
    /// `ModuleConfig`. The bearer is a session JWT this host refreshes; a value
    /// the module captured at load would work until it expired and then make
    /// every sync fail with an auth error that reads as the user being signed
    /// out. Asking per call means the module always gets the one valid now.
    ///
    /// `Ok(None)` means this host has no session to lend — a signed-out user,
    /// not a broken one. The engine turns that into a named refusal rather than
    /// treating it as "nothing to sync", which is the distinction that keeps a
    /// signed-out user from looking like a user with no connected sources.
    ///
    /// The failure is folded into `None` deliberately: a token store that
    /// errors and a store that is empty are the same fact to the caller — this
    /// process cannot authenticate a proxied call right now — and splitting
    /// them would give the module a second failure mode it cannot act on
    /// differently.
    async fn session_bearer(&self) -> tinybus::Result<Option<String>> {
        let config = self.live_config_or_installed().await;
        Ok(crate::api::jwt::get_session_token(config.as_ref())
            .ok()
            .flatten())
    }

    /// Whether *some* viable Composio client resolves right now.
    ///
    /// Deliberately the factory probe rather than a session-token check: a
    /// direct-mode user has no backend session, and testing for one would read
    /// as signed-out and skip them (#1710).
    async fn is_available(&self) -> tinybus::Result<bool> {
        use crate::openhuman::integrations::composio::client::create_composio_client;
        let config = self.live_config_or_installed().await;
        Ok(create_composio_client(config.as_ref()).is_ok())
    }
}

impl ComposioCallbacks {
    /// The host config as it is on disk right now.
    ///
    /// Addressed by the install-time config's paths rather than by the ambient
    /// ones, so a workspace-scoped process re-reads its own config and not the
    /// default profile's.
    async fn live_config(&self) -> Result<Config, String> {
        crate::openhuman::config::rpc::reload_config_from_paths(
            &self.0.config_path,
            &self.0.workspace_dir,
        )
        .await
    }

    /// [`Self::live_config`], falling back to the install-time snapshot.
    ///
    /// For the two probes only. They return a bare `Option`/`bool` with no room
    /// for "could not tell", and answering "no" on a transient read failure
    /// would stop a sync run that was working a minute ago.
    async fn live_config_or_installed(&self) -> Arc<Config> {
        match self.live_config().await {
            Ok(config) => Arc::new(config),
            Err(error) => {
                log::debug!(
                    "[memory:composio-host] config re-read failed, answering the probe from the \
                     install-time snapshot: {error}"
                );
                Arc::clone(&self.0)
            }
        }
    }
}

#[derive(Clone)]
struct RuntimeCallbacks(Arc<Config>);

#[tinybus::interface(name = "ai.tinyhumans.tinymemory.RuntimeHost")]
impl RuntimeCallbacks {
    /// Bridge a module-side memory event onto this host.
    ///
    /// Every arm is [`into_domain_event`]'s: it either maps the event onto a
    /// [`DomainEvent`](crate::core::events::DomainEvent) for the bus or handles
    /// it web-channel-side and answers `None`. `StoreCorruptQuarantined` is the
    /// second kind — it publishes the durable user error and returns `None`,
    /// the same shape the in-process sink's arm has in `memory::host`.
    ///
    /// **There is deliberately no in-process chunk-store reset here any more
    /// (#5560).** It existed because this process embedded a second copy of the
    /// engine whose cached SQLite handle still pointed at the inode the module
    /// had just renamed, so an in-process read kept failing with `database disk
    /// image is malformed` until restart (openhuman#5820). Every in-process
    /// reader it protected is gone: `sources::status` asks
    /// `MemoryChunks::source_ingest_status`, recall goes through
    /// `memory::binding` to this same driver, and the only surviving openers of
    /// the host's chunk store are `#[cfg(test)]`. Nothing else in the corruption
    /// path needs an engine either — `user_error`'s detectors classify text, and
    /// `tree::tree::rpc`'s `latest_quarantine` reads the directory.
    async fn publish_event(&self, event: MemoryEvent) -> tinybus::Result<()> {
        if let Some(event) = into_domain_event(event) {
            BUS.publish(event);
        }
        Ok(())
    }

    async fn report_error(
        &self,
        classify_expected: bool,
        rendered: String,
        domain: String,
        operation: String,
        tags: Vec<(String, String)>,
    ) -> tinybus::Result<()> {
        let borrowed: Vec<(&str, &str)> = tags
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        if classify_expected {
            crate::core::observability::report_error_or_expected(
                &rendered, &domain, &operation, &borrowed,
            );
        } else {
            crate::core::observability::report_error(&rendered, &domain, &operation, &borrowed);
        }
        Ok(())
    }

    async fn extract_spacy(&self, text: String) -> tinybus::Result<SpacyResponse> {
        let response = crate::openhuman::runtime::python_server::extract_spacy(&self.0, &text)
            .await
            .map_err(method_error)?;
        serde_json::from_value(serde_json::to_value(response).map_err(method_error)?)
            .map_err(method_error)
    }
}

fn method_error(error: impl std::fmt::Display) -> tinybus::Error {
    tinybus::Error::MethodFailed {
        name: "ai.tinyhumans.tinymemory.Error.Host".to_string(),
        message: error.to_string(),
    }
}

pub(super) async fn install(
    connection: &tinybus::Connection,
    config: Arc<Config>,
) -> tinybus::Result<()> {
    static INSTALLED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    INSTALLED
        .get_or_try_init(|| serve_interfaces(connection, config))
        .await
        .map(|_| ())
}

/// Serve every host interface on `connection` and claim its well-known name.
///
/// Split out of [`install`] so the served set can be exercised against a
/// throwaway broker. The latch there is process-wide by necessity — the module
/// runtime is a singleton — and a test that went through it would either be the
/// only one that could ever run or would silently assert against whichever
/// connection happened to win the race.
///
/// The claim order is serve-then-name for every interface: a name that resolves
/// before its object is exported would let a module land a call on a path that
/// is not there yet.
async fn serve_interfaces(
    connection: &tinybus::Connection,
    config: Arc<Config>,
) -> tinybus::Result<()> {
    connection
        .serve_at(
            ObjectPath::new(EMBEDDING_PATH)?,
            EmbeddingCallbacks(Arc::clone(&config)),
        )
        .await?;
    connection
        .serve_at(
            ObjectPath::new(CHAT_PATH)?,
            ChatCallbacks(Arc::clone(&config)),
        )
        .await?;
    connection
        .serve_at(
            ObjectPath::new(COMPOSIO_PATH)?,
            ComposioCallbacks(Arc::clone(&config)),
        )
        .await?;
    connection
        .serve_at(ObjectPath::new(RUNTIME_PATH)?, RuntimeCallbacks(config))
        .await?;
    connection.request_name(EMBEDDING_NAME).await?;
    connection.request_name(CHAT_NAME).await?;
    connection.request_name(COMPOSIO_NAME).await?;
    connection.request_name(RUNTIME_NAME).await
}

fn into_domain_event(event: MemoryEvent) -> Option<crate::core::events::DomainEvent> {
    use crate::core::events::DomainEvent;
    Some(match event {
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
        MemoryEvent::LocalModelUnavailable { origin } => {
            crate::openhuman::memory::tree::health::user_error::publish_local_model_unavailable_user_error(&origin);
            return None;
        }
        // Web-channel-only (openhuman#5820): the module's engine already
        // quarantined and rebuilt its store; the host's job is to make sure
        // the user durably hears it — this is the arm the incident lacked,
        // where a module-side quarantine was invisible to every host surface.
        MemoryEvent::StoreCorruptQuarantined {
            origin,
            quarantined_path,
        } => {
            crate::openhuman::memory::tree::health::user_error::publish_store_corrupt_user_error(
                &origin,
                quarantined_path.as_deref(),
            );
            return None;
        }
    })
}

#[cfg(test)]
#[path = "memory_host_tests.rs"]
mod tests;
