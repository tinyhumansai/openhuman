//! [`MemoryGuard`] — the kernel-owned policy decorator over a bound driver.

use std::sync::Arc;

use crate::openhuman::memory::api::capabilities::{Capabilities, Capability};
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::health::MemoryHealth;
use crate::openhuman::memory::api::provider::operations::{
    MemoryAnswer, MemoryConversationIngest, MemoryDocumentIngest, MemoryEventIngest,
    MemoryLearningIngest,
};
use crate::openhuman::memory::api::provider::scoring::MemoryScoring;
use crate::openhuman::memory::api::provider::{
    MemoryChunks, MemoryCodingSessions, MemoryDiff, MemoryDocuments, MemoryEntities,
    MemoryEpisodic, MemoryGoals, MemoryGraph, MemoryIngest, MemoryMaintenance, MemoryPeople,
    MemoryProfile, MemoryProvider, MemoryRetrieval, MemorySourceSink, MemorySourceSync,
    MemoryToolMemory, MemoryTree,
};
use async_trait::async_trait;

use super::families::{
    GuardedAnswer, GuardedChunks, GuardedCodingSessions, GuardedConversationIngest, GuardedDiff,
    GuardedDocumentIngest, GuardedDocuments, GuardedEntities, GuardedEpisodic, GuardedEventIngest,
    GuardedGoals, GuardedGraph, GuardedIngest, GuardedLearningIngest, GuardedMaintenance,
    GuardedPeople, GuardedProfile, GuardedRetrieval, GuardedScoring, GuardedSourceSync,
    GuardedSources, GuardedToolMemory, GuardedTree,
};
use super::policy::GuardPolicy;

/// The policy decorator every product caller receives instead of the raw
/// driver (`docs/specs/plan-memory.md` §3.4, `docs/specs/kernel.md` §3.4).
///
/// It implements [`MemoryProvider`], so it is transparent to callers and cannot
/// be "skipped" by a caller that simply keeps using the contract — there is no
/// second, unguarded shape to hold. Its fourteen `as_*` overrides hand back
/// **guarded** family handles rather than the inner driver's, which is what
/// closes the accessor bypass; see [`super::families`] for why that forces the
/// decorators to be owned fields.
pub struct MemoryGuard {
    inner: Arc<dyn MemoryProvider>,
    policy: Arc<GuardPolicy>,

    // The fourteen optional families. Each is `Some` **iff** the inner driver
    // provides it, so `provides()` — which the contract's `audit_provider`
    // compares against `capabilities()` — answers identically for the guard and
    // for the driver underneath it.
    ingest: Option<GuardedIngest>,
    documents: Option<GuardedDocuments>,
    tree: Option<GuardedTree>,
    entities: Option<GuardedEntities>,
    graph: Option<GuardedGraph>,
    diff: Option<GuardedDiff>,
    goals: Option<GuardedGoals>,
    tool_memory: Option<GuardedToolMemory>,
    sources: Option<GuardedSources>,
    maintenance: Option<GuardedMaintenance>,
    people: Option<GuardedPeople>,
    chunks: Option<GuardedChunks>,
    retrieval: Option<GuardedRetrieval>,
    profile: Option<GuardedProfile>,
    episodic: Option<GuardedEpisodic>,
    source_sync: Option<GuardedSourceSync>,
    coding_sessions: Option<GuardedCodingSessions>,
    scoring: Option<GuardedScoring>,
    document_ingest: Option<GuardedDocumentIngest>,
    conversation_ingest: Option<GuardedConversationIngest>,
    learning_ingest: Option<GuardedLearningIngest>,
    event_ingest: Option<GuardedEventIngest>,
    answer: Option<GuardedAnswer>,
}

impl MemoryGuard {
    /// Wrap `inner` in `policy`.
    ///
    /// Builds all fourteen decorators up front. That is not an optimisation: the
    /// `as_*` accessors return borrows, so a decorator constructed inside an
    /// accessor could not outlive the call.
    pub fn new(inner: Arc<dyn MemoryProvider>, policy: Arc<GuardPolicy>) -> Self {
        macro_rules! family {
            ($cap:ident, $ty:ident) => {
                inner
                    .provides(Capability::$cap)
                    .then(|| $ty::new(Arc::clone(&inner), Arc::clone(&policy)))
            };
        }
        Self {
            ingest: family!(Ingest, GuardedIngest),
            documents: family!(Documents, GuardedDocuments),
            tree: family!(Tree, GuardedTree),
            entities: family!(Entities, GuardedEntities),
            graph: family!(Graph, GuardedGraph),
            diff: family!(Diff, GuardedDiff),
            goals: family!(Goals, GuardedGoals),
            tool_memory: family!(ToolMemory, GuardedToolMemory),
            sources: family!(Sources, GuardedSources),
            maintenance: family!(Maintenance, GuardedMaintenance),
            people: family!(People, GuardedPeople),
            chunks: family!(Chunks, GuardedChunks),
            retrieval: family!(Retrieval, GuardedRetrieval),
            profile: family!(Profile, GuardedProfile),
            episodic: family!(Episodic, GuardedEpisodic),
            source_sync: family!(SourceSync, GuardedSourceSync),
            coding_sessions: family!(CodingSessions, GuardedCodingSessions),
            scoring: family!(Scoring, GuardedScoring),
            document_ingest: family!(DocumentIngest, GuardedDocumentIngest),
            conversation_ingest: family!(ConversationIngest, GuardedConversationIngest),
            learning_ingest: family!(LearningIngest, GuardedLearningIngest),
            event_ingest: family!(EventIngest, GuardedEventIngest),
            answer: family!(Answer, GuardedAnswer),
            inner,
            policy,
        }
    }

    /// The policy this guard enforces.
    pub fn policy(&self) -> &Arc<GuardPolicy> {
        &self.policy
    }

    /// The wrapped driver.
    ///
    /// `pub(crate)` on purpose: handing this out is exactly the bypass the
    /// guard exists to prevent, and the only legitimate use is inside the
    /// memory subsystem itself (identity, health, tests). Do not widen it.
    pub(crate) fn inner(&self) -> &Arc<dyn MemoryProvider> {
        &self.inner
    }
}

#[async_trait]
impl MemoryProvider for MemoryGuard {
    /// The **wrapped driver's** id, not a synthetic `"guard"`. The guard is a
    /// policy layer, not a driver: status output, spans, and audit events all
    /// name the thing that actually stores the bytes.
    fn driver_id(&self) -> &str {
        self.inner.driver_id()
    }

    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    async fn health(&self) -> MemoryHealth {
        self.inner.health().await
    }

    async fn shutdown(&self) -> Result<(), MemoryError> {
        self.inner.shutdown().await
    }

    fn as_document_ingest(&self) -> Option<&dyn MemoryDocumentIngest> {
        self.document_ingest
            .as_ref()
            .map(|g| g as &dyn MemoryDocumentIngest)
    }

    fn as_conversation_ingest(&self) -> Option<&dyn MemoryConversationIngest> {
        self.conversation_ingest
            .as_ref()
            .map(|g| g as &dyn MemoryConversationIngest)
    }

    fn as_learning_ingest(&self) -> Option<&dyn MemoryLearningIngest> {
        self.learning_ingest
            .as_ref()
            .map(|g| g as &dyn MemoryLearningIngest)
    }

    fn as_event_ingest(&self) -> Option<&dyn MemoryEventIngest> {
        self.event_ingest
            .as_ref()
            .map(|g| g as &dyn MemoryEventIngest)
    }

    fn as_answer(&self) -> Option<&dyn MemoryAnswer> {
        self.answer.as_ref().map(|g| g as &dyn MemoryAnswer)
    }

    fn as_ingest(&self) -> Option<&dyn MemoryIngest> {
        self.ingest.as_ref().map(|g| g as &dyn MemoryIngest)
    }

    fn as_documents(&self) -> Option<&dyn MemoryDocuments> {
        self.documents.as_ref().map(|g| g as &dyn MemoryDocuments)
    }

    fn as_tree(&self) -> Option<&dyn MemoryTree> {
        self.tree.as_ref().map(|g| g as &dyn MemoryTree)
    }

    fn as_entities(&self) -> Option<&dyn MemoryEntities> {
        self.entities.as_ref().map(|g| g as &dyn MemoryEntities)
    }

    fn as_graph(&self) -> Option<&dyn MemoryGraph> {
        self.graph.as_ref().map(|g| g as &dyn MemoryGraph)
    }

    fn as_diff(&self) -> Option<&dyn MemoryDiff> {
        self.diff.as_ref().map(|g| g as &dyn MemoryDiff)
    }

    fn as_goals(&self) -> Option<&dyn MemoryGoals> {
        self.goals.as_ref().map(|g| g as &dyn MemoryGoals)
    }

    fn as_tool_memory(&self) -> Option<&dyn MemoryToolMemory> {
        self.tool_memory
            .as_ref()
            .map(|g| g as &dyn MemoryToolMemory)
    }

    fn as_sources(&self) -> Option<&dyn MemorySourceSink> {
        self.sources.as_ref().map(|g| g as &dyn MemorySourceSink)
    }

    fn as_maintenance(&self) -> Option<&dyn MemoryMaintenance> {
        self.maintenance
            .as_ref()
            .map(|g| g as &dyn MemoryMaintenance)
    }

    fn as_people(&self) -> Option<&dyn MemoryPeople> {
        self.people.as_ref().map(|g| g as &dyn MemoryPeople)
    }

    fn as_chunks(&self) -> Option<&dyn MemoryChunks> {
        self.chunks.as_ref().map(|g| g as &dyn MemoryChunks)
    }

    fn as_retrieval(&self) -> Option<&dyn MemoryRetrieval> {
        self.retrieval.as_ref().map(|g| g as &dyn MemoryRetrieval)
    }

    fn as_profile(&self) -> Option<&dyn MemoryProfile> {
        self.profile.as_ref().map(|g| g as &dyn MemoryProfile)
    }

    fn as_episodic(&self) -> Option<&dyn MemoryEpisodic> {
        self.episodic.as_ref().map(|g| g as &dyn MemoryEpisodic)
    }

    fn as_source_sync(&self) -> Option<&dyn MemorySourceSync> {
        self.source_sync
            .as_ref()
            .map(|g| g as &dyn MemorySourceSync)
    }

    fn as_coding_sessions(&self) -> Option<&dyn MemoryCodingSessions> {
        self.coding_sessions
            .as_ref()
            .map(|g| g as &dyn MemoryCodingSessions)
    }
    fn as_scoring(&self) -> Option<&dyn MemoryScoring> {
        self.scoring.as_ref().map(|g| g as &dyn MemoryScoring)
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
