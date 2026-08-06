use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceSource {
    ToolLoop,
    AgentReflection,
    Manual,
    WorkflowCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceOutcome {
    Success,
    Failure,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentExperience {
    pub id: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub source: ExperienceSource,
    pub agent_id: Option<String>,
    pub entrypoint: Option<String>,
    /// Id of the agent profile the turn ran under when this experience was
    /// captured (1c). `None` for the default profile-less session and for every
    /// record written before profile scoping existed (legacy). Serde-defaulted
    /// so older stored payloads deserialize unchanged; retrieval treats `None`
    /// records as shared/legacy and surfaces them under any profile.
    #[serde(default)]
    pub profile_id: Option<String>,
    pub task_fingerprint: String,
    pub task_summary: String,
    pub tools_used: Vec<String>,
    pub tool_sequence: Vec<String>,
    pub outcome: ExperienceOutcome,
    pub error_class: Option<String>,
    pub lesson: String,
    pub reuse_hint: String,
    pub avoid_hint: Option<String>,
    pub confidence: f32,
    pub tags: Vec<String>,
    pub payload_hash: Option<String>,
    pub dismissed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperienceHit {
    pub experience: AgentExperience,
    pub score: f32,
    pub match_reasons: Vec<String>,
}

pub fn redact_text(input: &str) -> String {
    let redacted = bearer_regex().replace_all(input, "Bearer [redacted]");
    let redacted = openai_key_regex().replace_all(&redacted, "sk-[redacted]");
    secret_key_regex()
        .replace_all(&redacted, |captures: &Captures<'_>| {
            let key = captures.get(1).map_or("", |m| m.as_str());
            let separator = captures.get(2).map_or("", |m| m.as_str());
            let padding = if separator == ":" { " " } else { "" };
            format!("{key}{separator}{padding}[redacted]")
        })
        .into_owned()
}

pub fn stable_experience_id(
    task_summary: &str,
    tool_sequence: &[String],
    outcome: ExperienceOutcome,
) -> String {
    stable_experience_id_for_profile(task_summary, tool_sequence, outcome, None)
}

/// Derive a stable experience id, partitioning the storage key by the capturing
/// profile when one is set.
///
/// The store keys records by this id, so two profiles that learn the *same*
/// task/tool/outcome triple must not collapse onto one key (the later
/// `store.put()` would otherwise overwrite the earlier profile's record — see
/// the 1c retrieval partition). Mixing the profile id into the digest keeps each
/// profile's procedural experience distinct.
///
/// `profile_id == None` (the profile-less / legacy session) is **byte-identical**
/// to the pre-1c derivation, so every record written before profile scoping keeps
/// its exact id and stays retrievable. A `Some` profile appends a
/// domain-separated segment, so profile A, profile B, and `None` yield three
/// different keys for the same triple.
pub fn stable_experience_id_for_profile(
    task_summary: &str,
    tool_sequence: &[String],
    outcome: ExperienceOutcome,
    profile_id: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(task_summary.trim().to_lowercase().as_bytes());
    hasher.update(b"\0");
    for tool in tool_sequence {
        hasher.update(tool.trim().to_lowercase().as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(outcome_key(outcome).as_bytes());
    // Only stamp the profile segment when a non-empty id is present: an absent or
    // blank profile must reproduce the legacy digest byte-for-byte so existing
    // stored records keep their identity.
    if let Some(profile_id) = profile_id.map(str::trim).filter(|id| !id.is_empty()) {
        hasher.update(b"\0profile\0");
        hasher.update(profile_id.to_lowercase().as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    format!("exp_{}", &digest[..24])
}

fn outcome_key(outcome: ExperienceOutcome) -> &'static str {
    match outcome {
        ExperienceOutcome::Success => "success",
        ExperienceOutcome::Failure => "failure",
        ExperienceOutcome::Partial => "partial",
    }
}

fn bearer_regex() -> &'static Regex {
    static BEARER_RE: OnceLock<Regex> = OnceLock::new();
    BEARER_RE.get_or_init(|| Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+").unwrap())
}

fn openai_key_regex() -> &'static Regex {
    static OPENAI_KEY_RE: OnceLock<Regex> = OnceLock::new();
    OPENAI_KEY_RE.get_or_init(|| Regex::new(r"\bsk-[A-Za-z0-9]{20,}\b").unwrap())
}

fn secret_key_regex() -> &'static Regex {
    static SECRET_KEY_RE: OnceLock<Regex> = OnceLock::new();
    SECRET_KEY_RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(token|api[_-]?key|secret|password|passwd|pass|access[_-]?token|refresh[_-]?token)\s*([:=])\s*[^\s,;]+",
        )
        .unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_text_masks_secret_like_values() {
        let redacted = redact_text("token=abc123 password: hunter2 normal");
        assert!(redacted.contains("token=[redacted]"));
        assert!(redacted.contains("password: [redacted]"));
        assert!(!redacted.contains("abc123"));
        assert!(!redacted.contains("hunter2"));
        assert!(redacted.contains("normal"));
    }

    #[test]
    fn redact_text_masks_bearer_tokens_and_openai_style_keys() {
        let redacted =
            redact_text("Authorization: Bearer secret-token sk-abcdefghijklmnopqrstuvwxyz123456");
        assert!(!redacted.contains("secret-token"));
        assert!(!redacted.contains("sk-abcdefghijklmnopqrstuvwxyz123456"));
        assert!(redacted.contains("Bearer [redacted]"));
        assert!(redacted.contains("sk-[redacted]"));
    }

    #[test]
    fn stable_experience_id_is_repeatable() {
        let sequence = vec!["grep".to_string(), "file_read".to_string()];
        let first = stable_experience_id("same task", &sequence, ExperienceOutcome::Success);
        let second = stable_experience_id("same task", &sequence, ExperienceOutcome::Success);
        assert_eq!(first, second);
        assert!(first.starts_with("exp_"));
    }

    #[test]
    fn stable_experience_id_changes_when_outcome_changes() {
        let sequence = vec!["grep".to_string(), "file_read".to_string()];
        let success = stable_experience_id("same task", &sequence, ExperienceOutcome::Success);
        let failure = stable_experience_id("same task", &sequence, ExperienceOutcome::Failure);
        assert_ne!(success, failure);
    }

    #[test]
    fn stable_experience_id_for_profile_none_matches_legacy_derivation() {
        // `None` must be byte-identical to the pre-1c derivation so existing
        // stored records keep their identity.
        let sequence = vec!["grep".to_string(), "file_read".to_string()];
        let legacy = stable_experience_id("same task", &sequence, ExperienceOutcome::Success);
        let none = stable_experience_id_for_profile(
            "same task",
            &sequence,
            ExperienceOutcome::Success,
            None,
        );
        assert_eq!(legacy, none);
        // An empty / whitespace-only profile id is treated as `None`.
        let blank = stable_experience_id_for_profile(
            "same task",
            &sequence,
            ExperienceOutcome::Success,
            Some("   "),
        );
        assert_eq!(legacy, blank);
    }

    #[test]
    fn stable_experience_id_for_profile_partitions_by_profile() {
        // Same task/tool/outcome triple under profile A vs B vs None yields three
        // distinct keys, so no profile can overwrite another's record.
        let sequence = vec!["grep".to_string(), "file_read".to_string()];
        let none = stable_experience_id_for_profile(
            "same task",
            &sequence,
            ExperienceOutcome::Success,
            None,
        );
        let alice = stable_experience_id_for_profile(
            "same task",
            &sequence,
            ExperienceOutcome::Success,
            Some("alice"),
        );
        let bob = stable_experience_id_for_profile(
            "same task",
            &sequence,
            ExperienceOutcome::Success,
            Some("bob"),
        );
        assert_ne!(none, alice);
        assert_ne!(none, bob);
        assert_ne!(alice, bob);
        // Deterministic per profile.
        let alice_again = stable_experience_id_for_profile(
            "same task",
            &sequence,
            ExperienceOutcome::Success,
            Some("alice"),
        );
        assert_eq!(alice, alice_again);
    }
}
