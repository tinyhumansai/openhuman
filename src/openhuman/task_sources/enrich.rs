//! Enrichment: turn a raw [`NormalizedTask`] into an agent-ready
//! [`EnrichedTask`].
//!
//! Enrichment is deterministic and dependency-free: it derives an
//! urgency score from the task's priority / labels / due date, links the
//! assignee as a person, builds a concise summary, and templates an
//! actionable agent prompt. The downstream triage turn does the heavy
//! LLM reasoning, so enrichment intentionally avoids a second model call
//! here (cheaper, deterministic, unit-testable). An optional LLM
//! summarizer can be layered on later behind the same signature.

use chrono::Utc;

use super::types::EnrichedTask;
use super::NormalizedTask;

/// Maximum length of the derived summary, in characters.
const SUMMARY_MAX_CHARS: usize = 200;

/// Enrich a normalized task. Never fails — worst case it returns a
/// title-only summary with a neutral urgency.
pub fn enrich_task(task: NormalizedTask) -> EnrichedTask {
    let summary = derive_summary(&task);
    let urgency = derive_urgency(&task);
    let linked_people = task
        .assignee
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| vec![s.to_string()])
        .unwrap_or_default();
    let agent_prompt = build_agent_prompt(&task, &summary, urgency);

    EnrichedTask {
        task,
        summary,
        urgency,
        linked_people,
        linked_memory_ids: Vec::new(),
        agent_prompt,
        enriched_at: Utc::now(),
    }
}

/// One- to two-line summary: the first non-empty line of the body, else
/// the title. Truncated on a char boundary.
fn derive_summary(task: &NormalizedTask) -> String {
    let raw = task
        .body
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|b| b.lines().find(|l| !l.trim().is_empty()))
        .map(|l| l.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(task.title.trim());
    truncate_chars(raw, SUMMARY_MAX_CHARS)
}

/// Heuristic urgency in `0.0..=1.0` from priority, labels, and due date.
fn derive_urgency(task: &NormalizedTask) -> f32 {
    let mut score: f32 = 0.4; // neutral baseline

    if let Some(p) = task.priority.as_deref().map(|s| s.to_ascii_lowercase()) {
        if p.contains("urgent") || p.contains("critical") || p.contains("p0") {
            score = score.max(0.95);
        } else if p.contains("high") || p.contains("p1") {
            score = score.max(0.8);
        } else if p.contains("medium") || p.contains("p2") {
            score = score.max(0.55);
        } else if p.contains("low") || p.contains("p3") {
            // Low priority should pull urgency *down* from the 0.4 baseline.
            score = score.min(0.3);
        }
    }

    for label in &task.labels {
        let l = label.to_ascii_lowercase();
        if l.contains("urgent") || l.contains("p0") || l == "critical" || l == "blocker" {
            score = score.max(0.9);
        } else if l == "bug" || l.contains("p1") || l.contains("security") {
            score = score.max(0.7);
        }
    }

    // A present due date nudges urgency up; an overdue one more so.
    if let Some(due) = task.due.as_deref().and_then(parse_iso) {
        let now = Utc::now();
        if due <= now {
            score = score.max(0.85);
        } else {
            let days = (due - now).num_days();
            if days <= 1 {
                score = score.max(0.8);
            } else if days <= 3 {
                score = score.max(0.65);
            } else if days <= 7 {
                score = score.max(0.5);
            }
        }
    }

    score.clamp(0.0, 1.0)
}

fn build_agent_prompt(task: &NormalizedTask, summary: &str, urgency: f32) -> String {
    let mut lines = vec![format!(
        "A task was ingested from {} and needs your attention.",
        task.provider
    )];
    lines.push(format!("Title: {}", task.title));
    if !summary.is_empty() && summary != task.title.trim() {
        lines.push(format!("Summary: {summary}"));
    }
    if let Some(status) = task.status.as_deref().filter(|s| !s.trim().is_empty()) {
        lines.push(format!("Status: {status}"));
    }
    if let Some(assignee) = task.assignee.as_deref().filter(|s| !s.trim().is_empty()) {
        lines.push(format!("Assignee: {assignee}"));
    }
    if let Some(due) = task.due.as_deref().filter(|s| !s.trim().is_empty()) {
        lines.push(format!("Due: {due}"));
    }
    if let Some(url) = task.url.as_deref().filter(|s| !s.trim().is_empty()) {
        lines.push(format!("Link: {url}"));
    }
    lines.push(format!("Estimated urgency: {:.0}%.", urgency * 100.0));
    lines.push(
        "Decide whether this is actionable now; if so, make progress and update the todo card."
            .to_string(),
    );
    lines.join("\n")
}

fn parse_iso(raw: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn truncate_chars(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> NormalizedTask {
        NormalizedTask {
            external_id: "1".into(),
            provider: "github".into(),
            title: "Fix login bug".into(),
            ..Default::default()
        }
    }

    #[test]
    fn summary_prefers_first_body_line_then_title() {
        let mut t = task();
        t.body = Some("\n  First line of detail\nsecond line".into());
        assert_eq!(enrich_task(t).summary, "First line of detail");

        let bare = task();
        assert_eq!(enrich_task(bare).summary, "Fix login bug");
    }

    #[test]
    fn summary_truncates_long_text() {
        let mut t = task();
        t.title = "x".repeat(500);
        let e = enrich_task(t);
        assert!(e.summary.chars().count() <= SUMMARY_MAX_CHARS);
        assert!(e.summary.ends_with('…'));
    }

    #[test]
    fn urgency_baseline_is_neutral() {
        let e = enrich_task(task());
        assert!((e.urgency - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn urgency_escalates_with_priority_and_labels() {
        let mut t = task();
        t.priority = Some("Urgent".into());
        assert!(enrich_task(t).urgency >= 0.95);

        let mut t2 = task();
        t2.labels = vec!["bug".into()];
        assert!(enrich_task(t2).urgency >= 0.7);
    }

    #[test]
    fn urgency_escalates_when_overdue() {
        let mut t = task();
        t.due = Some("2000-01-01T00:00:00Z".into());
        assert!(enrich_task(t).urgency >= 0.85);
    }

    #[test]
    fn assignee_becomes_linked_person() {
        let mut t = task();
        t.assignee = Some("alice".into());
        let e = enrich_task(t);
        assert_eq!(e.linked_people, vec!["alice".to_string()]);
    }

    #[test]
    fn agent_prompt_includes_title_provider_and_link() {
        let mut t = task();
        t.url = Some("https://example.com/1".into());
        let e = enrich_task(t);
        assert!(e.agent_prompt.contains("github"));
        assert!(e.agent_prompt.contains("Fix login bug"));
        assert!(e.agent_prompt.contains("https://example.com/1"));
    }
}
