use std::sync::{Arc, Mutex};

use crate::openhuman::memory::api::capabilities::Capabilities;
use crate::openhuman::memory::api::chunks::Chunk;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::goals::GoalsDoc;
use crate::openhuman::memory::api::health::MemoryHealth;
use crate::openhuman::memory::api::provider::sessions::{
    CodingSessionIngestReport, CodingSessionIngestRequest, CodingSessionSource,
};
use crate::openhuman::memory::api::provider::sync::{
    RawArchiveCoverage, RawRebuildOutcome, SourceSyncState, SourceSyncStatus, SyncAuditEntry,
    SyncRunOutcome,
};
use crate::openhuman::memory::api::provider::types::{
    DiffReport, EntityHit, ExportPage, ExportRecord, ImportOutcome, IngestItem, IngestOutcome,
    MaintenanceReport, SnapshotRef, SourceItem, SourceScope,
};
use crate::openhuman::memory::api::provider::{
    AddressBookSeedOutcome, ChunkDetail, ChunkEmbedding, ChunkQuery, CoverWindowQuery, EntityMatch,
    EpisodicEvent, FacetType, FastRetrieveQuery, MemoryChunks, MemoryCodingSessions, MemoryCore,
    MemoryDiff, MemoryDocuments, MemoryEntities, MemoryEpisodic, MemoryGoals, MemoryGraph,
    MemoryIngest, MemoryMaintenance, MemoryPeople, MemoryPortability, MemoryProfile,
    MemoryProvider, MemoryRecall, MemoryRetrieval, MemoryScoring, MemorySourceSink,
    MemorySourceSync, MemoryToolMemory, MemoryTree, PersonHandle, PersonInteraction, PersonRecord,
    PersonScore, ProfileFacet, RankedPerson, ResolvedPerson, RetrievalHit, RetrievalResponse,
    SourceRetrievalQuery, UserState,
};
use crate::openhuman::memory::api::recall::OwnedRecallOpts;
use crate::openhuman::memory::api::tool_memory::ToolMemoryRule;
use crate::openhuman::memory::api::tree::{IngestRequest, QueryResult, TreeStatus};
use crate::openhuman::memory::api::types::{
    GraphRelationRecord, MemoryCategory, MemoryEntry, MemoryKvRecord, MemoryTaint,
    NamespaceDocumentInput, NamespaceMemoryHit, NamespaceRetrievalContext, NamespaceSummary,
    StoredMemoryDocument,
};
use async_trait::async_trait;

/// One call that reached the driver.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub method: String,
    /// Content the driver was handed, when the method carries any.
    pub content: Option<String>,
    /// Provenance the driver was handed, when the method carries any.
    pub taint: Option<MemoryTaint>,
    /// Whether the method received a `Some(scope)`.
    pub scoped: Option<bool>,
}

/// The scope's allow list rendered for assertions, sorted for determinism.
fn rendered_scope(scope: Option<&SourceScope>) -> Option<String> {
    scope.map(|s| {
        let mut allow = s.allow.clone();
        allow.sort();
        allow.join(",")
    })
}

impl Call {
    fn plain(method: &str) -> Self {
        Self {
            method: method.into(),
            content: None,
            taint: None,
            scoped: None,
        }
    }
}

/// A provider that records and answers with empties.
pub struct RecordingProvider {
    calls: Mutex<Vec<Call>>,
    /// What `recall` returns, so budget tests can drive a known result set.
    recall_result: Mutex<Vec<MemoryEntry>>,
}

impl Default for RecordingProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingProvider {
    pub fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            recall_result: Mutex::new(Vec::new()),
        }
    }

    pub fn with_recall_result(self, entries: Vec<MemoryEntry>) -> Self {
        *self.recall_result.lock().unwrap() = entries;
        self
    }

    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// The single recorded call, panicking when there is not exactly one.
    pub fn only_call(&self) -> Call {
        let calls = self.calls();
        assert_eq!(
            calls.len(),
            1,
            "expected exactly one driver call: {calls:?}"
        );
        calls.into_iter().next().unwrap()
    }
}

/// A [`GuardPolicy`](super::GuardPolicy) over an embedded driver with default
/// budgets — the shipped configuration.
pub fn embedded_policy() -> super::GuardPolicy {
    super::GuardPolicy::new(
        "recording",
        crate::core::subsystem::DriverClass::Embedded,
        crate::openhuman::config::schema::MemoryHooksConfig::default(),
        super::policy::TRUSTED,
    )
}

/// A policy over an *external* driver. No such driver can bind today
/// (`binding::admit` refuses them), so this is the only way to reach the class
/// branches that land for real in M6.
pub fn external_policy(trust_state: &str) -> super::GuardPolicy {
    super::GuardPolicy::new(
        "supermemory",
        crate::core::subsystem::DriverClass::External,
        crate::openhuman::config::schema::MemoryHooksConfig::default(),
        trust_state,
    )
}

/// An [`ExportRecord`] fixture.
pub fn export_record(taint: MemoryTaint) -> ExportRecord {
    ExportRecord {
        kind: "entry".into(),
        id: "r1".into(),
        namespace: Some("ns".into()),
        taint,
        payload: serde_json::Value::Null,
    }
}

/// A guard over a fresh recording provider, plus a handle on that provider.
pub fn guarded(policy: super::GuardPolicy) -> (Arc<RecordingProvider>, super::MemoryGuard) {
    guarded_with(RecordingProvider::new(), policy)
}

/// As [`guarded`], over a caller-configured provider.
pub fn guarded_with(
    provider: RecordingProvider,
    policy: super::GuardPolicy,
) -> (Arc<RecordingProvider>, super::MemoryGuard) {
    let provider = Arc::new(provider);
    let guard = super::MemoryGuard::new(
        Arc::clone(&provider) as Arc<dyn MemoryProvider>,
        Arc::new(policy),
    );
    (provider, guard)
}

/// A [`MemoryEntry`] fixture.
pub fn entry(content: &str) -> MemoryEntry {
    MemoryEntry {
        id: "id".into(),
        key: "key".into(),
        content: content.into(),
        namespace: Some("ns".into()),
        category: MemoryCategory::Core,
        timestamp: "2026-01-01T00:00:00Z".into(),
        session_id: None,
        score: None,
        taint: MemoryTaint::Internal,
    }
}

/// A [`TreeStatus`] fixture.
fn tree_status(namespace: &str) -> TreeStatus {
    TreeStatus {
        namespace: namespace.to_string(),
        total_nodes: 0,
        depth: 0,
        oldest_entry: None,
        newest_entry: None,
        last_run_at: None,
    }
}

/// A [`NamespaceDocumentInput`] fixture.
pub fn document(content: &str, taint: MemoryTaint) -> NamespaceDocumentInput {
    NamespaceDocumentInput {
        namespace: "ns".into(),
        key: "k".into(),
        title: "t".into(),
        content: content.into(),
        source_type: "chat".into(),
        priority: "normal".into(),
        tags: vec![],
        metadata: serde_json::Value::Null,
        category: "core".into(),
        session_id: None,
        document_id: None,
        taint,
    }
}

#[async_trait]
impl MemoryCore for RecordingProvider {
    async fn store(
        &self,
        _namespace: &str,
        _key: &str,
        content: &str,
        _category: MemoryCategory,
        _session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> Result<(), MemoryError> {
        self.record(Call {
            method: "core.store".into(),
            content: Some(content.to_string()),
            taint: Some(taint),
            scoped: None,
        });
        Ok(())
    }

    async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        self.record(Call::plain("core.get"));
        Ok(None)
    }

    async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool, MemoryError> {
        self.record(Call::plain("core.forget"));
        Ok(false)
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.record(Call::plain("core.list"));
        Ok(vec![])
    }

    async fn namespaces(&self) -> Result<Vec<NamespaceSummary>, MemoryError> {
        self.record(Call::plain("core.namespaces"));
        Ok(vec![])
    }
}

#[async_trait]
impl MemoryRecall for RecordingProvider {
    async fn recall(
        &self,
        query: &str,
        _limit: usize,
        _opts: &OwnedRecallOpts,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.record(Call {
            method: "recall.recall".into(),
            content: Some(query.to_string()),
            taint: None,
            scoped: Some(scope.is_some()),
        });
        Ok(self.recall_result.lock().unwrap().clone())
    }
}

#[async_trait]
impl MemoryPortability for RecordingProvider {
    async fn export_page(
        &self,
        _cursor: Option<&str>,
        _limit: usize,
    ) -> Result<ExportPage, MemoryError> {
        self.record(Call::plain("portability.export_page"));
        Ok(ExportPage::default())
    }

    async fn import_records(
        &self,
        records: Vec<ExportRecord>,
    ) -> Result<ImportOutcome, MemoryError> {
        self.record(Call {
            method: "portability.import_records".into(),
            content: None,
            taint: records.first().map(|r| r.taint),
            scoped: None,
        });
        Ok(ImportOutcome::default())
    }
}

#[async_trait]
impl MemoryIngest for RecordingProvider {
    async fn ingest_document(&self, item: IngestItem) -> Result<IngestOutcome, MemoryError> {
        self.record(Call {
            method: "ingest.ingest_document".into(),
            content: Some(item.content),
            taint: Some(item.taint),
            scoped: None,
        });
        Ok(IngestOutcome::default())
    }

    async fn ingest_chat(&self, messages: Vec<IngestItem>) -> Result<IngestOutcome, MemoryError> {
        self.record(Call {
            method: "ingest.ingest_chat".into(),
            content: messages.first().map(|m| m.content.clone()),
            taint: messages.first().map(|m| m.taint),
            scoped: None,
        });
        Ok(IngestOutcome::default())
    }
}

#[async_trait]
impl MemoryDocuments for RecordingProvider {
    async fn put_document(&self, input: NamespaceDocumentInput) -> Result<String, MemoryError> {
        self.record(Call {
            method: "documents.put_document".into(),
            content: Some(input.content),
            taint: Some(input.taint),
            scoped: None,
        });
        Ok("doc".into())
    }

    async fn get_document(
        &self,
        _namespace: &str,
        _key: &str,
    ) -> Result<Option<StoredMemoryDocument>, MemoryError> {
        self.record(Call::plain("documents.get_document"));
        Ok(None)
    }

    async fn list_documents(
        &self,
        _namespace: Option<&str>,
    ) -> Result<serde_json::Value, MemoryError> {
        self.record(Call::plain("documents.list_documents"));
        Ok(serde_json::json!({"documents": []}))
    }

    async fn list_namespaces(&self) -> Result<Vec<String>, MemoryError> {
        self.record(Call::plain("documents.list_namespaces"));
        Ok(vec![])
    }

    async fn delete_document(
        &self,
        _namespace: &str,
        _document_id: &str,
    ) -> Result<serde_json::Value, MemoryError> {
        self.record(Call::plain("documents.delete_document"));
        Ok(serde_json::json!({"deleted": false}))
    }

    async fn clear_namespace(&self, _namespace: &str) -> Result<(), MemoryError> {
        self.record(Call::plain("documents.clear_namespace"));
        Ok(())
    }

    async fn query_documents(
        &self,
        namespace: &str,
        query: &str,
        _limit: usize,
    ) -> Result<NamespaceRetrievalContext, MemoryError> {
        self.record(Call {
            method: "documents.query_documents".into(),
            content: Some(query.to_string()),
            taint: None,
            scoped: None,
        });
        Ok(NamespaceRetrievalContext {
            namespace: namespace.to_string(),
            query: Some(query.to_string()),
            context_text: String::new(),
            hits: vec![],
        })
    }

    async fn recall_documents(
        &self,
        namespace: &str,
        _limit: usize,
    ) -> Result<NamespaceRetrievalContext, MemoryError> {
        self.record(Call::plain("documents.recall_documents"));
        Ok(NamespaceRetrievalContext {
            namespace: namespace.to_string(),
            query: None,
            context_text: String::new(),
            hits: vec![],
        })
    }
}

#[async_trait]
impl MemoryTree for RecordingProvider {
    async fn append(&self, request: IngestRequest) -> Result<(), MemoryError> {
        self.record(Call {
            method: "tree.append".into(),
            content: Some(request.content),
            taint: None,
            scoped: None,
        });
        Ok(())
    }

    async fn query_source(
        &self,
        _namespace: &str,
        _source_id: &str,
        _limit: usize,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<Chunk>, MemoryError> {
        self.record(Call {
            method: "tree.query_source".into(),
            // The scope's allow list, rendered so a test can assert which one
            // arrived. Sorted because it comes from a `HashSet`.
            content: scope.map(|s| {
                let mut allow = s.allow.clone();
                allow.sort();
                allow.join(",")
            }),
            taint: None,
            scoped: Some(scope.is_some()),
        });
        Ok(vec![])
    }

    async fn drill_down(
        &self,
        _namespace: &str,
        _node_id: &str,
    ) -> Result<QueryResult, MemoryError> {
        self.record(Call::plain("tree.drill_down"));
        Err(MemoryError::NotFound("node".into()))
    }

    async fn seal(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        self.record(Call::plain("tree.seal"));
        Ok(tree_status(namespace))
    }

    async fn cascade(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        self.record(Call::plain("tree.cascade"));
        Ok(tree_status(namespace))
    }

    /// Records the folded bodies as one blob, so a redaction test can assert on
    /// what the driver's summariser would have been handed.
    async fn summarise(
        &self,
        inputs: &[crate::openhuman::memory::api::provider::content::SummaryInput],
        _context: &crate::openhuman::memory::api::provider::content::SummaryContext,
    ) -> Result<crate::openhuman::memory::api::provider::content::SummaryOutput, MemoryError> {
        self.record(Call {
            method: "tree.summarise".into(),
            content: Some(
                inputs
                    .iter()
                    .map(|input| input.content.clone())
                    .collect::<Vec<_>>()
                    .join("|"),
            ),
            taint: None,
            scoped: None,
        });
        Ok(Default::default())
    }

    async fn root_summaries_with_caps(
        &self,
        _per_namespace_cap: usize,
        _total_cap: usize,
    ) -> Result<Vec<crate::openhuman::memory::api::provider::content::RootSummary>, MemoryError>
    {
        self.record(Call::plain("tree.root_summaries_with_caps"));
        Ok(Vec::new())
    }

    // ── The runtime-tree and flavour doors ──────────────────────────────────
    //
    // Overridden for the same reason `summarise` and `root_summaries_with_caps`
    // are: each is defaulted on the trait, so a `GuardedTree` that forgot to
    // forward one still compiles and answers `Unsupported`. A driver that
    // *succeeds* here is what makes `the_defaulted_doors_are_forwarded_rather_than_refused`
    // able to tell the two apart.

    /// Records the buffered body, so a redaction test can assert what the
    /// driver's buffer would have been handed — [`Self::append`]'s twin.
    async fn runtime_buffer_write(
        &self,
        _namespace: &str,
        content: &str,
        _timestamp: chrono::DateTime<chrono::Utc>,
        _metadata: Option<serde_json::Value>,
    ) -> Result<String, MemoryError> {
        self.record(Call {
            method: "tree.runtime_buffer_write".into(),
            content: Some(content.to_string()),
            taint: None,
            scoped: None,
        });
        Ok("/buffer/2026/01/01/00.md".to_string())
    }

    async fn runtime_read_node(
        &self,
        _namespace: &str,
        _node_id: &str,
    ) -> Result<Option<crate::openhuman::memory::api::tree::TreeNode>, MemoryError> {
        self.record(Call::plain("tree.runtime_read_node"));
        Ok(None)
    }

    async fn runtime_read_children(
        &self,
        _namespace: &str,
        _parent_id: &str,
    ) -> Result<Vec<crate::openhuman::memory::api::tree::TreeNode>, MemoryError> {
        self.record(Call::plain("tree.runtime_read_children"));
        Ok(Vec::new())
    }

    async fn runtime_tree_status(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        self.record(Call::plain("tree.runtime_tree_status"));
        Ok(tree_status(namespace))
    }

    async fn runtime_summarize(
        &self,
        _namespace: &str,
        _timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<crate::openhuman::memory::api::tree::TreeNode>, MemoryError> {
        self.record(Call::plain("tree.runtime_summarize"));
        Ok(None)
    }

    async fn runtime_rebuild(&self, namespace: &str) -> Result<TreeStatus, MemoryError> {
        self.record(Call::plain("tree.runtime_rebuild"));
        Ok(tree_status(namespace))
    }

    async fn flavour_profile(&self, _scope: &str) -> Result<Option<String>, MemoryError> {
        self.record(Call::plain("tree.flavour_profile"));
        Ok(None)
    }
}

#[async_trait]
impl MemoryEntities for RecordingProvider {
    async fn entities(
        &self,
        _namespace: &str,
        _query: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<EntityHit>, MemoryError> {
        self.record(Call::plain("entities.entities"));
        Ok(vec![])
    }

    async fn entity_edges(
        &self,
        _namespace: &str,
        _entity_id: &str,
        _limit: usize,
    ) -> Result<Vec<GraphRelationRecord>, MemoryError> {
        self.record(Call::plain("entities.entity_edges"));
        Ok(vec![])
    }

    async fn touch_entities(
        &self,
        _namespace: &str,
        _entity_ids: &[String],
    ) -> Result<(), MemoryError> {
        self.record(Call::plain("entities.touch_entities"));
        Ok(())
    }
}

#[async_trait]
impl MemoryGraph for RecordingProvider {
    async fn kv_get(
        &self,
        _namespace: Option<&str>,
        _key: &str,
    ) -> Result<Option<MemoryKvRecord>, MemoryError> {
        self.record(Call::plain("graph.kv_get"));
        Ok(None)
    }

    async fn kv_put(
        &self,
        _namespace: Option<&str>,
        _key: &str,
        value: serde_json::Value,
    ) -> Result<(), MemoryError> {
        self.record(Call {
            method: "graph.kv_put".into(),
            content: Some(value.to_string()),
            taint: None,
            scoped: None,
        });
        Ok(())
    }

    async fn kv_delete(&self, _namespace: Option<&str>, _key: &str) -> Result<bool, MemoryError> {
        self.record(Call::plain("graph.kv_delete"));
        Ok(false)
    }

    async fn kv_list(
        &self,
        _namespace: Option<&str>,
        _prefix: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<MemoryKvRecord>, MemoryError> {
        self.record(Call::plain("graph.kv_list"));
        Ok(vec![])
    }

    async fn relations(
        &self,
        _namespace: Option<&str>,
        _subject: Option<&str>,
        _predicate: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<GraphRelationRecord>, MemoryError> {
        self.record(Call::plain("graph.relations"));
        Ok(vec![])
    }

    async fn put_relation(&self, _relation: GraphRelationRecord) -> Result<(), MemoryError> {
        self.record(Call::plain("graph.put_relation"));
        Ok(())
    }
}

#[async_trait]
impl MemoryDiff for RecordingProvider {
    async fn capture_snapshot(&self, _source_id: &str) -> Result<SnapshotRef, MemoryError> {
        self.record(Call::plain("diff.capture_snapshot"));
        Err(MemoryError::NotFound("source".into()))
    }

    async fn snapshots(
        &self,
        _source_id: &str,
        _limit: usize,
    ) -> Result<Vec<SnapshotRef>, MemoryError> {
        self.record(Call::plain("diff.snapshots"));
        Ok(vec![])
    }

    async fn diff(
        &self,
        _source_id: &str,
        _from: Option<&str>,
        _to: &str,
    ) -> Result<DiffReport, MemoryError> {
        self.record(Call::plain("diff.diff"));
        Err(MemoryError::NotFound("snapshot".into()))
    }
}

#[async_trait]
impl MemoryGoals for RecordingProvider {
    async fn goals(&self) -> Result<GoalsDoc, MemoryError> {
        self.record(Call::plain("goals.goals"));
        Ok(GoalsDoc::default())
    }

    async fn set_goals(&self, _goals: GoalsDoc) -> Result<(), MemoryError> {
        self.record(Call::plain("goals.set_goals"));
        Ok(())
    }
}

#[async_trait]
impl MemoryToolMemory for RecordingProvider {
    async fn tool_rules(&self, _tool_name: &str) -> Result<Vec<ToolMemoryRule>, MemoryError> {
        self.record(Call::plain("tool_memory.tool_rules"));
        Ok(vec![])
    }

    async fn put_tool_rule(&self, _rule: ToolMemoryRule) -> Result<(), MemoryError> {
        self.record(Call::plain("tool_memory.put_tool_rule"));
        Ok(())
    }

    async fn delete_tool_rule(
        &self,
        _tool_name: &str,
        _rule_id: &str,
    ) -> Result<bool, MemoryError> {
        self.record(Call::plain("tool_memory.delete_tool_rule"));
        Ok(false)
    }
}
