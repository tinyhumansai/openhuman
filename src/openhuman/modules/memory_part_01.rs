use std::sync::Arc;

use tinymemory_api::capabilities::{Capabilities, Capability};

/// The release whose capability set [`ARTIFACT_CAPABILITIES`] was read from.
///
/// Checked against the registry pin by `the_capability_list_matches_the_pinned_release`,
/// so bumping the pin without re-reading the list is a red test rather than a
/// silent over-claim.
pub(crate) const ARTIFACT_CAPABILITIES_PIN: &str = "1.13.6";

/// The capability families the **pinned artifact** actually serves.
///
/// Deliberately not `Capabilities::all()`. `Capability::ALL` is what the
/// *contract crate this host compiles against* declares; the loaded `cdylib` is
/// a specific release and may serve fewer families.
///
/// Re-read at tag `v1.13.3`. v1.13.0 added a `MemoryEvent` variant and two
/// additive audit fields, v1.13.1 fixed the module's source-registry path,
/// v1.13.2 fixed the `Embed` wire order, and v1.13.3 fixed folder-source path
/// resolution; none of those touched families. tinymemory#110 (in v1.13.2)
/// did add `Scoring`
/// (`ExtractEntities`, `EmbedText`, `EmbedderSlug`), which the artifact serves
/// and which `as_scoring` below forwards, so it is advertised here in the same
/// change, the way `Episodic` arrived with `as_episodic`.
///
/// Read at tag `v1.3.0`. Unchanged from v1.2.0 — the release added members
/// within existing families (`retry_failed`, the diagnostics trio,
/// `backfill_in_progress`), not families — verified with
/// `git diff v1.2.0..v1.3.0 -- crates/tinymemory-api/src/capabilities.rs`
/// returning empty. v1.2.0 is where four of the five families that v1.0.1
/// lacked arrived: `People`, `Chunks`, `Retrieval` and `Profile` all have bus
/// members there, so the under-claim that made them unreachable is over.
///
/// **`Episodic` is here in the same change that implements `as_episodic`**, as
/// the previous version of this comment required. The pinned module declares
/// the episodic methods (`InsertTurn`, `SessionTurns`, `OpenSegment`, …) and
/// [`ModuleMemoryProvider`] now forwards all of them, so the advertisement is
/// honest in both directions — the archivist writes its turns and segments
/// through this family.
///
/// **Widen this only together with the `version` bump in
/// [`super::registry`].** `the_capability_list_matches_the_pinned_release`
/// fails if the two drift.
pub(crate) const ARTIFACT_CAPABILITIES: &[Capability] = &[
    Capability::Core,
    Capability::Recall,
    Capability::Ingest,
    Capability::Documents,
    Capability::Tree,
    Capability::Entities,
    Capability::Graph,
    Capability::Diff,
    Capability::Goals,
    Capability::ToolMemory,
    Capability::Sources,
    Capability::Maintenance,
    Capability::Portability,
    // Arrived in v1.2.0. Verified against the module's declared `methods` list
    // at that tag rather than against the contract crate, which is always ahead
    // of whatever is pinned.
    Capability::People,
    Capability::Chunks,
    Capability::Retrieval,
    Capability::Profile,
    Capability::Episodic,
    // Arrived in v1.7.0 — the sync-execution and coding-session families that
    // let the host stop reaching into the engine for them. Verified against the
    // module's declared `methods` list at that tag, which serves all ten.
    Capability::SourceSync,
    Capability::CodingSessions,
    // Arrived in v1.13.2 (tinymemory#110): entity extraction, text embedding
    // and embedder identification, served by the module's engine and forwarded
    // by `MemoryScoring for ModuleMemoryProvider` below.
    Capability::Scoring,
];

/// Escape hatch for a locally-built module.
///
/// Set `OPENHUMAN_MEMORY_MODULE_ASSUME_FULL_CAPABILITIES=1` when the loaded
/// library was built from `vendor/tinymemory/crates/tinymemory-module` rather
/// than downloaded from the pinned release — that build serves the whole
/// contract, and pinning it to the older list would hide families it does have.
/// Deliberately **not** keyed off `TINYMEMORY_TEST_MODULE`: CI sets that to the
/// downloaded `v1.0.1` artifact, so keying off it would switch the guard off in
/// exactly the lane that must exercise it.
fn assume_full_capabilities() -> bool {
    matches!(
        std::env::var("OPENHUMAN_MEMORY_MODULE_ASSUME_FULL_CAPABILITIES")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// The set this build will advertise for the module driver.
fn artifact_capabilities() -> Capabilities {
    capabilities_for(assume_full_capabilities())
}

/// The advertised set for a given override state.
///
/// Split out from [`artifact_capabilities`] so the pinned-artifact invariants
/// can be asserted on the `false` branch directly. Reading the environment
/// inside the assertion would make those tests fail for anyone who has
/// `OPENHUMAN_MEMORY_MODULE_ASSUME_FULL_CAPABILITIES=1` exported — a documented,
/// supported configuration — and mutating the variable from a test would race
/// the rest of the binary.
fn capabilities_for(assume_full: bool) -> Capabilities {
    if assume_full {
        return Capabilities::all();
    }
    ARTIFACT_CAPABILITIES.iter().copied().collect()
}

/// Whether the pinned artifact serves `capability`. Drives the optional
/// `as_*()` accessors so they agree with [`artifact_capabilities`].
fn artifact_serves(capability: Capability) -> bool {
    assume_full_capabilities() || ARTIFACT_CAPABILITIES.contains(&capability)
}
use async_trait::async_trait;
use tinymemory_api::chunks::Chunk;
use tinymemory_api::error::MemoryError;
use tinymemory_api::goals::GoalsDoc;
use tinymemory_api::health::MemoryHealth;
use tinymemory_api::provider::sessions::{
    CodingSessionIngestReport, CodingSessionIngestRequest, CodingSessionSource,
};
use tinymemory_api::provider::sync::{
    RawArchiveCoverage, RawRebuildOutcome, SourceSyncState, SourceSyncStatus, SyncAuditEntry,
    SyncRunOutcome,
};
use tinymemory_api::provider::types::{
    ChunkEntityOccurrence, DiffReport, EntityHit, EntityOccurrence, ExportPage, ExportRecord,
    FlushOutcome, ForgetOutcome, ForgetSelector, ImportOutcome, IngestItem, IngestOutcome,
    MaintenanceReport, PurgeOutcome, QueueFailure, QueueStats, ResetOutcome, SnapshotRef,
    SourceItem, SourceScope, StoreStats,
};
use tinymemory_api::provider::{
    AddressBookSeedOutcome, ChunkDetail, ChunkEmbedding, ChunkListRow, ChunkQuery, ChunkScore,
    ConversationSegment, CoverWindowQuery, DegradedCapabilities, Diagnosis, EntityMatch,
    EpisodicEvent, EpisodicTurn, FacetType, FastRetrieveQuery, MemoryChunks, MemoryCodingSessions,
    MemoryCore, MemoryDiff, MemoryDocuments, MemoryEntities, MemoryEpisodic, MemoryGoals,
    MemoryGraph, MemoryIngest, MemoryMaintenance, MemoryPeople, MemoryPortability, MemoryProfile,
    MemoryProvider, MemoryRecall, MemoryRetrieval, MemoryScoring, MemorySourceSink,
    MemorySourceSync, MemoryToolMemory, MemoryTree, PersonHandle, PersonInteraction, PersonRecord,
    PersonScore, ProfileFacet, RankedPerson, ResolvedPerson, RetrievalHit, RetrievalResponse,
    RootSummary, SourceIngestQuery, SourceIngestStatus, SourceRetrievalQuery, SourceTotal,
    SummaryContext, SummaryInput, SummaryOutput, UserState,
};
use tinymemory_api::recall::OwnedRecallOpts;
use tinymemory_api::tool_memory::ToolMemoryRule;
use tinymemory_api::tree::{
    IngestRequest, QueryResult, SummaryForest, TreeLeaf, TreeNode, TreeStatus,
};
use tinymemory_api::types::{
    GraphRelationRecord, MemoryCategory, MemoryEntry, MemoryKvRecord, MemoryTaint,
    NamespaceDocumentInput, NamespaceMemoryHit, NamespaceRetrievalContext, NamespaceSummary,
    StoredMemoryDocument,
};
use tinymemory_api::wire;
use tinymemory_bus::names::methods;

use super::{host, ops, registry};
use crate::openhuman::config::Config;

/// Registry id of the module these calls go to.
pub const MODULE_ID: &str = "tinymemory";

/// The `[modules]` policy this process was booted with.
///
/// # Why a process-global and not a constructor argument
///
/// `memory::binding::build` is where a module driver is constructed, and it
/// receives only a workspace dir and a `MemorySubsystemConfig`. What
/// [`ops::ensure_loaded`] needs is `modules.{enabled, allow_download,
/// install_dir}`, which lives on the full `Config` — and threading a whole
/// `Config` down through `MemoryBinding::for_workspace` would widen that
/// function's dependency and change a cache key that ~4000 pre-boot tests hit.
///
/// So the policy is published once during boot instead. This is the same shape
/// `tinymemory_core::embedding_host` and `api::product` already use, and for the
/// same stated reason: the construction sites sit too deep to thread through.
///
/// # Unset means disabled, deliberately
///
/// A pre-boot test, or a host that never called [`set_modules_policy`], gets
/// `None` — and [`policy`] then reports modules disabled rather than assuming
/// permissive defaults. Defaulting `enabled` to `true` here would silently
/// ignore an operator who turned modules off, and would let a unit test reach
/// for a download.
static MODULES_POLICY: std::sync::OnceLock<Arc<Config>> = std::sync::OnceLock::new();

/// Publish the config a module driver should load against.
///
/// Call once during boot, before any workspace is bound. Later calls are
/// ignored — a driver already resolved against the first value must not have the
/// policy change underneath it.
pub fn set_modules_policy(config: Arc<Config>) {
    let _ = MODULES_POLICY.set(config);
}

/// The published policy, if boot supplied one.
pub(crate) fn policy() -> Option<&'static Arc<Config>> {
    MODULES_POLICY.get()
}

/// A memory driver served by the loaded `tinymemory` module.
pub struct ModuleMemoryProvider {
    /// The id reported by [`MemoryProvider::driver_id`].
    driver_id: String,
    /// The config to load against, when the caller had one to give.
    ///
    /// `None` is the binding-site case: `build` has no `Config`, so the provider
    /// falls back to the policy published at boot. Tests pass one explicitly.
    config: Option<Arc<Config>>,
    /// Set once the module has answered `Capabilities`, so the cross-check runs
    /// once rather than per call.
    verified: std::sync::OnceLock<()>,
    /// Memory subtree this driver is bound to, when it is not the shared one.
    ///
    /// `None` means `<workspace>/memory` — the root object the module serves
    /// eagerly at setup. `Some("memory-<id>")` is a profile that opted into
    /// dedicated memory; the first call asks the root object to open it and
    /// caches the object path it answers with.
    memory_subdir: Option<String>,
    /// Object path resolved for [`Self::memory_subdir`], once asked for.
    resolved_path: tokio::sync::OnceCell<String>,
}

impl std::fmt::Debug for ModuleMemoryProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `Config` is not rendered: it carries credentials.
        f.debug_struct("ModuleMemoryProvider")
            .field("driver_id", &self.driver_id)
            .finish_non_exhaustive()
    }
}

impl ModuleMemoryProvider {
    /// Bind the module-backed driver.
    ///
    /// Synchronous and I/O-free by requirement — see the module docs. Nothing is
    /// loaded until the first call.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self::with_optional_config(Some(config))
    }

    /// Bind against the policy published by [`set_modules_policy`].
    ///
    /// This is what `memory::binding::build` uses, because it has no `Config` to
    /// hand over. If boot published nothing, every call reports the module
    /// unavailable rather than guessing a permissive default.
    #[must_use]
    pub fn from_boot_policy() -> Self {
        Self::with_optional_config(None)
    }

    fn with_optional_config(config: Option<Arc<Config>>) -> Self {
        Self {
            driver_id: registry::find(MODULE_ID)
                .map_or_else(|| MODULE_ID.to_string(), |record| record.id.to_string()),
            config,
            verified: std::sync::OnceLock::new(),
            memory_subdir: None,
            resolved_path: tokio::sync::OnceCell::new(),
        }
    }

    /// Bind this driver to a named memory subtree rather than the shared one.
    ///
    /// `"memory"` is the shared tree and is treated as `None`, so a caller can
    /// pass whatever `memory_subdir_for_suffix` produced without special-casing
    /// the default.
    #[must_use]
    pub fn in_subdir(mut self, memory_subdir: &str) -> Self {
        if memory_subdir != "memory" && !memory_subdir.is_empty() {
            self.memory_subdir = Some(memory_subdir.to_string());
        }
        self
    }

    /// The object path this driver talks to, opening the subtree on first use.
    ///
    /// The root object is served eagerly at module setup, so the shared tree
    /// costs nothing here. A dedicated subtree is opened once and cached; the
    /// module is idempotent per subtree, so a lost race re-uses the same store
    /// rather than opening the database twice.
    async fn object_path(&self, proxy_root: &tinybus::Proxy) -> Result<String, MemoryError> {
        let record = registry::find(MODULE_ID)
            .ok_or_else(|| MemoryError::Other(anyhow::anyhow!("unknown module '{MODULE_ID}'")))?;
        let Some(subdir) = self.memory_subdir.as_deref() else {
            return Ok(record.object_path.to_string());
        };
        self.resolved_path
            .get_or_try_init(|| async {
                log::debug!("[modules:memory] opening a dedicated memory subtree");
                proxy_root
                    .call::<String>("OpenStore", (subdir.to_string(),))
                    .await
                    .map_err(|error| from_bus(&error))
            })
            .await
            .cloned()
    }

    /// Ensure the module is serving, and hand back a proxy for its object.
    ///
    /// `operation` identifies the forwarded call (e.g. `"store"`, `"recall"`)
    /// for the diagnostic below. Never `namespace`, `key`, `content`, or any
    /// record value — those are user memory content, not correlation fields.
    async fn proxy(&self, operation: &str) -> Result<tinybus::Proxy, MemoryError> {
        log::debug!(
            "[modules:memory] driver_id={} operation={operation} resolving module proxy",
            self.driver_id,
        );
        let config = self.config.as_ref().or_else(|| policy()).ok_or_else(|| {
            MemoryError::Other(anyhow::anyhow!(
                "the module host policy was never published, so module '{MODULE_ID}' \
                 cannot be loaded; call modules::memory::set_modules_policy during boot"
            ))
        })?;
        let runtime = host::runtime().await.map_err(|error| {
            MemoryError::Other(anyhow::anyhow!("the module bus is not running: {error}"))
        })?;
        super::memory_host::install(runtime.connection(), Arc::clone(config))
            .await
            .map_err(|error| {
                MemoryError::Other(anyhow::anyhow!(
                    "the memory module host callbacks are unavailable: {error}"
                ))
            })?;
        // TinyMemory resolves its embedding provider while the native library
        // is admitted. Host callbacks must therefore exist before loading,
        // including in tests and explicit-path overrides where no boot policy
        // was available when the shared module runtime first started.
        // A load failure is terminal for the process (the loader caches it),
        // so every memory member would otherwise return the loader's raw
        // message — release URLs, digest text, "restart the app" repeated per
        // call. Map it once into the subsystem's honest degraded state: a
        // user_error broadcast (once per process, metadata only) plus a
        // stable, actionable error for the caller. The raw reason goes to the
        // log, where an operator can act on it.
        ops::ensure_loaded(config, MODULE_ID).await.map_err(|message| {
            crate::openhuman::memory::tree::health::user_error::notice_memory_module_unavailable_once(
                &message,
            );
            MemoryError::Backend(
                "memory is unavailable: the memory module failed to load. Restart the app to                  retry; the reason is in the log."
                    .to_string(),
            )
        })?;

        let record = registry::find(MODULE_ID)
            .ok_or_else(|| MemoryError::Other(anyhow::anyhow!("unknown module '{MODULE_ID}'")))?;
        let root = runtime
            .proxy(record.bus_name, record.object_path)
            .map_err(|error| MemoryError::Other(anyhow::anyhow!(error.to_string())))?;

        self.verify(&root).await;

        // The shared tree is the root object itself, so this is a no-op for
        // every caller that did not ask for a dedicated subtree.
        let path = self.object_path(&root).await?;
        if path == record.object_path {
            return Ok(root);
        }
        runtime
            .proxy(record.bus_name, &path)
            .map_err(|error| MemoryError::Other(anyhow::anyhow!(error.to_string())))
    }

    /// Cross-check the module's advertised capabilities against what this build
    /// assumes, once per process.
    ///
    /// Compared against [`artifact_capabilities`] rather than
    /// `Capabilities::all()`: the pinned artifact answers fewer families than the
    /// contract declares (seventeen of eighteen at v1.2.0), so comparing with the
    /// full contract would warn
    /// on the *expected* state at every first module use and leave the warning
    /// permanently crying wolf. Against the configured set it fires only when
    /// the loaded artifact genuinely disagrees with the pin — including when the
    /// full-capability override is on but an older artifact was loaded.
    ///
    /// Logged rather than fatal. A mismatch means the registry pin and the
    /// artifact have diverged; the module advertising *less* than this build
    /// assumes is the dangerous direction, because the host assembles its memory
    /// RPC surface and tool families from the assumed set before any call.
    async fn verify(&self, proxy: &tinybus::Proxy) {
        if self.verified.get().is_some() {
            return;
        }
        match proxy.call::<Capabilities>("Capabilities", ()).await {
            Ok(actual) => {
                let assumed = artifact_capabilities();
                if actual != assumed {
                    log::warn!(
                        "[modules:memory] the module advertises {actual:?} but this build \
                         assumes {assumed:?}; the registry pin and the artifact have diverged"
                    );
                }
            }
            Err(error) => {
                log::warn!("[modules:memory] could not read module capabilities: {error}");
            }
        }
        let _ = self.verified.set(());
    }
}

/// Map a bus failure back onto a [`MemoryError`].
///
/// Uses the shared table so the host and the module cannot disagree about what a
/// name means. An unrecognised name becomes `Other`, never `Invalid`.
fn from_bus(error: &tinybus::Error) -> MemoryError {
    wire::from_wire(error.wire_name(), &error.to_string())
}

#[async_trait]
impl MemoryProvider for ModuleMemoryProvider {
    fn driver_id(&self) -> &str {
        &self.driver_id
    }

    /// The families the **pinned artifact** serves — not the whole contract.
    ///
    /// # This couples to the registry pin, and the coupling IS enforced
    ///
    /// `Capabilities::all()` grows whenever a family is added to the contract,
    /// but the *artifact* only grows when a release is cut and
    /// [`registry`](super::registry) is re-pinned to it. Returning `all()`
    /// between those two moments over-claimed: the host said it could do
    /// something the loaded binary could not, and [`Self::verify`] noticed and
    /// logged it without narrowing the advertised set. The failure mode was a
    /// call that reached the module and came back `UnknownMethod` (#5598)
    /// rather than a family that cleanly reported itself absent.
    ///
    /// [`ARTIFACT_CAPABILITIES`] is now the source of truth, and
    /// `the_capability_list_matches_the_pinned_release` fails if it is widened
    /// without moving [`ARTIFACT_CAPABILITIES_PIN`] and the registry pin
    /// together.
    ///
    /// The kernel filters its RPC surface and agent-tool assembly from this set,
    /// and the guard builds one family decorator per `provides()`, so an
    /// over-claim here is precisely what turns an absent family into a live
    /// method that answers `UnknownMethod`.
    fn capabilities(&self) -> Capabilities {
        artifact_capabilities()
    }

    async fn health(&self) -> MemoryHealth {
        // An unreachable module is a *health* answer, not an error: that is the
        // question this method exists to answer, and returning `Down` is how
        // status output shows an unsupported platform or a refused artifact.
        match self.proxy("health").await {
            Ok(proxy) => proxy
                .call::<MemoryHealth>("Health", ())
                .await
                .unwrap_or_else(|error| MemoryHealth::down(error.to_string())),
            Err(error) => MemoryHealth::down(error.to_string()),
        }
    }

    async fn shutdown(&self) -> Result<(), MemoryError> {
        // Deliberately does not load the module in order to shut it down: a
        // shutdown on a driver that was never used should be a no-op, not a
        // download. tinybus never unloads a library anyway, so this releases
        // backend resources only.
        if self.verified.get().is_none() {
            return Ok(());
        }
        let proxy = self.proxy("shutdown").await?;
        proxy
            .call::<()>(methods::SHUTDOWN, ())
            .await
            .map_err(|error| from_bus(&error))
    }

    fn as_ingest(&self) -> Option<&dyn MemoryIngest> {
        Some(self)
    }
    fn as_documents(&self) -> Option<&dyn MemoryDocuments> {
        Some(self)
    }
    fn as_tree(&self) -> Option<&dyn MemoryTree> {
        Some(self)
    }
    fn as_entities(&self) -> Option<&dyn MemoryEntities> {
        Some(self)
    }
    fn as_graph(&self) -> Option<&dyn MemoryGraph> {
        Some(self)
    }
    fn as_diff(&self) -> Option<&dyn MemoryDiff> {
        Some(self)
    }
    fn as_goals(&self) -> Option<&dyn MemoryGoals> {
        Some(self)
    }
    fn as_tool_memory(&self) -> Option<&dyn MemoryToolMemory> {
        Some(self)
    }
    fn as_sources(&self) -> Option<&dyn MemorySourceSink> {
        Some(self)
    }
    fn as_maintenance(&self) -> Option<&dyn MemoryMaintenance> {
        Some(self)
    }
    // The four families below are gated on the pinned artifact rather than
    // returning `Some(self)` unconditionally. `provides()` derives from these
    // accessors, the guard builds its decorators from `provides()`, and every
    // caller already writes a clean "driver does not support the X family"
    // error on `None` — so gating here converts a deep `UnknownMethod` into an
    // early, accurate refusal at every call site at once (#5598).
    fn as_people(&self) -> Option<&dyn MemoryPeople> {
        artifact_serves(Capability::People).then_some(self as &dyn MemoryPeople)
    }
    fn as_chunks(&self) -> Option<&dyn MemoryChunks> {
        artifact_serves(Capability::Chunks).then_some(self as &dyn MemoryChunks)
    }
    fn as_retrieval(&self) -> Option<&dyn MemoryRetrieval> {
        artifact_serves(Capability::Retrieval).then_some(self as &dyn MemoryRetrieval)
    }
    fn as_profile(&self) -> Option<&dyn MemoryProfile> {
        artifact_serves(Capability::Profile).then_some(self as &dyn MemoryProfile)
    }

    fn as_source_sync(&self) -> Option<&dyn MemorySourceSync> {
        artifact_serves(Capability::SourceSync).then_some(self as &dyn MemorySourceSync)
    }

    fn as_coding_sessions(&self) -> Option<&dyn MemoryCodingSessions> {
        artifact_serves(Capability::CodingSessions).then_some(self as &dyn MemoryCodingSessions)
    }
    fn as_episodic(&self) -> Option<&dyn MemoryEpisodic> {
        artifact_serves(Capability::Episodic).then_some(self as &dyn MemoryEpisodic)
    }
    fn as_scoring(&self) -> Option<&dyn MemoryScoring> {
        artifact_serves(Capability::Scoring).then_some(self as &dyn MemoryScoring)
    }
}

#[async_trait]
impl MemoryCore for ModuleMemoryProvider {
    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> Result<(), MemoryError> {
        // No log line carries `namespace`, `key` or `content`: all three are user
        // memory content.
        self.proxy("store")
            .await?
            .call::<()>(
                methods::STORE,
                (
                    namespace,
                    key,
                    content,
                    category,
                    session_id.map(str::to_string),
                    taint,
                ),
            )
            .await
            .map_err(|error| from_bus(&error))
    }

    async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        self.proxy("get")
            .await?
            .call(methods::GET, (namespace, key))
            .await
            .map_err(|error| from_bus(&error))
    }

    async fn forget(&self, namespace: &str, key: &str) -> Result<bool, MemoryError> {
        self.proxy("forget")
            .await?
            .call(methods::FORGET, (namespace, key))
            .await
            .map_err(|error| from_bus(&error))
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.proxy("list")
            .await?
            .call(
                methods::LIST,
                (
                    namespace.map(str::to_string),
                    category.cloned(),
                    session_id.map(str::to_string),
                ),
            )
            .await
            .map_err(|error| from_bus(&error))
    }

    async fn namespaces(&self) -> Result<Vec<NamespaceSummary>, MemoryError> {
        self.proxy("namespaces")
            .await?
            .call(methods::NAMESPACES, ())
            .await
            .map_err(|error| from_bus(&error))
    }
}

#[async_trait]
impl MemoryRecall for ModuleMemoryProvider {
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: &OwnedRecallOpts,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        // `scope` crosses as a value because the driver must apply it as a query
        // predicate internally; narrowing the result here instead would let the
        // module spend its `limit` on entries the caller may not see.
        self.proxy("recall")
            .await?
            .call(methods::RECALL, (query, limit, opts, scope.cloned()))
            .await
            .map_err(|error| from_bus(&error))
    }
}

#[async_trait]
impl MemoryPortability for ModuleMemoryProvider {
    async fn export_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ExportPage, MemoryError> {
        self.proxy("export_page")
            .await?
            .call(methods::EXPORT_PAGE, (cursor.map(str::to_string), limit))
            .await
            .map_err(|error| from_bus(&error))
    }

    async fn import_records(
        &self,
        records: Vec<ExportRecord>,
    ) -> Result<ImportOutcome, MemoryError> {
        self.proxy("import_records")
            .await?
            .call(methods::IMPORT_RECORDS, (records,))
            .await
            .map_err(|error| from_bus(&error))
    }
}

macro_rules! module_call {
    ($self:expr, $operation:literal, $method:expr, $args:expr) => {
        $self
            .proxy($operation)
            .await?
            .call($method, $args)
            .await
            .map_err(|error| from_bus(&error))
    };
}

/// [`module_call!`] with a deadline sized for bulk work.
///
/// The default bus deadline (30s) fits request-shaped members. The bulk
/// ingest members are not that: `AcceptSourceItems` embeds and writes a whole
/// connector page of records inside the call — a 200-email Gmail handoff
/// blew the 30s deadline live while the module went on to finish the work,
/// and the sync retry loop then re-ran the same handoff forever. Same
/// pathology, and same fix, as the connector module's `Sync` member.
macro_rules! module_call_slow {
    ($self:expr, $operation:literal, $method:expr, $args:expr) => {
        $self
            .proxy($operation)
            .await?
            .with_timeout(std::time::Duration::from_secs(15 * 60))
            .call($method, $args)
            .await
            .map_err(|error| from_bus(&error))
    };
}

#[async_trait]
impl MemoryIngest for ModuleMemoryProvider {
    async fn ingest_document(&self, item: IngestItem) -> Result<IngestOutcome, MemoryError> {
        module_call!(self, "ingest_document", methods::INGEST_DOCUMENT, (item,))
    }
    async fn ingest_chat(&self, messages: Vec<IngestItem>) -> Result<IngestOutcome, MemoryError> {
        module_call_slow!(self, "ingest_chat", methods::INGEST_CHAT, (messages,))
    }
    async fn ingest_email(&self, messages: Vec<IngestItem>) -> Result<IngestOutcome, MemoryError> {
        module_call_slow!(self, "ingest_email", methods::INGEST_EMAIL, (messages,))
    }
}
