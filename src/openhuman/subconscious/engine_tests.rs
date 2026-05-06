use super::*;

fn test_tasks() -> Vec<SubconsciousTask> {
    vec![
        SubconsciousTask {
            id: "t1".into(),
            title: "Check email".into(),
            source: TaskSource::User,
            recurrence: TaskRecurrence::Cron("0 8 * * *".into()),
            enabled: true,
            last_run_at: None,
            next_run_at: None,
            completed: false,
            created_at: 0.0,
        },
        SubconsciousTask {
            id: "t2".into(),
            title: "Monitor skills".into(),
            source: TaskSource::System,
            recurrence: TaskRecurrence::Pending,
            enabled: true,
            last_run_at: None,
            next_run_at: None,
            completed: false,
            created_at: 0.0,
        },
    ]
}

#[test]
fn parse_evaluation_response() {
    let json = r#"{"evaluations": [
        {"task_id": "t1", "decision": "act", "reason": "3 new urgent emails"},
        {"task_id": "t2", "decision": "noop", "reason": "All skills healthy"}
    ]}"#;
    let (evals, drafts) = parse_response(json, &test_tasks());
    assert_eq!(evals.len(), 2);
    assert_eq!(evals[0].decision, TickDecision::Act);
    assert_eq!(evals[1].decision, TickDecision::Noop);
    assert!(drafts.is_empty());
}

#[test]
fn parse_evaluation_bare_array() {
    let json = r#"[
        {"task_id": "t1", "decision": "escalate", "reason": "Deadline conflict"}
    ]"#;
    let (evals, drafts) = parse_response(json, &test_tasks());
    assert_eq!(evals.len(), 1);
    assert_eq!(evals[0].decision, TickDecision::Escalate);
    assert!(drafts.is_empty());
}

#[test]
fn parse_evaluation_in_markdown() {
    let json = "```json\n{\"evaluations\": [{\"task_id\": \"t1\", \"decision\": \"act\", \"reason\": \"Found items\"}]}\n```";
    let (evals, _) = parse_response(json, &test_tasks());
    assert_eq!(evals.len(), 1);
    assert_eq!(evals[0].decision, TickDecision::Act);
}

#[test]
fn parse_evaluation_garbage_falls_back_to_noop() {
    let (evals, drafts) = parse_response("Not JSON at all", &test_tasks());
    assert_eq!(evals.len(), 2);
    assert!(evals.iter().all(|e| e.decision == TickDecision::Noop));
    assert!(drafts.is_empty());
}

#[test]
fn parse_response_extracts_reflections() {
    let json = r#"{
        "evaluations": [{"task_id": "t1", "decision": "noop", "reason": "nothing"}],
        "reflections": [
            {
                "kind": "hotness_spike",
                "body": "Phoenix surge",
                "disposition": "notify",
                "proposed_action": "Pull mentions",
                "source_refs": ["entity:phoenix"]
            },
            {
                "kind": "daily_digest",
                "body": "New digest",
                "disposition": "observe"
            }
        ]
    }"#;
    let (evals, drafts) = parse_response(json, &test_tasks());
    assert_eq!(evals.len(), 1);
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0].body, "Phoenix surge");
    assert_eq!(drafts[1].body, "New digest");
}

#[test]
fn parse_response_handles_only_reflections() {
    // LLM emitted reflections but no per-task evaluations.
    let json = r#"{
        "evaluations": [],
        "reflections": [
            {"kind": "risk", "body": "Concerning pattern", "disposition": "notify"}
        ]
    }"#;
    let (evals, drafts) = parse_response(json, &test_tasks());
    // Tasks default to Noop so the existing tick loop still updates log entries.
    assert_eq!(evals.len(), 2);
    assert!(evals.iter().all(|e| e.decision == TickDecision::Noop));
    assert_eq!(drafts.len(), 1);
}

#[test]
fn extract_json_object() {
    assert_eq!(extract_json(r#"{"key": "val"}"#), r#"{"key": "val"}"#);
}

#[test]
fn extract_json_from_text() {
    let input = "Here's the result: {\"evaluations\": []} done.";
    assert!(extract_json(input).starts_with('{'));
    assert!(extract_json(input).ends_with('}'));
}
