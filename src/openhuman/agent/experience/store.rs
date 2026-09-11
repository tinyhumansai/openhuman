use crate::openhuman::agent::experience::types::{
    stable_experience_id_for_profile, AgentExperience, ExperienceHit,
};
use crate::openhuman::memory::safety::sanitize_text;
use crate::openhuman::memory::{Memory, MemoryCategory};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const AGENT_EXPERIENCE_NAMESPACE: &str = "agent_experience";

/// Encode a serialized experience so its structured payload survives the memory
/// layer's free-text content sanitizer.
///
/// `Memory::store` runs every document's `content` through the secret/PII
/// scrubber (`tinycortex … safety::sanitize_text`), whose *bare-numeric* PII
/// patterns (credit-card via Luhn, CPF, CNPJ) match any 11–19-digit run. A
/// serialized `AgentExperience` embeds `created_at_ms` / `updated_at_ms` as bare
/// 13-digit millisecond timestamps, so whenever `now_ms()` happens to be
/// Luhn-valid (~10% of the time) the scrubber rewrites the number to a
/// `[REDACTED_PII_*]` token — corrupting the JSON so it no longer parses back on
/// read. The record then silently vanishes from [`AgentExperienceStore::list`],
/// making recall non-deterministic run-to-run (issue #5209).
///
/// Base64 has no 11+-digit bare-numeric runs (and no `Bearer`/`sk-` literals),
/// so the sanitizer is a guaranteed no-op over the encoded payload and the
/// round-trip is lossless. The store still redacts the sensitive free-text
/// fields itself via [`redact_experience`] before serialization, so this does
/// not weaken secret handling.
fn encode_experience_payload(json: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(json.as_bytes())
}

/// Decode a stored experience payload.
///
/// New records are base64(JSON) (see [`encode_experience_payload`]); legacy
/// records are plain JSON. A JSON object starts with `{`, which is not in the
/// base64 alphabet, so the base64 decode fails cleanly on legacy content and we
/// fall back to parsing it as plain JSON — no ambiguity, no migration step.
fn decode_experience_payload(stored: &str) -> Result<AgentExperience, String> {
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(stored.trim()) {
        if let Ok(text) = std::str::from_utf8(&bytes) {
            if let Ok(experience) = serde_json::from_str::<AgentExperience>(text) {
                return Ok(experience);
            }
        }
    }
    serde_json::from_str::<AgentExperience>(stored)
        .map_err(|e| format!("parse agent experience: {e}"))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceQuery {
    pub query: String,
    pub tools: Vec<String>,
    pub tags: Vec<String>,
    pub agent_id: Option<String>,
    pub entrypoint: Option<String>,
    /// Profile partition (1c). When `Some(P)`, retrieval returns records stamped
    /// `P` plus unstamped legacy records and excludes records stamped with a
    /// different profile. When `None` (the profile-less session), every record
    /// is in scope — see [`experience_matches_profile`].
    pub profile_id: Option<String>,
    pub max_hits: usize,
}

/// Profile partition predicate shared by retrieval and RPC list (1c).
///
/// - A **profile-less** query (`query_profile == None`) sees **everything**:
///   the default session historically owns the whole shared experience pool and
///   must keep recalling every record it and prior versions wrote, so narrowing
///   it would silently drop guidance the default agent still relies on.
/// - A **profiled** query (`Some(P)`) sees records stamped `P` plus unstamped
///   legacy/shared records (`record_profile == None`), and excludes records
///   stamped with a different profile `Q` — the isolation the feature adds.
pub fn experience_matches_profile(
    record_profile: Option<&str>,
    query_profile: Option<&str>,
) -> bool {
    match query_profile {
        None => true,
        Some(active) => match record_profile {
            None => true,
            Some(owner) => owner == active,
        },
    }
}

#[derive(Clone)]
pub struct AgentExperienceStore {
    memory: Arc<dyn Memory>,
}

impl AgentExperienceStore {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    pub async fn put(&self, mut experience: AgentExperience) -> Result<AgentExperience, String> {
        if experience.id.trim().is_empty() {
            experience.id = stable_experience_id_for_profile(
                &experience.task_summary,
                &experience.tool_sequence,
                experience.outcome,
                experience.profile_id.as_deref(),
            );
        }
        if experience.task_summary.trim().is_empty() {
            return Err("task_summary is required".to_string());
        }
        if experience.lesson.trim().is_empty() {
            return Err("lesson is required".to_string());
        }

        let key = storage_key(&experience.id);
        if let Some(existing) = self.fetch(&key).await? {
            experience.created_at_ms = existing.created_at_ms;
        } else if experience.created_at_ms <= 0 {
            experience.created_at_ms = now_ms();
        }
        experience.updated_at_ms = now_ms();
        experience = redact_experience(experience);

        let content = serde_json::to_string(&experience).map_err(|e| e.to_string())?;
        let content = encode_experience_payload(&content);
        self.memory
            .store(
                AGENT_EXPERIENCE_NAMESPACE,
                &key,
                &content,
                MemoryCategory::Custom(AGENT_EXPERIENCE_NAMESPACE.into()),
                None,
            )
            .await
            .map_err(|e| format!("store agent experience: {e:#}"))?;

        Ok(experience)
    }

    pub async fn list(&self) -> Result<Vec<AgentExperience>, String> {
        let entries = self
            .memory
            .list(Some(AGENT_EXPERIENCE_NAMESPACE), None, None)
            .await
            .map_err(|e| format!("list agent experiences: {e:#}"))?;

        let mut experiences: Vec<AgentExperience> = entries
            .into_iter()
            .filter(|entry| entry.key.starts_with("experience/"))
            .filter_map(|entry| match decode_experience_payload(&entry.content) {
                Ok(experience) => Some(experience),
                Err(err) => {
                    log::warn!(
                        "[agent-experience] skipping malformed entry key={}: {err}",
                        entry.key
                    );
                    None
                }
            })
            .collect();

        experiences.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(experiences)
    }

    /// [`Self::list`] narrowed to a profile partition (1c). `profile_id == None`
    /// returns everything (the profile-less view); `Some(P)` returns records
    /// stamped `P` plus unstamped legacy records. Shares
    /// [`experience_matches_profile`] with retrieval so the two never diverge.
    pub async fn list_for_profile(
        &self,
        profile_id: Option<&str>,
    ) -> Result<Vec<AgentExperience>, String> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .filter(|experience| {
                experience_matches_profile(experience.profile_id.as_deref(), profile_id)
            })
            .collect())
    }

    pub async fn dismiss(&self, id: &str) -> Result<bool, String> {
        self.dismiss_for_profile(id, None).await
    }

    /// Dismiss an experience only when it belongs to the caller's visible
    /// profile partition. A profiled caller may dismiss its own or unstamped
    /// legacy records, but never a sibling profile's record even if it knows
    /// the storage id.
    pub async fn dismiss_for_profile(
        &self,
        id: &str,
        profile_id: Option<&str>,
    ) -> Result<bool, String> {
        let key = storage_key(id);
        let Some(mut experience) = self.fetch(&key).await? else {
            return Ok(false);
        };
        if !experience_matches_profile(experience.profile_id.as_deref(), profile_id) {
            return Ok(false);
        }
        experience.dismissed = true;
        experience.updated_at_ms = now_ms();
        self.put(experience).await?;
        Ok(true)
    }

    pub async fn retrieve(&self, query: ExperienceQuery) -> Result<Vec<ExperienceHit>, String> {
        if query.max_hits == 0 {
            return Ok(Vec::new());
        }

        let query_terms = terms(&query.query);
        let query_tools = normalized_set(&query.tools);
        let query_tags = normalized_set(&query.tags);

        let mut hits: Vec<ExperienceHit> = self
            .list()
            .await?
            .into_iter()
            .filter(|experience| !experience.dismissed)
            .filter(|experience| {
                experience_matches_profile(
                    experience.profile_id.as_deref(),
                    query.profile_id.as_deref(),
                )
            })
            .filter_map(|experience| {
                let (score, match_reasons) = score_experience(
                    &experience,
                    &query_terms,
                    &query_tools,
                    &query_tags,
                    query.agent_id.as_deref(),
                    query.entrypoint.as_deref(),
                );
                (score > 0.0).then_some(ExperienceHit {
                    experience,
                    score,
                    match_reasons,
                })
            })
            .collect();

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| b.experience.updated_at_ms.cmp(&a.experience.updated_at_ms))
                .then_with(|| a.experience.id.cmp(&b.experience.id))
        });
        hits.truncate(query.max_hits);
        Ok(hits)
    }

    async fn fetch(&self, key: &str) -> Result<Option<AgentExperience>, String> {
        let entry = self
            .memory
            .get(AGENT_EXPERIENCE_NAMESPACE, key)
            .await
            .map_err(|e| format!("get agent experience: {e:#}"))?;
        match entry {
            Some(entry) => decode_experience_payload(&entry.content).map(Some),
            None => Ok(None),
        }
    }
}

/// Retrieve one logical experience pool across multiple physical memory stores.
///
/// Dedicated profiles write new experiences into their own memory subtree, but
/// still need to recall unstamped legacy experiences from the shared store.
/// Keep the merge, de-duplication, ordering, and final limit in one place so the
/// RPC and live-turn paths cannot drift.
pub async fn retrieve_across_stores(
    stores: &[AgentExperienceStore],
    query: ExperienceQuery,
) -> Result<Vec<ExperienceHit>, String> {
    let max_hits = query.max_hits;
    let mut by_id: BTreeMap<String, ExperienceHit> = BTreeMap::new();
    for store in stores {
        for hit in store.retrieve(query.clone()).await? {
            let id = hit.experience.id.clone();
            match by_id.get(&id) {
                Some(existing) if existing.score >= hit.score => {}
                _ => {
                    by_id.insert(id, hit);
                }
            }
        }
    }
    let mut hits: Vec<_> = by_id.into_values().collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.experience.updated_at_ms.cmp(&a.experience.updated_at_ms))
            .then_with(|| a.experience.id.cmp(&b.experience.id))
    });
    hits.truncate(max_hits);
    Ok(hits)
}

fn storage_key(id: &str) -> String {
    format!("experience/{}", id.trim())
}

/// Redact secrets/PII from every captured free-text field before the record is
/// serialized and stored.
///
/// Previously the memory layer's own content scrubber ran over the stored JSON
/// blob, so any secret in any field was redacted at write time. We now
/// base64-encode the payload before [`Memory::store`] (so a Luhn-valid
/// millisecond timestamp can no longer be misread as a credit card and corrupt
/// the JSON — #5209), which makes that store-time scrub a no-op over the
/// payload. To preserve the security invariant we must therefore run the SAME
/// full scrubber ([`sanitize_text`] — private-key blocks,
/// Bearer/`sk-`/Stripe/npm/OAuth secrets, and the full national-ID / phone /
/// credit-card PII set) over the sensitive free-text fields ourselves, here,
/// before serialization. A secret placed in any of these is then redacted in
/// both the stored record and what recall returns.
///
/// Scope note: only free-text/description fields are scrubbed. The numeric
/// timestamp/confidence fields are left untouched — scrubbing structural
/// numbers is exactly what caused the corruption we fixed. `id` is the storage
/// key (scrubbing it would desync key vs. content; a secret-bearing key is
/// rejected up front by the memory layer's `has_likely_secret` guard) and
/// `profile_id` is a hard partition-filter key, so both are deliberately left
/// intact.
fn redact_experience(mut experience: AgentExperience) -> AgentExperience {
    fn scrub(value: &str) -> String {
        sanitize_text(value).value
    }

    experience.task_fingerprint = scrub(&experience.task_fingerprint);
    experience.task_summary = scrub(&experience.task_summary);
    experience.lesson = scrub(&experience.lesson);
    experience.reuse_hint = scrub(&experience.reuse_hint);
    experience.avoid_hint = experience.avoid_hint.as_deref().map(scrub);
    experience.error_class = experience.error_class.as_deref().map(scrub);
    experience.agent_id = experience.agent_id.as_deref().map(scrub);
    experience.entrypoint = experience.entrypoint.as_deref().map(scrub);
    experience.tools_used = experience.tools_used.iter().map(|t| scrub(t)).collect();
    experience.tool_sequence = experience.tool_sequence.iter().map(|t| scrub(t)).collect();
    experience.tags = experience.tags.iter().map(|t| scrub(t)).collect();
    experience
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn score_experience(
    experience: &AgentExperience,
    query_terms: &BTreeSet<String>,
    query_tools: &BTreeSet<String>,
    query_tags: &BTreeSet<String>,
    agent_id: Option<&str>,
    entrypoint: Option<&str>,
) -> (f32, Vec<String>) {
    let mut score = experience.confidence.clamp(0.0, 1.0) * 0.2;
    let mut reasons = Vec::new();

    let experience_tools = normalized_set(&experience.tools_used);
    let tool_overlap = overlap_count(query_tools, &experience_tools);
    if tool_overlap > 0 {
        score += 3.0 + tool_overlap as f32 * 0.5;
        reasons.push("tool_overlap".to_string());
    }

    let experience_tags = normalized_set(&experience.tags);
    let tag_overlap = overlap_count(query_tags, &experience_tags);
    if tag_overlap > 0 {
        score += 2.0 + tag_overlap as f32 * 0.25;
        reasons.push("tag_overlap".to_string());
    }

    let haystack = terms(&format!(
        "{} {} {} {}",
        experience.task_summary,
        experience.lesson,
        experience.reuse_hint,
        experience.avoid_hint.as_deref().unwrap_or_default()
    ));
    let query_overlap = overlap_count(query_terms, &haystack);
    if query_overlap > 0 {
        score += 1.0 + query_overlap as f32 * 0.2;
        reasons.push("query_overlap".to_string());
    }

    if let (Some(query_agent), Some(exp_agent)) = (agent_id, experience.agent_id.as_deref()) {
        if normalize(query_agent) == normalize(exp_agent) {
            score += 1.0;
            reasons.push("agent_match".to_string());
        }
    }

    if let (Some(query_entrypoint), Some(exp_entrypoint)) =
        (entrypoint, experience.entrypoint.as_deref())
    {
        if normalize(query_entrypoint) == normalize(exp_entrypoint) {
            score += 0.5;
            reasons.push("entrypoint_match".to_string());
        }
    }

    (score, reasons)
}

fn normalized_set(values: &[String]) -> BTreeSet<String> {
    values
        .iter()
        .map(|value| normalize(value))
        .filter(|value| !value.is_empty())
        .collect()
}

fn terms(input: &str) -> BTreeSet<String> {
    input
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(normalize)
        .filter(|term| term.len() > 2)
        .collect()
}

fn overlap_count(a: &BTreeSet<String>, b: &BTreeSet<String>) -> usize {
    a.intersection(b).count()
}

fn normalize(input: &str) -> String {
    input.trim().to_ascii_lowercase()
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
