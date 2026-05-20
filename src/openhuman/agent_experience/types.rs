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
    SkillCandidate,
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
    let mut hasher = Sha256::new();
    hasher.update(task_summary.trim().to_lowercase().as_bytes());
    hasher.update(b"\0");
    for tool in tool_sequence {
        hasher.update(tool.trim().to_lowercase().as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(outcome_key(outcome).as_bytes());
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
}
