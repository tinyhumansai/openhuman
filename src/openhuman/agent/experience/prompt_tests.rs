use super::*;
use crate::openhuman::agent::experience::types::{
    AgentExperience, ExperienceHit, ExperienceOutcome, ExperienceSource,
};

fn sample_hit(lesson: impl Into<String>) -> ExperienceHit {
    ExperienceHit {
        experience: AgentExperience {
            id: "exp_test".into(),
            created_at_ms: 1,
            updated_at_ms: 1,
            source: ExperienceSource::ToolLoop,
            agent_id: Some("orchestrator".into()),
            entrypoint: Some("chat".into()),
            profile_id: None,
            task_fingerprint: "fp".into(),
            task_summary: "search docs".into(),
            tools_used: vec!["grep".into(), "file_read".into()],
            tool_sequence: vec!["grep".into(), "file_read".into()],
            outcome: ExperienceOutcome::Success,
            error_class: None,
            lesson: lesson.into(),
            reuse_hint: "searching repository documentation".into(),
            avoid_hint: Some("retrying shell commands without narrowing the query".into()),
            confidence: 0.8,
            tags: vec!["docs".into()],
            payload_hash: None,
            dismissed: false,
        },
        score: 0.9,
        match_reasons: vec!["tool_overlap".into()],
    }
}

#[test]
fn render_experience_hits_returns_empty_for_no_hits() {
    assert!(render_experience_hits(&[], 2048).is_empty());
}

#[test]
fn render_experience_hits_includes_compact_operating_guidance() {
    let rendered = render_experience_hits(&[sample_hit("Use grep before opening files.")], 2048);
    assert!(rendered.contains("Relevant Operating Experience"));
    assert!(rendered.contains("grep -> file_read"));
    assert!(rendered.contains("Use grep before opening files."));
    assert!(rendered.contains("retrying shell commands"));
}

#[test]
fn render_experience_hits_respects_byte_cap() {
    let hits = vec![sample_hit("a".repeat(2000))];
    let rendered = render_experience_hits(&hits, 256);
    assert!(rendered.len() <= 256);
    assert!(rendered.contains("Relevant Operating Experience"));
}

#[test]
fn prepend_experience_block_places_block_before_user_context() {
    let enriched = prepend_experience_block(
        "memory context\nuser message",
        "## Relevant Operating Experience\n- use grep first",
    );

    assert!(enriched.starts_with("## Relevant Operating Experience"));
    assert!(enriched.ends_with("memory context\nuser message"));
    assert!(enriched.contains("\n\nmemory context"));
}

#[test]
fn prepend_experience_block_ignores_empty_block() {
    assert_eq!(
        prepend_experience_block("memory context\nuser message", "  "),
        "memory context\nuser message"
    );
}
