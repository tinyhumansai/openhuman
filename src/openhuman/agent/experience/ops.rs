//! JSON-RPC handlers for the agent-experience store, and the adapter that lets
//! it read and write through the **bound memory driver** instead of an
//! in-process engine handle (openhuman#5560).

use serde::{Deserialize, Serialize};

use crate::openhuman::agent::experience::store::{
    retrieve_across_stores, AgentExperienceStore, ExperienceQuery,
};
use crate::openhuman::agent::experience::types::{AgentExperience, ExperienceHit};
use crate::openhuman::config::Config;
use crate::openhuman::memory::api::health::MemoryHealth;
// `MemoryCore` and `MemoryRecall` are deliberately NOT imported: their methods
// are reached on a `dyn MemoryProvider` receiver, where supertrait methods are
// inherent object candidates rather than in-scope-trait candidates — so an
// import of either would be flagged unused and fail `clippy -D warnings`.
use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::api::recall::OwnedRecallOpts;
use crate::openhuman::memory::api::types::{
    MemoryCategory, MemoryEntry, MemoryTaint, NamespaceSummary, RecallOpts,
};
use crate::openhuman::memory::Memory;
use crate::rpc::RpcOutcome;
use async_trait::async_trait;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The [`Memory`] view of a bound memory driver.
///
/// # Why this exists
///
/// [`AgentExperienceStore`] is built on `Arc<dyn Memory>` — the storage trait
/// `tinymemory-api` exports — and the driver contract has no door that hands one
/// out. Before #5560 the host closed that gap with the in-process engine's own
/// `MemoryClient::memory_handle()`, which meant every experience read and write
/// went through a **second** engine over the same SQLite file as the loaded
/// TinyMemory module. This adapter closes it the other way round: `Memory`'s
/// methods are a subset of what `MemoryCore` and `MemoryRecall` already
/// promise, so the whole trait can be served from the bound driver with nothing
/// added to the contract.
///
/// # It wraps the driver, not the guard, and that is deliberate
///
/// [`MemoryGuard`](crate::openhuman::memory::guard::MemoryGuard) truncates
/// `store` content at `capture_max_chars`, which
/// `MemoryHooksConfig::default()` sets to **500**. An `AgentExperience` is
/// stored as base64 of its serialized JSON — precisely so the free-text
/// scrubber cannot rewrite a Luhn-valid millisecond timestamp and corrupt the
/// payload (#5209) — and truncated base64 does not decode, so the record would
/// silently vanish on read. That is the same class of bug #5209 fixed, so the
/// pre-#5560 behaviour is preserved exactly: no policy layer between this store
/// and the driver. The store runs the full scrubber over its own free-text
/// fields before serialization (`store::redact_experience`), which is what
/// keeps that safe rather than merely unguarded.
///
/// # Home
///
/// This belongs in `src/openhuman/memory/`, next to `binding`, not under
/// `agent::experience` — it is contract-to-trait plumbing, not an
/// agent-experience concept. It sits here because both of its callers do
/// (`open_store_in_subdir` below, and the session builder's
/// `shared_experience_memory`).
pub struct DriverMemory {
    provider: Arc<dyn MemoryProvider>,
}

impl DriverMemory {
    /// Wrap an already-resolved driver.
    pub fn new(provider: Arc<dyn MemoryProvider>) -> Self {
        Self { provider }
    }

    /// The driver bound for `config`'s workspace and shared `memory` subtree.
    ///
    /// # Errors
    ///
    /// Only binding-cache lock poisoning: an inadmissible driver falls back to
    /// the null driver loudly rather than failing here (kernel.md §3.7).
    pub fn for_config(config: &Config) -> Result<Arc<dyn Memory>, String> {
        let binding = crate::openhuman::memory::binding::for_config(config)?;
        log::debug!(
            "[agent-experience] bound shared memory subtree driver='{}'",
            binding.driver_id()
        );
        let memory: Arc<dyn Memory> = Arc::new(Self::new(binding.provider().clone()));
        Ok(memory)
    }

    /// The driver bound for one memory subtree of `config`'s workspace.
    ///
    /// `"memory"` is the shared tree; `"memory-<id>"` is a profile that opted
    /// into dedicated memory. Each subtree is its own binding and therefore its
    /// own store, which is what makes `dedicatedMemory` isolation hold.
    ///
    /// # Errors
    ///
    /// As [`Self::for_config`].
    pub fn for_subtree(config: &Config, memory_subdir: &str) -> Result<Arc<dyn Memory>, String> {
        let binding = crate::openhuman::memory::binding::for_subtree(
            &config.workspace_dir,
            memory_subdir,
            &config.subsystems.memory,
        )?;
        log::debug!(
            "[agent-experience] bound memory subtree '{memory_subdir}' driver='{}'",
            binding.driver_id()
        );
        let memory: Arc<dyn Memory> = Arc::new(Self::new(binding.provider().clone()));
        Ok(memory)
    }
}

#[async_trait]
impl Memory for DriverMemory {
    /// The **driver's** id, not a synthetic name: this string reaches logs and
    /// status output, which should name whatever actually stores the bytes.
    fn name(&self) -> &str {
        self.provider.driver_id()
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        Ok(self
            .provider
            .store(
                namespace,
                key,
                content,
                category,
                session_id,
                MemoryTaint::Internal,
            )
            .await?)
    }

    /// Overridden rather than inherited. The trait's default *bails* for any
    /// taint other than `Internal`, so inheriting it would turn a sync path's
    /// `ExternalSync` write into an error against a driver that records taint
    /// perfectly well.
    async fn store_with_taint(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> anyhow::Result<()> {
        Ok(self
            .provider
            .store(namespace, key, content, category, session_id, taint)
            .await?)
    }

    /// `scope` is passed explicitly, never left to ambient state.
    ///
    /// The per-turn source allowlist is a `tokio::task_local` and task-locals
    /// do not cross the bus — a module reads an unset one as *unrestricted*, so
    /// a call that relied on the driver picking it up would fail open. Rendering
    /// it host-side with `source_scope::as_bus_scope()` is the rule the whole
    /// memory seam follows.
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let owned: OwnedRecallOpts = opts.into();
        let scope = crate::openhuman::memory::source_scope::as_bus_scope();
        Ok(self
            .provider
            .recall(query, limit, &owned, scope.as_ref())
            .await?)
    }

    /// Situational preferences (Lane B) reach memory through **this** adapter:
    /// the session's `Arc<dyn Memory>` is a `DriverMemory` on every
    /// module-backed install. This used to sit on the trait's default — empty,
    /// "the documented opt-out for a backend that cannot answer it" — on the
    /// belief that Lane B was reached through some other handle. It was not,
    /// so the lane silently injected nothing for everyone (#6041).
    ///
    /// The contract has no vector-threshold member; the body is the guard
    /// path's, over `MemoryRetrieval::recall_namespace_scored`, filtered on the
    /// vector component. A driver without the retrieval family still answers
    /// empty; a driver that has it and fails answers `Err`, which Lane B reads
    /// as "no block" rather than as a broken turn.
    async fn recall_relevant_by_vector(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
        min_vector_similarity: f64,
    ) -> anyhow::Result<Vec<(String, String)>> {
        Ok(
            crate::openhuman::memory::preferences::recall_by_vector_over(
                self.provider.as_ref(),
                namespace,
                query,
                limit,
                min_vector_similarity,
            )
            .await?,
        )
    }

    async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        Ok(self.provider.get(namespace, key).await?)
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(self.provider.list(namespace, category, session_id).await?)
    }

    async fn forget(&self, namespace: &str, key: &str) -> anyhow::Result<bool> {
        Ok(self.provider.forget(namespace, key).await?)
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        Ok(self.provider.namespaces().await?)
    }

    /// Summed from the per-namespace counts the driver already reports, rather
    /// than from `list(None, None, None).len()`: the two answer the same
    /// question and only one of them pulls every entry's content across the bus
    /// to do it.
    async fn count(&self) -> anyhow::Result<usize> {
        let summaries = self.provider.namespaces().await?;
        Ok(summaries.iter().map(|summary| summary.count).sum())
    }

    /// `Degraded` counts as healthy here because the trait asks whether the
    /// backend is "reachable and able to serve requests" — a degraded driver
    /// is both. Only `Down` is false.
    async fn health_check(&self) -> bool {
        !matches!(self.provider.health().await, MemoryHealth::Down { .. })
    }

    /// The typed answer, so callers get the driver's own reason string instead
    /// of falling back to the boolean above.
    async fn health_probe(&self) -> Option<MemoryHealth> {
        Some(self.provider.health().await)
    }
}

#[derive(Debug, Deserialize)]
pub struct CaptureParams {
    pub experience: AgentExperience,
}

#[derive(Debug, Deserialize, Default)]
pub struct RetrieveParams {
    pub query: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub entrypoint: Option<String>,
    /// Profile partition filter (1c). `None` (omitted) recalls the whole pool;
    /// `Some(P)` recalls records stamped `P` plus unstamped legacy records.
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub max_hits: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListParams {
    /// Profile partition filter (1c), same semantics as `RetrieveParams`.
    #[serde(default)]
    pub profile_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DismissParams {
    pub id: String,
    #[serde(default)]
    pub profile_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DismissResult {
    pub id: String,
    pub dismissed: bool,
}

fn profile_memory_subdir(
    workspace_dir: &std::path::Path,
    profile_id: Option<&str>,
) -> Result<String, String> {
    let Some(profile_id) = profile_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok("memory".to_string());
    };
    let state = crate::openhuman::agent::profiles::load_profiles(workspace_dir)?;
    let profile = state
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("agent profile '{profile_id}' not found"))?;
    let suffix = crate::openhuman::agent::profiles::effective_memory_suffix(profile);
    Ok(crate::openhuman::agent::profiles::memory_subdir_for_suffix(
        &suffix,
    ))
}

/// The experience store for `profile_id`'s memory subtree.
///
/// Both arms used to resolve the in-process engine — `global::client_if_ready()`
/// with a `global::init` fallback for the shared tree, a freshly constructed
/// `UnifiedMemory` for a dedicated one. Neither exists any more (#5560): both
/// arms now go through [`DriverMemory::for_subtree`], with `"memory"` naming the
/// shared tree. That is the same workspace-and-subtree binding key the session
/// builder already resolves for the archivist, so the two cannot end up on
/// different stores for one profile.
async fn open_store(profile_id: Option<&str>) -> Result<AgentExperienceStore, String> {
    let profile_id = profile_id.map(str::trim).filter(|id| !id.is_empty());
    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    if profile_id.is_none() {
        return open_store_in_subdir(&config, "memory").await;
    }

    let memory_subdir = profile_memory_subdir(&config.workspace_dir, profile_id)?;

    open_store_in_subdir(&config, &memory_subdir).await
}

/// The experience store over one named memory subtree of `config`'s workspace.
///
/// # The embedder moved with the store, and that is the intended delta
///
/// The dedicated-subtree arm used to build its own `UnifiedMemory` with a
/// config-scoped embedding provider, so that the experience store's managed
/// embedder read the signed-in user's session rather than the keyless
/// `default_state_dir()` scope (#5501). The bound driver embeds with the
/// embedder the module policy publishes — which is that same user-scoped one,
/// resolved once at boot instead of per store — so #5501's fix survives the
/// move. What is gone is this call site's ability to choose a *different*
/// embedder from the rest of the subsystem, which was never the point.
async fn open_store_in_subdir(
    config: &Config,
    memory_subdir: &str,
) -> Result<AgentExperienceStore, String> {
    let memory = DriverMemory::for_subtree(config, memory_subdir)
        .map_err(|e| format!("open agent experience store '{memory_subdir}': {e}"))?;
    Ok(AgentExperienceStore::new(memory))
}

fn query_memory_subdirs(
    workspace_dir: &std::path::Path,
    profile_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let state = crate::openhuman::agent::profiles::load_profiles(workspace_dir)?;
    let mut subdirs = BTreeSet::from(["memory".to_string()]);
    let profile_id = profile_id.map(str::trim).filter(|id| !id.is_empty());

    for profile in &state.profiles {
        if profile_id.is_none_or(|id| profile.id == id) {
            let suffix = crate::openhuman::agent::profiles::effective_memory_suffix(profile);
            subdirs.insert(crate::openhuman::agent::profiles::memory_subdir_for_suffix(
                &suffix,
            ));
        }
    }
    if let Some(profile_id) = profile_id {
        if !state
            .profiles
            .iter()
            .any(|profile| profile.id == profile_id)
        {
            return Err(format!("agent profile '{profile_id}' not found"));
        }
    }
    Ok(subdirs.into_iter().collect())
}

async fn open_query_stores(profile_id: Option<&str>) -> Result<Vec<AgentExperienceStore>, String> {
    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    let subdirs = query_memory_subdirs(&config.workspace_dir, profile_id)?;
    let mut stores = Vec::with_capacity(subdirs.len());
    for subdir in subdirs {
        stores.push(open_store_in_subdir(&config, &subdir).await?);
    }
    Ok(stores)
}

pub async fn capture(params: CaptureParams) -> Result<RpcOutcome<AgentExperience>, String> {
    let store = open_store(params.experience.profile_id.as_deref()).await?;
    let stored = store.put(params.experience).await?;
    Ok(RpcOutcome::single_log(stored, "agent experience captured"))
}

pub async fn retrieve(params: RetrieveParams) -> Result<RpcOutcome<Vec<ExperienceHit>>, String> {
    let stores = open_query_stores(params.profile_id.as_deref()).await?;
    let max_hits = params.max_hits.unwrap_or(5);
    let query = ExperienceQuery {
        query: params.query,
        tools: params.tools,
        tags: params.tags,
        agent_id: params.agent_id,
        entrypoint: params.entrypoint,
        profile_id: params.profile_id,
        max_hits,
    };
    let hits = retrieve_across_stores(&stores, query).await?;
    Ok(RpcOutcome::single_log(hits, "agent experiences retrieved"))
}

pub async fn list(params: ListParams) -> Result<RpcOutcome<Vec<AgentExperience>>, String> {
    let stores = open_query_stores(params.profile_id.as_deref()).await?;
    let mut by_id: BTreeMap<String, AgentExperience> = BTreeMap::new();
    for store in stores {
        for experience in store.list_for_profile(params.profile_id.as_deref()).await? {
            let id = experience.id.clone();
            match by_id.get(&id) {
                Some(existing) if existing.updated_at_ms >= experience.updated_at_ms => {}
                _ => {
                    by_id.insert(id, experience);
                }
            }
        }
    }
    let mut experiences: Vec<_> = by_id.into_values().collect();
    experiences.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(RpcOutcome::single_log(
        experiences,
        "agent experiences listed",
    ))
}

pub async fn dismiss(params: DismissParams) -> Result<RpcOutcome<DismissResult>, String> {
    let stores = open_query_stores(params.profile_id.as_deref()).await?;
    let mut dismissed = false;
    for store in stores {
        dismissed |= store
            .dismiss_for_profile(&params.id, params.profile_id.as_deref())
            .await?;
    }
    Ok(RpcOutcome::single_log(
        DismissResult {
            id: params.id,
            dismissed,
        },
        "agent experience dismissed",
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
