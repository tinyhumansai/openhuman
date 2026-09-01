use std::borrow::Cow;
use std::sync::Arc;

use crate::openhuman::memory::api::capabilities::Capability;
use crate::openhuman::memory::api::chunks::Chunk;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::goals::GoalsDoc;
use crate::openhuman::memory::api::provider::chunks::{
    ChunkDetail, ChunkEmbedding, ChunkListRow, ChunkQuery, ChunkScore, MemoryChunks,
    SourceIngestQuery, SourceIngestStatus, SourceTotal,
};
use crate::openhuman::memory::api::provider::content::{
    RootSummary, SummaryContext, SummaryInput, SummaryOutput,
};
use crate::openhuman::memory::api::provider::diagnosis::{DegradedCapabilities, Diagnosis};
use crate::openhuman::memory::api::provider::episodic::{
    ConversationSegment, EpisodicTurn, MemoryEpisodic,
};
use crate::openhuman::memory::api::provider::people::{
    AddressBookSeedOutcome, MemoryPeople, PersonHandle, PersonInteraction, PersonRecord,
    PersonScore, RankedPerson, ResolvedPerson,
};
use crate::openhuman::memory::api::provider::profile::{
    FacetType, MemoryProfile, ProfileFacet, UserState,
};
use crate::openhuman::memory::api::provider::retrieval::{
    CoverWindowQuery, EntityMatch, FastRetrieveQuery, MemoryRetrieval, RetrievalHit,
    RetrievalResponse, SourceRetrievalQuery,
};
use crate::openhuman::memory::api::provider::scoring::MemoryScoring;
use crate::openhuman::memory::api::provider::sessions::{
    CodingSessionIngestReport, CodingSessionIngestRequest, CodingSessionSource,
};
use crate::openhuman::memory::api::provider::sync::{
    RawArchiveCoverage, RawRebuildOutcome, SourceSyncState, SourceSyncStatus, SyncAuditEntry,
    SyncRunOutcome,
};
use crate::openhuman::memory::api::provider::types::{
    ChunkEntityOccurrence, DiffReport, EntityHit, EntityOccurrence, ForgetOutcome, ForgetSelector,
    IngestItem, IngestOutcome, MaintenanceReport, PurgeOutcome, SnapshotRef, SourceItem,
    SourceScope,
};
use crate::openhuman::memory::api::provider::{
    EpisodicEvent, MemoryCodingSessions, MemoryDiff, MemoryDocuments, MemoryEntities, MemoryGoals,
    MemoryGraph, MemoryIngest, MemoryMaintenance, MemoryProvider, MemorySourceSink,
    MemorySourceSync, MemoryToolMemory, MemoryTree,
};
use crate::openhuman::memory::api::tool_memory::ToolMemoryRule;
use crate::openhuman::memory::api::tree::{
    IngestRequest, QueryResult, SummaryForest, TreeLeaf, TreeNode, TreeStatus,
};
use crate::openhuman::memory::api::types::NamespaceMemoryHit;
use crate::openhuman::memory::api::types::{
    GraphRelationRecord, MemoryKvRecord, MemoryTaint, NamespaceDocumentInput,
    NamespaceRetrievalContext, StoredMemoryDocument,
};
use async_trait::async_trait;

use super::audit::{trace_allowed, NO_NAMESPACE};
use super::policy::GuardPolicy;

/// Declares one decorator: the two shared fields, a constructor, and the
/// `family()` re-derivation.
macro_rules! decorator {
    ($(#[$meta:meta])* $name:ident, $fam:ty, $accessor:ident, $cap:ident) => {
        $(#[$meta])*
        pub struct $name {
            inner: Arc<dyn MemoryProvider>,
            policy: Arc<GuardPolicy>,
        }

        impl $name {
            pub(super) fn new(inner: Arc<dyn MemoryProvider>, policy: Arc<GuardPolicy>) -> Self {
                Self { inner, policy }
            }

            /// The underlying family handle.
            ///
            /// The `Err` arm is **structurally unreachable**: `MemoryGuard::new`
            /// only builds this decorator when the inner provider answered
            /// `provides(Capability::$cap)`, and the contract documents the
            /// capability set as fixed at bind time. It is written as a real
            /// error rather than `.expect(...)` because a panic inside a memory
            /// call is a strictly worse failure than an `Unsupported` a caller
            /// can already handle.
            fn family(&self) -> Result<&$fam, MemoryError> {
                self.inner
                    .$accessor()
                    .ok_or_else(|| MemoryError::unsupported(Capability::$cap))
            }
        }
    };
}

decorator!(
    /// Guarded [`MemoryIngest`].
    GuardedIngest,
    dyn MemoryIngest,
    as_ingest,
    Ingest
);
decorator!(
    /// Guarded [`MemoryDocuments`].
    GuardedDocuments,
    dyn MemoryDocuments,
    as_documents,
    Documents
);
decorator!(
    /// Guarded [`MemoryTree`] — the one family that carries step 2.
    GuardedTree,
    dyn MemoryTree,
    as_tree,
    Tree
);
decorator!(
    /// Guarded [`MemoryEntities`].
    GuardedEntities,
    dyn MemoryEntities,
    as_entities,
    Entities
);
decorator!(
    /// Guarded [`MemoryGraph`].
    GuardedGraph,
    dyn MemoryGraph,
    as_graph,
    Graph
);
decorator!(
    /// Guarded [`MemoryDiff`].
    GuardedDiff,
    dyn MemoryDiff,
    as_diff,
    Diff
);
decorator!(
    /// Guarded [`MemoryGoals`].
    GuardedGoals,
    dyn MemoryGoals,
    as_goals,
    Goals
);
decorator!(
    /// Guarded [`MemoryToolMemory`].
    GuardedToolMemory,
    dyn MemoryToolMemory,
    as_tool_memory,
    ToolMemory
);
decorator!(
    /// Guarded [`MemorySourceSink`].
    GuardedSources,
    dyn MemorySourceSink,
    as_sources,
    Sources
);
decorator!(
    /// Guarded [`MemoryMaintenance`].
    GuardedMaintenance,
    dyn MemoryMaintenance,
    as_maintenance,
    Maintenance
);
decorator!(
    /// Guarded [`MemoryPeople`].
    GuardedPeople,
    dyn MemoryPeople,
    as_people,
    People
);
decorator!(
    /// Guarded [`MemoryChunks`].
    GuardedChunks,
    dyn MemoryChunks,
    as_chunks,
    Chunks
);
decorator!(
    /// Guarded [`MemoryRetrieval`].
    GuardedRetrieval,
    dyn MemoryRetrieval,
    as_retrieval,
    Retrieval
);
decorator!(
    /// Guarded [`MemoryEpisodic`].
    GuardedEpisodic,
    dyn MemoryEpisodic,
    as_episodic,
    Episodic
);
decorator!(
    /// Guarded [`MemorySourceSync`].
    GuardedSourceSync,
    dyn MemorySourceSync,
    as_source_sync,
    SourceSync
);
decorator!(
    /// Guarded [`MemoryCodingSessions`].
    GuardedCodingSessions,
    dyn MemoryCodingSessions,
    as_coding_sessions,
    CodingSessions
);
decorator!(
    /// Guarded [`MemoryProfile`].
    GuardedProfile,
    dyn MemoryProfile,
    as_profile,
    Profile
);
decorator!(
    /// Guarded [`MemoryScoring`].
    GuardedScoring,
    dyn MemoryScoring,
    as_scoring,
    Scoring
);

// ── Ingest ───────────────────────────────────────────────────────────────────

impl GuardedIngest {
    /// Steps 3 + 4 over one ingest item: stamp provenance, redact on egress.
    fn admit(&self, mut item: IngestItem) -> IngestItem {
        item.taint = self.policy.stamp_taint(item.taint);
        item.content = self.policy.redact_outbound(&item.content).into_owned();
        item
    }
}

#[async_trait]
impl MemoryIngest for GuardedIngest {
    async fn ingest_document(&self, item: IngestItem) -> Result<IngestOutcome, MemoryError> {
        let namespace = item.namespace.clone().unwrap_or_else(|| "-".to_string());
        self.policy.admit_write(
            Capability::Ingest,
            "ingest.ingest_document",
            &namespace,
            true,
        )?;
        let item = self.admit(item);
        trace_allowed(
            &self.policy,
            "ingest.ingest_document",
            &namespace,
            item.content.chars().count(),
        );
        self.family()?.ingest_document(item).await
    }

    async fn ingest_chat(&self, messages: Vec<IngestItem>) -> Result<IngestOutcome, MemoryError> {
        self.policy
            .admit_write(Capability::Ingest, "ingest.ingest_chat", NO_NAMESPACE, true)?;
        let messages: Vec<IngestItem> = messages.into_iter().map(|m| self.admit(m)).collect();
        trace_allowed(
            &self.policy,
            "ingest.ingest_chat",
            NO_NAMESPACE,
            messages.iter().map(|m| m.content.chars().count()).sum(),
        );
        self.family()?.ingest_chat(messages).await
    }

    async fn ingest_email(&self, messages: Vec<IngestItem>) -> Result<IngestOutcome, MemoryError> {
        // Admitted exactly like chat: a thread is one conversation with no
        // namespace of its own, and every message is taint-stamped and
        // redacted before it reaches the driver.
        self.policy.admit_write(
            Capability::Ingest,
            "ingest.ingest_email",
            NO_NAMESPACE,
            true,
        )?;
        let messages: Vec<IngestItem> = messages.into_iter().map(|m| self.admit(m)).collect();
        trace_allowed(
            &self.policy,
            "ingest.ingest_email",
            NO_NAMESPACE,
            messages.iter().map(|m| m.content.chars().count()).sum(),
        );
        self.family()?.ingest_email(messages).await
    }
}

// ── Documents ────────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryDocuments for GuardedDocuments {
    async fn put_document(&self, mut input: NamespaceDocumentInput) -> Result<String, MemoryError> {
        self.policy.admit_write(
            Capability::Documents,
            "documents.put_document",
            &input.namespace,
            true,
        )?;
        input.taint = self.policy.stamp_taint(input.taint);
        input.title = self.policy.redact_outbound(&input.title).into_owned();
        input.content = self.policy.redact_outbound(&input.content).into_owned();
        input.metadata = self.policy.redact_outbound_json(input.metadata);
        trace_allowed(
            &self.policy,
            "documents.put_document",
            &input.namespace,
            input.content.chars().count(),
        );
        self.family()?.put_document(input).await
    }

    async fn get_document(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<StoredMemoryDocument>, MemoryError> {
        self.policy.admit_read(
            Capability::Documents,
            "documents.get_document",
            namespace,
            false,
        )?;
        self.family()?.get_document(namespace, key).await
    }

    async fn list_documents(
        &self,
        namespace: Option<&str>,
    ) -> Result<serde_json::Value, MemoryError> {
        self.policy.admit_read(
            Capability::Documents,
            "documents.list_documents",
            namespace.unwrap_or(NO_NAMESPACE),
            false,
        )?;
        self.family()?.list_documents(namespace).await
    }

    async fn list_namespaces(&self) -> Result<Vec<String>, MemoryError> {
        self.policy.admit_read(
            Capability::Documents,
            "documents.list_namespaces",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.list_namespaces().await
    }

    async fn delete_document(
        &self,
        namespace: &str,
        document_id: &str,
    ) -> Result<serde_json::Value, MemoryError> {
        self.policy.admit_write(
            Capability::Documents,
            "documents.delete_document",
            namespace,
            false,
        )?;
        self.family()?.delete_document(namespace, document_id).await
    }

    async fn clear_namespace(&self, namespace: &str) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::Documents,
            "documents.clear_namespace",
            namespace,
            false,
        )?;
        self.family()?.clear_namespace(namespace).await
    }

    async fn query_documents(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
    ) -> Result<NamespaceRetrievalContext, MemoryError> {
        // The query text itself crosses the boundary on an external driver.
        self.policy.admit_read(
            Capability::Documents,
            "documents.query_documents",
            namespace,
            true,
        )?;
        let query = self.policy.redact_outbound(query).into_owned();
        self.family()?
            .query_documents(namespace, &query, limit)
            .await
    }

    async fn recall_documents(
        &self,
        namespace: &str,
        limit: usize,
    ) -> Result<NamespaceRetrievalContext, MemoryError> {
        self.policy.admit_read(
            Capability::Documents,
            "documents.recall_documents",
            namespace,
            false,
        )?;
        self.family()?.recall_documents(namespace, limit).await
    }
}

// ── Tree ─────────────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryTree for GuardedTree {
    async fn append(&self, mut request: IngestRequest) -> Result<(), MemoryError> {
        self.policy
            .admit_write(Capability::Tree, "tree.append", &request.namespace, true)?;
        request.content = self.policy.redact_outbound(&request.content).into_owned();
        trace_allowed(
            &self.policy,
            "tree.append",
            &request.namespace,
            request.content.chars().count(),
        );
        self.family()?.append(request).await
    }

    /// **Step 2 lives here.** This is the only contract method in the tree
    /// today that both takes a [`SourceScope`] and applies it as a real query
    /// predicate: the embedded driver pushes `scope.allow` into
    /// `ListChunksQuery.source_scope`, which reaches SQL *before* `LIMIT`.
    ///
    /// The ambient allowlist
    /// ([`source_scope::current_source_scope`](crate::openhuman::memory::source_scope::current_source_scope))
    /// is therefore read at this boundary and passed down, rather than being
    /// applied to the returned rows. An explicit `scope` argument may only
    /// *narrow* it: the two are intersected by
    /// [`GuardPolicy::narrow_scope`](crate::openhuman::memory::guard::GuardPolicy::narrow_scope),
    /// so a caller that computed a tighter scope than the task-local still wins,
    /// while one that names a collection outside the ambient allowlist cannot
    /// widen the turn back out.
    ///
    /// There is **no double application**: the embedded `query_source` does not
    /// itself read the task-local (only the deeper `tree::retrieval` and
    /// `list_chunks` paths do, and the guard does not sit in front of those),
    /// so this fills a predicate that would otherwise be `None`.
    async fn query_source(
        &self,
        namespace: &str,
        source_id: &str,
        limit: usize,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<Chunk>, MemoryError> {
        self.policy
            .admit_read(Capability::Tree, "tree.query_source", namespace, false)?;
        let ambient = self.policy.ambient_scope();
        let effective = self.policy.narrow_scope(scope);
        log::debug!(
            "[memory:guard] tree.query_source namespace={namespace} limit={limit} \
             scoped={} scope_from={}",
            effective.is_some(),
            match (scope.is_some(), ambient.is_some()) {
                (true, true) => "argument∩ambient",
                (true, false) => "argument",
                (false, true) => "ambient",
                (false, false) => "none",
            }
        );
        self.family()?
            .query_source(namespace, source_id, limit, effective.as_ref())
            .await
    }

    async fn drill_down(&self, namespace: &str, node_id: &str) -> Result<QueryResult, MemoryError> {
        self.policy
            .admit_read(Capability::Tree, "tree.drill_down", namespace, false)?;
        self.family()?.drill_down(namespace, node_id).await
    }

    async fn seal(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        self.policy
            .admit_write(Capability::Tree, "tree.seal", namespace, false)?;
        self.family()?.seal(namespace).await
    }

    async fn cascade(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        self.policy
            .admit_write(Capability::Tree, "tree.cascade", namespace, false)?;
        self.family()?.cascade(namespace).await
    }

    /// Enumerates the sealed forest, so it narrows by the ambient scope for the
    /// same reason [`Self::query_source`] does: a summary is derived from the
    /// chunks beneath it, and handing back a node built from sources the caller
    /// may not read discloses their contents in condensed form.
    async fn summary_forest(
        &self,
        limit: usize,
        scope: Option<&SourceScope>,
    ) -> Result<SummaryForest, MemoryError> {
        self.policy
            .admit_read(Capability::Tree, "tree.summary_forest", NO_NAMESPACE, false)?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?
            .summary_forest(limit, effective.as_ref())
            .await
    }

    /// Sealing one source's tree is a write, and it names the scope it acts on
    /// — so unlike the reads above it is admitted against that scope rather
    /// than `NO_NAMESPACE`. `carries_content: false`: the caller supplies a
    /// scope label, never prose, and the seals it fires write content the
    /// driver already holds.
    async fn flush_source_tree(&self, source_scope: &str) -> Result<u64, MemoryError> {
        self.policy.admit_write(
            Capability::Tree,
            "tree.flush_source_tree",
            source_scope,
            false,
        )?;
        self.family()?.flush_source_tree(source_scope).await
    }

    /// Leaves are chunks, so this is the same disclosure as
    /// [`Self::query_source`] with a different ordering, and takes the same
    /// intersection.
    async fn recent_leaves(
        &self,
        limit: usize,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<TreeLeaf>, MemoryError> {
        self.policy
            .admit_read(Capability::Tree, "tree.recent_leaves", NO_NAMESPACE, false)?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?
            .recent_leaves(limit, effective.as_ref())
            .await
    }

    /// A **read** tier check even though the fold costs a provider call: it
    /// writes nothing. `seal` and `cascade` take the write tier because they
    /// persist the nodes they produce; this hands the summary back and leaves
    /// the tree exactly as it found it, so refusing it under `readonly` would
    /// stop a recap that stores nothing.
    ///
    /// It is nevertheless the only member of this family besides
    /// [`Self::append`] that carries prose *outbound* — every input's body
    /// crosses to the driver's own chat provider — so it declares
    /// `carries_content: true` and applies `append`'s scrub to each of them.
    /// Admitted against [`SummaryContext::tree_id`] rather than
    /// `NO_NAMESPACE` for the reason [`Self::flush_source_tree`] gives: it
    /// names the tree it acts on.
    async fn summarise(
        &self,
        inputs: &[SummaryInput],
        context: &SummaryContext,
    ) -> Result<SummaryOutput, MemoryError> {
        self.policy
            .admit_read(Capability::Tree, "tree.summarise", &context.tree_id, true)?;
        // `redact_outbound` borrows for every driver class but `External`, so
        // the re-owned slice is built only when the scrubber actually rewrote
        // something — a recap folds every turn of a segment, and cloning them
        // to hand back the same bytes is the one cost this can avoid.
        let mut scrubbed: Option<Vec<SummaryInput>> = None;
        for (index, input) in inputs.iter().enumerate() {
            if let Cow::Owned(content) = self.policy.redact_outbound(&input.content) {
                scrubbed.get_or_insert_with(|| inputs.to_vec())[index].content = content;
            }
        }
        let effective = scrubbed.as_deref().unwrap_or(inputs);
        trace_allowed(
            &self.policy,
            "tree.summarise",
            &context.tree_id,
            effective
                .iter()
                .map(|input| input.content.chars().count())
                .sum(),
        );
        self.family()?.summarise(effective, context).await
    }

    /// The markdown time tree's roots, one body per namespace.
    ///
    /// Takes no [`SourceScope`], so unlike [`Self::summary_forest`] there is
    /// nothing here to intersect with the ambient allowlist — the contract
    /// member has no scope parameter and the guard does not invent one. Both
    /// caps are the caller's and cross unchanged: they bound the *response*,
    /// and clipping them here would produce a body the driver did not choose
    /// the truncation point of.
    async fn root_summaries_with_caps(
        &self,
        per_namespace_cap: usize,
        total_cap: usize,
    ) -> Result<Vec<RootSummary>, MemoryError> {
        self.policy.admit_read(
            Capability::Tree,
            "tree.root_summaries_with_caps",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?
            .root_summaries_with_caps(per_namespace_cap, total_cap)
            .await
    }

    // ── The runtime-tree and flavour doors ──────────────────────────────────
    //
    // **Forwarding these is not optional.** All seven are defaulted on
    // [`MemoryTree`], so a decorator that omits one still compiles — and then
    // answers `Err(Unsupported)` for a driver that serves the member perfectly
    // well, because the guard *is* the handle every product caller holds and
    // its own default is what runs. That exact bug shipped once, on
    // `MemoryMaintenance::diagnose`. The rule this family follows: a new
    // defaulted member on a wrapped trait is a new override here, in the same
    // change.
    //
    // None of them takes a [`SourceScope`], so step 2 does not apply — the
    // contract members carry no scope parameter and the guard does not invent
    // one; see [`Self::root_summaries_with_caps`] for the same reasoning.

    /// Buffering raw content is [`Self::append`]'s write at a finer grain, so
    /// it takes `append`'s admission exactly: the write tier, against the
    /// namespace it names, `carries_content: true`, and the same outbound
    /// scrub applied to the body before it crosses.
    async fn runtime_buffer_write(
        &self,
        namespace: &str,
        content: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
        metadata: Option<serde_json::Value>,
    ) -> Result<String, MemoryError> {
        self.policy.admit_write(
            Capability::Tree,
            "tree.runtime_buffer_write",
            namespace,
            true,
        )?;
        let content = self.policy.redact_outbound(content);
        trace_allowed(
            &self.policy,
            "tree.runtime_buffer_write",
            namespace,
            content.chars().count(),
        );
        self.family()?
            .runtime_buffer_write(namespace, &content, timestamp, metadata)
            .await
    }

    /// A single node read, admitted like [`Self::drill_down`] — the same tree
    /// at the same grain, minus the child list.
    async fn runtime_read_node(
        &self,
        namespace: &str,
        node_id: &str,
    ) -> Result<Option<TreeNode>, MemoryError> {
        self.policy
            .admit_read(Capability::Tree, "tree.runtime_read_node", namespace, false)?;
        self.family()?.runtime_read_node(namespace, node_id).await
    }

    /// The other half of [`Self::drill_down`], admitted identically.
    async fn runtime_read_children(
        &self,
        namespace: &str,
        parent_id: &str,
    ) -> Result<Vec<TreeNode>, MemoryError> {
        self.policy.admit_read(
            Capability::Tree,
            "tree.runtime_read_children",
            namespace,
            false,
        )?;
        self.family()?
            .runtime_read_children(namespace, parent_id)
            .await
    }

    /// Counts and timestamps for one namespace: a read, and one that carries
    /// no prose either way.
    async fn runtime_tree_status(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        self.policy.admit_read(
            Capability::Tree,
            "tree.runtime_tree_status",
            namespace,
            false,
        )?;
        self.family()?.runtime_tree_status(namespace).await
    }

    /// The **write** tier, unlike [`Self::summarise`]: this drains the buffer
    /// into hour leaves and persists them, which is what [`Self::seal`] does
    /// and why `seal` takes the write tier too. `summarise` hands a fold back
    /// and leaves the tree as it found it; this one does not.
    ///
    /// `carries_content: false`: the caller supplies a namespace and an
    /// instant, never prose. The content the pass folds is already in the
    /// driver's own buffer, put there by [`Self::runtime_buffer_write`], which
    /// scrubbed it on the way in.
    async fn runtime_summarize(
        &self,
        namespace: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<TreeNode>, MemoryError> {
        self.policy
            .admit_write(Capability::Tree, "tree.runtime_summarize", namespace, false)?;
        self.family()?.runtime_summarize(namespace, timestamp).await
    }

    /// As [`Self::runtime_summarize`], on [`Self::cascade`]'s terms.
    async fn runtime_rebuild(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        self.policy
            .admit_write(Capability::Tree, "tree.runtime_rebuild", namespace, false)?;
        self.family()?.runtime_rebuild(namespace).await
    }

    /// A compiled profile read. The scope is the caller's naming scheme rather
    /// than a namespace, and it is what this call acts on, so it is what the
    /// admission names — the reasoning [`Self::flush_source_tree`] gives for
    /// admitting against its own label instead of `NO_NAMESPACE`.
    async fn flavour_profile(&self, scope: &str) -> Result<Option<String>, MemoryError> {
        self.policy
            .admit_read(Capability::Tree, "tree.flavour_profile", scope, false)?;
        self.family()?.flavour_profile(scope).await
    }
}
