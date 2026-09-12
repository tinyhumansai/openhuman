use std::sync::Arc;
use std::time::Duration;

use tinymemory_api::capabilities::{Capabilities, Capability};

/// The release whose capability set [`ARTIFACT_CAPABILITIES`] was read from.
///
/// Checked against the registry pin by `the_capability_list_matches_the_pinned_release`,
/// so bumping the pin without re-reading the list is a red test rather than a
/// silent over-claim.
pub(crate) const ARTIFACT_CAPABILITIES_PIN: &str = "1.16.0";

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
    // Re-read at tag `v1.15.1` (tinymemory#142), one commit past v1.15.0. The
    // CortexDB adapter now asks `v1/scopes/list` for every scope instead of
    // accepting the engine's undocumented first fifty, which had been
    // truncating namespace enumeration — a live instance holding 93 listed 50,
    // and everything downstream of `namespace_summaries` reported success on
    // the subset. Behaviour inside a hosted adapter, not the module's surface:
    // `git diff v1.15.0..v1.15.1 -- crates/tinymemory-api/src/capabilities.rs
    // crates/tinymemory-bus/src/capabilities.rs crates/tinymemory-bus/src/names.rs`
    // is empty and `crates/tinymemory-module/` moves only its `Cargo.lock`, so
    // the module declares the same members it did and the list below is
    // unchanged — only the pin advances.
    //
    // Re-read at tag `v1.15.0` (tinymemory#141, openhuman#6025). The connector
    // sink now embeds a whole `accept_source_items` batch together and the
    // vendored engine claims a due `reembed_backfill` ahead of the extraction
    // backlog (tinycortex#168). Behaviour inside `Sources`/`Maintenance`, no
    // new bus member and no new family: `git diff v1.14.1..v1.15.0 --
    // crates/tinymemory-bus/src/capabilities.rs crates/tinymemory-bus/src/names.rs`
    // is empty, so the list below is unchanged and only the pin moves.
    //
    // Re-read at tag `v1.14.1` (tinymemory#136 + #137, openhuman#6012). It adds a bus
    // *member*, `BackfillConnectorTrees`, and no capability: `Capability` is the
    // family enum, and the member is a method inside `Maintenance`, which this
    // build already advertises. `git diff v1.13.8..v1.14.1 --
    // crates/tinymemory-bus/src/capabilities.rs` is empty, so nothing below moves.
    //
    // Re-read at tag `v1.13.8` (tinymemory#134, openhuman#6007): the connector
    // sync path now routes its items into the memory-tree ingest funnel, and
    // `forget_source` sweeps the per-item tree rows it creates. Behaviour inside
    // `Sources`/`Maintenance`, not a new family —
    // `git diff v1.13.7..v1.13.8 -- crates/tinymemory-api/src/capabilities.rs`
    // returns empty, so the list below is unchanged and only the pin moves.
    //
    // v1.13.7 (tinymemory#125 + #127): the typed ingestion round and the
    // answer surface, served and advertised by the pinned artifact.
    Capability::DocumentIngest,
    Capability::ConversationIngest,
    Capability::LearningIngest,
    Capability::EventIngest,
    Capability::Answer,
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
use tinymemory_api::learning::LearningCandidate;
use tinymemory_api::provider::operations::{
    AnswerRequest, AnswerResponse, MemoryAnswer, MemoryConversationIngest, MemoryDocumentIngest,
    MemoryEventIngest, MemoryLearningIngest, RawMemoryEvent,
};
use tinymemory_api::provider::sessions::{
    CodingSessionIngestReport, CodingSessionIngestRequest, CodingSessionSource,
};
use tinymemory_api::provider::sync::{
    RawArchiveCoverage, RawRebuildOutcome, SourceSyncState, SourceSyncStatus, SyncAuditEntry,
    SyncRunOutcome,
};
use tinymemory_api::provider::types::{
    BackfillTreesOutcome, BackfillTreesRequest, ChunkEntityOccurrence, DiffReport, EntityHit,
    EntityOccurrence, ExportPage, ExportRecord, FlushOutcome, ForgetOutcome, ForgetSelector,
    ImportOutcome, IngestItem, IngestOutcome,
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

/// How long a bounded memory call waits for the module before reporting it as
/// still loading.
///
/// A warm launch maps the cached library in well under a second and the
/// module's own initialisation is capped at five by tinybus, so this is
/// comfortably past the whole happy path while staying well inside the
/// desktop's thirty-second RPC deadline — the caller learns "loading", not
/// "timed out".
const MODULE_LOADING_GRACE: Duration = Duration::from_secs(8);

/// The operations a caller may stop waiting for. **Everything else waits.**
///
/// A write that gives up is lost work: the message a chat turn autosaves, the
/// turn the archivist records after the fact, a sync an operator kicked off,
/// the shutdown that releases queue locks. None of those has a retry above it,
/// so a `Unavailable` answer during a cold load does not delay the write — it
/// discards it. Reads are the other case: a page or a turn is better served by
/// "memory is loading" now than by the right answer three minutes from now.
///
/// # Why this names the reads and not the writes
///
/// It was the other way round first, and that was a bug. Listing the *writes*
/// makes every unlisted member a read, so the default for anything the list
/// forgets — or anything a future contract adds — is to drop the call. The
/// first version of this list named nine operations out of seventy and
/// silently classified `ingest_document`, `ingest_chat`, `insert_turn`,
/// `set_goals`, `append` and two dozen other mutations as reads.
///
/// Naming the reads inverts the failure: a member nobody classified waits for
/// the module, which costs a slow first call on a cold launch and loses
/// nothing. Add a read here only when it genuinely cannot mutate — when in
/// doubt, leave it out and it waits.
///
/// # The list is total, and that is checked
///
/// Naming the reads is only safe from lost writes; it is not automatically
/// *complete*. The first pass named thirty-seven of the hundred and forty-one
/// members, so `entities`, `relations`, `summary_forest`, `retrieve_children`
/// and thirty-seven other genuine reads still waited out the whole download —
/// the tree, graph and sources panels this change exists to unblock. The
/// guard test partitions every dispatch label in the sources into this list
/// or its counterpart, so a new member fails the build until someone
/// classifies it.
const BOUNDED_READ_OPERATIONS: &[&str] = &[
    "answer",
    "backfill_in_progress",
    "chunk_detail",
    "chunk_embeddings",
    "chunk_entities",
    "chunk_score",
    "coding_session_status",
    "count_chunks",
    "cover_window",
    "degraded_state",
    "diagnose",
    "diff",
    "doctor",
    "drill_down",
    "embed_text",
    "embedder_slug",
    "entities",
    "entity_chunk_ids",
    "entity_edges",
    "estimate_sync_cost_usd",
    "export_page",
    "extract_entities",
    "facets_by_type",
    "fast_retrieve",
    "flavour_profile",
    "get",
    "get_chunk",
    "get_document",
    "get_facet",
    "get_person",
    "goals",
    "health",
    "is_toolkit_syncable",
    "kv_get",
    "kv_list",
    "latest_queue_failure",
    "list",
    "list_active_facets",
    "list_all_facets",
    "list_chunk_details",
    "list_chunks",
    "list_documents",
    "list_namespaces",
    "list_people",
    "namespaces",
    "query_documents",
    "query_source",
    "queue_stats",
    "raw_archive_coverage",
    "recall",
    "recall_documents",
    "recall_namespace_recent",
    "recall_namespace_scored",
    "recent_leaves",
    "relations",
    "resolve_handle",
    "retrieve_children",
    "retrieve_leaves",
    "retrieve_source",
    "root_summaries_with_caps",
    "runtime_read_children",
    "runtime_read_node",
    "runtime_tree_status",
    "score_person",
    "search_entities",
    // #6186. Selects closed segments with no summary; it writes nothing.
    // The write it leads to is `set_segment_summary`, classified separately.
    "segments_pending_summary",
    "session_turns",
    "snapshots",
    "source_ingest_status",
    "source_sync_state",
    "source_totals",
    "storage_kinds",
    "store_stats",
    "summary_forest",
    "sync_audit_log",
    "sync_statuses",
    "tool_rules",
    "top_entities",
    "workflow_identity_matches",
];

/// Install the host-side callbacks the module reaches for while it loads.
///
/// Idempotent and process-wide. Both the lazy path ([`ModuleMemoryProvider`]'s
/// first call) and the eager path (boot) go through here, because a module
/// admitted before these exist resolves no embedding provider and cannot be
/// repaired without a restart.
///
/// # Errors
///
/// Returns a message when the module bus cannot start or the callbacks cannot
/// be served on it.
pub async fn install_host_callbacks(config: Arc<Config>) -> Result<(), String> {
    let runtime = host::runtime()
        .await
        .map_err(|error| format!("the module bus is not running: {error}"))?;
    super::memory_host::install(runtime.connection(), config)
        .await
        .map_err(|error| format!("the memory module host callbacks are unavailable: {error}"))
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
    /// How long a bounded call waits for the module to load — see
    /// [`MODULE_LOADING_GRACE`]. Overridable so a test can observe the loading
    /// state without waiting eight seconds for it.
    loading_grace: Duration,
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
            loading_grace: MODULE_LOADING_GRACE,
        }
    }

    /// Bound how long a read waits for the module to load.
    #[must_use]
    pub fn with_loading_grace(mut self, grace: Duration) -> Self {
        self.loading_grace = grace;
        self
    }

    /// How long `operation` may wait for the module: `None` waits it out.
    ///
    /// Unrecognised operations wait — see [`BOUNDED_READ_OPERATIONS`].
    fn loading_grace(&self, operation: &str) -> Option<Duration> {
        if BOUNDED_READ_OPERATIONS.contains(&operation) {
            Some(self.loading_grace)
        } else {
            None
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
        // TinyMemory resolves its embedding provider while the native library
        // is admitted. Host callbacks must therefore exist before loading,
        // including in tests and explicit-path overrides where no boot policy
        // was available when the shared module runtime first started.
        install_host_callbacks(Arc::clone(config))
            .await
            .map_err(|message| MemoryError::Other(anyhow::anyhow!(message)))?;
        let runtime = host::runtime().await.map_err(|error| {
            MemoryError::Other(anyhow::anyhow!("the module bus is not running: {error}"))
        })?;
        // A load failure is terminal for the process (the loader caches it),
        // so every memory member would otherwise return the loader's raw
        // message — release URLs, digest text, "restart the app" repeated per
        // call. Map it once into the subsystem's honest degraded state: a
        // user_error broadcast (once per process, metadata only) plus a
        // stable, actionable error for the caller. The raw reason goes to the
        // log, where an operator can act on it.
        //
        // A load still in progress is different: nothing failed, the caller
        // simply asked before the download or the initialisation finished. A
        // read reports that as `Unavailable` — the retryable class — after its
        // grace instead of hanging into the caller's own deadline; everything
        // else waits it out (see `BOUNDED_READ_OPERATIONS`).
        let grace = self.loading_grace(operation);
        match ops::ensure_loaded_within(config, MODULE_ID, grace).await {
            Ok(()) => {}
            Err(ops::LoadError::StillLoading) => {
                log::info!(
                    "[modules:memory] operation={operation} module still loading after {grace:?}; \
                     answering unavailable"
                );
                return Err(MemoryError::Unavailable(
                    "memory is still starting: the memory module is loading; try again in a moment"
                        .to_string(),
                ));
            }
            Err(ops::LoadError::Failed(message)) => {
                crate::openhuman::memory::tree::health::user_error::notice_memory_module_unavailable_once(
                    &message,
                );
                return Err(MemoryError::Backend(
                    "memory is unavailable: the memory module failed to load. Restart the app to \
                     retry; the reason is in the log."
                        .to_string(),
                ));
            }
        }

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
