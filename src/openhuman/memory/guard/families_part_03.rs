// ── Chunks ───────────────────────────────────────────────────────────────────

// ── Retrieval ────────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryRetrieval for GuardedRetrieval {
    async fn fast_retrieve(
        &self,
        query: &str,
        options: FastRetrieveQuery,
        scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        self.policy.admit_read(
            Capability::Retrieval,
            "retrieval.fast_retrieve",
            NO_NAMESPACE,
            false,
        )?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?
            .fast_retrieve(query, options, effective.as_ref())
            .await
    }

    async fn cover_window(
        &self,
        window: &CoverWindowQuery,
        scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        self.policy.admit_read(
            Capability::Retrieval,
            "retrieval.cover_window",
            NO_NAMESPACE,
            false,
        )?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?
            .cover_window(window, effective.as_ref())
            .await
    }

    async fn retrieve_source(
        &self,
        query: &SourceRetrievalQuery,
        scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        self.policy.admit_read(
            Capability::Retrieval,
            "retrieval.retrieve_source",
            NO_NAMESPACE,
            false,
        )?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?
            .retrieve_source(query, effective.as_ref())
            .await
    }

    async fn retrieve_children(
        &self,
        node_id: &str,
        max_depth: u32,
        query: Option<&str>,
        limit: Option<usize>,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<RetrievalHit>, MemoryError> {
        self.policy.admit_read(
            Capability::Retrieval,
            "retrieval.retrieve_children",
            NO_NAMESPACE,
            false,
        )?;
        // Intersected with the ambient allowlist, never passed through — same
        // rule as `list_chunks`. See `GuardPolicy::narrow_scope`.
        let effective = self.policy.narrow_scope(scope);
        self.family()?
            .retrieve_children(node_id, max_depth, query, limit, effective.as_ref())
            .await
    }

    async fn retrieve_leaves(
        &self,
        chunk_ids: &[String],
        scope: Option<&SourceScope>,
    ) -> Result<Vec<RetrievalHit>, MemoryError> {
        self.policy.admit_read(
            Capability::Retrieval,
            "retrieval.retrieve_leaves",
            NO_NAMESPACE,
            false,
        )?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?
            .retrieve_leaves(chunk_ids, effective.as_ref())
            .await
    }

    /// Namespace-scoped, so the namespace reaches the tier check — unlike the
    /// other retrieval primitives, which span the store.
    async fn recall_namespace_scored(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
        exclude_session_id: Option<&str>,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        self.policy.admit_read(
            Capability::Retrieval,
            "retrieval.recall_namespace_scored",
            namespace,
            false,
        )?;
        self.family()?
            .recall_namespace_scored(namespace, query, limit, exclude_session_id)
            .await
    }

    /// Namespace-scoped like its scored sibling, and admitted under the same
    /// capability: recency versus ranking is a retrieval mode, not a policy
    /// boundary.
    async fn recall_namespace_recent(
        &self,
        namespace: &str,
        limit: usize,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        self.policy.admit_read(
            Capability::Retrieval,
            "retrieval.recall_namespace_recent",
            namespace,
            false,
        )?;
        self.family()?
            .recall_namespace_recent(namespace, limit)
            .await
    }

    async fn search_entities(
        &self,
        query: &str,
        kinds: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<EntityMatch>, MemoryError> {
        self.policy.admit_read(
            Capability::Retrieval,
            "retrieval.search_entities",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.search_entities(query, kinds, limit).await
    }
}

// ── Profile ──────────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryEpisodic for GuardedEpisodic {
    async fn insert_turn(&self, turn: &EpisodicTurn) -> Result<i64, MemoryError> {
        // A recorded turn is user-authored conversation content, so this is a
        // write and is admitted as one — the read/write split here is about
        // what the tier permits, not about how much data moves.
        // `carries_content: true`, unlike the tier note above, which is about
        // what the tier permits rather than how much data moves. This flag is a
        // different question: it decides whether the egress record classifies
        // the transfer as `FileContent` or `Metadata`. A turn IS the user's
        // prose, and an audit trail that calls a transcript "metadata"
        // understates what left the process — the one thing that record exists
        // to get right. `tree.append` has always passed `true` for this shape.
        self.policy.admit_write(
            Capability::Episodic,
            "episodic.insert_turn",
            NO_NAMESPACE,
            true,
        )?;
        let mut turn = turn.clone();
        turn.content = self.policy.redact_outbound(&turn.content).into_owned();
        self.family()?.insert_turn(&turn).await
    }

    async fn session_turns(&self, session_id: &str) -> Result<Vec<EpisodicTurn>, MemoryError> {
        self.policy.admit_read(
            Capability::Episodic,
            "episodic.session_turns",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.session_turns(session_id).await
    }

    async fn open_segment(
        &self,
        session_id: &str,
    ) -> Result<Option<ConversationSegment>, MemoryError> {
        self.policy.admit_read(
            Capability::Episodic,
            "episodic.open_segment",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.open_segment(session_id).await
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "trait signature; see the contract's rationale"
    )]
    async fn create_segment(
        &self,
        segment_id: &str,
        session_id: &str,
        namespace: &str,
        start_episodic_id: i64,
        start_seq: Option<u32>,
        start_timestamp: f64,
        now: f64,
    ) -> Result<(), MemoryError> {
        // One of the two episodic calls that names a namespace — `insert_event`
        // is the other — so it is admitted against that namespace rather than
        // `NO_NAMESPACE`. The rest of this family addresses a segment by id and
        // has no namespace to check.
        self.policy.admit_write(
            Capability::Episodic,
            "episodic.create_segment",
            namespace,
            false,
        )?;
        self.family()?
            .create_segment(
                segment_id,
                session_id,
                namespace,
                start_episodic_id,
                start_seq,
                start_timestamp,
                now,
            )
            .await
    }

    async fn append_turn(
        &self,
        segment_id: &str,
        episodic_id: i64,
        seq: Option<u32>,
        timestamp: f64,
        now: f64,
    ) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::Episodic,
            "episodic.append_turn",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?
            .append_turn(segment_id, episodic_id, seq, timestamp, now)
            .await
    }

    /// Admitted like its sibling writes; the event's namespace is the
    /// admission subject, since it is the one the record is scoped to.
    async fn insert_event(&self, event: &EpisodicEvent) -> Result<(), MemoryError> {
        // `carries_content: true` for the same reason as `insert_turn`: an
        // extracted event is the user's prose, not a descriptor of it.
        self.policy.admit_write(
            Capability::Episodic,
            "episodic.insert_event",
            &event.namespace,
            true,
        )?;
        let mut event = event.clone();
        event.content = self.policy.redact_outbound(&event.content).into_owned();
        self.family()?.insert_event(&event).await
    }

    async fn close_segment(&self, segment_id: &str, now: f64) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::Episodic,
            "episodic.close_segment",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.close_segment(segment_id, now).await
    }

    async fn set_segment_summary(
        &self,
        segment_id: &str,
        summary: &str,
        now: f64,
    ) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::Episodic,
            "episodic.set_segment_summary",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?
            .set_segment_summary(segment_id, summary, now)
            .await
    }

    /// A read: it selects rows, it changes none. Admitted as one so a
    /// read-only policy can still drive the re-summarisation pass (#6186) —
    /// the write it leads to is `set_segment_summary`, which is admitted
    /// separately on its own terms.
    async fn segments_pending_summary(
        &self,
        limit: u32,
    ) -> Result<Vec<ConversationSegment>, MemoryError> {
        self.policy.admit_read(
            Capability::Episodic,
            "episodic.segments_pending_summary",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.segments_pending_summary(limit).await
    }

    async fn upsert_segment_embedding(
        &self,
        segment_id: &str,
        model_signature: &str,
        embedding: &[f32],
        created_at: f64,
    ) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::Episodic,
            "episodic.upsert_segment_embedding",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?
            .upsert_segment_embedding(segment_id, model_signature, embedding, created_at)
            .await
    }
}

#[async_trait]
impl MemoryProfile for GuardedProfile {
    async fn list_active_facets(&self) -> Result<Vec<ProfileFacet>, MemoryError> {
        self.policy.admit_read(
            Capability::Profile,
            "profile.list_active_facets",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.list_active_facets().await
    }

    async fn list_all_facets(&self) -> Result<Vec<ProfileFacet>, MemoryError> {
        self.policy.admit_read(
            Capability::Profile,
            "profile.list_all_facets",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.list_all_facets().await
    }

    async fn get_facet(&self, key: &str) -> Result<Option<ProfileFacet>, MemoryError> {
        self.policy.admit_read(
            Capability::Profile,
            "profile.get_facet",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.get_facet(key).await
    }

    async fn facets_by_type(
        &self,
        facet_type: FacetType,
    ) -> Result<Vec<ProfileFacet>, MemoryError> {
        self.policy.admit_read(
            Capability::Profile,
            "profile.facets_by_type",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.facets_by_type(facet_type).await
    }

    async fn upsert_facet(&self, facet: &ProfileFacet) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::Profile,
            "profile.upsert_facet",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.upsert_facet(facet).await
    }

    async fn upsert_provider_facet(
        &self,
        facet_id: &str,
        facet_type: FacetType,
        key: &str,
        value: &str,
        confidence: f64,
        segment_id: Option<&str>,
        observed_at: f64,
    ) -> Result<(), MemoryError> {
        self.policy.admit_write(
            Capability::Profile,
            "profile.upsert_provider_facet",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?
            .upsert_provider_facet(
                facet_id,
                facet_type,
                key,
                value,
                confidence,
                segment_id,
                observed_at,
            )
            .await
    }

    async fn set_facet_user_state(
        &self,
        key: &str,
        user_state: UserState,
    ) -> Result<bool, MemoryError> {
        self.policy.admit_write(
            Capability::Profile,
            "profile.set_facet_user_state",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.set_facet_user_state(key, user_state).await
    }

    async fn delete_facet(&self, key: &str) -> Result<bool, MemoryError> {
        self.policy.admit_write(
            Capability::Profile,
            "profile.delete_facet",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.delete_facet(key).await
    }

    async fn delete_facet_by_id(&self, facet_id: &str) -> Result<bool, MemoryError> {
        self.policy.admit_write(
            Capability::Profile,
            "profile.delete_facet_by_id",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.delete_facet_by_id(facet_id).await
    }

    async fn drop_facets_below(&self, threshold: f64) -> Result<usize, MemoryError> {
        self.policy.admit_write(
            Capability::Profile,
            "profile.drop_facets_below",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.drop_facets_below(threshold).await
    }

    /// Refused reads answer `false`, matching the trait's "an error reads as
    /// no". A tier refusal is not evidence that the row matches.
    async fn workflow_identity_matches(&self, key_pattern: &str, canonical_value: &str) -> bool {
        if self
            .policy
            .admit_read(
                Capability::Profile,
                "profile.workflow_identity_matches",
                NO_NAMESPACE,
                false,
            )
            .is_err()
        {
            return false;
        }
        match self.family() {
            Ok(family) => {
                family
                    .workflow_identity_matches(key_pattern, canonical_value)
                    .await
            }
            Err(_) => false,
        }
    }
}

// ── Source sync ──────────────────────────────────────────────────────────────

#[async_trait]
impl MemorySourceSync for GuardedSourceSync {
    /// A write: it fetches from an upstream connector and ingests what it finds.
    /// The tier check is what stops a `readonly` operator triggering one.
    async fn run_connection_sync(
        &self,
        toolkit: &str,
        connection_id: &str,
    ) -> Result<SyncRunOutcome, MemoryError> {
        self.policy.admit_write(
            Capability::SourceSync,
            "source_sync.run_connection_sync",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?
            .run_connection_sync(toolkit, connection_id)
            .await
    }

    async fn source_sync_state(
        &self,
        toolkit: &str,
        connection_id: &str,
    ) -> Result<Option<SourceSyncState>, MemoryError> {
        self.policy.admit_read(
            Capability::SourceSync,
            "source_sync.source_sync_state",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?
            .source_sync_state(toolkit, connection_id)
            .await
    }

    async fn sync_audit_log(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<SyncAuditEntry>, MemoryError> {
        self.policy.admit_read(
            Capability::SourceSync,
            "source_sync.sync_audit_log",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.sync_audit_log(limit).await
    }

    /// Arithmetic over the driver's own price table — no stored content is read,
    /// so this is the lightest check in the family.
    async fn estimate_sync_cost_usd(
        &self,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<f64, MemoryError> {
        self.policy.admit_read(
            Capability::SourceSync,
            "source_sync.estimate_sync_cost_usd",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?
            .estimate_sync_cost_usd(input_tokens, output_tokens)
            .await
    }

    async fn sync_statuses(&self) -> Result<Vec<SourceSyncStatus>, MemoryError> {
        self.policy.admit_read(
            Capability::SourceSync,
            "source_sync.sync_statuses",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.sync_statuses().await
    }

    async fn raw_archive_coverage(
        &self,
        tree_scope: &str,
        archive_source_id: &str,
    ) -> Result<RawArchiveCoverage, MemoryError> {
        self.policy.admit_read(
            Capability::SourceSync,
            "source_sync.raw_archive_coverage",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?
            .raw_archive_coverage(tree_scope, archive_source_id)
            .await
    }

    /// Rebuilds a summary tree from the raw archive, so it writes.
    async fn rebuild_from_raw_archive(
        &self,
        tree_scope: &str,
        archive_source_id: &str,
    ) -> Result<RawRebuildOutcome, MemoryError> {
        self.policy.admit_write(
            Capability::SourceSync,
            "source_sync.rebuild_from_raw_archive",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?
            .rebuild_from_raw_archive(tree_scope, archive_source_id)
            .await
    }
}

// ── Scoring ──────────────────────────────────────────────────────────────────

#[async_trait]
impl MemoryScoring for GuardedScoring {
    async fn extract_entities(&self, query: &str) -> Result<Vec<String>, MemoryError> {
        self.policy.admit_read(
            Capability::Scoring,
            "scoring.extract_entities",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.extract_entities(query).await
    }

    async fn embed_text(&self, text: &str) -> Result<Vec<f32>, MemoryError> {
        self.policy.admit_read(
            Capability::Scoring,
            "scoring.embed_text",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.embed_text(text).await
    }

    async fn embedder_slug(&self) -> Result<String, MemoryError> {
        self.policy.admit_read(
            Capability::Scoring,
            "scoring.embedder_slug",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.embedder_slug().await
    }
}

// ── Coding sessions ──────────────────────────────────────────────────────────

#[async_trait]
impl MemoryCodingSessions for GuardedCodingSessions {
    async fn coding_session_status(&self) -> Result<Vec<CodingSessionSource>, MemoryError> {
        self.policy.admit_read(
            Capability::CodingSessions,
            "coding_sessions.coding_session_status",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.coding_session_status().await
    }

    /// `carries_content: true` — the request carries the session transcripts
    /// themselves, which is the case the egress record exists to classify
    /// correctly.
    async fn ingest_coding_sessions(
        &self,
        request: CodingSessionIngestRequest,
    ) -> Result<CodingSessionIngestReport, MemoryError> {
        self.policy.admit_write(
            Capability::CodingSessions,
            "coding_sessions.ingest_coding_sessions",
            NO_NAMESPACE,
            true,
        )?;
        self.family()?.ingest_coding_sessions(request).await
    }
}
