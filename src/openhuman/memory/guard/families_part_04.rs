// ── The v1.13.7 typed-ingestion round + Answer ──────────────────────────────
//
// Five families the ingestion round added to the contract, wired the day the
// registry pinned the release: the guard's audit invariant (advertised ==
// reachable-through-the-guard) is what forced this file to exist alongside
// the re-pin rather than after it.

use crate::openhuman::memory::api::provider::operations::{
    AnswerRequest, AnswerResponse, MemoryAnswer, MemoryConversationIngest,
    MemoryDocumentIngest, MemoryEventIngest, MemoryLearningIngest, RawMemoryEvent,
};
use tinymemory_api::learning::LearningCandidate;

decorator!(
    /// Guarded [`MemoryDocumentIngest`].
    GuardedDocumentIngest,
    dyn MemoryDocumentIngest,
    as_document_ingest,
    DocumentIngest
);
decorator!(
    /// Guarded [`MemoryConversationIngest`].
    GuardedConversationIngest,
    dyn MemoryConversationIngest,
    as_conversation_ingest,
    ConversationIngest
);
decorator!(
    /// Guarded [`MemoryLearningIngest`].
    GuardedLearningIngest,
    dyn MemoryLearningIngest,
    as_learning_ingest,
    LearningIngest
);
decorator!(
    /// Guarded [`MemoryEventIngest`].
    GuardedEventIngest,
    dyn MemoryEventIngest,
    as_event_ingest,
    EventIngest
);
decorator!(
    /// Guarded [`MemoryAnswer`].
    GuardedAnswer,
    dyn MemoryAnswer,
    as_answer,
    Answer
);

/// Steps 3 + 4 over one ingest item, shared by the typed-ingest decorators:
/// stamp provenance, redact on egress — the same admission
/// [`GuardedIngest::admit`] applies on the legacy family.
fn admit_typed_item(policy: &GuardPolicy, mut item: IngestItem) -> IngestItem {
    item.taint = policy.stamp_taint(item.taint);
    item.content = policy.redact_outbound(&item.content).into_owned();
    item
}

#[async_trait]
impl MemoryDocumentIngest for GuardedDocumentIngest {
    async fn ingest_document(&self, document: IngestItem) -> Result<IngestOutcome, MemoryError> {
        let namespace = document.namespace.clone().unwrap_or_else(|| "-".to_string());
        self.policy.admit_write(
            Capability::DocumentIngest,
            "document_ingest.ingest_document",
            &namespace,
            true,
        )?;
        let document = admit_typed_item(&self.policy, document);
        trace_allowed(
            &self.policy,
            "document_ingest.ingest_document",
            &namespace,
            document.content.chars().count(),
        );
        self.family()?.ingest_document(document).await
    }
}

#[async_trait]
impl MemoryConversationIngest for GuardedConversationIngest {
    async fn ingest_conversation(
        &self,
        messages: Vec<IngestItem>,
    ) -> Result<IngestOutcome, MemoryError> {
        self.policy.admit_write(
            Capability::ConversationIngest,
            "conversation_ingest.ingest_conversation",
            NO_NAMESPACE,
            true,
        )?;
        let messages: Vec<IngestItem> = messages
            .into_iter()
            .map(|m| admit_typed_item(&self.policy, m))
            .collect();
        trace_allowed(
            &self.policy,
            "conversation_ingest.ingest_conversation",
            NO_NAMESPACE,
            messages.iter().map(|m| m.content.chars().count()).sum(),
        );
        self.family()?.ingest_conversation(messages).await
    }
}

#[async_trait]
impl MemoryLearningIngest for GuardedLearningIngest {
    async fn ingest_learning(
        &self,
        learning: LearningCandidate,
    ) -> Result<IngestOutcome, MemoryError> {
        // No content redaction: a learning candidate is already extracted
        // structure, not raw user text — provenance is the driver's to stamp
        // from the evidence pointer it carries.
        self.policy.admit_write(
            Capability::LearningIngest,
            "learning_ingest.ingest_learning",
            NO_NAMESPACE,
            true,
        )?;
        trace_allowed(
            &self.policy,
            "learning_ingest.ingest_learning",
            NO_NAMESPACE,
            0,
        );
        self.family()?.ingest_learning(learning).await
    }
}

#[async_trait]
impl MemoryEventIngest for GuardedEventIngest {
    async fn ingest_event(&self, event: RawMemoryEvent) -> Result<IngestOutcome, MemoryError> {
        self.policy.admit_write(
            Capability::EventIngest,
            "event_ingest.ingest_event",
            NO_NAMESPACE,
            true,
        )?;
        trace_allowed(&self.policy, "event_ingest.ingest_event", NO_NAMESPACE, 0);
        self.family()?.ingest_event(event).await
    }
}

#[async_trait]
impl MemoryAnswer for GuardedAnswer {
    async fn answer(&self, request: AnswerRequest) -> Result<AnswerResponse, MemoryError> {
        // A read-shaped family: retrieval plus synthesis, no persistence.
        self.policy
            .admit_read(Capability::Answer, "answer.answer", NO_NAMESPACE, false)?;
        trace_allowed(&self.policy, "answer.answer", NO_NAMESPACE, 0);
        self.family()?.answer(request).await
    }
}

#[async_trait]
impl MemoryChunks for GuardedChunks {
    async fn list_chunks(
        &self,
        query: &ChunkQuery,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<Chunk>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.list_chunks",
            NO_NAMESPACE,
            false,
        )?;
        // Intersected with the ambient allowlist, never passed through. The
        // ambient scope is an upper bound: forwarding the caller's scope
        // unchanged would let a source-restricted turn widen itself back out by
        // naming a collection the restriction excluded. See
        // `GuardPolicy::narrow_scope`.
        let effective = self.policy.narrow_scope(scope);
        self.family()?.list_chunks(query, effective.as_ref()).await
    }

    /// The count that labels a [`Self::list_chunks`] page, and it must be
    /// narrowed by exactly the same rule. A total computed against a wider
    /// scope than the page it labels leaks the existence of rows the caller may
    /// not read — "showing 20 of 4000" tells a source-restricted turn how much
    /// it is not being shown.
    async fn count_chunks(
        &self,
        query: &ChunkQuery,
        scope: Option<&SourceScope>,
    ) -> Result<u64, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.count_chunks",
            NO_NAMESPACE,
            false,
        )?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?.count_chunks(query, effective.as_ref()).await
    }

    /// Same rows as [`Self::list_chunks`] with the stored facts beside them, so
    /// the same intersection applies for the same reason.
    async fn list_chunk_details(
        &self,
        query: &ChunkQuery,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<ChunkListRow>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.list_chunk_details",
            NO_NAMESPACE,
            false,
        )?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?
            .list_chunk_details(query, effective.as_ref())
            .await
    }

    /// Per-source totals are computed from the chunks the scope admits, not
    /// filtered afterwards — so a restricted caller must not learn that a
    /// forbidden source exists by seeing its row, nor see a permitted source
    /// carrying a count that includes rows it cannot read.
    async fn source_totals(
        &self,
        limit: usize,
        scope: Option<&SourceScope>,
    ) -> Result<Vec<SourceTotal>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.source_totals",
            NO_NAMESPACE,
            false,
        )?;
        let effective = self.policy.narrow_scope(scope);
        self.family()?
            .source_totals(limit, effective.as_ref())
            .await
    }

    async fn get_chunk(&self, chunk_id: &str) -> Result<Option<Chunk>, MemoryError> {
        self.policy
            .admit_read(Capability::Chunks, "chunks.get_chunk", NO_NAMESPACE, false)?;
        self.family()?.get_chunk(chunk_id).await
    }

    async fn chunk_detail(&self, chunk_id: &str) -> Result<Option<ChunkDetail>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.chunk_detail",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.chunk_detail(chunk_id).await
    }

    /// The catalog is not user content, so it takes no namespace and the
    /// lightest read check — refusing it under `readonly` would stop an
    /// operator finding out what the store can even hold.
    async fn storage_kinds(&self) -> Result<Vec<String>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.storage_kinds",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.storage_kinds().await
    }

    /// Vectors, not content — but still a read of stored material, so it takes
    /// the same tier check rather than being waved through as metadata.
    async fn chunk_embeddings(
        &self,
        chunk_ids: &[String],
        model_signature: &str,
    ) -> Result<Vec<ChunkEmbedding>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.chunk_embeddings",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?
            .chunk_embeddings(chunk_ids, model_signature)
            .await
    }

    /// One chunk's admission verdict, read by chunk id exactly as
    /// [`Self::chunk_detail`] is — so it takes that member's check, not
    /// [`Self::list_chunks`]'s scope intersection. There is no scope to narrow:
    /// the caller already holds the id.
    async fn chunk_score(&self, chunk_id: &str) -> Result<Option<ChunkScore>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.chunk_score",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.chunk_score(chunk_id).await
    }

    /// Ingest progress for the sources the caller names, one row per query.
    ///
    /// Unlike [`Self::source_totals`], which enumerates the groups that exist
    /// and therefore has to be narrowed to the ambient allowlist, this answers
    /// only about prefixes the caller supplied — it discloses no source the
    /// caller did not already name — and the contract member carries no
    /// [`SourceScope`] for the guard to intersect anything into.
    async fn source_ingest_status(
        &self,
        source_prefixes: &[SourceIngestQuery],
    ) -> Result<Vec<SourceIngestStatus>, MemoryError> {
        self.policy.admit_read(
            Capability::Chunks,
            "chunks.source_ingest_status",
            NO_NAMESPACE,
            false,
        )?;
        self.family()?.source_ingest_status(source_prefixes).await
    }
}
