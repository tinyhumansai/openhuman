//! Host capability adapter: [`AgentMemory`] over OpenHuman's memory stack.
//!
//! This is `docs/specs/plan-agents.md` Phase 4 for the memory seam. The agent
//! runtime is being made generic over its host, so it must stop reaching into
//! [`crate::openhuman::memory`] directly. Instead the crate declares
//! [`AgentMemory`] and this module is the single place OpenHuman's memory
//! domains meet it.
//!
//! # Domains adapted
//!
//! - [`crate::openhuman::memory`] — the `Memory` trait, `MemoryEntry`,
//!   `MemoryCategory`, `MemoryTaint`, `RecallOpts`.
//! - `crate::openhuman::agent::tinyagents::retriever::recall_through_facade` — the
//!   existing retrieval facade (issue #4249, 09.2). Recall goes **through** it
//!   rather than calling `Memory::recall` directly, so this adapter inherits
//!   OpenHuman's ranking engine verbatim, the `path_scope` dedupe rule, and the
//!   `AgentEvent::MemoryLoaded` emission instead of forking a second recall path.
//! - [`crate::openhuman::memory::store::safety`] — `sanitize_text`, the
//!   conservative secret + PII scrubber, applied on the way out of recall and on
//!   the way in to `remember`.
//! - [`crate::openhuman::memory::agent::memory_loader::MemoryCitation`] — the
//!   host's citation shape, rendered down to the opaque string the crate wants.
//!
//! # Where the policy lives, and why it lives *here*
//!
//! The trait's contract is explicit: items returned by [`AgentMemory::recall`]
//! have **already** been scope-filtered and redacted, and the runtime is
//! forbidden from re-ranking or re-filtering them. That makes this adapter the
//! last place OpenHuman's guards can run, so all of them run here:
//!
//! - **Scope is host-chosen, never runtime-chosen.** `RecallRequest`'s
//!   `agent_id` / `thread_id` / `limit` are documented as *hints about the
//!   caller*, not instructions about storage. The namespace comes from this
//!   adapter's own configuration and is never derived from a runtime field —
//!   the same "the tool has no namespace parameter" discipline
//!   `crate::openhuman::flows::memory_tools` uses to keep a flow inside its
//!   own sandbox. A thread hint may only *narrow* the query (it becomes
//!   `RecallOpts::session_id`), never widen it, and `cross_session` stays a
//!   wiring-time decision that defaults to `false`.
//! - **A defensive second scope pass.** Even with `session_id` set, recalled
//!   rows are re-checked host-side and any row carrying a *different* session is
//!   dropped. Unscoped rows (`session_id: None`) stay visible, matching the
//!   crate's own `InMemoryAgentMemory` scoping semantics. The pass is skipped
//!   when `cross_session` is on: that flag was already sent to the backend as an
//!   instruction to return other sessions' rows, so a same-session re-test would
//!   throw away everything the widening returned and reduce the opt-in to a
//!   no-op. The guard exists to catch a backend that ignored a *narrowing*
//!   request, and widening is the opposite of that.
//! - **Redaction is unconditional.** Every returned `text` and every citation
//!   snippet goes through `sanitize_text`, so no raw store row reaches the
//!   runtime. A row that the scrubber changed is logged (counts only, never
//!   content).
//! - **Provenance is stamped host-side.** `NewMemory` deliberately carries no
//!   taint field, so this adapter supplies one and writes through
//!   `Memory::store_with_taint`. It **fails closed to
//!   [`MemoryTaint::ExternalSync`]**: an agent turn may have been summarizing an
//!   email or a web page, this adapter cannot tell, and `ExternalSync` is the
//!   value OpenHuman's subconscious gate treats as "unknown origin, refuse
//!   external-effect tools". [`OpenHumanAgentMemory::with_taint`] lets a wiring
//!   site that genuinely knows better relax it.
//!
//! # Contract mismatches resolved
//!
//! 1. **`remember` returns an id, `Memory::store` does not.** OpenHuman's write
//!    path is an upsert keyed by `(namespace, key)` and hands nothing back. To
//!    keep the id space coherent with the ids `recall` returns, this adapter
//!    reads the row back with `Memory::get` after storing and returns the
//!    backend's own `MemoryEntry::id`, falling back to the synthetic
//!    `"{namespace}/{key}"` handle when the read-back finds nothing.
//! 2. **`MemoryItem::citation` is an opaque `String`.** The host's
//!    `MemoryCitation` is a structured UI contract; leaking its JSON would
//!    couple the crate to OpenHuman's frontend. It is built for real (so the
//!    seam uses the host type rather than a parallel one) and then rendered to a
//!    single flat `openhuman:memory/...` line by [`render_citation`].
//! 3. **`thread_summary` returns `Ok(None)`.** See the method docs — OpenHuman
//!    has no host-authored per-thread prose rollup to return, and the trait
//!    forbids synthesizing a substitute.
//! 4. **`NewMemory::tags` are dropped.** The trait calls them advisory and
//!    explicitly permits a host to discard them; OpenHuman's `Memory::store` has
//!    no tag column, and folding runtime-supplied labels into the namespace or
//!    key would turn an advisory hint into a scope, which the trait forbids.
//!    They are logged and otherwise ignored.

use std::sync::Arc;

use async_trait::async_trait;
use tinyagents::error::{Result as TaResult, TinyAgentsError};
use tinyagents::harness::host::{AgentMemory, MemoryId, MemoryItem, NewMemory, RecallRequest};
use tinyagents::harness::ids::ThreadId;

use crate::openhuman::memory::agent::memory_loader::MemoryCitation;
use crate::openhuman::memory::store::safety::sanitize_text;
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, MemoryTaint, RecallOpts};
use crate::openhuman::util::truncate_with_ellipsis;

/// Namespace agent-produced memories are written to and recalled from when the
/// wiring site does not choose one.
///
/// `"global"` is `tinycortex::memory::GLOBAL_NAMESPACE`, which is also what
/// `RecallOpts { namespace: None, .. }` falls back to — so the default is the
/// same namespace the rest of OpenHuman's recall already reads.
pub const DEFAULT_AGENT_MEMORY_NAMESPACE: &str = "global";

/// Number of items recalled when the runtime supplies no `limit`.
///
/// Matches `DefaultMemoryLoader`'s default `limit` so this seam injects the same
/// amount of context the legacy loader did.
pub const DEFAULT_RECALL_LIMIT: usize = 5;

/// Hard ceiling on how many items one recall may return, whatever the runtime
/// asks for.
///
/// The trait documents `limit` as an upper bound the *host* may lower for its
/// own reasons; this is that reason. Without it a runtime could turn one recall
/// into an unbounded scan of the user's memory.
pub const MAX_RECALL_LIMIT: usize = 50;

/// Relevance floor applied to recall.
///
/// Matches `DefaultMemoryLoader`'s `min_relevance_score`, so a memory that was
/// too weak to be injected by the legacy loader stays too weak here.
pub const DEFAULT_MIN_RELEVANCE_SCORE: f64 = 0.4;

/// Characters of a recalled memory carried into its citation snippet.
///
/// Matches `collect_recall_citations`, so a citation rendered through this
/// adapter carries the same amount of text the RPC surface already shows.
const CITATION_SNIPPET_CHARS: usize = 280;

/// OpenHuman's implementation of the crate's durable-memory capability.
///
/// Holds an `Arc<dyn Memory>` rather than building one: memory construction
/// needs a `MemoryConfig` plus a workspace dir (see
/// [`crate::openhuman::memory::store::factories::create_memory`]), and every
/// live call site already has a constructed backend in hand. Taking the handle
/// keeps this file a pure adapter and keeps the backend selection decision where
/// it already lives.
///
/// Every knob below is a **host** decision, deliberately not reachable from the
/// runtime side of the trait.
pub struct OpenHumanAgentMemory {
    /// The backend recall reads from and `remember` writes to.
    memory: Arc<dyn Memory>,
    /// The one namespace this adapter may touch. Never derived from a runtime
    /// field.
    namespace: String,
    /// Items returned when the runtime supplies no `limit`.
    default_limit: usize,
    /// Ceiling applied to whatever `limit` the runtime asks for.
    max_limit: usize,
    /// Relevance floor handed to `RecallOpts::min_score`.
    min_score: f64,
    /// Whether recall may reach conversational hits from other sessions.
    /// Defaults to `false` (tightest scope); widening it is a wiring decision.
    cross_session: bool,
    /// Provenance stamped on every `remember`. Fails closed to
    /// [`MemoryTaint::ExternalSync`].
    taint: MemoryTaint,
    /// Category stamped on every `remember`.
    category: MemoryCategory,
}

impl OpenHumanAgentMemory {
    /// Wraps `memory` with OpenHuman's default recall scope and a fail-closed
    /// `ExternalSync` write taint.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self {
            memory,
            namespace: DEFAULT_AGENT_MEMORY_NAMESPACE.to_string(),
            default_limit: DEFAULT_RECALL_LIMIT,
            max_limit: MAX_RECALL_LIMIT,
            min_score: DEFAULT_MIN_RELEVANCE_SCORE,
            cross_session: false,
            taint: MemoryTaint::ExternalSync,
            category: MemoryCategory::Conversation,
        }
    }

    /// Pins the namespace this adapter reads and writes.
    ///
    /// A blank namespace is ignored rather than accepted: an empty string would
    /// silently mean "the backend's fallback namespace", which is a scope change
    /// disguised as a typo.
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        let namespace = namespace.into();
        if namespace.trim().is_empty() {
            tracing::warn!(
                target: "tinyagents",
                "[tinyagents::host::memory] blank namespace ignored; keeping {}",
                self.namespace
            );
            return self;
        }
        self.namespace = namespace;
        self
    }

    /// Overrides the no-`limit` default and the ceiling.
    ///
    /// Both are clamped to at least 1 — a zero-item recall is indistinguishable
    /// from a broken backend at the call site, so it is not an expressible
    /// configuration.
    pub fn with_limits(mut self, default_limit: usize, max_limit: usize) -> Self {
        self.max_limit = max_limit.max(1);
        self.default_limit = default_limit.max(1).min(self.max_limit);
        self
    }

    /// Overrides the relevance floor handed to `RecallOpts::min_score`.
    pub fn with_min_score(mut self, min_score: f64) -> Self {
        self.min_score = min_score;
        self
    }

    /// Allows recall to reach conversational hits from other sessions.
    ///
    /// Off by default. This is the widest scope decision the adapter can make,
    /// so it is a wiring-time opt-in and never inferable from a runtime hint.
    pub fn with_cross_session(mut self, cross_session: bool) -> Self {
        self.cross_session = cross_session;
        self
    }

    /// Overrides the provenance stamped on `remember`.
    ///
    /// Only call this from a site that genuinely knows the turn's content could
    /// not have come from an external source. The default is the restrictive
    /// value on purpose.
    pub fn with_taint(mut self, taint: MemoryTaint) -> Self {
        self.taint = taint;
        self
    }

    /// Overrides the category stamped on `remember`.
    pub fn with_category(mut self, category: MemoryCategory) -> Self {
        self.category = category;
        self
    }

    /// Resolves the effective item cap for one request.
    fn effective_limit(&self, requested: Option<usize>) -> usize {
        match requested {
            Some(0) | None => self.default_limit,
            Some(n) => n.min(self.max_limit),
        }
    }

    /// Whether a recalled row survives the defensive host-side scope pass.
    ///
    /// A row with no session is unscoped and visible to every request; a scoped
    /// row is visible only to a request naming the same session. A request that
    /// names no session sees everything the backend already scoped for it.
    ///
    /// The backend's `RecallOpts::session_id` should have done this already —
    /// this is the belt-and-braces half, because the trait makes the runtime
    /// trust whatever comes back and there is no second filter downstream.
    ///
    /// `cross_session` disables the pass entirely. It has to: the same flag was
    /// already sent to the backend as an explicit instruction to return other
    /// sessions' rows, so re-applying a same-session test here would discard
    /// precisely the rows the widening produced and make the opt-in behave
    /// exactly like `false`. The guard protects against a backend that ignored
    /// a *narrowing* request, which is not what this is.
    fn scope_allows(cross_session: bool, requested: Option<&str>, stored: Option<&str>) -> bool {
        if cross_session {
            return true;
        }
        match (requested, stored) {
            (Some(requested), Some(stored)) => requested == stored,
            (Some(_), None) => true,
            (None, _) => true,
        }
    }

    /// Projects one already-scope-checked [`MemoryEntry`] onto a redacted
    /// [`MemoryItem`].
    ///
    /// Redaction runs before anything is copied out, so both the injected `text`
    /// and the citation snippet are scrubbed from the same cleaned string.
    fn item_from_entry(entry: &MemoryEntry) -> MemoryItem {
        let cleaned = sanitize_text(&entry.content);
        if cleaned.report.changed() {
            // Counts only — logging the matched span would defeat the redaction.
            tracing::debug!(
                target: "tinyagents",
                entry_id = %entry.id,
                secrets = cleaned.report.blocked_secret_hits,
                text = cleaned.report.text_redactions,
                pii = cleaned.report.pii_redactions,
                "[tinyagents::host::memory] redacted a recalled entry before injection"
            );
        }

        let snippet = if cleaned.value.chars().count() > CITATION_SNIPPET_CHARS {
            truncate_with_ellipsis(&cleaned.value, CITATION_SNIPPET_CHARS)
        } else {
            cleaned.value.clone()
        };

        let citation = MemoryCitation {
            id: entry.id.clone(),
            key: entry.key.clone(),
            namespace: entry.namespace.clone(),
            score: entry.score,
            timestamp: entry.timestamp.clone(),
            snippet,
        };

        let mut item = MemoryItem::new(entry.id.clone(), cleaned.value)
            .with_citation(render_citation(&citation));
        if let Some(score) = entry.score {
            item = item.with_score(score as f32);
        }
        item
    }
}

/// Flattens the host's [`MemoryCitation`] into the opaque string the crate
/// carries.
///
/// The crate types `MemoryItem::citation` as a `String` precisely so a host's
/// citation shape stays a host concern, so this deliberately does **not**
/// serialize the struct — a JSON blob would export OpenHuman's field names into
/// a redistributed crate and make them a de-facto wire contract. The rendered
/// form is a single flat line the runtime passes through and never parses.
///
/// The snippet is intentionally omitted: it is already the item's `text`, and
/// duplicating it into an attribution string doubles the tokens injected per
/// recalled memory.
pub fn render_citation(citation: &MemoryCitation) -> String {
    let namespace = citation.namespace.as_deref().unwrap_or("global");
    let mut out = format!(
        "openhuman:memory/{}/{}#{}",
        namespace, citation.key, citation.id
    );
    if !citation.timestamp.is_empty() {
        out.push('@');
        out.push_str(&citation.timestamp);
    }
    out
}

#[async_trait]
impl AgentMemory for OpenHumanAgentMemory {
    /// Recalls through OpenHuman's ranking engine, then scope-filters and
    /// redacts before handing anything back.
    ///
    /// Order is preserved exactly as the ranking engine produced it — the trait
    /// forbids the runtime from re-sorting, so re-sorting here would silently
    /// become the final order with no way for a caller to recover the host's.
    ///
    /// An empty result is `Ok(vec![])`; `Err` is reserved for a backend that
    /// could not answer, so "this deployment has no memory" (expressed by the
    /// wiring site passing `None`) stays distinguishable from "the store is
    /// down".
    async fn recall(&self, req: RecallRequest) -> TaResult<Vec<MemoryItem>> {
        let limit = self.effective_limit(req.limit);
        let session = req.thread_id.as_ref().map(|t| t.as_str());

        let opts = RecallOpts {
            namespace: Some(self.namespace.as_str()),
            category: None,
            session_id: session,
            min_score: Some(self.min_score),
            // Widening past the requested session is a wiring decision, never a
            // runtime hint.
            cross_session: self.cross_session,
        };

        let entries = crate::openhuman::agent::tinyagents::retriever::recall_through_facade(
            self.memory.as_ref(),
            &req.query,
            limit,
            opts,
        )
        .await
        .map_err(|e| TinyAgentsError::Capability(format!("openhuman memory recall failed: {e}")))?;

        let total = entries.len();
        let items: Vec<MemoryItem> = entries
            .iter()
            .filter(|entry| {
                Self::scope_allows(self.cross_session, session, entry.session_id.as_deref())
            })
            .map(Self::item_from_entry)
            .collect();

        tracing::debug!(
            target: "tinyagents",
            query_chars = req.query.chars().count(),
            agent_id = req.agent_id.as_deref().unwrap_or("<none>"),
            session = session.unwrap_or("<none>"),
            namespace = %self.namespace,
            limit,
            recalled = total,
            returned = items.len(),
            "[tinyagents::host::memory] recall scope-filtered and redacted"
        );

        Ok(items)
    }

    /// Stores a turn-produced memory, stamping namespace, key, category, and
    /// provenance host-side.
    ///
    /// The text is scrubbed before it is persisted, not only on the way out:
    /// a secret that reaches disk is a secret that leaks through every other
    /// reader of the store, not just this seam.
    async fn remember(&self, item: NewMemory) -> TaResult<MemoryId> {
        let cleaned = sanitize_text(&item.text);
        if cleaned.value.trim().is_empty() {
            return Err(TinyAgentsError::Validation(
                "refusing to store an empty memory".to_string(),
            ));
        }
        if cleaned.report.changed() {
            tracing::debug!(
                target: "tinyagents",
                secrets = cleaned.report.blocked_secret_hits,
                text = cleaned.report.text_redactions,
                pii = cleaned.report.pii_redactions,
                "[tinyagents::host::memory] redacted a memory before persisting it"
            );
        }
        if !item.tags.is_empty() {
            // Advisory only, and the trait forbids treating them as a scope, so
            // they are counted and dropped rather than folded into the key.
            tracing::debug!(
                target: "tinyagents",
                tags = item.tags.len(),
                "[tinyagents::host::memory] discarding advisory tags; no tag column exists"
            );
        }

        // The key is host-minted and unique per write: `Memory::store` upserts on
        // `(namespace, key)`, and a runtime-derivable key would let one turn
        // overwrite another's memory.
        let key = format!("agent.{}", uuid::Uuid::new_v4());
        let session = item.thread_id.as_ref().map(|t| t.as_str());

        self.memory
            .store_with_taint(
                &self.namespace,
                &key,
                &cleaned.value,
                self.category.clone(),
                session,
                self.taint,
            )
            .await
            .map_err(|e| {
                TinyAgentsError::Capability(format!("openhuman memory write failed: {e}"))
            })?;

        // Read back so the returned id lives in the same space as the ids
        // `recall` hands out; the synthetic handle is the honest fallback when
        // the backend cannot serve the row it just accepted.
        let id = match self.memory.get(&self.namespace, &key).await {
            Ok(Some(entry)) => entry.id,
            Ok(None) => format!("{}/{}", self.namespace, key),
            Err(e) => {
                tracing::warn!(
                    target: "tinyagents",
                    error = %e,
                    "[tinyagents::host::memory] stored, but reading the id back failed; \
                     returning the synthetic handle"
                );
                format!("{}/{}", self.namespace, key)
            }
        };

        tracing::debug!(
            target: "tinyagents",
            agent_id = item.agent_id.as_deref().unwrap_or("<none>"),
            session = session.unwrap_or("<none>"),
            namespace = %self.namespace,
            taint = self.taint.as_db_str(),
            "[tinyagents::host::memory] stored a turn-produced memory"
        );

        Ok(MemoryId::new(id))
    }

    /// Always `Ok(None)`.
    ///
    /// OpenHuman has no host-authored per-thread prose rollup to return.
    /// `ConversationThread` (`memory_conversations`) is metadata — title, counts,
    /// timestamps, labels — not a summary, and the `memory_tree` digests are
    /// scoped to sources and the entity index rather than to one thread.
    ///
    /// The trait is explicit that a runtime must not synthesize a substitute
    /// from recalled items because a synthesized summary is indistinguishable
    /// downstream from a host-authored one. That prohibition applies with equal
    /// force to a host adapter faking one, so this returns the contract-legal
    /// "no summary" rather than concatenating recall output.
    ///
    // TODO(phase4): if a real per-thread rollup lands (the natural home is a
    // summary column on `tinycortex::memory::conversations::ConversationThread`,
    // or a thread-scoped digest in `memory_tree::summarise`), return it here.
    async fn thread_summary(&self, _thread: &ThreadId) -> TaResult<Option<String>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
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
    }

    impl StubMemory {
        fn with_rows(rows: Vec<MemoryEntry>) -> Self {
            Self {
                rows: Mutex::new(rows),
                fail: None,
            }
        }

        fn failing() -> Self {
            Self {
                rows: Mutex::new(Vec::new()),
                fail: Some("backend down".to_string()),
            }
        }

        fn snapshot(&self) -> Vec<MemoryEntry> {
            self.rows.lock().unwrap().clone()
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
}
