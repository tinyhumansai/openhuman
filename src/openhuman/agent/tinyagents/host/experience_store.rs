//! Host adapter: [`tinyagents_harness::host::ExperienceStore`] backed by
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
use tinyagents_harness::error::{Result, TinyAgentsError};
use tinyagents_harness::host::{Experience, ExperienceStore};

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
#[path = "experience_store_tests.rs"]
mod tests;
