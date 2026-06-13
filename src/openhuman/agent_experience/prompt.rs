use crate::openhuman::agent_experience::types::{ExperienceHit, ExperienceOutcome};

pub const AGENT_EXPERIENCE_HEADING: &str = "## Relevant Operating Experience";

pub fn prepend_experience_block(enriched_message: &str, block: &str) -> String {
    let block = block.trim();
    if block.is_empty() {
        return enriched_message.to_string();
    }
    format!("{block}\n\n{enriched_message}")
}

pub fn render_experience_hits(hits: &[ExperienceHit], max_bytes: usize) -> String {
    if hits.is_empty() || max_bytes == 0 {
        return String::new();
    }

    let mut rendered = AGENT_EXPERIENCE_HEADING.to_string();
    rendered = truncate_to_boundary(&rendered, max_bytes);
    if rendered.len() >= max_bytes {
        return rendered;
    }

    let separator = "\n";
    for (index, hit) in hits.iter().enumerate() {
        let item = render_hit(hit, index + 1);
        let candidate_len = rendered.len() + separator.len() + item.len();

        if candidate_len <= max_bytes {
            rendered.push_str(separator);
            rendered.push_str(&item);
            continue;
        }

        let remaining = max_bytes.saturating_sub(rendered.len() + separator.len());
        if remaining > 0 {
            rendered.push_str(separator);
            rendered.push_str(&truncate_to_boundary(&item, remaining));
        }
        break;
    }

    rendered
}

fn render_hit(hit: &ExperienceHit, index: usize) -> String {
    let exp = &hit.experience;
    let mut parts = vec![
        format!(
            "{}. {} sequence: {}",
            index,
            outcome_label(exp.outcome),
            sequence_label(&exp.tool_sequence)
        ),
        format!("lesson: {}", compact_line(&exp.lesson)),
        format!("reuse: {}", compact_line(&exp.reuse_hint)),
    ];

    if let Some(avoid_hint) = exp
        .avoid_hint
        .as_deref()
        .filter(|hint| !hint.trim().is_empty())
    {
        parts.push(format!("avoid: {}", compact_line(avoid_hint)));
    }

    parts.join(" | ")
}

fn sequence_label(sequence: &[String]) -> String {
    if sequence.is_empty() {
        "no tools".to_string()
    } else {
        sequence.join(" -> ")
    }
}

fn outcome_label(outcome: ExperienceOutcome) -> &'static str {
    match outcome {
        ExperienceOutcome::Success => "success",
        ExperienceOutcome::Failure => "failure",
        ExperienceOutcome::Partial => "partial",
    }
}

fn compact_line(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_to_boundary(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }

    let mut end = 0;
    for (idx, _) in input.char_indices() {
        if idx > max_bytes {
            break;
        }
        end = idx;
    }

    if end == 0 {
        String::new()
    } else {
        input[..end].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent_experience::types::{
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
        let rendered =
            render_experience_hits(&[sample_hit("Use grep before opening files.")], 2048);
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
}
