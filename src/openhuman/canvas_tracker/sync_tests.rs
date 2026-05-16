use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use serde_json::json;
use tempfile::tempdir;

use super::store::CanvasTrackerStore;
use super::sync::{
    normalize_assignment, sync_once_with_client, CanvasAssignmentDto, CanvasCourseDto,
    CanvasPlannerItemDto, CanvasSyncApi,
};
use super::types::{CanvasTrackerSettings, CourseMatcher, LocalStatus, UrgencyLevel};

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
fn normalize_assignment_marks_malformed_due_date_unclear() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let assignment: CanvasAssignmentDto = serde_json::from_value(json!({
        "id": "bad-due",
        "name": "Ambiguous due date",
        "description": "Check Canvas.",
        "due_at": "next Friday",
        "submission_types": ["online_text_entry"]
    }))
    .unwrap();

    let task = normalize_assignment("101", "Soil", assignment, now);

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

#[derive(Default)]
struct FakeCanvasApi {
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeCanvasApi {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl CanvasSyncApi for FakeCanvasApi {
    async fn get_courses(&self) -> Result<Vec<CanvasCourseDto>, String> {
        self.calls.lock().unwrap().push("courses".to_string());
        Ok(vec![
            serde_json::from_value(json!({
                "id": "101",
                "name": "361100-Secrets of the Soil-Lec.001 | 801[3/68]"
            }))
            .unwrap(),
            serde_json::from_value(json!({
                "id": "303",
                "name": "001201 - CRIT READ AND EFFEC WRITE"
            }))
            .unwrap(),
        ])
    }

    async fn get_planner_items(
        &self,
        context_codes: Vec<String>,
    ) -> Result<Vec<CanvasPlannerItemDto>, String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("planner:{}", context_codes.join(",")));
        Ok(vec![serde_json::from_value(json!({
            "course_id": "101",
            "plannable_type": "assignment",
            "plannable_id": "55",
            "plannable": {
                "id": "55",
                "name": "Planner assignment",
                "description": "<p>Upload a PDF.</p>",
                "due_at": "2026-05-20T06:00:00Z",
                "submission_types": ["online_upload"],
                "submission": { "workflow_state": "unsubmitted" }
            }
        }))
        .unwrap()])
    }

    async fn get_assignments(&self, course_id: String) -> Result<Vec<CanvasAssignmentDto>, String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("assignments:{course_id}"));
        Ok(vec![])
    }

    async fn get_assignment(
        &self,
        course_id: String,
        assignment_id: String,
    ) -> Result<CanvasAssignmentDto, String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("assignment:{course_id}:{assignment_id}"));
        Err("unexpected assignment detail fetch".to_string())
    }
}

#[tokio::test]
async fn sync_fetches_planner_items_for_approved_courses_only() {
    let temp = tempdir().unwrap();
    let store = CanvasTrackerStore::new(temp.path()).unwrap();
    let mut settings = CanvasTrackerSettings::default();
    settings.allowlisted_courses = vec![CourseMatcher {
        canvas_id: Some("303".to_string()),
        name: "001201 - CRIT READ AND EFFEC WRITE".to_string(),
    }];
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let api = FakeCanvasApi::default();

    let summary = sync_once_with_client(&api, store, &settings, now)
        .await
        .unwrap();

    assert_eq!(summary.courses_seen, 2);
    assert_eq!(summary.courses_used, 1);
    assert_eq!(summary.courses_ignored, 1);
    assert_eq!(summary.assignments_seen, 1);
    let calls = api.calls();
    assert!(calls.contains(&"planner:course_101".to_string()));
    assert!(calls.contains(&"assignments:101".to_string()));
    assert!(!calls.iter().any(|call| call.contains("303")));

    let store = CanvasTrackerStore::new(temp.path()).unwrap();
    let tasks = store.list_tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignment_name, "Planner assignment");
}
