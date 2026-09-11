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

#[test]
fn pull_request_objective_and_prompt_say_review() {
    let mut t = task();
    t.kind = TaskKind::PullRequest;
    let e = enrich_task(t);
    assert_eq!(
        e.objective.as_deref(),
        Some("Review pull request: Fix login bug")
    );
    assert!(e.agent_prompt.contains("needs review"));
    assert!(e.agent_prompt.contains("Do not merge"));
}

#[test]
fn issue_objective_and_prompt_say_resolve() {
    let mut t = task();
    t.kind = TaskKind::Issue;
    let e = enrich_task(t);
    assert_eq!(e.objective.as_deref(), Some("Resolve issue: Fix login bug"));
    assert!(e.agent_prompt.contains("needs to be resolved"));
    assert!(e.agent_prompt.contains("implement and validate a fix"));
}

#[test]
fn generic_objective_is_bare_title_and_prompt_is_neutral() {
    // notion/linear/clickup default to Generic — no review/resolve framing.
    let e = enrich_task(task());
    assert_eq!(e.objective.as_deref(), Some("Fix login bug"));
    assert!(e.agent_prompt.contains("needs your attention"));
    assert!(e
        .agent_prompt
        .contains("make progress and update the todo card"));
}
