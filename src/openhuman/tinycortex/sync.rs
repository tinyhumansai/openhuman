//! OpenHuman service adapters for tinycortex live synchronization.

use async_trait::async_trait;
use tinycortex::memory::sync::{
    ClickUpSyncPipeline, ComposioClient, ExternalSourceReader, GitHubSyncPipeline,
    GithubRepoSyncPipeline, GmailSyncPipeline, LinearSyncPipeline, LocalDocument,
    LocalDocumentSink, NotionSyncPipeline, SkillDocSink, SkillDocument,
    SlackSearchBackfillPipeline, SlackSyncPipeline, SyncContext, SyncDispatcher, SyncEvent,
    SyncEventSink, SyncOutcome, SyncPipeline, SyncStage, SyncStateStore, WorkspaceSourcePipeline,
};

use crate::openhuman::config::Config;
use crate::openhuman::memory_sources::{MemorySourceEntry, SourceKind};
use crate::openhuman::memory_store::MemoryClientRef;

pub const HOST_SYNC_STATE_NAMESPACE: &str = "composio-sync-state";

pub struct HostSyncAdapter {
    memory: MemoryClientRef,
    config: Option<Config>,
}

#[derive(Debug)]
pub struct SourcePipelineFailure {
    pub message: String,
    pub actions_called: u32,
    pub provider_cost_usd: f64,
}

impl std::fmt::Display for SourcePipelineFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl SourcePipelineFailure {
    fn without_usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            actions_called: 0,
            provider_cost_usd: 0.0,
        }
    }
}

impl HostSyncAdapter {
    pub fn new(memory: MemoryClientRef) -> Self {
        Self {
            memory,
            config: None,
        }
    }

    fn with_config(memory: MemoryClientRef, config: Config) -> Self {
        Self {
            memory,
            config: Some(config),
        }
    }
}

#[async_trait]
impl ExternalSourceReader for HostSyncAdapter {
    async fn list_items(
        &self,
        source: &tinycortex::memory::sources::MemorySourceEntry,
    ) -> anyhow::Result<Vec<tinycortex::memory::sources::SourceItem>> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("external source reader requires host config"))?;
        let host_source: MemorySourceEntry = serde_json::from_value(serde_json::to_value(source)?)?;
        let reader = crate::openhuman::memory_sources::readers::reader_for(&host_source.kind);
        let items = reader
            .list_items(&host_source, config)
            .await
            .map_err(anyhow::Error::msg)?;
        serde_json::from_value(serde_json::to_value(items)?).map_err(Into::into)
    }

    async fn read_item(
        &self,
        source: &tinycortex::memory::sources::MemorySourceEntry,
        item_id: &str,
    ) -> anyhow::Result<tinycortex::memory::sources::SourceContent> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("external source reader requires host config"))?;
        let host_source: MemorySourceEntry = serde_json::from_value(serde_json::to_value(source)?)?;
        let reader = crate::openhuman::memory_sources::readers::reader_for(&host_source.kind);
        let content = reader
            .read_item(&host_source, item_id, config)
            .await
            .map_err(anyhow::Error::msg)?;
        serde_json::from_value(serde_json::to_value(content)?).map_err(Into::into)
    }
}

pub fn sync_context(memory: MemoryClientRef) -> SyncContext {
    let adapter = std::sync::Arc::new(HostSyncAdapter::new(memory));
    SyncContext {
        events: adapter.clone(),
        documents: adapter.clone(),
        state: adapter,
        local_documents: None,
        external_sources: None,
        summariser: None,
    }
}

fn source_sync_context(memory: MemoryClientRef, config: &Config, local: bool) -> SyncContext {
    let adapter = std::sync::Arc::new(HostSyncAdapter::with_config(memory, config.clone()));
    SyncContext {
        events: adapter.clone(),
        documents: adapter.clone(),
        state: adapter.clone(),
        local_documents: local.then(|| adapter.clone() as std::sync::Arc<dyn LocalDocumentSink>),
        external_sources: local.then_some(adapter as std::sync::Arc<dyn ExternalSourceReader>),
        summariser: local.then(|| {
            std::sync::Arc::new(super::HostSummariser::new(config.clone()))
                as std::sync::Arc<dyn tinycortex::memory::tree::Summariser>
        }),
    }
}

pub async fn run_source_pipeline(
    source: &MemorySourceEntry,
    config: &Config,
) -> Result<SyncOutcome, SourcePipelineFailure> {
    let memory = crate::openhuman::memory::global::client_if_ready()
        .ok_or_else(|| SourcePipelineFailure::without_usage("memory client is not ready"))?;
    let mut memory_config = super::memory_config_from(config, config.workspace_dir.clone());
    memory_config.sync.interval_secs = config.memory_sync_interval_secs;
    memory_config.sync.budget.max_items = source.max_items;
    memory_config.sync.budget.max_tokens_per_sync = source.max_tokens_per_sync;
    memory_config.sync.budget.max_cost_per_sync_usd = source.max_cost_per_sync_usd;
    memory_config.sync.budget.sync_depth_days = source.sync_depth_days;

    let pipeline = build_pipeline(source, config, &mut memory_config)
        .map_err(SourcePipelineFailure::without_usage)?;
    let pipeline_id = pipeline.id().to_owned();
    let mut dispatcher = SyncDispatcher::new();
    dispatcher
        .register(pipeline)
        .map_err(|error| SourcePipelineFailure::without_usage(error.to_string()))?;
    dispatcher
        .tick(
            &pipeline_id,
            &memory_config,
            &source_sync_context(memory, config, source.kind != SourceKind::Composio),
        )
        .await
        .map_err(|error| {
            let usage = error.downcast_ref::<tinycortex::memory::sync::SyncRunError>();
            SourcePipelineFailure {
                message: error.to_string(),
                actions_called: usage.map_or(0, |error| error.actions_called),
                provider_cost_usd: usage.map_or(0.0, |error| error.provider_cost_usd),
            }
        })
}

/// Run a Composio connection through tinycortex, preserving any source-level
/// budgets already configured in OpenHuman's registry.
pub async fn run_composio_connection(
    toolkit: &str,
    connection_id: &str,
    config: &Config,
) -> Result<SyncOutcome, SourcePipelineFailure> {
    run_composio_connection_with_budgets(toolkit, connection_id, config, None, None).await
}

/// Run a Composio connection with request-scoped budget overrides.
///
/// Provider RPCs carry these values in `ProviderContext`, before a source has
/// necessarily been persisted in the registry. Explicit values therefore take
/// precedence, while `None` preserves the registered/default source budget.
pub async fn run_composio_connection_with_budgets(
    toolkit: &str,
    connection_id: &str,
    config: &Config,
    max_items: Option<u32>,
    sync_depth_days: Option<u32>,
) -> Result<SyncOutcome, SourcePipelineFailure> {
    let mut source = config
        .memory_sources
        .iter()
        .find(|source| {
            source.kind == SourceKind::Composio
                && source.connection_id.as_deref() == Some(connection_id)
        })
        .cloned()
        .unwrap_or_else(|| {
            let (max_items, sync_depth_days) =
                crate::openhuman::memory_sources::memory_sync_defaults_for_toolkit(toolkit);
            MemorySourceEntry {
                id: format!("composio:{toolkit}:{connection_id}"),
                kind: SourceKind::Composio,
                label: format!("{toolkit} connection"),
                enabled: true,
                toolkit: Some(toolkit.to_ascii_lowercase()),
                connection_id: Some(connection_id.to_string()),
                path: None,
                glob: None,
                url: None,
                branch: None,
                paths: Vec::new(),
                max_commits: None,
                max_issues: None,
                max_prs: None,
                query: None,
                since_days: None,
                max_items,
                selector: None,
                max_tokens_per_sync: None,
                max_cost_per_sync_usd: None,
                sync_depth_days,
            }
        });

    source.max_items = max_items;
    source.sync_depth_days = sync_depth_days;

    tracing::debug!(
        toolkit,
        connection_id,
        source_id = %source.id,
        max_items = ?source.max_items,
        sync_depth_days = ?source.sync_depth_days,
        "[tinycortex:sync] dispatching Composio connection"
    );
    run_source_pipeline(&source, config).await
}

pub async fn load_composio_sync_state(
    toolkit: &str,
    connection_id: &str,
) -> anyhow::Result<tinycortex::memory::sync::SyncState> {
    let memory = crate::openhuman::memory::global::client_if_ready()
        .ok_or_else(|| anyhow::anyhow!("memory client is not ready"))?;
    let adapter = HostSyncAdapter::new(memory);
    tinycortex::memory::sync::SyncState::load(&adapter, toolkit, connection_id).await
}

pub async fn run_slack_search_backfill(
    connection_id: &str,
    backfill_days: i64,
    config: &Config,
) -> Result<SyncOutcome, SourcePipelineFailure> {
    let memory = crate::openhuman::memory::global::client_if_ready()
        .ok_or_else(|| SourcePipelineFailure::without_usage("memory client is not ready"))?;
    let mut memory_config = super::memory_config_from(config, config.workspace_dir.clone());
    let composio = composio_config(config).map_err(SourcePipelineFailure::without_usage)?;
    memory_config.sync.composio = Some(composio.clone());
    let pipeline = std::sync::Arc::new(SlackSearchBackfillPipeline::new(
        ComposioClient::new(composio),
        connection_id,
        backfill_days,
    ));
    let pipeline_id = pipeline.id().to_owned();
    let mut dispatcher = SyncDispatcher::new();
    dispatcher
        .register(pipeline)
        .map_err(|error| SourcePipelineFailure::without_usage(error.to_string()))?;
    dispatcher
        .tick(
            &pipeline_id,
            &memory_config,
            &source_sync_context(memory, config, false),
        )
        .await
        .map_err(|error| SourcePipelineFailure::without_usage(error.to_string()))
}

pub async fn run_gmail_backfill(
    connection_id: &str,
    query: &str,
    max_pages: usize,
    page_size: usize,
    config: &Config,
) -> Result<SyncOutcome, SourcePipelineFailure> {
    let memory = crate::openhuman::memory::global::client_if_ready()
        .ok_or_else(|| SourcePipelineFailure::without_usage("memory client is not ready"))?;
    let mut memory_config = super::memory_config_from(config, config.workspace_dir.clone());
    let composio = composio_config(config).map_err(SourcePipelineFailure::without_usage)?;
    memory_config.sync.composio = Some(composio.clone());
    let pipeline = std::sync::Arc::new(
        GmailSyncPipeline::new(ComposioClient::new(composio), connection_id)
            .with_limits(max_pages, page_size)
            .with_query(query),
    );
    let pipeline_id = pipeline.id().to_owned();
    let mut dispatcher = SyncDispatcher::new();
    dispatcher
        .register(pipeline)
        .map_err(|error| SourcePipelineFailure::without_usage(error.to_string()))?;
    dispatcher
        .tick(
            &pipeline_id,
            &memory_config,
            &source_sync_context(memory, config, false),
        )
        .await
        .map_err(|error| SourcePipelineFailure::without_usage(error.to_string()))
}

/// Composio toolkit slugs that have a native memory-sync pipeline in
/// [`build_pipeline`] — the authoritative "can actually ingest into memory" set.
///
/// This MUST stay in lockstep with the registered memory-sync providers
/// (`memory_sync::composio::all_composio_sync_providers`): the
/// `memory_sources.supported_toolkits` RPC advertises the provider registry, and
/// the `connection_created` auto-register gates on it, so any divergence
/// reintroduces the "connection reports ACTIVE but silently never ingests"
/// failure this guards against (#4957). The registry↔pipeline equality is pinned
/// by `composio_syncable_set_matches_provider_registry` in the tests below, and
/// the arms of the `match` in [`build_pipeline`] map 1:1 to these slugs.
pub fn syncable_composio_toolkits() -> &'static [&'static str] {
    &["clickup", "github", "gmail", "linear", "notion", "slack"]
}

/// Whether `toolkit` has a native memory-sync pipeline (case-insensitive).
/// Callers deciding *whether to offer/register* a Composio source should prefer
/// the provider registry (`get_composio_sync_provider`) so there is a single
/// advertised source of truth; this mirror exists for the sync layer itself.
pub fn is_composio_toolkit_syncable(toolkit: &str) -> bool {
    let slug = toolkit.trim().to_ascii_lowercase();
    syncable_composio_toolkits().contains(&slug.as_str())
}

fn build_pipeline(
    source: &MemorySourceEntry,
    config: &Config,
    memory_config: &mut tinycortex::memory::config::MemoryConfig,
) -> Result<std::sync::Arc<dyn SyncPipeline>, String> {
    if source.kind != SourceKind::Composio {
        let crate_source: tinycortex::memory::sources::MemorySourceEntry = serde_json::from_value(
            serde_json::to_value(source).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        if source.kind == SourceKind::GithubRepo {
            return GithubRepoSyncPipeline::new(crate_source)
                .map(|pipeline| std::sync::Arc::new(pipeline) as std::sync::Arc<dyn SyncPipeline>)
                .map_err(|error| error.to_string());
        }
        return WorkspaceSourcePipeline::new(crate_source)
            .map(|pipeline| std::sync::Arc::new(pipeline) as std::sync::Arc<dyn SyncPipeline>)
            .map_err(|error| error.to_string());
    }

    let toolkit = source
        .toolkit
        .as_deref()
        .map(str::trim)
        .filter(|toolkit| !toolkit.is_empty())
        .ok_or_else(|| "composio source missing toolkit".to_string())?
        .to_ascii_lowercase();
    let connection_id = source
        .connection_id
        .as_deref()
        .map(str::trim)
        .filter(|connection_id| !connection_id.is_empty())
        .ok_or_else(|| "composio source missing connection_id".to_string())?;
    // Fail closed *before* resolving credentials/client for any toolkit without a
    // native pipeline. This keeps the unsupported-toolkit error identical to the
    // match's fallback while making the syncable set a single, testable gate that
    // stays pinned to the provider registry (#4957).
    if !is_composio_toolkit_syncable(&toolkit) {
        return Err(format!(
            "tinycortex sync does not support toolkit '{toolkit}'"
        ));
    }
    let composio = composio_config(config)?;
    memory_config.sync.composio = Some(composio.clone());
    let client = ComposioClient::new(composio);
    let pipeline: std::sync::Arc<dyn SyncPipeline> = match toolkit.as_str() {
        "gmail" => std::sync::Arc::new(GmailSyncPipeline::new(client, connection_id)),
        "github" => std::sync::Arc::new(GitHubSyncPipeline::new(client, connection_id)),
        "notion" => std::sync::Arc::new(NotionSyncPipeline::new(client, connection_id)),
        "linear" => std::sync::Arc::new(LinearSyncPipeline::new(client, connection_id)),
        "clickup" => std::sync::Arc::new(ClickUpSyncPipeline::new(client, connection_id)),
        "slack" => std::sync::Arc::new(SlackSyncPipeline::new(client, connection_id)),
        _ => {
            return Err(format!(
                "tinycortex sync does not support toolkit '{toolkit}'"
            ))
        }
    };
    Ok(pipeline)
}

fn composio_config(
    config: &Config,
) -> Result<tinycortex::memory::config::ComposioSyncConfig, String> {
    use tinycortex::memory::config::{ComposioMode, ComposioSyncConfig, SecretString};

    if config.composio.mode.eq_ignore_ascii_case("direct") {
        let api_key = crate::openhuman::credentials::get_composio_api_key(config)?
            .or_else(|| config.composio.api_key.clone())
            .ok_or_else(|| "Composio direct API key is not configured".to_string())?;
        Ok(ComposioSyncConfig {
            mode: ComposioMode::Direct,
            base_url: "https://backend.composio.dev/api/v3".into(),
            api_key: Some(SecretString::new(api_key)),
            bearer_token: None,
            entity_id: Some(config.composio.entity_id.clone()),
        })
    } else {
        let bearer = crate::api::jwt::get_session_token(config)?
            .ok_or_else(|| "OpenHuman backend bearer token is not configured".to_string())?;
        Ok(ComposioSyncConfig {
            mode: ComposioMode::Proxied,
            base_url: crate::api::config::effective_backend_api_url(&config.api_url),
            api_key: None,
            bearer_token: Some(SecretString::new(bearer)),
            entity_id: Some(config.composio.entity_id.clone()),
        })
    }
}

#[async_trait]
impl SkillDocSink for HostSyncAdapter {
    async fn store(&self, document: SkillDocument) -> anyhow::Result<()> {
        tracing::debug!(
            toolkit = %document.toolkit,
            connection_id = %document.connection_id,
            document_id = %document.document_id,
            "[tinycortex:sync] storing synchronized document"
        );
        self.memory
            .store_skill_sync(
                &document.namespace_skill_id,
                &document.connection_id,
                &document.title,
                &document.content,
                Some("tinycortex-sync".into()),
                Some(document.metadata),
                Some("medium".into()),
                None,
                None,
                Some(document.document_id),
            )
            .await
            .map_err(anyhow::Error::msg)
    }

    async fn delete(&self, namespace_skill_id: &str, document_id: &str) -> anyhow::Result<()> {
        let namespace = format!("skill-{}", namespace_skill_id.trim());
        tracing::debug!(
            namespace,
            document_id,
            "[tinycortex:sync] deleting synchronized document"
        );
        self.memory
            .delete_document(&namespace, document_id)
            .await
            .map(|_| ())
            .map_err(anyhow::Error::msg)
    }
}

#[async_trait]
impl LocalDocumentSink for HostSyncAdapter {
    async fn upsert(&self, document: LocalDocument) -> anyhow::Result<()> {
        let config = self
            .config
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("local document sink missing host config"))?;
        let input = crate::openhuman::memory_sync::canonicalize::document::DocumentInput {
            provider: "memory_sources:local".into(),
            title: document.title,
            body: document.body,
            modified_at: document.modified_at,
            source_ref: document.source_ref,
        };
        crate::openhuman::memory::ingest_pipeline::ingest_document_with_scope(
            config,
            &document.source_id,
            &document.owner,
            document.tags,
            input,
            document.path_scope,
        )
        .await
        .map(|_| ())
        .map_err(anyhow::Error::msg)
    }

    async fn delete(&self, source_id: &str) -> anyhow::Result<()> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| anyhow::anyhow!("local document sink missing host config"))?;
        let source_id = source_id.to_owned();
        tokio::task::spawn_blocking(move || {
            crate::openhuman::memory_store::chunks::store::delete_chunks_by_source(
                &config,
                crate::openhuman::memory_store::chunks::types::SourceKind::Document,
                &source_id,
            )
        })
        .await
        .map_err(|error| anyhow::anyhow!("local delete task failed: {error}"))??;
        Ok(())
    }
}

#[async_trait]
impl SyncStateStore for HostSyncAdapter {
    async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<serde_json::Value>> {
        self.memory
            .kv_get(Some(namespace), key)
            .await
            .map_err(anyhow::Error::msg)
    }

    async fn set(
        &self,
        namespace: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> anyhow::Result<()> {
        self.memory
            .kv_set(Some(namespace), key, value)
            .await
            .map_err(anyhow::Error::msg)
    }
}

#[async_trait]
impl SyncEventSink for HostSyncAdapter {
    async fn emit(&self, event: SyncEvent) -> anyhow::Result<()> {
        crate::core::event_bus::publish_global(
            crate::core::event_bus::DomainEvent::MemorySyncStageChanged {
                trigger: "tinycortex".into(),
                stage: stage_name(event.stage).into(),
                provider: Some(event.toolkit),
                connection_id: event.connection_id,
                detail: event.message,
                source_id: Some(event.source_id),
            },
        );
        Ok(())
    }
}

fn stage_name(stage: SyncStage) -> &'static str {
    match stage {
        SyncStage::Requested => "requested",
        SyncStage::Fetching => "fetching",
        SyncStage::Stored => "stored",
        SyncStage::Ingesting => "ingesting",
        SyncStage::Completed => "completed",
        SyncStage::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::{build_pipeline, is_composio_toolkit_syncable, syncable_composio_toolkits};
    use crate::openhuman::config::Config;
    use crate::openhuman::memory_sources::MemorySourceEntry;
    use crate::openhuman::memory_sync::composio::{
        get_composio_sync_provider, init_default_composio_sync_providers,
    };

    /// The advertised set (`memory_sources.supported_toolkits`, sourced from the
    /// provider registry) and the syncable set (`build_pipeline`) must not
    /// diverge: a toolkit that is advertised but has no pipeline reports ACTIVE
    /// and then silently never ingests — the exact defect of #4957.
    ///
    /// Both directions are asserted against an explicit built-in slug set. The
    /// provider registry is process-global and sibling tests register throwaway
    /// providers into it without unregistering, so walking it directly would be
    /// order-flaky; pinning the built-in set keeps this deterministic.
    #[test]
    fn advertised_and_syncable_toolkit_sets_cannot_diverge() {
        init_default_composio_sync_providers();

        // Every syncable toolkit must have a registered provider — otherwise it
        // could never be advertised or auto-registered in the first place.
        for &slug in syncable_composio_toolkits() {
            assert!(
                get_composio_sync_provider(slug).is_some(),
                "syncable toolkit `{slug}` has no registered memory-sync provider"
            );
        }

        // Every built-in provider shipped by `init_default_composio_sync_providers`
        // must be syncable. This is the #4957 direction: advertising a provider
        // that `build_pipeline` rejects is the silent failure we guard against.
        //
        // We pin the built-in slug set explicitly rather than walking
        // `all_composio_sync_providers()`: that registry is process-global and
        // sibling tests register throwaway providers into it that they never
        // unregister (e.g. `provideronly` in composio/tools_tests.rs, `stub-no-active`
        // in composio/identity.rs), so a raw registry walk fails nondeterministically
        // depending on test execution order. A new built-in toolkit must be added to
        // this list, to `syncable_composio_toolkits`, and to `build_pipeline` together
        // — the assert_eq below fails loudly if the first two ever drift apart.
        const BUILTIN_SYNC_PROVIDERS: &[&str] =
            &["clickup", "github", "gmail", "linear", "notion", "slack"];

        let mut builtin = BUILTIN_SYNC_PROVIDERS.to_vec();
        builtin.sort_unstable();
        let mut syncable = syncable_composio_toolkits().to_vec();
        syncable.sort_unstable();
        assert_eq!(
            builtin, syncable,
            "the built-in provider set and syncable set diverged — a provider is \
             advertised without a matching `build_pipeline` arm, or vice versa (#4957)"
        );

        for &slug in BUILTIN_SYNC_PROVIDERS {
            assert!(
                get_composio_sync_provider(slug).is_some(),
                "built-in provider `{slug}` is not registered by \
                 init_default_composio_sync_providers"
            );
            assert!(
                is_composio_toolkit_syncable(slug),
                "built-in provider `{slug}` is advertised but has no build_pipeline arm — \
                 it would report ACTIVE and silently fail to sync (#4957)"
            );
        }
    }

    /// Behavioural regression for #4957: an unsupported Composio toolkit is
    /// rejected by `build_pipeline` *before* any credential/client resolution.
    ///
    /// We hand it a default `Config` (no Composio auth configured). If the gate
    /// ran AFTER config resolution we would get a config error ("backend bearer
    /// token is not configured" / "direct API key is not configured"); instead
    /// we must get the unsupported-toolkit error, proving the fail-closed
    /// ordering that stops an unsyncable toolkit from ever reaching a pipeline.
    #[test]
    fn build_pipeline_rejects_unsupported_toolkit_before_resolving_config() {
        // `googlecalendar` is a real Composio toolkit with no native pipeline —
        // exactly the prod case from #4957.
        let source: MemorySourceEntry = serde_json::from_value(serde_json::json!({
            "id": "composio:googlecalendar:conn-1",
            "kind": "composio",
            "label": "googlecalendar connection",
            "toolkit": "googlecalendar",
            "connection_id": "conn-1",
        }))
        .expect("construct composio source");

        let config = Config::default();
        let mut memory_config =
            tinycortex::memory::config::MemoryConfig::new("/tmp/openhuman-test-ws");

        // `build_pipeline` returns `Result<Arc<dyn SyncPipeline>, String>`; the
        // Ok arm is not `Debug`, so match rather than `expect_err`.
        let err = match build_pipeline(&source, &config, &mut memory_config) {
            Ok(_) => panic!("unsupported toolkit must be rejected before config resolution"),
            Err(e) => e,
        };
        assert!(
            err.contains("does not support toolkit 'googlecalendar'"),
            "expected the unsupported-toolkit error (proving rejection precedes \
             config resolution), got: {err}"
        );
    }

    /// Locks the reported prod failures (googlecalendar / googlesheets) as
    /// non-syncable, and pins case-insensitive/trimming behaviour.
    #[test]
    fn is_composio_toolkit_syncable_classifies_known_slugs() {
        assert!(!is_composio_toolkit_syncable("googlecalendar"));
        assert!(!is_composio_toolkit_syncable("googlesheets"));
        assert!(!is_composio_toolkit_syncable("discord"));
        assert!(!is_composio_toolkit_syncable(""));
        assert!(is_composio_toolkit_syncable("gmail"));
        assert!(is_composio_toolkit_syncable("Gmail"));
        assert!(is_composio_toolkit_syncable("  slack "));
    }
}
