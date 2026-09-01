#[async_trait]
impl MemoryCodingSessions for ModuleMemoryProvider {
    async fn coding_session_status(&self) -> Result<Vec<CodingSessionSource>, MemoryError> {
        module_call!(
            self,
            "coding_session_status",
            methods::CODING_SESSION_STATUS,
            ()
        )
    }
    /// # Why this one call sets its own bus deadline
    ///
    /// Every other member here takes tinybus' `DEFAULT_TIMEOUT`
    /// (`vendor/tinybus/crates/tinybus/src/connection.rs:56`) — a flat 30 s,
    /// applied by `Proxy::new` (`proxy.rs:59`) whenever nobody says otherwise.
    /// That is the right default for a memory read. It is the wrong one here:
    /// distilling a coding session is several *sequential* model calls, and the
    /// RPC above it already computes a budget sized to the work
    /// (`memory::sources::rpc::ingest_budget`, 120 s + 90 s per session, capped
    /// at 600 s).
    ///
    /// So there were two deadlines and the tighter one was the one nobody
    /// chose. A real 35 s import tripped the 30 s default; the caller was
    /// released with an error while the module kept working and finished
    /// seconds later, having imported everything. The UI reported a failure for
    /// work that had succeeded, and invited a retry that would redo it
    /// (#5802).
    ///
    /// tinybus is explicit that this is the caller's problem to size: *"A
    /// timeout does not cancel the remote work — tinybus cannot — it stops
    /// waiting and frees the caller"* (`connection.rs:22-23`). Abandoning the
    /// call early therefore does not save anything; it only loses the report.
    ///
    /// The budget is taken from `ingest_budget` rather than restated, so the
    /// two layers cannot drift, plus [`INGEST_BUS_GRACE`]. The grace makes the
    /// ordering deterministic instead of a race between two equal deadlines:
    /// the RPC's own `tokio::time::timeout` fires first and reports its clean
    /// structured message, and this deadline survives only as the
    /// wedged-forever backstop tinybus requires. Same shape as the client's
    /// `CODING_SESSION_RPC_GRACE_MS` sitting above the server budget.
    async fn ingest_coding_sessions(
        &self,
        request: CodingSessionIngestRequest,
    ) -> Result<CodingSessionIngestReport, MemoryError> {
        let deadline = crate::openhuman::memory::sources::rpc::ingest_budget(request.max_sessions)
            + INGEST_BUS_GRACE;
        self.proxy("ingest_coding_sessions")
            .await?
            .with_timeout(deadline)
            .call(methods::INGEST_CODING_SESSIONS, (request,))
            .await
            .map_err(|error| from_bus(&error))
    }
}

/// Head-room added to [`ingest_budget`](crate::openhuman::memory::sources::rpc::ingest_budget)
/// for the bus deadline on `IngestCodingSessions`.
///
/// Exists to order two deadlines, not to allow more work: the RPC's own
/// wall-clock ceiling must be the one that fires, because its message names the
/// budget rather than the wire member. Anything comfortably longer than the
/// scheduling jitter between the two `tokio::time::timeout` arms would do.
const INGEST_BUS_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

#[async_trait]
impl MemoryEpisodic for ModuleMemoryProvider {
    async fn insert_turn(&self, turn: &EpisodicTurn) -> Result<i64, MemoryError> {
        module_call!(self, "insert_turn", methods::INSERT_TURN, (turn,))
    }
    async fn session_turns(&self, session_id: &str) -> Result<Vec<EpisodicTurn>, MemoryError> {
        module_call!(self, "session_turns", methods::SESSION_TURNS, (session_id,))
    }
    async fn open_segment(
        &self,
        session_id: &str,
    ) -> Result<Option<ConversationSegment>, MemoryError> {
        module_call!(self, "open_segment", methods::OPEN_SEGMENT, (session_id,))
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
        module_call!(
            self,
            "create_segment",
            methods::CREATE_SEGMENT,
            (
                segment_id,
                session_id,
                namespace,
                start_episodic_id,
                start_seq,
                start_timestamp,
                now
            )
        )
    }
    async fn append_turn(
        &self,
        segment_id: &str,
        episodic_id: i64,
        seq: Option<u32>,
        timestamp: f64,
        now: f64,
    ) -> Result<(), MemoryError> {
        module_call!(
            self,
            "append_turn",
            methods::APPEND_TURN,
            (segment_id, episodic_id, seq, timestamp, now)
        )
    }
    async fn insert_event(&self, event: &EpisodicEvent) -> Result<(), MemoryError> {
        module_call!(self, "insert_event", methods::INSERT_EVENT, (event,))
    }
    async fn close_segment(&self, segment_id: &str, now: f64) -> Result<(), MemoryError> {
        module_call!(
            self,
            "close_segment",
            methods::CLOSE_SEGMENT,
            (segment_id, now)
        )
    }
    async fn set_segment_summary(
        &self,
        segment_id: &str,
        summary: &str,
        now: f64,
    ) -> Result<(), MemoryError> {
        module_call!(
            self,
            "set_segment_summary",
            methods::SET_SEGMENT_SUMMARY,
            (segment_id, summary, now)
        )
    }
    async fn upsert_segment_embedding(
        &self,
        segment_id: &str,
        model_signature: &str,
        embedding: &[f32],
        created_at: f64,
    ) -> Result<(), MemoryError> {
        module_call!(
            self,
            "upsert_segment_embedding",
            methods::UPSERT_SEGMENT_EMBEDDING,
            (segment_id, model_signature, embedding, created_at)
        )
    }
}

#[async_trait]
impl MemoryPeople for ModuleMemoryProvider {
    async fn list_people(&self, limit: Option<usize>) -> Result<Vec<RankedPerson>, MemoryError> {
        module_call!(self, "list_people", methods::LIST_PEOPLE, (limit,))
    }
    async fn get_person(&self, person_id: &str) -> Result<Option<PersonRecord>, MemoryError> {
        module_call!(self, "get_person", methods::GET_PERSON, (person_id,))
    }
    async fn resolve_handle(
        &self,
        handle: &PersonHandle,
        create_if_missing: bool,
    ) -> Result<Option<ResolvedPerson>, MemoryError> {
        module_call!(
            self,
            "resolve_handle",
            methods::RESOLVE_HANDLE,
            (handle, create_if_missing)
        )
    }
    async fn add_handle_alias(
        &self,
        person_id: &str,
        handle: &PersonHandle,
    ) -> Result<(), MemoryError> {
        module_call!(
            self,
            "add_handle_alias",
            methods::ADD_HANDLE_ALIAS,
            (person_id, handle)
        )
    }
    async fn score_person(&self, person_id: &str) -> Result<Option<PersonScore>, MemoryError> {
        module_call!(self, "score_person", methods::SCORE_PERSON, (person_id,))
    }
    async fn record_interaction(&self, interaction: &PersonInteraction) -> Result<(), MemoryError> {
        module_call!(
            self,
            "record_interaction",
            methods::RECORD_INTERACTION,
            (interaction,)
        )
    }
    async fn seed_from_address_book(&self) -> Result<AddressBookSeedOutcome, MemoryError> {
        module_call!(
            self,
            "seed_from_address_book",
            methods::SEED_FROM_ADDRESS_BOOK,
            ()
        )
    }
}

#[async_trait]
impl MemoryChunks for ModuleMemoryProvider {
    async fn list_chunks(
        &self,
        query: &ChunkQuery,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<Chunk>, MemoryError> {
        module_call!(self, "list_chunks", methods::LIST_CHUNKS, (query, scope))
    }
    async fn get_chunk(&self, chunk_id: &str) -> Result<Option<Chunk>, MemoryError> {
        module_call!(self, "get_chunk", methods::GET_CHUNK, (chunk_id,))
    }
    async fn chunk_detail(&self, chunk_id: &str) -> Result<Option<ChunkDetail>, MemoryError> {
        module_call!(self, "chunk_detail", methods::CHUNK_DETAIL, (chunk_id,))
    }
    async fn storage_kinds(&self) -> Result<Vec<String>, MemoryError> {
        module_call!(self, "storage_kinds", methods::STORAGE_KINDS, ())
    }
    async fn chunk_embeddings(
        &self,
        chunk_ids: &[String],
        model_signature: &str,
    ) -> Result<Vec<ChunkEmbedding>, MemoryError> {
        module_call!(
            self,
            "chunk_embeddings",
            methods::CHUNK_EMBEDDINGS,
            (chunk_ids, model_signature)
        )
    }
    async fn count_chunks(
        &self,
        query: &ChunkQuery,
        scope: Option<&SourceScope>,
    ) -> Result<u64, MemoryError> {
        module_call!(self, "count_chunks", methods::COUNT_CHUNKS, (query, scope))
    }
    async fn list_chunk_details(
        &self,
        query: &ChunkQuery,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<ChunkListRow>, MemoryError> {
        module_call!(
            self,
            "list_chunk_details",
            methods::LIST_CHUNK_DETAILS,
            (query, scope)
        )
    }
    async fn source_totals(
        &self,
        limit: usize,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<SourceTotal>, MemoryError> {
        module_call!(
            self,
            "source_totals",
            methods::SOURCE_TOTALS,
            (limit, scope)
        )
    }
    /// `Ok(None)` for a chunk the module never scored — a different fact from a
    /// chunk that scored zero, which is why the response is an `Option` rather
    /// than a zeroed row.
    async fn chunk_score(&self, chunk_id: &str) -> Result<Option<ChunkScore>, MemoryError> {
        module_call!(self, "chunk_score", methods::CHUNK_SCORE, (chunk_id,))
    }
    /// One row per query, in the order asked. The prefixes are derived from the
    /// **host's** source registry — the module has no access to it — so they
    /// cross as values rather than being re-derived on the far side.
    async fn source_ingest_status(
        &self,
        source_prefixes: &[SourceIngestQuery],
    ) -> Result<Vec<SourceIngestStatus>, MemoryError> {
        module_call!(
            self,
            "source_ingest_status",
            methods::SOURCE_INGEST_STATUS,
            (source_prefixes,)
        )
    }
}

#[async_trait]
impl MemoryRetrieval for ModuleMemoryProvider {
    async fn fast_retrieve(
        &self,
        query: &str,
        options: FastRetrieveQuery,
        scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        module_call!(
            self,
            "fast_retrieve",
            methods::FAST_RETRIEVE,
            (query, options, scope)
        )
    }
    async fn cover_window(
        &self,
        window: &CoverWindowQuery,
        scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        module_call!(self, "cover_window", methods::COVER_WINDOW, (window, scope))
    }
    async fn retrieve_source(
        &self,
        query: &SourceRetrievalQuery,
        scope: Option<&SourceScope>,
    ) -> Result<RetrievalResponse, MemoryError> {
        module_call!(
            self,
            "retrieve_source",
            methods::RETRIEVE_SOURCE,
            (query, scope)
        )
    }
    async fn retrieve_children(
        &self,
        node_id: &str,
        max_depth: u32,
        query: Option<&str>,
        limit: Option<usize>,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<RetrievalHit>, MemoryError> {
        module_call!(
            self,
            "retrieve_children",
            methods::RETRIEVE_CHILDREN,
            (node_id, max_depth, query, limit, scope)
        )
    }
    async fn retrieve_leaves(
        &self,
        chunk_ids: &[String],
        scope: Option<&SourceScope>,
    ) -> Result<Vec<RetrievalHit>, MemoryError> {
        module_call!(
            self,
            "retrieve_leaves",
            methods::RETRIEVE_LEAVES,
            (chunk_ids, scope)
        )
    }
    async fn recall_namespace_recent(
        &self,
        namespace: &str,
        limit: usize,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        module_call!(
            self,
            "recall_namespace_recent",
            methods::RECALL_NAMESPACE_RECENT,
            (namespace, limit)
        )
    }
    async fn recall_namespace_scored(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
        exclude_session_id: Option<&str>,
    ) -> Result<Vec<NamespaceMemoryHit>, MemoryError> {
        module_call!(
            self,
            "recall_namespace_scored",
            methods::RECALL_NAMESPACE_SCORED,
            (namespace, query, limit, exclude_session_id)
        )
    }
    async fn search_entities(
        &self,
        query: &str,
        kinds: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<EntityMatch>, MemoryError> {
        module_call!(
            self,
            "search_entities",
            methods::SEARCH_ENTITIES,
            (query, kinds, limit)
        )
    }
}

#[async_trait]
impl MemoryProfile for ModuleMemoryProvider {
    async fn list_active_facets(&self) -> Result<Vec<ProfileFacet>, MemoryError> {
        module_call!(self, "list_active_facets", methods::LIST_ACTIVE_FACETS, ())
    }
    async fn list_all_facets(&self) -> Result<Vec<ProfileFacet>, MemoryError> {
        module_call!(self, "list_all_facets", methods::LIST_ALL_FACETS, ())
    }
    async fn get_facet(&self, key: &str) -> Result<Option<ProfileFacet>, MemoryError> {
        module_call!(self, "get_facet", methods::GET_FACET, (key,))
    }
    async fn facets_by_type(
        &self,
        facet_type: FacetType,
    ) -> Result<Vec<ProfileFacet>, MemoryError> {
        module_call!(
            self,
            "facets_by_type",
            methods::FACETS_BY_TYPE,
            (facet_type,)
        )
    }
    async fn upsert_facet(&self, facet: &ProfileFacet) -> Result<(), MemoryError> {
        module_call!(self, "upsert_facet", methods::UPSERT_FACET, (facet,))
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
        module_call!(
            self,
            "upsert_provider_facet",
            methods::UPSERT_PROVIDER_FACET,
            (
                facet_id,
                facet_type,
                key,
                value,
                confidence,
                segment_id,
                observed_at
            )
        )
    }
    async fn set_facet_user_state(
        &self,
        key: &str,
        user_state: UserState,
    ) -> Result<bool, MemoryError> {
        module_call!(
            self,
            "set_facet_user_state",
            methods::SET_FACET_USER_STATE,
            (key, user_state)
        )
    }
    async fn delete_facet(&self, key: &str) -> Result<bool, MemoryError> {
        module_call!(self, "delete_facet", methods::DELETE_FACET, (key,))
    }
    async fn delete_facet_by_id(&self, facet_id: &str) -> Result<bool, MemoryError> {
        module_call!(
            self,
            "delete_facet_by_id",
            methods::DELETE_FACET_BY_ID,
            (facet_id,)
        )
    }
    async fn drop_facets_below(&self, threshold: f64) -> Result<usize, MemoryError> {
        module_call!(
            self,
            "drop_facets_below",
            methods::DROP_FACETS_BELOW,
            (threshold,)
        )
    }
    /// Any transport failure reads as `false` — the trait's documented rule for
    /// this predicate, and the reason it returns `bool` rather than a `Result`.
    async fn workflow_identity_matches(&self, key_pattern: &str, canonical_value: &str) -> bool {
        // Written out rather than via `module_call!`: that macro uses `?`, which
        // needs a `Result`-returning body, and this one returns `bool` on
        // purpose. Both failure points — resolving the proxy and the call
        // itself — collapse to `false`, which is the rule above.
        let Ok(proxy) = self.proxy("workflow_identity_matches").await else {
            return false;
        };
        proxy
            .call::<bool>("WorkflowIdentityMatches", (key_pattern, canonical_value))
            .await
            .unwrap_or(false)
    }
}

#[async_trait]
impl MemoryScoring for ModuleMemoryProvider {
    async fn extract_entities(&self, query: &str) -> Result<Vec<String>, MemoryError> {
        module_call!(
            self,
            "extract_entities",
            methods::EXTRACT_ENTITIES,
            (query,)
        )
    }
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>, MemoryError> {
        module_call!(self, "embed_text", methods::EMBED_TEXT, (text,))
    }
    async fn embedder_slug(&self) -> Result<String, MemoryError> {
        module_call!(self, "embedder_slug", methods::EMBEDDER_SLUG, ())
    }
}
