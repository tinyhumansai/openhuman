use super::*;
use crate::openhuman::memory::NamespaceSummary;
use std::sync::Mutex;

/// Minimal in-process [`Memory`] double.
///
/// Recall is an exact-substring scan honouring `namespace` / `session_id` /
/// `min_score`, which is enough to prove this adapter passes the right
/// `RecallOpts` down and enough to keep the tests free of SQLite, vectors,
/// and embeddings.
#[derive(Default)]
struct StubMemory {
    rows: Mutex<Vec<MemoryEntry>>,
    /// When set, every fallible method returns this error.
    fail: Option<String>,
    /// The `exclude_session_id` the adapter last asked for.
    ///
    /// Recorded rather than acted on. The real backend applies that field
    /// to document-kind hits only, while `session_id` / `cross_session`
    /// scope the episodic and event tiers — a flat row list cannot tell
    /// those apart, and a stub that filtered every row by it asserts a
    /// backend behaviour that does not exist. That is not hypothetical:
    /// doing so broke the two thread-hint tests below, which pin that a
    /// hint *narrows to* a session rather than away from it. What is the
    /// host's to get right is which exclusion it asks for.
    last_exclusion: Mutex<Option<Option<String>>>,
}

impl StubMemory {
    fn with_rows(rows: Vec<MemoryEntry>) -> Self {
        Self {
            rows: Mutex::new(rows),
            fail: None,
            last_exclusion: Mutex::new(None),
        }
    }

    fn failing() -> Self {
        Self {
            rows: Mutex::new(Vec::new()),
            fail: Some("backend down".to_string()),
            last_exclusion: Mutex::new(None),
        }
    }

    fn snapshot(&self) -> Vec<MemoryEntry> {
        self.rows.lock().unwrap().clone()
    }

    /// The `exclude_session_id` of the last recall, if one has run.
    fn last_exclusion(&self) -> Option<Option<String>> {
        self.last_exclusion.lock().unwrap().clone()
    }
}

fn entry(id: &str, key: &str, content: &str) -> MemoryEntry {
    MemoryEntry {
        id: id.to_string(),
        key: key.to_string(),
        content: content.to_string(),
        namespace: Some(DEFAULT_AGENT_MEMORY_NAMESPACE.to_string()),
        category: MemoryCategory::Conversation,
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        session_id: None,
        score: Some(0.9),
        taint: MemoryTaint::Internal,
    }
}

#[async_trait]
impl Memory for StubMemory {
    fn name(&self) -> &str {
        "stub"
    }

    async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.store_with_taint(
            namespace,
            key,
            content,
            category,
            session_id,
            MemoryTaint::Internal,
        )
        .await
    }

    async fn store_with_taint(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
        taint: MemoryTaint,
    ) -> anyhow::Result<()> {
        if let Some(err) = &self.fail {
            anyhow::bail!("{err}");
        }
        let mut rows = self.rows.lock().unwrap();
        rows.retain(|r| !(r.namespace.as_deref() == Some(namespace) && r.key == key));
        // Read the length before the `push` borrows `rows` mutably.
        let next_id = format!("row-{}", rows.len() + 1);
        rows.push(MemoryEntry {
            id: next_id,
            key: key.to_string(),
            content: content.to_string(),
            namespace: Some(namespace.to_string()),
            category,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            session_id: session_id.map(str::to_string),
            score: None,
            taint,
        });
        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        *self.last_exclusion.lock().unwrap() = Some(opts.exclude_session_id.map(str::to_string));
        if let Some(err) = &self.fail {
            anyhow::bail!("{err}");
        }
        let needle = query.to_lowercase();
        let rows = self.rows.lock().unwrap();
        let mut out: Vec<MemoryEntry> = rows
            .iter()
            .filter(|r| match opts.namespace {
                Some(ns) => r.namespace.as_deref() == Some(ns),
                None => true,
            })
            // Models the real backend: `cross_session` widens past the
            // session filter (`memory/store/memory_trait.rs` runs the
            // episodic cross-session search under exactly this flag). A
            // stub that ignored it would silently pass a host-side filter
            // that discards every widened row.
            .filter(|r| match (opts.session_id, r.session_id.as_deref()) {
                _ if opts.cross_session => true,
                (Some(want), Some(have)) => want == have,
                (Some(_), None) => true,
                (None, _) => true,
            })
            .filter(|r| match (opts.min_score, r.score) {
                (Some(floor), Some(score)) => score >= floor,
                _ => true,
            })
            .filter(|r| needle.is_empty() || r.content.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        out.truncate(limit);
        Ok(out)
    }

    async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        if let Some(err) = &self.fail {
            anyhow::bail!("{err}");
        }
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.namespace.as_deref() == Some(namespace) && r.key == key)
            .cloned())
    }

    async fn list(
        &self,
        _namespace: Option<&str>,
        _category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        Ok(self.snapshot())
    }

    async fn forget(&self, namespace: &str, key: &str) -> anyhow::Result<bool> {
        let mut rows = self.rows.lock().unwrap();
        let before = rows.len();
        rows.retain(|r| !(r.namespace.as_deref() == Some(namespace) && r.key == key));
        Ok(rows.len() != before)
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        Ok(Vec::new())
    }

    async fn count(&self) -> anyhow::Result<usize> {
        Ok(self.rows.lock().unwrap().len())
    }

    async fn health_check(&self) -> bool {
        self.fail.is_none()
    }
}

fn adapter(stub: StubMemory) -> (OpenHumanAgentMemory, Arc<StubMemory>) {
    let stub = Arc::new(stub);
    let memory: Arc<dyn Memory> = stub.clone();
    (OpenHumanAgentMemory::new(memory), stub)
}

// ── recall ────────────────────────────────────────────────────────────

#[tokio::test]
async fn empty_store_recalls_nothing_without_erroring() {
    let (mem, _) = adapter(StubMemory::default());
    let items = mem.recall(RecallRequest::new("anything")).await.unwrap();
    assert!(items.is_empty(), "absence is not a failure");
}

#[tokio::test]
async fn a_backend_failure_is_an_error_not_an_empty_result() {
    // The trait leans on this distinction: `None` at wiring time means "no
    // memory in this deployment", `Err` means "the store could not answer".
    // Collapsing a failure into `Ok(vec![])` would erase that.
    let (mem, _) = adapter(StubMemory::failing());
    let err = mem.recall(RecallRequest::new("x")).await.unwrap_err();
    assert!(matches!(err, TinyAgentsError::Capability(_)), "{err:?}");
}

#[tokio::test]
async fn recall_preserves_the_backend_order() {
    let (mem, _) = adapter(StubMemory::with_rows(vec![
        entry("r1", "k1", "note one"),
        entry("r2", "k2", "note two"),
        entry("r3", "k3", "note three"),
    ]));

    let items = mem.recall(RecallRequest::new("note")).await.unwrap();
    let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
    assert_eq!(texts, vec!["note one", "note two", "note three"]);
}

#[tokio::test]
async fn the_runtime_limit_is_honoured_but_capped_by_the_host() {
    let rows: Vec<MemoryEntry> = (0..10)
        .map(|i| entry(&format!("r{i}"), &format!("k{i}"), "note"))
        .collect();
    let (mem, _) = adapter(StubMemory::with_rows(rows));

    let capped = mem
        .recall(RecallRequest::new("note").with_limit(2))
        .await
        .unwrap();
    assert_eq!(capped.len(), 2);

    // A runtime asking for more than the ceiling gets the ceiling, not the
    // ask — `limit` is an upper bound the host may lower.
    let mem = mem.with_limits(5, 3);
    let ceiling = mem
        .recall(RecallRequest::new("note").with_limit(1_000))
        .await
        .unwrap();
    assert_eq!(ceiling.len(), 3);
}

#[tokio::test]
async fn no_limit_falls_back_to_the_host_default() {
    let rows: Vec<MemoryEntry> = (0..10)
        .map(|i| entry(&format!("r{i}"), &format!("k{i}"), "note"))
        .collect();
    let (mem, _) = adapter(StubMemory::with_rows(rows));

    let items = mem.recall(RecallRequest::new("note")).await.unwrap();
    assert_eq!(items.len(), DEFAULT_RECALL_LIMIT);
}

#[tokio::test]
async fn recall_asks_the_backend_to_exclude_the_turns_own_thread() {
    // The harness saves the user's message as a `[conversation]` document
    // tagged with the active thread *before* the agent runs, so a recall
    // issued during that turn can retrieve its own trigger as the best
    // "relevant" hit unless the backend is told to drop that thread's
    // documents.
    //
    // Asserted on the request rather than the result: the drop is the
    // engine's, and it applies to document-kind hits only, which a flat
    // stub cannot model without contradicting the thread-hint tests below.
    // Asking for the right exclusion is the part that lives here.
    let stub = Arc::new(StubMemory::with_rows(vec![entry("r1", "k1", "note")]));
    let memory: Arc<dyn Memory> = stub.clone();
    let mem = OpenHumanAgentMemory::new(memory);

    mem.recall(RecallRequest::new("note").with_thread(ThreadId::new("t1")))
        .await
        .unwrap();
    assert_eq!(
        stub.last_exclusion(),
        Some(Some("t1".to_string())),
        "the active thread must be excluded, or recall returns the turn's own request"
    );

    // No thread, nothing to echo: excluding a session the caller never
    // named would drop rows for no reason.
    mem.recall(RecallRequest::new("note")).await.unwrap();
    assert_eq!(stub.last_exclusion(), Some(None));
}

#[tokio::test]
async fn recall_is_pinned_to_the_adapters_namespace() {
    let mut foreign = entry("r-other", "k", "note in another namespace");
    foreign.namespace = Some("someone-elses".to_string());
    let (mem, _) = adapter(StubMemory::with_rows(vec![
        entry("r-mine", "k", "note in my namespace"),
        foreign,
    ]));

    let items = mem.recall(RecallRequest::new("note")).await.unwrap();
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["r-mine"],
        "a runtime cannot reach other namespaces"
    );
}

#[tokio::test]
async fn a_thread_hint_narrows_scope_and_keeps_unscoped_rows() {
    let mut mine = entry("r1", "k1", "scoped one");
    mine.session_id = Some("t1".to_string());
    let mut theirs = entry("r2", "k2", "scoped two");
    theirs.session_id = Some("t2".to_string());
    let unscoped = entry("r3", "k3", "scoped none");

    let (mem, _) = adapter(StubMemory::with_rows(vec![mine, theirs, unscoped]));
    let items = mem
        .recall(RecallRequest::new("scoped").with_thread(ThreadId::new("t1")))
        .await
        .unwrap();

    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, vec!["r1", "r3"]);
}

#[tokio::test]
async fn cross_session_with_a_thread_hint_recalls_other_sessions() {
    // The regression this pins: the flag reached the backend, the backend
    // widened, and the host-side pass then dropped every widened row — so
    // `with_cross_session(true)` behaved exactly like `false`. A unit test
    // on `scope_allows` alone would not have caught it; the bug lived in
    // the two halves disagreeing.
    let mut mine = entry("r1", "k1", "scoped one");
    mine.session_id = Some("t1".to_string());
    let mut theirs = entry("r2", "k2", "scoped two");
    theirs.session_id = Some("t2".to_string());

    let stub = Arc::new(StubMemory::with_rows(vec![mine, theirs]));
    let memory: Arc<dyn Memory> = stub.clone();
    let mem = OpenHumanAgentMemory::new(memory).with_cross_session(true);

    let items = mem
        .recall(RecallRequest::new("scoped").with_thread(ThreadId::new("t1")))
        .await
        .unwrap();

    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["r1", "r2"],
        "cross-session recall must reach the other session's rows"
    );
}

#[test]
fn the_defensive_scope_pass_drops_rows_from_another_session() {
    // The backend filter should already have done this; the second pass is
    // what makes the adapter safe if it ever does not, because the runtime
    // is forbidden from filtering again.
    assert!(OpenHumanAgentMemory::scope_allows(
        false,
        Some("t1"),
        Some("t1")
    ));
    assert!(!OpenHumanAgentMemory::scope_allows(
        false,
        Some("t1"),
        Some("t2")
    ));
    assert!(OpenHumanAgentMemory::scope_allows(false, Some("t1"), None));
    assert!(OpenHumanAgentMemory::scope_allows(false, None, Some("t2")));
}

#[test]
fn cross_session_recall_keeps_rows_the_widening_returned() {
    // `cross_session` is sent to the backend as "return other sessions'
    // rows". Re-applying the same-session test here would delete exactly
    // those rows and make the opt-in indistinguishable from `false`.
    assert!(OpenHumanAgentMemory::scope_allows(
        true,
        Some("t1"),
        Some("t2")
    ));
    assert!(OpenHumanAgentMemory::scope_allows(true, Some("t1"), None));
}

#[tokio::test]
async fn recalled_text_is_redacted_before_it_reaches_the_runtime() {
    let secret = "-----BEGIN PRIVATE KEY-----\nAAAABBBB\n-----END PRIVATE KEY-----";
    let (mem, _) = adapter(StubMemory::with_rows(vec![entry(
        "r1",
        "k1",
        &format!("deploy note {secret}"),
    )]));

    let items = mem.recall(RecallRequest::new("deploy")).await.unwrap();
    assert_eq!(items.len(), 1);
    assert!(
        !items[0].text.contains("AAAABBBB"),
        "raw store rows must never reach the runtime: {}",
        items[0].text
    );
    assert!(items[0].text.contains("deploy note"));
}

#[tokio::test]
async fn recall_carries_the_backend_score_and_a_flat_citation() {
    let (mem, _) = adapter(StubMemory::with_rows(vec![entry("r1", "k1", "a fact")]));
    let items = mem.recall(RecallRequest::new("fact")).await.unwrap();

    assert_eq!(items[0].score, Some(0.9));
    let citation = items[0].citation.as_deref().expect("citation rendered");
    assert!(citation.starts_with("openhuman:memory/global/k1#r1"));
    // The host's structured shape must not leak through the opaque string.
    assert!(!citation.contains('{'), "{citation}");
    assert!(!citation.contains("snippet"), "{citation}");
}

#[test]
fn render_citation_flattens_without_serializing_the_struct() {
    let citation = MemoryCitation {
        id: "r1".into(),
        key: "favorite_language".into(),
        namespace: Some("global".into()),
        score: Some(0.5),
        timestamp: "2026-01-01T00:00:00Z".into(),
        snippet: "Rust".into(),
    };
    assert_eq!(
        render_citation(&citation),
        "openhuman:memory/global/favorite_language#r1@2026-01-01T00:00:00Z"
    );

    // A namespace-less entry still renders, and an empty timestamp is
    // omitted rather than rendered as a dangling separator.
    let bare = MemoryCitation {
        namespace: None,
        timestamp: String::new(),
        ..citation
    };
    assert_eq!(
        render_citation(&bare),
        "openhuman:memory/global/favorite_language#r1"
    );
}

// ── remember ──────────────────────────────────────────────────────────

#[tokio::test]
async fn remember_stamps_provenance_host_side() {
    let (mem, stub) = adapter(StubMemory::default());
    mem.remember(NewMemory::new("the sky is blue"))
        .await
        .unwrap();

    let rows = stub.snapshot();
    assert_eq!(rows.len(), 1);
    // Fail-closed: the adapter cannot know whether the turn touched
    // untrusted content, so it stamps the restrictive value.
    assert_eq!(rows[0].taint, MemoryTaint::ExternalSync);
    assert_eq!(rows[0].category, MemoryCategory::Conversation);
    assert_eq!(
        rows[0].namespace.as_deref(),
        Some(DEFAULT_AGENT_MEMORY_NAMESPACE)
    );
}

#[tokio::test]
async fn a_wiring_site_can_relax_the_write_taint() {
    let stub = Arc::new(StubMemory::default());
    let memory: Arc<dyn Memory> = stub.clone();
    let mem = OpenHumanAgentMemory::new(memory).with_taint(MemoryTaint::Internal);
    mem.remember(NewMemory::new("a fact")).await.unwrap();
    assert_eq!(stub.snapshot()[0].taint, MemoryTaint::Internal);
}

#[tokio::test]
async fn remember_scopes_to_the_thread_when_one_is_given() {
    let (mem, stub) = adapter(StubMemory::default());
    mem.remember(NewMemory::new("a fact").with_thread(ThreadId::new("t1")))
        .await
        .unwrap();
    assert_eq!(stub.snapshot()[0].session_id.as_deref(), Some("t1"));
}

#[tokio::test]
async fn remember_redacts_before_persisting() {
    let (mem, stub) = adapter(StubMemory::default());
    mem.remember(NewMemory::new(
        "key -----BEGIN PRIVATE KEY-----\nAAAABBBB\n-----END PRIVATE KEY-----",
    ))
    .await
    .unwrap();

    let stored = &stub.snapshot()[0].content;
    assert!(
        !stored.contains("AAAABBBB"),
        "a secret must not reach disk: {stored}"
    );
}

#[tokio::test]
async fn remember_rejects_text_that_is_empty_after_scrubbing() {
    let (mem, stub) = adapter(StubMemory::default());
    let err = mem.remember(NewMemory::new("   ")).await.unwrap_err();
    assert!(matches!(err, TinyAgentsError::Validation(_)), "{err:?}");
    assert!(stub.snapshot().is_empty());
}

#[tokio::test]
async fn each_write_gets_a_distinct_key_so_turns_cannot_overwrite_each_other() {
    // `Memory::store` upserts on (namespace, key), so a shared key would let
    // one turn silently replace another's memory.
    let (mem, stub) = adapter(StubMemory::default());
    let first = mem.remember(NewMemory::new("same text")).await.unwrap();
    let second = mem.remember(NewMemory::new("same text")).await.unwrap();

    assert_ne!(first, second);
    assert_eq!(stub.snapshot().len(), 2);
}

#[tokio::test]
async fn remember_returns_the_backends_own_id_not_the_synthetic_handle() {
    let (mem, _) = adapter(StubMemory::default());
    let id = mem.remember(NewMemory::new("a fact")).await.unwrap();
    assert_eq!(id.as_str(), "row-1", "read-back id keeps one id space");
}

#[tokio::test]
async fn a_write_failure_surfaces_as_an_error() {
    let (mem, _) = adapter(StubMemory::failing());
    let err = mem.remember(NewMemory::new("a fact")).await.unwrap_err();
    assert!(matches!(err, TinyAgentsError::Capability(_)), "{err:?}");
}

#[tokio::test]
async fn advisory_tags_are_dropped_rather_than_becoming_a_scope() {
    let (mem, stub) = adapter(StubMemory::default());
    mem.remember(NewMemory::new("a fact").with_tag("secret-ns").with_tag("x"))
        .await
        .unwrap();

    let rows = stub.snapshot();
    assert_eq!(
        rows[0].namespace.as_deref(),
        Some(DEFAULT_AGENT_MEMORY_NAMESPACE),
        "a tag must never redirect the write"
    );
    assert!(!rows[0].key.contains("secret-ns"));
}

// ── round trip / thread_summary / wiring ──────────────────────────────

#[tokio::test]
async fn remembered_text_comes_back_through_recall() {
    let (mem, _) = adapter(StubMemory::default());
    mem.remember(NewMemory::new("the sky is blue"))
        .await
        .unwrap();

    // Stored rows carry no score, so the min_score floor must not drop them.
    let items = mem.recall(RecallRequest::new("sky")).await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].text, "the sky is blue");
}

#[tokio::test]
async fn thread_summary_is_none_and_never_synthesized_from_recall() {
    let (mem, _) = adapter(StubMemory::with_rows(vec![entry("r1", "k1", "a fact")]));
    assert!(mem
        .thread_summary(&ThreadId::new("t1"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn usable_behind_a_trait_object() {
    let (mem, _) = adapter(StubMemory::default());
    let boxed: Box<dyn AgentMemory> = Box::new(mem);
    boxed.remember(NewMemory::new("dyn safe")).await.unwrap();
    assert_eq!(
        boxed.recall(RecallRequest::new("dyn")).await.unwrap().len(),
        1
    );
}

#[test]
fn a_blank_namespace_is_refused_rather_than_silently_widening_scope() {
    let (mem, _) = adapter(StubMemory::default());
    let mem = mem.with_namespace("   ");
    assert_eq!(mem.namespace, DEFAULT_AGENT_MEMORY_NAMESPACE);

    let (mem, _) = adapter(StubMemory::default());
    assert_eq!(mem.with_namespace("agent-notes").namespace, "agent-notes");
}

#[test]
fn limits_are_clamped_so_a_zero_item_recall_is_not_expressible() {
    let (mem, _) = adapter(StubMemory::default());
    let mem = mem.with_limits(0, 0);
    assert_eq!(mem.max_limit, 1);
    assert_eq!(mem.default_limit, 1);
    assert_eq!(mem.effective_limit(Some(0)), 1);
    assert_eq!(mem.effective_limit(None), 1);
}
