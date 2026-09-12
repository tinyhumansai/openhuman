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
//! - [`crate::openhuman::memory::safety`] — `sanitize_text`, the
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
use tinyagents_harness::error::{Result as TaResult, TinyAgentsError};
use tinyagents_harness::host::{AgentMemory, MemoryId, MemoryItem, NewMemory, RecallRequest};
use tinyagents_harness::ids::ThreadId;

use crate::openhuman::memory::agent::memory_loader::MemoryCitation;
use crate::openhuman::memory::safety::sanitize_text;
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
/// [`tinymemory_core::store::factories::create_memory`]), and every
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
        // Bound outside the `RecallOpts` literal: the struct borrows it.
        let current_thread_id_ref =
            crate::openhuman::agent::tinyagents::thread_context::current_thread_id();

        let opts = RecallOpts {
            namespace: Some(self.namespace.as_str()),
            category: None,
            session_id: session,
            min_score: Some(self.min_score),
            // The self-echo exclusion, passed explicitly rather than left to the
            // engine's `thread_context` task-local. That task-local is only
            // visible to an in-process engine; once memory is reached through
            // the loadable module it reads as absent on the far side, and
            // absent means "exclude nothing" — the agent gets handed back what
            // it just said. Resolving it here keeps the behaviour identical on
            // both paths.
            //
            // It does not fight `session_id` above. The engine filters only
            // document-kind hits by this field, while `session_id` and
            // `cross_session` scope the episodic and event tiers — so scoping
            // to a session and excluding it is not a contradiction, and a
            // thread hint still narrows *to* that session.
            //
            // Ambient first, the request's thread as fallback — not either
            // alone. Inside a turn the task-local names the thread whose
            // trigger was auto-saved, and when both are set they agree. But a
            // recall reaching this adapter *outside* a turn (an RPC-driven
            // recall carrying a thread hint) has no ambient value, and its
            // hint names exactly the thread whose saved trigger would echo
            // back. Dropping the fallback reintroduces the echo on that path.
            exclude_session_id: current_thread_id_ref.as_deref().or(session),
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
    // summary column on `crate::openhuman::memory::conversations::ConversationThread`,
    // or a thread-scoped digest in `memory_tree::summarise`), return it here.
    async fn thread_summary(&self, _thread: &ThreadId) -> TaResult<Option<String>> {
        Ok(None)
    }
}

#[cfg(test)]
#[path = "agent_memory_tests.rs"]
mod tests;
