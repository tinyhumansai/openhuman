use crate::openhuman::agent_experience::types::{
    redact_text, stable_experience_id, AgentExperience, ExperienceHit,
};
use crate::openhuman::memory::{Memory, MemoryCategory};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::Arc;

pub const AGENT_EXPERIENCE_NAMESPACE: &str = "agent_experience";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceQuery {
    pub query: String,
    pub tools: Vec<String>,
    pub tags: Vec<String>,
    pub agent_id: Option<String>,
    pub entrypoint: Option<String>,
    pub max_hits: usize,
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
            experience.id = stable_experience_id(
                &experience.task_summary,
                &experience.tool_sequence,
                experience.outcome,
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
            .filter_map(
                |entry| match serde_json::from_str::<AgentExperience>(&entry.content) {
                    Ok(experience) => Some(experience),
                    Err(err) => {
                        log::warn!(
                            "[agent-experience] skipping malformed entry key={}: {err}",
                            entry.key
                        );
                        None
                    }
                },
            )
            .collect();

        experiences.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(experiences)
    }

    pub async fn dismiss(&self, id: &str) -> Result<bool, String> {
        let key = storage_key(id);
        let Some(mut experience) = self.fetch(&key).await? else {
            return Ok(false);
        };
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
            Some(entry) => serde_json::from_str::<AgentExperience>(&entry.content)
                .map(Some)
                .map_err(|e| format!("parse agent experience: {e}")),
            None => Ok(None),
        }
    }
}

fn storage_key(id: &str) -> String {
    format!("experience/{}", id.trim())
}

fn redact_experience(mut experience: AgentExperience) -> AgentExperience {
    experience.task_summary = redact_text(&experience.task_summary);
    experience.lesson = redact_text(&experience.lesson);
    experience.reuse_hint = redact_text(&experience.reuse_hint);
    experience.avoid_hint = experience.avoid_hint.map(|hint| redact_text(&hint));
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
mod tests {
    use super::*;
    use crate::openhuman::agent_experience::types::{
        AgentExperience, ExperienceOutcome, ExperienceSource,
    };
    use crate::openhuman::memory_tools::test_helpers::MockMemory;
    use std::sync::Arc;

    fn sample_experience(
        id: &str,
        task_summary: &str,
        tools: Vec<&str>,
        tags: Vec<&str>,
        confidence: f32,
    ) -> AgentExperience {
        let sequence = tools.iter().map(|tool| (*tool).to_string()).collect();
        AgentExperience {
            id: id.to_string(),
            created_at_ms: 1,
            updated_at_ms: 1,
            source: ExperienceSource::ToolLoop,
            agent_id: Some("orchestrator".into()),
            entrypoint: Some("chat".into()),
            task_fingerprint: format!("fp-{id}"),
            task_summary: task_summary.to_string(),
            tools_used: tools.iter().map(|tool| (*tool).to_string()).collect(),
            tool_sequence: sequence,
            outcome: ExperienceOutcome::Success,
            error_class: None,
            lesson: format!("lesson for {task_summary}"),
            reuse_hint: format!("reuse for {task_summary}"),
            avoid_hint: None,
            confidence,
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            payload_hash: None,
            dismissed: false,
        }
    }

    fn fresh_store() -> (AgentExperienceStore, Arc<MockMemory>) {
        let memory = Arc::new(MockMemory::default());
        (AgentExperienceStore::new(memory.clone()), memory)
    }

    #[tokio::test]
    async fn put_list_and_dismiss_round_trip() {
        let (store, memory) = fresh_store();
        store
            .put(sample_experience(
                "exp_success",
                "search repository docs",
                vec!["grep", "file_read"],
                vec!["docs"],
                0.8,
            ))
            .await
            .unwrap();

        assert!(memory.entries.lock().contains_key(&(
            AGENT_EXPERIENCE_NAMESPACE.into(),
            "experience/exp_success".into()
        )));

        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "exp_success");

        let dismissed = store.dismiss("exp_success").await.unwrap();
        assert!(dismissed);
        let listed = store.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].dismissed);
    }

    #[tokio::test]
    async fn retrieve_ranks_tool_and_query_matches() {
        let (store, _) = fresh_store();
        store
            .put(sample_experience(
                "exp_docs",
                "search repository docs",
                vec!["grep", "file_read"],
                vec!["docs"],
                0.6,
            ))
            .await
            .unwrap();
        store
            .put(sample_experience(
                "exp_email",
                "send a careful email",
                vec!["email"],
                vec!["mail"],
                1.0,
            ))
            .await
            .unwrap();

        let hits = store
            .retrieve(ExperienceQuery {
                query: "search docs with grep".into(),
                tools: vec!["grep".into()],
                tags: vec!["docs".into()],
                agent_id: Some("orchestrator".into()),
                entrypoint: Some("chat".into()),
                max_hits: 2,
            })
            .await
            .unwrap();

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].experience.id, "exp_docs");
        assert!(hits[0].score > hits[1].score);
        assert!(hits[0].match_reasons.contains(&"tool_overlap".into()));
        assert!(hits[0].match_reasons.contains(&"query_overlap".into()));
    }

    #[tokio::test]
    async fn retrieve_ignores_dismissed_records() {
        let (store, _) = fresh_store();
        store
            .put(sample_experience(
                "exp_dismissed",
                "search repository docs",
                vec!["grep", "file_read"],
                vec!["docs"],
                0.8,
            ))
            .await
            .unwrap();
        store.dismiss("exp_dismissed").await.unwrap();

        let hits = store
            .retrieve(ExperienceQuery {
                query: "search repository docs".into(),
                tools: vec!["grep".into()],
                tags: vec!["docs".into()],
                agent_id: None,
                entrypoint: None,
                max_hits: 5,
            })
            .await
            .unwrap();

        assert!(hits.is_empty());
    }
}
