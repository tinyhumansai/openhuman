//! Host adapter: [`tinyagents::harness::host::ExperienceStore`] backed by
//! OpenHuman's `agent_experience` domain.
//!
//! This is `docs/specs/plan-agents.md` Phase 4. The crate's runtime records what
//! it learned about *doing* a task and reads prior attempts back before a
//! similar one. OpenHuman already has exactly that domain — the Hermes-style
//! procedural memory in [`crate::openhuman::agent_experience`], written today by
//! [`AgentExperienceCaptureHook`](crate::openhuman::agent::experience::AgentExperienceCaptureHook)
//! and read by the retrieval path — so this adapter is a translation layer over
//! [`AgentExperienceStore`], not a new store.
//!
//! # The namespace separation the trait insists on
//!
//! The trait's module doc is emphatic that this is **not** `AgentMemory`:
//! procedural (how the agent performed) versus declarative (what the user
//! knows). OpenHuman shares one backing store between them — both go through
//! `Arc<dyn Memory>` — but they do **not** share a namespace: everything here is
//! confined to
//! [`AGENT_EXPERIENCE_NAMESPACE`](crate::openhuman::agent::experience::AGENT_EXPERIENCE_NAMESPACE)
//! under `experience/<id>` keys, which is precisely the "a host that has only
//! one backing store must still keep the two namespaces separate" case. Nothing
//! in this file reads or writes user memory namespaces.
//!
//! # Contract mismatches resolved here
//!
//! 1. **Ternary vs boolean outcome.** OpenHuman has
//!    [`ExperienceOutcome::Partial`]; the crate's `Experience` has only
//!    `success: bool`. Writing collapses `false` to `Failure`; reading maps
//!    `Success` to `true` and both `Failure` and `Partial` to `false`. Partial
//!    is *not* a success, and rounding it up would present a recovered-after-
//!    failure run as a clean one. The nuance survives in the prose we carry
//!    back in `outcome`, which names the OpenHuman outcome explicitly.
//! 2. **`lesson` is required; `Experience::outcome` may be empty.** The store
//!    rejects a blank `lesson`
//!    ([`AgentExperienceStore::put`]). The trait documents an empty `outcome` as
//!    normal *and* says `record` returning `Err` means the record was lost. So a
//!    blank outcome gets a synthesized, honest lesson line rather than an error.
//! 3. **`agent_id` is a score bonus, not a filter.** OpenHuman's
//!    `score_experience` only *boosts* an agent match, so a retrieval seeded
//!    with an agent id can still return another agent's records. The trait
//!    promises "prior attempts by `agent`", so this adapter filters the hits by
//!    agent id after retrieval. Filtering (rather than relaxing the promise) is
//!    the safe direction: it can only remove rows.
//!
//!    The **order** matters as much as the filter. The domain truncates to the
//!    requested `max_hits` before this adapter ever sees the rows, so filtering
//!    a truncated page would let a busier agent's highly-scored records occupy
//!    every slot and leave this recall empty while matching attempts sat just
//!    below the cut. So the query over-fetches (`candidate_hits`) and the
//!    truncation happens *after* the ownership filter, which is what makes
//!    `max_hits` mean "up to N of **this** agent's attempts".
//! 4. **Redaction stays with the host.** `put` runs the domain's
//!    [`redact_text`](crate::openhuman::agent::experience::redact_text) over the
//!    stored fields; this adapter also redacts on the way *in* before
//!    truncating, so a secret cannot survive by being pushed past the truncation
//!    boundary. Nothing here bypasses that guard.
//! 5. **No profile invention.** OpenHuman partitions experience by agent
//!    profile. The crate has no notion of one, so the profile is supplied to the
//!    adapter at construction and stamped onto every write; a profile-less
//!    adapter reproduces the legacy, unpartitioned behaviour byte-for-byte.

use std::sync::Arc;

use async_trait::async_trait;
use tinyagents::error::{Result, TinyAgentsError};
use tinyagents::harness::host::{Experience, ExperienceStore};

use crate::openhuman::agent::experience::store::{
    retrieve_across_stores, AgentExperienceStore, ExperienceQuery,
};
use crate::openhuman::agent::experience::types::{
    redact_text, stable_experience_id, stable_experience_id_for_profile, AgentExperience,
    ExperienceHit, ExperienceOutcome, ExperienceSource,
};
use crate::openhuman::memory::Memory;

/// Character cap applied to the prose fields written into the store.
///
/// Mirrors the `MAX_SUMMARY_CHARS` used by
/// [`AgentExperienceCaptureHook`](crate::openhuman::agent::experience::AgentExperienceCaptureHook)
/// so records written through the crate runtime and records written by the
/// native hook are the same shape in the store and render identically in the
/// experience prompt block. That constant is private to `capture.rs`, so the
/// value is restated here rather than imported.
const MAX_SUMMARY_CHARS: usize = 280;

/// Default number of prior attempts returned by
/// [`ExperienceStore::recall_for`].
///
/// Matches the RPC retrieval default (`RetrieveParams::max_hits` falls back to
/// `5`). The trait puts the bound on the host — "an implementation should …
/// bound the count itself" — so this is deliberately a host policy knob rather
/// than something the runtime passes in.
const DEFAULT_MAX_HITS: usize = 5;

/// Tag stamped on every record this adapter writes.
///
/// Makes runtime-written experience distinguishable from the native tool-loop
/// hook's records (`"tool-loop"`) when auditing the namespace, and gives
/// retrieval a tag to match on. Recording provenance on the host's own row is
/// exactly what the trait's `Experience` doc says a host should do.
const ADAPTER_TAG: &str = "tinyagents-runtime";

/// Confidence assigned to records written through this adapter.
///
/// Below the native hook's success confidence (`0.72`) and around its
/// partial-success figure (`0.62`): a runtime-reported attempt carries no tool
/// sequence and no error classification, so it is genuinely weaker evidence
/// than a record the capture hook derived from an observed turn. Confidence
/// only scales the base term in `score_experience`, so this affects ranking,
/// never inclusion.
const ADAPTER_CONFIDENCE: f32 = 0.6;

// ── Adapter ───────────────────────────────────────────────────────────────────

/// [`ExperienceStore`] over OpenHuman's `agent_experience` domain.
///
/// Holds an [`AgentExperienceStore`] (itself a thin façade over
/// `Arc<dyn Memory>` pinned to the experience namespace) plus the two pieces of
/// host context the crate cannot supply: the active agent profile and the
/// recall bound.
#[derive(Clone)]
pub struct OpenHumanExperienceStore {
    /// The namespace-scoped procedural store. All reads and writes go through
    /// it, so the `agent_experience` namespace confinement and the domain's
    /// redaction on `put` apply to everything this adapter does.
    store: AgentExperienceStore,
    /// Additional store consulted by `recall_for` only, never written.
    ///
    /// A dedicated-profile session keeps its procedural records in a
    /// profile-local memory subtree, but pre-profile builds wrote unstamped
    /// records into the shared workspace store. The live turn path
    /// (`session/turn/core.rs`) therefore queries both, profile-local first,
    /// and this adapter has to match it or a profile session would silently
    /// stop recalling everything it learned before profiles existed. Writes
    /// deliberately do **not** fan out: the profile-local store stays the sole
    /// write target so new records land inside the profile subtree.
    shared_recall_store: Option<AgentExperienceStore>,
    /// Agent profile the session runs under, stamped onto every write and used
    /// to partition recall. `None` is the profile-less session, whose records
    /// stay unstamped and are visible to every profile — the documented legacy
    /// behaviour, not a fallback we invented.
    profile_id: Option<String>,
    /// Maximum prior attempts returned by one `recall_for`.
    max_hits: usize,
}

impl OpenHumanExperienceStore {
    /// Adapter over `memory`, with no profile partition and the default recall
    /// bound.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self::with_profile(memory, None)
    }

    /// [`Self::new`] carrying the session's agent profile id.
    ///
    /// Blank ids are normalized to `None` so a caller threading an empty string
    /// through does not create a distinct, unreachable partition — the same
    /// normalization `stable_experience_id_for_profile` applies internally.
    pub fn with_profile(memory: Arc<dyn Memory>, profile_id: Option<String>) -> Self {
        Self::from_store(AgentExperienceStore::new(memory), profile_id)
    }

    /// Adapter over an already-opened [`AgentExperienceStore`].
    ///
    /// The RPC layer opens per-profile stores against different memory
    /// subtrees; this constructor lets a caller that has already resolved the
    /// right one hand it over instead of re-deriving it.
    pub fn from_store(store: AgentExperienceStore, profile_id: Option<String>) -> Self {
        Self {
            store,
            shared_recall_store: None,
            profile_id: normalized_profile(profile_id.as_deref()),
            max_hits: DEFAULT_MAX_HITS,
        }
    }

    /// Adds a second store that [`ExperienceStore::recall_for`] reads and
    /// [`ExperienceStore::record`] never writes.
    ///
    /// Used for the shared, pre-profile workspace store behind a
    /// dedicated-profile session. `None` is a no-op, so a profile-less session
    /// keeps the single-store behaviour.
    pub fn with_shared_recall_memory(mut self, memory: Option<Arc<dyn Memory>>) -> Self {
        self.shared_recall_store = memory.map(AgentExperienceStore::new);
        self
    }

    /// How many candidates to pull from the domain before the agent filter.
    ///
    /// Wider than [`Self::max_hits`] because the domain scores an agent match
    /// rather than filtering on it, so the candidate pool is mixed. The
    /// multiplier is a heuristic, not a guarantee — a store dominated by one
    /// very active agent can still crowd out a quieter one — but it turns the
    /// common "a handful of agents share a store" case from lossy into correct.
    /// `max_hits == 0` stays 0 so the documented "keep writing, feed none back"
    /// behaviour is preserved.
    fn candidate_hits(&self) -> usize {
        const AGENT_MIX_FACTOR: usize = 5;
        self.max_hits.saturating_mul(AGENT_MIX_FACTOR)
    }

    /// Overrides how many prior attempts one recall returns.
    ///
    /// Zero is honoured verbatim — `AgentExperienceStore::retrieve` short-
    /// circuits to an empty result — which is a legitimate way to keep writing
    /// experience while feeding none of it back.
    pub fn with_max_hits(mut self, max_hits: usize) -> Self {
        self.max_hits = max_hits;
        self
    }

    /// Translates a crate [`Experience`] into the domain record.
    ///
    /// `tool_sequence` and `tools_used` are left empty: the crate's
    /// `Experience` carries no tool trace, and fabricating one would poison
    /// `score_experience`'s tool-overlap term with tools that were never run.
    /// The renderer already handles the empty case (`"no tools"`).
    fn to_domain(&self, exp: &Experience) -> AgentExperience {
        let outcome = if exp.success {
            ExperienceOutcome::Success
        } else {
            ExperienceOutcome::Failure
        };
        // Redact before truncating: truncating first could cut a secret in half
        // and leave the tail unmatched by the redaction patterns.
        let task_summary = truncate_chars(&redact_text(&exp.task), MAX_SUMMARY_CHARS);
        // Namespaced so it can never be mistaken for a real tool name if the
        // digest inputs are ever inspected or logged.
        let agent_key = format!("agent:{}", exp.agent_id.trim().to_ascii_lowercase());
        let lesson = truncate_chars(&redact_text(&lesson_for(exp)), MAX_SUMMARY_CHARS);
        let reuse_hint = truncate_chars(&redact_text(&reuse_hint_for(exp)), MAX_SUMMARY_CHARS);

        AgentExperience {
            // Derived from the *stored* summary, so the id matches what a later
            // `put` of the same attempt would compute.
            //
            // The agent id is folded in through the `tool_sequence` slot, which
            // is otherwise empty here. That looks odd, so: the domain digest
            // covers task summary + tool sequence + outcome + profile and
            // deliberately **excludes** `agent_id`, because the native capture
            // hook always supplies a real tool sequence, which incidentally
            // keeps two agents' rows apart. This adapter has no tool trace to
            // supply (fabricating one would corrupt `score_experience`'s
            // tool-overlap term), so without this every agent recording the
            // same task with the same outcome would collide on one id and
            // `put` would upsert — the second writer silently destroying the
            // first agent's record. Using the one hashed field that is free
            // keeps identity per-agent without inventing a tool trace or
            // reaching into the domain's digest.
            id: stable_experience_id_for_profile(
                &task_summary,
                std::slice::from_ref(&agent_key),
                outcome,
                self.profile_id.as_deref(),
            ),
            // Left at zero so `put` stamps creation time itself, and preserves
            // the original `created_at_ms` when this id already exists.
            created_at_ms: 0,
            updated_at_ms: 0,
            // The runtime writes mechanically when a task finishes, which is
            // the tool loop's vantage point rather than a reflection step.
            source: ExperienceSource::ToolLoop,
            agent_id: normalized_profile(Some(&exp.agent_id)),
            entrypoint: None,
            profile_id: self.profile_id.clone(),
            // `capture.rs` fingerprints with the same public helper over an
            // empty sequence and a `Success` outcome, so fingerprints agree
            // across both writers for the same task text.
            task_fingerprint: stable_experience_id(&task_summary, &[], ExperienceOutcome::Success),
            task_summary,
            tools_used: Vec::new(),
            tool_sequence: Vec::new(),
            outcome,
            // No error taxonomy is available: the crate hands us prose, not a
            // classified failure.
            error_class: None,
            lesson,
            reuse_hint,
            avoid_hint: None,
            confidence: ADAPTER_CONFIDENCE,
            tags: vec![ADAPTER_TAG.to_string()],
            payload_hash: None,
            dismissed: false,
        }
    }
}

/// Trims a profile / agent id, mapping blank to `None`.
fn normalized_profile(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// The `lesson` line for a crate experience.
///
/// The store requires a non-empty lesson but the trait documents an empty
/// `outcome` as normal, so a blank one is replaced with a statement of the fact
/// we do have — whether the attempt succeeded. Erroring instead would report a
/// lost record for a perfectly valid one.
fn lesson_for(exp: &Experience) -> String {
    let outcome = exp.outcome.trim();
    if !outcome.is_empty() {
        return outcome.to_string();
    }
    if exp.success {
        "A prior attempt at this task succeeded; no further detail was recorded.".to_string()
    } else {
        "A prior attempt at this task did not succeed; no further detail was recorded.".to_string()
    }
}

/// The `reuse_hint` line, which the experience prompt block always renders.
///
/// Phrased as advice about the *previous* attempt rather than an instruction,
/// because the underlying prose is untrusted model/tool output and must not read
/// as a directive when it lands in a later prompt.
fn reuse_hint_for(exp: &Experience) -> String {
    if exp.success {
        format!(
            "A previous attempt at \"{}\" succeeded; the approach it used is worth considering.",
            exp.task.trim()
        )
    } else {
        format!(
            "A previous attempt at \"{}\" failed; treat its approach as unproven.",
            exp.task.trim()
        )
    }
}

/// Maps a domain hit back into the crate's inert record.
///
/// `outcome` reconstructs prose from the fields the domain actually stores. The
/// OpenHuman outcome is named explicitly so `Partial` — which has no
/// representation in `success: bool` — is not silently lost.
fn to_crate(hit: &ExperienceHit) -> Experience {
    let e = &hit.experience;
    let mut outcome = match e.outcome {
        ExperienceOutcome::Success => String::from("succeeded"),
        ExperienceOutcome::Failure => String::from("failed"),
        ExperienceOutcome::Partial => String::from("partially succeeded"),
    };
    if let Some(class) = e.error_class.as_deref().filter(|c| !c.trim().is_empty()) {
        outcome.push_str(&format!(" ({class})"));
    }
    if !e.lesson.trim().is_empty() {
        outcome.push_str(": ");
        outcome.push_str(e.lesson.trim());
    }
    if let Some(avoid) = e.avoid_hint.as_deref().filter(|h| !h.trim().is_empty()) {
        outcome.push_str(" Avoid: ");
        outcome.push_str(avoid.trim());
    }

    Experience {
        agent_id: e.agent_id.clone().unwrap_or_default(),
        task: e.task_summary.clone(),
        outcome,
        // Partial deliberately reads as "not a success"; see the module doc.
        success: matches!(e.outcome, ExperienceOutcome::Success),
    }
}

/// Truncates `input` to at most `max_chars` characters (not bytes).
fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect()
}

/// Case-insensitive, whitespace-insensitive agent id comparison, matching the
/// `normalize` the domain's scorer uses (which is private to `store.rs`).
fn same_agent(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

#[async_trait]
impl ExperienceStore for OpenHumanExperienceStore {
    async fn record(&self, exp: &Experience) -> Result<()> {
        if !exp.is_recallable() {
            // Nothing could ever match a record with no agent or no task, so
            // storing it only grows the namespace. Dropped, not rejected —
            // the trait treats an unrecallable record as a no-op, not a host
            // failure.
            tracing::debug!(
                target: "tinyagents",
                agent_id = %exp.agent_id,
                "[tinyagents][experience] dropping unrecallable experience"
            );
            return Ok(());
        }

        let record = self.to_domain(exp);
        tracing::debug!(
            target: "tinyagents",
            agent_id = %exp.agent_id,
            experience_id = %record.id,
            success = exp.success,
            profile_id = ?self.profile_id,
            "[tinyagents][experience] recording procedural experience"
        );
        self.store.put(record).await.map_err(|e| {
            // The trait says an Err means the record was lost; callers must not
            // fail the turn over it. Surfacing it as a memory-backend error is
            // accurate — the store is memory-backed.
            TinyAgentsError::Memory(format!("record agent experience: {e}"))
        })?;
        Ok(())
    }

    async fn recall_for(&self, agent_id: &str, task: &str) -> Result<Vec<Experience>> {
        let query = ExperienceQuery {
            query: task.to_string(),
            // No tool or tag seed: the crate gives us a task string only, and
            // an invented seed would skew the overlap terms.
            tools: Vec::new(),
            tags: Vec::new(),
            agent_id: normalized_profile(Some(agent_id)),
            entrypoint: None,
            profile_id: self.profile_id.clone(),
            // Over-fetch, because the agent filter below runs *after* the
            // domain has already truncated. Agent identity is only a score
            // bonus there, so another agent's highly-relevant records can fill
            // every slot and leave this adapter returning nothing while
            // matching records sit just below the cut. Widening the candidate
            // window and truncating after the filter is what makes `max_hits`
            // mean "up to N of *this agent's* attempts".
            max_hits: self.candidate_hits(),
        };

        // Profile-local store first, then the shared pre-profile one. Same
        // order and same dedupe-by-id/re-rank as the live turn path, so a
        // record visible to a native turn is visible here too.
        let mut stores = vec![self.store.clone()];
        if let Some(shared) = &self.shared_recall_store {
            stores.push(shared.clone());
        }

        let hits = retrieve_across_stores(&stores, query)
            .await
            .map_err(|e| TinyAgentsError::Memory(format!("recall agent experience: {e}")))?;

        // The domain only *boosts* an agent match, so filter here to keep the
        // trait's "prior attempts by `agent`" promise. Records with no agent id
        // are excluded rather than treated as shared: attributing an
        // unattributed attempt to this agent would be a guess.
        let found: Vec<Experience> = hits
            .iter()
            .filter(|hit| {
                hit.experience
                    .agent_id
                    .as_deref()
                    .is_some_and(|owner| same_agent(owner, agent_id))
            })
            .map(to_crate)
            // Truncate *after* filtering, so the bound counts this agent's
            // attempts rather than the mixed candidate pool.
            .take(self.max_hits)
            .collect();

        tracing::debug!(
            target: "tinyagents",
            %agent_id,
            candidates = hits.len(),
            returned = found.len(),
            max_hits = self.max_hits,
            "[tinyagents][experience] recalled prior attempts"
        );
        // Order is the domain's (score, then recency, then id) and is returned
        // as-is: the trait forbids the runtime re-ranking, so the host's ranking
        // is the answer.
        Ok(found)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tool_memory::test_helpers::MockMemory;

    fn adapter() -> OpenHumanExperienceStore {
        OpenHumanExperienceStore::new(Arc::new(MockMemory::default()))
    }

    fn exp(agent: &str, task: &str, outcome: &str, success: bool) -> Experience {
        let e = Experience::new(agent, task, outcome);
        if success {
            e.succeeded()
        } else {
            e
        }
    }

    #[tokio::test]
    async fn empty_store_recalls_nothing() {
        let found = adapter()
            .recall_for("planner", "migrate the schema")
            .await
            .expect("recall must not error on an empty store");
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn records_and_recalls_a_prior_attempt() {
        let store = adapter();
        store
            .record(&exp(
                "planner",
                "migrate the customer schema",
                "the migration step needed elevated permissions",
                false,
            ))
            .await
            .expect("record");

        let found = store
            .recall_for("planner", "migrate the customer schema")
            .await
            .expect("recall");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].agent_id, "planner");
        assert!(!found[0].success);
        assert!(
            found[0].outcome.contains("elevated permissions"),
            "the recorded prose must survive the round trip: {}",
            found[0].outcome
        );
    }

    #[tokio::test]
    async fn recall_spans_the_shared_store_while_writes_stay_profile_local() {
        // Stand in for a dedicated-profile session: `local` is the profile
        // subtree, `shared` the workspace store a pre-profile build wrote into.
        let local: Arc<dyn Memory> = Arc::new(MockMemory::default());
        let shared: Arc<dyn Memory> = Arc::new(MockMemory::default());

        // Seed the shared store the way a pre-profile build did — unstamped.
        OpenHumanExperienceStore::new(shared.clone())
            .record(&exp(
                "planner",
                "migrate the customer schema",
                "the legacy attempt hit a lock timeout",
                false,
            ))
            .await
            .expect("seed the shared store");

        let store = OpenHumanExperienceStore::with_profile(local.clone(), None)
            .with_shared_recall_memory(Some(shared.clone()));

        // Recall reaches the shared store even though nothing was written to
        // the profile-local one.
        let found = store
            .recall_for("planner", "migrate the customer schema")
            .await
            .expect("recall");
        assert_eq!(
            found.len(),
            1,
            "a profile session must still see pre-profile experience"
        );
        assert!(found[0].outcome.contains("lock timeout"));

        // A new record lands in the profile-local store, not the shared one.
        store
            .record(&exp("planner", "rotate the signing key", "clean run", true))
            .await
            .expect("record");

        // The domain scores rather than filters, so an unrelated query still
        // returns the seeded row — assert on which task each store *holds*,
        // not on the result count.
        let holds_new_task = |found: &[Experience]| {
            found
                .iter()
                .any(|e| e.task.contains("rotate the signing key"))
        };

        let shared_only = OpenHumanExperienceStore::new(shared)
            .recall_for("planner", "rotate the signing key")
            .await
            .expect("recall from the shared store");
        assert!(
            !holds_new_task(&shared_only),
            "writes must not fan out into the shared store, got: {shared_only:?}"
        );

        let local_only = OpenHumanExperienceStore::new(local)
            .recall_for("planner", "rotate the signing key")
            .await
            .expect("recall from the profile-local store");
        assert!(
            holds_new_task(&local_only),
            "the profile-local store is the write target, got: {local_only:?}"
        );
    }

    #[tokio::test]
    async fn recall_excludes_another_agents_attempt() {
        // The domain only score-boosts an agent match, so without the adapter's
        // post-filter this would return the writer's record too.
        let store = adapter();
        store
            .record(&exp(
                "planner",
                "migrate the customer schema",
                "worked",
                true,
            ))
            .await
            .expect("record");
        store
            .record(&exp(
                "writer",
                "migrate the customer schema",
                "worked",
                true,
            ))
            .await
            .expect("record");

        let found = store
            .recall_for("planner", "migrate the customer schema")
            .await
            .expect("recall");
        assert_eq!(found.len(), 1, "only the planner's attempt is in scope");
        assert_eq!(found[0].agent_id, "planner");
    }

    #[tokio::test]
    async fn an_empty_outcome_is_recorded_rather_than_erroring() {
        // The store rejects a blank `lesson`; the trait documents a blank
        // `outcome` as normal. A synthesized lesson reconciles the two.
        let store = adapter();
        store
            .record(&exp("planner", "deploy the service", "", true))
            .await
            .expect("a blank outcome must not be reported as a lost record");

        let found = store
            .recall_for("planner", "deploy the service")
            .await
            .expect("recall");
        assert_eq!(found.len(), 1);
        assert!(found[0].success);
        assert!(!found[0].outcome.trim().is_empty());
    }

    #[tokio::test]
    async fn unrecallable_records_are_dropped_without_error() {
        let store = adapter();
        store
            .record(&exp("", "migrate", "ok", true))
            .await
            .expect("record must not fail");
        store
            .record(&exp("planner", "   ", "ok", true))
            .await
            .expect("record must not fail");
        assert!(store
            .recall_for("planner", "migrate")
            .await
            .expect("recall")
            .is_empty());
    }

    #[tokio::test]
    async fn recall_is_bounded_by_the_host_not_the_runtime() {
        let store = adapter().with_max_hits(0);
        store
            .record(&exp("planner", "deploy the service", "ok", true))
            .await
            .expect("record");
        assert!(store
            .recall_for("planner", "deploy the service")
            .await
            .expect("recall")
            .is_empty());
    }

    #[tokio::test]
    async fn another_agents_records_cannot_crowd_out_this_agents_attempts() {
        // The domain scores an agent match rather than filtering on it, so a
        // busier agent's records rank alongside this one's. With truncation
        // before the agent filter, they could fill every slot and this recall
        // would return nothing even though matching attempts exist.
        let store = adapter().with_max_hits(2);
        for i in 0..8 {
            store
                .record(&exp("writer", &format!("ship release {i}"), "ok", true))
                .await
                .expect("record");
        }
        store
            .record(&exp(
                "planner",
                "ship release 9",
                "planner's own attempt",
                true,
            ))
            .await
            .expect("record");

        let found = store
            .recall_for("planner", "ship release")
            .await
            .expect("recall");

        assert!(
            !found.is_empty(),
            "the planner's attempt must survive a store dominated by another agent"
        );
        assert!(
            found.iter().all(|e| e.agent_id == "planner"),
            "only this agent's attempts may be returned"
        );
        assert!(found.len() <= 2, "max_hits still bounds the result");
    }

    #[tokio::test]
    async fn secrets_are_redacted_before_they_reach_the_store() {
        let store = adapter();
        store
            .record(&exp(
                "planner",
                "call the deployment endpoint",
                "failed until we set token=hunter2supersecret",
                false,
            ))
            .await
            .expect("record");

        let found = store
            .recall_for("planner", "call the deployment endpoint")
            .await
            .expect("recall");
        assert_eq!(found.len(), 1);
        assert!(
            !found[0].outcome.contains("hunter2supersecret"),
            "the domain's redaction guard must not be bypassed: {}",
            found[0].outcome
        );
        // Case-insensitive on purpose. More than one scrubber can run on this
        // path — `agent::experience::redact_text` writes `token=[redacted]`,
        // while the memory store's own safety pass replaces the whole
        // `key=value` with `[REDACTED]` — and which one lands first is not this
        // adapter's contract. What *is* the contract is the assertion above:
        // the secret must not survive. This second assertion only pins that
        // some redaction visibly happened rather than the prose being silently
        // dropped, so it must not break when a stronger scrubber wins.
        assert!(
            found[0].outcome.to_ascii_lowercase().contains("[redacted]"),
            "a redaction marker must survive into the recalled prose: {}",
            found[0].outcome
        );
    }

    #[test]
    fn writes_stay_inside_the_procedural_namespace() {
        // The whole point of the trait is that procedural experience does not
        // leak into the user's declarative memory. Pin the namespace constant
        // this adapter is confined to.
        assert_eq!(
            crate::openhuman::agent::experience::AGENT_EXPERIENCE_NAMESPACE,
            "agent_experience"
        );
    }

    #[test]
    fn partial_outcomes_read_as_unsuccessful() {
        // OpenHuman's ternary outcome has no boolean equivalent; a partial run
        // must not round up to a success.
        let hit = ExperienceHit {
            experience: AgentExperience {
                id: "exp_test".into(),
                created_at_ms: 1,
                updated_at_ms: 1,
                source: ExperienceSource::ToolLoop,
                agent_id: Some("planner".into()),
                entrypoint: None,
                profile_id: None,
                task_fingerprint: "fp".into(),
                task_summary: "migrate the schema".into(),
                tools_used: vec![],
                tool_sequence: vec![],
                outcome: ExperienceOutcome::Partial,
                error_class: None,
                lesson: "recovered after the first tool failed".into(),
                reuse_hint: "switch strategy".into(),
                avoid_hint: Some("do not repeat the failed call".into()),
                confidence: 0.62,
                tags: vec![],
                payload_hash: None,
                dismissed: false,
            },
            score: 1.0,
            match_reasons: vec![],
        };

        let mapped = to_crate(&hit);
        assert!(!mapped.success, "partial is not a success");
        assert!(mapped.outcome.contains("partially succeeded"));
        assert!(mapped.outcome.contains("recovered after the first tool"));
        assert!(mapped.outcome.contains("do not repeat the failed call"));
    }

    #[test]
    fn profile_ids_partition_the_storage_key() {
        let memory: Arc<dyn Memory> = Arc::new(MockMemory::default());
        let none = OpenHumanExperienceStore::with_profile(memory.clone(), None);
        let alice =
            OpenHumanExperienceStore::with_profile(memory.clone(), Some("alice".to_string()));
        let blank = OpenHumanExperienceStore::with_profile(memory, Some("   ".to_string()));

        let e = exp("planner", "migrate the schema", "ok", true);
        let none_id = none.to_domain(&e).id;
        let alice_id = alice.to_domain(&e).id;
        // A blank profile id must normalize to the profile-less partition, not
        // create a third unreachable one.
        assert_eq!(none_id, blank.to_domain(&e).id);
        assert_ne!(none_id, alice_id);
        assert_eq!(alice.to_domain(&e).profile_id.as_deref(), Some("alice"));
        assert!(none.to_domain(&e).profile_id.is_none());
    }

    #[test]
    fn long_prose_is_truncated_by_characters_not_bytes() {
        let long = "é".repeat(MAX_SUMMARY_CHARS + 50);
        let record = adapter().to_domain(&exp("planner", &long, "ok", true));
        assert_eq!(record.task_summary.chars().count(), MAX_SUMMARY_CHARS);
    }

    #[test]
    fn recorded_rows_carry_no_fabricated_tool_trace() {
        // A fake tool sequence would corrupt `score_experience`'s tool-overlap
        // term for every later query.
        let record = adapter().to_domain(&exp("planner", "deploy", "ok", true));
        assert!(record.tool_sequence.is_empty());
        assert!(record.tools_used.is_empty());
        assert_eq!(record.tags, vec![ADAPTER_TAG.to_string()]);
        assert_eq!(record.source, ExperienceSource::ToolLoop);
    }
}
