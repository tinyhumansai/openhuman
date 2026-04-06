//! Working-memory extraction for skill sync payloads.
//!
//! This module defines what "user working memory" means for skill sync data:
//! durable, user-scoped facts (preferences, goals, constraints, recurring entities)
//! extracted from successful integration sync payloads.
//!
//! Scope model:
//! - **Working memory**: persisted in `global` namespace with deterministic keys.
//! - **Ephemeral chat context**: transient prompt/history data, not persisted here.
//! - **TTL**: none by default (durable), with bounded growth via fixed keys + caps.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::openhuman::memory::NamespaceDocumentInput;

const GLOBAL_NAMESPACE: &str = "global";
const MAX_SCALAR_FIELDS: usize = 1200;
const MAX_ARRAY_ITEMS_PER_NODE: usize = 40;
const MAX_DOC_ITEMS_PER_BUCKET: usize = 12;
const MAX_ITEM_CHARS: usize = 180;

const KIND_PREFERENCES: &str = "preferences";
const KIND_GOALS: &str = "goals";
const KIND_CONSTRAINTS: &str = "constraints";
const KIND_ENTITIES: &str = "entities";
const KIND_SUMMARY: &str = "summary";

/// Metrics describing extraction quality for a sync batch.
#[derive(Debug, Clone, Default)]
pub(crate) struct WorkingMemoryExtractionStats {
    pub scalar_fields_seen: usize,
    pub sensitive_fields_skipped: usize,
    pub preferences: usize,
    pub goals: usize,
    pub constraints: usize,
    pub entities: usize,
    pub documents_generated: usize,
}

/// Output for a sync batch: deterministic memory documents + extraction stats.
#[derive(Debug, Clone, Default)]
pub(crate) struct WorkingMemoryExtractionOutcome {
    pub documents: Vec<NamespaceDocumentInput>,
    pub stats: WorkingMemoryExtractionStats,
}

/// Whether skill-sync working-memory extraction is enabled.
///
/// Product-level allow switch (consent/rollout safety):
/// - `OPENHUMAN_SKILLS_WORKING_MEMORY_ENABLED=0|false|off|no` disables extraction.
/// - Any other value (or unset) enables extraction.
pub(crate) fn skills_working_memory_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("OPENHUMAN_SKILLS_WORKING_MEMORY_ENABLED")
            .map(|raw| {
                !matches!(
                    raw.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "off" | "no"
                )
            })
            .unwrap_or(true)
    })
}

#[derive(Debug, Default)]
struct WorkingMemoryBuckets {
    preferences: BTreeSet<String>,
    goals: BTreeSet<String>,
    constraints: BTreeSet<String>,
    entities: BTreeSet<String>,
}

/// Build bounded working-memory documents from a skill sync payload.
///
/// The generated documents are user-scoped, deterministic, and upsert-friendly.
/// Keys are fixed per `(skill, kind)` to prevent unbounded growth.
pub(crate) fn working_memory_documents_from_sync(
    skill_id: &str,
    sync_content: &str,
) -> WorkingMemoryExtractionOutcome {
    let skill_slug = sanitize_skill_id(skill_id);
    let source_namespace = format!("skill-{skill_slug}");
    let mut stats = WorkingMemoryExtractionStats::default();

    let Ok(payload) = serde_json::from_str::<Value>(sync_content) else {
        log::debug!(
            "[skills-working-memory] non-JSON sync payload for skill='{}'; skipping extraction",
            skill_slug
        );
        return WorkingMemoryExtractionOutcome {
            documents: Vec::new(),
            stats,
        };
    };

    let mut scalars = Vec::new();
    collect_scalar_fields(
        &payload,
        "$",
        &mut scalars,
        &mut stats.sensitive_fields_skipped,
    );
    stats.scalar_fields_seen = scalars.len();

    let mut buckets = WorkingMemoryBuckets::default();
    for (path, raw_value) in scalars {
        let normalized = normalize_text(&raw_value);
        if normalized.len() < 3 {
            continue;
        }
        if is_sensitive_value(&normalized) {
            stats.sensitive_fields_skipped += 1;
            continue;
        }

        let sanitized = redact_common_pii(&normalized);
        classify_into_buckets(&path, &sanitized, &mut buckets);
    }

    cap_set(&mut buckets.preferences, MAX_DOC_ITEMS_PER_BUCKET);
    cap_set(&mut buckets.goals, MAX_DOC_ITEMS_PER_BUCKET);
    cap_set(&mut buckets.constraints, MAX_DOC_ITEMS_PER_BUCKET);
    cap_set(&mut buckets.entities, MAX_DOC_ITEMS_PER_BUCKET);

    stats.preferences = buckets.preferences.len();
    stats.goals = buckets.goals.len();
    stats.constraints = buckets.constraints.len();
    stats.entities = buckets.entities.len();

    let mut documents = Vec::new();
    if !buckets.preferences.is_empty() {
        documents.push(build_doc(
            &skill_slug,
            &source_namespace,
            KIND_PREFERENCES,
            "User preferences extracted from skill sync",
            &render_bucket(&buckets.preferences),
        ));
    }
    if !buckets.goals.is_empty() {
        documents.push(build_doc(
            &skill_slug,
            &source_namespace,
            KIND_GOALS,
            "User goals extracted from skill sync",
            &render_bucket(&buckets.goals),
        ));
    }
    if !buckets.constraints.is_empty() {
        documents.push(build_doc(
            &skill_slug,
            &source_namespace,
            KIND_CONSTRAINTS,
            "User constraints extracted from skill sync",
            &render_bucket(&buckets.constraints),
        ));
    }
    if !buckets.entities.is_empty() {
        documents.push(build_doc(
            &skill_slug,
            &source_namespace,
            KIND_ENTITIES,
            "Recurring entities extracted from skill sync",
            &render_bucket(&buckets.entities),
        ));
    }

    if !documents.is_empty() {
        documents.push(build_doc(
            &skill_slug,
            &source_namespace,
            KIND_SUMMARY,
            "Working memory summary from skill sync",
            &render_summary(&buckets),
        ));
    }

    stats.documents_generated = documents.len();

    WorkingMemoryExtractionOutcome { documents, stats }
}

fn collect_scalar_fields(
    value: &Value,
    path: &str,
    out: &mut Vec<(String, String)>,
    sensitive_skipped: &mut usize,
) {
    if out.len() >= MAX_SCALAR_FIELDS {
        return;
    }

    match value {
        Value::Object(map) => {
            for (key, v) in map {
                if out.len() >= MAX_SCALAR_FIELDS {
                    break;
                }
                if is_sensitive_key(key) {
                    *sensitive_skipped += 1;
                    continue;
                }
                let child_path = if path == "$" {
                    format!("$.{key}")
                } else {
                    format!("{path}.{key}")
                };
                collect_scalar_fields(v, &child_path, out, sensitive_skipped);
            }
        }
        Value::Array(items) => {
            for item in items.iter().take(MAX_ARRAY_ITEMS_PER_NODE) {
                if out.len() >= MAX_SCALAR_FIELDS {
                    break;
                }
                collect_scalar_fields(item, &format!("{path}[]"), out, sensitive_skipped);
            }
        }
        Value::String(s) => out.push((path.to_string(), clip(s, MAX_ITEM_CHARS))),
        Value::Bool(b) => out.push((path.to_string(), b.to_string())),
        Value::Number(n) => out.push((path.to_string(), n.to_string())),
        Value::Null => {}
    }
}

fn classify_into_buckets(path: &str, value: &str, buckets: &mut WorkingMemoryBuckets) {
    let path_l = path.to_ascii_lowercase();
    let value_l = value.to_ascii_lowercase();

    if looks_like_preference(&path_l, &value_l) {
        buckets
            .preferences
            .insert(format!("{} ({})", value, summarize_path(&path_l)));
    }
    if looks_like_goal(&path_l, &value_l) {
        buckets
            .goals
            .insert(format!("{} ({})", value, summarize_path(&path_l)));
    }
    if looks_like_constraint(&path_l, &value_l) {
        buckets
            .constraints
            .insert(format!("{} ({})", value, summarize_path(&path_l)));
    }
    if looks_like_entity(&path_l, value) {
        buckets.entities.insert(value.to_string());
    }
}

fn render_bucket(items: &BTreeSet<String>) -> String {
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_summary(buckets: &WorkingMemoryBuckets) -> String {
    let mut sections = Vec::new();
    if !buckets.preferences.is_empty() {
        sections.push(format!(
            "Preferences:\n{}",
            render_bucket(&buckets.preferences)
        ));
    }
    if !buckets.goals.is_empty() {
        sections.push(format!("Goals:\n{}", render_bucket(&buckets.goals)));
    }
    if !buckets.constraints.is_empty() {
        sections.push(format!(
            "Constraints:\n{}",
            render_bucket(&buckets.constraints)
        ));
    }
    if !buckets.entities.is_empty() {
        sections.push(format!(
            "Recurring entities:\n{}",
            render_bucket(&buckets.entities)
        ));
    }
    sections.join("\n\n")
}

fn build_doc(
    skill_slug: &str,
    source_namespace: &str,
    kind: &str,
    title: &str,
    content: &str,
) -> NamespaceDocumentInput {
    NamespaceDocumentInput {
        namespace: GLOBAL_NAMESPACE.to_string(),
        key: format!("working.user.{skill_slug}.{kind}"),
        title: format!("{skill_slug} • {kind}"),
        content: format!("{title}\n\n{content}"),
        source_type: "skill_sync_working_memory".to_string(),
        priority: "high".to_string(),
        tags: vec![
            "working-memory".to_string(),
            "skill-sync".to_string(),
            skill_slug.to_string(),
            kind.to_string(),
        ],
        metadata: serde_json::json!({
            "memory_scope": "user_working_memory",
            "source": "skills.sync",
            "source_namespace": source_namespace,
            "skill_id": skill_slug,
            "kind": kind,
            "ttl": "none",
            "version": 1,
        }),
        category: "core".to_string(),
        session_id: None,
        document_id: None,
    }
}

fn sanitize_skill_id(skill_id: &str) -> String {
    let normalized = skill_id
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();

    if normalized.is_empty() {
        "unknown-skill".to_string()
    } else {
        normalized
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key_l = key.to_ascii_lowercase();
    [
        "token",
        "secret",
        "password",
        "credential",
        "oauth",
        "auth",
        "apikey",
        "api_key",
        "jwt",
        "cookie",
        "bearer",
        "refresh",
        "access",
        "clientkey",
        "client_key",
    ]
    .iter()
    .any(|needle| key_l.contains(needle))
}

fn is_sensitive_value(value: &str) -> bool {
    let value_l = value.to_ascii_lowercase();
    if value_l.contains("bearer ") || value_l.contains("api_key") {
        return true;
    }
    // Heuristic for opaque secrets/tokens.
    value
        .split_whitespace()
        .any(|token| token.len() >= 32 && token.chars().all(|ch| ch.is_ascii_alphanumeric()))
}

fn looks_like_preference(path: &str, value: &str) -> bool {
    [
        "preference",
        "pref",
        "setting",
        "timezone",
        "language",
        "style",
    ]
    .iter()
    .any(|needle| path.contains(needle))
        || [
            "prefer", "likes", "favorite", "timezone", "language", "style",
        ]
        .iter()
        .any(|needle| value.contains(needle))
}

fn looks_like_goal(path: &str, value: &str) -> bool {
    [
        "goal",
        "objective",
        "target",
        "milestone",
        "roadmap",
        "plan",
    ]
    .iter()
    .any(|needle| path.contains(needle))
        || [
            "goal",
            "objective",
            "target",
            "milestone",
            "plan",
            "ship",
            "deliver",
        ]
        .iter()
        .any(|needle| value.contains(needle))
}

fn looks_like_constraint(path: &str, value: &str) -> bool {
    [
        "constraint",
        "limit",
        "restriction",
        "policy",
        "deadline",
        "availability",
    ]
    .iter()
    .any(|needle| path.contains(needle))
        || [
            "must",
            "cannot",
            "can't",
            "do not",
            "deadline",
            "limited",
            "restriction",
        ]
        .iter()
        .any(|needle| value.contains(needle))
}

fn looks_like_entity(path: &str, value: &str) -> bool {
    if value.len() > 80 {
        return false;
    }
    [
        "name",
        "title",
        "project",
        "workspace",
        "team",
        "customer",
        "account",
        "label",
    ]
    .iter()
    .any(|needle| path.contains(needle))
}

fn summarize_path(path: &str) -> String {
    path.trim_start_matches("$.")
        .replace("[]", "")
        .replace('.', " > ")
}

fn cap_set(set: &mut BTreeSet<String>, max_len: usize) {
    while set.len() > max_len {
        let last = set.iter().next_back().cloned();
        if let Some(value) = last {
            set.remove(&value);
        } else {
            break;
        }
    }
}

fn normalize_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn redact_common_pii(input: &str) -> String {
    static EMAIL_RE: OnceLock<Regex> = OnceLock::new();
    static PHONE_RE: OnceLock<Regex> = OnceLock::new();

    let email_re = EMAIL_RE.get_or_init(|| {
        Regex::new(r"(?i)\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b")
            .expect("email regex should compile")
    });
    let phone_re = PHONE_RE
        .get_or_init(|| Regex::new(r"\+?\d[\d\-\s]{8,}\d").expect("phone regex should compile"));
    let redacted = email_re.replace_all(input, "[redacted-email]");
    let redacted = phone_re.replace_all(&redacted, "[redacted-phone]");
    redacted.to_string()
}

fn clip(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut clipped = String::new();
    for ch in input.chars().take(max_chars) {
        clipped.push(ch);
    }
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::openhuman::memory::{embeddings::NoopEmbedding, UnifiedMemory};

    #[test]
    fn extracts_preferences_goals_constraints_entities_and_redacts_sensitive_fields() {
        let payload = serde_json::json!({
            "user": {
                "timezone": "America/Los_Angeles",
                "email": "alice@example.com",
                "api_key": "super-secret-value"
            },
            "preferences": {
                "writing_style": "prefers concise updates",
                "language": "English"
            },
            "goals": [
                "Ship onboarding redesign by Friday"
            ],
            "constraints": [
                "No meetings after 3pm"
            ],
            "projects": [
                {"name": "Atlas"},
                {"name": "Hermes"}
            ]
        })
        .to_string();

        let outcome = working_memory_documents_from_sync("gmail", &payload);
        assert!(!outcome.documents.is_empty());

        let all_content = outcome
            .documents
            .iter()
            .map(|doc| doc.content.clone())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(all_content.contains("prefers concise updates"));
        assert!(all_content.contains("Ship onboarding redesign by Friday"));
        assert!(all_content.contains("No meetings after 3pm"));
        assert!(all_content.contains("Atlas"));
        assert!(!all_content.contains("super-secret-value"));
        assert!(!all_content.contains("alice@example.com"));
        assert!(outcome.stats.sensitive_fields_skipped > 0);
        assert!(outcome.stats.preferences > 0);
        assert!(outcome.stats.goals > 0);
        assert!(outcome.stats.constraints > 0);
        assert!(outcome.stats.entities > 0);
    }

    #[tokio::test]
    async fn generated_working_memory_docs_upsert_without_growth() {
        let tmp = tempfile::tempdir().unwrap();
        let memory = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

        let payload_v1 = serde_json::json!({
            "preferences": {"style": "prefers concise responses"},
            "goals": ["Ship v1 this week"],
            "projects": [{"name": "Atlas"}]
        })
        .to_string();
        let payload_v2 = serde_json::json!({
            "preferences": {"style": "prefers concise responses"},
            "goals": ["Ship v2 next week"],
            "projects": [{"name": "Atlas"}, {"name": "Hermes"}]
        })
        .to_string();

        let outcome_v1 = working_memory_documents_from_sync("notion", &payload_v1);
        let expected_max_docs = outcome_v1.documents.len();
        for doc in outcome_v1.documents {
            memory.upsert_document(doc).await.unwrap();
        }

        let outcome_v2 = working_memory_documents_from_sync("notion", &payload_v2);
        for doc in outcome_v2.documents {
            memory.upsert_document(doc).await.unwrap();
        }

        let docs = memory.list_documents(Some("global")).await.unwrap();
        let working_docs = docs
            .get("documents")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter(|doc| {
                doc.get("key")
                    .and_then(Value::as_str)
                    .is_some_and(|key| key.starts_with("working.user.notion."))
            })
            .count();

        assert!(
            working_docs <= expected_max_docs,
            "working memory docs should not grow unbounded for repeated syncs"
        );

        let ranked = memory
            .query_namespace_ranked("global", "Ship v2 next week", 5)
            .await
            .unwrap();
        assert!(
            ranked
                .iter()
                .any(|hit| hit.content.contains("Ship v2 next week")),
            "updated goal should be discoverable in global working memory"
        );
    }
}
