use chrono::{TimeZone, Utc};
use serde_json::json;

use super::sync::{normalize_assignment, CanvasAssignmentDto};
use super::types::{LocalStatus, UrgencyLevel};

#[test]
fn normalize_assignment_extracts_required_fields() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let assignment: CanvasAssignmentDto = serde_json::from_value(json!({
        "id": 55,
        "name": "Soil reflection",
        "description": "<p><strong>Upload</strong> a project PDF.</p>",
        "due_at": "2026-05-20T06:00:00Z",
        "html_url": "https://mango-cmu.instructure.com/courses/101/assignments/55",
        "workflow_state": "published",
        "submission_types": ["online_upload"],
        "submission": {
            "workflow_state": "unsubmitted"
        }
    }))
    .unwrap();

    let task = normalize_assignment(
        "101",
        "361100-Secrets of the Soil-Lec.001 | 801[3/68]",
        assignment,
        now,
    );

    assert_eq!(task.course_id, "101");
    assert_eq!(
        task.course_name,
        "361100-Secrets of the Soil-Lec.001 | 801[3/68]"
    );
    assert_eq!(task.assignment_id, "55");
    assert_eq!(task.assignment_name, "Soil reflection");
    assert_eq!(task.due_at.as_deref(), Some("2026-05-20T06:00:00Z"));
    assert!(!task.due_at_unclear);
    assert_eq!(task.instructions_summary, "Upload a project PDF.");
    assert_eq!(task.submission_type.as_deref(), Some("online_upload"));
    assert_eq!(task.canvas_workflow_state.as_deref(), Some("published"));
    assert_eq!(task.canvas_submission_state.as_deref(), Some("unsubmitted"));
    assert_eq!(task.local_status, LocalStatus::NotStarted);
    assert_eq!(task.urgency_level, UrgencyLevel::Medium);
    assert_eq!(
        task.recommended_start_at.as_deref(),
        Some(now.to_rfc3339().as_str())
    );
    assert_eq!(
        task.source_url.as_deref(),
        Some("https://mango-cmu.instructure.com/courses/101/assignments/55")
    );
    assert_eq!(task.last_seen_at, now.to_rfc3339());
    assert!(task.reminders_needed.iter().any(|r| r.kind == "due_3d"));
}

#[test]
fn normalize_assignment_marks_missing_due_date_unclear() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let assignment: CanvasAssignmentDto = serde_json::from_value(json!({
        "id": "string-id",
        "name": "Open ended journal",
        "description": null,
        "due_at": null,
        "html_url": null,
        "workflow_state": "published",
        "submission_types": []
    }))
    .unwrap();

    let task = normalize_assignment("101", "Soil", assignment, now);

    assert_eq!(task.assignment_id, "string-id");
    assert!(task.due_at.is_none());
    assert!(task.due_at_unclear);
    assert_eq!(task.urgency_level, UrgencyLevel::Unclear);
    assert!(task.recommended_start_at.is_none());
    assert!(task
        .reminders_needed
        .iter()
        .any(|r| r.kind == "due_unclear"));
}

#[test]
fn normalize_assignment_marks_submitted_canvas_states() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();

    for state in ["submitted", "graded", "pending_review"] {
        let assignment: CanvasAssignmentDto = serde_json::from_value(json!({
            "id": 55,
            "name": "Soil reflection",
            "due_at": "2026-05-20T06:00:00Z",
            "submission": {
                "workflow_state": state
            }
        }))
        .unwrap();

        let task = normalize_assignment("101", "Soil", assignment, now);

        assert_eq!(task.local_status, LocalStatus::Submitted);
        assert_eq!(task.canvas_submission_state.as_deref(), Some(state));
        assert_eq!(task.urgency_level, UrgencyLevel::Low);
    }
}

#[test]
fn normalize_assignment_accepts_numeric_and_string_ids() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();

    let numeric: CanvasAssignmentDto = serde_json::from_value(json!({
        "id": 55,
        "name": "Numeric ID"
    }))
    .unwrap();
    let string: CanvasAssignmentDto = serde_json::from_value(json!({
        "id": "assignment-55",
        "name": "String ID"
    }))
    .unwrap();

    assert_eq!(
        normalize_assignment("101", "Soil", numeric, now).assignment_id,
        "55"
    );
    assert_eq!(
        normalize_assignment("101", "Soil", string, now).assignment_id,
        "assignment-55"
    );
}
