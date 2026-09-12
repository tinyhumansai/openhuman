// `RecordingProvider`'s own constructor, seeders and call accessors. They
// live here rather than beside the struct in part 01 because that file is at
// the repo's 750-line ceiling for `src/openhuman` (scripts/ci/check-openhuman-
// rust-layout.mjs); the inherent impl is the one self-contained block that
// moves without splitting a trait implementation across files.

impl RecordingProvider {
    /// A provider with an empty call log and every seeded answer at its
    /// default — the shape most tests want before layering a `with_*` on top.
    pub fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            recall_result: Mutex::new(Vec::new()),
            fast_retrieve_result: Mutex::new(RetrievalResponse::default()),
            namespace_hits: Mutex::new(Vec::new()),
            namespace_summaries: Mutex::new(Vec::new()),
            session_turns: Mutex::new(Vec::new()),
            pending_segments: Mutex::new(Vec::new()),
        }
    }

    /// Seed what `recall` answers, so a budget test can drive a known set.
    pub fn with_recall_result(self, entries: Vec<MemoryEntry>) -> Self {
        *self.recall_result.lock().unwrap() = entries;
        self
    }

    /// Seed what `fast_retrieve` answers, so the auto-recall lane can run
    /// through a real guard with known hits.
    pub fn with_fast_retrieve_result(self, response: RetrievalResponse) -> Self {
        *self.fast_retrieve_result.lock().unwrap() = response;
        self
    }

    /// Seed what `recall_namespace_scored` answers, so the vector-floored
    /// paths can be driven with known scores.
    pub fn with_namespace_hits(self, hits: Vec<NamespaceMemoryHit>) -> Self {
        *self.namespace_hits.lock().unwrap() = hits;
        self
    }

    /// Seed what `namespaces` answers, so a namespace can look populated
    /// without a real store behind it.
    pub fn with_namespace_summaries(self, summaries: Vec<NamespaceSummary>) -> Self {
        *self.namespace_summaries.lock().unwrap() = summaries;
        self
    }

    /// Seed what `session_turns` answers. Without this the archivist's
    /// finalize path stops at its empty-entries early return and never
    /// reaches the recap it is being tested for.
    pub fn with_session_turns(self, turns: Vec<EpisodicTurn>) -> Self {
        *self.session_turns.lock().unwrap() = turns;
        self
    }

    /// Seed the re-summarisation queue (#6186). The default is empty, so a
    /// test that does not set this drives the pass over nothing — which is
    /// the state a healthy store is in.
    pub fn with_pending_segments(self, segments: Vec<ConversationSegment>) -> Self {
        *self.pending_segments.lock().unwrap() = segments;
        self
    }

    /// Append one call to the log. Every family impl funnels through this.
    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    /// Every call the driver saw, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    /// How many calls reached the driver — for tests that only care that a
    /// path was or was not taken.
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

// Fixtures for the retrieval family's scored answers. Included into
// `test_support.rs` after the provider parts, so the imports there are in scope.

/// A [`NamespaceSummary`] saying `namespace` holds `count` entries.
pub fn namespace_summary(namespace: &str, count: usize) -> NamespaceSummary {
    NamespaceSummary {
        namespace: namespace.into(),
        count,
        last_updated: None,
    }
}

/// A [`NamespaceMemoryHit`] with only the vector component set — the signal the
/// vector-floored recall paths (Lane B, the contradiction check) filter on.
pub fn namespace_hit(
    namespace: &str,
    key: &str,
    content: &str,
    vector_similarity: f64,
) -> NamespaceMemoryHit {
    NamespaceMemoryHit {
        id: format!("{namespace}/{key}"),
        kind: crate::openhuman::memory::api::types::MemoryItemKind::Kv,
        namespace: namespace.into(),
        key: key.into(),
        title: None,
        content: content.into(),
        category: "core".into(),
        source_type: None,
        updated_at: 0.0,
        score: vector_similarity,
        score_breakdown: crate::openhuman::memory::api::types::RetrievalScoreBreakdown {
            vector_similarity,
            ..Default::default()
        },
        document_id: None,
        chunk_id: None,
        supporting_relations: Vec::new(),
        taint: MemoryTaint::default(),
    }
}

// Moved from part 02 for the same reason the inherent impl above moved: that
// file reached the 750-line ceiling when the episodic family grew
// `segments_pending_summary` (#6186). `MemoryAnswer` is the tail block and is
// self-contained, so it relocates without splitting a trait impl across files.
#[async_trait]
impl MemoryAnswer for RecordingProvider {
    async fn answer(
        &self,
        _request: crate::openhuman::memory::api::provider::operations::AnswerRequest,
    ) -> Result<crate::openhuman::memory::api::provider::operations::AnswerResponse, MemoryError>
    {
        self.record(Call {
            method: "answer.answer".into(),
            content: None,
            taint: None,
            scoped: None,
        });
        Ok(crate::openhuman::memory::api::provider::operations::AnswerResponse {
            answer: String::new(),
            model: None,
            citations: Vec::new(),
            steps: Vec::new(),
        })
    }
}
