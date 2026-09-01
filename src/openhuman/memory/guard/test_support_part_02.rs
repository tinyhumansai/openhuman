#[async_trait]
impl MemorySourceSink for RecordingProvider {
    async fn accept_source_items(
        &self,
        _source_id: &str,
        _source_kind: &str,
        items: Vec<SourceItem>,
        taint: MemoryTaint,
    ) -> Result<IngestOutcome, MemoryError> {
        self.record(Call {
            method: "sources.accept_source_items".into(),
            content: items.first().map(|i| i.content.clone()),
            taint: Some(taint),
            scoped: None,
        });
        Ok(IngestOutcome::default())
    }

    async fn forget_source(&self, _source_id: &str) -> Result<u64, MemoryError> {
        self.record(Call::plain("sources.forget_source"));
        Ok(0)
    }
}

#[async_trait]
impl MemoryMaintenance for RecordingProvider {
    async fn reembed(&self) -> Result<MaintenanceReport, MemoryError> {
        self.record(Call::plain("maintenance.reembed"));
        Ok(MaintenanceReport::default())
    }

    async fn compact(&self) -> Result<MaintenanceReport, MemoryError> {
        self.record(Call::plain("maintenance.compact"));
        Ok(MaintenanceReport::default())
    }

    async fn consolidate(&self) -> Result<MaintenanceReport, MemoryError> {
        self.record(Call::plain("maintenance.consolidate"));
        Ok(MaintenanceReport::default())
    }

    async fn doctor(&self) -> Result<MaintenanceReport, MemoryError> {
        self.record(Call::plain("maintenance.doctor"));
        Ok(MaintenanceReport::default())
    }

    async fn diagnose(
        &self,
    ) -> Result<crate::openhuman::memory::api::provider::diagnosis::Diagnosis, MemoryError> {
        self.record(Call::plain("maintenance.diagnose"));
        Ok(Default::default())
    }

    async fn degraded_state(
        &self,
    ) -> Result<crate::openhuman::memory::api::provider::diagnosis::DegradedCapabilities, MemoryError>
    {
        self.record(Call::plain("maintenance.degraded_state"));
        Ok(Default::default())
    }
}

#[async_trait]
impl MemoryProvider for RecordingProvider {
    fn driver_id(&self) -> &str {
        "recording"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::all()
    }

    async fn health(&self) -> MemoryHealth {
        MemoryHealth::Ready
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
    fn as_people(&self) -> Option<&dyn MemoryPeople> {
        Some(self)
    }
    fn as_chunks(&self) -> Option<&dyn MemoryChunks> {
        Some(self)
    }
    fn as_retrieval(&self) -> Option<&dyn MemoryRetrieval> {
        Some(self)
    }
    fn as_profile(&self) -> Option<&dyn MemoryProfile> {
        Some(self)
    }
    fn as_episodic(&self) -> Option<&dyn MemoryEpisodic> {
        Some(self)
    }
    fn as_source_sync(&self) -> Option<&dyn MemorySourceSync> {
        Some(self)
    }
    fn as_coding_sessions(&self) -> Option<&dyn MemoryCodingSessions> {
        Some(self)
    }
    fn as_scoring(&self) -> Option<&dyn MemoryScoring> {
        Some(self)
    }
}

// The two families tinymemory v1.7.0 added. `capabilities()` above answers
// `Capabilities::all()`, so a driver that advertises them and then hands back
// `None` from the accessor is exactly the inconsistency `audit_provider`
// exists to catch — the recorder has to serve them to stay honest.

#[async_trait]
impl MemorySourceSync for RecordingProvider {
    async fn run_connection_sync(
        &self,
        toolkit: &str,
        connection_id: &str,
    ) -> Result<SyncRunOutcome, MemoryError> {
        self.record(Call::plain("source_sync.run_connection_sync"));
        let _ = (toolkit, connection_id);
        Ok(SyncRunOutcome::default())
    }
    async fn source_sync_state(
        &self,
        toolkit: &str,
        connection_id: &str,
    ) -> Result<Option<SourceSyncState>, MemoryError> {
        self.record(Call::plain("source_sync.source_sync_state"));
        let _ = (toolkit, connection_id);
        Ok(None)
    }
    async fn sync_audit_log(
        &self,
        _limit: Option<usize>,
    ) -> Result<Vec<SyncAuditEntry>, MemoryError> {
        self.record(Call::plain("source_sync.sync_audit_log"));
        Ok(Vec::new())
    }
    async fn estimate_sync_cost_usd(
        &self,
        _input_tokens: u64,
        _output_tokens: u64,
    ) -> Result<f64, MemoryError> {
        self.record(Call::plain("source_sync.estimate_sync_cost_usd"));
        Ok(0.0)
    }
    async fn sync_statuses(&self) -> Result<Vec<SourceSyncStatus>, MemoryError> {
        self.record(Call::plain("source_sync.sync_statuses"));
        Ok(Vec::new())
    }
    async fn raw_archive_coverage(
        &self,
        tree_scope: &str,
        archive_source_id: &str,
    ) -> Result<RawArchiveCoverage, MemoryError> {
        self.record(Call::plain("source_sync.raw_archive_coverage"));
        let _ = (tree_scope, archive_source_id);
        Ok(RawArchiveCoverage::default())
    }
    async fn rebuild_from_raw_archive(
        &self,
        tree_scope: &str,
        archive_source_id: &str,
    ) -> Result<RawRebuildOutcome, MemoryError> {
        self.record(Call::plain("source_sync.rebuild_from_raw_archive"));
        let _ = (tree_scope, archive_source_id);
        Ok(RawRebuildOutcome::default())
    }
}

#[async_trait]
impl MemoryCodingSessions for RecordingProvider {
    async fn coding_session_status(&self) -> Result<Vec<CodingSessionSource>, MemoryError> {
        self.record(Call::plain("coding_sessions.coding_session_status"));
        Ok(Vec::new())
    }
    async fn ingest_coding_sessions(
        &self,
        _request: CodingSessionIngestRequest,
    ) -> Result<CodingSessionIngestReport, MemoryError> {
        self.record(Call::plain("coding_sessions.ingest_coding_sessions"));
        Ok(CodingSessionIngestReport::default())
    }
}

#[async_trait]
impl MemoryEpisodic for RecordingProvider {
    async fn insert_turn(
        &self,
        turn: &crate::openhuman::memory::api::provider::episodic::EpisodicTurn,
    ) -> Result<i64, MemoryError> {
        // Records the turn text, so a guard that failed to redact one would be
        // visible here rather than only in a live store.
        self.record(Call {
            method: "episodic.insert_turn".into(),
            content: Some(turn.content.clone()),
            taint: None,
            scoped: None,
        });
        Ok(1)
    }

    async fn session_turns(
        &self,
        _session_id: &str,
    ) -> Result<Vec<crate::openhuman::memory::api::provider::episodic::EpisodicTurn>, MemoryError>
    {
        self.record(Call::plain("episodic.session_turns"));
        Ok(vec![])
    }

    async fn open_segment(
        &self,
        _session_id: &str,
    ) -> Result<
        Option<crate::openhuman::memory::api::provider::episodic::ConversationSegment>,
        MemoryError,
    > {
        self.record(Call::plain("episodic.open_segment"));
        Ok(None)
    }

    async fn create_segment(
        &self,
        _segment_id: &str,
        _session_id: &str,
        _namespace: &str,
        _start_episodic_id: i64,
        _start_seq: Option<u32>,
        _start_timestamp: f64,
        _now: f64,
    ) -> Result<(), MemoryError> {
        self.record(Call::plain("episodic.create_segment"));
        Ok(())
    }

    async fn append_turn(
        &self,
        _segment_id: &str,
        _episodic_id: i64,
        _seq: Option<u32>,
        _timestamp: f64,
        _now: f64,
    ) -> Result<(), MemoryError> {
        self.record(Call::plain("episodic.append_turn"));
        Ok(())
    }

    async fn close_segment(&self, _segment_id: &str, _now: f64) -> Result<(), MemoryError> {
        self.record(Call::plain("episodic.close_segment"));
        Ok(())
    }

    async fn insert_event(&self, event: &EpisodicEvent) -> Result<(), MemoryError> {
        // Records the event text for the same reason `insert_turn` does: a guard
        // that stopped redacting one would otherwise be invisible to every test,
        // and the redaction on this path has already been missing once.
        self.record(Call {
            method: "episodic.insert_event".into(),
            content: Some(event.content.clone()),
            taint: None,
            scoped: None,
        });
        Ok(())
    }

    async fn set_segment_summary(
        &self,
        _segment_id: &str,
        summary: &str,
        _now: f64,
    ) -> Result<(), MemoryError> {
        self.record(Call {
            method: "episodic.set_segment_summary".into(),
            content: Some(summary.to_string()),
            taint: None,
            scoped: None,
        });
        Ok(())
    }

    async fn upsert_segment_embedding(
        &self,
        _segment_id: &str,
        _model_signature: &str,
        _embedding: &[f32],
        _created_at: f64,
    ) -> Result<(), MemoryError> {
        self.record(Call::plain("episodic.upsert_segment_embedding"));
        Ok(())
    }
}
#[async_trait]
impl MemoryProfile for RecordingProvider {
    async fn list_active_facets(&self) -> Result<Vec<ProfileFacet>, MemoryError> {
        self.record(Call::plain("profile.list_active_facets"));
        Ok(vec![])
    }
    async fn list_all_facets(&self) -> Result<Vec<ProfileFacet>, MemoryError> {
        self.record(Call::plain("profile.list_all_facets"));
        Ok(vec![])
    }
    async fn get_facet(&self, _key: &str) -> Result<Option<ProfileFacet>, MemoryError> {
        self.record(Call::plain("profile.get_facet"));
        Ok(None)
    }
    async fn facets_by_type(
        &self,
        _facet_type: FacetType,
    ) -> Result<Vec<ProfileFacet>, MemoryError> {
        self.record(Call::plain("profile.facets_by_type"));
        Ok(vec![])
    }
    async fn upsert_facet(&self, _facet: &ProfileFacet) -> Result<(), MemoryError> {
        self.record(Call::plain("profile.upsert_facet"));
        Ok(())
    }
    async fn upsert_provider_facet(
        &self,
        _facet_id: &str,
        _facet_type: FacetType,
        _key: &str,
        _value: &str,
        _confidence: f64,
        _segment_id: Option<&str>,
        _observed_at: f64,
    ) -> Result<(), MemoryError> {
        self.record(Call::plain("profile.upsert_provider_facet"));
        Ok(())
    }
    async fn set_facet_user_state(
        &self,
        _key: &str,
        _user_state: UserState,
    ) -> Result<bool, MemoryError> {
        self.record(Call::plain("profile.set_facet_user_state"));
        Ok(false)
    }
    async fn delete_facet(&self, _key: &str) -> Result<bool, MemoryError> {
        self.record(Call::plain("profile.delete_facet"));
        Ok(false)
    }
    async fn delete_facet_by_id(&self, _facet_id: &str) -> Result<bool, MemoryError> {
        self.record(Call::plain("profile.delete_facet_by_id"));
        Ok(false)
    }
    async fn drop_facets_below(&self, _threshold: f64) -> Result<usize, MemoryError> {
        self.record(Call::plain("profile.drop_facets_below"));
        Ok(0)
    }
    async fn workflow_identity_matches(&self, _pattern: &str, _value: &str) -> bool {
        self.record(Call::plain("profile.workflow_identity_matches"));
        false
    }
}

#[async_trait]
impl MemoryChunks for RecordingProvider {
    async fn list_chunks(
        &self,
        _query: &ChunkQuery,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<Chunk>, MemoryError> {
        self.record(Call {
            method: "chunks.list_chunks".into(),
            content: rendered_scope(scope),
            taint: None,
            scoped: Some(scope.is_some()),
        });
        Ok(vec![])
    }

    async fn get_chunk(&self, _chunk_id: &str) -> Result<Option<Chunk>, MemoryError> {
        self.record(Call::plain("chunks.get_chunk"));
        Ok(None)
    }

    async fn chunk_detail(&self, _chunk_id: &str) -> Result<Option<ChunkDetail>, MemoryError> {
        self.record(Call::plain("chunks.chunk_detail"));
        Ok(None)
    }

    async fn storage_kinds(&self) -> Result<Vec<String>, MemoryError> {
        self.record(Call::plain("chunks.storage_kinds"));
        Ok(vec![])
    }

    async fn chunk_embeddings(
        &self,
        _chunk_ids: &[String],
        _model_signature: &str,
    ) -> Result<Vec<ChunkEmbedding>, MemoryError> {
        self.record(Call::plain("chunks.chunk_embeddings"));
        Ok(vec![])
    }

    async fn chunk_score(
        &self,
        _chunk_id: &str,
    ) -> Result<Option<crate::openhuman::memory::api::provider::chunks::ChunkScore>, MemoryError>
    {
        self.record(Call::plain("chunks.chunk_score"));
        Ok(None)
    }

    async fn source_ingest_status(
        &self,
        _source_prefixes: &[crate::openhuman::memory::api::provider::chunks::SourceIngestQuery],
    ) -> Result<Vec<crate::openhuman::memory::api::provider::chunks::SourceIngestStatus>, MemoryError>
    {
        self.record(Call::plain("chunks.source_ingest_status"));
        Ok(vec![])
    }
}

#[async_trait]
impl MemoryRetrieval for RecordingProvider {
    async fn fast_retrieve(
        &self,
        _query: &str,
        _options: FastRetrieveQuery,
        scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        self.record(Call {
            method: "retrieval.fast_retrieve".into(),
            content: rendered_scope(scope),
            taint: None,
            scoped: Some(scope.is_some()),
        });
        Ok(RetrievalResponse::default())
    }

    async fn cover_window(
        &self,
        _window: &CoverWindowQuery,
        scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        self.record(Call {
            method: "retrieval.cover_window".into(),
            content: rendered_scope(scope),
            taint: None,
            scoped: Some(scope.is_some()),
        });
        Ok(RetrievalResponse::default())
    }

    async fn retrieve_source(
        &self,
        _query: &SourceRetrievalQuery,
        scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        self.record(Call {
            method: "retrieval.retrieve_source".into(),
            content: rendered_scope(scope),
            taint: None,
            scoped: Some(scope.is_some()),
        });
        Ok(RetrievalResponse::default())
    }

    async fn retrieve_children(
        &self,
        _node_id: &str,
        _max_depth: u32,
        _query: Option<&str>,
        _limit: Option<usize>,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<RetrievalHit>, MemoryError> {
        self.record(Call {
            method: "retrieval.retrieve_children".into(),
            content: rendered_scope(scope),
            taint: None,
            scoped: Some(scope.is_some()),
        });
        Ok(vec![])
    }

    async fn retrieve_leaves(
        &self,
        _chunk_ids: &[String],
        scope: Option<&SourceScope>,
    ) -> Result<Vec<RetrievalHit>, MemoryError> {
        self.record(Call {
            method: "retrieval.retrieve_leaves".into(),
            content: rendered_scope(scope),
            taint: None,
            scoped: Some(scope.is_some()),
        });
        Ok(vec![])
    }

    async fn recall_namespace_scored(
        &self,
        _namespace: &str,
        _query: &str,
        _limit: usize,
        _exclude_session_id: Option<&str>,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        self.record(Call::plain("retrieval.recall_namespace_scored"));
        Ok(vec![])
    }

    async fn recall_namespace_recent(
        &self,
        _namespace: &str,
        _limit: usize,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        self.record(Call::plain("retrieval.recall_namespace_recent"));
        Ok(vec![])
    }

    async fn search_entities(
        &self,
        _query: &str,
        _kinds: Option<&[String]>,
        _limit: usize,
    ) -> Result<Vec<EntityMatch>, MemoryError> {
        self.record(Call::plain("retrieval.search_entities"));
        Ok(vec![])
    }
}

#[async_trait]
impl MemoryPeople for RecordingProvider {
    async fn list_people(&self, _limit: Option<usize>) -> Result<Vec<RankedPerson>, MemoryError> {
        self.record(Call::plain("people.list_people"));
        Ok(vec![])
    }

    async fn get_person(&self, _person_id: &str) -> Result<Option<PersonRecord>, MemoryError> {
        self.record(Call::plain("people.get_person"));
        Ok(None)
    }

    async fn resolve_handle(
        &self,
        _handle: &PersonHandle,
        _create_if_missing: bool,
    ) -> Result<Option<ResolvedPerson>, MemoryError> {
        self.record(Call::plain("people.resolve_handle"));
        Ok(None)
    }

    async fn add_handle_alias(
        &self,
        _person_id: &str,
        _handle: &PersonHandle,
    ) -> Result<(), MemoryError> {
        self.record(Call::plain("people.add_handle_alias"));
        Ok(())
    }

    async fn score_person(&self, _person_id: &str) -> Result<Option<PersonScore>, MemoryError> {
        self.record(Call::plain("people.score_person"));
        Ok(None)
    }

    async fn record_interaction(
        &self,
        _interaction: &PersonInteraction,
    ) -> Result<(), MemoryError> {
        self.record(Call::plain("people.record_interaction"));
        Ok(())
    }

    async fn seed_from_address_book(&self) -> Result<AddressBookSeedOutcome, MemoryError> {
        self.record(Call::plain("people.seed_from_address_book"));
        Ok(AddressBookSeedOutcome::default())
    }
}

#[async_trait]
impl MemoryScoring for RecordingProvider {
    async fn extract_entities(&self, query: &str) -> Result<Vec<String>, MemoryError> {
        self.record(Call {
            method: "scoring.extract_entities".into(),
            content: Some(query.to_string()),
            taint: None,
            scoped: None,
        });
        Ok(Vec::new())
    }

    async fn embed_text(&self, text: &str) -> Result<Vec<f32>, MemoryError> {
        self.record(Call {
            method: "scoring.embed_text".into(),
            content: Some(text.to_string()),
            taint: None,
            scoped: None,
        });
        Ok(Vec::new())
    }

    async fn embedder_slug(&self) -> Result<String, MemoryError> {
        self.record(Call::plain("scoring.embedder_slug"));
        Ok(String::new())
    }
}
