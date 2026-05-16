use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::policy::{classify_urgency, recommended_start_at, reminder_plan, strip_html_summary};
use super::types::{CanvasTask, LocalStatus, UrgencyLevel};

#[derive(Debug, Clone, Deserialize)]
pub struct CanvasCourseDto {
    pub id: Value,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CanvasSubmissionDto {
    pub workflow_state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CanvasAssignmentDto {
    pub id: Value,
    pub name: Option<String>,
    pub description: Option<String>,
    pub due_at: Option<String>,
    pub html_url: Option<String>,
    pub workflow_state: Option<String>,
    #[serde(default)]
    pub submission_types: Vec<String>,
    pub submission: Option<CanvasSubmissionDto>,
}

pub fn normalize_assignment(
    course_id: &str,
    course_name: &str,
    assignment: CanvasAssignmentDto,
    now: DateTime<Utc>,
) -> CanvasTask {
    let canvas_submission_state = assignment
        .submission
        .as_ref()
        .and_then(|submission| submission.workflow_state.clone());
    let local_status = if canvas_submission_state
        .as_deref()
        .is_some_and(is_submitted_state)
    {
        LocalStatus::Submitted
    } else {
        LocalStatus::NotStarted
    };
    let due_at_unclear = assignment
        .due_at
        .as_deref()
        .map(|due_at| due_at.trim().is_empty())
        .unwrap_or(true);

    let mut task = CanvasTask {
        course_id: course_id.to_string(),
        course_name: course_name.to_string(),
        assignment_id: id_to_string(&assignment.id),
        assignment_name: assignment
            .name
            .unwrap_or_else(|| "Untitled assignment".to_string()),
        due_at: assignment.due_at,
        due_at_unclear,
        instructions_summary: assignment
            .description
            .as_deref()
            .map(strip_html_summary)
            .unwrap_or_default(),
        submission_type: assignment.submission_types.into_iter().next(),
        canvas_workflow_state: assignment.workflow_state,
        canvas_submission_state,
        local_status,
        urgency_level: UrgencyLevel::Unclear,
        recommended_start_at: None,
        reminders_needed: vec![],
        source_url: assignment.html_url,
        last_seen_at: now.to_rfc3339(),
    };

    task.urgency_level = classify_urgency(&task, now);
    task.recommended_start_at = recommended_start_at(&task, now);
    task.reminders_needed = reminder_plan(&task, now);
    task
}

fn id_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn is_submitted_state(value: &str) -> bool {
    matches!(value, "submitted" | "graded" | "pending_review")
}
