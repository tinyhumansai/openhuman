use crate::openhuman::agent::experience::types::{ExperienceHit, ExperienceOutcome};

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
#[path = "prompt_tests.rs"]
mod tests;
